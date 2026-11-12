import Foundation
import Testing
import Ykt

/// Deep observers with targets and paths, XML observers, and the document's after-transaction,
/// subdocument and destroy events; unsubscribing inside a callback; remote updates.
struct EventsTests {
    private func paths(_ event: YDeepEvent) -> [[YPathSegment]] {
        event.events.map { event in
            switch event {
            case .textChange(_, let path, _), .mapChange(_, let path, _), .arrayChange(_, let path, _): return path
            case .xmlChange(_, let path, _, _), .xmlTextChange(_, let path, _, _): return path
            case .weakLinkChange(_, let path): return path
            }
        }
    }

    @Test func mapDeepObserverPathsAndTargets() throws {
        let doc = YDoc()
        let root = try doc.getMap(name: "root")
        let list = try root.setArray(key: "list")
        let item = try list.insertMap(index: 0)
        let title = try item.setText(key: "title")
        let events = Box<YDeepEvent>()
        let sub = try root.observeDeep { events.append($0) }

        try root.set(key: "plain", value: .number(1))
        try title.insert(index: 0, chunk: "hi")
        try doc.transact(origin: "editor") { txn in
            try item.set(key: "done", value: .bool(true), txn: txn)
            try list.push(values: [.string("x")], txn: txn)
        }

        #expect(events.all.map(paths) == [
            [[]],
            [[.key(key: "list"), .index(index: 0), .key(key: "title")]],
            [[.key(key: "list")], [.key(key: "list"), .index(index: 0)]],
        ])
        guard case .textChange(let target, _, let delta) = events.all[1].events[0] else { Issue.record("not a text"); return }
        #expect(target == title)
        #expect(delta == [.yInsert(.string("hi"), nil)])
        #expect(events.all[2].originString == "editor")
        _ = sub
    }

    @Test func textDeepObserverHearsAnEmbeddedMap() throws {
        let doc = YDoc()
        let text = try doc.getText(name: "t")
        try text.insert(index: 0, chunk: "ab")
        let table = try text.insertEmbedMap(index: 1)
        let events = Box<YDeepEvent>()
        let sub = try text.observeDeep { events.append($0) }
        try table.set(key: "rows", value: .number(2))
        guard case .mapChange(let target, let path, let keys) = events.all.first?.events.first else { Issue.record("no map event"); return }
        #expect(target == table)
        #expect(path == [.index(index: 1)])
        #expect(keys == ["rows": .inserted(value: .value(value: .number(2)))])
        _ = sub
    }

    @Test func remoteChangesReachADeepObserver() throws {
        let a = YDoc(options: YDocOptions(clientId: 1))
        let item = try a.getArray(name: "list").insertMap(index: 0, entries: ["k": .number(1)])
        let b = YDoc(options: YDocOptions(clientId: 2))
        try applyUpdateV2(doc: b, update: try encodeStateAsUpdateV2(doc: a))
        let events = Box<YDeepEvent>()
        let sub = try b.getArray(name: "list").observeDeep { events.append($0) }
        try item.set(key: "k", value: .number(2))
        try applyUpdateV2(doc: b, update: try encodeStateAsUpdateV2(doc: a), origin: Data("server".utf8))
        #expect(events.all.map(\.originString) == ["server"])
        #expect(events.all.map(paths) == [[[.index(index: 0)]]])
        _ = sub
    }

    @Test func xmlObservers() throws {
        let doc = YDoc()
        let fragment = try doc.getXmlFragment(name: "x")
        let fragmentEvents = Box<YXmlEvent>()
        let sub = try fragment.observe { fragmentEvents.append($0) }
        let p = try YXmlElement(name: "p")
        try fragment.push(xmlNode: .element(p))
        let elementEvents = Box<YXmlEvent>()
        let elementSub = try p.observe { elementEvents.append($0) }
        try p.setAttribute(name: "class", value: .string("a"))
        try p.setAttribute(name: "class", value: .string("b"))

        #expect(fragmentEvents.count == 1)
        guard case .insert(let nodes) = fragmentEvents.all[0].children.first, case .xmlElement(let element) = nodes.first else {
            Issue.record("no inserted element"); return
        }
        #expect(element == p)
        #expect(elementEvents.all.map(\.attributes) == [
            ["class": .inserted(value: .value(value: .string("a")))],
            ["class": .updated(oldValue: .value(value: .string("a")), newValue: .value(value: .string("b")))],
        ])
        _ = (sub, elementSub)
    }

    @Test func unsubscribeInsideADeepCallback() throws {
        let doc = YDoc()
        let root = try doc.getMap(name: "root")
        let calls = Box<Bool>()
        final class Holder: @unchecked Sendable { var sub: YSubscription? }
        let holder = Holder()
        holder.sub = try root.observeDeep { _ in
            calls.append((try? doc.getMap(name: "root")) == nil) // locked in the callback
            _ = holder.sub?.unsubscribe()
        }
        try root.set(key: "a", value: .number(1))
        try root.set(key: "b", value: .number(2))
        #expect(calls.all == [true])
    }

    @Test func documentEvents() throws {
        let doc = YDoc(options: YDocOptions(clientId: 7))
        // Made first: getting a root type the first time is a transaction too.
        let text = try doc.getText(name: "t")
        let root = try doc.getMap(name: "root")
        let subdocs = Box<YSubdocsEvent>()
        let subdocsSub = try doc.observeSubdocs { subdocs.append($0) }
        // Subscribed last: registering a document observer is a transaction of its own, which an
        // after-transaction observer would hear.
        let after = Box<YAfterTransactionEvent>()
        let afterSub = try doc.observeAfterTransaction { after.append($0) }
        try text.insert(index: 0, chunk: "hello")
        try doc.transact(origin: "empty") { _ in }
        let page = try root.setDoc(key: "page", doc: YDoc())
        let destroyed = Box<String>()
        let destroySub = try page.observeDestroy { destroyed.append($0.guid) }
        try page.destroyDoc()

        #expect(after.all.prefix(2).map(\.originString) == [nil, "empty"])
        #expect(after.all[0].afterState[7] == 5)
        #expect(subdocs.all.map { $0.added.map { $0.guid() } }.first == [page.guid()])
        #expect(subdocs.all.last?.removed.map { $0.guid() } == [page.guid()])
        #expect(destroyed.all == [page.guid()])
        _ = (afterSub, subdocsSub, destroySub)
    }
}
