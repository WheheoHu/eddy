import EddyKit
import SwiftUI

extension CollectionEntryView: @retroactive Identifiable {}

extension CollectionKindView: @retroactive Identifiable {
    public var id: Self { self }

    static let all: [CollectionKindView] = [.book, .pdf, .webPage, .webSeries, .media]

    /// Stable name for stored preferences.
    var key: String {
        switch self {
        case .book: "book"
        case .pdf: "pdf"
        case .webPage: "webPage"
        case .webSeries: "webSeries"
        case .media: "media"
        }
    }

    /// Plural, for sidebars and section headers.
    var title: LocalizedStringKey {
        switch self {
        case .book: "Books"
        case .pdf: "PDFs"
        case .webPage: "Web Pages"
        case .webSeries: "Web Series"
        case .media: "Media"
        }
    }

    /// `title` as a string, for lists built from data.
    var pluralName: String {
        switch self {
        case .book: String(localized: "Books")
        case .pdf: String(localized: "PDFs")
        case .webPage: String(localized: "Web Pages")
        case .webSeries: String(localized: "Web Series")
        case .media: String(localized: "Media")
        }
    }

    var name: String {
        switch self {
        case .book: String(localized: "Book")
        case .pdf: String(localized: "PDF")
        case .webPage: String(localized: "Web Page")
        case .webSeries: String(localized: "Web Series")
        case .media: String(localized: "Media")
        }
    }

    var icon: String {
        switch self {
        case .book: "book.closed"
        case .pdf: "doc.richtext"
        case .webPage: "globe"
        case .webSeries: "list.bullet.rectangle"
        case .media: "play.rectangle"
        }
    }
}

/// Which of the items being read Continue Reading lists: all of them, the
/// subscription articles, or what is in the collection.
enum ReadingScope: String, Hashable {
    case all, subscriptions, collection

    /// Where the choice is kept; Three-Pane and the Library share it.
    static let key = "continueReading.scope"

    func contains(_ item: ItemView) -> Bool {
        switch self {
        case .all: true
        case .subscriptions: item.kind == .feed
        case .collection: item.kind != .feed
        }
    }

    static var options: [SwitchBar<ReadingScope>.Option] {
        [
            .symbol(.all, "line.3.horizontal", help: String(localized: "All")),
            .symbol(.subscriptions, "dot.radiowaves.up.forward", help: String(localized: "Subscriptions")),
            .symbol(.collection, "books.vertical", help: String(localized: "Collection")),
        ]
    }
}

extension CollectionEntryView {
    var isSeries: Bool { seriesId != nil }

    /// Read to the end, all of it: "Mark as Read" has nothing left to mark.
    var isDone: Bool { finished >= chapters }

    var host: String? {
        url.flatMap { URL(string: $0)?.host() }.map { $0.replacingOccurrences(of: "www.", with: "") }
    }

    /// Author, else the site.
    var byline: String? { author ?? host }

    var inProgress: Bool { progress > 0.01 && progress < 0.995 }

    /// Started or finished: "Mark as Unread" has something to reset.
    var hasBeenRead: Bool { progress > 0 || finished > 0 || lastReadAt != nil }

    var readingMinutes: Int {
        let perMinute = (lang == "cmn" || lang == "jpn") ? 400.0 : 1200.0
        return max(1, Int((Double(textLen) / perMinute).rounded()))
    }

    /// Progress in words: chapters for a series, minutes or a percentage
    /// for a single page or PDF.
    var progressText: String {
        if isSeries { return String(localized: "\(finished) of \(chapters) chapters") }
        if progress >= 0.995 || finished > 0 { return String(localized: "Finished") }
        if kind == .pdf { return inProgress ? "\(Int(progress * 100))%" : String(localized: "PDF") }
        let left = max(1, Int(Double(readingMinutes) * (1 - progress)))
        return inProgress ? String(localized: "\(left) min left") : String(localized: "\(readingMinutes) min")
    }

}

/// How a list of collection entries is ordered.
enum CollectionSort: String, CaseIterable, Identifiable {
    case added = "Recently Added"
    case read = "Recently Read"
    case title = "Title"
    case progress = "Progress"

    var id: String { rawValue }

    func inOrder(_ a: CollectionEntryView, _ b: CollectionEntryView) -> Bool {
        switch self {
        case .added: a.addedAt > b.addedAt
        case .read: (a.lastReadAt ?? 0, a.addedAt) > (b.lastReadAt ?? 0, b.addedAt)
        case .title: a.title.localizedStandardCompare(b.title) == .orderedAscending
        case .progress: (a.progress, a.lastReadAt ?? 0) > (b.progress, b.lastReadAt ?? 0)
        }
    }
}

/// One collection entry in a list.
struct CollectionRow: View {
    let entry: CollectionEntryView

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: entry.kind.icon)
                .font(.system(size: 15))
                .foregroundStyle(Theme.text2)
                .frame(width: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.title)
                    .font(.system(size: 15, weight: entry.hasBeenRead ? .regular : .semibold))
                    .foregroundStyle(entry.hasBeenRead ? Theme.text2 : Theme.text1)
                    .lineLimit(2)
                if let byline = entry.byline {
                    Text(byline).font(.system(size: 12)).foregroundStyle(Theme.text2).lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            Text(entry.progressText).font(.system(size: 12)).foregroundStyle(Theme.text3).lineLimit(1)
            if entry.progress >= 0.995 {
                Image(systemName: "checkmark").font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.ok).accessibilityLabel("Finished")
            } else if entry.inProgress {
                ProgressRing(value: entry.progress, size: 12)
            }
        }
        .padding(.vertical, 9)
        .overlay(alignment: .bottom) { Hairline().padding(.leading, 30) }
    }
}

/// A small filter field at the top of a contents column.
struct FilterField: View {
    @Binding var text: String
    var prompt: LocalizedStringKey = "Filter"

    var body: some View {
        HStack(spacing: 5) {
            Image(systemName: "line.3.horizontal.decrease").foregroundStyle(Theme.text3)
            TextField(prompt, text: $text).textFieldStyle(.plain).font(.system(size: 13))
            if !text.isEmpty {
                Button { text = "" } label: { Image(systemName: "xmark.circle.fill") }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Clear Filter")
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .frame(minHeight: 28)
        .background(Theme.track, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
    }
}

extension String {
    func matches(filter: String) -> Bool {
        filter.isEmpty || localizedCaseInsensitiveContains(filter)
    }
}
