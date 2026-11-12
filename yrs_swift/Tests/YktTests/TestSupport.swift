import Foundation
import Testing
import Ykt

/// Expects `body` to throw this case of `YError`, whatever its message:
/// `expectYError(YError.OutOfBounds) { try array.get(index: 9) }`.
func expectYError<R>(
    _ expected: (String) -> YError,
    sourceLocation: SourceLocation = #_sourceLocation,
    _ body: () throws -> R
) {
    let name = caseName(expected(""))
    do {
        _ = try body()
        Issue.record("expected YError.\(name), nothing was thrown", sourceLocation: sourceLocation)
    } catch let error as YError {
        #expect(caseName(error) == name, "\(error)", sourceLocation: sourceLocation)
    } catch {
        Issue.record("expected YError.\(name), got \(error)", sourceLocation: sourceLocation)
    }
}

/// `Locked` for `YError.Locked(message: …)`.
func caseName(_ error: YError) -> String {
    String(String(describing: error).prefix { $0 != "(" })
}
