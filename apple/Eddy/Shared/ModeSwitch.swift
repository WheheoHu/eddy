import AppKit
import SwiftUI

/// The mode switch (and background video conversion indicator), at the trailing
/// end of the title bar on every screen.
/// A toolbar item can't stay put: in Three-Pane the split view lays it out
/// right of the sidebar (`.principal` included), elsewhere it follows the
/// screen's own leading controls. Dynamic toolbar items also trigger AppKit/SwiftUI
/// constraint feedback loops on macOS 15. A title bar accessory belongs to the
/// window and the toolbar lays out around it cleanly.
struct ModeSwitchInstaller: NSViewRepresentable {
    @Environment(AppModel.self) private var model

    func makeNSView(context: Context) -> NSView {
        let view = Anchor()
        view.model = model
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {
        if let anchor = view as? Anchor {
            anchor.model = model
        }
    }

    private final class Anchor: NSView {
        var model: AppModel?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window, let model, !window.titlebarAccessoryViewControllers.contains(where: { $0 is Accessory }) else { return }
            window.addTitlebarAccessoryViewController(Accessory(model: model))
        }
    }

    private final class Accessory: NSTitlebarAccessoryViewController {
        let model: AppModel

        init(model: AppModel) {
            self.model = model
            super.init(nibName: nil, bundle: nil)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError() }

        override func loadView() {
            let host = NSHostingView(rootView: TrailingTitlebarControls().environment(model))
            // Reserve fixed width for [VideoJobsStatusButton + ModeSwitch] so dynamic job counts
            // never resize the accessory view during an AppKit display cycle.
            host.sizingOptions = []
            host.frame = CGRect(x: 0, y: 0, width: 176, height: 28)
            view = host
            layoutAttribute = .trailing
        }
    }
}

private struct TrailingTitlebarControls: View {
    @AppStorage("app.mode") private var mode: AppMode = .threePane

    var body: some View {
        HStack(spacing: 8) {
            Spacer(minLength: 0)
            VideoJobsStatusButton()
            Picker("Mode", selection: $mode) {
                ForEach(AppMode.allCases) { Label($0.rawValue, systemImage: $0.icon).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelStyle(.iconOnly)
            .labelsHidden()
            .fixedSize()
            .help("Three-Pane (⌘1), Library (⌘2) or Gallery (⌘3)")
        }
        .padding(.horizontal, 8)
        .frame(width: 176)
        .frame(maxHeight: .infinity)
    }
}
