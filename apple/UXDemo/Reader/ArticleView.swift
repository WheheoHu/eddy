import EddyKit
import SwiftUI

/// The reflowed article: header, blocks, translation, progress tracking and
/// resume. Each direction wraps it in its own chrome.
struct ArticleView: View {
    @Environment(DemoStore.self) private var store
    let itemID: String
    @Binding var mode: ReadingMode
    var typography = Typography()
    var topInset: CGFloat = 0
    var showHeader = true
    var progress: ReaderProgress?
    var chrome: ChromeState?

    @State private var doc: DocumentView?
    @State private var loading = true
    @State private var translations: [String: String] = [:]
    @State private var translating = false
    @State private var errorText: String?
    @State private var topBlock: String?
    @State private var resume: ResumePointView?
    @State private var ready = false
    @State private var lastTracked: String?

    private var item: ItemView? { store.item(itemID) }

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                if showHeader, let item {
                    ArticleHeader(item: item, translatedTitle: mode == .original ? nil : translations["title"], typography: typography, origin: doc?.origin)
                        .padding(.bottom, 12)
                        .id("header")
                }
                if let doc {
                    ForEach(doc.blocks, id: \.id) { block in
                        BlockContent(block: block, typography: typography, translation: translation(for: block), mode: mode)
                            .id(block.id)
                    }
                    if doc.blocks.isEmpty {
                        ContentUnavailableView("没有正文", systemImage: "doc.text", description: Text("可以试试抓取全文或打开原网页。"))
                    }
                } else if loading {
                    ProgressView().frame(maxWidth: .infinity).padding(60)
                } else {
                    ContentUnavailableView("暂时打不开", systemImage: "wifi.slash", description: Text("内容还没下载，联网后再试。"))
                }
            }
            .scrollTargetLayout()
            .frame(maxWidth: typography.width)
            .padding(.horizontal, 36)
            .padding(.top, 28 + topInset)
            .padding(.bottom, 120)
            .frame(maxWidth: .infinity)
        }
        .scrollPosition(id: $topBlock, anchor: .top)
        .task(id: itemID) { await load() }
        .onChange(of: topBlock) { _, id in track(id) }
        .onChange(of: mode) { _, newMode in
            if newMode != .original { Task { await translateIfNeeded() } }
        }
        .onDisappear { flush() }
        .overlay(alignment: .top) { banners.padding(.top, topInset + 8) }
    }

    @ViewBuilder
    private var banners: some View {
        VStack(spacing: 8) {
            if let resume, resume.fromOtherDevice, let local = resume.localBlockId {
                Banner(icon: "iphone", text: "已跳到另一台设备上的位置") {
                    Button("回到本机位置") { topBlock = local; self.resume = nil }
                }
            }
            if translating {
                Banner(icon: "character.book.closed", text: "正在翻译…") { ProgressView().controlSize(.small) }
            }
            if let errorText {
                Banner(icon: "exclamationmark.triangle", text: errorText) {
                    Button("关闭") { self.errorText = nil }
                }
            }
        }
        .frame(maxWidth: 520)
    }

    private func translation(for block: EddyKit.BlockView) -> String? {
        guard mode != .original else { return nil }
        if case .image = block.kind { return translations["\(block.id)#caption"] }
        return translations[block.id]
    }

    private func load() async {
        ready = false
        loading = true
        doc = nil
        translations = store.cachedTranslation(itemID)?.segments ?? [:]
        doc = await store.document(itemID)
        loading = false
        if let item, !item.read { store.setRead(item, true) }
        resume = store.resume(itemID)
        if let target = resume?.blockId, doc?.blocks.contains(where: { $0.id == target }) == true {
            try? await Task.sleep(for: .milliseconds(80))
            topBlock = target
        }
        ready = true
        if mode != .original { await translateIfNeeded() }
    }

    private func track(_ id: String?) {
        guard ready, let id, id != lastTracked, let doc, let index = doc.blocks.firstIndex(where: { $0.id == id }) else { return }
        let previous = lastTracked.flatMap { last in doc.blocks.firstIndex { $0.id == last } } ?? index
        lastTracked = id
        store.recordProgress(itemID, blockID: id)
        progress?.update(Double(index + 1) / Double(max(doc.blocks.count, 1)))
        // Scrolling back up brings the controls back, like Safari and Books.
        if index < previous { chrome?.reveal() } else { chrome?.scrolled() }
    }

    private func flush() {
        guard ready, let id = topBlock else { return }
        store.recordProgress(itemID, blockID: id, force: true)
    }

    private func translateIfNeeded() async {
        guard translations.isEmpty, !translating else { return }
        guard store.aiConfigured else {
            errorText = "在设置里填写模型服务后才能翻译"
            mode = .original
            return
        }
        translating = true
        defer { translating = false }
        do {
            let result = try await store.translate(itemID) { batch in
                translations.merge(batch) { $1 }
            }
            if let result { translations = result.segments }
        } catch {
            errorText = error.localizedDescription
        }
    }
}

struct ArticleHeader: View {
    let item: ItemView
    let translatedTitle: String?
    let typography: Typography
    let origin: ContentOriginView?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(item.sourceName.uppercased())
                .font(.caption.weight(.semibold))
                .foregroundStyle(Color.accentColor)
            Text(item.title)
                .font(.system(size: typography.size * 1.85, weight: .bold, design: typography.serif ? .serif : .default))
                .textSelection(.enabled)
            if let translatedTitle {
                Text(translatedTitle)
                    .font(.system(size: typography.size * 1.3, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            HStack(spacing: 6) {
                if let author = item.author, !author.isEmpty { Text(author) }
                Text(item.date, format: .dateTime.year().month().day())
                Text("·")
                Text("约 \(item.readingMinutes) 分钟")
                if origin == .extracted { Text("· 全文") }
                if origin == .captured { Text("· 浏览器捕获") }
            }
            .font(.callout)
            .foregroundStyle(.secondary)
        }
    }
}

struct Banner<Accessory: View>: View {
    let icon: String
    let text: String
    @ViewBuilder var accessory: Accessory

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: icon)
            Text(text).lineLimit(2)
            Spacer(minLength: 8)
            accessory
        }
        .font(.callout)
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(.regularMaterial, in: Capsule())
        .shadow(color: .black.opacity(0.08), radius: 8, y: 2)
    }
}

/// Small circular progress indicator for lists.
struct ProgressRing: View {
    let value: Double
    var size: CGFloat = 16

    var body: some View {
        ZStack {
            Circle().stroke(Color.secondary.opacity(0.25), lineWidth: 2)
            Circle().trim(from: 0, to: value).stroke(Color.accentColor, style: StrokeStyle(lineWidth: 2, lineCap: .round)).rotationEffect(.degrees(-90))
        }
        .frame(width: size, height: size)
    }
}

/// Summary card used by every direction.
struct SummaryCard: View {
    @Environment(DemoStore.self) private var store
    let itemID: String
    @State private var summary: SummaryView?
    @State private var loading = false
    @State private var errorText: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("AI 摘要", systemImage: "sparkles").font(.headline)
            if let summary {
                Text(summary.tldr).font(.body.weight(.medium))
                ForEach(summary.points, id: \.self) { point in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text("•").foregroundStyle(.secondary)
                        Text(point)
                    }
                }
            } else if loading {
                ProgressView().frame(maxWidth: .infinity)
            } else if let errorText {
                Text(errorText).foregroundStyle(.secondary)
            } else if !store.aiConfigured {
                Text("在设置里填写模型服务后可以生成摘要。").foregroundStyle(.secondary)
            } else {
                Button("生成摘要") { Task { await load() } }
            }
        }
        .padding(16)
        .frame(width: 360, alignment: .leading)
        .task(id: itemID) {
            summary = store.cachedSummary(itemID)
        }
    }

    private func load() async {
        loading = true
        defer { loading = false }
        do {
            summary = try await store.summary(itemID)
        } catch {
            errorText = error.localizedDescription
        }
    }
}
