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
        HStack(spacing: 0) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: chosen ? .semibold : .regular))
                .frame(width: 20)
            title
                .font(.system(size: 14, weight: chosen ? .semibold : .regular))
                .lineLimit(1)
                .padding(.leading, 9)
            mark.padding(.leading, 4)
            Spacer(minLength: 6)
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
    ///
    /// `active`: keyboard focus is in the row's column. The row chosen
    /// there carries a bar in the accent colour at its start, which says
    /// where the keys go: the sidebar's folder or subscription, or the
    /// list's article.
    ///
    /// `activeEdge`: the side the bar is on. At the end of the row when
    /// the keys have gone on from this column to the one after it (the
    /// article being read), so the bar is on the side they went.
    ///
    /// `lead`: from the shade's edge to the row's content.
    func flatRow(chosen: Bool, active: Bool = false, activeEdge: HorizontalEdge = .leading, inset: CGFloat = 10, lead: CGFloat = 8) -> some View {
        let edge: Edge.Set = activeEdge == .leading ? .leading : .trailing
        return listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets(top: 0, leading: FlatList.gutter + inset + lead, bottom: 0, trailing: inset + 4))
            .listRowBackground(
                RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous)
                    .fill(chosen ? Theme.track : Color.clear)
                    .overlay(alignment: activeEdge == .leading ? .leading : .trailing) {
                        // Always there, shown or not: a row's views don't
                        // change kind with its state.
                        RoundedRectangle(cornerRadius: 1.5, style: .continuous)
                            .fill(Theme.accent)
                            .frame(width: 3)
                            .padding(.vertical, 7)
                            .padding(edge, 2)
                            .opacity(chosen && active ? 1 : 0)
                    }
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

enum FlatList {
    /// Room left at the start of every row. The table behind a list puts a
    /// row's content 8 points in; with an indentation set for an outline's
    /// levels it puts it 26 in, and the rows were drawn for that. Nothing
    /// sets one any more (no list here has levels, and setting one against
    /// the list's own 0 left rows at either place, by when they were last
    /// laid out), so the difference is made up here.
    static let gutter: CGFloat = 18
}

/// The arrow at the start of a row that folds: it points down while the rows
/// under it show. Drawn in the row, inside its shade, not by the list at
/// the column's edge.
struct FoldArrow: View {
    let open: Bool
    let toggle: () -> Void

    static let width: CGFloat = 16

    var body: some View {
        Image(systemName: "chevron.right")
            .font(.system(size: 10, weight: .semibold))
            .foregroundStyle(Theme.text2)
            .rotationEffect(.degrees(open ? 90 : 0))
            .frame(width: Self.width, height: 34)
            .contentShape(Rectangle())
            .onTapGesture(perform: toggle)
            .accessibilityLabel(open ? Text("Fold") : Text("Unfold"))
            .accessibilityAddTraits(.isButton)
    }
}

/// A heading over a group of rows in a flat list. A row of the list like
/// the others, not a section's header: the table draws a ground of its own
/// under those.
struct SideHeading: View {
    let title: LocalizedStringKey
    var first = false
    var lead: CGFloat = 8

    var body: some View {
        Text(title)
            .font(Theme.heading)
            .foregroundStyle(Theme.text1)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, first ? 2 : 16)
            .padding(.bottom, 4)
            .flatRow(chosen: false, lead: lead)
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
            guard let table = nearestTable(to: self) else { return }
            if table.selectionHighlightStyle != .none { table.selectionHighlightStyle = .none }
            if table.floatsGroupRows { table.floatsGroupRows = false }
            table.backgroundColor = .clear
            table.enclosingScrollView?.drawsBackground = false
        }
    }
}

/// The table behind a SwiftUI list, for what has to be done to it by hand:
/// a list's rows come and go without any move of their own, so a folder's
/// rows are slid into place here (`fold`). Put behind the list with
/// `tableHandle(_:)`.
@MainActor
final class TableHandle {
    fileprivate(set) weak var table: NSTableView?

    private static let duration: CFTimeInterval = 0.2

    /// A folder at `row` has opened or shut, bringing or taking the `count`
    /// rows under it: the rows below slide from where they were, and the
    /// ones that came fade in. Called right after the change is made; it
    /// plays once the list has taken it up. Only how the rows are drawn
    /// changes, so rows that miss it are simply in place.
    func fold(at row: Int, count: Int, opening: Bool) {
        guard count > 0, let table, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else { return }
        let before = table.numberOfRows
        play(table, row: row, count: count, opening: opening, before: before, tries: 4)
    }

    /// Scrolls a row into view if it isn't all there, by the least it
    /// takes, with a move. Called right after the row is chosen.
    func reveal(row: Int) {
        DispatchQueue.main.async { [weak self] in
            guard let table = self?.table, row >= 0, row < table.numberOfRows,
                  let scroll = table.enclosingScrollView
            else { return }
            let clip = scroll.contentView
            let rect = table.rect(ofRow: row), visible = clip.documentVisibleRect
            guard rect.minY < visible.minY || rect.maxY > visible.maxY else { return }
            let wanted = rect.minY < visible.minY ? rect.minY : rect.maxY - visible.height
            let y = max(0, min(wanted, table.frame.height - visible.height)) - scroll.contentInsets.top
            let origin = NSPoint(x: clip.bounds.minX, y: y)
            guard !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else {
                clip.scroll(to: origin)
                scroll.reflectScrolledClipView(clip)
                return
            }
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0.25
                context.timingFunction = CAMediaTimingFunction(name: .easeOut)
                clip.animator().setBoundsOrigin(origin)
            } completionHandler: { [weak scroll, weak clip] in
                MainActor.assumeIsolated {
                    if let scroll, let clip { scroll.reflectScrolledClipView(clip) }
                }
            }
        }
    }

    private func play(_ table: NSTableView, row: Int, count: Int, opening: Bool, before: Int, tries: Int) {
        DispatchQueue.main.async { [weak self, weak table] in
            guard let self, let table else { return }
            // The list takes a change up a turn of the run loop or two on.
            guard table.numberOfRows != before else {
                if tries > 0 { self.play(table, row: row, count: count, opening: opening, before: before, tries: tries - 1) }
                return
            }
            guard row >= 0, row < table.numberOfRows else { return }
            let height = table.rect(ofRow: row).height * CGFloat(count)
            // Down the screen is up the layer's y axis unless the table is flipped.
            let down: CGFloat = table.isFlipped ? 1 : -1
            let ease = CAMediaTimingFunction(controlPoints: 0.2, 0.8, 0.2, 1)
            table.enumerateAvailableRowViews { view, index in
                guard index > row, let layer = view.layer else { return }
                let fresh = opening && index <= row + count
                let slide = CABasicAnimation(keyPath: "transform.translation.y")
                // The rows under an opening folder were higher up, the ones
                // under a shutting one lower; the rows that came drop in
                // from just above.
                slide.fromValue = (opening ? (fresh ? -8 : -height) : height) * down
                slide.toValue = 0
                slide.isAdditive = true
                slide.duration = Self.duration
                slide.timingFunction = ease
                layer.add(slide, forKey: "fold")
                if fresh {
                    let fade = CABasicAnimation(keyPath: "opacity")
                    fade.fromValue = 0
                    fade.toValue = 1
                    fade.duration = Self.duration
                    layer.add(fade, forKey: "foldFade")
                }
            }
        }
    }
}

private struct TableReach: NSViewRepresentable {
    let handle: TableHandle

    func makeNSView(context: Context) -> NSView { Probe(handle: handle) }

    func updateNSView(_ view: NSView, context: Context) {
        (view as? Probe)?.find()
    }

    private final class Probe: NSView {
        let handle: TableHandle

        init(handle: TableHandle) {
            self.handle = handle
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError("not used") }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            DispatchQueue.main.async { [weak self] in self?.find() }
        }

        func find() {
            if handle.table == nil { handle.table = nearestTable(to: self) }
        }
    }
}

extension View {
    /// Gives `handle` the table behind this list.
    func tableHandle(_ handle: TableHandle) -> some View {
        background(TableReach(handle: handle))
    }
}

/// The table of the list a background view sits behind: a sibling, a few
/// levels up.
@MainActor
private func nearestTable(to view: NSView) -> NSTableView? {
    func table(in view: NSView) -> NSTableView? {
        if let table = view as? NSTableView { return table }
        for child in view.subviews {
            if let found = table(in: child) { return found }
        }
        return nil
    }
    var root = view.superview
    for _ in 0..<4 {
        guard let current = root else { return nil }
        if let found = table(in: current) { return found }
        root = current.superview
    }
    return nil
}
