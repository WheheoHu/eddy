import Foundation
import os
import Security

/// Platform services backed by the macOS/iOS keychain and unified logging.
///
/// All secrets live in one keychain item, a JSON object of name → value,
/// which is read once when the platform is created and kept in memory.
/// A development build is signed ad hoc, so it is a different app to the
/// keychain each time and every item it reads puts up its own permission
/// dialog; with one item per secret those dialogs came up at launch, on
/// opening an article and on opening Settings. One item means one dialog,
/// at launch.
///
/// If that read is denied, every secret counts as "none" for the run, and
/// writes are refused until a later read succeeds, so a partial set never
/// replaces what is stored.
public final class KeychainPlatform: PlatformServices, @unchecked Sendable {
    /// Account name of the item that holds every secret.
    private static let bundleAccount = "secrets"

    private let service: String
    private let logger: Logger
    /// Guards `secrets` and `loaded`, and serializes keychain access.
    private let lock = NSLock()
    private var secrets: [String: String] = [:]
    /// False while the stored item could not be read (denied or failed).
    private var loaded = false

    public init(service: String = "dev.eddy.Eddy") {
        self.service = service
        self.logger = Logger(subsystem: service, category: "core")
        lock.lock()
        defer { lock.unlock() }
        _ = ensureLoaded()
    }

    private func query(_ account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrSynchronizable as String: false,
        ]
    }

    /// Reads one item's data, with the status that says why there is none.
    private func readItem(_ account: String) -> (status: OSStatus, data: Data?) {
        var q = query(account)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: AnyObject?
        let status = SecItemCopyMatching(q as CFDictionary, &result)
        return (status, result as? Data)
    }

    /// Loads the stored secrets if that has not succeeded yet. Call with
    /// the lock held. This is the read that can put up the dialog.
    private func ensureLoaded() -> Bool {
        if loaded { return true }
        let (status, data) = readItem(Self.bundleAccount)
        switch status {
        case errSecSuccess:
            if let data, let stored = try? JSONDecoder().decode([String: String].self, from: data) {
                secrets = stored
            } else {
                logger.error("Stored secrets are unreadable; starting empty")
                secrets = [:]
            }
        case errSecItemNotFound:
            secrets = [:]
        default:
            logger.warning("Keychain read failed (\(status, privacy: .public)); secrets unavailable")
            return false
        }
        loaded = true
        migrateLegacyItems()
        return true
    }

    /// Writes `secrets` back as one item. Deleting and adding, rather than
    /// updating, makes the item belong to this build: updating an item an
    /// earlier build created can ask for permission again.
    private func persist() -> Bool {
        guard let data = try? JSONEncoder().encode(secrets) else { return false }
        let deleted = SecItemDelete(query(Self.bundleAccount) as CFDictionary)
        guard deleted == errSecSuccess || deleted == errSecItemNotFound else {
            logger.error("Keychain delete failed (\(deleted, privacy: .public))")
            return false
        }
        if secrets.isEmpty { return true }
        var add = query(Self.bundleAccount)
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        let added = SecItemAdd(add as CFDictionary, nil)
        if added != errSecSuccess {
            logger.error("Keychain write failed (\(added, privacy: .public))")
        }
        return added == errSecSuccess
    }

    /// Moves secrets from the earlier one-item-per-secret layout into the
    /// single item. Listing the old items reads attributes only and asks
    /// for nothing; reading each one's value asks once per item, this one
    /// time. An item whose read is denied stays for the next launch.
    private func migrateLegacyItems() {
        let list: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrSynchronizable as String: false,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll,
        ]
        var result: AnyObject?
        guard SecItemCopyMatching(list as CFDictionary, &result) == errSecSuccess,
              let items = result as? [[String: Any]] else { return }
        let accounts = items
            .compactMap { $0[kSecAttrAccount as String] as? String }
            .filter { $0 != Self.bundleAccount }
        guard !accounts.isEmpty else { return }

        var moved: [String] = []
        var added: [String] = []
        for account in accounts {
            let (status, data) = readItem(account)
            guard status == errSecSuccess, let data,
                  let value = String(data: data, encoding: .utf8) else { continue }
            if secrets[account] == nil {
                secrets[account] = value
                added.append(account)
            }
            moved.append(account)
        }
        guard !moved.isEmpty else { return }
        guard added.isEmpty || persist() else {
            for account in added { secrets[account] = nil }
            return
        }
        for account in moved {
            SecItemDelete(query(account) as CFDictionary)
        }
        logger.info("Moved \(moved.count, privacy: .public) keychain items into one")
    }

    public func secretGet(key: String) -> String? {
        lock.lock()
        defer { lock.unlock() }
        return secrets[key]
    }

    public func secretSet(key: String, value: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard ensureLoaded() else { return false }
        let previous = secrets[key]
        if previous == value { return true }
        secrets[key] = value
        guard persist() else {
            secrets[key] = previous
            return false
        }
        return true
    }

    public func secretDelete(key: String) {
        lock.lock()
        defer { lock.unlock() }
        guard ensureLoaded(), let previous = secrets.removeValue(forKey: key) else { return }
        if !persist() { secrets[key] = previous }
    }

    public func log(level: String, message: String) {
        switch level {
        case "error": logger.error("\(message, privacy: .public)")
        case "warn": logger.warning("\(message, privacy: .public)")
        case "debug": logger.debug("\(message, privacy: .public)")
        default: logger.info("\(message, privacy: .public)")
        }
    }
}

public enum EddyPaths {
    /// Keychain service that holds this data directory's secrets. A
    /// directory chosen with `EDDY_DATA_DIR` (development, tests) gets its
    /// own, so trying things out there (pairing with a test server, say)
    /// never writes into the secrets of the real installation.
    public static var keychainService: String {
        let standard = "dev.eddy.Eddy"
        guard let override = ProcessInfo.processInfo.environment["EDDY_DATA_DIR"], !override.isEmpty else {
            return standard
        }
        let path = URL(fileURLWithPath: override, isDirectory: true).standardizedFileURL.path
        // FNV-1a: stable across launches, unlike `hashValue`.
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in path.utf8 {
            hash = (hash ^ UInt64(byte)) &* 0x100_0000_01b3
        }
        return "\(standard).dev.\(String(hash, radix: 16))"
    }

    /// `~/Library/Application Support/Eddy`, created on first use.
    /// Development builds can point elsewhere with `EDDY_DATA_DIR`.
    public static var dataDirectory: URL {
        if let override = ProcessInfo.processInfo.environment["EDDY_DATA_DIR"], !override.isEmpty {
            let dir = URL(fileURLWithPath: override, isDirectory: true)
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            return dir
        }
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let dir = base.appendingPathComponent("Eddy", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }
}
