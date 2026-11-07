import SwiftUI

/// A collapsible column on the leading side, for the library's screens.
/// The window holds one split view (three-pane mode); a second one that
/// replaces it loses its sidebar toggle and column widths, so the library
/// lays its columns out itself and puts `ColumnToggle` where the system
/// toggle sits, right after the window buttons.
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
                    .background(.background.secondary)
                    .transition(.move(edge: .leading))
                Divider()
            }
            content.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

/// Shows or hides a `LeadingColumn`; the first item of the toolbar.
struct ColumnToggle: ToolbarContent {
    @Binding var visible: Bool

    var body: some ToolbarContent {
        ToolbarItem(placement: .navigation) {
            Button { withAnimation(.easeInOut(duration: 0.2)) { visible.toggle() } } label: {
                Label(visible ? "Hide Sidebar" : "Show Sidebar", systemImage: "sidebar.left")
            }
            .help(visible ? "Hide sidebar" : "Show sidebar")
        }
    }
}

/// The mode switch and the Add menu. Each screen places them after its own
/// leading controls (the column toggle, Home while reading in the library),
/// so the toolbar reads the same way everywhere.
struct ModeToolbar: ToolbarContent {
    @Binding var mode: AppMode
    let model: AppModel

    var body: some ToolbarContent {
        ToolbarItem(placement: .navigation) {
            Picker("Mode", selection: $mode) {
                ForEach(AppMode.allCases) { Label($0.rawValue, systemImage: $0.icon).tag($0) }
            }
            .pickerStyle(.segmented)
            .help("Three-Pane (⌘1) or Library (⌘2)")
        }
        ToolbarItem(placement: .navigation) {
            Menu {
                Button("Save Page…") { model.savingURL = true }
                Button("Import EPUB or PDF…") { model.importing = true }
            } label: {
                Label("Add", systemImage: "plus")
            } primaryAction: {
                model.savingURL = true
            }
            .help("Save a page to read later (⌘L), or import an EPUB or PDF (⌘O)")
        }
    }
}
