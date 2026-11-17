import SwiftUI

/// How many of three-pane's columns the window has room for. The width
/// alone decides it.
enum PaneTier: Int, Comparable {
    /// One column at a time, as on a phone: sidebar, list, reader.
    case narrowest
    /// The reader alone; the sidebar and list come in over it.
    case narrow
    /// The list and the reader; the sidebar takes the list's place.
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
/// in the narrow one, whether the sidebar stands in for the list in the
/// medium one.
enum PanePlace: Int, Comparable {
    case sidebar, list, reader

    static func < (a: PanePlace, b: PanePlace) -> Bool { a.rawValue < b.rawValue }
}

/// What the columns are doing over or beside the reader, from the tier and
/// the place: the one description the container, the toolbar, the swipes
/// and the clicks all go by.
struct PaneArrangement: Equatable {
    let tier: PaneTier
    let place: PanePlace

    /// The container shows the sidebar; in the wide tier the split view
    /// shows its own.
    var sidebarShown: Bool {
        switch tier {
        case .wide: false
        case .medium, .narrowest: place == .sidebar
        case .narrow: place != .reader
        }
    }

    var listShown: Bool {
        switch tier {
        case .wide: true
        case .medium: place != .sidebar
        case .narrow: place != .reader
        case .narrowest: place == .list
        }
    }

    /// The reader is on screen with nothing over it.
    var readerShown: Bool {
        switch tier {
        case .wide, .medium: true
        case .narrow, .narrowest: place == .reader
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
        case .wide, .medium: dragged
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
        case .medium: return dragged
        case .narrow: return PaneTier.sidebarWidth
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
            return overColumns && place == .sidebar ? .list : nil
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

    /// What the capsule says of a swipe that leads to `place`.
    func target(_ place: PanePlace) -> EdgeTarget {
        switch place {
        case .sidebar:
            .reveal(String(localized: "Sidebar"), release: String(localized: "Release for the sidebar"))
        case .list where tier == .narrow:
            .reveal(String(localized: "Sidebar and List"), release: String(localized: "Release for the sidebar and list"))
        case .list:
            .reveal(String(localized: "List"), release: String(localized: "Release for the list"))
        case .reader:
            .reveal(String(localized: "Reader"), release: String(localized: "Release for the reader"))
        }
    }
}
