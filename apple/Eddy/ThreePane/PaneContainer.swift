import SwiftUI

/// Three-pane's list and reader, and its sidebar where the window is too
/// narrow for the split view's own, laid out for the tier the window is in.
/// Every column stays the same view from one tier to the next and while it
/// is out of sight (moved off the leading edge, not removed), so the text
/// keeps its place and the list its scroll position however the window is
/// resized. A swipe moves the columns with the fingers: `drag` says how far.
struct PaneContainer<Sidebar: View, Middle: View, Reader: View>: View {
    let arrangement: PaneArrangement
    /// The list's name, shown over it where the sidebar isn't beside it.
    let title: Text
    /// Something is open in the reader to go on to.
    let canRead: Bool
    /// Swipes and row clicks are off: something lies over the window.
    let gesturesEnabled: Bool
    /// The swipe between columns in progress, if any.
    let drag: PaneDrag
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
        // The columns as they will be where the swipe in progress leads.
        let heading = drag.destination.map { PaneArrangement(tier: tier, place: $0) }
        ZStack(alignment: .topLeading) {
            reader
                // The placeholder for nothing selected sits in the middle.
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(.leading, tier >= .medium ? column : 0)
                .allowsHitTesting(arrangement.readerShown)
                .accessibilityHidden(!arrangement.readerShown)
            // What dims the reader while the columns are in over it.
            Scrim(dimmed: arrangement.dimmed, becoming: heading?.dimmed, travel: PaneTier.sidebarWidth + column, drag: drag)
                .contentShape(Rectangle())
                .onTapGesture { if canRead { go(.reader) } }
                .allowsHitTesting(arrangement.dimmed)
                .accessibilityHidden(true)
            Sliding(
                slot: arrangement.listSlot, heading: heading?.listSlot,
                travel: tier == .narrowest ? nil : (tier == .narrow ? PaneTier.sidebarWidth + column : column), drag: drag
            ) {
                VStack(spacing: 0) {
                    if tier == .medium || tier == .narrowest {
                        header
                        Divider()
                    }
                    list
                }
                .frame(width: tier == .narrowest ? nil : column)
                .frame(maxHeight: .infinity)
                // A color, not the background style: that one stays where
                // it is when the column moves, and covers the reader.
                .background(Color(nsColor: .textBackgroundColor))
                .overlay(alignment: .trailing) { if tier != .narrowest { ColumnEdge() } }
            }
            // The list's space: beside the sidebar in the narrow tier, the
            // leading column otherwise. Pushed out of it by the sidebar,
            // the list is clipped away, not over the reader.
            .padding(.leading, tier == .narrow ? PaneTier.sidebarWidth : 0)
            .frame(width: tier == .narrowest ? nil : (tier == .narrow ? PaneTier.sidebarWidth + column : column), alignment: .leading)
            .clipped()
            .allowsHitTesting(arrangement.listShown)
            .accessibilityHidden(!arrangement.listShown)
            if tier != .wide {
                Sliding(
                    slot: arrangement.sidebarSlot, heading: heading?.sidebarSlot,
                    travel: tier == .narrowest ? nil : (tier == .narrow ? PaneTier.sidebarWidth + column : column), drag: drag
                ) {
                    sidebar
                        .frame(width: tier == .narrowest ? nil : (tier == .narrow ? PaneTier.sidebarWidth : column))
                        .frame(maxHeight: .infinity)
                        .background(Color(nsColor: .windowBackgroundColor))
                        .overlay(alignment: .trailing) { if tier != .narrowest { ColumnEdge() } }
                }
                .allowsHitTesting(arrangement.sidebarShown)
                .accessibilityHidden(!arrangement.sidebarShown)
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
                destination: { fingers, overColumns in
                    arrangement.swipe(fingers, overColumns: overColumns, canRead: canRead)
                },
                drag: drag,
                go: go,
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

/// Keeps a column in the view tree while it is out of sight: moved out of
/// its space by its own width (or `travel`), off the leading edge or to the
/// trailing side. Showing it again is then only a move, and what it held is
/// as it was. During a swipe that moves it, it goes as far as the fingers
/// have; only then does it read the drag, so the columns that stay put
/// aren't evaluated with every event.
private struct Sliding<Content: View>: View {
    /// Where it rests: 0 in place, -1 off the leading edge, 1 off the
    /// trailing side.
    let slot: Int
    /// Where it rests where the swipe in progress leads; nil with none on.
    let heading: Int?
    /// How far a slot is; nil for its own width.
    let travel: CGFloat?
    let drag: PaneDrag
    @ViewBuilder let content: Content
    /// Wide enough to be out of the window until it has been measured.
    @State private var width: CGFloat = 4000

    var body: some View {
        let step = (travel ?? width) + 1
        var x = CGFloat(slot) * step
        if let heading, heading != slot {
            x += CGFloat(heading > slot ? 1 : -1) * min(drag.distance, step * CGFloat(abs(heading - slot)))
        }
        return content
            // Measured only where its own width is the distance, so a window
            // being resized doesn't come through here otherwise.
            .onGeometryChange(for: CGFloat.self) { travel == nil ? $0.size.width : 0 } action: { new in
                if new > 0 { width = new }
            }
            .offset(x: x)
    }
}

/// What dims the reader while the columns are in over it, deepening as a
/// swipe brings them in.
private struct Scrim: View {
    let dimmed: Bool
    let becoming: Bool?
    /// How far the columns move to come all the way in.
    let travel: CGFloat
    let drag: PaneDrag

    var body: some View {
        var amount: CGFloat = dimmed ? 1 : 0
        if let becoming, becoming != dimmed {
            let moved = min(drag.distance / travel, 1)
            amount = dimmed ? 1 - moved : moved
        }
        return Color.black.opacity(0.18 * amount)
    }
}
