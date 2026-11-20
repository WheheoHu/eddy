import AppKit
import EddyKit
import ImageIO
import Observation
import os
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
/// A site's icon, and whether it is a dark or a light drawing on nothing:
/// such an icon is made for a browser's tab and vanishes on a background
/// of its own tone, so it is set on a plate of the other.
struct SiteIcon: @unchecked Sendable {
    enum Ink {
        case dark, light, mixed
    }

    let image: NSImage
    let ink: Ink

    /// Read from a 16 by 16 copy: icons with see-through parts whose solid
    /// pixels are all nearly black, or all nearly white.
    static func ink(of image: CGImage?) -> Ink {
        let side = 16
        var pixels = [UInt8](repeating: 0, count: side * side * 4)
        guard let image,
              let context = CGContext(data: &pixels, width: side, height: side, bitsPerComponent: 8, bytesPerRow: side * 4,
                                      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return .mixed }
        context.draw(image, in: CGRect(x: 0, y: 0, width: side, height: side))
        var solid = 0.0, dark = 0.0, light = 0.0
        for index in stride(from: 0, to: pixels.count, by: 4) {
            let alpha = Double(pixels[index + 3]) / 255
            guard alpha > 0.5 else { continue }
            // Premultiplied: back to the colour itself.
            let luminance = (0.2126 * Double(pixels[index]) + 0.7152 * Double(pixels[index + 1]) + 0.0722 * Double(pixels[index + 2])) / 255 / alpha
            solid += 1
            if luminance < 0.25 { dark += 1 } else if luminance > 0.85 { light += 1 }
        }
        // A square full of colour is its own background.
        guard solid > 0, solid < Double(side * side) * 0.9 else { return .mixed }
        if dark / solid > 0.9 { return .dark }
        if light / solid > 0.9 { return .light }
        return .mixed
    }
}

/// What the bottom banner says, and what it offers to do about it.
struct Notice: Equatable {
    enum Kind: Equatable {
        case info
        /// Something failed; the text can be copied.
        case error
        /// A page was saved; "Show" brings it into view.
        case saved(itemID: String)
        /// A feed was subscribed to; "Show" lists it.
        case subscribed(sourceID: String)
        /// A file was exported; "Show in Finder" reveals it.
        case exported(path: String)
    }

    let text: String
    var kind: Kind = .info
    /// The same words twice are two notices, each with its own timeout.
    var token = UUID()
}

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
        /// Translation on (the way it was last shown) or back to the original.
        case toggleTranslation
        /// Between the reflowed text and the web page.
        case toggleWeb
        /// Between the full text and the feed's content.
        case toggleFullText
        case step(Direction)
        case export(ExportFormatView)
        /// The contents beside the text, or over it where there is no room
        /// beside: shown or hidden the way the reader's own button does.
        case toggleContents
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

/// A video URL or local video file ready for the Video-to-Book preflight sheet.
struct VideoConvertRequest: Identifiable, Equatable {
    let id = UUID()
    var source: String
    var suggestedTitle: String?
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
    /// The banner along the bottom of the window.
    var notice: Notice?
    /// A plain notice, saying what happened.
    var message: String? {
        get { notice?.text }
        set { notice = newValue.map { Notice(text: $0) } }
    }

    /// Shows what went wrong; the banner can copy it.
    func report(_ error: Error, about subject: String? = nil) {
        let text = subject.map { "\($0): \(error.localizedDescription)" } ?? error.localizedDescription
        notice = Notice(text: text, kind: .error)
    }
    var openError: String?

    /// What is being read; shared by both modes so switching keeps context.
    var currentItemID: String?

    /// A collection entry just imported, for the current mode to bring into view.
    var revealed: String?

    var paletteVisible = false

    /// A sidebar is in over the content, in a window too narrow to have it
    /// beside. Kept here so that switching mode, from the switch at the
    /// sidebar's foot, leaves the sidebar in.
    var sidebarPulled = false

    /// Settings are a panel over the window.
    var settingsVisible = false

    /// Something lies over the window's content and takes its keys.
    var overlayVisible: Bool { paletteVisible || settingsVisible }

    /// Opens Settings, on a page if one is named.
    func showSettings(_ tab: SettingsTab? = nil) {
        if let tab { UserDefaults.standard.set(tab.rawValue, forKey: "settings.tab") }
        paletteVisible = false
        mediaRequest = nil
        settingsVisible = true
    }

    /// Translation on or off for the item being read (⌥A).
    func toggleTranslation() {
        guard let id = currentItemID else { return }
        readerCommand = PaletteRequest(value: ReaderCommand(itemID: id, action: .toggleTranslation))
    }

    /// What Three-Pane's sidebar has chosen. Kept here, not in the view: it
    /// outlives a switch of mode, and the Library sets it. The sidebars of
    /// the two modes follow each other, each choice made in one setting
    /// what the other shows next (`chose`).
    var threePaneSelection: ThreePaneView.Selection? = .smart(.all)

    /// Chosen in Three-Pane's sidebar: the Library's shelf follows, where
    /// it has one for it (not for unread, starred or a tag).
    func chose(_ selection: ThreePaneView.Selection?) {
        threePaneSelection = selection
        if let selection, let filter = LibraryFilter(selection, feeds: feeds) {
            UserDefaults.standard.set(filter.rawValue, forKey: LibraryFilter.key)
        }
    }

    /// Chosen in the Library's sidebar: Three-Pane's follows.
    func chose(_ filter: LibraryFilter) {
        threePaneSelection = ThreePaneView.Selection(filter, folders: folders)
    }

    /// A subscription opened from the Library's shelf: Three-Pane lists it.
    func opened(feed sourceID: String) {
        threePaneSelection = .feed(sourceID)
    }

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
    /// The Add Feed sheet, while it is up.
    var addingFeed: AddFeedRequest?
    var importing = false
    var mediaRequest: MediaRequest?
    var pendingVideoConvert: VideoConvertRequest?
    var activeExport: ActiveExportState?
    var videoJobs: [VideoJobView] = []
    var serverVideoCapable = false
    var showingVideoJobs = false
    private var completedJobIDs: Set<String> = []
    private var videoJobsRefreshTask: Task<Void, Never>?

    private var leadImages: [String: String?] = [:]
    /// Decoded pictures, up to about 768 MB of pixels.
    private var imageCache: NSCache<NSString, NSImage> = {
        let cache = NSCache<NSString, NSImage>()
        cache.totalCostLimit = 768 << 20
        return cache
    }()
    private var spoolWatcher: SpoolWatcher?
    private var refreshTimer: Timer?
    private var loadingHistory = false

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
        engine.setVideoJobListener(listener: VideoJobBridge { [weak self] in
            self?.scheduleVideoJobsRefresh()
        })
        // Changes that arrive from the server while the app is open.
        engine.setSyncListener(listener: SyncBridge(change: { [weak self] in
            self?.reloadSoon()
        }, progress: { [weak self] in
            self?.syncProgressed()
        }))
        spoolWatcher = SpoolWatcher(path: engine.spoolDir()) { [weak self] in
            Task { @MainActor in await self?.importCaptures() }
        }
        Task {
            await importCaptures()
            await refreshVideoJobs(initial: true)
            await refreshServerCapability()
        }
        observeActivation()
    }

    // MARK: Lists

    /// Reads everything the lists are drawn from and shows it, before
    /// returning: for a change the user just made and is looking at. It
    /// holds the main thread for as long as the library takes to count
    /// (a fifth of a second for a hundred thousand articles), so what can
    /// wait a moment goes through `reloadSoon`.
    func reload() {
        itemsCache.removeAll()
        guard let engine else { return }
        reloadTicket += 1
        show(Self.overview(engine))
    }

    /// What `reload` reads, in one piece so it can be read off the main thread.
    private struct Overview: Sendable {
        var status: StatusView?
        var sources: [SourceView]
        var stats: [String: SourceStatsView]
        var series: [SeriesView]
        var collection: [CollectionEntryView]
        var tags: [TagView]
        var galleryFolders: [String]
        var continueReadingCount: Int
    }

    /// The core reads it all on a connection of its own, so the calls the
    /// interface goes on making meanwhile don't wait for it.
    private nonisolated static func overview(_ engine: EddyEngine) -> Overview {
        if let read = try? engine.overview() {
            return Overview(
                status: read.status,
                sources: read.sources,
                stats: Dictionary(uniqueKeysWithValues: read.stats.map { ($0.sourceId, $0) }),
                series: read.series,
                collection: read.collection,
                tags: read.tags,
                galleryFolders: read.galleryFolders,
                continueReadingCount: Int(read.continueReading)
            )
        }
        // One piece failed: the others still show.
        let started = ItemQueryView(filter: .continueReading, unreadOnly: false, keep: [], limit: 0, offset: 0)
        return Overview(
            status: try? engine.status(),
            sources: (try? engine.sources()) ?? [],
            stats: Dictionary(uniqueKeysWithValues: ((try? engine.sourceOverview()) ?? []).map { ($0.sourceId, $0) }),
            series: (try? engine.seriesList()) ?? [],
            collection: (try? engine.collection()) ?? [],
            tags: (try? engine.tags()) ?? [],
            galleryFolders: (try? engine.galleryFolders()) ?? [],
            continueReadingCount: Int((try? engine.itemCount(query: started)) ?? 0)
        )
    }

    /// Only what differs is set: a view that reads the tags is not drawn
    /// again because an article was read.
    private func show(_ overview: Overview) {
        if status != overview.status { status = overview.status }
        if sources != overview.sources { sources = overview.sources }
        if stats != overview.stats { stats = overview.stats }
        if series != overview.series { series = overview.series }
        if collection != overview.collection { collection = overview.collection }
        if tags != overview.tags { tags = overview.tags }
        if galleryFolders != overview.galleryFolders { galleryFolders = overview.galleryFolders }
        if continueReadingCount != overview.continueReadingCount { continueReadingCount = overview.continueReadingCount }
        revision += 1
    }

    private var pendingReload: Task<Void, Never>?
    /// Counts the reloads asked for, so that one read in the background is
    /// dropped when a later one has been shown, or is on its way.
    @ObservationIgnored private var reloadTicket = 0
    /// Until when lists keep still: the reader is sliding between an
    /// article's text and its web page.
    @ObservationIgnored private var listsHeldUntil = Date.distantPast

    /// The reader calls this as its text and web page start trading places.
    func holdLists(for seconds: TimeInterval) {
        listsHeldUntil = max(listsHeldUntil, Date().addingTimeInterval(seconds))
    }

    /// Reading changes things in bursts (opening marks read, leaving saves
    /// the position, the end finishes it); one reload after the switch's
    /// animation covers them all instead of redrawing every list three times
    /// while it plays. The reading is done off the main thread, which only
    /// puts the answers in place: an article opened and at once turned to
    /// its web page used to stop mid-slide while the library was counted.
    func reloadSoon() {
        // What is listed has changed already; the rest follows shortly.
        itemsCache.removeAll()
        guard pendingReload == nil else { return }
        pendingReload = Task {
            try? await Task.sleep(for: .milliseconds(350))
            await listsAtRest()
            pendingReload = nil
            guard let engine else { return }
            reloadTicket += 1
            let ticket = reloadTicket
            let overview = await Task.detached(priority: .userInitiated) { Self.overview(engine) }.value
            // Something may have started moving while the library was read.
            await listsAtRest()
            guard ticket == reloadTicket else { return }
            itemsCache.removeAll()
            show(overview)
        }
    }

    /// Returns once nothing is moving that redrawing the lists would stop:
    /// fingers on the reader or the Gallery, the reader sliding between an
    /// article's text and its web page.
    private func listsAtRest() async {
        while EdgeGestureTracker.anyActive || galleryScrolling || Date() < listsHeldUntil {
            try? await Task.sleep(for: .milliseconds(50))
        }
    }

    /// `keep` stay in an unread-only list after they are read.
    ///
    /// The last few answers are kept until something changes (`reload`,
    /// `reloadSoon`): the views that list articles ask again whenever their
    /// body runs, which is several times as a swipe between columns ends,
    /// and each asking is a query and some hundreds of rows across the
    /// bridge.
    func items(_ filter: ItemFilterView, unreadOnly: Bool = false, keep: [String] = [], limit: UInt32 = 500) -> [ItemView] {
        _ = revision
        let query = ItemQueryView(filter: filter, unreadOnly: unreadOnly, keep: keep, limit: limit, offset: 0)
        if let kept = itemsCache[query] { return kept }
        let rows = (try? engine?.items(query: query)) ?? []
        if itemsCache.count >= 8 { itemsCache.removeAll() }
        itemsCache[query] = rows
        return rows
    }

    @ObservationIgnored private var itemsCache: [ItemQueryView: [ItemView]] = [:]

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

    /// How many articles of each subscription are part read: the ones
    /// Continue Reading lists.
    var readingArticles: [String: Int] {
        items(.continueReading).reduce(into: [:]) { counts, item in
            if item.kind == .feed, let source = item.sourceId { counts[source, default: 0] += 1 }
        }
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
        let found = (try? await engine.searchAll(query: query, limit: 80)) ?? []
        // The search may have found the server gone (or back).
        status = try? engine.status()
        return found
    }

    /// A long sync with the server started, moved or ended. Only the status
    /// is read again: lists reload on their own schedule while it runs, and
    /// once more when it ends.
    private func syncProgressed() {
        guard let engine else { return }
        let wasSyncing = status?.sync != nil
        status = try? engine.status()
        if wasSyncing, status?.sync == nil {
            reloadSoon()
        }
    }

    /// Server mode with the server unreachable: search covers only what is
    /// on this Mac, and changes wait to upload.
    var serverOffline: Bool {
        status?.mode == .server && status?.serverOnline == false
    }

    // MARK: Content

    /// Languages a code block can be highlighted as, for its menu.
    @ObservationIgnored lazy var codeLanguages: [String] = engine?.highlightLanguages() ?? []

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

    /// The most pixels along an image's longer side that the reader shows.
    static let readerPixels = 2400

    /// `maxPixelSize` bounds the longer side of the decoded picture; a grid
    /// of thumbnails asks for no more than its tiles show. `page` is the
    /// address of the article the picture is in, for hosts that only serve
    /// pictures to their own pages.
    func image(_ url: String, page: String? = nil, maxPixelSize: Int = AppModel.readerPixels) async -> NSImage? {
        let key = (maxPixelSize == Self.readerPixels ? url : "\(maxPixelSize):\(url)") as NSString
        if let cached = imageCache.object(forKey: key) { return cached }
        guard let bytes = await imageData(url, page: page) else { return nil }
        guard let image = await Task.detached(priority: .userInitiated, operation: { Self.decoded(bytes, maxPixelSize: maxPixelSize) }).value else { return nil }
        let pixels = image.representations.first.map { $0.pixelsWide * $0.pixelsHigh } ?? 0
        imageCache.setObject(image, forKey: key, cost: pixels * 4)
        return image
    }

    private static let imageLog = Logger(subsystem: EddyPaths.keychainService, category: "images")

    /// The picture's bytes from the core. One that can't be had is noted in
    /// the log, by host and reason: the reader shows only an empty frame.
    private func imageData(_ url: String, page: String?) async -> Data? {
        guard let engine else { return nil }
        do {
            return Data(try await engine.image(url: url, page: page).bytes)
        } catch {
            let host = URL(string: url)?.host ?? "?"
            Self.imageLog.info("image from \(host, privacy: .public) failed: \(String(describing: error), privacy: .public)")
            return nil
        }
    }

    // MARK: Feed icons

    /// The icons of subscriptions' sites, as decided this session: `nil`
    /// inside for a site without one, or one that could not be asked.
    @ObservationIgnored private var feedIcons: [String: SiteIcon?] = [:]
    @ObservationIgnored private var feedIconsLoading: [String: Task<SiteIcon?, Never>] = [:]
    /// Goes up when sites that gave no icon are worth asking again; the
    /// icons on screen ask again when it does.
    private(set) var feedIconEpoch = 0

    /// A subscription's icon if it is in hand: what a row can show in its
    /// first frame, so that scrolling doesn't make icons blink.
    func cachedFeedIcon(_ sourceID: String) -> SiteIcon? {
        feedIcons[sourceID] ?? nil
    }

    /// The icon of a subscription's site. The core looks for it once and
    /// keeps it; every row of the same feed waits on the one request.
    func feedIcon(_ sourceID: String) async -> SiteIcon? {
        if let known = feedIcons[sourceID] { return known }
        if let loading = feedIconsLoading[sourceID] { return await loading.value }
        guard let engine else { return nil }
        let loading = Task<SiteIcon?, Never> {
            guard let found = try? await engine.feedIcon(sourceId: sourceID) else { return nil }
            let bytes = Data(found.bytes)
            return await Task.detached(priority: .utility) { Self.decodedIcon(bytes) }.value
        }
        feedIconsLoading[sourceID] = loading
        let image = await loading.value
        feedIconsLoading[sourceID] = nil
        feedIcons[sourceID] = .some(image)
        return image
    }

    /// After a refresh that reached the network: the sites that gave no
    /// icon (the Mac may have been offline) are asked once more.
    private func retryMissingFeedIcons() {
        let missing = feedIcons.filter { $0.value == nil }.map(\.key)
        guard !missing.isEmpty else { return }
        for id in missing { feedIcons[id] = nil }
        feedIconEpoch += 1
    }

    /// An icon file often holds several sizes, smallest first: the largest
    /// is taken, and drawn down to what a row needs at twice its size.
    private nonisolated static func decodedIcon(_ data: Data) -> SiteIcon? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil), CGImageSourceGetCount(source) >= 1 else {
            // SVG, which ImageIO does not read.
            guard let image = NSImage(data: data), image.isValid else { return nil }
            image.size = NSSize(width: 64, height: 64)
            return SiteIcon(image: image, ink: SiteIcon.ink(of: image.cgImage(forProposedRect: nil, context: nil, hints: nil)))
        }
        var best = 0, bestWidth = 0
        for index in 0 ..< CGImageSourceGetCount(source) {
            let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any]
            let width = properties?[kCGImagePropertyPixelWidth] as? Int ?? 0
            if width > bestWidth { (best, bestWidth) = (index, width) }
        }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: 64,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, best, options as CFDictionary) else { return nil }
        return SiteIcon(image: NSImage(cgImage: image, size: NSSize(width: image.width, height: image.height)), ink: SiteIcon.ink(of: image))
    }

    /// A picture that is decoded already, without waiting for it: what a
    /// view can show in its very first frame, so that one taking the place
    /// of another with the same picture doesn't blink.
    func cachedImage(_ url: String, animated: Bool = false) -> NSImage? {
        imageCache.object(forKey: (animated ? "animated:\(url)" : url) as NSString)
    }

    /// An image with all its frames, for the media viewer to animate;
    /// `image(_:)` gives the first frame of a GIF, at a bounded size.
    func animatedImage(_ url: String) async -> NSImage? {
        let key = "animated:\(url)" as NSString
        if let cached = imageCache.object(forKey: key) { return cached }
        guard let bytes = await imageData(url, page: nil) else { return nil }
        guard let image = await Task.detached(priority: .userInitiated, operation: { NSImage(data: bytes) }).value else { return nil }
        imageCache.setObject(image, forKey: key)
        return image
    }

    /// Decodes off the main thread, at no more pixels than asked for:
    /// `NSImage(data:)` would decode a full-size photo on the main thread the
    /// first time it is drawn, in the middle of scrolling or a transition.
    /// The size in points stays the image's own, as `NSImage(data:)` gives.
    /// An animated GIF gives its first frame (`animatedImage` has them all);
    /// formats ImageIO can't read (SVG) go the old way.
    private nonisolated static func decoded(_ data: Data, maxPixelSize: Int) -> NSImage? {
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
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
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
            report(error)
        }
        listsReset += 1
        reload()
    }

    /// Bumped when an unread-only list should start over (after "Mark All
    /// as Read"); articles read in the meantime stay listed until then.
    var listsReset = 0

    /// Read and done with, by hand, on every device (and FreshRSS): read
    /// and finished, so out of Continue Reading. Opening an item only marks
    /// it read (`markOpened`).
    func markRead(_ itemIDs: [String]) {
        guard !itemIDs.isEmpty else { return }
        do {
            try engine?.markRead(itemIds: itemIDs)
        } catch {
            report(error)
        }
        reload()
    }

    /// A book or series, all its chapters; a page or PDF itself.
    func markRead(entry: CollectionEntryView) {
        markRead(entry.seriesId.map { chapters($0).map(\.id) } ?? [entry.id])
    }

    /// The one command that goes both ways (a key, the reader's button):
    /// done with, or back to unread once it is.
    func toggleRead(_ item: ItemView) {
        if item.isDone { markUnread([item.id]) } else { markRead([item.id]) }
    }

    /// The item the media viewer is showing, while it is up: what "Mark as
    /// Read" in the menu bar marks there.
    var viewedItemID: String?

    /// Back to unread and never started, on every device (and FreshRSS).
    func markUnread(_ itemIDs: [String]) {
        guard !itemIDs.isEmpty else { return }
        do {
            try engine?.markUnread(itemIds: itemIDs)
        } catch {
            report(error)
        }
        unreadMark = UnreadMark(items: Set(itemIDs))
        reload()
    }

    /// The items marked unread last. A reader showing one of them forgets
    /// where it had been read to, so leaving doesn't save that place again.
    struct UnreadMark: Equatable {
        let items: Set<String>
        var token = UUID()
    }

    var unreadMark: UnreadMark?

    /// A book or series resets all its chapters; a page or PDF itself.
    func markUnread(entry: CollectionEntryView) {
        markUnread(entry.seriesId.map { chapters($0).map(\.id) } ?? [entry.id])
    }

    func markOpened(_ id: String) {
        guard let item = item(id), !item.read else { return }
        setRead(item, true)
    }

    /// `atEnd`: the end of the text is on screen, which finishes the item.
    /// Lists refresh when that happens, and whenever the position saved has
    /// moved on (the engine saves one every few seconds of reading), so the
    /// row being read shows how far it is.
    func recordProgress(_ itemID: String, blockID: String, offset: Int, atEnd: Bool) {
        let before = item(itemID)
        _ = try? engine?.recordProgress(itemId: itemID, blockId: blockID, offset: UInt32(max(0, offset)), atEnd: atEnd, force: false)
        itemsCache.removeAll()
        guard let before, let now = try? engine?.item(id: itemID) else { return }
        if now.finished != before.finished || abs(now.progress - before.progress) >= 0.01 { reloadSoon() }
    }

    func flushProgress(_ itemID: String, blockID: String, offset: Int, atEnd: Bool) {
        _ = try? engine?.recordProgress(itemId: itemID, blockId: blockID, offset: UInt32(max(0, offset)), atEnd: atEnd, force: true)
        reloadSoon()
    }

    func recordPage(_ itemID: String, page: Int, of count: Int, force: Bool = false) {
        _ = try? engine?.recordPageProgress(itemId: itemID, page: UInt32(max(0, page)), pageCount: UInt32(max(0, count)), force: force)
        itemsCache.removeAll()
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

    /// Imports EPUB books and PDFs, then brings the last one into view;
    /// video files open the Video-to-Book preflight sheet.
    func importFiles(_ urls: [URL]) async {
        guard let engine, !urls.isEmpty else { return }
        if let videoURL = urls.first(where: { isVideoFile(path: $0.path) }) {
            pendingVideoConvert = VideoConvertRequest(source: videoURL.path, suggestedTitle: videoURL.deletingPathExtension().lastPathComponent)
            return
        }
        busy = true
        defer { busy = false }
        var imported: [ImportResultView] = []
        for url in urls {
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            do {
                imported.append(try await engine.importFile(path: url.path))
            } catch {
                report(error, about: url.lastPathComponent)
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
            report(error)
        }
        reload()
    }

    /// `quiet` is for refreshes nobody asked for (the timer, coming back to
    /// the app): an unreachable server is then not worth a banner, the
    /// status in Settings and the sidebar says it.
    func refresh(quiet: Bool = false) async {
        guard let engine, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            let s = try await engine.refresh()
            if s.offline {
                if s.newItems > 0 {
                    message = "\(s.newItems) new articles from FreshRSS. The sync server is offline."
                } else if !quiet {
                    message = "The sync server is offline. Changes on this Mac sync when it is back."
                }
            } else if s.newItems > 0 {
                message = "\(s.newItems) new articles"
            }
            loadHistory()
            retryMissingFeedIcons()
        } catch EddyError.Offline {
            if !quiet {
                message = "The sync server is offline. Everything on this Mac still works and syncs when it is back."
            }
        } catch EddyError.NotConfigured {
            message = "Connect FreshRSS or a sync server in Settings to load subscriptions."
        } catch {
            report(error)
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
            notice = Notice(text: String(localized: "Saved “\(item.title)”"), kind: .saved(itemID: item.id))
            reload()
            if let proposal = try? engine.detectSeriesIn(itemId: item.id), proposal.chapters.count >= 4 {
                pendingSeries = proposal
            }
        } catch {
            report(error)
        }
    }

    /// Subscribes to a feed, in a folder or in none. Throws what went wrong,
    /// for the sheet to say beside the address. The feed is listed at once;
    /// its articles come with the refresh started here.
    func addFeed(url: String, folder: String?) async throws {
        guard let engine else { throw EddyError.NotConfigured(message: "") }
        let added = try await engine.addFeed(url: url, folder: folder)
        reload()
        notice = Notice(text: String(localized: "Subscribed to “\(added.title)”"), kind: .subscribed(sourceID: added.sourceId))
        Task { await refresh(quiet: true) }
    }

    /// A subscription waiting for the user to confirm unsubscribing.
    var pendingUnsubscribe: SourceView?

    /// Unsubscribes, after the user has confirmed. The feed's articles go
    /// with it, so one of them being read is closed, and a sidebar that
    /// had the feed chosen goes back to everything.
    func unsubscribe(_ feed: SourceView) async {
        guard let engine else { return }
        let reading = currentItemID.flatMap { item($0) }.map { $0.kind == .feed && $0.sourceId == feed.id } ?? false
        do {
            try await engine.removeFeed(sourceId: feed.id)
            if reading { currentItemID = nil }
            if threePaneSelection == .feed(feed.id) { chose(.smart(.all)) }
            message = String(localized: "Unsubscribed from “\(feed.title)”")
        } catch EddyError.Offline {
            notice = Notice(text: String(localized: "The sync server is offline, and it is the one that unsubscribes. Try again when it is back."), kind: .error)
        } catch let EddyError.Failed(message), let EddyError.UpgradeRequired(message), let EddyError.Unauthorized(message) {
            notice = Notice(text: message, kind: .error)
        } catch {
            report(error)
        }
        reload()
    }

    /// Lists a subscription where the current mode lists them; the Gallery
    /// hands over to the three panes, as it does for a saved page.
    func show(feed sourceID: String) {
        let defaults = UserDefaults.standard
        switch defaults.string(forKey: "app.mode") {
        case AppMode.library.rawValue:
            navigation = PaletteRequest(value: .libraryFeed(sourceID: sourceID))
        case AppMode.gallery.rawValue:
            chose(.feed(sourceID))
            defaults.set(AppMode.threePane.rawValue, forKey: "app.mode")
        default:
            navigation = PaletteRequest(value: .threePane(.feed(sourceID)))
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
                report(error)
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

    /// Shows the article's full text or the feed's content; the page is
    /// fetched only the first time, and the choice stays with the article.
    func setFullText(_ id: String, _ on: Bool) async -> Bool {
        do {
            _ = try await engine?.setFullText(itemId: id, on: on)
            leadImages[id] = nil
            reload()
            return true
        } catch {
            report(error)
            return false
        }
    }

    /// Applies the full-text and AI rules to an item being opened: the full
    /// text they fetched, and what they ask to show.
    func applyRules(_ id: String) async -> RulesOnOpenView? {
        guard let applied = try? await engine?.applyRules(itemId: id) else { return nil }
        if applied.document != nil {
            leadImages[id] = nil
            reloadSoon()
        }
        return applied
    }

    func fullTextOnOpen(_ id: String) -> Bool {
        (try? engine?.fullTextOnOpen(itemId: id)) ?? false
    }

    private var captureImport: Task<Void, Never>?

    /// One import at a time: the spool watcher and a "Show" from the
    /// browser can both ask, and two at once would save a page twice.
    func importCaptures() async {
        let previous = captureImport
        let run = Task {
            await previous?.value
            if let n = try? await engine?.importCaptures(), n > 0 {
                message = n == 1 ? "Imported 1 page from the browser" : "Imported \(n) pages from the browser"
                reload()
            }
        }
        captureImport = run
        await run.value
    }

    /// Opens an item where it lives: a saved page in its collection, an
    /// article in its subscription. The Gallery has no place for a saved
    /// page, so it hands over to the three panes.
    func reveal(itemID id: String) {
        let defaults = UserDefaults.standard
        if defaults.string(forKey: "app.mode") == AppMode.gallery.rawValue {
            defaults.set(AppMode.threePane.rawValue, forKey: "app.mode")
        }
        revealedItem = PaletteRequest(value: id)
    }

    /// `eddy://show?url=…`, from the browser extension: the page saved from
    /// that address, once anything still waiting in the spool is in.
    /// `eddy://add-feed?url=…&title=…` opens the Add Feed sheet on a feed the
    /// extension found; nothing is subscribed to until it is confirmed there.
    func open(_ link: URL) async {
        guard link.scheme == "eddy" else { return }
        let query = URLComponents(url: link, resolvingAgainstBaseURL: false)?.queryItems ?? []
        func value(_ name: String) -> String? { query.first { $0.name == name }?.value }
        if link.host() == "add-feed" {
            guard let feed = value("url"), let scheme = URL(string: feed)?.scheme?.lowercased(),
                  ["http", "https"].contains(scheme) else { return }
            AppDelegate.comeForward()
            addingFeed = AddFeedRequest(url: feed, title: value("title"))
            return
        }
        guard link.host() == "show", let page = value("url") else { return }
        AppDelegate.comeForward()
        await importCaptures()
        if let id = (try? engine?.savedItemForUrl(url: page)) ?? nil {
            reveal(itemID: id)
        } else {
            message = String(localized: "That page isn’t in Eddy yet.")
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
        do { try engine?.setFolderGallery(name: folder, shown: shown) } catch { report(error) }
        reload()
    }

    /// Gives the subscription its own full-text mode (a rule at the top),
    /// or `nil` to leave it to the other rules.
    func setFullText(source: SourceView, _ mode: FullTextModeView?) {
        do { try engine?.setSourceFullTextRule(sourceId: source.id, mode: mode) } catch { report(error) }
        reload()
    }

    func setGallery(source: SourceView, _ shown: Bool) {
        do { try engine?.setSourceGallery(sourceId: source.id, shown: shown) } catch { report(error) }
        reload()
    }

    /// Off the main thread: the engine reads each item's media, and its
    /// article when that has no thumbnail yet.
    func galleryItems(_ scope: GalleryScopeView, unreadOnly: Bool, offset: Int, limit: Int = 200) async -> [GalleryEntryView] {
        guard let engine else { return [] }
        return await Task.detached(priority: .userInitiated) {
            (try? engine.galleryItems(scope: scope, unreadOnly: unreadOnly, limit: UInt32(limit), offset: UInt32(offset))) ?? []
        }.value
    }

    /// The Gallery's grid is moving. Lists reload once it comes to rest:
    /// reloading under it holds the scroll up.
    @ObservationIgnored var galleryScrolling = false

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

    var translationConfigured: Bool {
        guard let engine, engine.hasAiRoleKey(role: .translation) else { return false }
        return (try? engine.aiSettings().translationProvider) != nil
    }

    var summaryConfigured: Bool {
        guard let engine, engine.hasAiRoleKey(role: .summary) else { return false }
        return (try? engine.aiSettings().summaryProvider) != nil
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

    /// Translates text that isn't stored content (a web page's paragraphs);
    /// nothing is kept. `key` is for `cancelTranslation`.
    func translateText(_ key: String, _ segments: [SegmentView], onBatch: @escaping @MainActor ([String: String]) -> Void) async throws -> [String: String] {
        guard let engine else { return [:] }
        return try await engine.translateText(key: key, segments: segments, listener: BatchListener(onBatch))
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
            Task { @MainActor in await self?.refresh(quiet: true) }
        }
    }

    private func becameActive() {
        guard let engine, let status else { return }
        if status.mode == .server {
            Task {
                await engine.startLiveSync()
                await refreshServerCapability()
            }
        }
        Task { await refreshVideoJobs(initial: false) }
        let stale = status.lastRefresh.map { Date().timeIntervalSince1970 * 1000 - Double($0) > 10 * 60 * 1000 } ?? true
        if status.mode != .unconfigured, stale { Task { await refresh(quiet: true) } }
    }

    // MARK: Video-to-Book Jobs

    var activeVideoJobsCount: Int {
        videoJobs.filter { $0.status == "queued" || $0.status == "running" }.count
    }

    func requestVideoConvert(source: String = "", title: String? = nil) {
        pendingVideoConvert = VideoConvertRequest(source: source, suggestedTitle: title)
    }

    func refreshServerCapability() async {
        guard let engine, status?.mode == .server else {
            serverVideoCapable = false
            return
        }
        serverVideoCapable = (try? await engine.serverStatus())?.videoCapable ?? false
    }

    func scheduleVideoJobsRefresh() {
        guard videoJobsRefreshTask == nil else { return }
        videoJobsRefreshTask = Task {
            try? await Task.sleep(for: .milliseconds(250))
            videoJobsRefreshTask = nil
            await refreshVideoJobs(initial: false)
        }
    }

    func refreshVideoJobs(initial: Bool) async {
        guard let engine else { return }
        let latest = (try? await engine.videoJobs()) ?? []
        if initial {
            completedJobIDs = Set(latest.filter { $0.status == "succeeded" }.map(\.id))
            videoJobs = latest
            return
        }
        var newlyFinished: VideoJobView?
        for job in latest where job.status == "succeeded" && !completedJobIDs.contains(job.id) {
            completedJobIDs.insert(job.id)
            newlyFinished = job
        }
        videoJobs = latest
        if let done = newlyFinished {
            if done.target == .server {
                _ = try? await engine.refresh()
            }
            reload()
            if let itemID = done.resultItemId {
                notice = Notice(text: String(localized: "Converted “\(done.title)”"), kind: .saved(itemID: itemID))
            } else {
                message = String(localized: "Converted “\(done.title)”")
            }
        }
    }

    func startVideoJob(_ request: StartVideoJobRequestView) async {
        guard let engine else { return }
        do {
            _ = try await engine.startVideoJob(request: request)
            await refreshVideoJobs(initial: false)
            showingVideoJobs = true
        } catch {
            report(error)
        }
    }

    func retryVideoJob(_ job: VideoJobView) async {
        guard let engine else { return }
        do {
            _ = try await engine.retryVideoJob(id: job.id, target: job.target)
            await refreshVideoJobs(initial: false)
        } catch {
            report(error)
        }
    }

    func cancelVideoJob(_ job: VideoJobView) async {
        guard let engine else { return }
        do {
            _ = try await engine.cancelVideoJob(id: job.id, target: job.target)
            await refreshVideoJobs(initial: false)
        } catch {
            report(error)
        }
    }

    func dismissVideoJob(_ job: VideoJobView) async {
        guard let engine else { return }
        do {
            try await engine.dismissVideoJob(id: job.id, target: job.target)
            completedJobIDs.remove(job.id)
            await refreshVideoJobs(initial: false)
        } catch {
            report(error)
        }
    }
}

/// Forwards background video job updates to the main actor.
final class VideoJobBridge: VideoJobListener, @unchecked Sendable {
    private let handler: @MainActor () -> Void

    init(_ handler: @escaping @MainActor () -> Void) {
        self.handler = handler
    }

    func onChange() {
        let handler = handler
        Task { @MainActor in handler() }
    }
}

/// Forwards "a background sync changed something" and "a long sync moved"
/// to the main actor.
final class SyncBridge: SyncListener, @unchecked Sendable {
    private let change: @MainActor () -> Void
    private let progress: @MainActor () -> Void

    init(change: @escaping @MainActor () -> Void, progress: @escaping @MainActor () -> Void) {
        self.change = change
        self.progress = progress
    }

    func onChange() {
        let change = change
        Task { @MainActor in change() }
    }

    func onProgress() {
        let progress = progress
        Task { @MainActor in progress() }
    }
}

extension SyncProgressView {
    /// 0...1, or nil when the server does not say how much there is.
    var fraction: Double? {
        guard let total, total > 0 else { return nil }
        return min(1, Double(done) / Double(total))
    }

    /// The full sentence, for Settings.
    var detail: String {
        let count = total.map { "\(done.formatted()) of \($0.formatted())" } ?? done.formatted()
        switch phase {
        case .uploading: return "Uploading… \(count) changes"
        case .downloading: return "Downloading… \(count) changes"
        case .fetchingContent: return "Fetching articles for offline reading… \(count)"
        }
    }

    /// One short line, for the sidebar.
    var summary: String {
        let percent = fraction.map { " \(Int(($0 * 100).rounded(.down)))%" } ?? ""
        switch phase {
        case .uploading: return "Uploading…\(percent)"
        case .downloading: return "Syncing…\(percent)"
        case .fetchingContent: return "Fetching articles…\(percent)"
        }
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
