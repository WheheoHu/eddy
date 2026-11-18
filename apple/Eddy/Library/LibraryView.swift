import EddyKit
import SwiftUI

/// One shelf item: a subscription or a collection entry.
struct Book: Identifiable, Equatable {
    enum Kind: Equatable {
        case feed(sourceID: String)
        case entry(CollectionEntryView)
    }

    let id: String
    let kind: Kind
    let title: String
    var subtitle: String?
    /// Author, site or category.
    let byline: String
    let typeName: String
    var category: String?
    let detail: String
    let symbol: String
    var coverItemID: String?
    var coverURL: String?
    var pdfItemID: String?
    let updated: Int64
    let lastRead: Int64
    let added: Int64
    let unread: Int
    let progress: Double

    var entry: CollectionEntryView? {
        if case let .entry(entry) = kind { entry } else { nil }
    }
}

enum ShelfSort: String, CaseIterable, Identifiable {
    case updated = "Recently Updated"
    case read = "Recently Read"
    case title = "Title"
    case unread = "Unread"
    case progress = "Progress"
    var id: String { rawValue }
}

enum ShelfLayout: String {
    case covers, list
}

/// What the library sidebar shows.
enum LibraryFilter: Hashable, RawRepresentable {
    case all, continueReading, saved, kind(CollectionKindView), subscriptions, category(String)

    init?(rawValue: String) {
        switch rawValue {
        case "all": self = .all
        case "continue": self = .continueReading
        case "saved": self = .saved
        case "subscriptions": self = .subscriptions
        default:
            if rawValue.hasPrefix("kind:"), let kind = CollectionKindView.all.first(where: { "kind:\($0.key)" == rawValue }) {
                self = .kind(kind)
            } else if rawValue.hasPrefix("category:") {
                self = .category(String(rawValue.dropFirst("category:".count)))
            } else {
                return nil
            }
        }
    }

    var rawValue: String {
        switch self {
        case .all: "all"
        case .continueReading: "continue"
        case .saved: "saved"
        case let .kind(kind): "kind:\(kind.key)"
        case .subscriptions: "subscriptions"
        case let .category(name): "category:\(name)"
        }
    }

    /// The name over the shelf.
    var title: Text {
        switch self {
        case .all: Text("Library")
        case .continueReading: Text("Continue Reading")
        case .saved: Text("Saved")
        case let .kind(kind): Text(kind.title)
        case .subscriptions: Text("All Subscriptions")
        case let .category(name): Text(verbatim: name)
        }
    }

    /// Subscriptions have no covers, so they start as a list.
    var defaultLayout: ShelfLayout {
        switch self {
        case .subscriptions, .category: .list
        default: .covers
        }
    }

    func contains(_ book: Book) -> Bool {
        switch (self, book.kind) {
        case (.all, _), (.saved, .entry), (.subscriptions, .feed): true
        case let (.continueReading, .entry(entry)): entry.inProgress
        case let (.kind(kind), .entry(entry)): entry.kind == kind
        case let (.category(name), .feed): book.category == name
        default: false
        }
    }
}

/// Library mode: filters in the sidebar, the matching books as covers or a
/// list; opening one swaps in its contents and the text.
struct LibraryView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("app.mode") private var mode: AppMode = .threePane
    @AppStorage("library.sort") private var sort: ShelfSort = .updated
    @AppStorage("library.filter") private var filter: LibraryFilter = .all
    @AppStorage("library.sidebar") private var showSidebar = true
    /// Whether the window has room for the sidebar beside the shelf.
    @State private var dock = ColumnDock()
    /// Covers or list, per filter.
    @AppStorage("library.layouts") private var layoutsJSON = "{}"
    @AppStorage("library.columns") private var columnsData = Data()
    @State private var columns = TableColumnCustomization<Book>()
    @State private var tableSort = [KeyPathComparator(\Book.updated, order: .reverse)]
    @State private var tableSelection: Set<Book.ID> = []
    @State private var query = ""
    @State private var open: Book?
    /// Scrolled back into view when the shelf returns.
    @State private var lastOpened: String?
    @FocusState private var focus: PaneFocus?
    /// The cover picked with the arrow keys.
    @State private var selectedBook: Book.ID?
    /// Columns the covers grid lays out, for Up and Down.
    @State private var gridColumns = 1
    /// Coming back from a book: the shelf takes focus once it is there.
    @State private var focusShelf = false

    var body: some View {
        Group {
            if let open {
                LibraryReader(book: open, mode: $mode, dock: dock, close: close)
                    .id(open.id)
            } else {
                shelf
            }
        }
        .docks(dock, model: model)
        .onAppear(perform: openCurrent)
        .onChange(of: model.revealed) { _, revealed in
            guard let revealed else { return }
            model.revealed = nil
            if let book = allBooks.first(where: { $0.entry?.id == revealed }) { show(book) }
        }
        .onChange(of: model.revision) {
            if let entry = open?.entry, model.entry(entry.id) == nil { open = nil }
        }
        .onChange(of: model.navigation) { _, request in
            switch request?.value {
            case let .library(destination):
                model.navigation = nil
                open = nil
                query = ""
                filter = destination
                focusShelf = true
                DispatchQueue.main.async { claimShelfFocus() }
            case let .libraryFeed(sourceID):
                model.navigation = nil
                if let book = allBooks.first(where: { $0.kind == .feed(sourceID: sourceID) }) { show(book) }
            default:
                break
            }
        }
        .onChange(of: model.revealedItem, initial: true) { _, request in
            guard let id = request?.value else { return }
            model.revealedItem = nil
            reveal(id)
        }
    }

    // MARK: Shelf

    private var shelf: some View {
        LeadingColumn(dock: dock, beside: $showSidebar, width: PaneTier.sidebarWidth, gesturesEnabled: !model.overlayVisible) {
            SidebarColumn(dock: dock, beside: $showSidebar) {
                LibrarySidebar(selection: $filter, focus: $focus, onReturn: enterShelf)
            }
        } content: {
            let books = shelfBooks
            VStack(spacing: 0) {
                ContentStrip(dock: dock, beside: $showSidebar) {
                    HStack(spacing: 6) {
                        Image(systemName: "magnifyingglass").font(.system(size: 12)).foregroundStyle(Theme.text3)
                        TextField("Filter", text: $query).textFieldStyle(.plain).font(.system(size: 13))
                    }
                    .padding(.horizontal, 8)
                    .frame(minWidth: 70, maxWidth: 170, minHeight: 28, maxHeight: 28)
                    .background(Theme.track, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                    if layout == .covers {
                        FlatMenu("Sort By", symbol: "arrow.up.arrow.down", help: "Sort order") {
                            Picker("Sort By", selection: $sort) { ForEach(ShelfSort.allCases) { Text($0.rawValue).tag($0) } }
                                .pickerStyle(.inline)
                        }
                    }
                    SwitchBar(
                        selection: layoutBinding,
                        options: [
                            .symbol(.covers, "square.grid.2x2", help: String(localized: "Covers")),
                            .symbol(.list, "list.bullet", help: String(localized: "List")),
                        ],
                        segment: 34
                    )
                    .padding(.leading, 4)
                }
                ColumnTitle(title: filter.title, note: Text("^[\(books.count) Item](inflect: true)"))
                if layout == .list {
                    table(books)
                } else {
                    covers(books)
                }
            }
            .overlay {
                if books.isEmpty {
                    if allBooks.isEmpty {
                        ContentUnavailableView("Your Library Is Empty", systemImage: "books.vertical", description: Text("Save a page with ⌘L, import an EPUB or PDF with ⌘O, or connect FreshRSS in Settings."))
                    } else {
                        ContentUnavailableView("Nothing Here", systemImage: "tray")
                    }
                }
            }
        }
        .onAppear {
            columns = (try? JSONDecoder().decode(TableColumnCustomization<Book>.self, from: columnsData)) ?? columns
        }
        .onChange(of: columns) {
            columnsData = (try? JSONEncoder().encode(columns)) ?? columnsData
        }
    }

    private func covers(_ books: [Book]) -> some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: coverWidth, maximum: coverWidth + 32), spacing: coverSpacing, alignment: .top)], alignment: .leading, spacing: 22) {
                    ForEach(books) { book in
                        BookTile(book: book, selected: focus == .list && selectedBook == book.id)
                            .onTapGesture {
                                selectedBook = book.id
                                show(book)
                            }
                            .contextMenu { menu(for: book) }
                            .accessibilityAddTraits(.isButton)
                            .accessibilityLabel(Text("\(book.title), \(book.detail)"))
                    }
                }
                .padding(shelfPadding)
                .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { gridColumns = columns(in: $0) }
            }
            .focusable()
            .focusEffectDisabled()
            .pane($focus, .list, onReturn: { if let book = books.first(where: { $0.id == selectedBook }) { show(book) } }, onEscape: { focus = .sidebar })
            .onKeyPress(keys: [.upArrow, .downArrow, .leftArrow, .rightArrow]) { press in
                moveSelection(press.key, in: books, proxy: proxy)
            }
            .onAppear {
                if let lastOpened { proxy.scrollTo(lastOpened, anchor: .center) }
                claimShelfFocus()
            }
        }
    }

    /// Covers are smaller in a narrow window, so that a screen of the
    /// shelf still holds a dozen books.
    private var coverWidth: CGFloat {
        switch dock.tier {
        case .narrowest: 96
        case .narrow: 108
        default: 118
        }
    }
    private var coverSpacing: CGFloat { dock.tier == .narrowest ? 14 : 18 }
    private var shelfPadding: CGFloat { dock.tier == .narrowest ? 16 : 28 }

    /// How many columns the adaptive grid fits in `width`.
    private func columns(in width: CGFloat) -> Int {
        max(1, Int((width - 2 * shelfPadding + coverSpacing) / (coverWidth + coverSpacing)))
    }

    /// Left and Right move one cover, Up and Down one row; Down past the
    /// last full row lands on the last cover.
    private func moveSelection(_ key: KeyEquivalent, in books: [Book], proxy: ScrollViewProxy) -> KeyPress.Result {
        guard !books.isEmpty else { return .ignored }
        guard let index = books.firstIndex(where: { $0.id == selectedBook }) else {
            selectedBook = books[0].id
            return .handled
        }
        let step = switch key {
        case .leftArrow: -1
        case .rightArrow: 1
        case .upArrow: -gridColumns
        default: gridColumns
        }
        var target = index + step
        if target >= books.count, key == .downArrow, index / gridColumns < (books.count - 1) / gridColumns { target = books.count - 1 }
        guard books.indices.contains(target) else { return .handled }
        selectedBook = books[target].id
        withAnimation(.easeInOut(duration: 0.15)) { proxy.scrollTo(books[target].id) }
        return .handled
    }

    private func table(_ books: [Book]) -> some View {
        let rows = books.sorted(using: tableSort)
        return ScrollViewReader { proxy in
            Table(rows, selection: $tableSelection, sortOrder: $tableSort, columnCustomization: $columns) {
                TableColumn("Title", value: \.title) { book in
                    Label { Text(book.title).lineLimit(1) } icon: { Image(systemName: book.symbol).foregroundStyle(.secondary) }
                }
                .width(min: 180, ideal: 320)
                .customizationID("title")
                .disabledCustomizationBehavior(.visibility)
                TableColumn("Type", value: \.typeName)
                    .width(min: 60, ideal: 90)
                    .customizationID("type")
                TableColumn("Author/Site", value: \.byline)
                    .width(min: 80, ideal: 160)
                    .customizationID("byline")
                TableColumn("Unread", value: \.unread) { book in
                    Text(book.unread > 0 ? "\(book.unread)" : "–").monospacedDigit()
                }
                .width(min: 44, ideal: 60)
                .customizationID("unread")
                TableColumn("Progress", value: \.progress) { book in
                    ProgressView(value: book.progress).frame(maxWidth: 80)
                }
                .width(min: 60, ideal: 90)
                .customizationID("progress")
                TableColumn("Last Read", value: \.lastRead) { book in DateCell(ms: book.lastRead) }
                    .width(min: 70, ideal: 110)
                    .customizationID("lastRead")
                TableColumn("Updated", value: \.updated) { book in DateCell(ms: book.updated) }
                    .width(min: 70, ideal: 110)
                    .customizationID("updated")
                TableColumn("Added", value: \.added) { book in DateCell(ms: book.added) }
                    .width(min: 70, ideal: 110)
                    .customizationID("added")
            }
            .contextMenu(forSelectionType: Book.ID.self) { ids in
                if ids.count == 1, let book = rows.first(where: { ids.contains($0.id) }) {
                    Button("Open") { show(book) }
                    menu(for: book)
                }
            } primaryAction: { ids in
                if let book = rows.first(where: { ids.contains($0.id) }) { show(book) }
            }
            .pane($focus, .list, onReturn: { if let book = rows.first(where: { tableSelection.contains($0.id) }) { show(book) } }, onEscape: { focus = .sidebar })
            .onDeleteCommand {
                if tableSelection.count == 1, let entry = rows.first(where: { tableSelection.contains($0.id) })?.entry {
                    model.requestDelete(entry)
                }
            }
            .onAppear {
                if let lastOpened {
                    tableSelection = [lastOpened]
                    proxy.scrollTo(lastOpened, anchor: .center)
                }
                claimShelfFocus()
            }
        }
    }

    @ViewBuilder
    private func menu(for book: Book) -> some View {
        if let entry = book.entry {
            if entry.hasBeenRead {
                Button("Mark as Unread") { model.markUnread(entry: entry) }
            }
            if entry.kind != .media {
                Menu("Export") {
                    Button("Export as PDF…") { model.beginExport(targetID: entry.id, format: .pdf, fromReader: false) }
                    Button("Export as EPUB…") { model.beginExport(targetID: entry.id, format: .epub, fromReader: false) }
                }
            }
            if entry.kind == .webPage {
                Button("Convert to Web Series…") { Task { await model.convertToSeries(entry) } }
            }
            if let rawURL = entry.url, isVideoUrl(url: rawURL) {
                Button("Convert to Book…") {
                    model.requestVideoConvert(source: rawURL, title: entry.title)
                }
            }
            Button("Delete…", role: .destructive) { model.requestDelete(entry) }
        }
    }

    private var layouts: [String: String] {
        (try? JSONDecoder().decode([String: String].self, from: Data(layoutsJSON.utf8))) ?? [:]
    }

    private var layout: ShelfLayout {
        layouts[filter.rawValue].flatMap(ShelfLayout.init(rawValue:)) ?? filter.defaultLayout
    }

    private var layoutBinding: Binding<ShelfLayout> {
        Binding {
            layout
        } set: { new in
            var all = layouts
            all[filter.rawValue] = new.rawValue
            layoutsJSON = (try? String(data: JSONEncoder().encode(all), encoding: .utf8)) ?? layoutsJSON
        }
    }

    // MARK: Navigation

    private func show(_ book: Book) {
        lastOpened = book.id
        open = book
    }

    /// Back to the shelf, with the book just read selected.
    private func close() {
        open = nil
        selectedBook = lastOpened
        tableSelection = lastOpened.map { [$0] } ?? []
        focusShelf = true
    }

    private func claimShelfFocus() {
        guard focusShelf else { return }
        focusShelf = false
        DispatchQueue.main.async { focus = .list }
    }

    /// Return in the sidebar: the shelf takes focus, on the book read last
    /// when it is in this filter, else the first one.
    private func enterShelf() {
        let books = layout == .list ? shelfBooks.sorted(using: tableSort) : shelfBooks
        let pick = books.first { $0.id == lastOpened }?.id ?? books.first?.id
        selectedBook = pick
        tableSelection = pick.map { [$0] } ?? []
        focus = .list
    }

    /// Switching from three-pane mode keeps reading the same item.
    private func openCurrent() {
        guard open == nil, let id = model.currentItemID else { return }
        reveal(id)
    }

    /// Opens the book an item belongs to, at that item: its entry, or its
    /// subscription. A book already open moves to the item itself.
    private func reveal(_ id: String) {
        guard let item = model.item(id) else { return }
        let target: Book? = if let entry = model.entry(containing: item) {
            allBooks.first { $0.entry?.id == entry.id }
        } else if let sourceID = item.sourceId, item.kind == .feed {
            allBooks.first { $0.kind == .feed(sourceID: sourceID) }
        } else {
            nil
        }
        guard let target else { return }
        model.currentItemID = id
        if open?.id != target.id { show(target) }
    }

    // MARK: Books

    private var allBooks: [Book] {
        _ = model.revision
        var books: [Book] = []
        for feed in model.feeds {
            let s = model.stats[feed.id]
            let total = Int(s?.total ?? 0)
            books.append(Book(
                id: "feed:\(feed.id)", kind: .feed(sourceID: feed.id), title: feed.title,
                byline: feed.category ?? feed.siteUrl.flatMap { URL(string: $0)?.host() } ?? "",
                typeName: String(localized: "Subscription"), category: feed.category ?? "Subscriptions",
                detail: feed.unread > 0 ? String(localized: "\(feed.unread) unread") : String(localized: "\(total) articles"),
                symbol: "dot.radiowaves.up.forward",
                updated: s?.latestPublishedAt ?? 0, lastRead: s?.lastReadAt ?? 0, added: 0, unread: Int(feed.unread),
                progress: total > 0 ? 1 - Double(feed.unread) / Double(total) : 0
            ))
        }
        for entry in model.collection {
            books.append(Book(
                id: "entry:\(entry.id)", kind: .entry(entry), title: entry.title,
                subtitle: entry.kind == .book || entry.kind == .pdf ? entry.author : nil,
                byline: entry.byline ?? "", typeName: entry.kind.name,
                detail: entry.progressText, symbol: entry.kind.icon,
                coverItemID: entry.kind == .webPage ? entry.id : nil, coverURL: entry.cover,
                pdfItemID: entry.kind == .pdf ? entry.id : nil,
                updated: entry.updatedAt, lastRead: entry.lastReadAt ?? 0, added: entry.addedAt,
                unread: Int(entry.chapters) - Int(entry.finished), progress: entry.progress
            ))
        }
        return books
    }

    private var shelfBooks: [Book] {
        var books = allBooks.filter(filter.contains)
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

/// Library filters: everything, reading in progress, the collection and
/// its kinds, subscriptions and their categories.
private struct LibrarySidebar: View {
    @Environment(AppModel.self) private var model
    @Binding var selection: LibraryFilter
    var focus: FocusState<PaneFocus?>.Binding?
    /// Return: on to the shelf.
    var onReturn: (() -> Void)?

    var body: some View {
        List(selection: Binding(get: { selection }, set: { if let new = $0 { selection = new } })) {
            SideHeading(title: "Library", first: true)
            row(.all, "books.vertical", Text("All"))
            row(.continueReading, "book", Text("Continue Reading"), model.collection.filter(\.inProgress).count)
            row(.saved, "bookmark", Text("Saved"), model.collection.count)
            let kinds = CollectionKindView.all.filter { kind in model.collection.contains { $0.kind == kind } }
            if !kinds.isEmpty {
                SideHeading(title: "Collection")
                ForEach(kinds) { kind in
                    row(.kind(kind), kind.icon, Text(kind.title), model.collection.filter { $0.kind == kind }.count)
                }
            }
            if !model.feeds.isEmpty {
                SideHeading(title: "Subscriptions")
                row(.subscriptions, "dot.radiowaves.up.forward", Text("All Subscriptions"), model.unreadTotal)
                if categories.count > 1 {
                    ForEach(categories, id: \.self) { category in
                        row(.category(category), "folder", Text(verbatim: category), unread(in: category))
                    }
                }
            }
        }
        .flatList()
        .pane(focus, .sidebar, onReturn: onReturn)
    }

    private func row(_ filter: LibraryFilter, _ symbol: String, _ title: Text, _ count: Int = 0) -> some View {
        SideRow(symbol: symbol, title: title, count: count, chosen: selection == filter)
            .flatRow(chosen: selection == filter)
            .tag(filter)
    }

    private var categories: [String] {
        Array(Set(model.feeds.map { $0.category ?? "Subscriptions" })).sorted()
    }

    /// Shelf categories file subscriptions without a folder under "Subscriptions".
    private func unread(in category: String) -> Int {
        model.feeds.filter { ($0.category ?? "Subscriptions") == category }.reduce(0) { $0 + Int($1.unread) }
    }
}

private struct DateCell: View {
    let ms: Int64

    var body: some View {
        if ms > 0 {
            Text(Date(timeIntervalSince1970: TimeInterval(ms) / 1000), format: .relative(presentation: .named))
                .foregroundStyle(.secondary)
        } else {
            Text("–").foregroundStyle(.tertiary)
        }
    }
}

private struct BookTile: View {
    let book: Book
    /// Picked with the keyboard.
    var selected = false
    @State private var hovering = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            BookCover(title: book.title, subtitle: book.subtitle, imageItemID: book.coverItemID, imageURL: book.coverURL, pdfItemID: book.pdfItemID, symbol: book.symbol)
                .aspectRatio(2 / 3, contentMode: .fit)
                .shadow(color: .black.opacity(hovering ? 0.28 : 0.16), radius: hovering ? 9 : 5, y: hovering ? 5 : 3)
                .overlay {
                    if selected {
                        RoundedRectangle(cornerRadius: 8).strokeBorder(Color.accentColor, lineWidth: 3).padding(-4)
                    }
                }
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
