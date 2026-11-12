import Foundation
import Testing
import Ykt

/// YMap and YArray: reads and writes, values of every kind (locally and on a replica), out-of-range
/// errors, concurrent edits, observers with origins, remote updates and unsubscribing, transactions.
struct MapArrayTests {
    private func sync(_ from: YDoc, _ to: YDoc, origin: String? = nil) throws {
        try applyUpdateV2(doc: to, update: try encodeStateAsUpdateV2(doc: from), origin: origin.map { Data($0.utf8) })
    }

    private let everyKind: [(String, YValue)] = [
        ("null", .null),
        ("bool", .bool(true)),
        ("int", .number(42)),
        ("float", .number(-1.5)),
        ("bigint", .bigInt((1 << 60) + 1)),
        ("string", .string("héllo 😀")),
        ("buffer", .buffer(Data([0, 1, 255]))),
        ("array", .array([.number(1), .string("two"), .null, .array([.bool(false)])])),
        ("map", .attrMap(["a": .number(1), "b": .attrMap([:])])),
    ]

    // MARK: - YMap

    @Test func mapSetGetDelete() throws {
        let doc = YDoc()
        let map = try doc.getMap(name: "m")
        #expect(try map.get(key: "a") == nil)

        try map.set(key: "a", value: .number(1))
        try map.set(key: "b", value: .string("two"))
        try map.set(key: "a", value: .number(3))

        #expect(try map.get(key: "a") == .number(3))
        #expect(try map.containsKey(key: "b"))
        #expect(try !map.containsKey(key: "c"))
        #expect(try map.len() == 2)
        #expect(try Set(map.keys()) == ["a", "b"])
        #expect(try Dictionary(uniqueKeysWithValues: zip(map.keys(), map.values())) == map.toJson())
        #expect(try map.toJson() == ["a": .number(3), "b": .string("two")])
        #expect(try map.delete(key: "a") == .number(3))
        #expect(try map.delete(key: "a") == nil)
        try map.clear()
        #expect(try map.len() == 0)
    }

    @Test func mapValuesOfEveryKindOnAReplica() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        for (key, value) in everyKind { try a.getMap(name: "m").set(key: key, value: value) }
        let b = YDoc(options: YDocOptions(clientId: 2))
        try sync(a, b)

        for doc in [a, b] {
            for (key, value) in everyKind {
                #expect(try doc.getMap(name: "m").get(key: key) == value, "\(key)")
            }
        }
    }

    @Test func mapObserverKeysOriginsAndRemote() throws {
        let doc = YDoc(options: YDocOptions(clientId: 1))
        let map = try doc.getMap(name: "m")
        try map.set(key: "kept", value: .number(1))
        try map.set(key: "gone", value: .number(2))
        let events = Box<YMapEvent>()
        let sub = try map.observe { events.append($0) }

        try doc.transact(origin: "editor") { txn in
            try map.set(key: "new", value: .string("y"), txn: txn)
            try map.set(key: "kept", value: .number(10), txn: txn)
            _ = try map.delete(key: "gone", txn: txn)
        }
        let remote = YDoc(options: YDocOptions(clientId: 2))
        try sync(doc, remote)
        try remote.getMap(name: "m").set(key: "remote", value: .bool(true))
        try sync(remote, doc, origin: "server")
        _ = sub.unsubscribe()
        try map.set(key: "unheard", value: .null)

        #expect(events.all.map(\.keys) == [
            ["new": .inserted(value: .string("y")), "kept": .updated(oldValue: .number(1), newValue: .number(10)), "gone": .removed(oldValue: .number(2))],
            ["remote": .inserted(value: .bool(true))],
        ])
        #expect(events.all.map(\.originString) == ["editor", "server"])
    }

    // MARK: - YArray

    @Test func arrayInsertPushGetDelete() throws {
        let doc = YDoc()
        let array = try doc.getArray(name: "a")
        try array.push(values: [.number(1), .number(2)])
        try array.insert(index: 0, values: [.string("first")])
        try array.insert(index: 3, values: [.string("last")])

        #expect(try array.toJson() == [.string("first"), .number(1), .number(2), .string("last")])
        #expect(try array.get(index: 3) == .string("last"))
        #expect(try array.len() == 4)
        try array.delete(index: 1, length: 2)
        #expect(try array.toJson() == [.string("first"), .string("last")])

        #expect(throws: YError.OutOfBounds) { try array.insert(index: 3, values: [.null]) }
        #expect(throws: YError.OutOfBounds) { try array.delete(index: 1, length: 2) }
        #expect(throws: YError.OutOfBounds) { try array.get(index: 2) }
        #expect(try array.len() == 2)
    }

    @Test func arrayValuesOfEveryKindOnAReplica() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        let values = everyKind.map(\.1)
        try a.getArray(name: "a").push(values: values)
        let b = YDoc(options: YDocOptions(clientId: 2))
        try sync(a, b)

        #expect(try a.getArray(name: "a").toJson() == values)
        #expect(try b.getArray(name: "a").toJson() == values)
    }

    @Test func concurrentArrayInsertsConverge() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        let b = YDoc(options: YDocOptions(clientId: 2))
        try a.getArray(name: "a").push(values: [.string("base")])
        try sync(a, b)
        try a.getArray(name: "a").insert(index: 0, values: [.string("a")])
        try b.getArray(name: "a").push(values: [.string("b")])

        try sync(a, b)
        try sync(b, a)

        #expect(try a.getArray(name: "a").toJson() == b.getArray(name: "a").toJson())
        #expect(try a.getArray(name: "a").len() == 3)
    }

    @Test func arrayObserverDeltasAndOrigins() throws {
        let doc = YDoc()
        let array = try doc.getArray(name: "a")
        let events = Box<YArrayEvent>()
        var sub: YSubscription? = try array.observe { events.append($0) }

        try array.push(values: [.number(1), .number(2), .number(3)])
        try array.insert(index: 1, values: [.string("x")])
        try doc.transact(origin: "editor") { txn in
            try array.delete(index: 0, length: 1, txn: txn)
            try array.push(values: [.bool(true)], txn: txn)
        }
        sub = nil // released: unsubscribed
        try array.push(values: [.null])

        #expect(events.all.map(\.delta) == [
            [.insert([.number(1), .number(2), .number(3)])],
            [.retain(1), .insert([.string("x")])],
            [.delete(1), .retain(3), .insert([.bool(true)])],
        ])
        #expect(events.all.map(\.originString) == [nil, nil, "editor"])
        #expect(sub == nil)
    }

    // MARK: - Transactions

    @Test func insideATransactionAndACallback() throws {
        let doc = YDoc()
        let map = try doc.getMap(name: "m")
        let array = try doc.getArray(name: "a")
        try doc.transact { txn in
            try map.set(key: "k", value: .number(1), txn: txn)
            try array.push(values: [.number(1)], txn: txn)
            #expect(try map.get(key: "k", txn: txn) == .number(1))
            #expect(try array.len(txn: txn) == 1)
            #expect(throws: YError.AnotherRwTx) { try map.get(key: "k") }
            #expect(throws: YError.AnotherTx) { try doc.getMap(name: "m") }
        }

        let results = Box<Bool>()
        let handle = try doc.getMap(name: "m")
        let sub = try map.observe { _ in results.append((try? handle.get(key: "k")) == nil) }
        try map.set(key: "k", value: .number(2))
        #expect(results.all == [true], "reading in a callback throws")
        _ = sub
    }
}
