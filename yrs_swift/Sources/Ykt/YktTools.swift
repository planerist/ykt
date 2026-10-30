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
