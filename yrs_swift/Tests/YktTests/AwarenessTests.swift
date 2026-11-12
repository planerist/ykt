import Foundation
import Testing
import Ykt

/// Awareness: two peers exchanging states, observers with added / updated / removed clients and the
/// origin, removal by clearing, by removeStates and by the outdated check on an injected clock,
/// what callbacks may not do, garbage.
struct AwarenessTests {
    final class TestClock: YAwarenessClock, @unchecked Sendable {
        private let lock = NSLock()
        private var now: UInt64 = 0
        func nowMillis() -> UInt64 { lock.withLock { now } }
        func advance(_ millis: UInt64) { lock.withLock { now += millis } }
    }

    private func peer(_ client: UInt64, _ clock: TestClock) -> YAwareness {
        YAwareness(doc: YDoc(options: YDocOptions(clientId: client)), clock: clock)
    }

    private func send(_ from: YAwareness, _ to: YAwareness) throws {
        try to.applyUpdate(update: try from.encodeUpdate(), origin: Data("peer".utf8))
    }

    private let ann: YValue = .attrMap(["name": .string("Ann"), "cursor": .attrMap(["index": .number(3)])])

    @Test func twoPeersExchangeStates() throws {
        let clock = TestClock()
        let a = peer(1, clock)
        let b = peer(2, clock)
        try a.setLocalState(state: ann)
        try b.setLocalStateJson(json: #"{"name":"Bob"}"#)
        try send(a, b)
        try send(b, a)
        for p in [a, b] {
            #expect(try p.states() == [1: ann, 2: .attrMap(["name": .string("Bob")])])
        }
        #expect(try b.statesJson()[2] == #"{"name":"Bob"}"#)
        #expect(try a.localState() == ann)
        #expect(try b.meta(clientId: 1)?.clock == 1)
    }

    @Test func observersHearClientsAndTheOrigin() throws {
        let clock = TestClock()
        let a = peer(1, clock)
        let b = peer(2, clock)
        let changes = Box<YAwarenessEvent>()
        let sub = try b.observeChange { changes.append($0) }
        try a.setLocalState(state: ann)
        try send(a, b)
        try a.setLocalState(state: .attrMap(["name": .string("Ann K.")]))
        try send(a, b)
        try a.clearLocalState()
        try send(a, b)
        #expect(changes.all.map { [$0.added, $0.updated, $0.removed] } == [[[1], [], []], [[], [1], []], [[], [], [1]]])
        #expect(changes.all.map(\.originString) == ["peer", "peer", "peer"])
        #expect(try b.states().isEmpty)
        _ = sub
    }

    @Test func outdatedPeersAreRemoved() throws {
        let clock = TestClock()
        let a = peer(1, clock)
        let b = peer(2, clock)
        try a.setLocalState(state: ann)
        try b.setLocalState(state: .bool(true))
        try send(a, b)
        clock.advance(20_000)
        #expect(try b.removeOutdated(timeoutMillis: 30_000) == [])
        clock.advance(15_000)
        #expect(try b.removeOutdated(timeoutMillis: 30_000) == [1])
        #expect(try b.states() == [2: .bool(true)])
        try b.removeStates(clients: [2]) // the local one too, if asked
        #expect(try b.localState() == nil)
    }

    @Test func insideACallbackItIsLocked() throws {
        let a = peer(1, TestClock())
        let results = Box<String>()
        let sub = try a.observeChange { _ in
            do { _ = try a.states(); results.append("worked") } catch { results.append(caseName(error as! YError)) }
        }
        try a.setLocalState(state: ann)
        #expect(results.all == ["Locked"])
        _ = sub
    }

    @Test func garbageIsInvalidData() throws {
        let b = peer(2, TestClock())
        expectYError(YError.InvalidData) { try b.applyUpdate(update: Data([0xff, 0xff, 0xff])) }
        expectYError(YError.InvalidData) { try b.setLocalStateJson(json: "{nope") }
        #expect(try b.states().isEmpty)
    }
}
