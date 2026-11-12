import Testing
import Ykt

struct YDocTests {
    @Test func snapshotState() throws {
        let d1 = YDoc(options: YDocOptions(clientId: 1, gc: false))

        let text = try d1.getText(name: "text")
        try text.insert(index: 0, chunk: "hello")

        let prev = try snapshot(doc: d1)
        try text.insert(index: 5, chunk: " world")

        let state = try encodeStateFromSnapshotV2(doc: d1, snapshot: prev)

        let d2 = YDoc(options: YDocOptions(clientId: 2))
        let txt2 = try d2.getText(name: "text")
        try applyUpdateV2(doc: d2, update: state)

        #expect(try txt2.toText() == "hello")
    }

    @Test func stateAsUpdateDefault() throws {
        let d1 = YDoc(options: YDocOptions(clientId: 1, gc: false))

        let text = try d1.getText(name: "text")
        try text.insert(index: 0, chunk: "hello")

        let state = try encodeStateAsUpdateV2(doc: d1)

        let d2 = YDoc(options: YDocOptions(clientId: 2))
        try applyUpdateV2(doc: d2, update: state)

        let txt2 = try d2.getText(name: "text")
        #expect(try txt2.toText() == "hello")
    }

    @Test func delta() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "text")
        try text.insert(index: 0, chunk: "hello")
        let prev = try snapshot(doc: doc)
        try text.insert(index: 5, chunk: " world")
        let next = try snapshot(doc: doc)

        let delta = try text.toDelta(snapshot: next, prevSnapshot: prev)
        #expect(delta == [
            .yInsert(.string("hello"), nil),
            .yInsert(.string(" world"), nil),
        ])
    }

    @Test func deltaWithAttrs() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "text")
        try text.insert(index: 0, chunk: "hello")
        let prev = try snapshot(doc: doc)
        try text.insert(index: 5, chunk: " world", attributes: "{\"bold\":true}")
        let next = try snapshot(doc: doc)

        let delta = try text.toDelta(snapshot: next, prevSnapshot: prev)
        #expect(delta == [
            .yInsert(.string("hello"), nil),
            .yInsert(.string(" world"), ["bold": .bool(true)]),
        ])
    }

    @Test func deltaApplyDelta() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "text")
        try text.insert(index: 0, chunk: "hello")
        try text.insert(index: 5, chunk: " world", attributes: "{\"bold\":true}")

        try text.applyDelta(delta: [
            .yRetain(11, nil),
            .yInsert(.string("after"), nil),
        ])

        #expect(try text.toDelta() == [
            .yInsert(.string("hello"), nil),
            .yInsert(.string(" world"), ["bold": .bool(true)]),
            .yInsert(.string("after"), nil),
        ])
    }

    @Test func deltaApplyDelta2() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "text")
        try text.insert(index: 0, chunk: "hello")
        try text.insert(index: 5, chunk: " world", attributes: "{\"bold\":true}")

        try text.applyDelta(delta: [
            .yRetain(5, nil),
            .yRetain(2, ["italic": .bool(true)]),
        ])

        #expect(try text.toDelta() == [
            .yInsert(.string("hello"), nil),
            .yInsert(.string(" w"), ["bold": .bool(true), "italic": .bool(true)]),
            .yInsert(.string("orld"), ["bold": .bool(true)]),
        ])
    }

    @Test func deltaApplyDelta3() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "text")
        try text.applyDelta(delta: [
            .yInsert(.string("hello"), nil),
            .yInsert(.string("\n"), nil),
        ])
        #expect(try text.length() == 6)

        try text.applyDelta(delta: [
            .yRetain(6, nil),
            .yInsert(.string("hello2"), nil),
            .yInsert(.string("\n"), nil),
        ])
        #expect(try text.length() == 13)

        #expect(try text.toDelta() == [
            .yInsert(.string("hello\nhello2\n"), nil),
        ])
    }

    @Test func offsetKind() throws {
        let doc = YDoc(options: YDocOptions(clientId: 1, gc: false))
        let text = try doc.getText(name: "text")
        try text.applyDelta(delta: [.yInsert(.string(" – "), nil)])
        #expect(try text.length() == 3)

        let doc2 = YDoc(options: YDocOptions(clientId: 1, gc: false, bytesOffset: true))
        let text2 = try doc2.getText(name: "text")
        try text2.applyDelta(delta: [.yInsert(.string(" – "), nil)])
        #expect(try text2.length() == 5)
    }

    @Test func toText() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "text")
        try text.insert(index: 0, chunk: "hello")
        try text.insert(index: 5, chunk: " world", attributes: "{\"bold\":true}")

        #expect(try text.toText() == "hello world")
    }

    @Test func invalidOp() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "text")
        try text.insert(index: 0, chunk: "hello")

        // Past the end: nothing is deleted (yrs used to delete what there was, then panic).
        expectYError(YError.OutOfBounds) { try text.delete(index: 4, length: 3) }
        #expect(try text.toText() == "hello")

        try text.insert(index: 5, chunk: " world")
        #expect(try text.toText() == "hello world")
    }
}
