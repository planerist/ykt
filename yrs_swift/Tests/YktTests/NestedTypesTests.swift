import Foundation
import Testing
import Ykt

/// Shared types nested in maps, arrays and texts: handles from `setText` / `setMap` / `setArray`,
/// `insertText` / `insertMap`, `insertEmbedMap`, read back with `getOutput`; edits replicating;
/// observers reporting handles; ids; undo over a map.
struct NestedTypesTests {
    private func sync(_ from: YDoc, _ to: YDoc) throws {
        let missing = try diffUpdateV2(update: encodeStateAsUpdateV2(doc: from), stateVector: encodeStateVector(doc: to))
        try applyUpdateV2(doc: to, update: missing)
    }

    @Test func mapHandlesReplicateBothWays() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        let root = try a.getMap(name: "root")
        let title = try root.setText(key: "title", text: "Hello")
        let meta = try root.setMap(key: "meta", entries: ["pinned": .bool(true)])
        let tags = try root.setArray(key: "tags", values: [.string("work")])
        try title.insert(index: 5, chunk: ", world")
        try meta.set(key: "color", value: .string("red"))
        try tags.push(values: [.string("home")])

        let b = YDoc(options: YDocOptions(clientId: 2))
        try sync(a, b)
        let replica = try b.getMap(name: "root")
        let bTitle = try #require(try replica.getOutput(key: "title")?.asText)
        #expect(try bTitle.toText() == "Hello, world")
        #expect(try replica.getOutput(key: "meta")?.asMap?.toJson() == ["pinned": .bool(true), "color": .string("red")])
        #expect(try replica.getOutput(key: "tags")?.asArray?.toJson() == [.string("work"), .string("home")])
        #expect(try replica.get(key: "title") == .string("Hello, world"), "the plain API gives JSON")

        try bTitle.insert(index: 0, chunk: "> ")
        try sync(b, a)
        #expect(try title.toText() == "> Hello, world")
        #expect(try root.getOutput(key: "title") == .text(text: title), "handles of one text are equal")
        #expect(try bTitle.id() == title.id())
    }

    @Test func arrayHandles() throws {
        let doc = YDoc()
        let list = try doc.getArray(name: "list")
        let text = try list.insertText(index: 0, text: "first")
        let map = try list.insertMap(index: 1)
        try map.set(key: "k", value: .number(1))
        expectYError(YError.OutOfBounds) { try list.insertArray(index: 5) }
        #expect(try list.getOutput(index: 0).asText == text)
        #expect(try list.getOutput(index: 1).asMap?.toJson() == ["k": .number(1)])
        #expect(try list.toJson() == [.string("first"), .attrMap(["k": .number(1)])])
    }

    @Test func observersReportHandles() throws {
        let doc = YDoc()
        let root = try doc.getMap(name: "root")
        let events = Box<YMapEvent>()
        let sub = try root.observe { events.append($0) }
        let meta = try root.setMap(key: "meta")
        try root.set(key: "meta", value: .string("replaced"))
        #expect(events.all.map(\.keys) == [
            ["meta": .inserted(value: .map(map: meta))],
            ["meta": .updated(oldValue: .map(map: meta), newValue: .value(value: .string("replaced")))],
        ])
        expectYError(YError.TypeDeleted) { try meta.set(key: "k", value: .null) }
        _ = sub
    }

    @Test func embeddedMapInAText() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "t")
        try text.insert(index: 0, chunk: "ab")
        let table = try text.insertEmbedMap(index: 1, entries: ["rows": .number(2)])
        try table.set(key: "cols", value: .number(3))
        #expect(try text.toDelta() == [
            .yInsert(.string("a"), nil), .yInsert(.attrMap(["rows": .number(2), "cols": .number(3)]), nil), .yInsert(.string("b"), nil),
        ])
    }

    @Test func undoOverAMap() throws {
        let doc = YDoc()
        let root = try doc.getMap(name: "root")
        let undo = try YUndoManager.forMap(doc: doc, scope: root, options: YUndoOptions(captureTimeoutMillis: 0, trackedOrigins: ["editor"]))
        try doc.transact(origin: "editor") { txn in try root.set(key: "title", value: .string("Draft"), txn: txn) }
        let body = try root.setText(key: "body") // untracked
        try doc.transact(origin: "editor") { txn in try body.insert(index: 0, chunk: "hello", txn: txn) }

        #expect(try undo.undo())
        #expect(try body.toText() == "")
        #expect(try undo.undo())
        #expect(try root.get(key: "title") == nil)
        #expect(try root.containsKey(key: "body"))
        #expect(try undo.redo())
        #expect(try root.get(key: "title") == .string("Draft"))
    }
}
