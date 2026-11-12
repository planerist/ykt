import Foundation
import Testing
import Ykt

/// Update utilities without a document: merging, diffing against a replica's state vector, state
/// vectors read from updates, and bytes that aren't updates (an error, never a crash).
struct UpdateTests {
    /// A document edited in five transactions, with every transaction's update as it was sent.
    private func edited(v2: Bool) throws -> (YDoc, [Data]) {
        let doc = YDoc(options: YDocOptions(clientId: 1))
        let updates = Box<Data>()
        let sub: YSubscription
        if v2 {
            sub = try doc.observeUpdateV2 { updates.append($0.update) }
        } else {
            sub = try doc.observeUpdateV1 { updates.append($0.update) }
        }
        let text = try doc.getText(name: "t")
        try text.insert(index: 0, chunk: "hello")
        try text.insert(index: 5, chunk: " world")
        try text.format(index: 0, length: 5, attributes: #"{"bold":true}"#)
        try text.delete(index: 0, length: 1)
        try text.insert(index: 10, chunk: "!")
        _ = sub.unsubscribe()
        return (doc, updates.all)
    }

    private func apply(_ update: Data, to doc: YDoc, v2: Bool) throws {
        if v2 { try applyUpdateV2(doc: doc, update: update) } else { try applyUpdate(doc: doc, update: update) }
    }

    private func isInvalidData(_ body: () throws -> Any) -> Bool {
        do {
            _ = try body()
            return false
        } catch YError.InvalidData {
            return true
        } catch {
            return false
        }
    }

    @Test(arguments: [false, true]) func mergingIsApplyingOneByOne(v2: Bool) throws {
        let (source, updates) = try edited(v2: v2)
        #expect(updates.count == 5)
        let merged = try v2 ? mergeUpdatesV2(updates: updates) : mergeUpdatesV1(updates: updates)
        let reversed = try v2 ? mergeUpdatesV2(updates: updates.reversed()) : mergeUpdatesV1(updates: updates.reversed())

        for update in [merged, reversed] {
            let target = YDoc(options: YDocOptions(clientId: 2))
            try apply(update, to: target, v2: v2)
            #expect(try target.getText(name: "t").toText() == "ello world!")
            #expect(try target.getText(name: "t").toDelta() == source.getText(name: "t").toDelta())
            #expect(try encodeStateVector(doc: target) == encodeStateVector(doc: source))
        }
    }

    @Test(arguments: [false, true]) func mergingNothing(v2: Bool) throws {
        let empty = try v2 ? mergeUpdatesV2(updates: []) : mergeUpdatesV1(updates: [])
        let target = YDoc()
        let heard = Box<Int>()
        let sub = try target.observeUpdateV1 { _ in heard.append(1) }

        try apply(empty, to: target, v2: v2)

        #expect(heard.count == 0)
        #expect(try target.getText(name: "t").toText() == "")
        _ = sub
    }

    @Test(arguments: [false, true]) func diffIsWhatAReplicaLacks(v2: Bool) throws {
        let (source, updates) = try edited(v2: v2)
        let full = try v2 ? encodeStateAsUpdateV2(doc: source) : encodeStateAsUpdate(doc: source)
        let replica = YDoc(options: YDocOptions(clientId: 2))
        try apply(updates[0], to: replica, v2: v2)
        try apply(updates[1], to: replica, v2: v2)
        let replicaState = try encodeStateVector(doc: replica)

        let missing = try v2 ? diffUpdateV2(update: full, stateVector: replicaState) : diffUpdateV1(update: full, stateVector: replicaState)
        #expect(missing.count < full.count)
        let fresh = YDoc(options: YDocOptions(clientId: 3))
        try apply(missing, to: fresh, v2: v2)
        #expect(try fresh.getText(name: "t").toText() == "", "alone it builds on nothing")

        try apply(missing, to: replica, v2: v2)
        #expect(try replica.getText(name: "t").toText() == "ello world!")
        #expect(try replica.getText(name: "t").toDelta() == source.getText(name: "t").toDelta())
    }

    @Test(arguments: [false, true]) func diffKeepsDeletions(v2: Bool) throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        try a.getText(name: "t").insert(index: 0, chunk: "abc")
        let replica = YDoc(options: YDocOptions(clientId: 2))
        try apply(v2 ? encodeStateAsUpdateV2(doc: a) : encodeStateAsUpdate(doc: a), to: replica, v2: v2)
        try a.getText(name: "t").delete(index: 0, length: 1)

        let full = try v2 ? encodeStateAsUpdateV2(doc: a) : encodeStateAsUpdate(doc: a)
        let state = try encodeStateVector(doc: replica)
        try apply(v2 ? diffUpdateV2(update: full, stateVector: state) : diffUpdateV1(update: full, stateVector: state), to: replica, v2: v2)

        #expect(try replica.getText(name: "t").toText() == "bc")
    }

    @Test(arguments: [false, true]) func stateVectorOfAnUpdateIsTheDocuments(v2: Bool) throws {
        let (source, updates) = try edited(v2: v2)
        let full = try v2 ? encodeStateAsUpdateV2(doc: source) : encodeStateAsUpdate(doc: source)

        let fromUpdate = try v2 ? encodeStateVectorFromUpdateV2(update: full) : encodeStateVectorFromUpdateV1(update: full)
        let ofOneLaterTransaction = try v2 ? encodeStateVectorFromUpdateV2(update: updates[1]) : encodeStateVectorFromUpdateV1(update: updates[1])

        #expect(try fromUpdate == encodeStateVector(doc: source))
        #expect(try ofOneLaterTransaction == encodeStateVector(doc: YDoc()), "clocks count from 0: a later part says nothing")
    }

    @Test func garbageThrows() throws {
        let (_, updates) = try edited(v2: false)
        for bytes in [Data(), Data(repeating: 0xff, count: 12), Data([1, 2, 3])] {
            #expect(isInvalidData { try mergeUpdatesV1(updates: [bytes]) })
            #expect(isInvalidData { try mergeUpdatesV2(updates: [bytes]) })
            #expect(isInvalidData { try diffUpdateV1(update: bytes, stateVector: Data([0])) })
            #expect(isInvalidData { try diffUpdateV2(update: bytes, stateVector: Data([0])) })
            #expect(isInvalidData { try encodeStateVectorFromUpdateV1(update: bytes) })
            #expect(isInvalidData { try encodeStateVectorFromUpdateV2(update: bytes) })
        }
        #expect(isInvalidData { try mergeUpdatesV1(updates: [updates[0], Data(repeating: 0xff, count: 5)]) })
        #expect(isInvalidData { try diffUpdateV1(update: updates[0], stateVector: Data([0xff, 0xff])) })
        #expect(isInvalidData { try mergeUpdatesV2(updates: [updates[0]]) }, "a v1 update isn't v2")
        // Every cut of a real update: a result or an error, never a crash.
        let full = try mergeUpdatesV2(updates: try edited(v2: true).1)
        for length in 0..<full.count {
            _ = try? mergeUpdatesV2(updates: [full.prefix(length)])
            _ = try? encodeStateVectorFromUpdateV2(update: full.prefix(length))
        }
    }
}
