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
}
