import Testing
@testable import ChatKit

@Suite @MainActor struct KeySettingsInSettings {
    @Test func aTypedKeyIsSavedTrimmedAndTheFieldIsCleared() {
        let store = InMemoryKeyStore()
        let settings = KeySettings(store: store)
        #expect(!settings.hasKey)
        settings.draft = "  \(testKey)\n"
        settings.save()
        #expect(store.read() == testKey)
        #expect(settings.draft.isEmpty)
        #expect(settings.hasKey)
        #expect(settings.message == "Your key is saved in the Keychain.")
    }

    @Test func aBlankKeyIsNotSaved() {
        let store = InMemoryKeyStore(testKey)
        let settings = KeySettings(store: store)
        settings.draft = "   "
        settings.save()
        #expect(store.read() == testKey)
        #expect(settings.message == "Type a key first.")
    }

    @Test func removingTheKeyDeletesIt() {
        let store = InMemoryKeyStore(testKey)
        let settings = KeySettings(store: store)
        #expect(settings.hasKey)
        settings.remove()
        #expect(store.read() == nil)
        #expect(!settings.hasKey)
        #expect(settings.message == "Your key is removed.")
    }

    @Test func aKeychainFailureIsShownWithoutTheKey() {
        let settings = KeySettings(store: FailingStore())
        settings.draft = testKey
        settings.save()
        #expect(settings.message == "Keychain: refused")
        #expect(settings.draft == testKey)
    }
}

private struct FailingStore: APIKeyStore {
    func hasKey() throws(KeyStoreError) -> Bool { throw KeyStoreError("Keychain: refused") }
    func read() throws(KeyStoreError) -> String? { throw KeyStoreError("Keychain: refused") }
    func save(_ key: String) throws(KeyStoreError) { throw KeyStoreError("Keychain: refused") }
    func delete() throws(KeyStoreError) { throw KeyStoreError("Keychain: refused") }
}
