import AppKit
import EddyKit
import Observation
import SwiftUI

/// Shared model for every UX direction. Uses its own data directory and
/// keychain service so the demo never touches the real app's data.
@Observable
@MainActor
final class DemoStore {
    let engine: EddyEngine?
    var status: StatusView?
    var sources: [SourceView] = []
    var series: [SeriesView] = []
    var tags: [TagView] = []
    /// Bumped whenever item data changes, so lists re-query.
    var revision = 0
    var seed: SeedProgress?
    var message: String?
    var busy = false
    /// What is being read, shared by both modes so switching keeps context.
    var currentItemID: String?
    var currentSeriesID: String?

    private var leadImages: [String: String?] = [:]
    private var spoolWatcher: SpoolWatcher?

    static var dataDirectory: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let dir = base.appendingPathComponent("Eddy-UXDemo", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    init() {
        do {
            let engine = try EddyEngine(
                dataDir: Self.dataDirectory.path,
                platform: KeychainPlatform(service: "dev.eddy.UXDemo"),
                defaultCacheLimit: 2 * 1024 * 1024 * 1024
            )
            try engine.setWebRenderer(renderer: WebViewRenderer())
            self.engine = engine
        } catch {
            engine = nil
            message = "打开引擎失败：\(error.localizedDescription)"
        }
        reload()
        if let engine {
            spoolWatcher = SpoolWatcher(path: engine.spoolDir()) { [weak self] in
                Task { @MainActor in await self?.importCaptures() }
            }
        }
        Task { await seedIfNeeded() }
    }

    var aiConfigured: Bool {
        guard let engine, engine.hasAiKey() else { return false }
        return (try? engine.aiSettings().provider) != nil
    }

    func reload() {
        guard let engine else { return }
        status = try? engine.status()
        sources = (try? engine.sources()) ?? []
        series = (try? engine.seriesList()) ?? []
        tags = (try? engine.tags()) ?? []
        revision += 1
    }

    func items(_ filter: ItemFilterView, unreadOnly: Bool = false, limit: UInt32 = 400) -> [ItemView] {
        _ = revision
        return (try? engine?.items(query: ItemQueryView(filter: filter, unreadOnly: unreadOnly, limit: limit, offset: 0))) ?? []
    }

    func item(_ id: String) -> ItemView? {
        _ = revision
        return try? engine?.item(id: id)
    }

    func document(_ id: String) async -> DocumentView? {
        guard let engine else { return nil }
        return try? await engine.document(itemId: id)
    }

    /// First image of an item's document, for cards and covers.
    func leadImage(_ id: String) async -> String? {
        if let cached = leadImages[id] { return cached }
        let doc = await document(id)
        let src = doc?.blocks.lazy.compactMap { block -> String? in
            if case let .image(src, _, _, _, _) = block.kind { return src }
            return nil
        }.first
        leadImages[id] = src
        return src
    }

    func image(_ url: String) async -> NSImage? {
        guard let engine, let data = try? await engine.image(url: url) else { return nil }
        return NSImage(data: Data(data.bytes))
    }

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

    func recordProgress(_ itemID: String, blockID: String, force: Bool = false) {
        _ = try? engine?.recordProgress(itemId: itemID, blockId: blockID, offset: 0, force: force)
        if force { reload() }
    }

    func resume(_ itemID: String) -> ResumePointView? {
        try? engine?.resumePoint(itemId: itemID)
    }

    func translate(_ itemID: String, onBatch: @escaping @MainActor ([String: String]) -> Void) async throws -> TranslationView? {
        guard let engine else { return nil }
        let listener = BatchListener(onBatch)
        return try await engine.translate(itemId: itemID, force: false, listener: listener)
    }

    func cachedTranslation(_ itemID: String) -> TranslationView? {
        try? engine?.cachedTranslation(itemId: itemID)
    }

    func summary(_ itemID: String) async throws -> SummaryView? {
        try await engine?.summarize(itemId: itemID, force: false)
    }

    func cachedSummary(_ itemID: String) -> SummaryView? {
        _ = revision
        return try? engine?.cachedSummary(itemId: itemID)
    }

    func fetchFullText(_ itemID: String) async {
        do {
            _ = try await engine?.fetchFullText(itemId: itemID)
            leadImages[itemID] = nil
            reload()
        } catch {
            message = "抓取全文失败：\(error.localizedDescription)"
        }
    }

    func save(url: String) async {
        guard let engine, !url.isEmpty else { return }
        busy = true
        defer { busy = false }
        do {
            let item = try await engine.saveUrl(url: url)
            message = "已保存：\(item.title)"
            reload()
        } catch {
            message = "保存失败：\(error.localizedDescription)"
        }
    }

    func refresh() async {
        guard let engine else { return }
        busy = true
        defer { busy = false }
        do {
            let s = try await engine.refresh()
            message = "新文章 \(s.newItems) 篇"
        } catch EddyError.NotConfigured {
            message = "单机示例数据：在设置里连接 FreshRSS 后可以刷新订阅"
        } catch {
            message = "刷新失败：\(error.localizedDescription)"
        }
        reload()
    }

    func importCaptures() async {
        if let n = try? await engine?.importCaptures(), n > 0 {
            message = "导入了 \(n) 个浏览器捕获的页面"
            reload()
        }
    }

    func search(_ query: String) -> [SearchResultView] {
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty else { return [] }
        return (try? engine?.search(query: query, limit: 50)) ?? []
    }

    func seriesChapters(_ seriesID: String) -> [ItemView] {
        items(.series(id: seriesID), limit: 1000)
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

extension ItemView: @retroactive Identifiable {}
extension SeriesView: @retroactive Identifiable {}
extension SourceView: @retroactive Identifiable {}
extension TagView: @retroactive Identifiable {}

extension ItemView {
    /// Rough reading time: CJK at ~400 chars/min, Latin at ~1200 chars/min.
    var readingMinutes: Int {
        let perMinute = (lang == "cmn" || lang == "jpn") ? 400.0 : 1200.0
        return max(1, Int((Double(textLen) / perMinute).rounded()))
    }

    var minutesLeft: Int {
        max(1, Int(Double(readingMinutes) * (1 - progress)))
    }

    var date: Date { Date(timeIntervalSince1970: TimeInterval(publishedAt) / 1000) }

    var host: String {
        url.flatMap { URL(string: $0)?.host() }?.replacingOccurrences(of: "www.", with: "") ?? ""
    }

    var sourceName: String { sourceTitle == "保存的网页" ? host : (sourceTitle ?? host) }
}
