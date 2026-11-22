import Observation

/// Settings' key field: the person types the key, it goes to the key store,
/// and the field is cleared. The saved key is never read back to show it.
@MainActor @Observable
public final class KeySettings {
    /// What is typed in the secure field.
    public var draft = ""
    public private(set) var hasKey: Bool
    /// What the last save or removal did.
    public private(set) var message: String?

    @ObservationIgnored private let store: any APIKeyStore

    public init(store: any APIKeyStore) {
        self.store = store
        hasKey = (try? store.hasKey()) ?? false
    }

    public func save() {
        let key = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty else {
            message = "Type a key first."
            return
        }
        do {
            try store.save(key)
            draft = ""
            hasKey = true
            message = "Your key is saved in the Keychain."
        } catch {
            message = error.description
        }
    }

    public func remove() {
        do {
            try store.delete()
            hasKey = false
            message = "Your key is removed."
        } catch {
            message = error.description
        }
    }
}
