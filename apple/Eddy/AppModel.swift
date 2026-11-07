import AppKit
import EddyKit
import Observation
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
    var tags: [TagView] = []
    var revision = 0
    var busy = false
    var message: String?
    var openError: String?

    /// What is being read; shared by both modes so switching keeps context.
    var currentItemID: String?

    private var leadImages: [String: String?] = [:]
    private var imageCache = NSCache<NSString, NSImage>()
    private var spoolWatcher: SpoolWatcher?
    private var refreshTimer: Timer?

    init() {
        do {
            let engine = try EddyCore.openDefault()
            try engine.setWebRenderer(renderer: WebViewRenderer())
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

    /// Saved pages grouped by site.
    var savedSites: [(host: String, count: Int)] {
        let saved = items(.saved, limit: 5000)
        let groups = Dictionary(grouping: saved, by: \.host)
        return groups.map { ($0.key, $0.value.count) }.sorted { $0.host < $1.host }
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

    func resume(_ itemID: String) -> ResumePointView? {
        try? engine?.resumePoint(itemId: itemID)
    }

    func refresh() async {
        guard let engine, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            let s = try await engine.refresh()
            if s.newItems > 0 { message = "\(s.newItems) new articles" }
        } catch EddyError.NotConfigured {
            message = "Connect FreshRSS or a sync server in Settings to load subscriptions."
        } catch {
            message = error.localizedDescription
        }
        leadImages.removeAll()
        reload()
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
