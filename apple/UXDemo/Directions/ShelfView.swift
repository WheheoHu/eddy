import EddyKit
import SwiftUI

/// Direction B — reading first (Books + Readwise Reader): a library home
/// with shelves, and an immersive reader whose chrome gets out of the way.
struct ShelfView: View {
    @Environment(DemoStore.self) private var store
    @State private var reading: String?
    @State private var readingSeries: String?

    var body: some View {
        ZStack {
            Library(open: open)
                .opacity(reading == nil ? 1 : 0)
            if let reading {
                ImmersiveReader(itemID: reading, seriesID: readingSeries, open: open) {
                    withAnimation(.easeInOut(duration: 0.25)) { self.reading = nil }
                }
                .transition(.opacity.combined(with: .scale(scale: 0.98)))
            }
        }
    }

    private func open(_ id: String, _ series: String?) {
        withAnimation(.easeInOut(duration: 0.25)) {
            readingSeries = series
            reading = id
        }
    }
}

private struct Library: View {
    @Environment(DemoStore.self) private var store
    let open: (String, String?) -> Void
    @State private var pasted = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 34) {
                header
                let shelf = store.items(.continueReading)
                if !shelf.isEmpty {
                    Shelf(title: "继续阅读", subtitle: "\(shelf.count) 篇读到一半") {
                        ForEach(shelf) { item in
                            CoverCard(item: item).onTapGesture { open(item.id, nil) }
                        }
                    }
                }
                if !store.series.isEmpty {
                    Shelf(title: "系列", subtitle: "教程和书") {
                        ForEach(store.series) { s in
                            SeriesBook(series: s, nextTitle: s.nextItemId.flatMap { store.item($0)?.title })
                                .onTapGesture { if let next = s.nextItemId { open(next, s.id) } }
                        }
                    }
                }
                Inbox(open: open)
                let saved = store.items(.saved, limit: 12)
                if !saved.isEmpty {
                    VStack(alignment: .leading, spacing: 12) {
                        SectionTitle(title: "最近保存", subtitle: nil)
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 240), spacing: 16)], spacing: 16) {
                            ForEach(saved) { item in
                                SavedTile(item: item).onTapGesture { open(item.id, nil) }
                            }
                        }
                    }
                }
            }
            .padding(40)
            .frame(maxWidth: 1100)
            .frame(maxWidth: .infinity)
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(greeting).font(.system(size: 34, weight: .bold, design: .serif))
            HStack(spacing: 16) {
                let unread = store.sources.reduce(0) { $0 + Int($1.unread) }
                Label("\(unread) 篇未读", systemImage: "tray")
                Label("\(store.items(.continueReading).count) 篇在读", systemImage: "book")
                if let s = store.seed {
                    Label("正在准备示例内容 \(s.done)/\(s.total) · \(s.current)", systemImage: "arrow.down.circle")
                }
            }
            .font(.callout)
            .foregroundStyle(.secondary)
            HStack {
                Image(systemName: "link")
                TextField("粘贴网址，稍后在这里读", text: $pasted)
                    .textFieldStyle(.plain)
                    .onSubmit(save)
                if store.busy { ProgressView().controlSize(.small) }
                Button("保存", action: save).disabled(pasted.isEmpty)
            }
            .padding(12)
            .background(RoundedRectangle(cornerRadius: 12).fill(Color.secondary.opacity(0.08)))
            .frame(maxWidth: 560)
        }
    }

    private var greeting: String {
        switch Calendar.current.component(.hour, from: Date()) {
        case 5..<11: "早上好"
        case 11..<14: "中午好"
        case 14..<19: "下午好"
        default: "晚上好"
        }
    }

    private func save() {
        let url = pasted
        pasted = ""
        Task { await store.save(url: url) }
    }
}

private struct SectionTitle: View {
    let title: String
    let subtitle: String?

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(title).font(.title2.weight(.semibold))
            if let subtitle { Text(subtitle).foregroundStyle(.secondary) }
        }
    }
}

private struct Shelf<Content: View>: View {
    let title: String
    let subtitle: String?
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            SectionTitle(title: title, subtitle: subtitle)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(alignment: .top, spacing: 18) { content }.padding(.vertical, 6)
            }
        }
    }
}

/// Generated cover: the article's lead image, or a gradient with its title.
private struct Cover: View {
    @Environment(DemoStore.self) private var store
    let item: ItemView
    @State private var image: String?

    var body: some View {
        ZStack(alignment: .bottomLeading) {
            LinearGradient(colors: palette, startPoint: .topLeading, endPoint: .bottomTrailing)
            if let image {
                EngineImage(url: image, contentMode: .fill)
            } else {
                Text(item.title)
                    .font(.system(size: 17, weight: .bold, design: .serif))
                    .foregroundStyle(.white)
                    .padding(14)
                    .lineLimit(4)
            }
        }
        .clipped()
        .task(id: item.id) { image = await store.leadImage(item.id) }
    }

    private var palette: [Color] {
        let hue = Double(abs(item.id.hashValue) % 360) / 360
        return [Color(hue: hue, saturation: 0.5, brightness: 0.55), Color(hue: (hue + 0.08).truncatingRemainder(dividingBy: 1), saturation: 0.6, brightness: 0.35)]
    }
}

private struct CoverCard: View {
    let item: ItemView

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Cover(item: item)
                .frame(width: 220, height: 150)
                .clipShape(RoundedRectangle(cornerRadius: 12))
                .shadow(color: .black.opacity(0.15), radius: 8, y: 4)
            Text(item.title).font(.headline).lineLimit(2).frame(width: 220, alignment: .leading)
            Text(item.sourceName).font(.caption).foregroundStyle(.secondary)
            ProgressView(value: item.progress).frame(width: 220)
            Text("还剩约 \(item.minutesLeft) 分钟").font(.caption2).foregroundStyle(.secondary)
        }
        .contentShape(Rectangle())
    }
}

private struct SeriesBook: View {
    let series: SeriesView
    let nextTitle: String?

    var body: some View {
        HStack(spacing: 0) {
            Rectangle().fill(Color.black.opacity(0.25)).frame(width: 10)
            VStack(alignment: .leading, spacing: 10) {
                Text(series.title).font(.system(size: 18, weight: .bold, design: .serif)).lineLimit(3)
                Spacer()
                Text("\(series.finished) / \(series.chapters) 章").font(.caption.weight(.semibold))
                ProgressView(value: series.progress).tint(.white)
                if let nextTitle { Text("下一章：\(nextTitle)").font(.caption2).lineLimit(2) }
            }
            .padding(14)
        }
        .foregroundStyle(.white)
        .frame(width: 180, height: 240)
        .background(LinearGradient(colors: [.indigo, .purple.opacity(0.8)], startPoint: .top, endPoint: .bottom))
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .shadow(color: .black.opacity(0.2), radius: 8, y: 4)
    }
}

/// New unread items to triage: read later (star), done (mark read), open.
private struct Inbox: View {
    @Environment(DemoStore.self) private var store
    let open: (String, String?) -> Void
    @State private var hovered: String?

    var body: some View {
        let items = store.items(.all, unreadOnly: true, limit: 12)
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(title: "收件箱", subtitle: items.isEmpty ? "都处理完了" : "新内容，先分拣再读")
            ForEach(items) { item in
                HStack(spacing: 12) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(item.title).font(.body.weight(.medium)).lineLimit(1)
                        Text("\(item.sourceName) · 约 \(item.readingMinutes) 分钟").font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    if hovered == item.id {
                        Button { store.setStarred(item, true); store.setRead(item, true) } label: { Label("稍后读", systemImage: "bookmark") }
                        Button { store.setRead(item, true) } label: { Label("不读了", systemImage: "checkmark") }
                    }
                    Button("阅读") { open(item.id, nil) }.buttonStyle(.borderedProminent).controlSize(.small)
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                .background(RoundedRectangle(cornerRadius: 10).fill(hovered == item.id ? Color.secondary.opacity(0.1) : Color.secondary.opacity(0.04)))
                .onHover { hovered = $0 ? item.id : nil }
            }
        }
    }
}

private struct SavedTile: View {
    let item: ItemView

    var body: some View {
        HStack(spacing: 12) {
            Cover(item: item).frame(width: 64, height: 64).clipShape(RoundedRectangle(cornerRadius: 8))
            VStack(alignment: .leading, spacing: 4) {
                Text(item.title).font(.callout.weight(.medium)).lineLimit(2)
                Text(item.sourceName).font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color.secondary.opacity(0.05)))
        .contentShape(Rectangle())
    }
}

/// Full-window reading: a top progress line, floating controls that fade
/// while scrolling, and chapter navigation for series.
private struct ImmersiveReader: View {
    @Environment(DemoStore.self) private var store
    let itemID: String
    let seriesID: String?
    let open: (String, String?) -> Void
    let close: () -> Void
    @State private var progress = 0.0
    @State private var mode: ReadingMode = .original
    @State private var chromeVisible = true
    @State private var hideTask: Task<Void, Never>?
    @State private var showType = false
    @State private var showSummary = false
    @AppStorage("shelf.size") private var size = 18.0
    @AppStorage("shelf.serif") private var serif = true
    @AppStorage("shelf.width") private var width = 660.0

    var body: some View {
        ZStack(alignment: .top) {
            ArticleView(itemID: itemID, mode: $mode, typography: Typography(size: size, serif: serif, width: width, lineSpacing: 8), topInset: 30) { p in
                progress = p
                flashChrome(hide: true)
            }
            .id(itemID)
            GeometryReader { geo in
                Rectangle().fill(Color.accentColor).frame(width: geo.size.width * progress, height: 3)
            }
            .frame(height: 3)
            controls
                .opacity(chromeVisible ? 1 : 0)
                .animation(.easeInOut(duration: 0.3), value: chromeVisible)
        }
        .background(Color(nsColor: .textBackgroundColor))
        .onContinuousHover { phase in
            if case let .active(point) = phase, point.y < 80 { flashChrome(hide: false) }
        }
        .safeAreaInset(edge: .bottom) { if seriesID != nil { chapterBar } }
        .onExitCommand(perform: close)
    }

    private var controls: some View {
        HStack(spacing: 10) {
            Button(action: close) { Label("书架", systemImage: "chevron.left") }
            Spacer()
            Picker("", selection: $mode) { ForEach(ReadingMode.allCases) { Text($0.rawValue).tag($0) } }
                .pickerStyle(.segmented)
                .frame(width: 150)
            Button { showSummary.toggle() } label: { Image(systemName: "sparkles") }
                .popover(isPresented: $showSummary) { SummaryCard(itemID: itemID) }
            Button { showType.toggle() } label: { Image(systemName: "textformat.size") }
                .popover(isPresented: $showType) { typePanel }
        }
        .buttonStyle(.borderless)
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.regularMaterial, in: Capsule())
        .padding(.horizontal, 20)
        .padding(.top, 10)
    }

    private var typePanel: some View {
        Form {
            Slider(value: $size, in: 14...24, step: 1) { Text("字号") }
            Slider(value: $width, in: 520...860, step: 20) { Text("行宽") }
            Toggle("衬线字体", isOn: $serif)
        }
        .padding()
        .frame(width: 280)
    }

    @ViewBuilder
    private var chapterBar: some View {
        if let seriesID, let info = store.series.first(where: { $0.id == seriesID }) {
            let chapters = store.seriesChapters(seriesID)
            let index = chapters.firstIndex { $0.id == itemID } ?? 0
            HStack {
                Button { open(chapters[index - 1].id, seriesID) } label: { Label("上一章", systemImage: "chevron.left") }
                    .disabled(index == 0)
                Spacer()
                Menu("\(info.title) · 第 \(index + 1)/\(chapters.count) 章") {
                    ForEach(chapters) { c in
                        Button((c.finished ? "✓ " : "") + c.title) { open(c.id, seriesID) }
                    }
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                Spacer()
                Button { open(chapters[index + 1].id, seriesID) } label: { Label("下一章", systemImage: "chevron.right") }
                    .disabled(index + 1 >= chapters.count)
            }
            .buttonStyle(.borderless)
            .padding(.horizontal, 20)
            .padding(.vertical, 10)
            .background(.bar)
        }
    }

    private func flashChrome(hide: Bool) {
        hideTask?.cancel()
        chromeVisible = !hide || chromeVisible
        if hide {
            hideTask = Task {
                try? await Task.sleep(for: .seconds(1.5))
                if !Task.isCancelled { chromeVisible = false }
            }
        } else {
            chromeVisible = true
        }
    }
}
