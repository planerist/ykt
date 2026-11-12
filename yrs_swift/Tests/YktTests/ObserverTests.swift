import Foundation
import Testing
import Ykt

/// Collects what callbacks get, from any thread.
final class Box<T>: @unchecked Sendable {
    private let lock = NSLock()
    private var items: [T] = []
    func append(_ item: T) { lock.withLock { items.append(item) } }
    var all: [T] { lock.withLock { items } }
    var count: Int { all.count }
}

struct ObserverTests {
    // MARK: - Text

    @Test func insertFormatDeleteAsDeltas() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "t")
        let deltas = Box<[YDelta]>()
        let sub = try text.observe { deltas.append($0.delta) }

        try text.insert(index: 0, chunk: "hello")
        try text.insert(index: 5, chunk: " world", attributes: #"{"bold":true}"#)
        try text.format(index: 0, length: 5, attributes: #"{"italic":true}"#)
        try text.delete(index: 1, length: 3)

        #expect(deltas.all == [
            [.yInsert(.string("hello"), nil)],
            [.yRetain(5, nil), .yInsert(.string(" world"), ["bold": .bool(true)])],
            [.yRetain(5, ["italic": .bool(true)])],
            [.yRetain(1, nil), .yDelete(3)],
        ])
        #expect(sub.isActive())
    }

    @Test func oneEventPerTransactionWithItsOrigin() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "t")
        let events = Box<YTextEvent>()
        let sub = try text.observe { events.append($0) }

        try doc.transact(origin: "editor") { txn in
            try text.insert(index: 0, chunk: "ab", txn: txn)
            try text.insert(index: 2, chunk: "cd", txn: txn)
            #expect(events.count == 0)
        }
        try text.insert(index: 0, chunk: "x")

        #expect(events.all.map(\.delta) == [[.yInsert(.string("abcd"), nil)], [.yInsert(.string("x"), nil)]])
        #expect(events.all.map(\.originString) == ["editor", nil])
        _ = sub
    }

    @Test func remoteUpdateWithItsOrigin() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        try a.getText(name: "t").insert(index: 0, chunk: "hello")
        let b = YDoc(options: YDocOptions(clientId: 2))
        let text = try b.getText(name: "t")
        let events = Box<YTextEvent>()
        let sub = try text.observe { events.append($0) }

        try applyUpdateV2(doc: b, update: try encodeStateAsUpdateV2(doc: a), origin: Data("server".utf8))

        #expect(events.all.map(\.delta) == [[.yInsert(.string("hello"), nil)]])
        #expect(events.all.first?.originString == "server")
        _ = sub
    }

    @Test func unsubscribeAndRelease() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "t")
        let first = Box<Int>()
        let second = Box<Int>()
        let firstSub = try text.observe { _ in first.append(1) }
        var secondSub: YSubscription? = try text.observe { _ in second.append(1) }

        try text.insert(index: 0, chunk: "a")
        #expect(firstSub.unsubscribe())
        #expect(!firstSub.unsubscribe())
        #expect(!firstSub.isActive())
        try text.insert(index: 1, chunk: "b")
        secondSub = nil // released: unsubscribed
        try text.insert(index: 2, chunk: "c")

        #expect(first.count == 1)
        #expect(second.count == 2)
        #expect(secondSub == nil)
    }

    @Test func subscriptionOutlivesTheDocument() throws {
        let heard = Box<Int>()
        var sub: YSubscription?
        do {
            let doc = YDoc()
            let text = try doc.getText(name: "t")
            sub = try text.observe { _ in heard.append(1) }
            try text.insert(index: 0, chunk: "a")
        }
        #expect(sub?.unsubscribe() == true)
        sub = nil
        #expect(heard.count == 1)
    }

    @Test func readingWritingSubscribingInsideACallbackThrows() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "t")
        let handle = try doc.getText(name: "t")
        let results = Box<String>()
        let sub = try text.observe { _ in
            results.append((try? handle.toText()) == nil ? "read throws" : "read worked")
            results.append((try? handle.insert(index: 0, chunk: "x")) == nil ? "write throws" : "write worked")
            results.append((try? handle.observe { _ in }) == nil ? "subscribe throws" : "subscribe worked")
        }

        try text.insert(index: 0, chunk: "a")

        #expect(results.all == ["read throws", "write throws", "subscribe throws"])
        #expect(try text.toText() == "a")
        _ = sub
    }

    @Test func unsubscribeInsideItsOwnCallback() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "t")
        let calls = Box<Int>()
        final class Holder: @unchecked Sendable { var sub: YSubscription? }
        let holder = Holder()
        holder.sub = try text.observe { _ in
            calls.append(1)
            _ = holder.sub?.unsubscribe()
        }
        let updates = Box<Int>()
        final class UpdateHolder: @unchecked Sendable { var sub: YSubscription? }
        let updateHolder = UpdateHolder()
        updateHolder.sub = try doc.observeUpdateV2 { _ in
            updates.append(1)
            _ = updateHolder.sub?.unsubscribe() // the document is locked: switched off now, removed later
        }

        try text.insert(index: 0, chunk: "a")
        try text.insert(index: 1, chunk: "b")

        #expect(calls.count == 1)
        #expect(updates.count == 1)
    }

    @Test func preliminaryTextCannotBeObserved() {
        let text = YText.newWithText(init: "prelim")
        #expect(throws: (any Error).self) { try text.observe { _ in } }
    }

    // MARK: - Document

    @Test func updatesReplayOnAnotherReplica() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        let b = YDoc(options: YDocOptions(clientId: 2))
        let updates = Box<YUpdateEvent>()
        let sub = try a.observeUpdateV2 { updates.append($0) }
        let text = try a.getText(name: "t")

        try text.insert(index: 0, chunk: "hello")
        try a.transact(origin: "editor") { txn in
            try text.insert(index: 5, chunk: " world", txn: txn)
            try text.delete(index: 0, length: 1, txn: txn)
        }
        try a.transact { _ in } // no changes: no update

        #expect(updates.all.map(\.originString) == [nil, "editor"])
        for event in updates.all { try applyUpdateV2(doc: b, update: event.update) }
        #expect(try b.getText(name: "t").toText() == "ello world")
        _ = sub
    }

    @Test func subscribingWhileATransactionIsOpenThrows() throws {
        let doc = YDoc()
        let txn = try doc.transaction()
        #expect(throws: (any Error).self) { try doc.observeUpdateV2 { _ in } }
        try txn.commit()
        _ = try doc.observeUpdateV2 { _ in }
    }

    @Test func cleanupStateVectors() throws {
        let doc = YDoc(options: YDocOptions(clientId: 7))
        let text = try doc.getText(name: "t")
        let events = Box<YTransactionCleanupEvent>()
        let sub = try doc.observeTransactionCleanup { events.append($0) }

        try text.insert(index: 0, chunk: "hello")
        try doc.transact(origin: "cut") { txn in try text.delete(index: 0, length: 2, txn: txn) }

        #expect(events.count == 2)
        #expect(events.all[0].afterState[7] == 5)
        #expect(events.all[1].beforeState[7] == 5)
        #expect(events.all[1].originString == "cut")
        #expect(events.all[1].deleteSet.count > 1)
        _ = sub
    }

    // MARK: - Threads

    /// One call into a document at a time (ykt's contract), from many threads: a lock around it.
    /// Every callback runs on the thread that committed.
    @Test func sharedDocumentBehindALock() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "t")
        let lock = NSLock()
        let callbackThreads = Box<String>()
        let sub = try text.observe { _ in
            callbackThreads.append(Thread.current.threadDictionary["worker"] as? String ?? "?")
        }
        let workers = 8
        let perWorker = 50

        DispatchQueue.concurrentPerform(iterations: workers) { worker in
            Thread.current.threadDictionary["worker"] = "worker-\(worker)"
            for _ in 0..<perWorker {
                lock.withLock { _ = try? text.insert(index: 0, chunk: "x") }
            }
            Thread.current.threadDictionary["worker"] = nil
        }

        #expect(callbackThreads.count == workers * perWorker)
        #expect(!callbackThreads.all.contains("?"), "each callback ran on a committing worker's thread")
        #expect(try text.length() == UInt32(workers * perWorker))
        _ = sub
    }

    /// Two replicas, each on a serial queue of its own (its only thread of access), edit at once and
    /// sync through their update events, as over a network: they converge. Updates applied from
    /// the other side have the origin "remote" and aren't sent back.
    @Test func twoReplicasOnTwoQueuesConverge() throws {
        final class Replica: @unchecked Sendable {
            let queue: DispatchQueue
            let doc: YDoc
            let text: YText
            var peer: Replica?
            var subs: [YSubscription] = []
            let heardRemote = Box<Int>()
            init(_ name: String, client: UInt64) throws {
                queue = DispatchQueue(label: name)
                doc = YDoc(options: YDocOptions(clientId: client))
                text = try doc.getText(name: "t")
            }
            func start() throws {
                subs.append(try doc.observeUpdateV2 { [weak self] event in
                    guard event.originString != "remote", let peer = self?.peer else { return }
                    peer.queue.async { try? applyUpdateV2(doc: peer.doc, update: event.update, origin: Data("remote".utf8)) }
                })
                subs.append(try text.observe { [weak self] event in
                    if event.originString == "remote" { self?.heardRemote.append(1) }
                })
            }
        }
        let a = try Replica("a", client: 1)
        let b = try Replica("b", client: 2)
        a.peer = b
        b.peer = a
        try a.queue.sync { try a.start() }
        try b.queue.sync { try b.start() }
        let edits = 200
        let group = DispatchGroup()
        for (replica, letter) in [(a, "a"), (b, "b")] {
            replica.queue.async(group: group) {
                for _ in 0..<edits { _ = try? replica.text.insert(index: 0, chunk: letter) }
            }
        }
        group.wait()
        // What each side sent is queued on the other: let both queues drain.
        a.queue.sync {}
        b.queue.sync {}
        a.queue.sync {}

        let textA = try a.queue.sync { try a.text.toText() }
        let textB = try b.queue.sync { try b.text.toText() }
        #expect(textA == textB)
        #expect(textA.count == 2 * edits)
        #expect(a.heardRemote.count > 0 && b.heardRemote.count > 0)
        a.queue.sync { a.subs = [] }
        b.queue.sync { b.subs = [] }
    }
}
