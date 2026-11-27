import AppKit
import Testing

/// Whether the login session's screen is locked. While it is, AppKit leaves
/// a `miniaturize(_:)` call pending: the window never becomes miniaturized,
/// so a test that needs it can't run. Read from the window server's session
/// dictionary, the same fact `scripts/check-swift-tests.py` probes.
@MainActor enum SessionLock {
    static let lockedReason: Comment = "session locked: AppKit will not minimise"

    static var isLocked: Bool {
        guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else { return false }
        return (session["CGSSessionScreenIsLocked"] as? Bool) == true
    }
}

/// Skips a test that minimises a window while the screen is locked, naming
/// the lock.
struct RequiresUnlockedSession: TestTrait {
    func prepare(for test: Test) async throws {
        if await SessionLock.isLocked {
            try await ConditionTrait.disabled(SessionLock.lockedReason).prepare(for: test)
        }
    }
}

extension Trait where Self == RequiresUnlockedSession {
    static var requiresUnlockedSession: Self { RequiresUnlockedSession() }
}
