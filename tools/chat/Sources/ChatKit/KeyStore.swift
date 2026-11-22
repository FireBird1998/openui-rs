import Foundation
import Security

/// Where the Thesys API key is kept. The app uses the Keychain
/// (`KeychainKeyStore`); tests use a store in memory. Nothing else in the
/// app holds the key: only the Gateway client reads it, at the moment it
/// builds a request.
public protocol APIKeyStore: Sendable {
    /// Whether a key is saved, without reading it.
    func hasKey() throws(KeyStoreError) -> Bool
    /// The saved key, or nil.
    func read() throws(KeyStoreError) -> String?
    /// Saves `key` in place of any saved one.
    func save(_ key: String) throws(KeyStoreError)
    /// Removes the saved key; removing none is not an error.
    func delete() throws(KeyStoreError)
}

/// Why the key could not be kept or read. Its message never holds the key.
public struct KeyStoreError: Error, Sendable, Equatable, CustomStringConvertible {
    public let description: String
    public init(_ description: String) { self.description = description }
}

/// The key as a generic password item in the login Keychain, under this
/// app's own service name. No access group is named, so the item belongs to
/// this app's signature alone; after a rebuild changes the ad hoc signature,
/// macOS may ask once more before the app reads it.
public struct KeychainKeyStore: APIKeyStore {
    public static let service = "dev.openui-rs.chat"
    public static let account = "thesys-api-key"

    public init() {}

    private var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: Self.service,
         kSecAttrAccount as String: Self.account]
    }

    public func hasKey() throws(KeyStoreError) -> Bool {
        var attributes = query
        attributes[kSecReturnAttributes as String] = true
        let status = SecItemCopyMatching(attributes as CFDictionary, nil)
        if status == errSecItemNotFound { return false }
        try Self.check(status)
        return true
    }

    public func read() throws(KeyStoreError) -> String? {
        var request = query
        request[kSecReturnData as String] = true
        var item: CFTypeRef?
        let status = SecItemCopyMatching(request as CFDictionary, &item)
        if status == errSecItemNotFound { return nil }
        try Self.check(status)
        guard let data = item as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    public func save(_ key: String) throws(KeyStoreError) {
        let data = Data(key.utf8)
        let status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var item = query
            item[kSecValueData as String] = data
            try Self.check(SecItemAdd(item as CFDictionary, nil))
        } else {
            try Self.check(status)
        }
    }

    public func delete() throws(KeyStoreError) {
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecItemNotFound { try Self.check(status) }
    }

    private static func check(_ status: OSStatus) throws(KeyStoreError) {
        guard status == errSecSuccess else {
            let message = SecCopyErrorMessageString(status, nil) as String? ?? "status \(status)"
            throw KeyStoreError("Keychain: \(message)")
        }
    }
}
