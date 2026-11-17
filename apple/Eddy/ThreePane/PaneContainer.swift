import SwiftUI

/// Three-pane's list and reader, and its sidebar where the window is too
/// narrow for the split view's own, laid out for the tier the window is in.
/// Every column stays the same view from one tier to the next and while it
/// is out of sight (moved off the leading edge, not removed), so the text
/// keeps its place and the list its scroll position however the window is
/// resized.
struct PaneContainer<Sidebar: View, Middle: View, Reader: View>: View {
    let arrangement: PaneArrangement
    /// The list's name, shown over it where the sidebar isn't beside it.
    let title: Text
    /// Something is open in the reader to go on to.
    let canRead: Bool
    /// Swipes and row clicks are off: something lies over the window.
    let gesturesEnabled: Bool
    /// Moves to a column: the way back over the list, the scrim, a swipe,
    /// a click on a row.
    let go: (PanePlace) -> Void
    @ViewBuilder let sidebar: Sidebar
    @ViewBuilder let list: Middle
    @ViewBuilder let reader: Reader

    /// The list's width beside the reader, as dragged.
    @AppStorage("threePane.listWidth") private var listWidth = Double(PaneTier.listWidth)
    @State private var dragFrom: Double?

    private static var listWidths: ClosedRange<Double> { 300...480 }

    var body: some View {
        let tier = arrangement.tier
        let dragged = CGFloat(min(max(listWidth, Self.listWidths.lowerBound), Self.listWidths.upperBound))
        let column = arrangement.columnWidth(dragged)
        ZStack(alignment: .topLeading) {
            reader
                .padding(.leading, tier >= .medium ? column : 0)
                .allowsHitTesting(arrangement.readerShown)
                .accessibilityHidden(!arrangement.readerShown)
            // What dims the reader while the columns are in over it.
            Color.black
                .opacity(tier == .narrow && !arrangement.readerShown ? 0.18 : 0)
                .contentShape(Rectangle())
                .onTapGesture { if canRead { go(.reader) } }
                .allowsHitTesting(tier == .narrow && !arrangement.readerShown)
                .accessibilityHidden(true)
            // The list leaves only for the reader; the sidebar comes in over it.
            Stowable(stowed: tier <= .narrow && arrangement.place == .reader, distance: tier == .narrow ? PaneTier.sidebarWidth + column : nil) {
                VStack(spacing: 0) {
                    if tier == .medium || tier == .narrowest {
                        header
                        Divider()
                    }
                    list
                }
                .frame(width: tier == .narrowest ? nil : column)
                .frame(maxHeight: .infinity)
                .background(.background)
                .overlay(alignment: .trailing) { if tier != .narrowest { ColumnEdge() } }
                .offset(x: tier == .narrow ? PaneTier.sidebarWidth : 0)
            }
            // Under the sidebar where that stands in for it.
            .allowsHitTesting(arrangement.listShown)
            .accessibilityHidden(!arrangement.listShown)
            if tier != .wide {
                Stowable(stowed: !arrangement.sidebarShown, distance: tier == .narrowest ? nil : (tier == .narrow ? PaneTier.sidebarWidth + column : column)) {
                    sidebar
                        .frame(width: tier == .narrowest ? nil : (tier == .narrow ? PaneTier.sidebarWidth : column))
                        .frame(maxHeight: .infinity)
                        .background(.background.secondary)
                        .overlay(alignment: .trailing) { if tier != .narrowest { ColumnEdge() } }
                }
            }
            if tier >= .medium {
                resizeHandle.offset(x: column - 4)
            }
            PaneSwipes(
                enabled: gesturesEnabled && tier != .wide,
                columnsExtent: arrangement.columnsExtent(dragged),
                readerLive: arrangement.readerShown,
                sidebarExtent: arrangement.sidebarExtent(dragged),
                listLeadsOn: arrangement.listLeadsOn,
                target: { fingers, overColumns in
                    arrangement.swipe(fingers, overColumns: overColumns, canRead: canRead).map(arrangement.target) ?? .none
                },
                commit: { fingers, overColumns in
                    if let place = arrangement.swipe(fingers, overColumns: overColumns, canRead: canRead) { go(place) }
                },
                rowClicked: { column in
                    switch column {
                    case .sidebar: go(.list)
                    case .list: if canRead { go(.reader) }
                    case .reader: break
                    }
                }
            )
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .clipped()
    }

    /// The list's name and the way back to the sidebar, where the list has
    /// taken its place.
    private var header: some View {
        Button { go(.sidebar) } label: {
            HStack(spacing: 6) {
                Image(systemName: "chevron.left")
                    .font(.body.weight(.semibold))
                    .foregroundStyle(.secondary)
                title.font(.headline).lineLimit(1)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Back to the sidebar")
        .accessibilityLabel(Text("Back to the sidebar"))
    }

    /// Dragging the line between the list and the reader sets the list's
    /// width.
    private var resizeHandle: some View {
        Color.clear
            .frame(width: 8)
            .frame(maxHeight: .infinity)
            .contentShape(Rectangle())
            .pointerStyle(.columnResize)
            .gesture(
                DragGesture(minimumDistance: 1, coordinateSpace: .global)
                    .onChanged { drag in
                        let from = dragFrom ?? listWidth
                        dragFrom = from
                        listWidth = min(max(from + drag.translation.width, Self.listWidths.lowerBound), Self.listWidths.upperBound)
                    }
                    .onEnded { _ in dragFrom = nil }
            )
            .accessibilityHidden(true)
    }
}

/// The line down a column's trailing side. (A `Divider` outside a stack
/// lies flat.)
struct ColumnEdge: View {
    var body: some View {
        Rectangle().fill(.separator).frame(width: 1).frame(maxHeight: .infinity)
    }
}

/// Keeps a column in the view tree while it is out of sight: moved off the
/// leading edge by its own width (or `distance`), where it takes no clicks.
/// Showing it again is then only a move, and what it held is as it was.
private struct Stowable<Content: View>: View {
    let stowed: Bool
    /// How far to move it; nil for its own width.
    let distance: CGFloat?
    @ViewBuilder let content: Content
    /// Wide enough to be out of the window until it has been measured.
    @State private var width: CGFloat = 4000

    var body: some View {
        content
            // Measured only where its own width is the distance, so a window
            // being resized doesn't come through here otherwise.
            .onGeometryChange(for: CGFloat.self) { distance == nil ? $0.size.width : 0 } action: { new in
                if new > 0 { width = new }
            }
            .offset(x: stowed ? -((distance ?? width) + 1) : 0)
            .allowsHitTesting(!stowed)
            .accessibilityHidden(stowed)
    }
}
