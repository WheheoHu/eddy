import SwiftUI

/// A collapsible column on the leading side, for the Library's and the
/// Gallery's screens, with the content beside it. Each side has its own
/// strip along the top; `SidebarToggle` goes first in whichever leads.
struct LeadingColumn<Column: View, Content: View>: View {
    let visible: Bool
    let width: CGFloat
    @ViewBuilder var column: Column
    @ViewBuilder var content: Content

    var body: some View {
        HStack(spacing: 0) {
            if visible {
                column
                    .frame(width: width)
                    .background(Theme.side)
                    .transition(.move(edge: .leading))
                Hairline(vertical: true)
            }
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Theme.list)
        }
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
    @Binding var visible: Bool
    @ViewBuilder var rows: Rows

    var body: some View {
        VStack(spacing: 0) {
            ColumnStrip(leads: true) {
                SidebarToggle(visible: $visible)
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
