import EddyKit
import SwiftUI

/// Direction A — classic three panes (Reeder / NetNewsWire): sources and
/// smart lists, a dense article list, the reader. Keyboard first.
struct ClassicView: View {
    @Environment(DemoStore.self) private var store
    @State private var selection: SidebarItem? = .smart(.all)
    @State private var selectedItem: String?
    @State private var unreadOnly = false
    @State private var query = ""
    @State private var mode: ReadingMode = .original
    @State private var showSummary = false

    enum Smart: String, CaseIterable {
        case all = "全部", unread = "未读", starred = "星标", continueReading = "继续阅读", saved = "保存的网页"
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
        case smart(Smart), series(String), tag(String), source(String), category(String)
    }

    var body: some View {
        NavigationSplitView {
            sidebar.navigationSplitViewColumnWidth(min: 200, ideal: 230)
        } content: {
            list.navigationSplitViewColumnWidth(min: 300, ideal: 360)
        } detail: {
            detail
        }
        .searchable(text: $query, placement: .toolbar, prompt: "搜索全文")
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
                Section("系列") {
                    ForEach(store.series) { s in
                        HStack {
                            Label(s.title, systemImage: "books.vertical")
                            Spacer()
                            Text("\(Int(s.progress * 100))%").font(.caption).foregroundStyle(.secondary)
                        }
                        .tag(SidebarItem.series(s.id))
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
            ForEach(categories, id: \.self) { category in
                Section(category) {
                    ForEach(store.sources.filter { !$0.saved && ($0.category ?? "订阅") == category }) { s in
                        Label(s.title, systemImage: "dot.radiowaves.up.forward").badge(Int(s.unread)).tag(SidebarItem.source(s.id))
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

    // MARK: List

    private var filter: ItemFilterView {
        switch selection {
        case .smart(.starred): .starred
        case .smart(.continueReading): .continueReading
        case .smart(.saved): .saved
        case let .series(id): .series(id: id)
        case let .tag(id): .tag(id: id)
        case let .source(id): .source(id: id)
        case let .category(name): .category(name: name)
        default: .all
        }
    }

    private var rows: [ItemView] {
        if !query.isEmpty { return store.search(query).map(\.item) }
        return store.items(filter, unreadOnly: unreadOnly || selection == .smart(.unread))
    }

    private var list: some View {
        List(rows, selection: $selectedItem) { item in
            ClassicRow(item: item).tag(item.id)
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

    private var current: ItemView? { rows.first { $0.id == selectedItem } }

    private func move(_ delta: Int) {
        let ids = rows.map(\.id)
        guard !ids.isEmpty else { return }
        let index = selectedItem.flatMap { ids.firstIndex(of: $0) } ?? -1
        selectedItem = ids[min(max(index + delta, 0), ids.count - 1)]
    }

    // MARK: Detail

    @ViewBuilder
    private var detail: some View {
        if let id = selectedItem, let item = store.item(id) {
            ArticleView(itemID: id, mode: $mode)
                .id(id)
                .toolbar {
                    ToolbarItemGroup {
                        Picker("阅读模式", selection: $mode) {
                            ForEach(ReadingMode.allCases) { Text($0.rawValue).tag($0) }
                        }
                        .pickerStyle(.segmented)
                        .frame(width: 160)
                        Button { showSummary.toggle() } label: { Image(systemName: "sparkles") }
                            .help("AI 摘要")
                            .popover(isPresented: $showSummary) { SummaryCard(itemID: id) }
                        Button { Task { await store.fetchFullText(id) } } label: { Image(systemName: "doc.text.magnifyingglass") }
                            .help("抓取全文")
                            .disabled(item.kind == .saved)
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

struct ClassicRow: View {
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
