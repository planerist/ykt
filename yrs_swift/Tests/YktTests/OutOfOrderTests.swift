import Foundation
import Testing
import Ykt

/// Updates out of order: a document that got a client's update before that client's earlier one, and
/// other clients' updates before the ones they build on, encoded whole or as a diff, in lib0 v1 and
/// v2. Every encoding reads on a fresh replica, which converges once the missing update arrives.
struct OutOfOrderTests {
    private func doc(_ clientId: UInt64) -> YDoc { YDoc(options: YDocOptions(clientId: clientId, gc: false)) }

    private func edit(_ doc: YDoc, _ change: (YText, YTransaction) throws -> Void) throws -> Data {
        let text = try doc.getText(name: "t")
        return try doc.transact { txn in
            try change(text, txn)
            return try txn.encodeUpdateV2()
        }
    }

    /// A fresh replica that read `update`: it throws if the bytes don't read.
    private func replica(_ update: Data, v2: Bool) throws -> YDoc {
        let replica = doc(99)
        if v2 { try applyUpdateV2(doc: replica, update: update) } else { try applyUpdate(doc: replica, update: update) }
        return replica
    }

    @Test func everyEncodingReadsAndConverges() throws {
        // Client 1: "abc", then "x" at the start, then "y" at the end (built on the first only).
        let source = doc(1)
        let first = try edit(source) { t, txn in try t.insert(index: 0, chunk: "abc", txn: txn) }
        let second = try edit(source) { t, txn in try t.insert(index: 0, chunk: "x", txn: txn) }
        let third = try edit(source) { t, txn in try t.insert(index: 4, chunk: "y", txn: txn) }
        // Client 2 builds on the first: "B" after "a".
        let other = doc(2)
        try applyUpdateV2(doc: other, update: first)
        let fromOther = try edit(other) { t, txn in try t.insert(index: 1, chunk: "B", txn: txn) }
        // Client 3 builds on both: "C" after "B".
        let thirdClient = doc(3)
        for update in [first, fromOther] { try applyUpdateV2(doc: thirdClient, update: update) }
        let fromThird = try edit(thirdClient) { t, txn in try t.insert(index: 2, chunk: "C", txn: txn) }

        // The server gets them out of order — clients 2's and 3's before what they build on, client
        // 1's third before its second — and never the second.
        let server = doc(4)
        let heard = Box<Data>()
        let sub = try server.observeUpdateV2 { heard.append($0.update) }
        for update in [fromThird, fromOther, third, first] { try applyUpdateV2(doc: server, update: update) }
        _ = sub.unsubscribe()

        let nothing = try encodeStateVector(doc: doc(98))
        let encodings: [(Data, Bool)] = [
            (try encodeStateAsUpdateV2(doc: server), true),
            (try encodeStateAsUpdateV2(doc: server, vector: decodeStateVector(vector: nothing)), true),
            (try encodeStateAsUpdate(doc: server), false),
            (try encodeStateAsUpdate(doc: server, vector: decodeStateVector(vector: nothing)), false),
        ]
        let txn = try server.transaction()
        let diff = try txn.diffV2(vector: nothing)
        try txn.commit()
        let more: [(Data, Bool)] = [
            (diff, true),
            (try encodeStateFromSnapshotV2(doc: server, snapshot: snapshot(doc: server)), true),
        ] + heard.all.map { ($0, true) }
        for (bytes, v2) in encodings + more { _ = try replica(bytes, v2: v2) }

        // The whole state, then the missing update: everything.
        for (bytes, v2) in encodings {
            let replica = try replica(bytes, v2: v2)
            try applyUpdateV2(doc: replica, update: second)
            #expect(try replica.getText(name: "t").toText() == "xaBCbcy")
        }
        try applyUpdateV2(doc: server, update: second)
        #expect(try server.getText(name: "t").toText() == "xaBCbcy")
    }
}
