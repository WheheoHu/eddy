import AppKit
import SwiftUI

/// The mode switch, at the trailing end of the title bar on every screen.
/// A toolbar item can't stay put: in Three-Pane the split view lays it out
/// right of the sidebar (`.principal` included), elsewhere it follows the
/// screen's own leading controls. A view floated over the window's middle
/// stays put but the toolbar doesn't make room, so the reader's controls
/// slide under it. A title bar accessory belongs to the window and the
/// toolbar lays out around it.
struct ModeSwitchInstaller: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { Anchor() }
    func updateNSView(_ view: NSView, context: Context) {}

    private final class Anchor: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window, !window.titlebarAccessoryViewControllers.contains(where: { $0 is Accessory }) else { return }
            window.addTitlebarAccessoryViewController(Accessory())
        }
    }

    private final class Accessory: NSTitlebarAccessoryViewController {
        override func loadView() {
            let host = NSHostingView(rootView: ModeSwitch())
            host.frame.size = host.fittingSize
            view = host
            layoutAttribute = .trailing
        }
    }
}

private struct ModeSwitch: View {
    @AppStorage("app.mode") private var mode: AppMode = .threePane

    var body: some View {
        Picker("Mode", selection: $mode) {
            ForEach(AppMode.allCases) { Label($0.rawValue, systemImage: $0.icon).tag($0) }
        }
        .pickerStyle(.segmented)
        .labelStyle(.iconOnly)
        .labelsHidden()
        .fixedSize()
        .padding(.horizontal, 8)
        .frame(maxHeight: .infinity)
        .help("Three-Pane (⌘1), Library (⌘2) or Gallery (⌘3)")
    }
}
