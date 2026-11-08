import EddyKit
import SwiftUI

/// One item, read: renderer choice, reading mode, translation, summary,
/// typography, progress and resume. Used by both app modes.
struct ReaderPane: View {
    @Environment(AppModel.self) private var model
    let itemID: String
    @Binding var scrollTarget: ScrollTarget?
    /// A page to show in a PDF (outline jumps from the contents column).
    var pageTarget: PageTarget?
    /// Opens another item (a chapter a book link points to).
    var openItem: ((String) -> Void)?
    /// Shows or hides a contents panel beside the reader; the toggle leads
    /// the reader's toolbar.
    var showContents: Binding<Bool>?
    /// The chapters or articles around this one; nil when there are none
    /// to move to (a page or PDF saved on its own).
    var neighbors: Neighbors?
    /// Open at the end of the text instead of the resume point: a previous
    /// chapter, reached by reading backwards.
    var startAtEnd = false
    /// Esc in the text: focus goes back a column.
    var onEscape: (() -> Void)?
    /// Keyboard focus belongs in the reader; its view takes it.
    var focused = false
    /// The reader's view took focus, by a click or by `focused`.
    var onFocus: (() -> Void)?

    @State private var document: DocumentView?
    @State private var pdfURL: URL?
    @State private var resumePage: PageTarget?
    @State private var lastPage: (page: Int, count: Int)?
    @State private var loading = true
    @State private var rendererID = RendererRegistry.article.id
    @State private var mode: ReadingMode = .original
    @State private var translations: [String: String] = [:]
    /// The whole text is translated; otherwise translating resumes.
    @State private var translationComplete = false
    @State private var translating = false
    /// A translation was cancelled or failed part-way; it can continue.
    @State private var translationStopped = false
    /// Batches wait here so the text is rebuilt at most every 0.6 s.
    @State private var pendingTranslations: [String: String] = [:]
    @State private var flushScheduled = false
    @State private var translationRun: Task<Void, Never>?
    /// Bumped by each start and cancel; a run finishing late leaves the
    /// banner to the newer one.
    @State private var translationGeneration = 0
    @State private var showSummary = false
    @State private var showType = false
    @State private var resumeNote: ResumePointView?
    @State private var lastPosition: (block: String, offset: Int, atEnd: Bool)?
    @State private var cache = ArticleCache()
    @State private var paneHeight: CGFloat = 600
    /// Where the next renderer slides in from.
    @State private var rendererArrival: Edge = .trailing

    @AppStorage("reader.fontSize") private var fontSize = 17.0
    @AppStorage("reader.serif") private var serif = false
    @AppStorage("reader.width") private var width = 700.0
    @AppStorage("reader.lineHeight") private var lineHeight = 1.45
    @AppStorage("reader.appearance") private var appearance: ReaderAppearance = .app
    @Environment(\.colorScheme) private var appScheme

    private var style: ReaderStyle {
        ReaderStyle(fontSize: fontSize, serif: serif, lineHeight: lineHeight, maxWidth: width)
    }

    /// The content's own scheme; the toolbar keeps the app's.
    private var scheme: ColorScheme { appearance.scheme(in: appScheme) }

    private var item: ItemView? { model.item(itemID) }

    var body: some View {
        VStack(spacing: 0) {
            if showSummary {
                SummaryPanel(itemID: itemID, maxHeight: paneHeight * 0.4) { toggleSummary(false) }
                    .transition(.move(edge: .top).combined(with: .opacity))
                Divider()
            }
            content
                .overlay(alignment: .top) { banners }
        }
        .clipped()
        .environment(\.colorScheme, scheme)
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { paneHeight = $0 }
        .toolbar { toolbar }
        .task(id: itemID) { await load() }
        .onDisappear(perform: flush)
        .onChange(of: model.readerCommand) { _, request in
            guard let command = request?.value, command.itemID == itemID else { return }
            model.readerCommand = nil
            perform(command.action)
        }
        .onChange(of: mode) { _, new in
            guard new != .original else {
                cancelTranslation()
                return
            }
            // Translations show in the reflowed text.
            if rendererID == RendererRegistry.pdf.id {
                guard document?.blocks.isEmpty == false else {
                    model.message = "This PDF has no text to translate."
                    mode = .original
                    return
                }
                rendererID = RendererRegistry.article.id
            }
            if !translationComplete, !translating { startTranslation() }
        }
    }

    @ViewBuilder
    private var content: some View {
        if let item {
            if rendererID == RendererRegistry.pdf.id {
                if let pdfURL {
                    PDFReaderView(url: pdfURL, target: pageTarget ?? resumePage, onPage: { page, count in
                        lastPage = (page, count)
                        model.recordPage(itemID, page: page, of: count)
                    }, focused: focused, onFocus: onFocus, onEscape: escape, appearance: appearance.nsAppearance)
                    .accessibilityLabel(Text("PDF: \(item.title)"))
                } else if loading {
                    ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ContentUnavailableView("PDF Unavailable", systemImage: "doc.richtext", description: Text("The file isn’t on this Mac and couldn’t be downloaded."))
                }
            } else if rendererID == RendererRegistry.web.id || (document == nil && !loading), let url = item.url.flatMap(URL.init(string:)) {
                WebPageView(url: url, gestures: gestures, onEscape: escape, focused: focused, onFocus: onFocus, appearance: appearance.nsAppearance)
                    .transition(.push(from: rendererArrival))
            } else if let document {
                let version = "\(document.contentHash)|\(mode.rawValue)|\(translations.count)|\(fontSize)|\(serif)|\(lineHeight)|\(scheme)"
                let article = cache.article(version) {
                    // Formulas are images drawn now, in the content's colors.
                    var built: BuiltArticle!
                    scheme.drawingAppearance.performAsCurrentDrawingAppearance {
                        built = ArticleBuilder.build(doc: document, item: item, translations: translations, mode: mode, style: style)
                    }
                    return built
                }
                ArticleTextView(
                    article: article,
                    version: version,
                    style: style,
                    scrollTarget: scrollTarget,
                    loadImage: { url in await model.image(url) },
                    onPosition: { block, offset, atEnd in
                        lastPosition = (block, offset, atEnd)
                        model.recordProgress(itemID, blockID: block, offset: offset, atEnd: atEnd)
                        if translating { model.setTranslationFocus(itemID, blockID: block) }
                    },
                    onBookLink: { url in
                        guard let target = model.item(forLink: url) else { return }
                        if let openItem { openItem(target) } else { model.currentItemID = target }
                    },
                    onMediaLink: { url in model.play(link: url, in: item) },
                    gestures: gestures,
                    onStep: neighbors.map { neighbors in { neighbors.go($0, .vertical) } },
                    onEscape: escape,
                    focused: focused,
                    onFocus: onFocus,
                    appearance: appearance.nsAppearance
                )
                .id(itemID)
                .transition(.push(from: rendererArrival))
                .background(Color(nsColor: .textBackgroundColor))
                .accessibilityLabel(Text("Article: \(item.title)"))
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        } else {
            ContentUnavailableView("Not Found", systemImage: "doc")
        }
    }

    @ViewBuilder
    private var banners: some View {
        VStack(spacing: 8) {
            if let item, rendererID != RendererRegistry.web.id, item.url.map(isMediaUrl(url:)) == true {
                Banner(icon: "play.rectangle", text: "This post is a video or pictures.") {
                    Button("Play") { model.play(item) }
                        .keyboardShortcut(.return, modifiers: .command)
                        .help("Show it in the media viewer (⌘Return)")
                }
            }
            if let note = resumeNote, note.fromOtherDevice, let local = note.localBlockId {
                Banner(icon: "arrow.triangle.2.circlepath", text: "Continued where you left off on another device.") {
                    Button("Back to This Mac’s Position") {
                        scrollTarget = ScrollTarget(blockID: local, offset: Int(note.localOffset))
                        resumeNote = nil
                    }
                    Button { resumeNote = nil } label: { Image(systemName: "xmark") }
                }
            }
            if translating {
                Banner(icon: "character.bubble", text: "Translating…") {
                    ProgressView().controlSize(.small)
                    Button("Cancel", action: cancelTranslation).help("Stop translating (Esc); what is translated stays")
                }
            } else if translationStopped, mode != .original {
                Banner(icon: "pause.circle", text: "Translation stopped") {
                    Button("Continue", action: startTranslation)
                    Button { translationStopped = false } label: { Image(systemName: "xmark") }
                }
            }
        }
        .padding(.top, 10)
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItemGroup {
            if let showContents {
                Button { withAnimation(.easeInOut(duration: 0.2)) { showContents.wrappedValue.toggle() } } label: {
                    Label("Contents", systemImage: "sidebar.left")
                }
                .help(showContents.wrappedValue ? "Hide contents" : "Show contents")
            }
            if let item {
                let context = RenderContext(item: item, document: document)
                let renderers = RendererRegistry.available(for: context)
                if renderers.count > 1 {
                    Picker("View", selection: $rendererID) {
                        ForEach(renderers) { Label($0.title, systemImage: $0.icon).tag($0.id) }
                    }
                    .pickerStyle(.segmented)
                    .help("How to view this item")
                }
                Picker("Language", selection: $mode) {
                    ForEach(ReadingMode.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.menu)
                .frame(width: 120)
                .help("Show a translation alongside or instead of the original")
                Button { toggleSummary(!showSummary) } label: { Label("Summary", systemImage: "sparkles") }
                    .help(showSummary ? "Hide the AI summary" : "AI summary")
                Button { showType.toggle() } label: { Label("Text Style", systemImage: "textformat.size") }
                    .popover(isPresented: $showType) { typography }
                if item.kind == .feed {
                    Button { Task { if await model.fetchFullText(itemID) { await reloadDocument() } } } label: {
                        Label("Fetch Full Text", systemImage: "doc.text.magnifyingglass")
                    }
                    .help("Download the full article from the original page")
                }
                Button { model.setStarred(item, !item.starred) } label: {
                    Label(item.starred ? "Unstar" : "Star", systemImage: item.starred ? "star.fill" : "star")
                }
                if let url = item.url.flatMap(URL.init(string:)) {
                    ShareLink(item: url)
                    Link(destination: url) { Label("Open in Browser", systemImage: "arrow.up.forward.square") }
                }
            }
        }
    }

    private var typography: some View {
        Form {
            Slider(value: $fontSize, in: 13...26, step: 1) { Text("Size") }
            Slider(value: $width, in: 520...900, step: 20) { Text("Line Width") }
            Slider(value: $lineHeight, in: 1.2...1.9, step: 0.05) { Text("Line Spacing") }
            Toggle("Serif", isOn: $serif)
            Picker("Appearance", selection: $appearance) {
                ForEach(ReaderAppearance.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
        }
        .padding()
        .frame(width: 300)
    }

    private func load() async {
        loading = true
        document = nil
        pdfURL = nil
        resumePage = nil
        resumeNote = nil
        lastPosition = nil
        lastPage = nil
        scrollTarget = nil
        rendererID = RendererRegistry.preferred(for: item).id
        let cached = model.cachedTranslation(itemID)
        translations = cached.segments
        translationComplete = cached.complete
        if item?.format == .pdf {
            if let page = model.resume(itemID)?.page { resumePage = PageTarget(page: Int(page)) }
            pdfURL = await model.contentFile(itemID)
        }
        await reloadDocument()
        model.markOpened(itemID)
        if startAtEnd, rendererID == RendererRegistry.article.id, document?.blocks.isEmpty == false {
            scrollTarget = .end
        } else if let resume = model.resume(itemID), let block = resume.blockId {
            resumeNote = resume
            if scrollTarget == nil || scrollTarget?.blockID != block {
                scrollTarget = ScrollTarget(blockID: block, offset: Int(resume.offset))
            }
        }
        if mode != .original, !translationComplete { startTranslation() }
    }

    private func reloadDocument() async {
        document = await model.document(itemID)
        loading = false
    }

    private func flush() {
        // Marked unread while open: leaving doesn't put the position back.
        guard model.item(itemID)?.read != false else { return }
        if let lastPosition { model.flushProgress(itemID, blockID: lastPosition.block, offset: lastPosition.offset, atEnd: lastPosition.atEnd) }
        if let lastPage { model.recordPage(itemID, page: lastPage.page, of: lastPage.count, force: true) }
    }

    private func toggleSummary(_ show: Bool) {
        withAnimation(.easeInOut(duration: 0.2)) { showSummary = show }
    }

    private func perform(_ action: ReaderCommand.Action) {
        switch action {
        case .summary:
            toggleSummary(true)
        case let .readingMode(new):
            mode = new
        case .toggleWeb:
            if rendererID == RendererRegistry.web.id, hasText {
                slide(to: RendererRegistry.article.id)
            } else if rendererID != RendererRegistry.web.id, hasPage {
                slide(to: RendererRegistry.web.id)
            }
        case .fetchFullText:
            Task { if await model.fetchFullText(itemID) { await reloadDocument() } }
        case let .step(direction):
            neighbors?.go(direction, .vertical)
        }
    }

    private var hasPage: Bool {
        guard let item else { return false }
        return RendererRegistry.web.supports(RenderContext(item: item, document: document))
    }

    private var hasText: Bool { document?.blocks.isEmpty == false }

    /// The web page lies to the right of the reader: it slides in from the
    /// right, and the reader comes back from the left. The edge is set first
    /// so the outgoing view leaves the same way.
    private func slide(to renderer: String) {
        rendererArrival = renderer == RendererRegistry.web.id ? .trailing : .leading
        DispatchQueue.main.async {
            withAnimation(.easeInOut(duration: 0.25)) { rendererID = renderer }
        }
    }

    /// Gestures past the reader's edges: pulls to the chapters or articles
    /// around this one; fingers moving left to the web page and right back
    /// from it. An item without a page (a book's chapter) turns chapters with
    /// a swipe instead, left for the next one. PDFs don't swipe.
    private var gestures: EdgeGestures {
        EdgeGestures(
            pullTarget: neighbors.map { neighbors in { neighbors.target($0) } },
            swipeTarget: { fingers in
                if rendererID == RendererRegistry.web.id {
                    return fingers == .right && hasText
                        ? .item(String(localized: "Reader"), release: String(localized: "Release for the reader")) : .none
                }
                guard rendererID == RendererRegistry.article.id else { return .none }
                if hasPage {
                    return fingers == .left
                        ? .item(String(localized: "Web Page"), release: String(localized: "Release for the web page")) : .none
                }
                return neighbors?.target(fingers == .left ? .next : .previous) ?? .none
            },
            commit: { kind in
                switch kind {
                case let .pull(direction):
                    neighbors?.go(direction, .vertical)
                case let .swipe(fingers):
                    if rendererID == RendererRegistry.web.id {
                        slide(to: RendererRegistry.article.id)
                    } else if hasPage {
                        slide(to: RendererRegistry.web.id)
                    } else {
                        neighbors?.go(fingers == .left ? .next : .previous, .horizontal)
                    }
                }
            }
        )
    }

    /// The block on screen: the last reported position, else where the
    /// text is about to open.
    private var readingBlock: String? {
        [lastPosition?.block, scrollTarget?.blockID].compactMap { $0 }.first { !$0.isEmpty }
    }

    /// Translates what isn't yet; batches done before are reused. A run
    /// still finishing its last batch after a cancel goes first.
    private func startTranslation() {
        guard model.aiConfigured else {
            model.message = "Add an AI provider in Settings to translate."
            mode = .original
            return
        }
        translationGeneration += 1
        let generation = translationGeneration
        let previous = translationRun
        translating = true
        translationStopped = false
        translationRun = Task {
            await previous?.value
            if let block = readingBlock { model.setTranslationFocus(itemID, blockID: block) }
            do {
                let result = try await model.translate(itemID) { queueTranslations($0) }
                flushTranslations()
                translations.merge(result.segments) { $1 }
                guard generation == translationGeneration else { return }
                translationComplete = result.complete
                translationStopped = !result.complete
            } catch {
                flushTranslations()
                guard generation == translationGeneration else { return }
                model.message = error.localizedDescription
                translationStopped = true
            }
            translating = false
        }
    }

    /// Stops at once on screen; the batch in flight still arrives and shows.
    private func cancelTranslation() {
        guard translating else { return }
        model.cancelTranslation(itemID)
        translationGeneration += 1
        translating = false
        translationStopped = true
    }

    /// Esc in the reader cancels a translation first.
    private var escape: () -> Void {
        { if translating { cancelTranslation() } else { onEscape?() } }
    }

    /// The first batch shows at once; later ones together, every 0.6 s, so
    /// the text isn't rebuilt for each.
    private func queueTranslations(_ batch: [String: String]) {
        pendingTranslations.merge(batch) { $1 }
        if translations.isEmpty {
            flushTranslations()
            return
        }
        guard !flushScheduled else { return }
        flushScheduled = true
        Task {
            try? await Task.sleep(for: .milliseconds(600))
            flushTranslations()
        }
    }

    private func flushTranslations() {
        flushScheduled = false
        guard !pendingTranslations.isEmpty else { return }
        translations.merge(pendingTranslations) { $1 }
        pendingTranslations = [:]
    }
}

/// Keeps the last built article so redraws don't rebuild it.
final class ArticleCache {
    private var version: String?
    private var article: BuiltArticle?

    func article(_ version: String, build: () -> BuiltArticle) -> BuiltArticle {
        if version == self.version, let article { return article }
        let built = build()
        self.version = version
        article = built
        return built
    }
}

/// The AI summary across the top of the reader, above the text it pushes
/// down. Summarizes as soon as it opens when nothing is cached.
struct SummaryPanel: View {
    @Environment(AppModel.self) private var model
    let itemID: String
    /// Longer summaries scroll within this height.
    let maxHeight: CGFloat
    let close: () -> Void
    @State private var summary: SummaryView?
    @State private var loading = false
    @State private var errorText: String?
    @State private var detailsHeight: CGFloat = 20

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Label("Summary", systemImage: "sparkles").font(.headline)
                Spacer()
                Button(action: close) { Image(systemName: "xmark") }
                    .buttonStyle(.borderless)
                    .help("Hide the summary")
                    .accessibilityLabel("Hide Summary")
            }
            ScrollView {
                details.onGeometryChange(for: CGFloat.self) { $0.size.height } action: { detailsHeight = $0 }
            }
            .scrollBounceBehavior(.basedOnSize)
            .frame(height: min(detailsHeight, max(maxHeight, 80)))
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary)
        .task(id: itemID) {
            summary = (try? model.engine?.cachedSummary(itemId: itemID)) ?? nil
            if summary == nil, model.aiConfigured { await load() }
        }
    }

    @ViewBuilder
    private var details: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let summary {
                Text(summary.tldr).font(.body.weight(.medium))
                ForEach(summary.points, id: \.self) { point in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text("•").foregroundStyle(.secondary)
                        Text(point)
                    }
                }
            } else if loading {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Summarizing…").foregroundStyle(.secondary)
                }
            } else if let errorText {
                HStack {
                    Text(errorText).foregroundStyle(.secondary)
                    Button("Try Again") { Task { await load() } }
                }
            } else if !model.aiConfigured {
                Text("Add an AI provider in Settings to summarize articles.").foregroundStyle(.secondary)
            } else {
                Button("Summarize") { Task { await load() } }
            }
        }
        .textSelection(.enabled)
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func load() async {
        loading = true
        errorText = nil
        defer { loading = false }
        do {
            summary = try await model.engine?.summarize(itemId: itemID, force: false)
        } catch {
            errorText = error.localizedDescription
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
        .buttonStyle(.borderless)
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(.regularMaterial, in: Capsule())
        .shadow(color: .black.opacity(0.08), radius: 8, y: 2)
        .frame(maxWidth: 560)
    }
}

/// Previous/next through an ordered list (chapters, a feed's articles).
struct SequenceBar: View {
    let title: String
    let items: [ItemView]
    let current: String
    let open: (String) -> Void

    var body: some View {
        let index = items.firstIndex { $0.id == current } ?? 0
        HStack {
            Button { open(items[index - 1].id) } label: { Label("Previous", systemImage: "chevron.left") }
                .disabled(index == 0)
                .keyboardShortcut("[", modifiers: .command)
            Spacer()
            Text("\(title) · \(index + 1) of \(items.count)").font(.callout).foregroundStyle(.secondary).lineLimit(1)
            Spacer()
            Button { open(items[index + 1].id) } label: { Label("Next", systemImage: "chevron.right") }
                .disabled(index + 1 >= items.count)
                .keyboardShortcut("]", modifiers: .command)
        }
        .buttonStyle(.borderless)
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.bar)
    }
}
