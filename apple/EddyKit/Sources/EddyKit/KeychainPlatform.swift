import Foundation
import os
import Security

/// Platform services backed by the macOS/iOS keychain and unified logging.
public final class KeychainPlatform: PlatformServices, @unchecked Sendable {
    private let service: String
    private let logger: Logger

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
        var q = query(key)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: AnyObject?
        guard SecItemCopyMatching(q as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    public func secretSet(key: String, value: String) -> Bool {
        let data = Data(value.utf8)
        let status = SecItemUpdate(query(key) as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecSuccess { return true }
        guard status == errSecItemNotFound else { return false }
        var add = query(key)
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }

    public func secretDelete(key: String) {
        SecItemDelete(query(key) as CFDictionary)
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
