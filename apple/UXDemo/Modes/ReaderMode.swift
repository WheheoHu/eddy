import EddyKit
import SwiftUI

/// Reader mode: a library for publications and saved pages, a magazine-style
/// issue for subscriptions, and one immersive reader for both.
struct ReaderMode: View {
    @Environment(DemoStore.self) private var store
    @AppStorage("reader.tab") private var tab: Tab = .library
    @State private var reading: Reading?

    enum Tab: String, CaseIterable, Identifiable {
        case library = "书架"
        case weekly = "周刊"
        var id: String { rawValue }
    }

    /// What is open, and the sequence it belongs to for previous/next.
    struct Reading: Equatable {
        var itemID: String
        var title: String
        var sequence: [String]
    }

    var body: some View {
        ZStack {
            VStack(spacing: 0) {
                Picker("", selection: $tab) { ForEach(Tab.allCases) { Text($0.rawValue).tag($0) } }
                    .pickerStyle(.segmented)
                    .frame(width: 200)
                    .padding(.vertical, 10)
                switch tab {
                case .library: Library(open: open)
                case .weekly: WeeklyIssue(open: open)
                }
            }
            .opacity(reading == nil ? 1 : 0)
            if let reading {
                ImmersiveReader(reading: reading, open: { id in self.reading?.itemID = id; store.currentItemID = id }) {
                    withAnimation(.easeInOut(duration: 0.2)) { self.reading = nil }
                    store.reload()
                }
                .transition(.opacity)
            }
        }
        .onAppear(perform: resumeFromThreePane)
    }

    private func open(_ reading: Reading) {
        store.currentItemID = reading.itemID
        withAnimation(.easeInOut(duration: 0.2)) { self.reading = reading }
    }

    /// Switching from three-pane mode keeps reading the same thing.
    private func resumeFromThreePane() {
        guard reading == nil, let id = store.currentItemID, let item = store.item(id) else { return }
        if let seriesID = item.seriesId, let info = store.series.first(where: { $0.id == seriesID }) {
            reading = Reading(itemID: id, title: info.title, sequence: store.seriesChapters(seriesID).map(\.id))
        } else {
            reading = Reading(itemID: id, title: item.sourceName, sequence: [id])
        }
    }
}

// MARK: - Library

private struct Library: View {
    @Environment(DemoStore.self) private var store
    let open: (ReaderMode.Reading) -> Void
    @State private var pasted = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 34) {
                header
                let shelf = store.items(.continueReading)
                if !shelf.isEmpty {
                    Shelf(title: "继续阅读", subtitle: "\(shelf.count) 篇读到一半") {
                        ForEach(shelf) { item in
                            CoverCard(item: item).onTapGesture { openItem(item) }
                        }
                    }
                }
                if !store.series.isEmpty {
                    Shelf(title: "出版物", subtitle: "教程、书和论文") {
                        ForEach(store.series) { s in
                            PublicationBook(series: s, nextTitle: s.nextItemId.flatMap { store.item($0)?.title })
                                .onTapGesture { if let next = s.nextItemId { openSeries(s, at: next) } }
                        }
                    }
                }
                let saved = store.items(.saved, limit: 18)
                if !saved.isEmpty {
                    VStack(alignment: .leading, spacing: 12) {
                        SectionTitle(title: "稍后读", subtitle: "保存的网页")
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 260), spacing: 16, alignment: .top)], spacing: 16) {
                            ForEach(saved) { item in
                                SavedTile(item: item).onTapGesture { openItem(item) }
                            }
                        }
                    }
                }
            }
            .padding(.horizontal, 40)
            .padding(.bottom, 40)
            .frame(maxWidth: 1100)
            .frame(maxWidth: .infinity)
        }
    }

    private func openItem(_ item: ItemView) {
        if let seriesID = item.seriesId, let s = store.series.first(where: { $0.id == seriesID }) {
            openSeries(s, at: item.id)
        } else {
            open(.init(itemID: item.id, title: item.sourceName, sequence: [item.id]))
        }
    }

    private func openSeries(_ s: SeriesView, at id: String) {
        store.currentSeriesID = s.id
        open(.init(itemID: id, title: s.title, sequence: store.seriesChapters(s.id).map(\.id)))
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(greeting).font(.system(size: 34, weight: .bold, design: .serif))
            HStack(spacing: 16) {
                Label("\(store.items(.continueReading).count) 篇在读", systemImage: "book")
                Label("\(store.series.count) 部出版物", systemImage: "books.vertical")
                if let s = store.seed {
                    Label("正在准备示例内容 \(s.done)/\(s.total) · \(s.current)", systemImage: "arrow.down.circle")
                }
            }
            .font(.callout)
            .foregroundStyle(.secondary)
            HStack {
                Image(systemName: "link")
                TextField("粘贴网址，稍后在这里读", text: $pasted).textFieldStyle(.plain).onSubmit(save)
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

struct SectionTitle: View {
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

/// The article's lead image, or a generated gradient with its title.
struct Cover: View {
    @Environment(DemoStore.self) private var store
    let item: ItemView
    var titleSize: CGFloat = 17
    @State private var image: String?

    var body: some View {
        ZStack(alignment: .bottomLeading) {
            LinearGradient(colors: palette, startPoint: .topLeading, endPoint: .bottomTrailing)
            if let image {
                Color.clear.overlay { EngineImage(url: image, contentMode: .fill) }
            } else {
                Text(item.title)
                    .font(.system(size: titleSize, weight: .bold, design: .serif))
                    .foregroundStyle(.white)
                    .padding(14)
                    .lineLimit(4)
            }
        }
        .clipped()
        .task(id: item.id) { image = await store.leadImage(item.id) }
    }

    private var palette: [Color] {
        let hue = Double(item.id.unicodeScalars.reduce(0) { ($0 &* 31 &+ Int($1.value)) % 360 }) / 360
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

private struct PublicationBook: View {
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

private struct SavedTile: View {
    let item: ItemView

    var body: some View {
        HStack(spacing: 12) {
            Cover(item: item, titleSize: 9).frame(width: 64, height: 64).clipShape(RoundedRectangle(cornerRadius: 8))
            VStack(alignment: .leading, spacing: 4) {
                Text(item.title).font(.callout.weight(.medium)).lineLimit(2)
                HStack(spacing: 6) {
                    Text(item.sourceName)
                    if item.progress > 0.02 && !item.finished { ProgressRing(value: item.progress, size: 11) }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .padding(10)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color.secondary.opacity(0.05)))
        .contentShape(Rectangle())
    }
}

// MARK: - Immersive reader

private struct ImmersiveReader: View {
    @Environment(DemoStore.self) private var store
    let reading: ReaderMode.Reading
    let open: (String) -> Void
    let close: () -> Void
    @State private var progress = ReaderProgress()
    @State private var chrome = ChromeState()
    @State private var mode: ReadingMode = .original
    @AppStorage("reader.size") private var size = 18.0
    @AppStorage("reader.serif") private var serif = true
    @AppStorage("reader.width") private var width = 660.0

    var body: some View {
        ZStack(alignment: .top) {
            ArticleView(
                itemID: reading.itemID,
                mode: $mode,
                typography: Typography(size: size, serif: serif, width: width, lineSpacing: 8),
                topInset: 34,
                progress: progress,
                chrome: chrome
            )
            .id(reading.itemID)
            ProgressLine(progress: progress)
            ReaderControls(chrome: chrome, itemID: reading.itemID, mode: $mode, size: $size, serif: $serif, width: $width, close: close)
        }
        .background(Color(nsColor: .textBackgroundColor))
        .onContinuousHover { phase in
            if case let .active(point) = phase, point.y < 90 { chrome.reveal() }
        }
        .safeAreaInset(edge: .bottom) {
            if reading.sequence.count > 1 {
                SequenceBar(title: reading.title, items: reading.sequence.compactMap { store.item($0) }, current: reading.itemID, open: open)
            }
        }
        .onExitCommand(perform: close)
    }
}

/// Floating controls; the only view that observes chrome visibility.
private struct ReaderControls: View {
    let chrome: ChromeState
    let itemID: String
    @Binding var mode: ReadingMode
    @Binding var size: Double
    @Binding var serif: Bool
    @Binding var width: Double
    let close: () -> Void
    @State private var showType = false
    @State private var showSummary = false

    var body: some View {
        HStack(spacing: 10) {
            Button(action: close) { Label("返回", systemImage: "chevron.left") }
                .keyboardShortcut(.cancelAction)
            Spacer()
            Picker("", selection: $mode) { ForEach(ReadingMode.allCases) { Text($0.rawValue).tag($0) } }
                .pickerStyle(.segmented)
                .frame(width: 150)
            Button { showSummary.toggle() } label: { Image(systemName: "sparkles") }
                .popover(isPresented: $showSummary) { SummaryCard(itemID: itemID) }
            Button { showType.toggle() } label: { Image(systemName: "textformat.size") }
                .popover(isPresented: $showType) {
                    Form {
                        Slider(value: $size, in: 14...24, step: 1) { Text("字号") }
                        Slider(value: $width, in: 520...860, step: 20) { Text("行宽") }
                        Toggle("衬线字体", isOn: $serif)
                    }
                    .padding()
                    .frame(width: 280)
                }
        }
        .buttonStyle(.borderless)
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.regularMaterial, in: Capsule())
        .padding(.horizontal, 20)
        .padding(.top, 10)
        .opacity(chrome.visible || showType || showSummary ? 1 : 0)
        .animation(.easeInOut(duration: 0.25), value: chrome.visible)
    }
}
