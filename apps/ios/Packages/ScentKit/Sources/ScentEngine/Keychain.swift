import Foundation
import Security

/// The broker password, in this device's Keychain and nowhere else: service
/// `scent-control`, account `<username>@<broker host>`, as the Mac keeps it. It never
/// leaves the Keychain except to log in, and is never written to a file or a log.
enum Keychain {
  private static let service = "scent-control"

  /// The account for these settings, or nil for an address that isn't one.
  static func account(_ settings: BrokerSettings) -> String? {
    guard let host = URL(string: settings.url)?.host, !host.isEmpty, !settings.username.isEmpty else {
      return nil
    }
    return "\(settings.username)@\(host)"
  }

  /// The saved password, or nil when none is saved.
  static func password(for settings: BrokerSettings) -> String? {
    guard let account = account(settings) else { return nil }
    var query = baseQuery(account)
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var item: CFTypeRef?
    guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
      let data = item as? Data
    else { return nil }
    return String(data: data, encoding: .utf8)
  }

  /// Saves the password, replacing any earlier one. Readable only while the phone is
  /// unlocked, and never synced to other devices.
  static func save(_ password: String, for settings: BrokerSettings) throws(KeychainError) {
    guard let account = account(settings) else {
      throw KeychainError(message: "Enter the broker's address and username first.")
    }
    let data = Data(password.utf8)
    let query = baseQuery(account)
    let update: [String: Any] = [
      kSecValueData as String: data,
      kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
    ]
    var status = SecItemUpdate(query as CFDictionary, update as CFDictionary)
    if status == errSecItemNotFound {
      var add = query
      add.merge(update) { _, new in new }
      status = SecItemAdd(add as CFDictionary, nil)
    }
    guard status == errSecSuccess else {
      throw KeychainError(message: "The Keychain couldn't save the broker password (error \(status)).")
    }
  }

  private static func baseQuery(_ account: String) -> [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: account,
    ]
  }
}

struct KeychainError: Error, Equatable {
  let message: String
}

/// Where the engine keeps the broker password: the Keychain in the app, memory in tests.
protocol PasswordStore {
  func password(for settings: BrokerSettings) -> String?
  func save(_ password: String, for settings: BrokerSettings) throws(KeychainError)
}

struct KeychainPasswords: PasswordStore {
  func password(for settings: BrokerSettings) -> String? {
    Keychain.password(for: settings)
  }

  func save(_ password: String, for settings: BrokerSettings) throws(KeychainError) {
    try Keychain.save(password, for: settings)
  }
}
