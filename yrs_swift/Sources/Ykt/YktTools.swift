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

public extension YMap {
    /// Yjs's `ymap.observe`: the changed keys, with old and new values, after every committed
    /// transaction that changed the map.
    func observe(_ onChange: @escaping @Sendable (YMapEvent) -> Void) throws -> YSubscription {
        try observe(observer: MapObserverClosure(onChange))
    }
}

public extension YArray {
    /// Yjs's `yarray.observe`: the array's change as a delta, after every committed transaction
    /// that changed it.
    func observe(_ onChange: @escaping @Sendable (YArrayEvent) -> Void) throws -> YSubscription {
        try observe(observer: ArrayObserverClosure(onChange))
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

public extension YUndoManager {
    /// A stack item was added — a tracked change recorded (`.undo`), or an undo recorded for redo
    /// (`.redo`): return the meta to keep with it (e.g. the selection before the change), or nil.
    func observeItemAdded(_ onItem: @escaping @Sendable (YUndoEvent) -> [String: YValue]?) throws -> YSubscription {
        try observeItemAdded(observer: UndoItemObserverClosure(onItem))
    }

    /// The last stack item was extended by a change within the capture timeout: return its new
    /// meta, or nil to keep it.
    func observeItemUpdated(_ onItem: @escaping @Sendable (YUndoEvent) -> [String: YValue]?) throws -> YSubscription {
        try observeItemUpdated(observer: UndoItemObserverClosure(onItem))
    }

    /// `undo()` (`.undo`) or `redo()` (`.redo`) popped a stack item, with its meta. Runs before
    /// they return, the document still locked: keep what it needs and apply it after.
    func observeItemPopped(_ onPop: @escaping @Sendable (YUndoEvent) -> Void) throws -> YSubscription {
        try observeItemPopped(observer: UndoPopObserverClosure(onPop))
    }
}

// MARK: - Nested shared types

public extension YOutput {
    /// The plain value; nil for a shared type.
    var asValue: YValue? { if case .value(let value) = self { return value }; return nil }
    /// The text; nil for anything else.
    var asText: YText? { if case .text(let text) = self { return text }; return nil }
    var asMap: YMap? { if case .map(let map) = self { return map }; return nil }
    var asArray: YArray? { if case .array(let array) = self { return array }; return nil }
    /// The subdocument; nil for anything else.
    var asDoc: YDoc? { if case .doc(let doc) = self { return doc }; return nil }
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

public extension YUndoEvent {
    var originString: String? { origin.map { String(decoding: $0, as: UTF8.self) } }
}

public extension YMapEvent {
    var originString: String? { origin.map { String(decoding: $0, as: UTF8.self) } }
}

public extension YArrayEvent {
    var originString: String? { origin.map { String(decoding: $0, as: UTF8.self) } }
}

private final class TextObserverClosure: YTextObserver, @unchecked Sendable {
    let body: @Sendable (YTextEvent) -> Void
    init(_ body: @escaping @Sendable (YTextEvent) -> Void) { self.body = body }
    func onChange(event: YTextEvent) { body(event) }
}

private final class MapObserverClosure: YMapObserver, @unchecked Sendable {
    let body: @Sendable (YMapEvent) -> Void
    init(_ body: @escaping @Sendable (YMapEvent) -> Void) { self.body = body }
    func onChange(event: YMapEvent) { body(event) }
}

private final class ArrayObserverClosure: YArrayObserver, @unchecked Sendable {
    let body: @Sendable (YArrayEvent) -> Void
    init(_ body: @escaping @Sendable (YArrayEvent) -> Void) { self.body = body }
    func onChange(event: YArrayEvent) { body(event) }
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

private final class UndoItemObserverClosure: YUndoItemObserver, @unchecked Sendable {
    let body: @Sendable (YUndoEvent) -> [String: YValue]?
    init(_ body: @escaping @Sendable (YUndoEvent) -> [String: YValue]?) { self.body = body }
    func onItem(event: YUndoEvent) -> [String: YValue]? { body(event) }
}

private final class UndoPopObserverClosure: YUndoPopObserver, @unchecked Sendable {
    let body: @Sendable (YUndoEvent) -> Void
    init(_ body: @escaping @Sendable (YUndoEvent) -> Void) { self.body = body }
    func onPop(event: YUndoEvent) { body(event) }
}
