import AppKit
import SwiftUI

/// The browser web addresses open in: the system's default one.
@MainActor
@Observable
final class DefaultBrowser {
    static let shared = DefaultBrowser()

    /// The image drawn for it, in `Assets.xcassets/Browsers`; nil for a
    /// browser without a mark there.
    private(set) var mark: String?

    private init() {
        refresh()
        // The default can change in System Settings while Eddy runs.
        NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
    }

    private func refresh() {
        let app = URL(string: "https://example.com").flatMap { NSWorkspace.shared.urlForApplication(toOpen: $0) }
        let id = app.flatMap { Bundle(url: $0)?.bundleIdentifier }?.lowercased() ?? ""
        let found = Self.marks.first { id.hasPrefix($0.bundle) }?.mark
        if found != mark { mark = found }
    }

    /// Bundle identifiers, lowercased, by how they start: a browser's beta
    /// and nightly channels add to its identifier.
    private static let marks: [(bundle: String, mark: String)] = [
        ("com.apple.safari", "safari"),
        ("com.google.chrome", "chrome"),
        ("org.chromium.chromium", "chrome"),
        ("com.microsoft.edgemac", "edge"),
        ("org.mozilla.firefox", "firefox"),
        ("org.mozilla.nightly", "firefox"),
        ("company.thebrowser.browser", "arc"),
        ("com.operasoftware.opera", "opera"),
        ("com.vivaldi.vivaldi", "vivaldi"),
    ]
}

/// The default browser's mark, drawn in lines like the symbols around it;
/// a globe for a browser without one. It takes the color of its place.
struct BrowserIcon: View {
    /// The side of the square the mark is drawn in; its lines fill three
    /// quarters of that, which at 18 matches the symbols of a toolbar.
    var size: CGFloat = 18

    var body: some View {
        if let mark = DefaultBrowser.shared.mark {
            Image("Browsers/\(mark)")
                .resizable()
                .scaledToFit()
                .frame(width: size, height: size)
        } else {
            Image(systemName: "globe")
        }
    }
}
