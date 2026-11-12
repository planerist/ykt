import Foundation
import Testing
import Ykt

struct YXmlTests {
    @Test func snapshotState() throws {
        let d1 = YDoc(options: YDocOptions(clientId: 1, gc: false))

        let xml = try d1.getXmlFragment(name: "xml")
        try d1.transact { txn in
            let p = try YXmlElement(name: "p", attributes: [:], children: [.text(YXmlText(text: "hello", attributes: [:]))])
            try xml.push(xmlNode: .element(p), txn: txn)
            try xml.push(xmlNode: .text(YXmlText(text: "world", attributes: [:])), txn: txn)
        }

        #expect(try xml.toText() == "<p>hello</p>world")
    }

    @Test func attributes() throws {
        let doc = YDoc(options: YDocOptions(clientId: 1, gc: false))
        let root = try doc.getXmlFragment(name: "test")
        let xml = try YXmlElement(name: "div", attributes: [:], children: [])

        try root.push(xmlNode: .element(xml))

        let actual = try doc.transact { txn in
            try xml.setAttribute(name: "key1", value: .string("value1"), txn: txn)
            try xml.setAttribute(name: "key2", value: .bigInt(42), txn: txn)
            try xml.setAttribute(name: "key3", value: .bool(false), txn: txn)

            return try xml.attributes(txn: txn)
        }

        #expect(actual == [
            "key1": .string("value1"),
            "key2": .number(42),
            "key3": .bool(false),
        ])

        try doc.transact { txn in
            try xml.removeAttribute(name: "key1", txn: txn)

            #expect(try xml.getAttribute(name: "key1", txn: txn) == nil)
            #expect(try xml.getAttribute(name: "key2", txn: txn) == .number(42))
            #expect(try xml.getAttribute(name: "key3", txn: txn) == .bool(false))
        }
    }

    @Test func integerAttributes() throws {
        let doc = YDoc(options: YDocOptions(clientId: 1, gc: false))
        let root = try doc.getXmlFragment(name: "test")
        let xml = try YXmlElement(name: "div", attributes: [:], children: [])

        try root.push(xmlNode: .element(xml))

        let maxSafe: Int64 = (1 << 53) - 1
        let bigInt: Int64 = 1 << 60
        let actual = try doc.transact { txn in
            try xml.setAttribute(name: "safe", value: .bigInt(maxSafe), txn: txn)
            try xml.setAttribute(name: "big", value: .bigInt(bigInt), txn: txn)
            try xml.setAttribute(name: "negBig", value: .bigInt(-bigInt), txn: txn)
            return try xml.attributes(txn: txn)
        }

        #expect(actual == [
            "safe": .number(Double(maxSafe)),
            "big": .bigInt(bigInt),
            "negBig": .bigInt(-bigInt),
        ])
    }

    @Test func siblings() throws {
        let d1 = YDoc(options: YDocOptions(clientId: 1, gc: false))
        let root = try d1.getXmlFragment(name: "test")

        let first = try d1.transact { txn in
            let p = try YXmlElement(name: "p", attributes: [:], children: [.text(YXmlText(text: "hello"))])
            #expect(p.prelim())

            try root.push(xmlNode: .element(p), txn: txn)
            try root.push(xmlNode: .text(YXmlText(text: "world")), txn: txn)

            return p
        }

        #expect(!first.prelim())
        #expect(try first.prevSibling() == nil)

        guard case .text(let second) = try first.nextSibling() else {
            Issue.record("expected text sibling")
            return
        }
        #expect(try second.toText() == "world")
        #expect(try second.nextSibling() == nil)

        guard case .element(let prev) = try second.prevSibling() else {
            Issue.record("expected element sibling")
            return
        }
        #expect(try prev.toText() == first.toText())
    }

    @Test func applyDeltaToAnXmlText() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        let text = YXmlText(text: "hello world")
        try a.getXmlFragment(name: "x").push(xmlNode: .text(text))
        try text.applyDelta(delta: [
            .yRetain(5, ["bold": .bool(true)]),
            .yInsert(.string(","), nil),
            .yRetain(1, nil),
            .yDelete(5),
            .yInsert(.string("there"), ["italic": .bool(true)]),
        ])
        #expect(try text.toText() == "<bold>hello</bold>, <italic>there</italic>")

        let b = YDoc(options: YDocOptions(clientId: 2))
        try applyUpdateV2(doc: b, update: encodeStateAsUpdateV2(doc: a))
        guard case .text(let replica) = try b.getXmlFragment(name: "x").firstChild() else { Issue.record("not a text"); return }
        #expect(try replica.toText() == "<bold>hello</bold>, <italic>there</italic>")

        // A delete past the end throws before anything is applied; a preliminary text has nothing to apply it to.
        expectYError(YError.OutOfBounds) { try text.applyDelta(delta: [.yInsert(.string("!"), nil), .yDelete(20)]) }
        #expect(try text.toText() == "<bold>hello</bold>, <italic>there</italic>")
        expectYError(YError.NotIntegrated) { try YXmlText(text: "x").applyDelta(delta: [.yDelete(1)]) }

        // Inside its own callback, the document is locked.
        let results = Box<String>()
        let sub = try text.observe { _ in
            do { try text.applyDelta(delta: [.yDelete(1)]); results.append("worked") } catch { results.append(caseName(error as! YError)) }
        }
        try text.applyDelta(delta: [.yRetain(5, ["bold": .null])])
        #expect(results.all == ["Locked"])
        #expect(try text.toText() == "hello, <italic>there</italic>")
        _ = sub
    }

    private func describe(_ nodes: [YXmlChild]) throws -> [String] {
        try nodes.map { node in
            switch node {
            case .element(let e): return "<\(try e.name())>"
            case .text(let t): return try t.toText()
            case .fragment: return "#fragment"
            }
        }
    }

    @Test func treeWalker() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        let root = try a.getXmlFragment(name: "x")
        let b = try YXmlElement(name: "b", children: [.text(YXmlText(text: "world"))])
        let p = try YXmlElement(name: "p", children: [.text(YXmlText(text: "Hello ")), .element(b)])
        #expect(try describe(p.treeWalker()) == ["Hello ", "<b>", "world"], "a new node's tree")
        try root.push(xmlNode: .element(p))
        try root.push(xmlNode: .text(YXmlText(text: "again")))
        try root.push(xmlNode: .element(YXmlElement(name: "hr")))

        let everything = ["<p>", "Hello ", "<b>", "world", "again", "<hr>"]
        #expect(try describe(root.treeWalker()) == everything)
        #expect(try describe(p.treeWalker()) == ["Hello ", "<b>", "world"], "only what is inside")
        guard case .element(let found) = try root.treeWalker()[2] else { Issue.record("not an element"); return }
        #expect(found == b, "a handle of the node itself")

        let replica = YDoc(options: YDocOptions(clientId: 2))
        try applyUpdateV2(doc: replica, update: encodeStateAsUpdateV2(doc: a))
        #expect(try describe(replica.getXmlFragment(name: "x").treeWalker()) == everything)

        try root.delete(index: 0, length: 1)
        #expect(try describe(root.treeWalker()) == ["again", "<hr>"], "deleted nodes are left out")
        let results = Box<String>()
        let sub = try root.observe { _ in
            do { _ = try root.treeWalker(); results.append("worked") } catch { results.append(caseName(error as! YError)) }
        }
        try root.delete(index: 0, length: 2)
        #expect(results.all == ["Locked"])
        #expect(try root.treeWalker().isEmpty)
        _ = sub
    }
}
