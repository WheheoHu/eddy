import EddyKit
import SwiftUI

/// Three-pane mode: subscriptions browse as lists; publications (tutorials,
/// books, papers) are sources whose second column lists their chapters.
struct ThreePaneMode: View {
    @Environment(DemoStore.self) private var store
    @State private var selection: SidebarItem? = .smart(.all)
    @State private var unreadOnly = false
    @State private var query = ""
    @State private var mode: ReadingMode = .original
    @State private var showSummary = false

    enum Smart: String, CaseIterable {
        case all = "全部", unread = "未读", starred = "星标", continueReading = "继续阅读", saved = "稍后读"
        var icon: String {
            switch self {
            case .all: "tray.full"
            case .unread: "circle.inset.filled"
            case .starred: "star"
            case .continueReading: "book"
            case .saved: "bookmark"
            }
        }
    }

    enum SidebarItem: Hashable {
        case smart(Smart), publication(String), tag(String), source(String)
    }

    var body: some View {
        @Bindable var store = store
        NavigationSplitView {
            sidebar.navigationSplitViewColumnWidth(min: 210, ideal: 240)
        } content: {
            Group {
                if case let .publication(id) = selection {
                    ChapterList(seriesID: id, selected: $store.currentItemID)
                } else {
                    list
                }
            }
            .navigationSplitViewColumnWidth(min: 300, ideal: 360)
        } detail: {
            detail
        }
        .searchable(text: $query, placement: .toolbar, prompt: "搜索全文")
        .onAppear { syncSelectionFromStore() }
        .onChange(of: selection) { _, new in
            if case let .publication(id) = new { store.currentSeriesID = id } else { store.currentSeriesID = nil }
        }
    }

    /// Coming from reader mode: show the publication being read.
    private func syncSelectionFromStore() {
        if let series = store.currentSeriesID { selection = .publication(series) }
    }

    // MARK: Sidebar

    private var sidebar: some View {
        List(selection: $selection) {
            Section("智能列表") {
                ForEach(Smart.allCases, id: \.self) { smart in
                    Label(smart.rawValue, systemImage: smart.icon)
                        .badge(smart == .unread ? unreadTotal : 0)
                        .tag(SidebarItem.smart(smart))
                }
            }
            if !store.series.isEmpty {
                Section("出版物") {
                    ForEach(store.series) { s in
                        HStack(spacing: 8) {
                            Image(systemName: "book.closed").foregroundStyle(.secondary)
                            Text(s.title).lineLimit(1)
                            Spacer()
                            ProgressRing(value: s.progress, size: 14)
                        }
                        .tag(SidebarItem.publication(s.id))
                    }
                }
            }
            ForEach(categories, id: \.self) { category in
                Section(category) {
                    ForEach(store.sources.filter { !$0.saved && ($0.category ?? "订阅") == category }) { s in
                        Label(s.title, systemImage: "dot.radiowaves.up.forward").badge(Int(s.unread)).tag(SidebarItem.source(s.id))
                    }
                }
            }
            if !store.tags.isEmpty {
                Section("标签") {
                    ForEach(store.tags) { t in
                        Label(t.name, systemImage: "number").badge(Int(t.count)).tag(SidebarItem.tag(t.id))
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .safeAreaInset(edge: .bottom) { SyncFooter() }
    }

    private var categories: [String] {
        Array(Set(store.sources.filter { !$0.saved }.map { $0.category ?? "订阅" })).sorted()
    }

    private var unreadTotal: Int {
        store.sources.reduce(0) { $0 + Int($1.unread) }
    }

    // MARK: Article list

    private var filter: ItemFilterView {
        switch selection {
        case .smart(.starred): .starred
        case .smart(.continueReading): .continueReading
        case .smart(.saved): .saved
        case let .tag(id): .tag(id: id)
        case let .source(id): .source(id: id)
        default: .all
        }
    }

    private var rows: [ItemView] {
        if !query.isEmpty { return store.search(query).map(\.item) }
        return store.items(filter, unreadOnly: unreadOnly || selection == .smart(.unread))
    }

    private var list: some View {
        @Bindable var store = store
        return List(rows, selection: $store.currentItemID) { item in
            ArticleRow(item: item).tag(item.id)
                .contextMenu {
                    Button(item.starred ? "取消星标" : "加星标") { store.setStarred(item, !item.starred) }
                    Button(item.read ? "标为未读" : "标为已读") { store.setRead(item, !item.read) }
                }
        }
        .listStyle(.inset)
        .overlay {
            if rows.isEmpty { ContentUnavailableView(query.isEmpty ? "这里还没有文章" : "没有找到", systemImage: "tray") }
        }
        .toolbar {
            ToolbarItem {
                Toggle(isOn: $unreadOnly) { Image(systemName: "line.3.horizontal.decrease.circle") }.help("只看未读")
            }
            ToolbarItem {
                Button { Task { await store.refresh() } } label: { Image(systemName: "arrow.clockwise") }.help("刷新").disabled(store.busy)
            }
        }
        .background { keyboard }
    }

    /// j/k move through the list, s stars, t toggles translation.
    private var keyboard: some View {
        Group {
            Button("") { move(1) }.keyboardShortcut("j", modifiers: [])
            Button("") { move(-1) }.keyboardShortcut("k", modifiers: [])
            Button("") { if let item = current { store.setStarred(item, !item.starred) } }.keyboardShortcut("s", modifiers: [])
            Button("") { mode = mode == .original ? .bilingual : .original }.keyboardShortcut("t", modifiers: [])
        }
        .opacity(0)
        .frame(width: 0, height: 0)
    }

    private var current: ItemView? { store.currentItemID.flatMap { store.item($0) } }

    private func move(_ delta: Int) {
        let ids = rows.map(\.id)
        guard !ids.isEmpty else { return }
        let index = store.currentItemID.flatMap { ids.firstIndex(of: $0) } ?? -1
        store.currentItemID = ids[min(max(index + delta, 0), ids.count - 1)]
    }

    // MARK: Reader

    @ViewBuilder
    private var detail: some View {
        if let id = store.currentItemID, let item = store.item(id) {
            ArticleView(itemID: id, mode: $mode)
                .id(id)
                .safeAreaInset(edge: .bottom) {
                    if let seriesID = item.seriesId, let info = store.series.first(where: { $0.id == seriesID }) {
                        SequenceBar(title: info.title, items: store.seriesChapters(seriesID), current: id) { store.currentItemID = $0 }
                    }
                }
                .toolbar {
                    ToolbarItemGroup {
                        Picker("阅读模式", selection: $mode) {
                            ForEach(ReadingMode.allCases) { Text($0.rawValue).tag($0) }
                        }
                        .pickerStyle(.segmented)
                        .frame(width: 150)
                        Button { showSummary.toggle() } label: { Image(systemName: "sparkles") }
                            .help("AI 摘要")
                            .popover(isPresented: $showSummary) { SummaryCard(itemID: id) }
                        Button { Task { await store.fetchFullText(id) } } label: { Image(systemName: "doc.text.magnifyingglass") }
                            .help("抓取全文")
                            .disabled(item.kind != .feed)
                        Button { store.setStarred(item, !item.starred) } label: { Image(systemName: item.starred ? "star.fill" : "star") }
                            .help("星标 (s)")
                        if let url = item.url.flatMap(URL.init(string:)) {
                            Link(destination: url) { Image(systemName: "safari") }.help("打开原网页")
                        }
                    }
                }
        } else {
            ContentUnavailableView("选一篇文章", systemImage: "text.alignleft", description: Text("j / k 上下切换，s 加星标，t 切换双语"))
        }
    }
}

/// Second column for a publication: overall progress and its chapters.
struct ChapterList: View {
    @Environment(DemoStore.self) private var store
    let seriesID: String
    @Binding var selected: String?

    var body: some View {
        let chapters = store.seriesChapters(seriesID)
        let info = store.series.first { $0.id == seriesID }
        List(selection: $selected) {
            if let info {
                Section {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(info.title).font(.title3.weight(.semibold))
                        ProgressView(value: info.progress)
                        HStack {
                            Text("已读 \(info.finished) / \(info.chapters) 章").font(.caption).foregroundStyle(.secondary)
                            Spacer()
                            if let next = info.nextItemId, let chapter = chapters.first(where: { $0.id == next }) {
                                Button("继续：\(chapter.title)") { selected = next }
                                    .buttonStyle(.borderedProminent)
                                    .controlSize(.small)
                                    .lineLimit(1)
                            }
                        }
                    }
                    .padding(.vertical, 6)
                }
            }
            Section("章节") {
                ForEach(Array(chapters.enumerated()), id: \.element.id) { index, chapter in
                    HStack(spacing: 10) {
                        Text("\(index + 1)").font(.caption.monospacedDigit()).foregroundStyle(.secondary).frame(width: 22, alignment: .trailing)
                        Text(Self.displayTitle(chapter.title, number: index + 1)).lineLimit(2).fontWeight(chapter.finished ? .regular : .medium).foregroundStyle(chapter.finished ? .secondary : .primary)
                        Spacer()
                        if chapter.finished {
                            Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                        } else if chapter.progress > 0.02 {
                            ProgressRing(value: chapter.progress, size: 14)
                        }
                    }
                    .padding(.vertical, 3)
                    .tag(chapter.id)
                }
            }
        }
        .listStyle(.inset)
    }

    /// Chapter titles often carry their own number ("2. Using …"); the list
    /// already shows it.
    static func displayTitle(_ title: String, number: Int) -> String {
        for prefix in ["\(number). ", "\(number) ", "第\(number)章 ", "Chapter \(number): ", "Chapter \(number). "] where title.hasPrefix(prefix) {
            return String(title.dropFirst(prefix.count))
        }
        return title
    }
}

struct ArticleRow: View {
    let item: ItemView

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Circle().fill(item.read ? Color.clear : Color.accentColor).frame(width: 7, height: 7)
                Text(item.sourceName).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                Spacer()
                if item.starred { Image(systemName: "star.fill").font(.caption2).foregroundStyle(.yellow) }
                if item.progress > 0.02 && !item.finished { ProgressRing(value: item.progress, size: 12) }
                Text(item.date, format: .relative(presentation: .named)).font(.caption).foregroundStyle(.tertiary)
            }
            Text(item.title).font(.body.weight(item.read ? .regular : .semibold)).lineLimit(2)
            if let excerpt = item.excerpt, !excerpt.isEmpty {
                Text(excerpt).font(.callout).foregroundStyle(.secondary).lineLimit(2)
            }
        }
        .padding(.vertical, 4)
    }
}

/// Mode and last sync, shown under the sidebar.
struct SyncFooter: View {
    @Environment(DemoStore.self) private var store

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: store.status?.mode == .server ? "icloud" : store.status?.mode == .standalone ? "dot.radiowaves.left.and.right" : "internaldrive")
            Text(modeText).lineLimit(1)
            Spacer()
            if store.busy { ProgressView().controlSize(.mini) }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(10)
    }

    private var modeText: String {
        switch store.status?.mode {
        case .server: "服务端同步"
        case .standalone: "直连 FreshRSS"
        default: "单机 · 示例数据"
        }
    }
}
