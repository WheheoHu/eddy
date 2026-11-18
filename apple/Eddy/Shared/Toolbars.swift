import SwiftUI

/// A column on the leading side, for the Library's and the Gallery's
/// screens, with the content beside it. Each side has its own strip along
/// the top; `SidebarToggle` goes first in whichever leads.
///
/// In a window too narrow for both (`over`), the column lies over the
/// content instead and is off the leading edge until asked for: by the
/// toggle, or by a swipe to the right, which brings it in with the fingers.
/// The content sinks back behind it. A swipe to the left, a click on the
/// content or Esc sends it away again. It stays mounted either way, so it
/// keeps its scroll position, and going from one arrangement to the other
/// is a move.
///
/// Whether the column shows is read here, not by the screen that uses this:
/// the column coming in or going away then runs only this body, not the
/// screen's, which would lay its whole shelf or grid out again as each
/// swipe ends.
struct LeadingColumn<Column: View, Content: View>: View {
    @Environment(AppModel.self) private var model
    let dock: ColumnDock
    /// The column shows beside the content, where there is room for both.
    @Binding var beside: Bool
    let width: CGFloat
    /// Swipes and Esc are off: something lies over the window.
    var gesturesEnabled = true
    @ViewBuilder var column: Column
    @ViewBuilder var content: Content

    var body: some View {
        let over = dock.over, drag = dock.drag
        let shown = dock.shown($beside, model: model).wrappedValue
        let covered = over && shown
        ZStack(alignment: .topLeading) {
            Theme.well
            Sinking(sunk: covered, becoming: over ? { $0 == .sidebar } : nil, travel: width, drag: drag) {
                content
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Theme.list)
            }
            .padding(.leading, !over && shown ? width : 0)
            .allowsHitTesting(!covered)
            .accessibilityHidden(covered)
            Color.clear
                .contentShape(Rectangle())
                .onTapGesture { set(false) }
                .allowsHitTesting(covered)
                .accessibilityHidden(true)
            Sliding(slot: shown ? 0 : -1, heading: { $0 == .sidebar ? 0 : -1 }, travel: width, drag: drag) {
                column
                    .frame(width: width)
                    .frame(maxHeight: .infinity)
                    .background(Theme.side)
                    .overlay(alignment: .trailing) { ColumnEdge() }
                    .shadow(color: .black.opacity(covered ? 0.14 : 0), radius: 18, x: 8)
            }
            .allowsHitTesting(shown)
            .accessibilityHidden(!shown)
            PaneSwipes(
                enabled: gesturesEnabled && over,
                columnsExtent: shown ? width : 0,
                readerLive: !shown,
                sidebarExtent: 0,
                listLeadsOn: false,
                destination: { fingers, _ in
                    switch fingers {
                    case .right: shown ? nil : .sidebar
                    case .left: shown ? .list : nil
                    }
                },
                drag: drag,
                go: { set($0 == .sidebar) },
                rowClicked: { _ in }
            )
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
        .clipped()
        .background {
            // Esc sends away a column that lies over the content.
            Button("Hide Sidebar") { set(false) }
                .keyboardShortcut(.cancelAction)
                .disabled(!(covered && gesturesEnabled))
                .opacity(0)
                .accessibilityHidden(true)
        }
    }

    private func set(_ new: Bool) {
        withAnimation(.easeOut(duration: 0.24)) {
            dock.shown($beside, model: model).wrappedValue = new
            dock.drag.end()
        }
    }
}

/// The strip over the content beside a `LeadingColumn`: it leads the
/// window, with the toggle first, while the column doesn't show. A view of
/// its own for the reason `LeadingColumn` reads the column's state itself.
struct ContentStrip<Trailing: View>: View {
    @Environment(AppModel.self) private var model
    let dock: ColumnDock
    @Binding var beside: Bool
    @ViewBuilder var trailing: Trailing

    var body: some View {
        let shown = dock.shown($beside, model: model)
        ColumnStrip(leads: !shown.wrappedValue) {
            if !shown.wrappedValue { SidebarToggle(visible: shown) }
        } trailing: {
            trailing
        }
    }
}

/// Whether a leading column has room beside the content: the window's
/// width, in three-pane's tiers. The Library and the Gallery keep one and
/// measure the window into it; below the tier that holds a list and an
/// article side by side, the column goes over the content.
@MainActor
@Observable
final class ColumnDock {
    private(set) var tier = PaneTier.wide
    private var measured = false
    /// The swipe that brings the column in or sends it away.
    let drag = PaneDrag()

    var over: Bool { tier < .medium }

    /// The window is `new` wide. A change of tier is a glide, which shows
    /// where the column went; the first measure is not.
    func fit(_ new: PaneTier, model: AppModel) {
        if !measured {
            measured = true
            tier = new
        } else if new != tier {
            withAnimation(.easeOut(duration: 0.28)) {
                tier = new
                drag.end()
            }
        }
        if !over { model.sidebarPulled = false }
    }

    /// The column shows: as the user left it where it is beside the
    /// content (`beside`), and when it has been pulled in where it is over.
    func shown(_ beside: Binding<Bool>, model: AppModel) -> Binding<Bool> {
        Binding {
            self.over ? model.sidebarPulled : beside.wrappedValue
        } set: { new in
            if self.over { model.sidebarPulled = new } else { beside.wrappedValue = new }
        }
    }
}

extension View {
    /// Measures the window into a dock.
    func docks(_ dock: ColumnDock, model: AppModel) -> some View {
        onGeometryChange(for: PaneTier.self) { PaneTier.at($0.size.width, from: dock.tier) } action: { dock.fit($0, model: model) }
    }
}

/// Shows or hides a `LeadingColumn`: the first button of the strip that
/// leads the window.
struct SidebarToggle: View {
    @Binding var visible: Bool

    var body: some View {
        Button { withAnimation(.easeInOut(duration: 0.2)) { visible.toggle() } } label: {
            Label(visible ? "Hide Sidebar" : "Show Sidebar", systemImage: "sidebar.left")
        }
        .buttonStyle(FlatButtonStyle(on: visible))
        .help(visible ? "Hide sidebar" : "Show sidebar")
    }
}

/// A sidebar under its strip, over the status line and the mode switch:
/// what the Library's and the Gallery's leading columns share.
struct SidebarColumn<Rows: View>: View {
    @Environment(AppModel.self) private var model
    let dock: ColumnDock
    @Binding var beside: Bool
    @ViewBuilder var rows: Rows

    var body: some View {
        VStack(spacing: 0) {
            ColumnStrip(leads: true) {
                SidebarToggle(visible: dock.shown($beside, model: model))
            } trailing: {
                Button { Task { await model.refresh() } } label: { Label("Refresh", systemImage: "arrow.clockwise") }
                    .help("Refresh (⌘R)")
                    .disabled(model.busy)
                AddMenu()
            }
            rows
            StatusFooter()
            ModeBar()
        }
    }
}
