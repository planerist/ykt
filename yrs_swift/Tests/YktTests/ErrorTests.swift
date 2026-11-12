import Foundation
import Testing
import Ykt

/// `YError`, one case at a time: where each one is thrown, that nothing changed, and its message.
/// Also what used to crash or misbehave: an index past the end, a delta deleting past the end, a
/// handle used inside its own callback, a transaction used after its commit or of another document.
struct ErrorTests {
    private func hello(gc: Bool = true) throws -> (YDoc, YText) {
        let doc = YDoc(options: YDocOptions(clientId: 1, gc: gc))
        let text = try doc.getText(name: "t")
        try text.insert(index: 0, chunk: "hello")
        return (doc, text)
    }

    @Test func lockedWhileATransactionIsOpen() throws {
        let (doc, text) = try hello()
        try doc.transact { _ in
            expectYError(YError.Locked) { try text.toText() }
            expectYError(YError.Locked) { try doc.transaction() }
            expectYError(YError.Locked) { try snapshot(doc: doc) }
            expectYError(YError.Locked) { try encodeStateVector(doc: doc) }
        }
        #expect(try text.toText() == "hello")
    }

    @Test func theSameHandleInsideItsOwnCallbackIsLocked() throws {
        let (_, text) = try hello()
        let results = Box<String>()
        let sub = try text.observe { _ in
            for body in [{ _ = try text.toText() }, { try text.insert(index: 0, chunk: "x") }] as [() throws -> Void] {
                do { try body(); results.append("worked") } catch { results.append(caseName(error as! YError)) }
            }
        }
        try text.insert(index: 5, chunk: "!")
        #expect(results.all == ["Locked", "Locked"])
        #expect(try text.toText() == "hello!")
        _ = sub
    }

    @Test func aCommittedTransactionThrowsEverywhere() throws {
        let (doc, text) = try hello()
        let txn = try doc.transaction()
        try txn.commit()
        expectYError(YError.TransactionCommitted) { try txn.commit() }
        expectYError(YError.TransactionCommitted) { try txn.beforeState() }
        expectYError(YError.TransactionCommitted) { try txn.origin() }
        expectYError(YError.TransactionCommitted) { try txn.stateVectorV1() }
        expectYError(YError.TransactionCommitted) { try txn.encodeUpdateV2() }
        expectYError(YError.TransactionCommitted) { try text.alive(txn: txn) }
        expectYError(YError.TransactionCommitted) { try text.insert(index: 0, chunk: "x", txn: txn) }
    }

    @Test func anotherDocumentsTransactionIsTheWrongDocument() throws {
        let (a, text) = try hello()
        let b = YDoc(options: YDocOptions(clientId: 2))
        let bText = try b.getText(name: "t")
        try b.transact { txn in
            expectYError(YError.WrongDocument) { try text.insert(index: 0, chunk: "x", txn: txn) }
            expectYError(YError.WrongDocument) { try a.getMap(name: "m").set(key: "k", value: .null, txn: txn) }
        }
        expectYError(YError.WrongDocument) { try YUndoManager(doc: a, scope: bText) }
        #expect(try bText.toText() == "")
        #expect(try text.toText() == "hello")
    }

    @Test func aDeletedNodeIsTypeDeleted() throws {
        let doc = YDoc()
        let fragment = try doc.getXmlFragment(name: "x")
        let element = try YXmlElement(name: "p")
        try fragment.push(xmlNode: .element(element))
        try fragment.delete(index: 0, length: nil)
        expectYError(YError.TypeDeleted) { try element.length() }
        let alive = try doc.transact { txn in try element.alive(txn: txn) }
        #expect(!alive)
    }

    @Test func preliminaryAndAlreadyIntegrated() throws {
        let prelim = YText.newWithText(init: "abc")
        expectYError(YError.NotIntegrated) { try prelim.format(index: 0, length: 1, attributes: #"{"bold":true}"#) }
        expectYError(YError.NotIntegrated) { try prelim.toDelta() }
        expectYError(YError.NotIntegrated) { try prelim.observe { _ in } }

        let doc = YDoc()
        let fragment = try doc.getXmlFragment(name: "x")
        let element = try YXmlElement(name: "p")
        try fragment.push(xmlNode: .element(element))
        expectYError(YError.AlreadyIntegrated) { try fragment.push(xmlNode: .element(element)) }
        #expect(try fragment.length() == 1)
    }

    @Test func pastTheEndIsOutOfBoundsAndChangesNothing() throws {
        let (_, text) = try hello()
        expectYError(YError.OutOfBounds) { try text.insert(index: 6, chunk: "x") }
        expectYError(YError.OutOfBounds) { try text.delete(index: 3, length: 3) }
        expectYError(YError.OutOfBounds) { try text.format(index: 4, length: 2, attributes: #"{"bold":true}"#) }
        expectYError(YError.OutOfBounds) { try text.applyDelta(delta: [.yRetain(2, nil), .yDelete(6)]) }
        #expect(try text.toText() == "hello")

        let prelim = YText.newWithText(init: "😀a")
        #expect(try prelim.length() == 3, "UTF-16 units, as an integrated text")
        expectYError(YError.OutOfBounds) { try prelim.insert(index: 1, chunk: "x") }

        let fragment = try YDoc().getXmlFragment(name: "x")
        expectYError(YError.OutOfBounds) { try fragment.insert(index: 1, xmlNode: .element(YXmlElement(name: "p"))) }
        expectYError(YError.OutOfBounds) { try fragment.delete(index: 0, length: nil) }
    }

    @Test func attributesThatAreNotAnObjectAreInvalid() throws {
        let (_, text) = try hello()
        for bad in ["[1]", "not json"] {
            expectYError(YError.InvalidAttributes) { try text.insert(index: 0, chunk: "x", attributes: bad) }
        }
        #expect(try text.toText() == "hello")
    }

    @Test func malformedBytesAreInvalidData() throws {
        let (doc, _) = try hello()
        let update = try encodeStateAsUpdateV2(doc: doc)
        for cut in 1..<update.count {
            do { try applyUpdateV2(doc: YDoc(), update: update.prefix(cut)) } catch { #expect(caseName(error as! YError) == "InvalidData") }
        }
        expectYError(YError.InvalidData) { try applyUpdate(doc: doc, update: Data(repeating: 0xff, count: 9)) }
        expectYError(YError.InvalidData) { try decodeSnapshotV1(snapshot: Data(repeating: 0xff, count: 9)) }
    }

    @Test func snapshotsOfADocumentWithGcAreGcEnabled() throws {
        let (doc, text) = try hello()
        let before = try snapshot(doc: doc)
        try text.insert(index: 5, chunk: "!")
        expectYError(YError.GcEnabled) { try encodeStateFromSnapshotV2(doc: doc, snapshot: before) }
    }

    @Test func messagesSayWhatHappened() throws {
        let (doc, text) = try hello()
        try doc.transact { _ in
            do {
                _ = try text.toText()
            } catch let error as YError {
                guard case .Locked(let message) = error else { Issue.record("\(error)"); return }
                #expect(message == "the document is locked: a transaction is open or a callback is running")
                #expect(error.localizedDescription.contains(message))
            }
        }
    }
}
