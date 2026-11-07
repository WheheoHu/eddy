import AppKit
import EddyKit
import Observation
import PDFKit
import SwiftUI

enum AppMode: String, CaseIterable, Identifiable {
    case threePane = "Three-Pane"
    case library = "Library"
    var id: String { rawValue }
    var icon: String { self == .threePane ? "rectangle.split.3x1" : "books.vertical" }
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

    /// A collection entry waiting for the user to confirm its deletion.
    var pendingDeletion: CollectionEntryView?

    var savingURL = false
    var importing = false

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
        revision += 1
    }

    func items(_ filter: ItemFilterView, unreadOnly: Bool = false, limit: UInt32 = 500) -> [ItemView] {
        _ = revision
        return (try? engine?.items(query: ItemQueryView(filter: filter, unreadOnly: unreadOnly, limit: limit, offset: 0))) ?? []
    }

    func item(_ id: String) -> ItemView? {
        _ = revision
        return try? engine?.item(id: id)
    }

    var feeds: [SourceView] { sources.filter { !$0.saved } }
    var savedSourceID: String? { sources.first { $0.saved }?.id }
    var unreadTotal: Int { feeds.reduce(0) { $0 + Int($1.unread) } }

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
        guard let engine, let data = try? await engine.image(url: url), let image = NSImage(data: Data(data.bytes)) else { return nil }
        imageCache.setObject(image, forKey: url as NSString)
        return image
    }

    // MARK: Actions

    func setStarred(_ item: ItemView, _ on: Bool) {
        Task {
            try? await engine?.setStarred(itemId: item.id, starred: on)
            reload()
        }
    }

    func setRead(_ item: ItemView, _ on: Bool) {
        Task {
            try? await engine?.setRead(itemId: item.id, read: on)
            reload()
        }
    }

    func markOpened(_ id: String) {
        guard let item = item(id), !item.read else { return }
        setRead(item, true)
    }

    func recordProgress(_ itemID: String, blockID: String, offset: Int) {
        _ = try? engine?.recordProgress(itemId: itemID, blockId: blockID, offset: UInt32(max(0, offset)), force: false)
    }

    func flushProgress(_ itemID: String, blockID: String, offset: Int) {
        _ = try? engine?.recordProgress(itemId: itemID, blockId: blockID, offset: UInt32(max(0, offset)), force: true)
        reload()
    }

    func recordPage(_ itemID: String, page: Int, of count: Int, force: Bool = false) {
        _ = try? engine?.recordPageProgress(itemId: itemID, page: UInt32(max(0, page)), pageCount: UInt32(max(0, count)), force: force)
        if force { reload() }
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
                reload()
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
        _ = try? engine?.createSeries(title: proposal.title, tocUrl: proposal.tocUrl, chapters: proposal.chapters)
        pendingSeries = nil
        reload()
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

    // MARK: AI

    var aiConfigured: Bool {
        guard let engine, engine.hasAiKey() else { return false }
        return (try? engine.aiSettings().provider) != nil
    }

    func translate(_ id: String, onBatch: @escaping @MainActor ([String: String]) -> Void) async throws -> [String: String] {
        guard let engine else { return [:] }
        let result = try await engine.translate(itemId: id, force: false, listener: BatchListener(onBatch))
        return result.segments
    }

    func cachedTranslation(_ id: String) -> [String: String] {
        (try? engine?.cachedTranslation(itemId: id))??.segments ?? [:]
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
