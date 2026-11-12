import Foundation

public extension YDoc {
    // ARC gives no guarantee when the transaction is released, so commit explicitly
    func transact<R>(origin: String? = nil, _ block: (YTransaction) throws -> R) throws -> R {
        let txn = try transaction(origin: origin)
        let result = try block(txn)
        try txn.commit()
        return result
    }
}

// MARK: - Observers with closures

// The callbacks run synchronously on the thread that commits the transaction, while the document
// is still locked: they must not read or write the document (that throws); everything they need is
// in the event. Keep the returned `YSubscription`: releasing it (or `unsubscribe()`) stops them.

public extension YText {
    /// Yjs's `ytext.observe`: the text's change as a delta, after every committed transaction that
    /// changed it.
    func observe(_ onChange: @escaping @Sendable (YTextEvent) -> Void) throws -> YSubscription {
        try observe(observer: TextObserverClosure(onChange))
    }
}

public extension YDoc {
    /// Yjs's `doc.on('update')`: every committed transaction's changes as one update (lib0 v1).
    func observeUpdateV1(_ onUpdate: @escaping @Sendable (YUpdateEvent) -> Void) throws -> YSubscription {
        try observeUpdateV1(observer: UpdateObserverClosure(onUpdate))
    }

    /// Yjs's `doc.on('updateV2')`: every committed transaction's changes as one update (lib0 v2).
    func observeUpdateV2(_ onUpdate: @escaping @Sendable (YUpdateEvent) -> Void) throws -> YSubscription {
        try observeUpdateV2(observer: UpdateObserverClosure(onUpdate))
    }

    /// The state vectors before and after every committed transaction (an empty one too).
    func observeTransactionCleanup(_ onCleanup: @escaping @Sendable (YTransactionCleanupEvent) -> Void) throws -> YSubscription {
        try observeTransactionCleanup(observer: CleanupObserverClosure(onCleanup))
    }
}

public extension YTextEvent {
    /// The transaction's origin as text (`transaction(origin:)` takes a string).
    var originString: String? { origin.map { String(decoding: $0, as: UTF8.self) } }
}

public extension YUpdateEvent {
    var originString: String? { origin.map { String(decoding: $0, as: UTF8.self) } }
}

public extension YTransactionCleanupEvent {
    var originString: String? { origin.map { String(decoding: $0, as: UTF8.self) } }
}

private final class TextObserverClosure: YTextObserver, @unchecked Sendable {
    let body: @Sendable (YTextEvent) -> Void
    init(_ body: @escaping @Sendable (YTextEvent) -> Void) { self.body = body }
    func onChange(event: YTextEvent) { body(event) }
}

private final class UpdateObserverClosure: YUpdateObserver, @unchecked Sendable {
    let body: @Sendable (YUpdateEvent) -> Void
    init(_ body: @escaping @Sendable (YUpdateEvent) -> Void) { self.body = body }
    func onUpdate(event: YUpdateEvent) { body(event) }
}

private final class CleanupObserverClosure: YTransactionCleanupObserver, @unchecked Sendable {
    let body: @Sendable (YTransactionCleanupEvent) -> Void
    init(_ body: @escaping @Sendable (YTransactionCleanupEvent) -> Void) { self.body = body }
    func onCleanup(event: YTransactionCleanupEvent) { body(event) }
}
