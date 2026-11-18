import AppKit
import SwiftUI

/// The window has no title bar or toolbar of the system's: every column
/// carries a strip of its own controls along its top, and the window's
/// buttons sit in the strip of whichever column leads. This takes the title
/// bar away and puts the buttons where a strip leaves room for them.
struct WindowChrome: NSViewRepresentable {
    /// From the window's leading edge to the first button.
    static let inset: CGFloat = 20
    /// The room a leading strip leaves for the buttons.
    static let buttonsWidth: CGFloat = 72

    func makeNSView(context: Context) -> NSView { Anchor() }

    func updateNSView(_ view: NSView, context: Context) {
        (view as? Anchor)?.place()
    }

    private final class Anchor: NSView {
        private var observers: [Any] = []

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            observers.forEach(NotificationCenter.default.removeObserver)
            observers = []
            guard let window else { return }
            window.titleVisibility = .hidden
            window.titlebarAppearsTransparent = true
            window.titlebarSeparatorStyle = .none
            window.styleMask.insert(.fullSizeContentView)
            window.toolbar = nil
            // AppKit lays the buttons out again whenever the window's frame
            // changes or it comes forward; each time they go back.
            let names = [
                NSWindow.didResizeNotification, NSWindow.didBecomeKeyNotification, NSWindow.didBecomeMainNotification,
                NSWindow.didEndLiveResizeNotification, NSWindow.didExitFullScreenNotification, NSWindow.didChangeScreenNotification,
            ]
            observers = names.map { name in
                NotificationCenter.default.addObserver(forName: name, object: window, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { self?.place() }
                }
            }
            // And whenever it moves them for a reason of its own (the window
            // becoming main, the appearance changing).
            if let close = window.standardWindowButton(.closeButton) {
                for view in [close, close.superview, close.superview?.superview].compactMap({ $0 }) {
                    view.postsFrameChangedNotifications = true
                    observers.append(NotificationCenter.default.addObserver(forName: NSView.frameDidChangeNotification, object: view, queue: .main) { [weak self] _ in
                        MainActor.assumeIsolated { self?.place() }
                    })
                }
            }
            // The window is still being put together when this view joins it.
            for delay in [0, 0.1, 0.6] {
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in self?.place() }
            }
        }

        deinit {
            observers.forEach(NotificationCenter.default.removeObserver)
        }

        /// Makes the title bar as tall as a strip and centres the buttons
        /// in it. In full screen the system keeps them in its own bar.
        func place() {
            guard let window, !window.styleMask.contains(.fullScreen),
                  let close = window.standardWindowButton(.closeButton),
                  let container = close.superview?.superview else { return }
            var frame = container.frame
            frame.size.height = Theme.strip
            frame.origin.y = window.frame.height - Theme.strip
            if container.frame != frame { container.frame = frame }
            var x = WindowChrome.inset
            for kind in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
                guard let button = window.standardWindowButton(kind) else { continue }
                let origin = NSPoint(x: x, y: (Theme.strip - button.frame.height) / 2)
                // Only when it is elsewhere: moving it tells this again.
                if button.frame.origin != origin { button.setFrameOrigin(origin) }
                x += button.frame.width + 6
            }
        }
    }
}

/// The strip of controls along the top of a column. The column that leads
/// the window leaves room at its start for the window's buttons.
struct ColumnStrip<Leading: View, Trailing: View>: View {
    var leads = false
    @ViewBuilder var leading: Leading
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 4) {
            if leads { Color.clear.frame(width: WindowChrome.buttonsWidth, height: 1) }
            leading
            Spacer(minLength: 0)
            trailing
        }
        .buttonStyle(FlatButtonStyle())
        .labelStyle(.iconOnly)
        .padding(.horizontal, 12)
        .frame(height: Theme.strip)
    }
}

/// A column's name in large type under its strip, with a line about what
/// is in it.
struct ColumnTitle: View {
    let title: Text
    var note: Text?

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            title.font(Theme.title()).foregroundStyle(Theme.text1).lineLimit(1)
            if let note {
                note.font(Theme.small).foregroundStyle(Theme.text2).lineLimit(1)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 20)
        .padding(.top, 6)
        .padding(.bottom, 14)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

/// The bar under a sidebar: the app's mode, and video conversions when
/// there are any.
struct ModeBar: View {
    @AppStorage("app.mode") private var mode: AppMode = .threePane

    var body: some View {
        VStack(spacing: 0) {
            Hairline()
            HStack(spacing: 8) {
                SwitchBar(
                    selection: $mode,
                    options: AppMode.allCases.enumerated().map { index, mode in
                        .symbol(mode, mode.icon, help: "\(mode.rawValue) (⌘\(index + 1))")
                    },
                    segment: 36
                )
                Spacer(minLength: 0)
                VideoJobsStatusButton()
            }
            .padding(.horizontal, 12)
            .frame(height: 56)
        }
    }
}

/// The Add menu: a click saves a page, the menu has the rest.
struct AddMenu: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        FlatMenu("Add", symbol: "plus", help: "Save a page (⌘L), convert a presentation video, or import a file (⌘O)") {
            Button("Save Page…") { model.savingURL = true }
            Button("Convert Video to Book…") { model.requestVideoConvert() }
            Button("Import EPUB, PDF, or Video…") { model.importing = true }
        }
    }
}

/// A menu behind a flat icon button.
struct FlatMenu<Content: View>: View {
    let title: LocalizedStringKey
    let symbol: String
    let help: LocalizedStringKey
    var on = false
    @ViewBuilder let content: Content
    @State private var hovering = false

    init(_ title: LocalizedStringKey, symbol: String, help: LocalizedStringKey, on: Bool = false, @ViewBuilder content: () -> Content) {
        self.title = title
        self.symbol = symbol
        self.help = help
        self.on = on
        self.content = content()
    }

    var body: some View {
        Menu {
            content
        } label: {
            Image(systemName: symbol).font(.system(size: 15, weight: on ? .semibold : .regular))
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .tint(Theme.text1)
        .frame(width: 32, height: 32)
        .background(on ? Theme.track : hovering ? Theme.hover : Color.clear, in: RoundedRectangle(cornerRadius: Theme.Radius.control, style: .continuous))
        .onHover { hovering = $0 }
        .help(help)
        .accessibilityLabel(Text(title))
    }
}

/// A row of a sidebar: a line icon, a name, and a count at the end.
struct SideRow<Mark: View>: View {
    let symbol: String
    let title: Text
    var count = 0
    let chosen: Bool
    @ViewBuilder var mark: Mark

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: chosen ? .semibold : .regular))
                .frame(width: 20)
            title
                .font(.system(size: 14, weight: chosen ? .semibold : .regular))
                .lineLimit(1)
            mark
            Spacer(minLength: 4)
            Text(count > 0 ? "\(count)" : "")
                .font(.system(size: 13))
                .monospacedDigit()
                .foregroundStyle(Theme.text2)
        }
        .foregroundStyle(Theme.text1)
        .frame(height: 34)
        .contentShape(Rectangle())
    }
}

extension SideRow where Mark == EmptyView {
    init(symbol: String, title: Text, count: Int = 0, chosen: Bool) {
        self.init(symbol: symbol, title: title, count: count, chosen: chosen) { EmptyView() }
    }
}

extension View {
    /// A row of a flat list: no rule of the system's, and a shade behind it
    /// while it is the one chosen.
    func flatRow(chosen: Bool, inset: CGFloat = 10) -> some View {
        listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets(top: 0, leading: inset + 8, bottom: 0, trailing: inset + 8))
            .listRowBackground(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(chosen ? Theme.track : Color.clear)
                    .padding(.horizontal, inset)
            )
    }

    /// A list drawn in the app's language: on the column's own ground, with
    /// the selection shown by `flatRow` and not by the system's highlight.
    func flatList() -> some View {
        listStyle(.plain)
            .scrollContentBackground(.hidden)
            .environment(\.defaultMinListRowHeight, 30)
            .background(TableTuner())
    }
}

/// A heading over a group of rows in a flat list. A row of the list like
/// the others, not a section's header: the table draws a ground of its own
/// under those.
struct SideHeading: View {
    let title: LocalizedStringKey
    var first = false

    var body: some View {
        Text(title)
            .font(Theme.heading)
            .foregroundStyle(Theme.text1)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, first ? 2 : 16)
            .padding(.bottom, 4)
            .flatRow(chosen: false, inset: 2)
            .selectionDisabled()
            .accessibilityAddTraits(.isHeader)
    }
}

/// Reaches the table behind a SwiftUI list for what the list has no
/// modifier for: no highlight of the system's on the selected row, clear
/// ground, and headings that scroll away with their rows.
private struct TableTuner: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { Probe() }

    func updateNSView(_ view: NSView, context: Context) {
        (view as? Probe)?.tune()
    }

    private final class Probe: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            DispatchQueue.main.async { [weak self] in self?.tune() }
        }

        func tune() {
            // The list is a sibling of this background, a few levels up.
            var root: NSView? = superview
            for _ in 0..<4 {
                guard let current = root else { return }
                if let table = Self.table(in: current) {
                    if table.selectionHighlightStyle != .none { table.selectionHighlightStyle = .none }
                    if table.floatsGroupRows { table.floatsGroupRows = false }
                    table.backgroundColor = .clear
                    table.enclosingScrollView?.drawsBackground = false
                    return
                }
                root = current.superview
            }
        }

        private static func table(in view: NSView) -> NSTableView? {
            if let table = view as? NSTableView { return table }
            for child in view.subviews {
                if let found = table(in: child) { return found }
            }
            return nil
        }
    }
}
