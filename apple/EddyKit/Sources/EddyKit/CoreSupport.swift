import Foundation

/// Swift-side helpers layered on top of the generated UniFFI bindings in `Generated/`.
public enum EddyCore {
    public static var version: String { coreVersion() }
}
