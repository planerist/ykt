import Foundation
import Testing
import Ykt

/// Typed attributes, embeds of every value kind and the text's id: round trips through `toDelta`, an
/// observer and a replica; removal by `.null`; preliminary texts.
struct AttributesTests {
    private func replica(of doc: YDoc) throws -> YDoc {
        let replica = YDoc(options: YDocOptions(clientId: 99))
        try applyUpdateV2(doc: replica, update: try encodeStateAsUpdateV2(doc: doc))
        return replica
    }

    private let rich: [String: YValue] = [
        "bold": .bool(true),
        "color": .string("#ff0000"),
        "size": .number(14),
        "link": .attrMap(["href": .string("https://yaranga.net")]),
        "tags": .array([.string("a"), .null]),
    ]

    @Test func roundTripThroughDeltaObserverAndReplica() throws {
        let doc = YDoc(options: YDocOptions(clientId: 1))
        let text = try doc.getText(name: "t")
        let deltas = Box<[YDelta]>()
        let sub = try text.observe { deltas.append($0.delta) }

        try text.insertWithAttributes(index: 0, chunk: "hello", attributes: rich)
        try text.insertWithAttributes(index: 5, chunk: " world", attributes: [:]) // plain, not the formatting before it
        try text.formatWithAttributes(index: 6, length: 5, attributes: ["italic": .bool(true)])

        let expected: [YDelta] = [.yInsert(.string("hello"), rich), .yInsert(.string(" "), nil), .yInsert(.string("world"), ["italic": .bool(true)])]
        #expect(try text.toDelta() == expected)
        #expect(try replica(of: doc).getText(name: "t").toDelta() == expected)
        #expect(deltas.all == [
            [.yInsert(.string("hello"), rich)],
            [.yRetain(5, nil), .yInsert(.string(" world"), nil)],
            [.yRetain(6, nil), .yRetain(5, ["italic": .bool(true)])],
        ])
        _ = sub
    }

    @Test func nullRemovesAnAttribute() throws {
        let text = try YDoc().getText(name: "t")
        try text.insertWithAttributes(index: 0, chunk: "hello", attributes: ["bold": .bool(true), "italic": .bool(true)])
        try text.formatWithAttributes(index: 0, length: 2, attributes: ["bold": .null])
        #expect(try text.toDelta() == [
            .yInsert(.string("he"), ["italic": .bool(true)]),
            .yInsert(.string("llo"), ["bold": .bool(true), "italic": .bool(true)]),
        ])
    }

    @Test func embedsOfEveryKind() throws {
        let every: [YValue] = [
            .null, .bool(false), .number(42), .number(-1.5), .bigInt((1 << 60) + 1), .string("embedded"),
            .buffer(Data([0, 1, 255])), .array([.number(1), .string("two")]), .attrMap(["mention": .string("note://1")]),
        ]
        let doc = YDoc(options: YDocOptions(clientId: 1))
        let text = try doc.getText(name: "t")
        try text.insert(index: 0, chunk: "ab")
        for (i, value) in every.enumerated() {
            try text.insertEmbed(index: UInt32(1 + i), embed: value)
        }
        try text.insertEmbed(index: 1, embed: .bool(true), attributes: ["bold": .bool(true)])

        let expected: [YDelta] = [.yInsert(.string("a"), nil), .yInsert(.bool(true), ["bold": .bool(true)])]
            + every.map { .yInsert($0, nil) } + [.yInsert(.string("b"), nil)]
        #expect(try text.length() == UInt32(2 + every.count + 1))
        #expect(try text.toDelta() == expected)
        #expect(try replica(of: doc).getText(name: "t").toDelta() == expected)
    }

    @Test func pastTheEndAndPreliminary() throws {
        let text = try YDoc().getText(name: "t")
        try text.insert(index: 0, chunk: "abc")
        expectYError(YError.OutOfBounds) { try text.insertWithAttributes(index: 4, chunk: "x", attributes: rich) }
        expectYError(YError.OutOfBounds) { try text.insertEmbed(index: 4, embed: .null) }
        let prelim = YText.newWithText(init: "abc")
        expectYError(YError.NotIntegrated) { try prelim.insertWithAttributes(index: 0, chunk: "x", attributes: rich) }
        expectYError(YError.NotIntegrated) { try prelim.formatWithAttributes(index: 0, length: 1, attributes: rich) }
        expectYError(YError.NotIntegrated) { try prelim.insertEmbed(index: 0, embed: .null) }
        expectYError(YError.NotIntegrated) { try prelim.id() }
    }

    @Test func rootTextIdIsItsName() throws {
        let doc = YDoc(options: YDocOptions(clientId: 1))
        let text = try doc.getText(name: "lines")
        try text.insert(index: 0, chunk: "x")
        #expect(try text.id() == .root(name: "lines"))
        #expect(try replica(of: doc).getText(name: "lines").id() == text.id())
    }
}
