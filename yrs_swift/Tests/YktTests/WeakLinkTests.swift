import Foundation
import Testing
import Ykt

/// Weak links: to map entries (`YMap.link`) and quotes of array ranges (`YArray.quote`), inserted
/// into maps, arrays and texts, read with `unquote` / `deref` locally and on a replica after v2
/// updates; bounds and errors; observers, and callbacks that may not read the document.
struct WeakLinkTests {
    private func num(_ n: Double) -> YOutput { .value(value: .number(n)) }
    private func out(_ s: String) -> YOutput { .value(value: .string(s)) }
    private func numbers(_ list: YArray) throws { try list.push(values: [1, 2, 3, 4].map { .number($0) }) }
    private func sync(_ from: YDoc, _ to: YDoc) throws {
        let missing = try diffUpdateV2(update: encodeStateAsUpdateV2(doc: from), stateVector: encodeStateVector(doc: to))
        try applyUpdateV2(doc: to, update: missing)
    }

    @Test func mapLinkFollowsItsEntry() throws {
        let settings = try YDoc().getMap(name: "settings")
        try settings.set(key: "theme", value: .string("dark"))
        let quote = try #require(try settings.link(key: "theme"))
        #expect(quote.prelim())
        expectYError(YError.NotIntegrated) { try quote.deref() }

        let current = try settings.setWeakLink(key: "current", link: quote)
        #expect(try current.deref() == out("dark"))
        try settings.set(key: "theme", value: .string("light"))
        #expect(try current.deref() == out("light"), "the entry's value now")
        #expect(try settings.getOutput(key: "current")?.asWeakLink == current)
        #expect(try settings.get(key: "current") == .null, "the plain API gives its JSON form")

        _ = try settings.delete(key: "theme")
        #expect(try current.deref() == nil)
        #expect(try current.unquote() == [])
        #expect(try settings.link(key: "theme") == nil, "no entry to link")
    }

    @Test func arrayQuoteAcrossReplicas() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        let list = try a.getArray(name: "list")
        try numbers(list)
        let link = try a.getMap(name: "notes").setWeakLink(key: "q", link: list.quote(index: 1, length: 2))
        try list.insert(index: 2, values: [.string("inside")])
        try list.insert(index: 1, values: [.string("before")]) // at its edge: not quoted
        #expect(try link.unquote() == [num(2), out("inside"), num(3)])
        #expect(try link.deref() == num(2), "a quote's first value")

        let b = YDoc(options: YDocOptions(clientId: 2))
        try sync(a, b)
        let replica = try #require(try b.getMap(name: "notes").getOutput(key: "q")?.asWeakLink)
        #expect(try replica.unquote() == link.unquote())
        #expect(try replica.id() == link.id())

        try b.getArray(name: "list").insert(index: 3, values: [.string("remote")]) // between 2 and "inside"
        try sync(b, a)
        #expect(try link.unquote() == [num(2), out("remote"), out("inside"), num(3)])
    }

    @Test func insertingLinksBoundsAndErrors() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        let list = try a.getArray(name: "list")
        expectYError(YError.OutOfBounds) { try list.quote(index: 0, length: 1) }
        try numbers(list)
        expectYError(YError.OutOfBounds) { try list.quote(index: 1, length: 0) }
        expectYError(YError.OutOfBounds) { try list.quote(index: 3, length: 2) }

        let quote = try list.quote(index: 0, length: 2)
        let refs = try a.getArray(name: "refs")
        let inArray = try refs.insertWeakLink(index: 0, link: quote)
        let text = try a.getText(name: "t")
        try text.insert(index: 0, chunk: "see ")
        let inText = try text.insertEmbedWeakLink(index: 4, link: quote, attributes: ["bold": .bool(true)])
        let copy = try a.getMap(name: "notes").setWeakLink(key: "copy", link: inText) // an integrated link is copied
        for link in [inArray, inText, copy] { #expect(try link.unquote() == [num(1), num(2)]) }
        #expect(try text.toDelta()[1] == .yInsert(.null, ["bold": .bool(true)]))

        expectYError(YError.OutOfBounds) { try refs.insertWeakLink(index: 2, link: quote) }
        expectYError(YError.WrongDocument) { try YDoc().getMap(name: "m").setWeakLink(key: "k", link: quote) }
        try refs.delete(index: 0, length: 1)
        expectYError(YError.TypeDeleted) { try refs.insertWeakLink(index: 0, link: inArray) }
    }

    @Test func observersAndLockedCallbacks() throws {
        let doc = YDoc()
        let list = try doc.getArray(name: "list")
        try numbers(list)
        let notes = try doc.getMap(name: "notes")
        let link = try notes.setWeakLink(key: "q", link: list.quote(index: 0, length: 2))
        let events = Box<YWeakLinkEvent>()
        let results = Box<String>()
        let sub = try link.observe { event in
            events.append(event)
            for body in [{ _ = try link.unquote() }, { _ = try list.quote(index: 0, length: 1) }, { _ = try notes.setWeakLink(key: "k", link: link) }] as [() throws -> Void] {
                do { try body(); results.append("worked") } catch { results.append(caseName(error as! YError)) }
            }
        }
        try doc.transact(origin: "editor") { txn in try list.insert(index: 1, values: [.string("x")], txn: txn) }
        try list.push(values: [.string("outside")])
        #expect(events.all.map(\.originString) == ["editor"])
        #expect(events.all.first?.target == link)
        #expect(results.all == ["Locked", "Locked", "Locked"])
        #expect(try link.unquote() == [num(1), out("x"), num(2)])
        _ = sub

        // A link to a map hears the map's own changes deeply.
        let meta = try notes.setMap(key: "meta")
        let toMeta = try notes.setWeakLink(key: "toMeta", link: #require(try notes.link(key: "meta")))
        let deep = Box<YDeepEvent>()
        let deepSub = try toMeta.observeDeep { deep.append($0) }
        try meta.set(key: "color", value: .string("red"))
        guard case .mapChange(let target, _, _)? = deep.all.first?.events.first else { Issue.record("not a map change"); return }
        #expect(target == meta)
        _ = deepSub
    }
}
