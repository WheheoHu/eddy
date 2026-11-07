import EddyKit
import SwiftUI

/// Three-pane mode: subscriptions browse as lists; publications and saved
/// pages are sources too, with their chapters or pages in the middle column.
struct ThreePaneView: View {
    @Environment(AppModel.self) private var model
    @State private var selection: Selection? = .smart(.all)
    @State private var unreadOnly = false
    @State private var query = ""
    @State private var results: [ItemView]?
    @State private var scrollTarget: ScrollTarget?

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
        case smart(Smart), savedAll, savedSite(String), publication(String), feed(String), tag(String)
    }

    var body: some View {
        @Bindable var model = model
        NavigationSplitView {
            sidebar.navigationSplitViewColumnWidth(min: 210, ideal: 240)
        } content: {
            middle.navigationSplitViewColumnWidth(min: 300, ideal: 360)
        } detail: {
            detail
        }
        .searchable(text: $query, placement: .toolbar, prompt: "Search")
        .onChange(of: model.revealed) { _, revealed in
            switch revealed {
            case let .publication(id): selection = .publication(id)
            case let .document(id): selection = model.item(id).map { .savedSite($0.host) } ?? .savedAll
            case nil: return
            }
            query = ""
            model.revealed = nil
        }
        .task(id: query) {
            guard !query.isEmpty else { results = nil; return }
            try? await Task.sleep(for: .milliseconds(250))
            results = await model.search(query).map(\.item)
        }
    }

    // MARK: Sidebar

    private var sidebar: some View {
        List(selection: $selection) {
            Section("Library") {
                ForEach(Smart.allCases, id: \.self) { smart in
                    Label(smart.rawValue, systemImage: smart.icon)
                        .badge(smart == .unread ? model.unreadTotal : 0)
                        .tag(Selection.smart(smart))
                }
            }
            let sites = model.savedSites
            if !sites.isEmpty {
                Section("Saved") {
                    Label("All Saved Pages", systemImage: "bookmark").badge(sites.reduce(0) { $0 + $1.count }).tag(Selection.savedAll)
                    ForEach(sites, id: \.host) { site in
                        Label(site.host, systemImage: site.pdf ? "doc.richtext" : "globe").badge(site.count).tag(Selection.savedSite(site.host))
                    }
                }
            }
            if !model.series.isEmpty {
                Section("Publications") {
                    ForEach(model.series) { s in
                        HStack(spacing: 8) {
                            Image(systemName: "book.closed").foregroundStyle(.secondary)
                            Text(s.title).lineLimit(1)
                            Spacer()
                            ProgressRing(value: s.progress)
                        }
                        .tag(Selection.publication(s.id))
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

    // MARK: Middle column

    @ViewBuilder
    private var middle: some View {
        @Bindable var model = model
        if results == nil, case let .publication(id) = selection {
            ChapterList(seriesID: id, selected: $model.currentItemID)
        } else {
            let rows = results ?? rowsForSelection
            List(rows, selection: $model.currentItemID) { item in
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
                ToolbarItem {
                    Button { Task { await model.refresh() } } label: { Label("Refresh", systemImage: "arrow.clockwise") }
                        .help("Refresh (⌘R)")
                        .disabled(model.busy)
                }
            }
            .background { keyboard(rows) }
        }
    }

    private var rowsForSelection: [ItemView] {
        switch selection {
        case .smart(.unread): model.items(.all, unreadOnly: true)
        case .smart(.starred): model.items(.starred, unreadOnly: unreadOnly)
        case .smart(.continueReading): model.items(.continueReading)
        case .savedAll: model.items(.saved, unreadOnly: unreadOnly)
        case let .savedSite(host): model.items(.saved, unreadOnly: unreadOnly).filter { $0.host == host }
        case let .feed(id): model.items(.source(id: id), unreadOnly: unreadOnly)
        case let .tag(id): model.items(.tag(id: id), unreadOnly: unreadOnly)
        default: model.items(.all, unreadOnly: unreadOnly)
        }
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
            VStack(spacing: 0) {
                ReaderPane(itemID: id, scrollTarget: $scrollTarget)
                    .id(id)
                if let seriesID = item.seriesId, let info = model.series.first(where: { $0.id == seriesID }) {
                    Divider()
                    SequenceBar(title: info.title, items: model.chapters(seriesID), current: id) { next in
                        scrollTarget = nil
                        model.currentItemID = next
                    }
                }
            }
        } else {
            ContentUnavailableView("No Article Selected", systemImage: "text.alignleft", description: Text("J and K move through the list."))
        }
    }
}

/// Middle column for a publication: progress and chapters.
struct ChapterList: View {
    @Environment(AppModel.self) private var model
    let seriesID: String
    @Binding var selected: String?

    var body: some View {
        let chapters = model.chapters(seriesID)
        let info = model.series.first { $0.id == seriesID }
        List(selection: $selected) {
            if let info {
                Section {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(info.title).font(.title3.weight(.semibold))
                        if let author = info.author { Text(author).foregroundStyle(.secondary) }
                        ProgressView(value: info.progress)
                        HStack {
                            Text("\(info.finished) of \(info.chapters) chapters read").font(.caption).foregroundStyle(.secondary)
                            Spacer()
                            if let next = info.nextItemId {
                                Button("Continue") { selected = next }
                                    .buttonStyle(.borderedProminent)
                                    .controlSize(.small)
                            }
                        }
                    }
                    .padding(.vertical, 6)
                }
            }
            Section("Chapters") {
                ForEach(Array(chapters.enumerated()), id: \.element.id) { index, chapter in
                    HStack(spacing: 10) {
                        Text("\(index + 1)").font(.caption.monospacedDigit()).foregroundStyle(.secondary).frame(width: 22, alignment: .trailing)
                        Text(Self.displayTitle(chapter.title, number: index + 1))
                            .lineLimit(2)
                            .fontWeight(chapter.finished ? .regular : .medium)
                            .foregroundStyle(chapter.finished ? .secondary : .primary)
                        Spacer()
                        if chapter.finished {
                            Image(systemName: "checkmark.circle.fill").foregroundStyle(.green).accessibilityLabel("Finished")
                        } else if chapter.inProgress {
                            ProgressRing(value: chapter.progress)
                        }
                    }
                    .padding(.vertical, 3)
                    .tag(chapter.id)
                }
            }
        }
        .listStyle(.inset)
    }

    /// Chapter titles often carry their own number; the list shows it already.
    static func displayTitle(_ title: String, number: Int) -> String {
        for prefix in ["\(number). ", "\(number) ", "Chapter \(number): ", "Chapter \(number). "] where title.hasPrefix(prefix) {
            return String(title.dropFirst(prefix.count))
        }
        return title
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
