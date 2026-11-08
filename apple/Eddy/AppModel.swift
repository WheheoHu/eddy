import AppKit
import EddyKit
import ImageIO
import Observation
import PDFKit
import SwiftUI

enum AppMode: String, CaseIterable, Identifiable {
    case threePane = "Three-Pane"
    case library = "Library"
    case gallery = "Gallery"
    var id: String { rawValue }
    var icon: String {
        switch self {
        case .threePane: "rectangle.split.3x1"
        case .library: "books.vertical"
        case .gallery: "photo.on.rectangle.angled"
        }
    }
}

/// Something picked in the command palette, for the view that can do it.
/// The token makes picking the same thing twice a change.
struct PaletteRequest<Value: Equatable>: Equatable {
    let value: Value
    var token = UUID()
}

/// A sidebar entry of the current mode.
enum Destination: Equatable {
    case threePane(ThreePaneView.Selection)
    case library(LibraryFilter)
    /// A subscription on the library's shelf.
    case libraryFeed(sourceID: String)
    case gallery(GalleryScopeView)
}

/// What the palette asks of the reader showing `itemID`.
struct ReaderCommand: Equatable {
    enum Action: Equatable {
        case summary
        case readingMode(ReadingMode)
        /// Between the reflowed text and the web page.
        case toggleWeb
        case fetchFullText
        case step(Direction)
    }

    let itemID: String
    let action: Action
}

/// Media to show in the full-window viewer from outside the Gallery (the
/// reader): an item whose link is media, or one video of an article.
struct MediaRequest: Identifiable {
    let id = UUID()
    let entry: GalleryEntryView
}

/// A list of subscription articles, as "Mark All as Read" sees it.
struct ArticleList: Equatable {
    let filter: ItemFilterView
    let title: String
}

/// What "Mark All as Read" would mark: counted, with the moment it was
/// counted, when the dialog opens. Articles arriving after aren't marked.
struct MarkAllRequest: Identifiable {
    let id = UUID()
    let list: ArticleList
    let count: Int
    let until: Int64
}

/// App-wide state over the engine. Views read lists through it so a single
/// `revision` bump refreshes everything after a change.
@Observable
@MainActor
final class AppModel {
    let engine: EddyEngine?
    var status: StatusView?
    var sources: [SourceView] = []
    var stats: [String: SourceStatsView] = [:]
    var series: [SeriesView] = []
    /// Everything that is not a subscription article.
    var collection: [CollectionEntryView] = []
    var tags: [TagView] = []
    /// Folders shown in the Gallery as a whole.
    var galleryFolders: [String] = []
    /// Articles, pages and chapters started but not finished.
    var continueReadingCount = 0
    var revision = 0
    var busy = false
    /// Older articles added so far while FreshRSS history loads; 0 otherwise.
    var historyAdded = 0
    var message: String?
    var openError: String?

    /// What is being read; shared by both modes so switching keeps context.
    var currentItemID: String?

    /// A collection entry just imported, for the current mode to bring into view.
    var revealed: String?

    var paletteVisible = false
    /// Picked in the palette, for the current mode: a sidebar entry, an item
    /// to open, a command for the reader.
    var navigation: PaletteRequest<Destination>?
    var revealedItem: PaletteRequest<String>?
    var readerCommand: PaletteRequest<ReaderCommand>?

    /// A collection entry waiting for the user to confirm its deletion.
    var pendingDeletion: CollectionEntryView?

    /// The article list on screen, for "Mark All as Read"; nil when none is
    /// (collections, search results, a book).
    var currentList: ArticleList?
    /// "Mark All as Read" waiting for the user to confirm.
    var pendingMarkAll: MarkAllRequest?

    var savingURL = false
    var importing = false
    var mediaRequest: MediaRequest?

    private var leadImages: [String: String?] = [:]
    private var imageCache = NSCache<NSString, NSImage>()
    private var spoolWatcher: SpoolWatcher?
    private var refreshTimer: Timer?
    private var loadingHistory = false

    init() {
        do {
            let engine = try EddyCore.openDefault()
            try engine.setWebRenderer(renderer: WebViewRenderer())
            engine.setPdfTextExtractor(extractor: PDFKitTextExtractor())
            self.engine = engine
        } catch {
            engine = nil
            openError = error.localizedDescription
        }
        reload()
        guard let engine else { return }
        spoolWatcher = SpoolWatcher(path: engine.spoolDir()) { [weak self] in
            Task { @MainActor in await self?.importCaptures() }
        }
        Task { await importCaptures() }
        observeActivation()
    }

    // MARK: Lists

    func reload() {
        guard let engine else { return }
        status = try? engine.status()
        sources = (try? engine.sources()) ?? []
        stats = Dictionary(uniqueKeysWithValues: ((try? engine.sourceOverview()) ?? []).map { ($0.sourceId, $0) })
        series = (try? engine.seriesList()) ?? []
        collection = (try? engine.collection()) ?? []
        tags = (try? engine.tags()) ?? []
        galleryFolders = (try? engine.galleryFolders()) ?? []
        let started = ItemQueryView(filter: .continueReading, unreadOnly: false, keep: nil, limit: 0, offset: 0)
        continueReadingCount = Int((try? engine.itemCount(query: started)) ?? 0)
        revision += 1
    }

    private var pendingReload: Task<Void, Never>?

    /// Reading changes things in bursts (opening marks read, leaving saves
    /// the position, the end finishes it); one reload after the switch's
    /// animation covers them all instead of redrawing every list three times
    /// while it plays.
    func reloadSoon() {
        guard pendingReload == nil else { return }
        pendingReload = Task {
            try? await Task.sleep(for: .milliseconds(350))
            // Not while the fingers are moving the reader.
            while EdgeGestureTracker.anyActive {
                try? await Task.sleep(for: .milliseconds(100))
            }
            pendingReload = nil
            reload()
        }
    }

    /// `keep` stays in an unread-only list after it is read.
    func items(_ filter: ItemFilterView, unreadOnly: Bool = false, keep: String? = nil, limit: UInt32 = 500) -> [ItemView] {
        _ = revision
        return (try? engine?.items(query: ItemQueryView(filter: filter, unreadOnly: unreadOnly, keep: keep, limit: limit, offset: 0))) ?? []
    }

    func item(_ id: String) -> ItemView? {
        _ = revision
        return try? engine?.item(id: id)
    }

    var feeds: [SourceView] { sources.filter { !$0.saved } }
    var savedSourceID: String? { sources.first { $0.saved }?.id }
    var unreadTotal: Int { feeds.reduce(0) { $0 + Int($1.unread) } }

    /// The folders subscriptions are filed in, by name.
    var folders: [String] {
        Array(Set(feeds.compactMap(\.category))).sorted { $0.localizedStandardCompare($1) == .orderedAscending }
    }

    /// Subscriptions in a folder; `nil` for the ones in none.
    func feeds(inFolder folder: String?) -> [SourceView] {
        feeds.filter { $0.category == folder }
    }

    func unread(inFolder folder: String?) -> Int {
        feeds(inFolder: folder).reduce(0) { $0 + Int($1.unread) }
    }

    func entry(_ id: String) -> CollectionEntryView? {
        collection.first { $0.id == id }
    }

    /// The collection entry an item belongs to: its series, or the saved
    /// page or PDF itself. Subscription articles belong to none.
    func entry(containing item: ItemView) -> CollectionEntryView? {
        if let seriesID = item.seriesId { return collection.first { $0.seriesId == seriesID } }
        return item.kind == .feed ? nil : entry(item.id)
    }

    func entry(containing itemID: String?) -> CollectionEntryView? {
        guard let itemID, let item = item(itemID) else { return nil }
        return entry(containing: item)
    }

    func chapters(_ seriesID: String) -> [ItemView] {
        items(.series(id: seriesID), limit: 2000)
    }

    /// Articles and saved pages by title, newest first.
    func titleMatches(_ text: String, limit: UInt32 = 20) -> [ItemView] {
        items(.title(text: text), limit: limit)
    }

    func search(_ query: String) async -> [SearchResultView] {
        guard let engine, !query.trimmingCharacters(in: .whitespaces).isEmpty else { return [] }
        return (try? await engine.searchAll(query: query, limit: 80)) ?? []
    }

    // MARK: Content

    func document(_ id: String) async -> DocumentView? {
        guard let engine else { return nil }
        return try? await engine.document(itemId: id)
    }

    func leadImage(_ id: String) async -> String? {
        if let cached = leadImages[id] { return cached }
        let src = await document(id)?.blocks.lazy.compactMap { block -> String? in
            if case let .image(src, _, _, _, _) = block.kind { return src }
            return nil
        }.first
        leadImages[id] = src
        return src
    }

    func image(_ url: String) async -> NSImage? {
        if let cached = imageCache.object(forKey: url as NSString) { return cached }
        guard let engine, let data = try? await engine.image(url: url) else { return nil }
        let bytes = Data(data.bytes)
        guard let image = await Task.detached(priority: .userInitiated, operation: { Self.decoded(bytes) }).value else { return nil }
        imageCache.setObject(image, forKey: url as NSString)
        return image
    }

    /// An image with all its frames, for the media viewer to animate;
    /// `image(_:)` gives the first frame of a GIF, at a bounded size.
    func animatedImage(_ url: String) async -> NSImage? {
        let key = "animated:\(url)" as NSString
        if let cached = imageCache.object(forKey: key) { return cached }
        guard let engine, let data = try? await engine.image(url: url) else { return nil }
        let bytes = Data(data.bytes)
        guard let image = await Task.detached(priority: .userInitiated, operation: { NSImage(data: bytes) }).value else { return nil }
        imageCache.setObject(image, forKey: key)
        return image
    }

    /// Decodes off the main thread, at no more pixels than the reader shows:
    /// `NSImage(data:)` would decode a full-size photo on the main thread the
    /// first time it is drawn, in the middle of scrolling or a transition.
    /// The size in points stays the image's own, as `NSImage(data:)` gives.
    /// An animated GIF gives its first frame (`animatedImage` has them all);
    /// formats ImageIO can't read (SVG) go the old way.
    private nonisolated static func decoded(_ data: Data) -> NSImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              CGImageSourceGetCount(source) >= 1,
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? CGFloat,
              let height = properties[kCGImagePropertyPixelHeight] as? CGFloat
        else { return NSImage(data: data) }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: 2400,
        ]
        guard let cgImage = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return NSImage(data: data) }
        let orientation = properties[kCGImagePropertyOrientation] as? UInt32 ?? 1
        // Orientations 5 to 8 swap width and height.
        let size = orientation >= 5 ? CGSize(width: height, height: width) : CGSize(width: width, height: height)
        return NSImage(cgImage: cgImage, size: size)
    }

    // MARK: Actions

    func setStarred(_ item: ItemView, _ on: Bool) {
        Task {
            try? await engine?.setStarred(itemId: item.id, starred: on)
            reloadSoon()
        }
    }

    func setRead(_ item: ItemView, _ on: Bool) {
        Task {
            try? await engine?.setRead(itemId: item.id, read: on)
            reloadSoon()
        }
    }

    /// Counts what a list would mark and asks; nothing to mark says so.
    func requestMarkAllRead(_ list: ArticleList) {
        let until = Int64(Date().timeIntervalSince1970 * 1000)
        let count = Int((try? engine?.unreadFeedCount(filter: list.filter, until: until)) ?? 0)
        guard count > 0 else {
            message = String(localized: "Everything in “\(list.title)” is read.")
            return
        }
        pendingMarkAll = MarkAllRequest(list: list, count: count, until: until)
    }

    func markAllRead(_ request: MarkAllRequest) async {
        do {
            _ = try await engine?.markAllRead(filter: request.list.filter, until: request.until)
        } catch {
            message = error.localizedDescription
        }
        reload()
    }

    /// Back to unread and never started, on every device (and FreshRSS).
    func markUnread(_ itemIDs: [String]) {
        guard !itemIDs.isEmpty else { return }
        do {
            try engine?.markUnread(itemIds: itemIDs)
        } catch {
            message = error.localizedDescription
        }
        reload()
    }

    /// A book or series resets all its chapters; a page or PDF itself.
    func markUnread(entry: CollectionEntryView) {
        markUnread(entry.seriesId.map { chapters($0).map(\.id) } ?? [entry.id])
    }

    func markOpened(_ id: String) {
        guard let item = item(id), !item.read else { return }
        setRead(item, true)
    }

    /// `atEnd`: the end of the text is on screen, which finishes the item.
    /// Lists refresh when that happens so it leaves Continue Reading at once.
    func recordProgress(_ itemID: String, blockID: String, offset: Int, atEnd: Bool) {
        let finishing = atEnd && item(itemID)?.finished == false
        _ = try? engine?.recordProgress(itemId: itemID, blockId: blockID, offset: UInt32(max(0, offset)), atEnd: atEnd, force: false)
        if finishing { reloadSoon() }
    }

    func flushProgress(_ itemID: String, blockID: String, offset: Int, atEnd: Bool) {
        _ = try? engine?.recordProgress(itemId: itemID, blockId: blockID, offset: UInt32(max(0, offset)), atEnd: atEnd, force: true)
        reloadSoon()
    }

    func recordPage(_ itemID: String, page: Int, of count: Int, force: Bool = false) {
        _ = try? engine?.recordPageProgress(itemId: itemID, page: UInt32(max(0, page)), pageCount: UInt32(max(0, count)), force: force)
        if force { reloadSoon() }
    }

    func resume(_ itemID: String) -> ResumePointView? {
        try? engine?.resumePoint(itemId: itemID)
    }

    /// The original file of an imported PDF.
    func contentFile(_ itemID: String) async -> URL? {
        guard let path = try? await engine?.contentFile(itemId: itemID) else { return nil }
        return URL(fileURLWithPath: path)
    }

    /// The chapter an in-book link points to.
    func item(forLink url: URL) -> String? {
        (try? engine?.itemForLink(url: url.absoluteString)) ?? nil
    }

    /// First page of a PDF, for shelf covers.
    func pdfCover(_ itemID: String) async -> NSImage? {
        let key = "pdf:\(itemID)" as NSString
        if let cached = imageCache.object(forKey: key) { return cached }
        guard let url = await contentFile(itemID) else { return nil }
        let image = await Task.detached {
            PDFDocument(url: url)?.page(at: 0)?.thumbnail(of: CGSize(width: 300, height: 450), for: .cropBox)
        }.value
        if let image { imageCache.setObject(image, forKey: key) }
        return image
    }

    /// Imports EPUB books and PDFs, then brings the last one into view.
    func importFiles(_ urls: [URL]) async {
        guard let engine, !urls.isEmpty else { return }
        busy = true
        defer { busy = false }
        var imported: [ImportResultView] = []
        for url in urls {
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            do {
                imported.append(try await engine.importFile(path: url.path))
            } catch {
                message = "\(url.lastPathComponent): \(error.localizedDescription)"
            }
        }
        reload()
        guard let last = imported.last else { return }
        if imported.count > 1 {
            message = "Imported \(imported.count) files"
        } else {
            message = last.existing ? "“\(last.title)” is already in your library" : "Imported “\(last.title)”"
        }
        currentItemID = last.itemId
        revealed = last.seriesId ?? last.itemId
    }

    func requestDelete(_ entry: CollectionEntryView) {
        pendingDeletion = entry
    }

    /// Deletes a collection entry on every device.
    func delete(_ entry: CollectionEntryView) async {
        guard let engine else { return }
        let reading = self.entry(containing: currentItemID)?.id == entry.id
        do {
            try await engine.deleteEntry(id: entry.id)
            if reading { currentItemID = nil }
            message = "Deleted “\(entry.title)”"
        } catch {
            message = error.localizedDescription
        }
        reload()
    }

    func refresh() async {
        guard let engine, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            let s = try await engine.refresh()
            if s.newItems > 0 { message = "\(s.newItems) new articles" }
            loadHistory()
        } catch EddyError.NotConfigured {
            message = "Connect FreshRSS or a sync server in Settings to load subscriptions."
        } catch {
            message = error.localizedDescription
        }
        leadImages.removeAll()
        reload()
    }

    /// Walks older FreshRSS articles in the background, refreshing lists as
    /// each page arrives. The engine resumes where it stopped and returns at
    /// once when the history is complete.
    private func loadHistory() {
        guard let engine, !loadingHistory else { return }
        loadingHistory = true
        Task {
            let listener = HistoryProgress { [weak self] added in
                guard let self, loadingHistory else { return }
                historyAdded = added
                reloadSoon()
            }
            _ = try? await engine.backfillHistory(listener: listener)
            loadingHistory = false
            historyAdded = 0
        }
    }

    func save(url: String) async {
        guard let engine, !url.isEmpty else { return }
        busy = true
        defer { busy = false }
        do {
            let item = try await engine.saveUrl(url: url)
            message = "Saved “\(item.title)”"
            reload()
            if let proposal = try? engine.detectSeriesIn(itemId: item.id), proposal.chapters.count >= 4 {
                pendingSeries = proposal
            }
        } catch {
            message = error.localizedDescription
        }
    }

    /// A table of contents found in a page just saved, waiting for the user.
    var pendingSeries: SeriesProposalView?

    func createSeries(_ proposal: SeriesProposalView) {
        let seriesID = try? engine?.createSeries(title: proposal.title, tocUrl: proposal.tocUrl, chapters: proposal.chapters)
        pendingSeries = nil
        reload()
        if let seriesID { revealed = seriesID }
    }

    /// Offers a saved page's table of contents as a web series: from the
    /// stored copy, else from the live page.
    func convertToSeries(_ entry: CollectionEntryView) async {
        guard let engine, let url = entry.url else { return }
        busy = true
        defer { busy = false }
        var proposal = try? engine.detectSeriesIn(itemId: entry.id)
        if proposal == nil {
            do {
                proposal = try await engine.detectSeriesAt(url: url)
            } catch {
                message = error.localizedDescription
                return
            }
        }
        guard let proposal else {
            message = String(localized: "No table of contents found on this page.")
            return
        }
        // The page goes into the series only when the addresses match, and
        // the live page may have redirected.
        pendingSeries = SeriesProposalView(title: proposal.title, tocUrl: url, chapters: proposal.chapters)
    }

    func fetchFullText(_ id: String) async -> Bool {
        do {
            _ = try await engine?.fetchFullText(itemId: id)
            leadImages[id] = nil
            reload()
            return true
        } catch {
            message = error.localizedDescription
            return false
        }
    }

    func importCaptures() async {
        if let n = try? await engine?.importCaptures(), n > 0 {
            message = n == 1 ? "Imported 1 page from the browser" : "Imported \(n) pages from the browser"
            reload()
        }
    }

    // MARK: Gallery

    /// Media resolved so far, shared by the Gallery, its viewer and the
    /// reader's media pane, so an item resolved in one shows at once in
    /// the others.
    @ObservationIgnored lazy var media = GalleryMedia(model: self)

    /// Subscriptions in the Gallery, on their own or through their folder.
    var galleryFeeds: [SourceView] {
        feeds.filter { $0.gallery || $0.category.map(galleryFolders.contains) == true }
    }

    func inGallery(folder: String) -> Bool { galleryFolders.contains(folder) }

    /// Whether the item belongs in the Gallery, as the engine decides it:
    /// an article of a subscription shown there, or a saved media link.
    func inGallery(_ item: ItemView) -> Bool {
        switch item.kind {
        case .saved: item.seriesId == nil && item.url.map(isMediaUrl(url:)) == true
        case .feed: item.sourceId.map { id in galleryFeeds.contains { $0.id == id } } == true
        case .chapter: false
        }
    }

    func setGallery(folder: String, _ shown: Bool) {
        do { try engine?.setFolderGallery(name: folder, shown: shown) } catch { message = error.localizedDescription }
        reload()
    }

    /// Gives the subscription its own full-text mode (a rule at the top),
    /// or `nil` to leave it to the other rules.
    func setFullText(source: SourceView, _ mode: FullTextModeView?) {
        do { try engine?.setSourceFullTextRule(sourceId: source.id, mode: mode) } catch { message = error.localizedDescription }
        reload()
    }

    func setGallery(source: SourceView, _ shown: Bool) {
        do { try engine?.setSourceGallery(sourceId: source.id, shown: shown) } catch { message = error.localizedDescription }
        reload()
    }

    func galleryItems(_ scope: GalleryScopeView, unreadOnly: Bool, offset: Int, limit: Int = 200) -> [GalleryEntryView] {
        (try? engine?.galleryItems(scope: scope, unreadOnly: unreadOnly, limit: UInt32(limit), offset: UInt32(offset))) ?? []
    }

    /// Unread items in a part of the Gallery.
    func galleryUnread(_ scope: GalleryScopeView) -> Int {
        _ = revision
        return Int((try? engine?.galleryCount(scope: scope, unreadOnly: true)) ?? 0)
    }

    func resolveMedia(_ itemID: String, force: Bool = false) async -> MediaSetView? {
        try? await engine?.resolveMedia(itemId: itemID, force: force)
    }

    /// Opens the viewer on an item whose link is media.
    func play(_ item: ItemView) {
        mediaRequest = MediaRequest(entry: GalleryEntryView(item: item, media: nil, thumbnail: nil))
    }

    /// Opens the viewer on one video of an article (an `eddy-media:` link).
    func play(link: URL, in item: ItemView) {
        let query = URLComponents(url: link, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let value = { (name: String) in query.first { $0.name == name }?.value }
        guard let src = value("src") else { return }
        let kind: MediaTypeView = value("kind") == "embed" ? .embed : .video
        let video = MediaItemView(kind: kind, src: src, poster: value("poster"), width: nil, height: nil, duration: nil, hasAudio: true, provider: nil)
        let set = MediaSetView(status: .ready, items: [video], thumbnail: value("poster"))
        mediaRequest = MediaRequest(entry: GalleryEntryView(item: item, media: set, thumbnail: value("poster")))
    }

    /// After a thumbnail fails: drops the media the sites removed.
    func verifyMedia(_ itemID: String) async -> MediaSetView? {
        (try? await engine?.verifyMedia(itemId: itemID)) ?? nil
    }

    func prefetchMedia(_ itemIDs: [String]) {
        guard let engine, !itemIDs.isEmpty else { return }
        Task { await engine.prefetchMedia(itemIds: itemIDs) }
    }

    // MARK: AI

    var aiConfigured: Bool {
        guard let engine, engine.hasAiKey() else { return false }
        return (try? engine.aiSettings().provider) != nil
    }

    /// Returns what is translated, and whether that is all of it: a
    /// cancelled translation returns once the batch in flight is done.
    func translate(_ id: String, onBatch: @escaping @MainActor ([String: String]) -> Void) async throws -> (segments: [String: String], complete: Bool) {
        guard let engine else { return ([:], false) }
        let result = try await engine.translate(itemId: id, force: false, listener: BatchListener(onBatch))
        return (result.segments, result.complete)
    }

    func cancelTranslation(_ id: String) {
        engine?.cancelTranslation(itemId: id)
    }

    /// The block being read; a running translation continues from there.
    func setTranslationFocus(_ id: String, blockID: String) {
        engine?.setTranslationFocus(itemId: id, blockId: blockID)
    }

    /// The stored translation, or the part of one that was stopped.
    func cachedTranslation(_ id: String) -> (segments: [String: String], complete: Bool) {
        guard let cached = (try? engine?.cachedTranslation(itemId: id)) ?? nil else { return ([:], false) }
        return (cached.segments, cached.complete)
    }

    // MARK: Lifecycle

    private func observeActivation() {
        let center = NotificationCenter.default
        center.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.becameActive() }
        }
        center.addObserver(forName: NSApplication.didResignActiveNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.engine?.stopLiveSync() }
        }
        refreshTimer = Timer.scheduledTimer(withTimeInterval: 15 * 60, repeats: true) { [weak self] _ in
            Task { @MainActor in await self?.refresh() }
        }
    }

    private func becameActive() {
        guard let engine, let status else { return }
        if status.mode == .server {
            Task { await engine.startLiveSync() }
        }
        let stale = status.lastRefresh.map { Date().timeIntervalSince1970 * 1000 - Double($0) > 10 * 60 * 1000 } ?? true
        if status.mode != .unconfigured, stale { Task { await refresh() } }
    }
}

/// Forwards history progress to the main actor.
final class HistoryProgress: HistoryListener, @unchecked Sendable {
    private let handler: @MainActor (Int) -> Void

    init(_ handler: @escaping @MainActor (Int) -> Void) {
        self.handler = handler
    }

    func onProgress(added: UInt32) {
        let handler = handler
        Task { @MainActor in handler(Int(added)) }
    }
}

/// Forwards translation batches to the main actor.
final class BatchListener: TranslationListener, @unchecked Sendable {
    private let handler: @MainActor ([String: String]) -> Void

    init(_ handler: @escaping @MainActor ([String: String]) -> Void) {
        self.handler = handler
    }

    func onBatch(segments: [String: String]) {
        let handler = handler
        Task { @MainActor in handler(segments) }
    }
}
