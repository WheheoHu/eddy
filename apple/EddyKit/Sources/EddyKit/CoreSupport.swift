import Foundation

/// Swift-side helpers layered on top of the generated UniFFI bindings in `Generated/`.
public enum EddyCore {
    public static var version: String { coreVersion() }

    /// Opens the engine on the default data directory with keychain-backed secrets.
    public static func openDefault(cacheLimit: UInt64 = 2 * 1024 * 1024 * 1024) throws -> EddyEngine {
        try EddyEngine(
            dataDir: EddyPaths.dataDirectory.path,
            platform: KeychainPlatform(),
            defaultCacheLimit: cacheLimit
        )
    }
}

public extension BlockKindView {
    /// Plain text of a block, for debugging and simple renderers.
    var plainText: String {
        func join(_ runs: [RunView]) -> String {
            runs.map { $0.math ? "$\($0.text)$" : $0.text }.joined()
        }
        switch self {
        case let .heading(_, runs), let .paragraph(runs): return join(runs)
        case let .listItem(_, _, _, _, runs): return join(runs)
        case let .code(_, text, _): return text
        case let .math(tex): return "$$\(tex)$$"
        case let .image(src, alt, _, _, caption): return "[image \(alt.isEmpty ? src : alt)] \(join(caption))"
        case let .table(rows): return rows.map { $0.cells.map { join($0.runs) }.joined(separator: " | ") }.joined(separator: "\n")
        case let .media(_, src, _, _): return "[media \(src)]"
        case .rule: return "———"
        }
    }
}
