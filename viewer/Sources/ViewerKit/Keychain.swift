import Foundation
import Security

/// The Anthropic API key, kept in the macOS Keychain and nowhere else: not in
/// a file, the app's settings, or a log.
public enum APIKeyStore {
    static let service = "dev.openui-rs.viewer"
    static let account = "anthropic-api-key"

    private static var query: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service,
         kSecAttrAccount as String: account]
    }

    /// Whether a key is saved, without reading it.
    public static var hasKey: Bool {
        var attributes = query
        attributes[kSecReturnAttributes as String] = true
        return SecItemCopyMatching(attributes as CFDictionary, nil) == errSecSuccess
    }

    /// The saved key, or nil. The Keychain may ask the maintainer to allow
    /// this, after each rebuild of the viewer (its signature changes).
    public static func read() -> String? {
        var request = query
        request[kSecReturnData as String] = true
        var item: CFTypeRef?
        guard SecItemCopyMatching(request as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data
        else { return nil }
        return String(data: data, encoding: .utf8)
    }

    public static func save(_ key: String) throws {
        let data = Data(key.trimmingCharacters(in: .whitespacesAndNewlines).utf8)
        let status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var item = query
            item[kSecValueData as String] = data
            try check(SecItemAdd(item as CFDictionary, nil))
        } else {
            try check(status)
        }
    }

    public static func delete() throws {
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecItemNotFound { try check(status) }
    }

    private static func check(_ status: OSStatus) throws {
        guard status == errSecSuccess else {
            let message = SecCopyErrorMessageString(status, nil) as String? ?? "status \(status)"
            throw ClaudeError(description: "Keychain: \(message)")
        }
    }
}
