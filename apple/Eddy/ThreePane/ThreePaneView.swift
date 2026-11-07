import EddyKit
import SwiftUI

/// Three-pane mode: filters in the sidebar, what they select in the middle
/// (articles, or the entries of the collection), the reader on the right.
/// A collection entry keeps its contents in a panel beside the text.
struct ThreePaneView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("app.mode") private var mode: AppMode = .threePane
    @AppStorage("threePane.contents") private var showContents = false
    @State private var selection: Selection? = .smart(.all)
    /// The collection entry picked in the middle column.
    @State private var entryID: String?
    @State private var unreadOnly = false
    @State private var query = ""
    @State private var results: [ItemView]?
    @State private var scrollTarget: ScrollTarget?
    @State private var pageTarget: PageTarget?

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
        case smart(Smart), saved, kind(CollectionKindView), feed(String), tag(String)

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
        .toolbar { ModeToolbar(mode: $mode, model: model) }
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
        .onChange(of: model.currentItemID) { pageTarget = nil }
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
                        .badge(smart == .unread ? model.unreadTotal : 0)
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
            ForEach(categories, id: \.self) { category in
                Section(category) {
                    ForEach(model.feeds.filter { ($0.category ?? "Subscriptions") == category }) { feed in
                        Label(feed.title, systemImage: "dot.radiowaves.up.forward").badge(Int(feed.unread)).tag(Selection.feed(feed.id))
                    }
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
        .safeAreaInset(edge: .bottom) { StatusFooter() }
    }

    private var categories: [String] {
        Array(Set(model.feeds.map { $0.category ?? "Subscriptions" })).sorted()
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

    private func show(entryID id: String) {
        guard let entry = model.entry(id) else { return }
        selection = .kind(entry.kind)
        entryID = entry.id
    }

    // MARK: Middle column

    @ViewBuilder
    private var middle: some View {
        if results == nil, let kinds = selection?.kinds {
            CollectionList(kinds: kinds, selection: Binding(get: { entryID }, set: openEntry))
                .toolbar { refreshItem }
        } else {
            articles
        }
    }

    private var articles: some View {
        @Bindable var model = model
        let rows = results ?? rowsForSelection
        return List(rows, selection: $model.currentItemID) { item in
            ArticleRow(item: item).tag(item.id)
                .contextMenu {
                    Button(item.starred ? "Unstar" : "Star") { model.setStarred(item, !item.starred) }
                    Button(item.read ? "Mark as Unread" : "Mark as Read") { model.setRead(item, !item.read) }
                    if let url = item.url.flatMap(URL.init(string:)) {
                        Button("Open in Browser") { NSWorkspace.shared.open(url) }
                        Button("Copy Link") {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(url.absoluteString, forType: .string)
                        }
                    }
                }
        }
        .listStyle(.inset)
        .overlay {
            if rows.isEmpty {
                ContentUnavailableView(results != nil ? "No Results" : "Nothing Here", systemImage: results != nil ? "magnifyingglass" : "tray")
            }
        }
        .toolbar {
            ToolbarItem {
                Toggle(isOn: $unreadOnly) { Label("Unread Only", systemImage: "line.3.horizontal.decrease.circle") }.help("Show unread only")
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

    private var rowsForSelection: [ItemView] {
        switch selection {
        case .smart(.unread): model.items(.all, unreadOnly: true)
        case .smart(.starred): model.items(.starred, unreadOnly: unreadOnly)
        case .smart(.continueReading): model.items(.continueReading)
        case let .feed(id): model.items(.source(id: id), unreadOnly: unreadOnly)
        case let .tag(id): model.items(.tag(id: id), unreadOnly: unreadOnly)
        default: model.items(.all, unreadOnly: unreadOnly)
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

    /// j/k move through the list, s stars, m toggles read.
    private func keyboard(_ rows: [ItemView]) -> some View {
        Group {
            Button("Next") { move(rows, 1) }.keyboardShortcut("j", modifiers: [])
            Button("Previous") { move(rows, -1) }.keyboardShortcut("k", modifiers: [])
            Button("Star") { if let item = current(rows) { model.setStarred(item, !item.starred) } }.keyboardShortcut("s", modifiers: [])
            Button("Read") { if let item = current(rows) { model.setRead(item, !item.read) } }.keyboardShortcut("m", modifiers: [])
        }
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
                    EntryContents(entry: entry, currentItemID: id, open: open, jump: jump)
                        .frame(width: 260)
                        .background(.background.secondary)
                    Divider()
                }
                VStack(spacing: 0) {
                    ReaderPane(itemID: id, scrollTarget: $scrollTarget, pageTarget: pageTarget, openItem: open, showContents: entry == nil ? nil : $showContents)
                        .id(id)
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
}

/// Middle column for Saved and the collection kinds; Saved groups its
/// entries by kind.
struct CollectionList: View {
    @Environment(AppModel.self) private var model
    let kinds: [CollectionKindView]
    @Binding var selection: String?

    var body: some View {
        let entries = model.collection.filter { kinds.contains($0.kind) }.sorted(by: CollectionEntryView.byRecency)
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
                Button("Delete…", role: .destructive) { model.requestDelete(entry) }
            }
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
