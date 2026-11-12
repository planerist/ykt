import Foundation
import Testing
import Ykt

/// Relative positions: assoc, local and remote edits, the start and the end, encoding (Yjs's bytes)
/// and other replicas, transactions, bytes that aren't a position.
struct StickyIndexTests {
    private func textWith(_ doc: YDoc, _ content: String) throws -> YText {
        let text = try doc.getText(name: "t")
        try text.insert(index: 0, chunk: content)
        return text
    }

    private func sync(_ from: YDoc, _ to: YDoc) throws {
        try applyUpdateV2(doc: to, update: try encodeStateAsUpdateV2(doc: from))
    }

    @Test func afterMovesOnWithAnInsertAtItsSpotAndBeforeStays() throws {
        let doc = YDoc(options: YDocOptions(clientId: 1))
        let text = try textWith(doc, "abc")
        let before = try text.stickyIndex(index: 1, assoc: .before)
        let after = try text.stickyIndex(index: 1, assoc: .after)

        try text.insert(index: 1, chunk: "XY")

        #expect(try before.offset(doc: doc) == YStickyOffset(index: 1, assoc: .before))
        #expect(try after.offset(doc: doc) == YStickyOffset(index: 3, assoc: .after))
    }

    @Test func localAndRemoteEdits() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        let aText = try textWith(a, "hello world")
        let b = YDoc(options: YDocOptions(clientId: 2))
        try sync(a, b)
        let w = try b.getText(name: "t").stickyIndex(index: 6, assoc: .after) // before "w", made on B

        try aText.insert(index: 0, chunk: ">> ")
        try aText.delete(index: 3, length: 5) // ">>  world"
        try sync(a, b)
        #expect(try w.offset(doc: b)?.index == 4)

        try b.getText(name: "t").delete(index: 0, length: 2) // "  world"
        #expect(try w.offset(doc: b)?.index == 2)
        try b.getText(name: "t").delete(index: 1, length: 4) // " ld": its neighbour is gone
        #expect(try w.offset(doc: b)?.index == 1)
    }

    @Test func theStartAndTheEnd() throws {
        let doc = YDoc(options: YDocOptions(clientId: 1))
        let text = try doc.getText(name: "t")
        let start = try text.stickyIndex(index: 0, assoc: .before)
        let end = try text.stickyIndex(index: 0, assoc: .after) // an empty text: the end

        try text.insert(index: 0, chunk: "abc")
        let beforeC = try text.stickyIndex(index: 3, assoc: .before)
        try text.insert(index: 3, chunk: "de")
        try text.insert(index: 0, chunk: "X")

        #expect(try start.offset(doc: doc)?.index == 0)
        #expect(try end.offset(doc: doc)?.index == 6)
        #expect(try beforeC.offset(doc: doc)?.index == 4)
        expectYError(YError.OutOfBounds) { try text.stickyIndex(index: 7, assoc: .after) }
    }

    @Test func utf16Offsets() throws {
        let doc = YDoc()
        let text = try textWith(doc, "😀ab")
        let b = try text.stickyIndex(index: 3, assoc: .after)
        try text.insert(index: 0, chunk: "😀")
        #expect(try b.offset(doc: doc)?.index == 5)
    }

    @Test func encodedAsYjsAndReadOnAnotherReplica() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        let text = try textWith(a, "abc")
        let b = try text.stickyIndex(index: 1, assoc: .after)
        #expect(b.encode() == Data([0, 1, 1, 0]), "Y.encodeRelativePosition's bytes")
        #expect(try text.stickyIndex(index: 3, assoc: .after).encode() == Data([1, 1, UInt8(ascii: "t"), 0]))

        let received = try YStickyIndex.decode(bytes: b.encode())
        #expect(received == b)
        #expect(received.assoc() == .after)
        let replica = YDoc(options: YDocOptions(clientId: 2))
        #expect(try received.offset(doc: replica) == nil, "not there yet")
        try sync(a, replica)
        #expect(try received.offset(doc: replica)?.index == 1)
        #expect(received.description == "YStickyIndex(after 1#1)")
    }

    @Test func garbageThrows() {
        for bytes in [Data(), Data(repeating: 0xff, count: 6), Data([9, 1, 1, 0]), Data([0, 1])] {
            #expect(throws: YError.self) { try YStickyIndex.decode(bytes: bytes) }
        }
        let prelim = YText.newWithText(init: "abc")
        expectYError(YError.NotIntegrated) { try prelim.stickyIndex(index: 0, assoc: .after) }
    }

    @Test func insideATransaction() throws {
        let doc = YDoc()
        let text = try textWith(doc, "abc")
        let c = try text.stickyIndex(index: 2, assoc: .after)

        try doc.transact { txn in
            try text.insert(index: 0, chunk: "XY", txn: txn)
            #expect(try c.offset(doc: doc, txn: txn)?.index == 4)
            #expect(try text.stickyIndex(index: 2, assoc: .after, txn: txn).offset(doc: doc, txn: txn)?.index == 2)
            expectYError(YError.Locked) { try c.offset(doc: doc) }
        }
        #expect(try c.offset(doc: doc)?.index == 4)
    }
}
