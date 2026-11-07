import EddyKit
import SwiftUI

/// One shelf item: a subscription, a publication, or a saved page.
struct Book: Identifiable, Equatable {
    enum Kind: Equatable {
        case feed(sourceID: String)
        case publication(seriesID: String)
        case saved(itemID: String)
    }

    let id: String
    let kind: Kind
    let title: String
    let detail: String
    let symbol: String
    let coverItemID: String?
    let updated: Int64
    let lastRead: Int64
    let unread: Int
    let progress: Double
}

enum ShelfSort: String, CaseIterable, Identifiable {
    case updated = "Recently Updated"
    case read = "Recently Read"
    case title = "Title"
    case unread = "Unread"
    case progress = "Progress"
    var id: String { rawValue }
}

enum ShelfFilter: String, CaseIterable, Identifiable {
    case all = "All"
    case feeds = "Subscriptions"
    case publications = "Publications"
    case saved = "Saved"
    var id: String { rawValue }
}

/// Library mode: the whole collection as one dense bookshelf.
struct LibraryView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("library.sort") private var sort: ShelfSort = .updated
    @AppStorage("library.filter") private var filter: ShelfFilter = .all
    @State private var query = ""
    @State private var open: Book?

    var body: some View {
        Group {
            if let open {
                LibraryReader(book: open) { self.open = nil }
                    .id(open.id)
            } else {
                shelf
            }
        }
        .onAppear(perform: openCurrent)
    }

    private var shelf: some View {
        let books = sortedBooks
        return ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 118, maximum: 150), spacing: 18, alignment: .top)], alignment: .leading, spacing: 22) {
                ForEach(books) { book in
                    BookTile(book: book)
                        .onTapGesture { open = book }
                        .accessibilityAddTraits(.isButton)
                        .accessibilityLabel(Text("\(book.title), \(book.detail)"))
                }
            }
            .padding(28)
        }
        .overlay {
            if books.isEmpty {
                ContentUnavailableView("Your Library Is Empty", systemImage: "books.vertical", description: Text("Save a page with ⌘L, or connect FreshRSS in Settings."))
            }
        }
        .searchable(text: $query, placement: .toolbar, prompt: "Filter")
        .toolbar {
            ToolbarItem {
                Picker("Show", selection: $filter) { ForEach(ShelfFilter.allCases) { Text($0.rawValue).tag($0) } }
                    .pickerStyle(.menu)
                    .help("Which books to show")
            }
            ToolbarItem {
                Picker("Sort By", selection: $sort) { ForEach(ShelfSort.allCases) { Text($0.rawValue).tag($0) } }
                    .pickerStyle(.menu)
                    .help("Sort order")
            }
            ToolbarItem {
                Button { Task { await model.refresh() } } label: { Label("Refresh", systemImage: "arrow.clockwise") }.disabled(model.busy)
            }
        }
    }

    /// Switching from three-pane mode keeps reading the same item.
    private func openCurrent() {
        guard open == nil, let id = model.currentItemID, let item = model.item(id) else { return }
        open = allBooks.first { book in
            switch book.kind {
            case let .feed(sourceID): item.kind == .feed && item.sourceId == sourceID
            case let .publication(seriesID): item.seriesId == seriesID
            case let .saved(itemID): itemID == id
            }
        }
    }

    private var allBooks: [Book] {
        _ = model.revision
        var books: [Book] = []
        for feed in model.feeds {
            let s = model.stats[feed.id]
            books.append(Book(
                id: "feed:\(feed.id)", kind: .feed(sourceID: feed.id), title: feed.title,
                detail: feed.unread > 0 ? "\(feed.unread) unread" : "\(s?.total ?? 0) articles",
                symbol: "dot.radiowaves.up.forward", coverItemID: nil,
                updated: s?.latestPublishedAt ?? 0, lastRead: s?.lastReadAt ?? 0, unread: Int(feed.unread),
                progress: (s?.total ?? 0) > 0 ? 1 - Double(feed.unread) / Double(s!.total) : 0
            ))
        }
        for series in model.series {
            let chapters = model.chapters(series.id)
            books.append(Book(
                id: "pub:\(series.id)", kind: .publication(seriesID: series.id), title: series.title,
                detail: "\(series.finished)/\(series.chapters) chapters", symbol: "book.closed",
                coverItemID: nil,
                updated: chapters.map(\.publishedAt).max() ?? 0, lastRead: chapters.compactMap(\.lastReadAt).max() ?? 0,
                unread: Int(series.chapters - series.finished), progress: series.progress
            ))
        }
        for item in model.items(.saved, limit: 5000) {
            books.append(Book(
                id: "saved:\(item.id)", kind: .saved(itemID: item.id), title: item.title,
                detail: item.finished ? "Finished" : item.inProgress ? "\(Int(item.progress * 100))% · \(item.minutesLeft) min left" : "\(item.readingMinutes) min · \(item.host)",
                symbol: "bookmark", coverItemID: item.id,
                updated: item.publishedAt, lastRead: item.lastReadAt ?? 0, unread: item.read ? 0 : 1, progress: item.progress
            ))
        }
        return books
    }

    private var sortedBooks: [Book] {
        var books = allBooks.filter { book in
            switch (filter, book.kind) {
            case (.all, _), (.feeds, .feed), (.publications, .publication), (.saved, .saved): true
            default: false
            }
        }
        if !query.isEmpty {
            books = books.filter { $0.title.localizedCaseInsensitiveContains(query) }
        }
        switch sort {
        case .updated: books.sort { $0.updated > $1.updated }
        case .read: books.sort { ($0.lastRead, $0.updated) > ($1.lastRead, $1.updated) }
        case .title: books.sort { $0.title.localizedStandardCompare($1.title) == .orderedAscending }
        case .unread: books.sort { ($0.unread, $0.updated) > ($1.unread, $1.updated) }
        case .progress: books.sort { ($0.progress, $0.lastRead) > ($1.progress, $1.lastRead) }
        }
        return books
    }
}

private struct BookTile: View {
    let book: Book
    @State private var hovering = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            BookCover(title: book.title, subtitle: nil, imageItemID: book.coverItemID, symbol: book.symbol)
                .aspectRatio(2 / 3, contentMode: .fit)
                .shadow(color: .black.opacity(hovering ? 0.28 : 0.16), radius: hovering ? 9 : 5, y: hovering ? 5 : 3)
                .overlay(alignment: .topTrailing) {
                    if book.unread > 0, case .feed = book.kind {
                        Text("\(book.unread)")
                            .font(.caption2.bold())
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(Capsule().fill(Color.accentColor))
                            .foregroundStyle(.white)
                            .padding(6)
                    }
                }
                .overlay(alignment: .bottom) {
                    if book.progress > 0.01, book.progress < 0.995 {
                        GeometryReader { geo in
                            Rectangle().fill(Color.white.opacity(0.9)).frame(width: geo.size.width * book.progress, height: 3)
                        }
                        .frame(height: 3)
                        .clipShape(RoundedRectangle(cornerRadius: 1.5))
                        .padding(.horizontal, 8)
                        .padding(.bottom, 6)
                    }
                }
                .scaleEffect(hovering ? 1.03 : 1)
                .animation(.easeOut(duration: 0.15), value: hovering)
            Text(book.title).font(.caption.weight(.semibold)).lineLimit(2)
            Text(book.detail).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
        }
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
    }
}
