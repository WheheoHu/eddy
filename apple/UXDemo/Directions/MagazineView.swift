import EddyKit
import SwiftUI

/// Direction C — stream / magazine (Folo / Feedly): cards with images and AI
/// one-liners for fast triage; reading slides in over the grid.
struct MagazineView: View {
    @Environment(DemoStore.self) private var store
    @State private var filter: Chip = .all
    @State private var query = ""
    @State private var reading: String?
    @State private var dense = false

    enum Chip: Hashable {
        case all, unread, starred, saved, category(String), tag(String)
    }

    var body: some View {
        ZStack(alignment: .trailing) {
            VStack(spacing: 0) {
                chips
                Divider()
                grid
            }
            if let reading {
                Color.black.opacity(0.18).ignoresSafeArea().onTapGesture { close() }
                ReaderPanel(itemID: reading, items: items, open: { self.reading = $0 }, close: close)
                    .frame(width: 760)
                    .transition(.move(edge: .trailing))
            }
        }
        .animation(.spring(duration: 0.3), value: reading)
        .searchable(text: $query, placement: .toolbar, prompt: "搜索")
        .toolbar {
            ToolbarItem {
                Picker("", selection: $dense) {
                    Image(systemName: "square.grid.2x2").tag(false)
                    Image(systemName: "list.bullet").tag(true)
                }
                .pickerStyle(.segmented)
            }
            ToolbarItem {
                Button { Task { await store.refresh() } } label: { Image(systemName: "arrow.clockwise") }.disabled(store.busy)
            }
        }
    }

    private var items: [ItemView] {
        if !query.isEmpty { return store.search(query).map(\.item) }
        switch filter {
        case .all: return store.items(.all)
        case .unread: return store.items(.all, unreadOnly: true)
        case .starred: return store.items(.starred)
        case .saved: return store.items(.saved)
        case let .category(name): return store.items(.category(name: name))
        case let .tag(id): return store.items(.tag(id: id))
        }
    }

    private var chips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                chip("全部", .all)
                chip("未读", .unread)
                chip("星标", .starred)
                chip("保存的网页", .saved)
                ForEach(Array(Set(store.sources.compactMap(\.category))).sorted(), id: \.self) { c in chip(c, .category(c)) }
                ForEach(store.tags) { t in chip("#\(t.name)", .tag(t.id)) }
                if let s = store.seed { Text("准备示例内容 \(s.done)/\(s.total)…").font(.caption).foregroundStyle(.secondary) }
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 10)
        }
    }

    private func chip(_ title: String, _ value: Chip) -> some View {
        Button(title) { filter = value }
            .buttonStyle(.plain)
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(Capsule().fill(filter == value ? Color.accentColor : Color.secondary.opacity(0.1)))
            .foregroundStyle(filter == value ? Color.white : Color.primary)
    }

    private var grid: some View {
        ScrollView {
            if dense {
                LazyVStack(spacing: 0) {
                    ForEach(items) { item in
                        CompactCard(item: item).onTapGesture { reading = item.id }
                        Divider()
                    }
                }
                .padding(.horizontal, 20)
            } else {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 300, maximum: 420), spacing: 20, alignment: .top)], spacing: 20) {
                    ForEach(items) { item in
                        StoryCard(item: item).onTapGesture { reading = item.id }
                    }
                }
                .padding(20)
            }
        }
        .overlay {
            if items.isEmpty { ContentUnavailableView("没有内容", systemImage: "square.grid.2x2") }
        }
    }

    private func close() { reading = nil }
}

private struct StoryCard: View {
    @Environment(DemoStore.self) private var store
    let item: ItemView
    @State private var lead: String?
    @State private var hovering = false
    @State private var summarizing = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let lead {
                // The overlay keeps the image's natural size from widening the card.
                Color.clear
                    .frame(height: 170)
                    .frame(maxWidth: .infinity)
                    .overlay { EngineImage(url: lead, contentMode: .fill) }
                    .clipped()
            }
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 6) {
                    Text(String(item.sourceName.prefix(1)).uppercased())
                        .font(.caption2.bold())
                        .frame(width: 18, height: 18)
                        .background(Circle().fill(Color.accentColor.opacity(0.2)))
                    Text(item.sourceName).font(.caption.weight(.medium)).lineLimit(1)
                    Text("·").foregroundStyle(.tertiary)
                    Text(item.date, format: .relative(presentation: .named)).font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    if !item.read { Circle().fill(Color.accentColor).frame(width: 7, height: 7) }
                }
                Text(item.title).font(.title3.weight(.semibold)).lineLimit(3).foregroundStyle(item.read ? .secondary : .primary)
                if let summary = store.cachedSummary(item.id) {
                    HStack(alignment: .top, spacing: 6) {
                        Image(systemName: "sparkles").foregroundStyle(.purple)
                        Text(summary.tldr).font(.callout)
                    }
                    .padding(10)
                    .background(RoundedRectangle(cornerRadius: 8).fill(Color.purple.opacity(0.08)))
                } else if let excerpt = item.excerpt {
                    Text(excerpt).font(.callout).foregroundStyle(.secondary).lineLimit(3)
                }
                HStack(spacing: 14) {
                    Text("约 \(item.readingMinutes) 分钟").font(.caption).foregroundStyle(.secondary)
                    if item.progress > 0.02 && !item.finished { ProgressRing(value: item.progress, size: 12) }
                    Spacer()
                    if hovering {
                        if store.aiConfigured && store.cachedSummary(item.id) == nil {
                            Button { summarize() } label: { summarizing ? AnyView(ProgressView().controlSize(.mini)) : AnyView(Image(systemName: "sparkles")) }.help("一句话摘要")
                        }
                        Button { store.setRead(item, !item.read) } label: { Image(systemName: item.read ? "circle" : "checkmark.circle") }.help(item.read ? "标为未读" : "标为已读")
                    }
                    Button { store.setStarred(item, !item.starred) } label: { Image(systemName: item.starred ? "star.fill" : "star") }
                        .foregroundStyle(item.starred ? .yellow : .secondary)
                }
                .buttonStyle(.plain)
            }
            .padding(14)
        }
        .background(RoundedRectangle(cornerRadius: 14).fill(Color(nsColor: .controlBackgroundColor)))
        .clipShape(RoundedRectangle(cornerRadius: 14))
        .shadow(color: .black.opacity(hovering ? 0.14 : 0.06), radius: hovering ? 12 : 6, y: hovering ? 6 : 2)
        .scaleEffect(hovering ? 1.01 : 1)
        .animation(.easeOut(duration: 0.15), value: hovering)
        .onHover { hovering = $0 }
        .contentShape(Rectangle())
        .task(id: item.id) { lead = await store.leadImage(item.id) }
    }

    private func summarize() {
        summarizing = true
        Task {
            _ = try? await store.summary(item.id)
            summarizing = false
            store.reload()
        }
    }
}

private struct CompactCard: View {
    @Environment(DemoStore.self) private var store
    let item: ItemView

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Circle().fill(item.read ? Color.clear : Color.accentColor).frame(width: 7, height: 7).padding(.top, 7)
            VStack(alignment: .leading, spacing: 4) {
                Text(item.title).font(.body.weight(item.read ? .regular : .semibold)).lineLimit(1)
                Text(store.cachedSummary(item.id)?.tldr ?? item.excerpt ?? "").font(.callout).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            Text(item.sourceName).font(.caption).foregroundStyle(.secondary).lineLimit(1).frame(width: 140, alignment: .trailing)
        }
        .padding(.vertical, 10)
        .contentShape(Rectangle())
    }
}

/// Reader sliding over the grid; arrow keys move between stories.
private struct ReaderPanel: View {
    @Environment(DemoStore.self) private var store
    let itemID: String
    let items: [ItemView]
    let open: (String) -> Void
    let close: () -> Void
    @State private var mode: ReadingMode = .original
    @State private var showSummary = false

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Button(action: close) { Image(systemName: "xmark") }.keyboardShortcut(.cancelAction)
                Button { step(-1) } label: { Image(systemName: "chevron.up") }.keyboardShortcut(.upArrow, modifiers: [])
                Button { step(1) } label: { Image(systemName: "chevron.down") }.keyboardShortcut(.downArrow, modifiers: [])
                Spacer()
                Picker("", selection: $mode) { ForEach(ReadingMode.allCases) { Text($0.rawValue).tag($0) } }
                    .pickerStyle(.segmented).frame(width: 150)
                Button { showSummary.toggle() } label: { Image(systemName: "sparkles") }
                    .popover(isPresented: $showSummary) { SummaryCard(itemID: itemID) }
                if let url = store.item(itemID)?.url.flatMap(URL.init(string:)) {
                    Link(destination: url) { Image(systemName: "safari") }
                }
            }
            .buttonStyle(.borderless)
            .padding(12)
            Divider()
            ArticleView(itemID: itemID, mode: $mode, typography: Typography(size: 16, serif: false, width: 640))
                .id(itemID)
        }
        .background(Color(nsColor: .textBackgroundColor))
        .shadow(color: .black.opacity(0.2), radius: 20, x: -4)
    }

    private func step(_ delta: Int) {
        guard let i = items.firstIndex(where: { $0.id == itemID }) else { return }
        let next = i + delta
        if items.indices.contains(next) { open(items[next].id) }
    }
}
