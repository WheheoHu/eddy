import EddyKit
import SwiftUI

/// Which way to move from the item being read.
enum Direction {
    case previous, next
}

/// The items around the one being read, for every way of moving on from it:
/// Cmd+arrows, pulling past the edge of the text, holding an arrow key
/// there, and swipes through a book.
@MainActor
struct Neighbors {
    enum Kind {
        case chapter, article
    }

    typealias Open = @MainActor (_ id: String, _ direction: Direction, _ atEnd: Bool) -> Void

    let kind: Kind
    /// What a move would open; looked up when asked, since the lists change.
    let peek: @MainActor (Direction) -> ItemView?
    /// Opens it.
    let go: @MainActor (Direction) -> Void

    /// What a gesture towards `direction` would open, worded for the capsule.
    func target(_ direction: Direction) -> EdgeTarget {
        guard let item = peek(direction) else {
            switch (kind, direction) {
            case (.chapter, .next): return .end(String(localized: "This is the last chapter"))
            case (.chapter, .previous): return .end(String(localized: "This is the first chapter"))
            case (.article, .next): return .end(String(localized: "This is the last article"))
            case (.article, .previous): return .end(String(localized: "This is the first article"))
            }
        }
        let title = item.title
        return direction == .next
            ? .item(String(localized: "Next: \(title)"), release: String(localized: "Release for next: \(title)"))
            : .item(String(localized: "Previous: \(title)"), release: String(localized: "Release for previous: \(title)"))
    }

    /// Chapters in book order. A previous chapter opens at its end, the
    /// way reading backwards would reach it.
    static func chapters(
        _ chapters: @escaping @MainActor () -> [ItemView],
        current: String,
        open: @escaping Open
    ) -> Neighbors {
        let peek: @MainActor (Direction) -> ItemView? = { direction in
            let all = chapters()
            guard let index = all.firstIndex(where: { $0.id == current }) else { return nil }
            let target = index + (direction == .next ? 1 : -1)
            return all.indices.contains(target) ? all[target] : nil
        }
        return Neighbors(kind: .chapter, peek: peek) { direction in
            guard let target = peek(direction) else { return }
            open(target.id, direction, direction == .previous)
        }
    }

    /// Articles of a list: back and forward through what was read first,
    /// like a browser, then the rows above and below the current one.
    static func articles(
        _ rows: @escaping @MainActor () -> [ItemView],
        current: String,
        history: ReadingHistory,
        item: @escaping @MainActor (String) -> ItemView?,
        open: @escaping Open
    ) -> Neighbors {
        let row: @MainActor (Direction) -> ItemView? = { direction in
            let all = rows()
            guard let index = all.firstIndex(where: { $0.id == current }) else { return nil }
            let target = index + (direction == .next ? 1 : -1)
            return all.indices.contains(target) ? all[target] : nil
        }
        let peek: @MainActor (Direction) -> ItemView? = { direction in
            history.prune { item($0) != nil }
            if let id = history.peek(direction), let found = item(id) { return found }
            return row(direction)
        }
        return Neighbors(kind: .article, peek: peek) { direction in
            history.prune { item($0) != nil }
            if let id = history.move(direction) {
                open(id, direction, false)
            } else if let target = row(direction) {
                open(target.id, direction, false)
            }
        }
    }
}

/// Back and forward through the articles of one list, like a browser.
/// Opening an article any other way than back or forward pushes the one
/// being left and clears forward. Lives in memory, one per list. Nothing
/// on screen shows it, so it isn't observed.
@MainActor
final class ReadingHistory {
    private var back: [String] = []
    private var forward: [String] = []
    private var current: String?
    /// The next `opened` comes from `move` and is already accounted for.
    private var moving: String?

    private static let limit = 200

    /// Every change of the article being read goes through here.
    func opened(_ id: String?) {
        guard id != current else { return }
        defer { current = id }
        if let moving, moving == id {
            self.moving = nil
            return
        }
        moving = nil
        guard let id else { return }
        if let current, current != id {
            back.append(current)
            if back.count > Self.limit { back.removeFirst() }
        }
        forward.removeAll()
    }

    /// Starts over, for a new list.
    func reset(_ id: String?) {
        back.removeAll()
        forward.removeAll()
        moving = nil
        current = id
    }

    func peek(_ direction: Direction) -> String? {
        direction == .previous ? back.last : forward.last
    }

    /// Steps back or forward; the article to open, if there is one.
    func move(_ direction: Direction) -> String? {
        let target: String?
        switch direction {
        case .previous:
            target = back.popLast()
            if target != nil, let current { forward.append(current) }
        case .next:
            target = forward.popLast()
            if target != nil, let current { back.append(current) }
        }
        moving = target
        return target
    }

    /// Drops articles that are gone.
    func prune(_ keep: (String) -> Bool) {
        back.removeAll { !keep($0) }
        forward.removeAll { !keep($0) }
    }
}
