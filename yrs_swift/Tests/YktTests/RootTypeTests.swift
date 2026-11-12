import Foundation
import Testing
import Ykt

/// Runs `body` on a thread of its own and waits at most 10 s for it: a call that waits for the
/// document's lock fails the test instead of hanging the suite.
func within<T>(_ body: @escaping @Sendable () throws -> T, sourceLocation: SourceLocation = #_sourceLocation) throws -> T? {
    let done = DispatchSemaphore(value: 0)
    let result = Box<Result<T, any Error>>()
    Thread.detachNewThread {
        result.append(Result { try body() })
        done.signal()
    }
    guard done.wait(timeout: .now() + 10) == .success else {
        Issue.record("timed out: a call waited for the document's lock", sourceLocation: sourceLocation)
        return nil
    }
    return try result.all[0].get()
}

/// Root types by name: the same name gives the same type; while the document is locked (an open
/// transaction, an observer's callback) the getters throw instead of waiting.
struct RootTypeTests {
    /// What each getter did: "throws" for the document's lock error.
    private static func getters(_ doc: YDoc) -> [String] {
        var results: [String] = []
        func check(_ name: String, _ body: () throws -> Void) {
            do {
                try body()
                results.append("\(name) works")
            } catch YError.Locked {
                results.append("\(name) throws")
            } catch {
                results.append("\(name): \(error)")
            }
        }
        check("getText") { _ = try doc.getText(name: "t") }
        check("getXmlFragment") { _ = try doc.getXmlFragment(name: "x") }
        check("getMap") { _ = try doc.getMap(name: "m") }
        check("getArray") { _ = try doc.getArray(name: "a") }
        return results
    }

    private static let allThrow = ["getText throws", "getXmlFragment throws", "getMap throws", "getArray throws"]
    private static let allWork = ["getText works", "getXmlFragment works", "getMap works", "getArray works"]

    @Test func sameNameSameText() throws {
        _ = try within {
            let doc = YDoc()
            try doc.getText(name: "t").insert(index: 0, chunk: "hello")
            #expect(try doc.getText(name: "t").toText() == "hello")
            #expect(try doc.getXmlFragment(name: "x").length() == 0)
        }
    }

    @Test func insideATransactionTheyThrow() throws {
        _ = try within {
            let doc = YDoc()
            try doc.transact { _ in #expect(Self.getters(doc) == Self.allThrow) }
            #expect(Self.getters(doc) == Self.allWork)
        }
    }

    @Test func insideATextObserverTheyThrow() throws {
        _ = try within {
            let doc = YDoc()
            let text = try doc.getText(name: "t")
            let results = Box<[String]>()
            let sub = try text.observe { _ in results.append(Self.getters(doc)) }
            try text.insert(index: 0, chunk: "a")
            #expect(results.all == [Self.allThrow])
            _ = sub
        }
    }

    @Test func insideAnUpdateObserverOfARemoteUpdateTheyThrow() throws {
        _ = try within {
            let a = YDoc(options: YDocOptions(clientId: 1))
            try a.getText(name: "t").insert(index: 0, chunk: "hi")
            let b = YDoc(options: YDocOptions(clientId: 2))
            let results = Box<[String]>()
            let sub = try b.observeUpdateV2 { _ in results.append(Self.getters(b)) }
            try applyUpdateV2(doc: b, update: try encodeStateAsUpdateV2(doc: a))
            #expect(results.all == [Self.allThrow])
            #expect(try b.getText(name: "t").toText() == "hi")
            _ = sub
        }
    }
}
