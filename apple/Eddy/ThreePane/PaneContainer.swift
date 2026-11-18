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
        // The columns as they will be where the swipe in progress leads.
        let heading = drag.destination.map { PaneArrangement(tier: tier, place: $0, sidebarOpen: arrangement.sidebarOpen) }
        let travel = arrangement.travel(column)
        ZStack(alignment: .topLeading) {
            // What shows around a reader that has sunk.
            Theme.well
            Sinking(sunk: !arrangement.readerShown, becoming: heading.map { !$0.readerShown }, travel: travel, drag: drag) {
                reader
                    // The placeholder for nothing selected sits in the middle.
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Theme.page)
            }
            .padding(.leading, arrangement.readerBase(column))
            .allowsHitTesting(arrangement.readerShown)
            .accessibilityHidden(!arrangement.readerShown)
            // A click on the reader beside the columns over it sends them away.
            Color.clear
                .contentShape(Rectangle())
                .onTapGesture { if let back = arrangement.dismissal, back != .reader || canRead { go(back) } }
                .allowsHitTesting(arrangement.dismissal != nil)
                .accessibilityHidden(true)
            Sliding(slot: arrangement.listSlot, heading: heading?.listSlot, travel: travel, drag: drag) {
                list
                    .frame(width: tier == .narrowest ? nil : column)
                    .frame(maxHeight: .infinity)
                    // Colors, not background styles: those stay where they
                    // are when a column moves, and cover the reader.
                    .background(Theme.list)
                    .overlay(alignment: .trailing) { if tier != .narrowest { ColumnEdge() } }
                    .shadow(color: .black.opacity(arrangement.readerShown || tier == .narrowest ? 0 : 0.14), radius: 18, x: 8)
            }
            .padding(.leading, arrangement.listBase)
            .allowsHitTesting(arrangement.listShown)
            .accessibilityHidden(!arrangement.listShown)
            Sliding(slot: arrangement.sidebarSlot, heading: heading?.sidebarSlot, travel: travel, drag: drag) {
                sidebar
                    .frame(width: tier == .narrowest ? nil : PaneTier.sidebarWidth)
                    .frame(maxHeight: .infinity)
                    .background(Theme.side)
                    .overlay(alignment: .trailing) { if tier != .narrowest { ColumnEdge() } }
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

/// A column the others have come in over: it sinks back, a little smaller
/// with its corners rounded, and dims. A swipe takes it there by degrees;
/// only then does it read the drag.
private struct Sinking<Content: View>: View {
    let sunk: Bool
    let becoming: Bool?
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
        if let becoming, becoming != sunk {
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
