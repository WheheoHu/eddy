import Foundation
import os
import Security

/// Platform services backed by the macOS/iOS keychain and unified logging.
///
/// Each secret is read from the keychain once per run and kept in memory:
/// the engine asks for the same secrets on every sync and every check
/// whether AI is set up, and a development build (signed ad hoc, so a
/// different app to the keychain each time) would put up the permission
/// dialog for each of those reads. A denied read counts as "none" for the
/// rest of the run too. Writes and deletes keep the cache current.
public final class KeychainPlatform: PlatformServices, @unchecked Sendable {
    private let service: String
    private let logger: Logger
    /// Guards `cached` and serializes the keychain reads, so two callers
    /// asking for the same key at once produce one dialog, not two.
    private let lock = NSLock()
    private var cached: [String: String?] = [:]

    public init(service: String = "dev.eddy.Eddy") {
        self.service = service
        self.logger = Logger(subsystem: service, category: "core")
    }

    private func query(_ key: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
            kSecAttrSynchronizable as String: false,
        ]
    }

    public func secretGet(key: String) -> String? {
        lock.lock()
        defer { lock.unlock() }
        if let known = cached[key] { return known }
        let value = readKeychain(key)
        cached[key] = .some(value)
        return value
    }

    private func readKeychain(_ key: String) -> String? {
        var q = query(key)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: AnyObject?
        guard SecItemCopyMatching(q as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    public func secretSet(key: String, value: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        let data = Data(value.utf8)
        var status = SecItemUpdate(query(key) as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var add = query(key)
            add[kSecValueData as String] = data
            add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
            status = SecItemAdd(add as CFDictionary, nil)
        }
        guard status == errSecSuccess else { return false }
        cached[key] = .some(value)
        return true
    }

    public func secretDelete(key: String) {
        lock.lock()
        defer { lock.unlock() }
        SecItemDelete(query(key) as CFDictionary)
        cached[key] = .some(nil)
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
