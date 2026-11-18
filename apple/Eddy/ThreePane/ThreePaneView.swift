import EddyKit
import SwiftUI

/// Three-pane mode: filters in the sidebar, what they select in the middle
/// (articles, or the entries of the collection), the reader on the right.
/// A collection entry keeps its contents in a panel beside the text.
///
/// Each column has a strip of its own controls along its top; there is no
/// toolbar. A window too narrow for all three shows fewer (`PaneTier`), and
/// `PaneContainer` lays the columns out for the tier.
struct ThreePaneView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("app.mode") private var mode: AppMode = .threePane
    @AppStorage("threePane.contents") private var showContents = false
    @AppStorage("threePane.collectionSort") private var collectionSort: CollectionSort = .added
    /// What the sidebar has chosen; the model keeps it (`threePaneSelection`).
    private var selection: Selection? { model.threePaneSelection }
    /// The subscription folders folded up; new folders start open. Kept
    /// here and handed to the sidebar, which only draws again when what it
    /// is handed changes; state, so that folding animates the same from a
    /// click and from a key.
    @State private var collapsed = ThreePaneView.storedFolds()
    /// The sidebar's table, to slide its rows when a folder folds.
    @State private var sidebarTable = TableHandle()
    /// The collection entry picked in the middle column.
    @State private var entryID: String?
    /// Articles read while the unread-only list has been on screen. They
    /// stay listed, so rows don't move under the pointer with each one
    /// read, until the list starts over: another sidebar selection, the
    /// filter toggled, or everything marked read.
    @State private var readHere: Set<String> = []
    @State private var scrollTarget: ScrollTarget?
    @State private var pageTarget: PageTarget?
    /// Back and forward through the articles of the middle column.
    @State private var history = ReadingHistory()
    /// The side the text of the item being opened comes in from.
    @State private var arrival: Arrival?
    /// A previous chapter, opened at its end.
    @State private var landAtEnd: String?
    /// Where the rows of Continue Reading stood when the list came on
    /// screen. Reading moves an item to the top of that list and, at its
    /// end, out of it; the rows stay as they were until the list starts
    /// over, so the ones above and below the article being read are still
    /// there to move to.
    @State private var continueOrder: [String: Int] = [:]
    /// Which of the items being read Continue Reading lists.
    @AppStorage(ReadingScope.key) private var readingScope: ReadingScope = .all
    @FocusState private var focus: PaneFocus?
    /// Keyboard focus is in the reader, which keeps it from one item to the
    /// next; a list taking focus clears it.
    @State private var readerFocused = false
    /// How many columns the window has room for. Only the tier is kept, not
    /// the width: this view's body reads the lists and must not run with
    /// every step of a resize.
    @State private var tier = PaneTier.wide
    /// The column the reader is at, where not all of them show.
    @State private var place = PanePlace.list
    /// The sidebar is open in the wide tier, where it can be shut.
    @AppStorage("threePane.sidebar") private var showSidebar = true
    /// Which of the articles the middle column lists.
    @State private var filter = ListFilter.all
    /// The contents over the text, in the narrowest tier, where there is no
    /// room for them beside it.
    @State private var contentsDrawer = false
    /// A swipe between the columns in progress. Only the columns that move
    /// read it; this view's body doesn't.
    @State private var drag = PaneDrag()
    /// The window's width has been measured once.
    @State private var fitted = false

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

    /// The switch under the article list.
    enum ListFilter: Hashable { case starred, unread, all }

    private var unreadOnly: Bool { filter == .unread }

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
        PaneContainer(
            arrangement: arrangement, canRead: canRead,
            gesturesEnabled: !model.overlayVisible && model.mediaRequest == nil, drag: drag, go: go
        ) {
            sidebarColumn
        } list: {
            listColumn
        } reader: {
            detail
        }
        .onGeometryChange(for: PaneTier.self) { PaneTier.at($0.size.width, from: tier) } action: { fit($0) }
        .onAppear {
            showCurrentEntry()
            // The sidebar's choice was made before this view was: its list
            // starts here.
            startOver()
            // A subscription chosen in another mode shows in its folder.
            if case let .feed(id) = selection, let folder = model.feeds.first(where: { $0.id == id })?.category, collapsed.contains(folder) {
                toggleFolder(folder)
            }
            // Once the columns are there to take the keys.
            DispatchQueue.main.async { focusFirstColumn() }
            // Coming from another mode with its sidebar in: it stays in.
            place = model.sidebarPulled ? .sidebar : (canRead ? .reader : .list)
        }
        .onChange(of: sidebarPulled) { _, pulled in model.sidebarPulled = pulled }
        .onChange(of: model.revealed) { _, revealed in
            guard let revealed else { return }
            model.revealed = nil
            show(entryID: revealed)
            if !arrangement.listShown { go(.list) }
        }
        .onChange(of: model.revision) {
            if let entryID, model.entry(entryID) == nil { self.entryID = nil }
        }
        .onChange(of: model.currentItemID) { old, new in
            pageTarget = nil
            history.opened(new)
            if new != landAtEnd { landAtEnd = nil }
            // Nothing to read: back at the list, and still there when its
            // selection moves to another article.
            if new == nil, place == .reader { place = .list }
            // Moving to a neighbor has said which side it lies on; any
            // other way of opening goes by where the two stand.
            if let new {
                if arrival?.item != new { arrival = Arrival(item: new, side: .from(old, to: new, in: order(old, new))) }
            } else {
                arrival = nil
            }
            if let new, holdsRows(new) { readHere.insert(new) }
        }
        .onChange(of: selection) {
            history.reset(model.currentItemID)
            startOver()
        }
        .onChange(of: filter) { startOver() }
        .onChange(of: readingScope) { startOver() }
        .onChange(of: model.listsReset) { startOver() }
        .onChange(of: focus) { _, new in if new != nil { readerFocused = false } }
        .onChange(of: articleList, initial: true) { _, list in model.currentList = list }
        .onDisappear { model.currentList = nil }
        .onChange(of: model.navigation) { _, request in
            guard case let .threePane(destination) = request?.value else { return }
            model.navigation = nil
            choose(destination)
            go(.list)
        }
        .onChange(of: model.revealedItem, initial: true) { _, request in
            guard let id = request?.value else { return }
            model.revealedItem = nil
            reveal(id)
        }
    }

    // MARK: Columns

    private var canRead: Bool { model.currentItemID != nil }

    /// With nothing open the reader has only a placeholder, so the list
    /// shows in its place.
    private var arrangement: PaneArrangement {
        PaneArrangement(tier: tier, place: place == .reader && !canRead ? .list : place, sidebarOpen: showSidebar)
    }

    /// The sidebar is in over the other columns, not beside them.
    private var sidebarPulled: Bool { tier != .wide && arrangement.sidebarShown }

    /// The window is in another tier. Narrowing past the list leaves the
    /// reader, if something is open; the columns don't come in over it
    /// unasked. The columns glide to where the new tier puts them, which
    /// shows where the one that left went.
    private func fit(_ new: PaneTier) {
        guard fitted else {
            // The first measure: the window opens as it is, with no glide.
            fitted = true
            tier = new
            if new == .wide { model.sidebarPulled = false }
            return
        }
        guard new != tier else { return }
        withAnimation(.easeOut(duration: 0.28)) {
            if tier >= .medium, new <= .narrow {
                place = canRead ? .reader : .list
            } else if new == .medium, place == .sidebar {
                place = .list
            }
            tier = new
            drag.end()
            contentsDrawer = false
        }
    }

    /// Moves to a column, and keyboard focus with it. In the wide tier,
    /// where all three show, only the focus moves.
    private func go(_ new: PanePlace) {
        if new == .reader, !canRead { return }
        if tier == .wide {
            place = new
            drag.end()
        } else {
            // From wherever a swipe has brought the columns, the rest of
            // the way.
            withAnimation(.easeOut(duration: 0.24)) {
                place = new
                drag.end()
            }
        }
        switch new {
        case .sidebar: focus = .sidebar
        case .list: focus = .list
        case .reader: readerFocused = true
        }
    }

    /// The first button of the strip that leads the window: back a column
    /// in the narrowest tier, the sidebar shown or hidden in the others.
    @ViewBuilder
    private func leadButton(in column: PanePlace) -> some View {
        if arrangement.leading == column {
            let shown = arrangement.sidebarShown
            if tier == .narrowest, arrangement.place == .reader {
                Button { go(.list) } label: { Label("Back", systemImage: "chevron.left") }
                    .help("Back to the list")
            } else {
                Button {
                    switch tier {
                    case .wide: withAnimation(.easeOut(duration: 0.24)) { showSidebar.toggle() }
                    case .medium, .narrowest: go(shown ? .list : .sidebar)
                    case .narrow: go(shown ? .reader : .list)
                    }
                } label: {
                    Label(shown ? "Hide Sidebar" : "Show Sidebar", systemImage: "sidebar.left")
                }
                .buttonStyle(FlatButtonStyle(on: shown))
                .help(shown ? "Hide sidebar" : "Show sidebar")
                .disabled(tier == .narrow && shown && !canRead)
            }
        }
    }

    /// What the reader's swipe to the right leads back to, where columns
    /// can come in over it: they come with the fingers.
    private var readerBack: ReaderBack? {
        guard tier <= .medium, arrangement.readerShown else { return nil }
        let back: PanePlace = tier == .medium ? .sidebar : .list
        return ReaderBack(
            commit: PaneDrag.commit,
            follow: { [drag] distance in
                if let distance { drag.move(to: back, distance) } else { drag.cancel() }
            },
            go: { go(back) }
        )
    }

    /// The middle column's name.
    private var listTitle: Text {
        switch selection {
        case .saved: Text("Saved")
        case let .kind(kind): Text(kind.title)
        default: Text(articleList?.title ?? "")
        }
    }

    /// Esc in the middle column: back to the reader from the columns over
    /// it, else to the sidebar.
    private func leaveList() {
        if tier == .narrow, canRead { go(.reader) } else if tier != .wide || showSidebar { go(.sidebar) }
    }

    // MARK: Sidebar

    private var sidebarColumn: some View {
        VStack(spacing: 0) {
            ColumnStrip(leads: arrangement.leading == .sidebar) {
                leadButton(in: .sidebar)
            } trailing: {
                Button { Task { await model.refresh() } } label: { Label("Refresh", systemImage: "arrow.clockwise") }
                    .help("Refresh (⌘R)")
                    .disabled(model.busy)
                AddMenu()
            }
            ThreePaneSidebar(
                selection: selection, tier: tier, canRead: canRead, collapsed: collapsed, active: focus == .sidebar,
                table: sidebarTable, focus: $focus, choose: choose, go: go, enter: enterList, toggle: toggleFolder
            )
                .equatable()
            StatusFooter()
            ModeBar()
        }
    }

    private static let foldsKey = "threePane.collapsedFolders"

    private static func storedFolds() -> Set<String> {
        let json = UserDefaults.standard.string(forKey: foldsKey) ?? "[]"
        return (try? JSONDecoder().decode(Set<String>.self, from: Data(json.utf8))) ?? []
    }

    /// Folds a folder of the sidebar, or unfolds it: its subscriptions go
    /// or come with the same move whatever asked, the arrow or Space. The
    /// list itself only swaps its rows, so the move is made on its table.
    private func toggleFolder(_ name: String) {
        let opening = collapsed.contains(name)
        let row = ThreePaneSidebar.row(ofFolder: name, in: model, collapsed: collapsed)
        collapsed.formSymmetricDifference([name])
        if let row { sidebarTable.fold(at: row, count: model.feeds(inFolder: name).count, opening: opening) }
        if let json = try? String(data: JSONEncoder().encode(collapsed.sorted()), encoding: .utf8) {
            UserDefaults.standard.set(json, forKey: Self.foldsKey)
        }
    }

    /// Return or Right in the sidebar: on to the middle column, at its
    /// first row unless one of its rows is open already.
    private func enterList() {
        go(.list)
        if let kinds = selection?.kinds {
            guard entryID == nil else { return }
            // In the order the list shows them: Saved by kind.
            let first = kinds.lazy.compactMap { kind in
                model.collection.filter { $0.kind == kind }.sorted(by: collectionSort.inOrder).first
            }.first
            if let first { openEntry(first.id) }
        } else {
            let rows = rowsForSelection
            if let current = model.currentItemID, rows.contains(where: { $0.id == current }) { return }
            if let first = rows.first { open(first.id) }
        }
    }

    /// The keys start in the first column that shows: the sidebar's chosen
    /// row, where there is a sidebar.
    private func focusFirstColumn() {
        if arrangement.sidebarShown {
            readerFocused = false
            focus = .sidebar
        } else if arrangement.listShown {
            readerFocused = false
            focus = .list
        } else {
            readerFocused = true
        }
    }

    /// A click in the sidebar starts over: nothing is open until something
    /// is picked in the middle column. The list also reports the choice it
    /// is given when it appears, which is no click: coming from another
    /// mode, what is being read there stays open.
    private func choose(_ new: Selection?) {
        guard new != selection else { return }
        model.chose(new)
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
        if let entry = model.entry(containing: item) {
            show(entryID: entry.id)
        } else if let sourceID = item.sourceId {
            model.chose(.feed(sourceID))
            entryID = nil
        }
        open(id)
        go(.reader)
    }

    /// Picks an entry in the middle column, in the list of its kind unless
    /// the one showing (Saved) has it already.
    private func show(entryID id: String) {
        guard let entry = model.entry(id) else { return }
        if selection?.kinds?.contains(entry.kind) != true { model.chose(.kind(entry.kind)) }
        entryID = entry.id
    }

    // MARK: Middle column

    /// The middle column: its strip, its name in large type, the list, and
    /// under a list of articles the switch that filters it.
    private var listColumn: some View {
        let kinds = selection?.kinds
        let rows = kinds == nil ? rowsForSelection : []
        return VStack(spacing: 0) {
            ColumnStrip(leads: arrangement.leading == .list) {
                leadButton(in: .list)
            } trailing: {
                if kinds != nil {
                    FlatMenu("Sort By", symbol: "arrow.up.arrow.down", help: "Sort order") {
                        Picker("Sort By", selection: $collectionSort) {
                            ForEach(CollectionSort.allCases) { Text($0.rawValue).tag($0) }
                        }
                        .pickerStyle(.inline)
                    }
                } else if let articleList {
                    Button { model.requestMarkAllRead(articleList) } label: {
                        Label("Mark All as Read", systemImage: "checkmark.circle")
                    }
                    .help("Mark every unread article here as read")
                    // It marks subscription articles, which aren't listed.
                    .disabled(selection == .smart(.continueReading) && readingScope == .collection)
                }
                Button { model.paletteVisible = true } label: { Label("Search", systemImage: "magnifyingglass") }
                    .help("Go to or search (⌘K)")
            }
            ColumnTitle(title: listTitle, note: listNote(kinds: kinds, rows: rows))
            if let kinds {
                CollectionList(
                    kinds: kinds, sort: collectionSort, selection: Binding(get: { entryID }, set: openEntry),
                    active: focus == .list || readerFocused,
                    focus: $focus, onReturn: read, onEscape: leaveList
                )
            } else {
                ThreePaneArticles(
                    rows: rows, marksFinished: selection == .smart(.continueReading),
                    active: focus == .list || readerFocused,
                    focus: $focus, read: read, leave: leaveList
                )
                    .equatable()
                if showsFilter {
                    Hairline()
                    SwitchBar(
                        selection: $filter,
                        options: [
                            .symbol(.starred, "star", help: String(localized: "Starred")),
                            .symbol(.unread, "circle.inset.filled", help: String(localized: "Unread")),
                            .symbol(.all, "line.3.horizontal", help: String(localized: "All")),
                        ],
                        segment: 52
                    )
                    .frame(height: 56)
                } else if selection == .smart(.continueReading) {
                    Hairline()
                    SwitchBar(selection: $readingScope, options: ReadingScope.options, segment: 52)
                        .frame(height: 56)
                }
            }
        }
    }

    /// What the line under the list's name says is in it.
    private func listNote(kinds: [CollectionKindView]?, rows: [ItemView]) -> Text {
        if let kinds {
            let count = model.collection.filter { kinds.contains($0.kind) }.count
            return Text("^[\(count) Item](inflect: true)")
        }
        return Text("^[\(rows.filter { !$0.read }.count) Unread Item](inflect: true)")
    }

    /// The lists that are a filter already have no switch under them.
    private var showsFilter: Bool {
        switch selection {
        case .smart(.unread), .smart(.continueReading), .saved, .kind, nil: false
        default: true
        }
    }

    /// The middle column as a list of subscription articles, for "Mark All
    /// as Read"; nil for the collection.
    private var articleList: ArticleList? {
        guard let selection else { return nil }
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

    /// The middle column lists unread articles only.
    private var showsUnreadOnly: Bool {
        switch selection {
        case .smart(.unread): true
        case .smart(.continueReading), .saved, .kind, nil: false
        default: unreadOnly && showsFilter
        }
    }

    /// Opening an article marks it read; it stays in an unread-only list,
    /// with the others read here, until the list starts over.
    private var rowsForSelection: [ItemView] {
        if selection == .smart(.continueReading) { return continuing }
        var kept = readHere
        if let current = model.currentItemID { kept.insert(current) }
        let keep = Array(kept)
        let rows: [ItemView] = switch selection {
        case .smart(.unread): model.items(.all, unreadOnly: true, keep: keep)
        case .smart(.starred): model.items(.starred, unreadOnly: unreadOnly, keep: keep)
        case let .folder(name): model.items(.category(name: name), unreadOnly: unreadOnly, keep: keep)
        case let .feed(id): model.items(.source(id: id), unreadOnly: unreadOnly, keep: keep)
        case let .tag(id): model.items(.tag(id: id), unreadOnly: unreadOnly, keep: keep)
        default: model.items(.all, unreadOnly: unreadOnly, keep: keep)
        }
        return filter == .starred && showsFilter ? rows.filter(\.starred) : rows
    }

    /// Continue Reading: what is being read, with the items opened from
    /// this list kept in it, in the order the list had when it started.
    /// Ones that have joined since come first.
    private var continuing: [ItemView] {
        let rows = model.items(.continueReading, keep: readHere.sorted()).filter(readingScope.contains)
        let new = rows.filter { continueOrder[$0.id] == nil }
        let known = rows.compactMap { row in continueOrder[row.id].map { (row, $0) } }.sorted { $0.1 < $1.1 }.map(\.0)
        return new + known
    }

    /// Opening `id` keeps its row where it is: in an unread-only list, and
    /// in Continue Reading for the rows it lists (not a chapter reached
    /// from one of them).
    private func holdsRows(_ id: String) -> Bool {
        if selection == .smart(.continueReading) { return continuing.contains { $0.id == id } }
        return showsUnreadOnly
    }

    /// The middle column's list starts over: rows kept in place are let go.
    private func startOver() {
        readHere = []
        continueOrder = selection == .smart(.continueReading)
            ? Dictionary(uniqueKeysWithValues: model.items(.continueReading).enumerated().map { ($1.id, $0) })
            : [:]
    }

    /// The list `old` and `new` are both in, to tell which side `new`
    /// comes in from: a book's chapters, else the middle column's articles.
    private func order(_ old: String?, _ new: String) -> [ItemView] {
        if let old, let series = model.item(new)?.seriesId, model.item(old)?.seriesId == series {
            return model.chapters(series)
        }
        return selection?.kinds == nil ? rowsForSelection : []
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

    /// Return in a list, or a click on a row where the list covers the
    /// reader: keyboard focus goes to the text.
    private func read() {
        go(.reader)
    }

    /// Esc in the reader: back to the contents panel when it is open, else
    /// to the middle column.
    private func leaveReader(contentsShown: Bool) {
        readerFocused = false
        if contentsShown { focus = .contents } else { go(.list) }
    }

    // MARK: Reader

    /// What leads the reader's strip: the button for the columns where the
    /// reader leads the window, and the one that closes what is open.
    private var readerLead: AnyView {
        AnyView(HStack(spacing: 4) {
            leadButton(in: .reader)
            Button { if tier >= .medium { open(nil) } else { go(.list) } } label: { Label("Close", systemImage: "xmark") }
                .help(tier >= .medium ? "Close the article" : "Back to the list (Esc)")
        })
    }

    @ViewBuilder
    private var detail: some View {
        if let id = model.currentItemID, let item = model.item(id) {
            let entry = model.entry(containing: item)
            // Beside the text where there is room; over it, on request, in
            // the narrowest tier.
            let beside = showContents && tier != .narrowest
            HStack(spacing: 0) {
                if let entry, beside {
                    EntryContents(
                        entry: entry, currentItemID: id, open: open, jump: jump, active: focus == .contents,
                        focus: $focus, onReturn: read, onEscape: { go(.list) }
                    )
                        .frame(width: 260)
                        .background(Theme.side)
                    Hairline(vertical: true)
                }
                VStack(spacing: 0) {
                    ReaderPane(
                        itemID: id, scrollTarget: $scrollTarget, pageTarget: pageTarget, openItem: open,
                        showContents: entry == nil ? nil : (tier == .narrowest ? $contentsDrawer : $showContents),
                        neighbors: neighbors(of: item, in: entry), startAtEnd: landAtEnd == id,
                        arrival: Arrival.side(arrival, for: id), leftLeaves: true,
                        onEscape: { leaveReader(contentsShown: entry != nil && beside) },
                        focused: readerFocused, onFocus: { readerFocused = true },
                        onBack: readerBack, stripLeads: arrangement.leading == .reader, stripLeading: readerLead
                    )
                    .readerSlot(id)
                    if let seriesID = item.seriesId, let info = model.series.first(where: { $0.id == seriesID }) {
                        Hairline()
                        SequenceBar(title: info.title, items: model.chapters(seriesID), current: id) { open($0) }
                    }
                }
            }
            .overlay {
                if entry != nil, contentsDrawer, tier == .narrowest {
                    Theme.scrim
                        .contentShape(Rectangle())
                        .onTapGesture(perform: closeContents)
                        .accessibilityHidden(true)
                        .transition(.opacity)
                }
            }
            .overlay(alignment: .leading) {
                if let entry, contentsDrawer, tier == .narrowest {
                    EntryContents(
                        entry: entry, currentItemID: id,
                        open: { open($0); closeContents() }, jump: { jump($0); closeContents() },
                        focus: $focus, onReturn: { closeContents(); read() }, onEscape: closeContents
                    )
                        .frame(width: 280)
                        .background(Theme.side)
                        .overlay(alignment: .trailing) { ColumnEdge() }
                        .transition(.move(edge: .leading))
                }
            }
        } else {
            VStack(spacing: 0) {
                ColumnStrip(leads: arrangement.leading == .reader) { leadButton(in: .reader) } trailing: { EmptyView() }
                ContentUnavailableView("No Article Selected", systemImage: "text.alignleft", description: Text("J and K move through the list."))
                    .frame(maxHeight: .infinity)
            }
        }
    }

    private func closeContents() {
        withAnimation(.easeInOut(duration: 0.2)) { contentsDrawer = false }
    }

    private func jump(_ target: OutlineEntry.Target) {
        switch target {
        case let .block(id): scrollTarget = ScrollTarget(blockID: id)
        case let .page(page, block): pageTarget = PageTarget(page: page, block: block)
        }
    }

    /// A chapter's book, or the articles of the middle column; nothing to
    /// move to from a page or PDF saved on its own, unless it is one of the
    /// rows of Continue Reading.
    private func neighbors(of item: ItemView, in entry: CollectionEntryView?) -> Neighbors? {
        if let seriesID = item.seriesId {
            return .chapters({ model.chapters(seriesID) }, current: item.id, open: travel)
        }
        guard entry == nil || selection == .smart(.continueReading) else { return nil }
        return .articles({ rowsForSelection }, current: item.id, history: history, item: { model.item($0) }, open: travel)
    }

    /// Opens a neighbor; its text comes in from the side it lies on.
    private func travel(to id: String, _ direction: Direction, _ atEnd: Bool) {
        arrival = Arrival(item: id, side: ArrivalSide(direction))
        landAtEnd = atEnd ? id : nil
        open(id)
    }
}

/// Three-pane's sidebar. A view of its own, and equal to itself for as long
/// as what is chosen in it and where it stands are the same: three-pane's
/// body runs several times as a swipe between the columns ends, and each
/// run would otherwise bring every row here up to date again.
private struct ThreePaneSidebar: View, Equatable {
    @Environment(AppModel.self) private var model
    let selection: ThreePaneView.Selection?
    let tier: PaneTier
    let canRead: Bool
    /// The subscription folders folded up. The view that owns them hands
    /// them over: a folder folding has to count as a change here, or the
    /// rows wouldn't follow until something else changed.
    let collapsed: Set<String>
    /// Keyboard focus is in this column: the row chosen carries the mark.
    let active: Bool
    /// Given the list's table, for the owner to slide rows on.
    let table: TableHandle
    var focus: FocusState<PaneFocus?>.Binding
    let choose: (ThreePaneView.Selection?) -> Void
    let go: (PanePlace) -> Void
    /// Return or Right: on to the middle column.
    let enter: () -> Void
    /// Folds a folder, or unfolds it.
    let toggle: (String) -> Void

    static func == (a: Self, b: Self) -> Bool {
        a.selection == b.selection && a.tier == b.tier && a.canRead == b.canRead
            && a.collapsed == b.collapsed && a.active == b.active
    }

    var body: some View {
        ScrollViewReader { proxy in
            rows(lead: 8)
                // Coming from another mode with a subscription or folder
                // chosen there: it may lie further down than shows. Once
                // the list has its rows.
                .onAppear { DispatchQueue.main.async { reveal(selection, in: proxy, anchor: .center) } }
                // Chosen from elsewhere (the palette, a subscription's
                // folder with Left): into view, if it isn't. A row out of
                // view isn't the table's selected row, and the arrow keys
                // would start from the top.
                .onChange(of: selection) { _, new in reveal(new, in: proxy, anchor: nil) }
        }
    }

    /// Which row of the list a folder is, counting the rows `rows` puts
    /// above it; nil if there is no such folder. Keep it in step with
    /// `rows`.
    static func row(ofFolder name: String, in model: AppModel, collapsed: Set<String>) -> Int? {
        let folders = model.folders
        guard let index = folders.firstIndex(of: name) else { return nil }
        // "Library", the smart lists, Saved.
        var row = 1 + ThreePaneView.Smart.allCases.count + 1
        let kinds = CollectionKindView.all.filter { kind in model.collection.contains { $0.kind == kind } }
        if !kinds.isEmpty { row += 1 + kinds.count }
        // "Subscriptions", then each folder before with what it shows.
        row += 1
        for folder in folders[..<index] {
            row += 1 + (collapsed.contains(folder) ? 0 : model.feeds(inFolder: folder).count)
        }
        return row
    }

    /// Brings the row chosen into view, where it is one of the many.
    private func reveal(_ selection: ThreePaneView.Selection?, in proxy: ScrollViewProxy, anchor: UnitPoint?) {
        switch selection {
        case let .feed(id): proxy.scrollTo(id, anchor: anchor)
        case let .folder(name): proxy.scrollTo(name, anchor: anchor)
        case let .tag(id): proxy.scrollTo(id, anchor: anchor)
        default: break
        }
    }

    private func rows(lead: CGFloat) -> some View {
        List(selection: Binding(get: { selection }, set: choose)) {
            SideHeading(title: "Library", first: true, lead: lead)
            ForEach(ThreePaneView.Smart.allCases, id: \.self) { smart in
                let tag = ThreePaneView.Selection.smart(smart)
                SideRow(symbol: smart.icon, title: Text(LocalizedStringKey(smart.rawValue)), count: badge(smart), chosen: selection == tag)
                    .flatRow(chosen: selection == tag, active: active, lead: lead)
                    .tag(tag)
            }
            SideRow(symbol: "bookmark", title: Text("Saved"), count: model.collection.count, chosen: selection == .saved)
                .flatRow(chosen: selection == .saved, active: active, lead: lead)
                .tag(ThreePaneView.Selection.saved)
            let kinds = CollectionKindView.all.filter { kind in model.collection.contains { $0.kind == kind } }
            if !kinds.isEmpty {
                SideHeading(title: "Collection", lead: lead)
                ForEach(kinds) { kind in
                    let tag = ThreePaneView.Selection.kind(kind)
                    SideRow(symbol: kind.icon, title: Text(kind.title), count: model.collection.filter { $0.kind == kind }.count, chosen: selection == tag)
                        .flatRow(chosen: selection == tag, active: active, lead: lead)
                        .tag(tag)
                }
            }
            if !model.feeds.isEmpty {
                SideHeading(title: "Subscriptions", lead: lead)
                // A folder's subscriptions are rows of the list like any
                // other, there while it is open: the list has no folding of
                // its own to draw an arrow for or to animate differently
                // for a click and a key.
                ForEach(model.folders, id: \.self) { folder in
                    let tag = ThreePaneView.Selection.folder(folder)
                    let open = !collapsed.contains(folder)
                    HStack(spacing: 0) {
                        FoldArrow(open: open) { toggle(folder) }
                        SideRow(symbol: "folder", title: Text(verbatim: folder), count: model.unread(inFolder: folder), chosen: selection == tag) {
                            if model.inGallery(folder: folder) { GalleryMark() }
                        }
                    }
                    // The arrow stands before the icon, in the room every
                    // row has at its start, so the icons stay in one line.
                    .flatRow(chosen: selection == tag, active: active, lead: lead - FoldArrow.width)
                    .tag(tag)
                    .contextMenu {
                        Button("Mark All as Read…") { model.requestMarkAllRead(ArticleList(filter: .category(name: folder), title: folder)) }
                        let shown = model.inGallery(folder: folder)
                        Button(shown ? "Remove from Gallery" : "Show in Gallery") { model.setGallery(folder: folder, !shown) }
                    }
                    if open { feedRows(model.feeds(inFolder: folder), lead + 14) }
                }
                feedRows(model.feeds(inFolder: nil), lead)
            }
            if !model.tags.isEmpty {
                SideHeading(title: "Tags", lead: lead)
                ForEach(model.tags) { t in
                    let tag = ThreePaneView.Selection.tag(t.id)
                    SideRow(symbol: "number", title: Text(verbatim: t.name), count: Int(t.count), chosen: selection == tag)
                        .flatRow(chosen: selection == tag, active: active, lead: lead)
                        .tag(tag)
                }
            }
        }
        .flatList()
        .tableHandle(table)
        .pane(focus, .sidebar, onReturn: enter, onEscape: sidebarEscape)
        .onKeyPress(keys: [.leftArrow, .rightArrow, .space]) { press in key(press.key) }
    }

    /// Esc in the sidebar sends it away again, where it is in over the
    /// other columns.
    private var sidebarEscape: (() -> Void)? {
        switch tier {
        case .wide: nil
        case .narrow: canRead ? { go(.reader) } : nil
        case .medium, .narrowest: { go(.list) }
        }
    }

    /// Right goes on to the middle column, as Return does. Left goes back a
    /// level: from a subscription to its folder, else where Esc leads.
    /// Space folds the folder chosen, or unfolds it.
    private func key(_ key: KeyEquivalent) -> KeyPress.Result {
        switch key {
        case .space:
            guard case let .folder(name) = selection else { return .ignored }
            toggle(name)
            return .handled
        case .rightArrow:
            enter()
            return .handled
        default:
            if case let .feed(id) = selection, let folder = model.feeds.first(where: { $0.id == id })?.category {
                choose(.folder(folder))
                return .handled
            }
            guard let sidebarEscape else { return .ignored }
            sidebarEscape()
            return .handled
        }
    }

    private func badge(_ smart: ThreePaneView.Smart) -> Int {
        switch smart {
        case .unread: model.unreadTotal
        case .continueReading: model.continueReadingCount
        default: 0
        }
    }

    private func feedRows(_ feeds: [SourceView], _ lead: CGFloat) -> some View {
        ForEach(feeds) { feed in
            let tag = ThreePaneView.Selection.feed(feed.id)
            SideRow(symbol: "dot.radiowaves.up.forward", title: Text(verbatim: feed.title), count: Int(feed.unread), chosen: selection == tag) {
                if let reason = model.galleryReason(feed) { GalleryMark(throughFolder: reason.isEmpty ? nil : reason) }
            }
            .flatRow(chosen: selection == tag, active: active, lead: lead)
            .tag(tag)
            .contextMenu {
                Button("Mark All as Read…") { model.requestMarkAllRead(ArticleList(filter: .source(id: feed.id), title: feed.title)) }
                if let folder = feed.category, model.inGallery(folder: folder) {
                    Button("Shown in Gallery with “\(folder)”") {}.disabled(true)
                } else {
                    Button(feed.gallery ? "Remove from Gallery" : "Show in Gallery") { model.setGallery(source: feed, !feed.gallery) }
                }
                FullTextMenu(feed: feed)
            }
        }
    }
}

/// The middle column's articles. A view of its own, equal to itself while
/// it lists the same rows, for the reason the sidebar is.
private struct ThreePaneArticles: View, Equatable {
    @Environment(AppModel.self) private var model
    let rows: [ItemView]
    /// Finished articles show a check: Continue Reading keeps them listed.
    let marksFinished: Bool
    /// The keys are with the articles, in this column or in the one being
    /// read: its row carries the mark.
    let active: Bool
    var focus: FocusState<PaneFocus?>.Binding
    /// Return, or a double click: on to the reader.
    let read: () -> Void
    /// Esc: back a column.
    let leave: () -> Void

    static func == (a: Self, b: Self) -> Bool { a.rows == b.rows && a.marksFinished == b.marksFinished && a.active == b.active }

    var body: some View {
        @Bindable var model = model
        return List(rows, selection: $model.currentItemID) { item in
            ArticleRow(item: item, marksFinished: marksFinished)
                .flatRow(chosen: item.id == model.currentItemID, active: active, inset: 6)
                .tag(item.id)
        }
        .contextMenu(forSelectionType: String.self) { ids in
            if let id = ids.first, let item = rows.first(where: { $0.id == id }) {
                Button(item.starred ? "Unstar" : "Star") { model.setStarred(item, !item.starred) }
                if !item.isDone {
                    Button("Mark as Read") { model.markRead([item.id]) }
                }
                if item.hasBeenRead {
                    Button("Mark as Unread") { model.markUnread([item.id]) }
                }
                if model.canExport(item.id, fromReader: false) {
                    Menu("Export") {
                        Button("Export as PDF…") { model.beginExport(targetID: item.id, format: .pdf, fromReader: false) }
                        Button("Export as EPUB…") { model.beginExport(targetID: item.id, format: .epub, fromReader: false) }
                    }
                }
                if let url = item.url.flatMap(URL.init(string:)) {
                    if isVideoUrl(url: url.absoluteString) {
                        Button("Convert Video to Book…") {
                            model.requestVideoConvert(source: url.absoluteString, title: item.title)
                        }
                    }
                    Button("Open in Browser") { NSWorkspace.shared.open(url) }
                    Button("Copy Link") {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(url.absoluteString, forType: .string)
                    }
                }
            }
        } primaryAction: { _ in read() }
        .flatList()
        .pane(focus, .list, onReturn: read, onEscape: leave)
        // Right goes on to the article, Left back a column.
        .onKeyPress(keys: [.leftArrow, .rightArrow]) { press in
            if press.key == .rightArrow { read() } else { leave() }
            return .handled
        }
        .overlay {
            if rows.isEmpty {
                ContentUnavailableView("Nothing Here", systemImage: "tray")
            }
        }
        .background { keyboard(rows) }
    }

    /// j/k move through the list, s stars, m toggles read.
    private func keyboard(_ rows: [ItemView]) -> some View {
        Group {
            Button("Next") { move(rows, 1) }.keyboardShortcut("j", modifiers: [])
            Button("Previous") { move(rows, -1) }.keyboardShortcut("k", modifiers: [])
            Button("Star") { if let item = current(rows) { model.setStarred(item, !item.starred) } }.keyboardShortcut("s", modifiers: [])
            Button("Read") { if let item = current(rows) { model.toggleRead(item) } }.keyboardShortcut("m", modifiers: [])
        }
        .disabled(model.overlayVisible)
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
}

/// Middle column for Saved and the collection kinds; Saved groups its
/// entries by kind.
struct CollectionList: View {
    @Environment(AppModel.self) private var model
    let kinds: [CollectionKindView]
    let sort: CollectionSort
    @Binding var selection: String?
    /// The keys are with the entries: the one selected carries the mark.
    var active = false
    var focus: FocusState<PaneFocus?>.Binding?
    /// Return or Right: read the entry selected.
    var onReturn: (() -> Void)?
    /// Esc or Left: back a column.
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
                        SideHeading(title: kind.title)
                        rows(group)
                    }
                }
            }
        }
        .flatList()
        .contextMenu(forSelectionType: String.self) { ids in
            if let id = ids.first, let entry = model.entry(id) {
                if !entry.isDone {
                    Button("Mark as Read") { model.markRead(entry: entry) }
                }
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
        } primaryAction: { _ in onReturn?() }
        .pane(focus, .list, onReturn: onReturn, onEscape: onEscape)
        .onKeyPress(keys: [.leftArrow, .rightArrow]) { press in
            guard let act = press.key == .rightArrow ? onReturn : onEscape else { return .ignored }
            act()
            return .handled
        }
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
            CollectionRow(entry: entry).flatRow(chosen: entry.id == selection, active: active, inset: 6).tag(entry.id)
        }
    }

    private func deleteSelected() {
        if let selection, let entry = model.entry(selection) { model.requestDelete(entry) }
    }
}

struct StatusFooter: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 6) {
                Image(systemName: icon)
                Text(text).lineLimit(1)
                Spacer()
                if sync == nil, model.busy || model.historyAdded > 0 { ProgressView().controlSize(.mini) }
            }
            // A long sync (the first one after pairing, or after a long time
            // offline) shows how far it is.
            if let sync {
                if let fraction = sync.fraction {
                    ProgressView(value: fraction).controlSize(.small)
                } else {
                    ProgressView().progressViewStyle(.linear).controlSize(.small)
                }
            }
        }
        .font(Theme.small)
        .foregroundStyle(Theme.text2)
        .padding(.horizontal, 20)
        .padding(.vertical, 8)
        .help(sync?.detail ?? "")
    }

    private var sync: SyncProgressView? { model.status?.sync }

    private var icon: String {
        switch model.status?.mode {
        case .server: "arrow.triangle.2.circlepath.icloud"
        case .standalone: "dot.radiowaves.left.and.right"
        default: "internaldrive"
        }
    }

    private var text: String {
        switch model.status?.mode {
        case .server where sync != nil: sync?.summary ?? ""
        case .server where model.serverOffline: "Sync server offline"
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

extension ThreePaneView.Selection {
    /// What Three-Pane's sidebar shows for a shelf of the Library. All
    /// subscriptions are All Articles here; the shelf of subscriptions in
    /// no folder has no counterpart.
    init(_ filter: LibraryFilter, folders: [String]) {
        switch filter {
        case .all, .subscriptions: self = .smart(.all)
        case .continueReading: self = .smart(.continueReading)
        case .saved: self = .saved
        case let .kind(kind): self = .kind(kind)
        case let .category(name): self = folders.contains(name) ? .folder(name) : .smart(.all)
        }
    }
}
