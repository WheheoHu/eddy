import EddyKit
import SwiftUI

/// Three-pane mode: filters in the sidebar, what they select in the middle
/// (articles, or the entries of the collection), the reader on the right.
/// A collection entry keeps its contents in a panel beside the text.
struct ThreePaneView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("app.mode") private var mode: AppMode = .threePane
    @AppStorage("threePane.contents") private var showContents = false
    @AppStorage("threePane.collectionSort") private var collectionSort: CollectionSort = .added
    /// Names of the subscription folders folded up; new folders start open.
    @AppStorage("threePane.collapsedFolders") private var collapsedFoldersJSON = "[]"
    @State private var selection: Selection? = .smart(.all)
    /// The collection entry picked in the middle column.
    @State private var entryID: String?
    @State private var unreadOnly = false
    @State private var query = ""
    @State private var results: [ItemView]?
    @State private var scrollTarget: ScrollTarget?
    @State private var pageTarget: PageTarget?
    /// Back and forward through the articles of the middle column.
    @State private var history = ReadingHistory()
    /// Where the reader slides in from when moving to a neighbor.
    @State private var arrival: Edge = .bottom
    /// A previous chapter, opened at its end.
    @State private var landAtEnd: String?
    @FocusState private var focus: PaneFocus?
    /// Keyboard focus is in the reader, which keeps it from one item to the
    /// next; a list taking focus clears it.
    @State private var readerFocused = false

    enum Smart: String, CaseIterable {
        case all = "All Articles", unread = "Unread", starred = "Starred", continueReading = "Continue Reading"
        var icon: String {
            switch self {
            case .all: "tray.full"
            case .unread: "circle.inset.filled"
            case .starred: "star"
            case .continueReading: "book"
            }
        }
    }

    enum Selection: Hashable {
        case smart(Smart), saved, kind(CollectionKindView), folder(String), feed(String), tag(String)

        /// The collection kinds a selection lists, if it lists entries.
        var kinds: [CollectionKindView]? {
            switch self {
            case .saved: CollectionKindView.all
            case let .kind(kind): [kind]
            default: nil
            }
        }
    }

    var body: some View {
        NavigationSplitView {
            sidebar.navigationSplitViewColumnWidth(min: 210, ideal: 240)
        } content: {
            middle.navigationSplitViewColumnWidth(min: 300, ideal: 360)
        } detail: {
            detail
        }
        .searchable(text: $query, placement: .toolbar, prompt: "Search")
        .toolbar { AddToolbar(model: model) }
        .toolbar(removing: .title)
        .onAppear(perform: showCurrentEntry)
        .onChange(of: model.revealed) { _, revealed in
            guard let revealed else { return }
            model.revealed = nil
            query = ""
            show(entryID: revealed)
        }
        .onChange(of: model.revision) {
            if let entryID, model.entry(entryID) == nil { self.entryID = nil }
        }
        .onChange(of: model.currentItemID) { _, new in
            pageTarget = nil
            history.opened(new)
            if new != landAtEnd { landAtEnd = nil }
        }
        .onChange(of: selection) { history.reset(model.currentItemID) }
        .onChange(of: query) { history.reset(model.currentItemID) }
        .onChange(of: focus) { _, new in if new != nil { readerFocused = false } }
        .onChange(of: articleList, initial: true) { _, list in model.currentList = list }
        .onDisappear { model.currentList = nil }
        .onChange(of: model.navigation) { _, request in
            guard case let .threePane(destination) = request?.value else { return }
            model.navigation = nil
            query = ""
            choose(destination)
            focus = .list
        }
        .onChange(of: model.revealedItem) { _, request in
            guard let id = request?.value else { return }
            model.revealedItem = nil
            reveal(id)
        }
        .task(id: query) {
            guard !query.isEmpty else { results = nil; return }
            try? await Task.sleep(for: .milliseconds(250))
            results = await model.search(query).map(\.item)
        }
    }

    // MARK: Sidebar

    private var sidebar: some View {
        List(selection: Binding(get: { selection }, set: choose)) {
            Section("Library") {
                ForEach(Smart.allCases, id: \.self) { smart in
                    Label(smart.rawValue, systemImage: smart.icon)
                        .badge(badge(smart))
                        .tag(Selection.smart(smart))
                }
                Label("Saved", systemImage: "bookmark").badge(model.collection.count).tag(Selection.saved)
            }
            let kinds = CollectionKindView.all.filter { kind in model.collection.contains { $0.kind == kind } }
            if !kinds.isEmpty {
                Section("Collection") {
                    ForEach(kinds) { kind in
                        Label(kind.title, systemImage: kind.icon)
                            .badge(model.collection.filter { $0.kind == kind }.count)
                            .tag(Selection.kind(kind))
                    }
                }
            }
            if !model.feeds.isEmpty {
                Section("Subscriptions") {
                    ForEach(model.folders, id: \.self) { folder in
                        DisclosureGroup(isExpanded: expanded(folder)) {
                            feedRows(model.feeds(inFolder: folder))
                        } label: {
                            Label {
                                HStack(spacing: 4) {
                                    Text(folder)
                                    if model.inGallery(folder: folder) { GalleryMark() }
                                }
                            } icon: {
                                Image(systemName: "folder")
                            }
                                .badge(model.unread(inFolder: folder))
                                .tag(Selection.folder(folder))
                                .contextMenu {
                                    let shown = model.inGallery(folder: folder)
                                    Button(shown ? "Remove from Gallery" : "Show in Gallery") { model.setGallery(folder: folder, !shown) }
                                }
                        }
                    }
                    feedRows(model.feeds(inFolder: nil))
                }
            }
            if !model.tags.isEmpty {
                Section("Tags") {
                    ForEach(model.tags) { t in
                        Label(t.name, systemImage: "number").badge(Int(t.count)).tag(Selection.tag(t.id))
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .pane($focus, .sidebar, onReturn: { focus = .list })
        .onKeyPress(keys: [.leftArrow, .rightArrow]) { press in folderKey(press.key) }
        .safeAreaInset(edge: .bottom) { StatusFooter() }
    }

    /// Left folds the selected folder, or goes up to it from a subscription
    /// inside it; Right unfolds it.
    private func folderKey(_ key: KeyEquivalent) -> KeyPress.Result {
        switch selection {
        case let .folder(name):
            let open = !collapsedFolders.contains(name)
            guard key == .leftArrow ? open : !open else { return .ignored }
            withAnimation(.easeInOut(duration: 0.15)) { expanded(name).wrappedValue = !open }
            return .handled
        case let .feed(id):
            guard key == .leftArrow, let folder = model.feeds.first(where: { $0.id == id })?.category else { return .ignored }
            choose(.folder(folder))
            return .handled
        default:
            return .ignored
        }
    }

    private func badge(_ smart: Smart) -> Int {
        switch smart {
        case .unread: model.unreadTotal
        case .continueReading: model.continueReadingCount
        default: 0
        }
    }

    private func feedRows(_ feeds: [SourceView]) -> some View {
        ForEach(feeds) { feed in
            Label {
                HStack(spacing: 4) {
                    Text(feed.title)
                    if let reason = model.galleryReason(feed) { GalleryMark(throughFolder: reason.isEmpty ? nil : reason) }
                }
            } icon: {
                Image(systemName: "dot.radiowaves.up.forward")
            }
            .badge(Int(feed.unread)).tag(Selection.feed(feed.id))
                .contextMenu {
                    if let folder = feed.category, model.inGallery(folder: folder) {
                        Button("Shown in Gallery with “\(folder)”") {}.disabled(true)
                    } else {
                        Button(feed.gallery ? "Remove from Gallery" : "Show in Gallery") { model.setGallery(source: feed, !feed.gallery) }
                    }
                    FullTextMenu(feed: feed)
                }
        }
    }

    private var collapsedFolders: Set<String> {
        (try? JSONDecoder().decode(Set<String>.self, from: Data(collapsedFoldersJSON.utf8))) ?? []
    }

    private func expanded(_ folder: String) -> Binding<Bool> {
        Binding {
            !collapsedFolders.contains(folder)
        } set: { open in
            var collapsed = collapsedFolders
            if open { collapsed.remove(folder) } else { collapsed.insert(folder) }
            collapsedFoldersJSON = (try? String(data: JSONEncoder().encode(collapsed.sorted()), encoding: .utf8)) ?? collapsedFoldersJSON
        }
    }

    /// A click in the sidebar starts over: nothing is open until something
    /// is picked in the middle column.
    private func choose(_ new: Selection?) {
        selection = new
        entryID = nil
        model.currentItemID = nil
    }

    /// Coming from the library mode, the entry being read stays in view.
    private func showCurrentEntry() {
        if let entry = model.entry(containing: model.currentItemID) { show(entryID: entry.id) }
    }

    /// Opens an item from anywhere: a collection item in its entry, an
    /// article in its subscription.
    private func reveal(_ id: String) {
        guard let item = model.item(id) else { return }
        query = ""
        if let entry = model.entry(containing: item) {
            show(entryID: entry.id)
        } else if let sourceID = item.sourceId {
            selection = .feed(sourceID)
            entryID = nil
        }
        open(id)
        readerFocused = true
    }

    private func show(entryID id: String) {
        guard let entry = model.entry(id) else { return }
        selection = .kind(entry.kind)
        entryID = entry.id
    }

    // MARK: Middle column

    @ViewBuilder
    private var middle: some View {
        if results == nil, let kinds = selection?.kinds {
            CollectionList(
                kinds: kinds, sort: collectionSort, selection: Binding(get: { entryID }, set: openEntry),
                focus: $focus, onReturn: read, onEscape: { focus = .sidebar }
            )
                .toolbar {
                    ToolbarItem {
                        Menu {
                            Picker("Sort By", selection: $collectionSort) {
                                ForEach(CollectionSort.allCases) { Text($0.rawValue).tag($0) }
                            }
                            .pickerStyle(.inline)
                        } label: {
                            Label("Sort By", systemImage: "arrow.up.arrow.down")
                        }
                        .help("Sort order")
                    }
                    refreshItem
                }
        } else {
            articles
        }
    }

    private var articles: some View {
        @Bindable var model = model
        let rows = results ?? rowsForSelection
        return List(rows, selection: $model.currentItemID) { item in
            ArticleRow(item: item).tag(item.id)
        }
        .contextMenu(forSelectionType: String.self) { ids in
            if let id = ids.first, let item = rows.first(where: { $0.id == id }) {
                Button(item.starred ? "Unstar" : "Star") { model.setStarred(item, !item.starred) }
                if item.hasBeenRead {
                    Button("Mark as Unread") { model.markUnread([item.id]) }
                } else {
                    Button("Mark as Read") { model.setRead(item, true) }
                }
                if let url = item.url.flatMap(URL.init(string:)) {
                    Button("Open in Browser") { NSWorkspace.shared.open(url) }
                    Button("Copy Link") {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(url.absoluteString, forType: .string)
                    }
                }
            }
        } primaryAction: { _ in read() }
        .listStyle(.inset)
        .pane($focus, .list, onReturn: read, onEscape: { focus = .sidebar })
        .overlay {
            if rows.isEmpty {
                ContentUnavailableView(results != nil ? "No Results" : "Nothing Here", systemImage: results != nil ? "magnifyingglass" : "tray")
            }
        }
        .toolbar {
            ToolbarItem {
                Toggle(isOn: $unreadOnly) { Label("Unread Only", systemImage: "line.3.horizontal.decrease.circle") }.help("Show unread only")
            }
            if let articleList {
                ToolbarItem {
                    Button { model.requestMarkAllRead(articleList) } label: {
                        Label("Mark All as Read", systemImage: "checkmark.circle")
                    }
                    .help("Mark every unread article here as read")
                }
            }
            refreshItem
        }
        .background { keyboard(rows) }
    }

    private var refreshItem: some ToolbarContent {
        ToolbarItem {
            Button { Task { await model.refresh() } } label: { Label("Refresh", systemImage: "arrow.clockwise") }
                .help("Refresh (⌘R)")
                .disabled(model.busy)
        }
    }

    /// The middle column as a list of subscription articles, for "Mark All
    /// as Read"; nil for the collection and for search results.
    private var articleList: ArticleList? {
        guard results == nil, let selection else { return nil }
        switch selection {
        case let .smart(smart):
            let filter: ItemFilterView = switch smart {
            case .all, .unread: .all
            case .starred: .starred
            case .continueReading: .continueReading
            }
            return ArticleList(filter: filter, title: String(localized: String.LocalizationValue(smart.rawValue)))
        case let .folder(name):
            return ArticleList(filter: .category(name: name), title: name)
        case let .feed(id):
            return ArticleList(filter: .source(id: id), title: model.feeds.first { $0.id == id }?.title ?? "")
        case let .tag(id):
            return ArticleList(filter: .tag(id: id), title: model.tags.first { $0.id == id }?.name ?? "")
        case .saved, .kind:
            return nil
        }
    }

    /// Opening an article marks it read; it leaves an unread-only list once
    /// another article is opened, not while it is being read.
    private var rowsForSelection: [ItemView] {
        let keep = model.currentItemID
        return switch selection {
        case .smart(.unread): model.items(.all, unreadOnly: true, keep: keep)
        case .smart(.starred): model.items(.starred, unreadOnly: unreadOnly, keep: keep)
        case .smart(.continueReading): model.items(.continueReading)
        case let .folder(name): model.items(.category(name: name), unreadOnly: unreadOnly, keep: keep)
        case let .feed(id): model.items(.source(id: id), unreadOnly: unreadOnly, keep: keep)
        case let .tag(id): model.items(.tag(id: id), unreadOnly: unreadOnly, keep: keep)
        default: model.items(.all, unreadOnly: unreadOnly, keep: keep)
        }
    }

    /// Opens an entry where reading stopped; the reader restores the position.
    private func openEntry(_ id: String?) {
        entryID = id
        guard let id, let entry = model.entry(id) else { return }
        open(entry.resumeItemId)
    }

    private func open(_ itemID: String?) {
        scrollTarget = nil
        pageTarget = nil
        model.currentItemID = itemID
    }

    /// Return in a list: keyboard focus goes to the text.
    private func read() {
        guard model.currentItemID != nil else { return }
        readerFocused = true
    }

    /// Esc in the reader: back to the contents panel when it is open, else
    /// to the middle column.
    private func leaveReader(contentsShown: Bool) {
        readerFocused = false
        focus = contentsShown ? .contents : .list
    }

    /// j/k move through the list, s stars, m toggles read.
    private func keyboard(_ rows: [ItemView]) -> some View {
        Group {
            Button("Next") { move(rows, 1) }.keyboardShortcut("j", modifiers: [])
            Button("Previous") { move(rows, -1) }.keyboardShortcut("k", modifiers: [])
            Button("Star") { if let item = current(rows) { model.setStarred(item, !item.starred) } }.keyboardShortcut("s", modifiers: [])
            Button("Read") { if let item = current(rows) { model.setRead(item, !item.read) } }.keyboardShortcut("m", modifiers: [])
        }
        .disabled(model.paletteVisible)
        .opacity(0)
        .frame(width: 0, height: 0)
        .accessibilityHidden(true)
    }

    private func current(_ rows: [ItemView]) -> ItemView? {
        rows.first { $0.id == model.currentItemID }
    }

    private func move(_ rows: [ItemView], _ delta: Int) {
        guard !rows.isEmpty else { return }
        let index = model.currentItemID.flatMap { id in rows.firstIndex { $0.id == id } } ?? -1
        model.currentItemID = rows[min(max(index + delta, 0), rows.count - 1)].id
    }

    // MARK: Reader

    @ViewBuilder
    private var detail: some View {
        if let id = model.currentItemID, let item = model.item(id) {
            let entry = model.entry(containing: item)
            HStack(spacing: 0) {
                if let entry, showContents {
                    EntryContents(
                        entry: entry, currentItemID: id, open: open, jump: jump,
                        focus: $focus, onReturn: read, onEscape: { focus = .list }
                    )
                        .frame(width: 260)
                        .background(.background.secondary)
                    Divider()
                }
                VStack(spacing: 0) {
                    ReaderPane(
                        itemID: id, scrollTarget: $scrollTarget, pageTarget: pageTarget, openItem: open,
                        showContents: entry == nil ? nil : $showContents,
                        neighbors: neighbors(of: item, in: entry), startAtEnd: landAtEnd == id,
                        onEscape: { leaveReader(contentsShown: entry != nil && showContents) },
                        focused: readerFocused, onFocus: { readerFocused = true }
                    )
                    .id(id)
                    .transition(.push(from: arrival))
                    if let seriesID = item.seriesId, let info = model.series.first(where: { $0.id == seriesID }) {
                        Divider()
                        SequenceBar(title: info.title, items: model.chapters(seriesID), current: id) { open($0) }
                    }
                }
            }
        } else {
            ContentUnavailableView("No Article Selected", systemImage: "text.alignleft", description: Text("J and K move through the list."))
        }
    }

    private func jump(_ target: OutlineEntry.Target) {
        switch target {
        case let .block(id): scrollTarget = ScrollTarget(blockID: id)
        case let .page(page): pageTarget = PageTarget(page: page)
        }
    }

    /// A chapter's book, or the articles of the middle column; nothing to
    /// move to from a page or PDF saved on its own.
    private func neighbors(of item: ItemView, in entry: CollectionEntryView?) -> Neighbors? {
        if let seriesID = item.seriesId {
            return .chapters({ model.chapters(seriesID) }, current: item.id, open: travel)
        }
        guard entry == nil else { return nil }
        return .articles({ results ?? rowsForSelection }, current: item.id, history: history, item: { model.item($0) }, open: travel)
    }

    /// Opens a neighbor, sliding it in from the side it lies on. The edge
    /// is set first so the outgoing reader leaves the same way.
    private func travel(to id: String, _ direction: Direction, _ axis: Axis, _ atEnd: Bool) {
        arrival = .arriving(direction, along: axis)
        landAtEnd = atEnd ? id : nil
        DispatchQueue.main.async {
            withAnimation(.easeInOut(duration: 0.2)) { open(id) }
        }
    }
}

/// Middle column for Saved and the collection kinds; Saved groups its
/// entries by kind.
struct CollectionList: View {
    @Environment(AppModel.self) private var model
    let kinds: [CollectionKindView]
    let sort: CollectionSort
    @Binding var selection: String?
    var focus: FocusState<PaneFocus?>.Binding?
    /// Return: read the entry selected.
    var onReturn: (() -> Void)?
    var onEscape: (() -> Void)?
    /// The order taken when the list appears or its filter or sort changes.
    /// Reading an entry changes its last-read time and progress; re-sorting
    /// on that would move rows away from under the pointer.
    @State private var order: [String: Int] = [:]

    var body: some View {
        let entries = ordered(model.collection.filter { kinds.contains($0.kind) })
        List(selection: $selection) {
            if kinds.count == 1 {
                rows(entries)
            } else {
                ForEach(kinds) { kind in
                    let group = entries.filter { $0.kind == kind }
                    if !group.isEmpty {
                        Section(kind.title) { rows(group) }
                    }
                }
            }
        }
        .listStyle(.inset)
        .contextMenu(forSelectionType: String.self) { ids in
            if let id = ids.first, let entry = model.entry(id) {
                if entry.hasBeenRead {
                    Button("Mark as Unread") { model.markUnread(entry: entry) }
                }
                if entry.kind == .webPage {
                    Button("Convert to Web Series…") { Task { await model.convertToSeries(entry) } }
                }
                Button("Delete…", role: .destructive) { model.requestDelete(entry) }
            }
        } primaryAction: { _ in onReturn?() }
        .pane(focus, .list, onReturn: onReturn, onEscape: onEscape)
        .onDeleteCommand(perform: deleteSelected)
        .onKeyPress(keys: [.delete], phases: .down) { press in
            guard press.modifiers.contains(.command) else { return .ignored }
            deleteSelected()
            return .handled
        }
        .overlay {
            if entries.isEmpty {
                ContentUnavailableView("Nothing Here", systemImage: "tray", description: Text("Save a page with ⌘L, or import an EPUB or PDF with ⌘O."))
            }
        }
        .onAppear(perform: reorder)
        .onChange(of: kinds) { reorder() }
        .onChange(of: sort) { reorder() }
    }

    private func reorder() {
        order = Dictionary(uniqueKeysWithValues: model.collection.sorted(by: sort.inOrder).enumerated().map { ($1.id, $0) })
    }

    /// Entries in the order taken last; ones added since come first.
    private func ordered(_ entries: [CollectionEntryView]) -> [CollectionEntryView] {
        let new = entries.filter { order[$0.id] == nil }.sorted(by: sort.inOrder)
        let known = entries.compactMap { entry in order[entry.id].map { (entry, $0) } }.sorted { $0.1 < $1.1 }.map(\.0)
        return new + known
    }

    private func rows(_ entries: [CollectionEntryView]) -> some View {
        ForEach(entries) { entry in
            CollectionRow(entry: entry).tag(entry.id)
        }
    }

    private func deleteSelected() {
        if let selection, let entry = model.entry(selection) { model.requestDelete(entry) }
    }
}

struct StatusFooter: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: icon)
            Text(text).lineLimit(1)
            Spacer()
            if model.busy || model.historyAdded > 0 { ProgressView().controlSize(.mini) }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(10)
    }

    private var icon: String {
        switch model.status?.mode {
        case .server: "arrow.triangle.2.circlepath.icloud"
        case .standalone: "dot.radiowaves.left.and.right"
        default: "internaldrive"
        }
    }

    private var text: String {
        switch model.status?.mode {
        case .server: "Syncing with server"
        case .standalone where model.historyAdded > 0: "Loading history · \(model.historyAdded.formatted()) older articles"
        case .standalone: model.status?.account.map { "FreshRSS · \($0.username)" } ?? "FreshRSS"
        default: "On this Mac"
        }
    }
}

/// A subscription's full-text mode, from its context menu. Picking one
/// adds a rule for it at the top of the full-text rules.
private struct FullTextMenu: View {
    @Environment(AppModel.self) private var model
    let feed: SourceView

    var body: some View {
        Menu("Full Text") {
            ForEach(FullTextModeView.allCases, id: \.self) { mode in
                Toggle(isOn: Binding(get: { feed.sourceRule == mode }, set: { _ in model.setFullText(source: feed, mode) })) {
                    Text(mode.title)
                }
            }
            Divider()
            Button("Use Rules") { model.setFullText(source: feed, nil) }.disabled(feed.sourceRule == nil)
            Text("Now: \(Text(feed.fullText.title))")
        }
    }
}
