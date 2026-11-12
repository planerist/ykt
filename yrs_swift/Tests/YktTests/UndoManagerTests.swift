import Foundation
import Testing
import Ykt

/// The undo manager with the web editor's setup (tracked origins = the editor's, so remote edits
/// are never undone), redo, the capture timeout, clearing, origins, stack item meta, what throws
/// inside a transaction or a callback, releasing it, and threads behind a lock.
struct UndoManagerTests {
    private func manager(_ doc: YDoc, _ text: YText, timeout: UInt64 = 0, origins: [String] = ["editor"]) throws -> YUndoManager {
        try YUndoManager(doc: doc, scope: text, options: YUndoOptions(captureTimeoutMillis: timeout, trackedOrigins: origins))
    }

    private func insert(_ doc: YDoc, _ text: YText, _ origin: String, _ index: UInt32, _ chunk: String) throws {
        try doc.transact(origin: origin) { txn in try text.insert(index: index, chunk: chunk, txn: txn) }
    }

    /// What `remote` has that `local` lacks, applied to `local` with `origin`.
    private func receive(_ local: YDoc, from remote: YDoc, origin: String? = nil) throws {
        let missing = try diffUpdateV2(update: encodeStateAsUpdateV2(doc: remote), stateVector: encodeStateVector(doc: local))
        try applyUpdateV2(doc: local, update: missing, origin: origin.map { Data($0.utf8) })
    }

    @Test func undoTakesBackTheEditorsChangesAndKeepsTheRemoteOnes() throws {
        let local = YDoc(options: YDocOptions(clientId: 1))
        let text = local.getText(name: "t")
        let undo = try manager(local, text)
        let remote = YDoc(options: YDocOptions(clientId: 2))

        try insert(local, text, "editor", 0, "hello")
        try receive(remote, from: local)
        try remote.getText(name: "t").insert(index: 5, chunk: " world")
        try remote.getText(name: "t").insert(index: 2, chunk: "~") // inside the editor's "hello"
        try receive(local, from: remote, origin: "remote")
        try insert(local, text, "editor", 0, "> ")
        #expect(try text.toText() == "> he~llo world")

        #expect(try undo.undo())
        #expect(try text.toText() == "he~llo world")
        #expect(try undo.undo())
        #expect(try text.toText() == "~ world")
        #expect(try !undo.canUndo())
        #expect(try !undo.undo())

        #expect(try undo.redo())
        #expect(try undo.redo())
        #expect(try text.toText() == "> he~llo world")
        #expect(try !undo.canRedo())
    }

    @Test func captureTimeoutMergesAndStopCapturingSplits() throws {
        let doc = YDoc()
        let text = doc.getText(name: "t")
        let undo = try manager(doc, text, timeout: 3_600_000)
        try insert(doc, text, "editor", 0, "a")
        try insert(doc, text, "editor", 1, "b")
        try undo.stopCapturing()
        try insert(doc, text, "editor", 2, "c")
        try insert(doc, text, "editor", 3, "d")

        #expect(try undo.undo())
        #expect(try text.toText() == "ab")
        #expect(try undo.undo())
        #expect(try text.toText() == "")
    }

    @Test func clearAndUntrackedOrigins() throws {
        let doc = YDoc()
        let text = doc.getText(name: "t")
        let undo = try manager(doc, text)
        try insert(doc, text, "other", 0, "a")
        try text.insert(index: 1, chunk: "b") // no origin
        #expect(try !undo.canUndo())

        try undo.addTrackedOrigin(origin: "other")
        try insert(doc, text, "other", 2, "c")
        try undo.removeTrackedOrigin(origin: "editor")
        try insert(doc, text, "editor", 3, "d")
        #expect(try undo.canUndo())
        try undo.clear()
        #expect(try !undo.canUndo())
        #expect(try !undo.undo())
        #expect(try text.toText() == "abcd")
    }

    @Test func defaultOptionsTrackChangesWithoutAnOrigin() throws {
        let doc = YDoc()
        let text = doc.getText(name: "t")
        let undo = try YUndoManager(doc: doc, scope: text)
        try text.insert(index: 0, chunk: "a")
        try insert(doc, text, "editor", 1, "b")

        #expect(try undo.undo())
        #expect(try text.toText() == "b")
    }

    @Test func metaComesBackWhenTheItemIsPopped() throws {
        let doc = YDoc()
        let text = doc.getText(name: "t")
        let undo = try manager(doc, text)
        final class Selection: @unchecked Sendable { var at: Double = 0 }
        let selection = Selection()
        let added = Box<YUndoEvent>()
        let popped = Box<YUndoEvent>()
        let addedSub = try undo.observeItemAdded { event in
            added.append(event)
            return ["selection": .number(selection.at)]
        }
        let poppedSub = try undo.observeItemPopped { popped.append($0) }

        selection.at = 1
        try insert(doc, text, "editor", 0, "a")
        selection.at = 9
        _ = try undo.undo()
        _ = try undo.redo()

        let own = undo.origin()
        #expect(added.all.map(\.kind) == [.undo, .redo, .undo])
        #expect(added.all.first?.originString == "editor")
        #expect(added.all.first?.meta == [:])
        #expect(popped.all == [
            YUndoEvent(kind: .undo, origin: own, meta: ["selection": .number(1)]),
            YUndoEvent(kind: .redo, origin: own, meta: ["selection": .number(9)]),
        ])
        _ = (addedSub, poppedSub)
    }

    @Test func anExtendedItemCanChangeItsMeta() throws {
        let doc = YDoc()
        let text = doc.getText(name: "t")
        let undo = try manager(doc, text, timeout: 3_600_000)
        let popped = Box<[String: YValue]>()
        let subs = [
            try undo.observeItemAdded { _ in ["n": .number(1)] },
            try undo.observeItemUpdated { event in event.meta.merging(["n": .number(2)]) { $1 } },
            try undo.observeItemPopped { popped.append($0.meta) },
        ]
        try insert(doc, text, "editor", 0, "a")
        try insert(doc, text, "editor", 1, "b")
        _ = try undo.undo()

        #expect(popped.all == [["n": .number(2)]])
        _ = subs
    }

    @Test func undoTransactionsCarryTheManagersOrigin() throws {
        let doc = YDoc()
        let text = doc.getText(name: "t")
        let undo = try manager(doc, text)
        let origins = Box<Data?>()
        let sub = try text.observe { origins.append($0.origin) }

        try insert(doc, text, "editor", 0, "a")
        _ = try undo.undo()

        #expect(origins.all == [Data("editor".utf8), undo.origin()])
        _ = sub
    }

    @Test func insideATransactionOrACallbackItThrows() throws {
        let doc = YDoc()
        let text = doc.getText(name: "t")
        let undo = try manager(doc, text)
        try insert(doc, text, "editor", 0, "a")

        try doc.transact(origin: "editor") { (_: YTransaction) throws in
            #expect(throws: YError.AnotherTx) { try undo.undo() }
            #expect(throws: YError.AnotherTx) { try undo.redo() }
            #expect(throws: YError.AnotherTx) { try undo.clear() }
            #expect(try undo.canUndo())
        }

        let results = Box<String>()
        let textSub = try text.observe { _ in
            results.append((try? undo.undo()) == nil ? "undo throws in a text observer" : "undo worked")
        }
        final class Weak: @unchecked Sendable { weak var undo: YUndoManager? }
        let weak = Weak()
        weak.undo = undo
        let poppedSub = try undo.observeItemPopped { _ in
            results.append((try? weak.undo?.canRedo()) == nil ? "the manager throws in its callback" : "it worked")
        }
        try insert(doc, text, "editor", 1, "b")
        _ = textSub.unsubscribe()
        #expect(try undo.undo())

        #expect(results.all == ["undo throws in a text observer", "the manager throws in its callback"])
        #expect(try text.toText() == "a")
        _ = poppedSub
    }

    @Test func releasingTheManagerIsSafeAnywhere() throws {
        let doc = YDoc()
        let text = doc.getText(name: "t")
        var undo: YUndoManager? = try manager(doc, text)
        let sub = try undo!.observeItemAdded { _ in nil }
        try insert(doc, text, "editor", 0, "a")

        try doc.transact(origin: "editor") { txn in
            try text.insert(index: 1, chunk: "b", txn: txn)
            undo = nil // the document is locked: the manager stays behind, harmlessly
        }
        try insert(doc, text, "editor", 2, "c")

        #expect(try text.toText() == "abc")
        #expect(sub.unsubscribe(), "a subscription may outlive its manager")
    }

    /// One call at a time (ykt's contract), from many threads: a lock around the document and its
    /// manager.
    @Test func sharedBehindALock() throws {
        let doc = YDoc()
        let text = doc.getText(name: "t")
        let undo = try manager(doc, text)
        let lock = NSLock()
        let workers = 8
        let perWorker = 20

        DispatchQueue.concurrentPerform(iterations: workers) { _ in
            for _ in 0..<perWorker {
                lock.withLock { _ = try? self.insert(doc, text, "editor", 0, "x") }
            }
        }

        var undone = 0
        while try lock.withLock({ try undo.undo() }) { undone += 1 }
        #expect(undone == workers * perWorker)
        #expect(try text.toText() == "")
    }
}
