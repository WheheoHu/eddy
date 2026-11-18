import EddyKit
import SwiftUI

/// What a swipe to the right leads back to from the reader: something
/// that comes in over it, following the fingers.
struct ReaderBack {
    /// How far the fingers move before letting go goes back.
    var commit: CGFloat
    /// The fingers have moved this far; nil when they let go short of that.
    var follow: (CGFloat?) -> Void
    var go: () -> Void
}

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
    /// The side the text comes in from: where this item lies from the one
    /// read before it.
    var arrival: ArrivalSide = .below
    /// Esc in the text: focus goes back a column.
    var onEscape: (() -> Void)?
    /// Keyboard focus belongs in the reader; its view takes it.
    var focused = false
    /// The reader's view took focus, by a click or by `focused`.
    var onFocus: (() -> Void)?
    /// What lies behind the reader, for a swipe to the right where the
    /// reader has nothing of its own that way; nil where nothing does.
    var onBack: ReaderBack?
    /// The reader's strip leads the window: it leaves room for the window's
    /// buttons.
    var stripLeads = false
    /// What comes first in the reader's strip: the way back, or the button
    /// that closes what is open.
    var stripLeading: AnyView?

    @State private var document: DocumentView?
    @State private var pdfURL: URL?
    @State private var resumePage: PageTarget?
    @State private var lastPage: (page: Int, count: Int)?
    @State private var loading = true
    @State private var switchingFullText = false
    /// The rules are fetching the full text the item opened without.
    @State private var fetchingFullText = false
    /// The summary and translation the rules opened for this item; the
    /// next one gets its own.
    @State private var summaryByRule = false
    @State private var translationByRule = false
    @State private var rendererID = RendererRegistry.article.id
    @State private var mode: ReadingMode = .original
    /// What "Translate" (⌥A) turns on: the way it was last shown.
    @AppStorage("reader.translationMode") private var lastTranslationMode: ReadingMode = .bilingual
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
    /// Translates the web page in place; the reflowed text has its own.
    @State private var webTranslator: WebTranslator?
    @State private var showSummary = false
    @State private var showType = false
    @State private var resumeNote: ResumePointView?
    @State private var lastPosition: (block: String, offset: Int, atEnd: Bool)?
    @State private var cache = ArticleCache()
    /// The item's text has yet to arrive (`ReaderMotion`).
    @State private var arriving = Arriving()
    /// Bumped when a code block's language is changed, so the text is
    /// rebuilt from the re-highlighted document.
    @State private var codeEdits = 0
    @State private var paneHeight: CGFloat = 600
    @State private var paneWidth: CGFloat = 800
    /// Where the next renderer slides in from.
    @State private var rendererArrival: Edge = .trailing
    /// Pages links led to, over the item; the last is on screen.
    @State private var links: [LinkedPage] = []
    @AppStorage("reader.openLinksInApp") private var openLinksInApp = true

    @AppStorage("reader.fontSize") private var fontSize = 17.0
    @AppStorage(ReaderFonts.latinKey) private var latinFont = ReaderFonts.systemSans
    @AppStorage(ReaderFonts.cjkKey) private var cjkFont = ReaderFonts.systemSans
    @AppStorage(ReaderFonts.spacingKey) private var cjkLatinSpacing = true
    @AppStorage(ReaderFonts.justifyKey) private var justify = false
    @AppStorage("reader.width") private var width = 700.0
    @AppStorage("reader.lineHeight") private var lineHeight = 1.45
    @AppStorage("reader.appearance") private var appearance: ReaderAppearance = .app
    @AppStorage("viewer.muted") private var muted = true
    @Environment(\.colorScheme) private var appScheme
    @Environment(\.openURL) private var openURL

    private var style: ReaderStyle {
        ReaderStyle(fontSize: fontSize, fonts: ReaderFonts(latin: latinFont, cjk: cjkFont, spacing: cjkLatinSpacing), justify: justify, lineHeight: lineHeight, maxWidth: width)
    }

    /// The content's own scheme; the toolbar keeps the app's.
    private var scheme: ColorScheme { appearance.scheme(in: appScheme) }

    private var item: ItemView? { model.item(itemID) }
    private var inGallery: Bool { item.map(model.inGallery) ?? false }
    private var showingMedia: Bool { rendererID == RendererRegistry.media.id }
    /// The web page is on screen: chosen, or standing in for text there
    /// isn't any of.
    private var showingWeb: Bool {
        guard !showingMedia, rendererID != RendererRegistry.pdf.id, item?.url != nil else { return false }
        return rendererID == RendererRegistry.web.id || (document == nil && !loading)
    }
    var body: some View {
        VStack(spacing: 0) {
            strip
            pane
        }
        .task(id: itemID) { await load() }
        .onDisappear(perform: flush)
        // Marked unread while open: where it was read to until then is
        // forgotten, so leaving doesn't put that position back. Reading on
        // from here is a position again.
        .onChange(of: model.unreadMark) { _, mark in
            guard mark?.items.contains(itemID) == true else { return }
            lastPosition = nil
            lastPage = nil
        }
        .onChange(of: model.readerCommand) { _, request in
            guard let command = request?.value, command.itemID == itemID else { return }
            model.readerCommand = nil
            perform(command.action)
        }
        .onChange(of: mode) { _, new in
            if new != .original { lastTranslationMode = new }
            if showingWeb {
                showWebTranslation(new)
                return
            }
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
        // The translation follows the view: the page's when the page shows,
        // the text's when the text does.
        .onChange(of: rendererID) { old, new in
            guard mode != .original else { return }
            if new == RendererRegistry.web.id {
                cancelTranslation()
                showWebTranslation(mode)
            } else if old == RendererRegistry.web.id {
                webTranslator?.cancel()
                if new == RendererRegistry.article.id, !translationComplete, !translating { startTranslation() }
            }
        }
        // A PDF's outline names pages; the reflowed text goes to the entry's
        // own heading, or else to where the page starts.
        .onChange(of: pageTarget) { _, target in
            guard let target, rendererID == RendererRegistry.article.id else { return }
            if let block = target.block {
                scrollTarget = ScrollTarget(blockID: block)
            } else if let pages = document?.pages, pages.indices.contains(target.page) {
                scrollTarget = ScrollTarget(blockID: pages[target.page])
            }
        }
    }

    /// The text (or page, or media) and what shows over it, in the
    /// content's own appearance; the strip above keeps the app's.
    private var pane: some View {
        VStack(spacing: 0) {
            if showSummary {
                SummaryPanel(
                    id: itemID,
                    maxHeight: paneHeight * 0.4,
                    cached: { (try? model.engine?.cachedSummary(itemId: itemID)) ?? nil },
                    summarize: { try await model.engine?.summarize(itemId: itemID, force: false) },
                    close: { toggleSummary(false) }
                )
                    .transition(.move(edge: .top).combined(with: .opacity))
                Divider()
            }
            ZStack {
                content
                    .overlay(alignment: .top) { banners }
                    .offset(x: covered ? -paneWidth : 0)
                    .allowsHitTesting(!covered)
                    .accessibilityHidden(covered)
                ForEach(Array(links.enumerated()), id: \.element.id) { index, page in
                    linkedPage(page, onTop: index == links.count - 1)
                }
            }
        }
        .clipped()
        .environment(\.colorScheme, scheme)
        .onGeometryChange(for: CGSize.self) { $0.size } action: {
            paneHeight = $0.height
            paneWidth = $0.width
        }
    }

    @ViewBuilder
    private var content: some View {
        if let item {
            if showingMedia {
                MediaPane(item: item, neighbors: neighbors, onEscape: escape, focused: focused, onFocus: onFocus)
                    .transition(.push(from: rendererArrival))
            } else if rendererID == RendererRegistry.pdf.id {
                if let pdfURL {
                    PDFReaderView(url: pdfURL, target: pageTarget ?? resumePage, onPage: { page, count in
                        lastPage = (page, count)
                        model.recordPage(itemID, page: page, of: count)
                    }, focused: focused, onFocus: onFocus, onEscape: escape, appearance: appearance.nsAppearance)
                    .accessibilityLabel(Text("PDF: \(item.title)"))
                } else if loading {
                    LateProgress()
                } else {
                    ContentUnavailableView("PDF Unavailable", systemImage: "doc.richtext", description: Text("The file isn’t on this Mac and couldn’t be downloaded."))
                }
            } else if rendererID == RendererRegistry.web.id || (document == nil && !loading), let url = item.url.flatMap(URL.init(string:)) {
                WebPageView(url: url, gestures: covered ? EdgeGestures() : gestures, onEscape: escape, focused: focused && !covered, onFocus: onFocus, appearance: appearance.nsAppearance, translator: webTranslator, onLink: openLinksInApp ? openLink : nil)
                    .transition(.push(from: rendererArrival))
            } else if let document {
                let version = "\(document.contentHash)|\(mode.rawValue)|\(translations.count)|\(fontSize)|\(latinFont)|\(cjkFont)|\(cjkLatinSpacing)|\(justify)|\(lineHeight)|\(scheme)|\(codeEdits)"
                let article = cache.article(version) {
                    // Formulas are images drawn now, in the content's colors.
                    var built: BuiltArticle!
                    scheme.drawingAppearance.performAsCurrentDrawingAppearance {
                        built = ArticleBuilder.build(
                            doc: document, item: item, translations: translations, mode: mode, style: style,
                            translatedRuns: { text in model.engine?.translatedRuns(text: text) ?? [RunView(text: text, style: 0, link: nil, math: false)] }
                        )
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
                    onWebLink: openLinksInApp ? openLink : nil,
                    gestures: covered ? EdgeGestures() : gestures,
                    onStep: neighbors.map { neighbors in { neighbors.go($0) } },
                    onEscape: escape,
                    focused: focused && !covered,
                    onFocus: onFocus,
                    appearance: appearance.nsAppearance,
                    codeLanguages: model.codeLanguages,
                    onCodeLanguage: setCodeLanguage,
                    arrival: arrival,
                    arriving: arriving
                )
                .id(itemID)
                .transition(.push(from: rendererArrival))
                .background(Color(nsColor: .textBackgroundColor))
                .accessibilityLabel(Text("Article: \(item.title)"))
            } else {
                LateProgress()
            }
        } else {
            ContentUnavailableView("Not Found", systemImage: "doc")
        }
    }

    /// A link's page, slid in from the right over the item; the ones under
    /// the top wait off to the left, still loaded.
    private func linkedPage(_ page: LinkedPage, onTop: Bool) -> some View {
        VStack(spacing: 0) {
            if page.showSummary {
                SummaryPanel(
                    id: "\(page.id.uuidString)#\(page.loads)",
                    maxHeight: paneHeight * 0.4,
                    waiting: page.loads == 0,
                    cached: { links.first { $0.id == page.id }?.summary },
                    summarize: { try await summarizeLink(page.id) },
                    close: { toggleLinkSummary(false) }
                )
                .transition(.move(edge: .top).combined(with: .opacity))
                Divider()
            }
            WebPageView(
                url: page.url,
                gestures: onTop ? linkGestures : EdgeGestures(),
                onEscape: {
                    if page.translator.translating { page.translator.cancel() } else { closeLink() }
                },
                focused: focused && onTop,
                onFocus: onFocus,
                appearance: appearance.nsAppearance,
                translator: page.translator,
                onLink: openLinksInApp ? openLink : nil,
                onPage: { url, title in
                    guard let index = links.firstIndex(where: { $0.id == page.id }) else { return }
                    if let url { links[index].current = url }
                    links[index].title = title.flatMap { $0.isEmpty ? nil : $0 }
                },
                // A new page: an open summary summarizes it afresh.
                onLoad: {
                    guard let index = links.firstIndex(where: { $0.id == page.id }) else { return }
                    links[index].summary = nil
                    links[index].loads += 1
                }
            )
            .overlay(alignment: .top) { linkBanners(page) }
        }
        .clipped()
        .offset(x: onTop ? 0 : -paneWidth)
        .allowsHitTesting(onTop)
        .accessibilityHidden(!onTop)
        .transition(.move(edge: .trailing))
    }

    private var covered: Bool { !links.isEmpty }

    @ViewBuilder
    private func linkBanners(_ page: LinkedPage) -> some View {
        let translator = page.translator
        VStack {
            if translator.translating {
                Banner(icon: "character.bubble", text: "Translating the page…") {
                    ProgressView().controlSize(.small)
                    Button("Cancel") { translator.cancel() }.help("Stop translating (Esc); what is translated stays")
                }
            } else if translator.stopped, page.mode != .original {
                Banner(icon: "pause.circle", text: "Translation stopped") {
                    Button("Continue") { translator.show(page.mode) }
                    Button { setLinkMode(.original) } label: { Image(systemName: "xmark") }
                }
            }
        }
        .padding(.top, 10)
    }

    private func openLink(_ url: URL) {
        // A summary on screen stays on for the page the link opens.
        let summarizing = links.last?.showSummary ?? showSummary
        withAnimation(.easeInOut(duration: 0.25)) { links.append(LinkedPage(url: url, model: model, showSummary: summarizing)) }
    }

    private func closeLink() {
        guard let page = links.last else { return }
        page.translator.cancel()
        withAnimation(.easeInOut(duration: 0.25)) { _ = links.removeLast() }
    }

    /// Translates the page on top as `mode` asks, once Translation is set up.
    private func setLinkMode(_ mode: ReadingMode) {
        guard let top = links.indices.last else { return }
        if mode != .original, !model.translationConfigured {
            model.message = "Add a provider in Settings → Translation to translate."
            return
        }
        if mode != .original { lastTranslationMode = mode }
        links[top].mode = mode
        links[top].translator.show(mode)
    }

    private func toggleLinkSummary(_ show: Bool) {
        guard let top = links.indices.last else { return }
        withAnimation(.easeInOut(duration: 0.2)) { links[top].showSummary = show }
    }

    /// Summarizes the page as it is now, without the translations written
    /// into it; the summary stays with the page.
    private func summarizeLink(_ id: UUID) async throws -> SummaryView? {
        guard let page = links.first(where: { $0.id == id }), let view = page.translator.webView else { return nil }
        let script = "(() => { const d = document.documentElement.cloneNode(true); d.querySelectorAll('.eddy-tr').forEach(e => e.remove()); return '<!doctype html>\\n' + d.outerHTML })()"
        guard let html = try await view.evaluateJavaScript(script) as? String else { return nil }
        let summary = try await model.engine?.summarizePage(url: page.address.absoluteString, html: html)
        if let index = links.firstIndex(where: { $0.id == id }) { links[index].summary = summary }
        return summary
    }

    /// Fingers moving right go back a page.
    private var linkGestures: EdgeGestures {
        EdgeGestures(
            swipeTarget: { fingers in
                fingers == .right ? .item(String(localized: "Back"), release: String(localized: "Release to go back")) : .none
            },
            commit: { _ in closeLink() }
        )
    }

    @ViewBuilder
    private var banners: some View {
        VStack(spacing: 8) {
            if fetchingFullText, rendererID == RendererRegistry.article.id {
                Banner(icon: "doc.text.magnifyingglass", text: "Fetching the full text…") {
                    ProgressView().controlSize(.small)
                }
            }
            if let item, rendererID == RendererRegistry.article.id, item.url.map(isMediaUrl(url:)) == true {
                Banner(icon: "play.rectangle", text: "This post is a video or pictures.") {
                    if let rawURL = item.url, isVideoUrl(url: rawURL) {
                        Button("Convert to Book…") { model.requestVideoConvert(source: rawURL, title: item.title) }
                    }
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
            if showingWeb, let webTranslator {
                if webTranslator.translating {
                    Banner(icon: "character.bubble", text: "Translating the page…") {
                        ProgressView().controlSize(.small)
                        Button("Cancel") { webTranslator.cancel() }.help("Stop translating (Esc); what is translated stays")
                    }
                } else if webTranslator.stopped, mode != .original {
                    Banner(icon: "pause.circle", text: "Translation stopped") {
                        Button("Continue") { webTranslator.show(mode) }
                        Button { mode = .original } label: { Image(systemName: "xmark") }
                    }
                }
            } else if translating {
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

    /// The reader's controls, along its top. Where they don't all fit, the
    /// ones used least go into a menu.
    ///
    /// Which set shows goes by the reader's width, not by `ViewThatFits`:
    /// that builds every candidate afresh to measure it whenever layout
    /// runs, pop-up buttons and all, and layout runs with every frame of a
    /// column sliding past.
    private var strip: some View {
        VStack(spacing: 0) {
            ColumnStrip(leads: stripLeads) {
                if let stripLeading { stripLeading }
            } trailing: {
                // The same set whether or not the strip leads the window:
                // going to the reader and back must not rebuild it.
                HStack(spacing: 4) { controls(compact: paneWidth < 620) }
                    .popover(isPresented: $showType, arrowEdge: .bottom) { typography }
            }
            Hairline()
        }
        .background(Theme.page)
    }

    @ViewBuilder
    private func controls(compact: Bool) -> some View {
        if let showContents {
            Button { withAnimation(.easeInOut(duration: 0.2)) { showContents.wrappedValue.toggle() } } label: {
                Label("Contents", systemImage: "list.bullet.indent")
            }
            .buttonStyle(FlatButtonStyle(on: showContents.wrappedValue))
            .help(showContents.wrappedValue ? "Hide contents" : "Show contents")
        }
        if let page = links.last {
            Button(action: closeLink) { Label("Back", systemImage: "chevron.left") }
                .help("Back (Esc)")
            if !compact {
                Text(page.title ?? page.address.host() ?? page.address.absoluteString)
                    .font(.system(size: 13))
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: 220)
                    .foregroundStyle(Theme.text2)
            }
            languageMenu(Binding(get: { page.mode }, set: setLinkMode))
            Button { toggleLinkSummary(!page.showSummary) } label: { Label("Summary", systemImage: "sparkles") }
                .buttonStyle(FlatButtonStyle(on: page.showSummary))
                .help(page.showSummary ? "Hide the AI summary" : "AI summary")
            Button { Task { await model.save(url: page.address.absoluteString) } } label: {
                Label("Save Page", systemImage: "square.and.arrow.down")
            }
            .disabled(model.busy)
            .help("Save this page to read later")
            openInBrowser(page.address)
        } else if let item {
            let context = RenderContext(item: item, document: document, inGallery: inGallery)
            let renderers = RendererRegistry.available(for: context)
            if renderers.count > 1 {
                SwitchBar(selection: $rendererID, options: renderers.map { .symbol($0.id, $0.icon, help: $0.title) }, segment: 34)
                    .padding(.horizontal, 4)
            }
            if showingMedia {
                // Nothing to translate, summarize or set type for.
                Button { muted.toggle() } label: {
                    Label(muted ? "Unmute" : "Mute", systemImage: muted ? "speaker.slash.fill" : "speaker.wave.2.fill")
                }
                .help(muted ? "Unmute (M)" : "Mute (M)")
                Button { model.play(item) } label: { Label("Full Screen", systemImage: "arrow.up.left.and.arrow.down.right") }
                    .help("Show it over the whole window")
                if let rawURL = item.url, isVideoUrl(url: rawURL) {
                    Button {
                        model.requestVideoConvert(source: rawURL, title: item.title)
                    } label: {
                        Label("Convert to Book…", systemImage: "book.pages")
                    }
                    .help("Convert this presentation video into an illustrated multi-chapter book")
                }
            } else if !compact {
                languageMenu($mode)
                Button { toggleSummary(!showSummary) } label: { Label("Summary", systemImage: "sparkles") }
                    .buttonStyle(FlatButtonStyle(on: showSummary))
                    .help(showSummary ? "Hide the AI summary" : "AI summary")
                Button { showType.toggle() } label: { Label("Text Style", systemImage: "textformat.size") }
                    .buttonStyle(FlatButtonStyle(on: showType))
                    .help("Text style")
                if item.kind == .feed {
                    Button { setFullText(!showingFullText) } label: {
                        Label("Full Text", systemImage: "doc.text.magnifyingglass")
                    }
                    .buttonStyle(FlatButtonStyle(on: showingFullText))
                    .disabled(switchingFullText || document == nil)
                    .help(showingFullText ? "Showing the full article from the original page; turn off for the feed’s version" : "Show the full article from the original page")
                }
            }
            Button { model.setStarred(item, !item.starred) } label: {
                Label(item.starred ? "Unstar" : "Star", systemImage: item.starred ? "star.fill" : "star")
                    .foregroundStyle(item.starred ? Theme.star : Theme.text1)
            }
            .help(item.starred ? "Unstar (S)" : "Star (S)")
            Button { model.toggleRead(item) } label: {
                Label(item.isDone ? "Mark as Unread" : "Mark as Read", systemImage: item.isDone ? "checkmark.circle.fill" : "checkmark.circle")
            }
            .help(item.isDone ? "Mark as unread and start over (⇧⌘U)" : "Mark as read and finished (⇧⌘U)")
            let exports = !showingMedia && model.canExport(item.id, fromReader: true)
            if !compact, exports {
                FlatMenu("Export", symbol: "arrow.down.doc", help: "Export as PDF or EPUB") { exportItems(item) }
            }
            if let url = item.url.flatMap(URL.init(string:)) {
                openInBrowser(url)
            }
            if compact, !showingMedia {
                FlatMenu("More", symbol: "ellipsis", help: "Translation, summary, text style and more") {
                    Picker("Language", selection: $mode) {
                        ForEach(ReadingMode.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.inline)
                    Divider()
                    Toggle("Summary", isOn: Binding(get: { showSummary }, set: toggleSummary))
                    Button("Text Style…") { showType = true }
                    if item.kind == .feed {
                        Toggle("Full Text", isOn: Binding(get: { showingFullText }, set: setFullText))
                            .disabled(switchingFullText || document == nil)
                    }
                    if exports {
                        Divider()
                        exportItems(item)
                    }
                }
            }
        }
    }

    /// Original, both, or the translation alone.
    private func languageMenu(_ selection: Binding<ReadingMode>) -> some View {
        FlatMenu("Language", symbol: "translate", help: "Show a translation alongside or instead of the original", on: selection.wrappedValue != .original) {
            Picker("Language", selection: selection) {
                ForEach(ReadingMode.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.inline)
        }
    }

    @ViewBuilder
    private func exportItems(_ item: ItemView) -> some View {
        Button("Export as PDF…") {
            model.beginExport(targetID: item.id, format: .pdf, fromReader: true, preferredMode: mode)
        }
        Button("Export as EPUB…") {
            model.beginExport(targetID: item.id, format: .epub, fromReader: true, preferredMode: mode)
        }
    }

    /// Opens the address in the default browser, under that browser's mark.
    /// A button, not a `Link`: a link's label takes the accent color, and
    /// the mark is drawn like the symbols beside it.
    private func openInBrowser(_ url: URL) -> some View {
        Button { openURL(url) } label: {
            Label { Text("Open in Browser") } icon: { BrowserIcon() }
        }
        .help("Open in Browser")
    }

    private var typography: some View {
        Form {
            Slider(value: $fontSize, in: 13...26, step: 1) { Text("Size") }
            Slider(value: $width, in: 520...900, step: 20) { Text("Line Width") }
            Slider(value: $lineHeight, in: 1.2...1.9, step: 0.05) { Text("Line Spacing") }
            ReaderFontPicker(title: "Latin Font", selection: $latinFont, families: ReaderFonts.latinFamilies)
            ReaderFontPicker(title: "Chinese Font", selection: $cjkFont, families: ReaderFonts.cjkFamilies)
            Toggle("Space Chinese and Latin", isOn: $cjkLatinSpacing)
            Toggle("Justify Text", isOn: $justify)
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
        if summaryByRule { showSummary = false }
        if translationByRule { mode = .original }
        summaryByRule = false
        translationByRule = false
        links.forEach { $0.translator.cancel() }
        links = []
        document = nil
        pdfURL = nil
        resumePage = nil
        resumeNote = nil
        lastPosition = nil
        lastPage = nil
        scrollTarget = nil
        rendererID = RendererRegistry.preferred(for: item, inGallery: inGallery).id
        webTranslator?.cancel()
        webTranslator = WebTranslator(model: model, itemID: itemID)
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
        await applyRules()
    }

    /// The full-text and AI rules for the item, whenever it arrived: its
    /// full text comes in under what is shown (unless the version was
    /// switched meanwhile), then the summary and translation they ask for
    /// open, made from it.
    private func applyRules() async {
        let shown = document?.contentHash
        fetchingFullText = model.fullTextOnOpen(itemID)
        let applied = await model.applyRules(itemID)
        fetchingFullText = false
        guard let applied, !Task.isCancelled else { return }
        if let full = applied.document, !switchingFullText, document?.contentHash == shown {
            document = full
            if mode != .original {
                cancelTranslation()
                let cached = model.cachedTranslation(itemID)
                translations = cached.segments
                translationComplete = cached.complete
                if !translationComplete { startTranslation() }
            }
        } else if let error = applied.fullTextError {
            model.message = error
        }
        if applied.summarize, !showSummary {
            toggleSummary(true)
            summaryByRule = true
        }
        if applied.translate, mode == .original, !showingMedia, rendererID != RendererRegistry.pdf.id {
            translationByRule = true
            mode = lastTranslationMode
        }
    }

    private func reloadDocument() async {
        document = await model.document(itemID)
        loading = false
    }

    /// Highlights a code block as the language picked from its label, from
    /// now on; the text keeps its place.
    private func setCodeLanguage(_ mark: CodeBlockMark, _ language: String?) {
        Task {
            do {
                try model.engine?.setCodeLanguage(itemId: itemID, blockId: mark.blockID, language: language)
            } catch {
                model.report(error)
                return
            }
            guard let fresh = await model.document(itemID), !Task.isCancelled else { return }
            document = fresh
            codeEdits += 1
        }
    }

    private func flush() {
        webTranslator?.cancel()
        links.forEach { $0.translator.cancel() }
        if let lastPosition { model.flushProgress(itemID, blockID: lastPosition.block, offset: lastPosition.offset, atEnd: lastPosition.atEnd) }
        if let lastPage { model.recordPage(itemID, page: lastPage.page, of: lastPage.count, force: true) }
    }

    private func toggleSummary(_ show: Bool) {
        withAnimation(.easeInOut(duration: 0.2)) { showSummary = show }
    }

    private func perform(_ action: ReaderCommand.Action) {
        if let page = links.last {
            switch action {
            case .summary: toggleLinkSummary(true)
            case let .readingMode(new): setLinkMode(new)
            case .toggleTranslation: setLinkMode(page.mode == .original ? lastTranslationMode : .original)
            case .toggleWeb, .toggleFullText, .step, .export: break
            }
            return
        }
        switch action {
        case .summary:
            toggleSummary(true)
        case let .readingMode(new):
            mode = new
        case .toggleTranslation:
            guard !showingMedia else { return }
            mode = mode == .original ? lastTranslationMode : .original
        case .toggleWeb:
            if rendererID == RendererRegistry.web.id, hasText {
                slide(to: RendererRegistry.article.id)
            } else if rendererID != RendererRegistry.web.id, hasPage {
                slide(to: RendererRegistry.web.id)
            }
        case .toggleFullText:
            setFullText(!showingFullText)
        case let .step(direction):
            neighbors?.go(direction)
        case let .export(format):
            guard !showingMedia else { return }
            model.beginExport(targetID: itemID, format: format, fromReader: true, preferredMode: mode)
        }
    }

    private var hasPage: Bool {
        guard let item else { return false }
        return RendererRegistry.web.supports(RenderContext(item: item, document: document))
    }

    private var hasText: Bool { document?.blocks.isEmpty == false }

    private var showingFullText: Bool { document?.origin == .extracted }

    /// The article stays on the version picked here until switched again.
    private func setFullText(_ on: Bool) {
        guard on != showingFullText, !switchingFullText else { return }
        switchingFullText = true
        Task {
            if await model.setFullText(itemID, on) { await reloadDocument() }
            switchingFullText = false
        }
    }

    /// The web page lies to the right of the reader: it slides in from the
    /// right, and the reader comes back from the left. The edge is set first
    /// so the outgoing view leaves the same way.
    private func slide(to renderer: String) {
        // From here on text comes in with the slide, not as an arrival.
        _ = arriving.claim()
        rendererArrival = renderer == RendererRegistry.web.id ? .trailing : .leading
        DispatchQueue.main.async {
            withAnimation(.easeInOut(duration: 0.25)) { rendererID = renderer }
        }
    }

    /// Gestures past the reader's edges: pulls to the chapters or articles
    /// around this one; fingers moving left to the web page and right back
    /// from it. An item without a page (a book's chapter) turns chapters with
    /// a swipe instead, left for the next one. PDFs don't swipe. Where
    /// something lies behind the reader (`onBack`), fingers moving right go
    /// there once the reader has nothing of its own that way: from the web
    /// page the first swipe is still back to the text.
    private var gestures: EdgeGestures {
        EdgeGestures(
            pullTarget: neighbors.map { neighbors in { neighbors.target($0) } },
            swipeTarget: { fingers in
                let back: EdgeTarget = onBack.map { .slide(commit: $0.commit) } ?? .none
                if rendererID == RendererRegistry.web.id {
                    guard fingers == .right else { return .none }
                    return hasText
                        ? .item(String(localized: "Reader"), release: String(localized: "Release for the reader")) : back
                }
                guard rendererID == RendererRegistry.article.id else { return .none }
                if fingers == .right, onBack != nil { return back }
                if hasPage {
                    return fingers == .left
                        ? .item(String(localized: "Web Page"), release: String(localized: "Release for the web page")) : .none
                }
                return neighbors?.target(fingers == .left ? .next : .previous) ?? .none
            },
            slide: onBack?.follow,
            commit: { kind in
                switch kind {
                case let .pull(direction):
                    neighbors?.go(direction)
                case let .swipe(fingers):
                    if rendererID == RendererRegistry.web.id {
                        if hasText { slide(to: RendererRegistry.article.id) } else { onBack?.go() }
                    } else if fingers == .right, let onBack {
                        onBack.go()
                    } else if hasPage {
                        slide(to: RendererRegistry.web.id)
                    } else {
                        neighbors?.go(fingers == .left ? .next : .previous)
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
        guard model.translationConfigured else {
            model.message = "Add a provider in Settings → Translation to translate."
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
                model.report(error)
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
        {
            if translating {
                cancelTranslation()
            } else if showingWeb, webTranslator?.translating == true {
                webTranslator?.cancel()
            } else {
                onEscape?()
            }
        }
    }

    /// Translates the web page as `mode` asks, once Translation is set up.
    private func showWebTranslation(_ mode: ReadingMode) {
        if mode != .original, !model.translationConfigured {
            model.message = "Add a provider in Settings → Translation to translate."
            self.mode = .original
            return
        }
        webTranslator?.show(mode)
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

/// A page a link led to, shown over the item. Its translation and summary
/// are its own, and go with it.
struct LinkedPage: Identifiable {
    let id = UUID()
    /// The link's address; the view loads it once.
    let url: URL
    /// Where the page is now, after redirects.
    var current: URL?
    var title: String?
    let translator: WebTranslator
    var mode: ReadingMode = .original
    var showSummary: Bool
    /// Pages finished loading here; the summary is of the latest.
    var loads = 0
    var summary: SummaryView?

    @MainActor
    init(url: URL, model: AppModel, showSummary: Bool) {
        self.url = url
        self.showSummary = showSummary
        translator = WebTranslator(model: model, key: "link:\(id.uuidString)")
    }

    var address: URL { current ?? url }
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
    /// What is summarized; another one starts over.
    let id: String
    /// Longer summaries scroll within this height.
    let maxHeight: CGFloat
    /// What is to be summarized isn't there yet (a page still loading).
    var waiting = false
    let cached: () -> SummaryView?
    let summarize: () async throws -> SummaryView?
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
        .task(id: id) {
            summary = cached()
            errorText = nil
            if summary == nil, !waiting, model.summaryConfigured { await load() }
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
            } else if loading || (waiting && model.summaryConfigured) {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(loading ? "Summarizing…" : "Waiting for the page to load…").foregroundStyle(.secondary)
                }
            } else if let errorText {
                HStack {
                    Text(errorText).foregroundStyle(.secondary)
                    Button("Try Again") { Task { await load() } }
                }
            } else if !model.summaryConfigured {
                Text("Add a provider in Settings → Summaries to summarize articles.").foregroundStyle(.secondary)
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
            summary = try await summarize()
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
