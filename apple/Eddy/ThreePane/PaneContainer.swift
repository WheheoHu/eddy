import SwiftUI

/// Three-pane's three columns, laid out for the tier the window is in.
/// Every column stays the same view from one tier to the next and while it
/// is out of sight (moved off an edge, not removed), so the text keeps its
/// place and the lists their scroll positions however the window is
/// resized. A swipe moves the columns with the fingers: `drag` says how far.
/// A reader the columns have come in over sinks back behind them.
struct PaneContainer<Sidebar: View, Middle: View, Reader: View>: View {
    let arrangement: PaneArrangement
    /// Something is open in the reader to go on to.
    let canRead: Bool
    /// Swipes and row clicks are off: something lies over the window.
    let gesturesEnabled: Bool
    /// The swipe between columns in progress, if any.
    let drag: PaneDrag
    /// Moves to a column: a strip's button, a click beside the columns, a
    /// swipe, a click on a row.
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
        // The columns as they will be where a swipe in progress leads. The
        // columns that move ask; this body doesn't read the swipe, so its
        // starting and ending doesn't run it.
        let heading: (PanePlace) -> PaneArrangement = { PaneArrangement(tier: tier, place: $0, sidebarOpen: arrangement.sidebarOpen) }
        let travel = arrangement.leadingTravel(column)
        ZStack(alignment: .topLeading) {
            // What shows around a column that has sunk.
            Theme.well
            Sinking(sunk: !arrangement.readerShown, becoming: { !heading($0).readerShown }, travel: arrangement.coverTravel(column), drag: drag) {
                reader
                    // The placeholder for nothing selected sits in the middle.
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Theme.page)
            }
            .padding(.leading, arrangement.readerBase(column))
            // Wholly behind the list, which has sunk itself: it would show
            // through around the list's edges.
            .opacity(arrangement.listSunk ? 0 : 1)
            .allowsHitTesting(arrangement.readerShown)
            .accessibilityHidden(!arrangement.readerShown)
            // A click on the reader beside the columns over it sends them away.
            Color.clear
                .contentShape(Rectangle())
                .onTapGesture { if let back = arrangement.dismissal, back != .reader || canRead { go(back) } }
                .allowsHitTesting(arrangement.dismissal != nil)
                .accessibilityHidden(true)
            Sliding(
                slot: arrangement.listSlot, heading: { heading($0).listSlot },
                travel: arrangement.listLeadingTravel(column), push: PaneTier.sidebarWidth, drag: drag
            ) {
                Sinking(sunk: arrangement.listSunk, becoming: { heading($0).listSunk }, travel: PaneTier.sidebarWidth, drag: drag) {
                    list
                        .frame(width: tier == .narrowest ? nil : column)
                        .frame(maxHeight: .infinity)
                        // Colors, not background styles: those stay where they
                        // are when a column moves, and cover the reader.
                        .background(Theme.list)
                        .overlay(alignment: .trailing) { if tier != .narrowest { ColumnEdge() } }
                }
                .shadow(color: .black.opacity(arrangement.readerShown || tier == .narrowest ? 0 : 0.14), radius: 18, x: 8)
            }
            .padding(.leading, arrangement.listBase)
            .allowsHitTesting(arrangement.listShown)
            .accessibilityHidden(!arrangement.listShown)
            Sliding(slot: arrangement.sidebarSlot, heading: { heading($0).sidebarSlot }, travel: travel, drag: drag) {
                sidebar
                    .frame(width: PaneTier.sidebarWidth)
                    .frame(maxHeight: .infinity)
                    .background(Theme.side)
                    .overlay(alignment: .trailing) { ColumnEdge() }
                    .shadow(color: .black.opacity(arrangement.listSunk ? 0.14 : 0), radius: 18, x: 8)
            }
            .allowsHitTesting(arrangement.sidebarShown)
            .accessibilityHidden(!arrangement.sidebarShown)
            if tier >= .medium, arrangement.readerShown {
                resizeHandle.offset(x: arrangement.readerBase(column) - 4)
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
                    // A choice in the sidebar leaves the sidebar where it is.
                    if column == .list, canRead { go(.reader) }
                }
            )
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .clipped()
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
        Rectangle().fill(Theme.line).frame(width: 1).frame(maxHeight: .infinity)
    }
}

/// Keeps a column in the view tree while it is out of sight: moved out of
/// its place, off the leading edge (by `travel`, or its own width) or
/// along towards the trailing side (by `push`). Showing it again is then
/// only a move, and what it held is as it was. During a swipe that moves
/// it, it goes as far as the fingers have; only then does it read the drag,
/// so the columns that stay put aren't evaluated with every event.
struct Sliding<Content: View>: View {
    /// Where it rests: 0 in place, -1 off the leading edge, 1 pushed along.
    let slot: Int
    /// Where it rests at the place a swipe leads to.
    let heading: (PanePlace) -> Int
    /// How far slot -1 is; nil for its own width.
    let travel: CGFloat?
    /// How far slot 1 is.
    var push: CGFloat = 0
    let drag: PaneDrag
    @ViewBuilder let content: Content
    /// Wide enough to be out of the window until it has been measured.
    @State private var width: CGFloat = 4000

    var body: some View {
        var x = rest(slot)
        if let heading = drag.destination.map(heading), heading != slot {
            let to = rest(heading)
            x += (to > x ? 1 : -1) * min(drag.distance, abs(to - x))
        }
        return content
            // Measured only where its own width is the distance, so a window
            // being resized doesn't come through here otherwise.
            .onGeometryChange(for: CGFloat.self) { travel == nil ? $0.size.width : 0 } action: { new in
                if new > 0 { width = new }
            }
            .offset(x: x)
    }

    private func rest(_ slot: Int) -> CGFloat {
        slot > 0 ? push : slot < 0 ? -((travel ?? width) + 1) : 0
    }
}

/// A column the others have come in over: it sinks back, a little smaller
/// with its corners rounded, and dims. A swipe takes it there by degrees;
/// only then does it read the drag.
struct Sinking<Content: View>: View {
    let sunk: Bool
    /// Whether it is sunk at the place a swipe leads to; nil where no
    /// swipe moves it.
    let becoming: ((PanePlace) -> Bool)?
    /// How far the columns move to come all the way in; nil for the
    /// window's width.
    let travel: CGFloat?
    let drag: PaneDrag
    @ViewBuilder let content: Content
    @State private var width: CGFloat = 480

    /// How much smaller it is, sunk.
    private var depth: CGFloat { 0.06 }

    var body: some View {
        var amount: CGFloat = sunk ? 1 : 0
        if let becoming = drag.destination.flatMap({ becoming?($0) }), becoming != sunk {
            let moved = min(drag.distance / (travel ?? width), 1)
            amount = sunk ? 1 - moved : moved
        }
        return content
            .overlay { Theme.scrim.opacity(amount).allowsHitTesting(false) }
            .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.panel * amount, style: .continuous))
            .scaleEffect(1 - depth * amount)
            .onGeometryChange(for: CGFloat.self) { travel == nil ? $0.size.width : 0 } action: { new in
                if new > 0 { width = new }
            }
    }
}
