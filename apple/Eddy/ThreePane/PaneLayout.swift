import SwiftUI

/// How many of three-pane's columns the window has room for. The width
/// alone decides it.
enum PaneTier: Int, Comparable {
    /// One column at a time, as on a phone: sidebar, list, reader.
    case narrowest
    /// The reader alone; the sidebar and list come in over it.
    case narrow
    /// The list and the reader; the sidebar comes in beside the list, over
    /// the reader.
    case medium
    /// All three side by side.
    case wide

    static let sidebarWidth: CGFloat = 240
    static let listWidth: CGFloat = 360
    /// The narrowest reader that still holds the shortest line the text
    /// style allows (520) with its margins.
    static let readerWidth: CGFloat = 576
    static let minWindowWidth: CGFloat = 480
    /// A tier is kept until the window is this much narrower than where it
    /// starts, so a window resting on a boundary doesn't flicker.
    static let slack: CGFloat = 16

    /// The narrowest window the tier is entered at.
    var floor: CGFloat {
        switch self {
        case .wide: Self.sidebarWidth + Self.listWidth + Self.readerWidth + 4
        case .medium: Self.listWidth + Self.readerWidth + 4
        // Room beside the columns that come in, to click the reader behind.
        case .narrow: Self.sidebarWidth + Self.listWidth + 100
        case .narrowest: 0
        }
    }

    /// The tier for a window `width` wide that is in `current` now.
    static func at(_ width: CGFloat, from current: PaneTier) -> PaneTier {
        for tier in [PaneTier.wide, .medium, .narrow] where width >= tier.floor - (current >= tier ? slack : 0) {
            return tier
        }
        return .narrowest
    }

    static func < (a: PaneTier, b: PaneTier) -> Bool { a.rawValue < b.rawValue }
}

/// The column the reader is at, in the tiers that don't show all of them:
/// which page of the narrowest, whether the columns are in over the reader
/// in the narrow one, whether the sidebar is in beside the list (over the
/// reader) in the medium one.
enum PanePlace: Int, Comparable {
    case sidebar, list, reader

    static func < (a: PanePlace, b: PanePlace) -> Bool { a.rawValue < b.rawValue }
}

/// What the columns are doing over or beside the reader, from the tier and
/// the place: the one description the container, the strips, the swipes
/// and the clicks all go by.
///
/// Columns to the left lie over what is to their right: a column that comes
/// in covers the reader, which sinks back (a little smaller, and dimmed).
/// The exception is the sidebar over the list in the narrowest tier, which
/// pushes the list out instead: two lists one over the other under the
/// pointer send AppKit's hover tracking into a loop that ends the app.
struct PaneArrangement: Equatable {
    let tier: PaneTier
    let place: PanePlace
    /// The sidebar is open, in the wide tier where it has a column to itself.
    var sidebarOpen = true

    var sidebarShown: Bool {
        switch tier {
        case .wide: sidebarOpen
        case .medium, .narrowest: place == .sidebar
        case .narrow: place != .reader
        }
    }

    var listShown: Bool {
        switch tier {
        case .wide, .medium: true
        case .narrow: place != .reader
        case .narrowest: place == .list
        }
    }

    /// The reader is on screen with nothing over it.
    var readerShown: Bool {
        switch tier {
        case .wide: true
        case .medium: place != .sidebar
        case .narrow, .narrowest: place == .reader
        }
    }

    /// The column whose strip has the window's buttons over it.
    var leading: PanePlace {
        switch tier {
        case .wide: sidebarOpen ? .sidebar : .list
        case .medium: place == .sidebar ? .sidebar : .list
        case .narrow: place == .reader ? .reader : .sidebar
        case .narrowest: place
        }
    }

    /// Where the list rests, in steps of `listTravel` from its place: 0 in
    /// it, -1 towards the leading edge, 1 towards the trailing one.
    var listSlot: Int {
        switch tier {
        case .wide: sidebarOpen ? 1 : 0
        case .medium: place == .sidebar ? 1 : 0
        case .narrow: place == .reader ? -1 : 0
        case .narrowest: place == .sidebar ? 1 : (place == .reader ? -1 : 0)
        }
    }

    /// Where the sidebar rests: 0 in its place, -1 off the leading edge.
    var sidebarSlot: Int { sidebarShown ? 0 : -1 }

    /// How far a step of the sidebar and of the list is; nil for the
    /// column's own width (the whole window, in the narrowest tier).
    func travel(_ column: CGFloat) -> CGFloat? {
        switch tier {
        case .wide, .medium: PaneTier.sidebarWidth
        case .narrow: PaneTier.sidebarWidth + column
        case .narrowest: nil
        }
    }

    /// The list's leading edge with nothing moved: beside the sidebar in the
    /// narrow tier, where the two come and go together.
    var listBase: CGFloat { tier == .narrow ? PaneTier.sidebarWidth : 0 }

    /// The reader's leading edge.
    func readerBase(_ column: CGFloat) -> CGFloat {
        switch tier {
        case .wide: (sidebarOpen ? PaneTier.sidebarWidth : 0) + column
        case .medium: column
        case .narrow, .narrowest: 0
        }
    }

    /// The list's width where it is one of two or three columns, from the
    /// width it was dragged to.
    func columnWidth(_ dragged: CGFloat) -> CGFloat {
        tier == .narrow ? min(dragged, PaneTier.listWidth) : dragged
    }

    /// How far from the leading edge the columns reach; nil for the whole
    /// width.
    func columnsExtent(_ dragged: CGFloat) -> CGFloat? {
        switch tier {
        case .wide, .medium: (sidebarShown ? PaneTier.sidebarWidth : 0) + dragged
        case .narrow: place == .reader ? 0 : PaneTier.sidebarWidth + columnWidth(dragged)
        case .narrowest: place == .reader ? 0 : nil
        }
    }

    /// How far from the leading edge the sidebar reaches, where a click on
    /// one of its rows goes on to the list; nil for the whole width.
    func sidebarExtent(_ dragged: CGFloat) -> CGFloat? {
        guard sidebarShown else { return 0 }
        switch tier {
        case .wide: return 0
        case .medium, .narrow: return PaneTier.sidebarWidth
        case .narrowest: return nil
        }
    }

    /// A click on a row of the list goes on to the reader: where the list
    /// covers it.
    var listLeadsOn: Bool {
        switch tier {
        case .wide, .medium: false
        case .narrow, .narrowest: listShown
        }
    }

    /// Where a click on the reader beside the columns that cover it leads:
    /// they go away again.
    var dismissal: PanePlace? {
        switch tier {
        case .medium: place == .sidebar ? .list : nil
        case .narrow: place != .reader ? .reader : nil
        case .wide, .narrowest: nil
        }
    }

    /// Where a sideways swipe leads from here, with the pointer over the
    /// columns or beside them; nil where it leads nowhere. `canRead`:
    /// something is open in the reader to go on to.
    func swipe(_ fingers: SwipeDirection, overColumns: Bool, canRead: Bool) -> PanePlace? {
        switch (tier, fingers) {
        case (.wide, _):
            return nil
        case (.medium, .right):
            return overColumns && place != .sidebar ? .sidebar : nil
        case (.medium, .left):
            return place == .sidebar ? .list : nil
        case (.narrow, .right):
            return place == .reader ? .list : nil
        case (.narrow, .left):
            return place != .reader && canRead ? .reader : nil
        case (.narrowest, .right):
            return place == .sidebar ? nil : PanePlace(rawValue: place.rawValue - 1)
        case (.narrowest, .left):
            switch place {
            case .sidebar: return .list
            case .list: return canRead ? .reader : nil
            case .reader: return nil
            }
        }
    }
}

/// A swipe between the columns while the fingers are down: the column it
/// leads to and how far the fingers have come. The columns that move read
/// it and follow; nothing else does, so a swipe doesn't run three-pane's
/// body with every event.
@MainActor
@Observable
final class PaneDrag {
    /// How far the fingers move before letting go goes to the column;
    /// short of it the columns go back where they were.
    static let commit: CGFloat = 90

    /// Nil when no swipe is on.
    private(set) var destination: PanePlace?
    /// In points, along the swipe.
    private(set) var distance: CGFloat = 0

    func move(to destination: PanePlace, _ distance: CGFloat) {
        if self.destination != destination { self.destination = destination }
        self.distance = max(distance, 0)
    }

    /// The fingers are up: the columns are where the place puts them
    /// again. Called in an animation, they glide the rest of the way.
    func end() {
        guard destination != nil else { return }
        destination = nil
        distance = 0
    }

    /// The fingers let go short of going anywhere.
    func cancel() {
        withAnimation(.easeOut(duration: 0.2)) { end() }
    }
}
