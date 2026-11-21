import EddyKit
import SwiftUI

extension GalleryScopeView {
    /// Stable name for stored preferences.
    var key: String {
        switch self {
        case .all: "all"
        case .saved: "saved"
        case let .folder(name): "folder:\(name)"
        case let .source(id): "source:\(id)"
        }
    }

    init(key: String) {
        if key.hasPrefix("folder:") {
            self = .folder(name: String(key.dropFirst("folder:".count)))
        } else if key.hasPrefix("source:") {
            self = .source(id: String(key.dropFirst("source:".count)))
        } else {
            self = key == "saved" ? .saved : .all
        }
    }
}

extension GalleryEntryView: @retroactive Identifiable {
    public var id: String { item.id }
}

/// Media resolved while the Gallery is on screen, for its tiles and the
/// viewer. The engine keeps results, so a reopened Gallery has them at once.
@Observable
@MainActor
final class GalleryMedia {
    private(set) var sets: [String: MediaSetView] = [:]
    @ObservationIgnored private var prefetched: Set<String> = []
    @ObservationIgnored private let model: AppModel

    init(model: AppModel) {
        self.model = model
    }

    func set(for entry: GalleryEntryView) -> MediaSetView? {
        sets[entry.item.id] ?? entry.media
    }

    /// Counts the changes that move the grid: an item's shape, or an item
    /// found to have no media. The grid lays out again when it changes, and
    /// not for every item resolved.
    private(set) var layoutVersion = 0

    /// `known`: the media the item's entry came with, to tell what changed.
    func resolve(_ itemID: String, force: Bool = false, known: MediaSetView? = nil) async {
        guard force || sets[itemID] == nil else { return }
        if let set = await model.resolveMedia(itemID, force: force) { store(set, for: itemID, known: known) }
    }

    private func store(_ set: MediaSetView, for itemID: String, known: MediaSetView? = nil) {
        // An item not resolved yet is laid out as one with media of no
        // known shape.
        let before = (sets[itemID] ?? known).map(Self.shape) ?? Shape(noMedia: false, aspect: nil)
        sets[itemID] = set
        if before != Self.shape(set) { layoutVersion += 1 }
    }

    /// What of a media set the grid's layout depends on.
    private struct Shape: Equatable {
        var noMedia: Bool
        var aspect: CGFloat?
    }

    private static func shape(_ set: MediaSetView) -> Shape {
        Shape(noMedia: set.status == .noMedia, aspect: ownAspect(set))
    }

    /// Width over height as the media itself says: its size, or 16:9 for
    /// a player.
    private static func ownAspect(_ set: MediaSetView?) -> CGFloat? {
        guard let first = set?.items.first else { return nil }
        if let w = first.width, let h = first.height, w > 0, h > 0 { return CGFloat(w) / CGFloat(h) }
        return first.kind == .embed ? 16 / 9 : nil
    }

    @ObservationIgnored private var verified: Set<String> = []

    /// A thumbnail didn't load: asks the engine to check the item's media
    /// with the sites, once per item, and drop what was removed. An item
    /// left with nothing drops out of the grid.
    func verify(_ itemID: String) async {
        guard verified.insert(itemID).inserted, let set = await model.verifyMedia(itemID) else { return }
        store(set, for: itemID)
    }

    /// Resolves what is about to scroll into view, in the background.
    func prefetch(_ entries: some Sequence<GalleryEntryView>) {
        let ids = entries.filter { $0.media == nil && sets[$0.item.id] == nil && prefetched.insert($0.item.id).inserted }.map(\.item.id)
        model.prefetchMedia(ids)
    }

    /// Width over height of thumbnails as they load, for items whose media
    /// doesn't say.
    @ObservationIgnored private var loadedAspects: [String: CGFloat] = [:]

    func noteAspect(_ aspect: CGFloat, for entry: GalleryEntryView) {
        guard aspect.isFinite, aspect > 0, loadedAspects[entry.item.id] == nil else { return }
        loadedAspects[entry.item.id] = aspect
        if Self.ownAspect(set(for: entry)) == nil { layoutVersion += 1 }
    }

    /// The shape an item takes in the grid: its media's own size, players
    /// at 16:9, else its thumbnail once loaded; square until known.
    func aspect(for entry: GalleryEntryView) -> CGFloat {
        let known = Self.ownAspect(set(for: entry)) ?? loadedAspects[entry.item.id]
        return min(max(known ?? 1, 0.45), 2.6)
    }
}

/// Rows of the Gallery grid: each row's items keep their shapes and are
/// scaled so the row spans the width; the last row keeps the target
/// height and starts at the leading edge.
struct JustifiedRow: Identifiable {
    struct Cell {
        let entry: GalleryEntryView
        let index: Int
        let width: CGFloat
    }

    let cells: [Cell]
    let height: CGFloat
    var id: String { cells.first?.entry.id ?? "" }

    static func rows(_ entries: [GalleryEntryView], width: CGFloat, targetHeight: CGFloat, spacing: CGFloat, aspect: (GalleryEntryView) -> CGFloat) -> [JustifiedRow] {
        guard width > 0 else { return [] }
        var rows: [JustifiedRow] = []
        var pending: [(GalleryEntryView, Int, CGFloat)] = []
        var aspectSum: CGFloat = 0
        func close(height: CGFloat) {
            rows.append(JustifiedRow(cells: pending.map { Cell(entry: $0.0, index: $0.1, width: ($0.2 * height).rounded(.down)) }, height: height.rounded(.down)))
            pending = []
            aspectSum = 0
        }
        for (index, entry) in entries.enumerated() {
            let a = aspect(entry)
            pending.append((entry, index, a))
            aspectSum += a
            let gaps = spacing * CGFloat(pending.count - 1)
            if aspectSum * targetHeight + gaps >= width {
                close(height: (width - gaps) / aspectSum)
            }
        }
        if !pending.isEmpty {
            let gaps = spacing * CGFloat(pending.count - 1)
            close(height: min(targetHeight, (width - gaps) / aspectSum))
        }
        return rows
    }
}

/// How large the Gallery's thumbnails are: the height its rows aim for.
/// Stored under `key`; the View menu's Zoom In and Zoom Out step through
/// `steps`.
enum GalleryDensity {
    static let key = "gallery.rowHeight"
    static let standard: Double = 220
    static let steps: [Double] = [110, 150, 220, 300, 400]

    static func larger(than height: Double) -> Double? {
        steps.first { $0 > height + 0.5 }
    }

    static func smaller(than height: Double) -> Double? {
        steps.last { $0 < height - 0.5 }
    }

    /// The pixels to decode a thumbnail at, along its longer side: enough
    /// for a tile twice as wide as the rows are high, and one of a few
    /// sizes so a small change of height keeps the pictures already decoded.
    static func thumbnailPixels(rowHeight: Double, scale: CGFloat) -> Int {
        let needed = Int(rowHeight * 2 * Double(scale))
        return [640, 960, 1440].first { $0 >= needed } ?? AppModel.readerPixels
    }
}

/// Gallery mode: media from marked subscriptions and folders and from
/// saved media links, as a grid; a tile opens the media viewer.
struct GalleryView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.displayScale) private var displayScale
    @AppStorage("app.mode") private var mode: AppMode = .threePane
    @AppStorage("gallery.sidebar") private var showSidebar = true
    /// Whether the window has room for the sidebar beside the grid.
    @State private var dock = ColumnDock()
    @AppStorage("gallery.unreadOnly") private var unreadOnly = false
    @AppStorage("gallery.scope") private var scopeKey = "all"
    @AppStorage(GalleryDensity.key) private var rowHeight = GalleryDensity.standard
    @State private var entries: [GalleryEntryView] = []
    @State private var exhausted = false
    /// The first load has finished: an empty grid is empty, not waiting.
    @State private var loaded = false
    /// Counts loads, so one that was overtaken leaves its result unused.
    @State private var generation = 0
    @State private var loadingMore = false
    /// The item open in the viewer. By ID, not position: items found to
    /// have no media drop out of the grid while the viewer is open, and a
    /// position would then name a different item.
    @State private var viewing: String?
    /// The keys are in the sidebar or with the grid.
    @FocusState private var focus: PaneFocus?
    /// The item the arrow keys are at, framed while the keys are with the
    /// grid; Space or Return opens it.
    @State private var picked: String?
    /// Lists changed while the viewer was open; reload when it closes so
    /// the items being looked at don't move.
    @State private var stale = false
    /// Read by scrolling past them here: they show as read and stay in an
    /// unread-only list until it is loaded afresh, so nothing above the
    /// view drops out and moves the grid.
    @State private var scrolledPast: Set<String> = []

    @State private var gridWidth: CGFloat = 0
    /// The grid as laid out: the entries with media, their rows, and where
    /// each row ends in the scrolled content. Kept, not worked out each
    /// time the view updates: see `relayout`.
    @State private var items: [GalleryEntryView] = []
    @State private var rows: [JustifiedRow] = []
    @State private var rowBottoms: [CGFloat] = []
    /// How many rows are above the top of the view. Not state: it changes
    /// as the grid scrolls and nothing is drawn from it.
    @State private var position = ScrollMark()
    /// The grid is moving: under the fingers (`scrollPhase`) or gliding on
    /// after them.
    @State private var scrollPhase = ScrollPhase.idle
    @State private var gliding = false

    private final class ScrollMark {
        var rowsAbove = 0
    }

    private static let pageSize = 200
    private static let prefetchAhead = 20
    private static let spacing: CGFloat = 6
    private static let gridPadding: CGFloat = 8

    private var scope: GalleryScopeView { GalleryScopeView(key: scopeKey) }
    private var media: GalleryMedia { model.media }

    var body: some View {
        LeadingColumn(
            dock: dock, beside: $showSidebar, width: PaneTier.sidebarWidth,
            gesturesEnabled: !model.overlayVisible && viewing == nil && model.mediaRequest == nil,
            onShown: { shown in if !shown, focus == .sidebar { focus = .list } }
        ) {
            SidebarColumn(dock: dock, beside: $showSidebar) {
                GallerySidebar(scopeKey: $scopeKey, focus: $focus, enter: enterGrid, escape: leaveSidebar)
            }
        } content: {
            VStack(spacing: 0) {
                ContentStrip(dock: dock, beside: $showSidebar) {
                    Button { unreadOnly.toggle() } label: { Label("Unread Only", systemImage: "circle.inset.filled") }
                        .buttonStyle(FlatButtonStyle(on: unreadOnly))
                        .help("Show only media you haven’t opened")
                }
                ColumnTitle(title: Text("Gallery"), note: loaded ? Text("^[\(items.count) Item](inflect: true)") : nil)
                grid
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: {
                        gridWidth = $0
                        relayout()
                    }
            }
        }
        .docks(dock, model: model)
        .overlay {
            // The viewer gets every loaded item, including ones since found
            // to have no media, and steps over those itself: the list only
            // grows while it is open, so the item on screen stays put.
            if let id = viewing, entries.contains(where: { $0.id == id }) {
                MediaViewer(entries: entries, current: Binding(get: { id }, set: { viewing = $0 }), media: media) {
                    viewing = nil
                }
            }
        }
        .onAppear {
            load()
            // The keys start in the first column that shows: the sidebar,
            // at the scope chosen, or the grid where the sidebar is away.
            DispatchQueue.main.async { focus = sidebarShows ? .sidebar : .list }
        }
        // The window turned out, or became, too narrow for the sidebar
        // beside the grid: the keys don't stay in it out of sight.
        .onChange(of: dock.over) { if focus == .sidebar, !sidebarShows { focus = .list } }
        .onDisappear { model.galleryScrolling = false }
        .onChange(of: scopeKey) { load(reset: true) }
        .onChange(of: unreadOnly) { load(reset: true) }
        .onChange(of: model.revision) {
            if viewing == nil { load() } else { stale = true }
        }
        .onChange(of: viewing) { _, now in
            if let now {
                // The viewer moves on from item to item: the grid follows.
                picked = now
            } else {
                // The viewer had the keys; they go back to the grid.
                DispatchQueue.main.async { focus = .list }
            }
            if now == nil, stale {
                stale = false
                load()
            }
        }
        .onChange(of: media.layoutVersion) { relayout() }
        .onChange(of: model.navigation) { _, request in
            guard case let .gallery(destination) = request?.value else { return }
            model.navigation = nil
            scopeKey = destination.key
        }
    }

    @ViewBuilder
    private var grid: some View {
        if !loaded {
            Color.clear
        } else if items.isEmpty {
            if model.galleryFeeds.isEmpty && !model.collection.contains(where: { $0.kind == .media }) {
                ContentUnavailableView("No Media Yet", systemImage: "photo.on.rectangle.angled", description: Text("In Three-Pane, right-click a subscription or folder and choose Show in Gallery. Saved YouTube, RedGIFs and image links appear here too."))
            } else {
                ContentUnavailableView(unreadOnly ? "Nothing Unread" : "Nothing Here", systemImage: "photo.on.rectangle")
            }
        } else {
            let pixels = GalleryDensity.thumbnailPixels(rowHeight: rowHeight, scale: displayScale)
            let bottoms = rowBottoms
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: Self.spacing) {
                        ForEach(rows) { row in
                            HStack(spacing: Self.spacing) {
                                ForEach(row.cells, id: \.entry.id) { cell in
                                    tile(cell, pixels: pixels)
                                        .frame(width: cell.width, height: row.height)
                                }
                            }
                        }
                    }
                    .padding(Self.gridPadding)
                    .background(LongMomentumScrolling { gliding in
                        self.gliding = gliding
                        model.galleryScrolling = gliding || scrollPhase != .idle
                    })
                }
                // One reading of where the grid is for all of it, and only
                // when a row passes the top: a tile each watching its own
                // place is work on every frame of a scroll.
                .onScrollGeometryChange(for: Int.self) { geometry in
                    Self.rowsAbove(geometry.contentOffset.y + geometry.contentInsets.top, in: bottoms)
                } action: { before, now in
                    position.rowsAbove = now
                    if now > before { markScrolledPast(rows: before..<now) }
                }
                .onScrollPhaseChange { _, phase in
                    scrollPhase = phase
                    model.galleryScrolling = gliding || phase != .idle
                }
                .onChange(of: rowHeight) { resize(proxy) }
                .focusable()
                .focusEffectDisabled()
                .pane($focus, .list, onReturn: openPicked, onEscape: revealSidebar)
                .onKeyPress(keys: [.upArrow, .downArrow, .leftArrow, .rightArrow, .space]) { press in
                    gridKey(press.key, proxy)
                }
                // Back from the viewer, which may have gone on to an item
                // further down.
                .onChange(of: viewing) { _, now in
                    if now == nil, let row = place(of: picked)?.row { proxy.scrollTo(rows[row].id) }
                }
            }
        }
    }

    /// The number of rows that end above `offset` in the content.
    private static func rowsAbove(_ offset: CGFloat, in bottoms: [CGFloat]) -> Int {
        var (low, high) = (0, bottoms.count)
        while low < high {
            let middle = (low + high) / 2
            if bottoms[middle] < offset { low = middle + 1 } else { high = middle }
        }
        return low
    }

    // MARK: Keys

    /// The sidebar is on screen: beside the grid, or pulled in over it.
    /// Read when a key is handled, not in `body`.
    private var sidebarShows: Bool { dock.shown($showSidebar, model: model).wrappedValue }

    /// Esc in the grid, or Left from the start of a row: back to the
    /// sidebar, which comes in if it was away.
    private func revealSidebar() {
        if !sidebarShows {
            withAnimation(.easeOut(duration: 0.24)) { dock.shown($showSidebar, model: model).wrappedValue = true }
        }
        focus = .sidebar
    }

    /// Esc in the sidebar: it comes in if the keys were in it while it was
    /// away; where it lies over the grid it goes away and the keys go to
    /// the grid, which is what `LeadingColumn`'s own Esc does too.
    private func leaveSidebar() {
        if !sidebarShows {
            revealSidebar()
        } else if dock.over {
            withAnimation(.easeOut(duration: 0.24)) { model.sidebarPulled = false }
            focus = .list
        }
    }

    /// Return or Right in the sidebar: the keys go to the grid, at the item
    /// they were at last, or the first of the rows in view. A sidebar that
    /// lies over the grid goes away.
    private func enterGrid() {
        guard !items.isEmpty else { return }
        if place(of: picked) == nil {
            let top = rows.indices.contains(position.rowsAbove) ? rows[position.rowsAbove] : rows.first
            picked = top?.cells.first?.entry.id
        }
        if dock.over, sidebarShows {
            withAnimation(.easeOut(duration: 0.24)) { model.sidebarPulled = false }
        }
        focus = .list
    }

    /// Space or Return on the item picked: the viewer.
    private func openPicked() {
        if place(of: picked) != nil { viewing = picked }
    }

    /// Where an item is in the grid.
    private func place(of id: String?) -> (row: Int, cell: Int)? {
        guard let id else { return nil }
        for (r, row) in rows.enumerated() {
            if let c = row.cells.firstIndex(where: { $0.entry.id == id }) { return (r, c) }
        }
        return nil
    }

    /// The arrows move through the grid: along a row and on to the next,
    /// up and down to the item nearest above or below. Left from the start
    /// of a row goes back to the sidebar.
    private func gridKey(_ key: KeyEquivalent, _ proxy: ScrollViewProxy) -> KeyPress.Result {
        if key == .space {
            openPicked()
            return .handled
        }
        guard let at = place(of: picked) else {
            enterGrid()
            return .handled
        }
        var to = at
        switch key {
        case .leftArrow:
            guard at.cell > 0 else {
                revealSidebar()
                return .handled
            }
            to.cell -= 1
        case .rightArrow:
            if at.cell + 1 < rows[at.row].cells.count {
                to.cell += 1
            } else if at.row + 1 < rows.count {
                to = (at.row + 1, 0)
            }
        default:
            let row = at.row + (key == .upArrow ? -1 : 1)
            guard rows.indices.contains(row) else { return .handled }
            // The item whose middle is nearest this one's.
            func middles(_ row: JustifiedRow) -> [CGFloat] {
                var x: CGFloat = 0
                return row.cells.map { cell in
                    defer { x += cell.width + Self.spacing }
                    return x + cell.width / 2
                }
            }
            let here = middles(rows[at.row])[at.cell]
            let there = middles(rows[row])
            to = (row, there.indices.min { abs(there[$0] - here) < abs(there[$1] - here) } ?? 0)
        }
        picked = rows[to.row].cells[to.cell].entry.id
        proxy.scrollTo(rows[to.row].id)
        return .handled
    }

    /// Lays the grid out again. Called when what it depends on changes
    /// (the entries, the width, the row height, an item's shape) and not
    /// from `body`: that runs for every item resolved and every thumbnail
    /// loaded, and laying out all the entries each time held scrolling up.
    private func relayout() {
        let media = media
        items = entries.filter { media.set(for: $0)?.status != .noMedia }
        rows = JustifiedRow.rows(items, width: gridWidth - 2 * Self.gridPadding, targetHeight: rowHeight, spacing: Self.spacing, aspect: media.aspect(for:))
        var bottom = Self.gridPadding - Self.spacing
        rowBottoms = rows.map { row in
            bottom += Self.spacing + row.height
            return bottom
        }
    }

    /// The thumbnails changed size: the row at the top keeps its first item
    /// there.
    private func resize(_ proxy: ScrollViewProxy) {
        let top = rows.indices.contains(position.rowsAbove) ? rows[position.rowsAbove].cells.first?.entry.id : nil
        relayout()
        guard let top, let row = rows.first(where: { $0.cells.contains { $0.entry.id == top } }) else { return }
        // After the new rows are in the grid.
        DispatchQueue.main.async { proxy.scrollTo(row.id, anchor: .top) }
    }

    private func tile(_ cell: JustifiedRow.Cell, pixels: Int) -> some View {
        let (entry, index) = (cell.entry, cell.index)
        let media = media
        return GalleryTile(
            entry: entry, media: media, pixels: pixels,
            read: entry.item.read || scrolledPast.contains(entry.id)
        )
        // The item the keys are at, while they are with the grid.
        .overlay {
            Rectangle()
                .strokeBorder(Theme.accent, lineWidth: 3)
                .opacity(entry.id == picked && focus == .list ? 1 : 0)
                .allowsHitTesting(false)
        }
        .onTapGesture {
            picked = entry.id
            viewing = entry.id
        }
        .contextMenu { menu(for: entry.item) }
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(Text(entry.item.title))
        .task(id: entry.id) {
            let items = items
            media.prefetch(items[min(index + 1, items.count)..<min(index + 1 + Self.prefetchAhead, items.count)])
            if index >= items.count - Self.prefetchAhead { loadMore() }
            await media.resolve(entry.item.id, known: entry.media)
        }
    }

    @ViewBuilder
    private func menu(for item: ItemView) -> some View {
        Button(item.starred ? "Unstar" : "Star") { model.setStarred(item, !item.starred) }
        if item.read {
            Button("Mark as Unread") { model.markUnread([item.id]) }
        } else {
            Button("Mark as Read") { model.markRead([item.id]) }
        }
        if let url = item.url.flatMap(URL.init(string:)) {
            Button("Open in Browser") { NSWorkspace.shared.open(url) }
            Button("Copy Link") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(url.absoluteString, forType: .string)
            }
        }
    }

    /// Scrolled up past the top of the grid: looked at, so read. There is
    /// no progress to keep for a picture.
    private func markScrolledPast(rows range: Range<Int>) {
        for row in rows[range.clamped(to: rows.indices)] {
            for cell in row.cells where !cell.entry.item.read && scrolledPast.insert(cell.entry.id).inserted {
                model.setRead(cell.entry.item, true)
            }
        }
    }

    /// Reloads the list, keeping as many items as are loaded unless `reset`.
    private func load(reset: Bool = false) {
        if reset { scrolledPast = [] }
        generation += 1
        let (token, scope, unreadOnly) = (generation, scope, unreadOnly)
        let count = reset ? Self.pageSize : max(Self.pageSize, entries.count)
        Task {
            var fresh = await model.galleryItems(scope, unreadOnly: unreadOnly, offset: 0, limit: count)
            guard token == generation else { return }
            exhausted = fresh.count < count
            if unreadOnly, !scrolledPast.isEmpty {
                let known = Set(fresh.map(\.id))
                fresh += entries.filter { scrolledPast.contains($0.id) && !known.contains($0.id) }
                // The engine's order: newest first, then by ID.
                fresh.sort { $0.item.publishedAt != $1.item.publishedAt ? $0.item.publishedAt > $1.item.publishedAt : $0.id < $1.id }
            }
            entries = fresh
            loaded = true
            relayout()
        }
    }

    private func loadMore() {
        guard !exhausted, !loadingMore else { return }
        loadingMore = true
        let (token, scope, unreadOnly) = (generation, scope, unreadOnly)
        Task {
            let more = await model.galleryItems(scope, unreadOnly: unreadOnly, offset: entries.count, limit: Self.pageSize)
            loadingMore = false
            // A reload since then has its own idea of how far the list goes.
            guard token == generation else { return }
            exhausted = more.count < Self.pageSize
            let known = Set(entries.map(\.id))
            entries.append(contentsOf: more.filter { !known.contains($0.id) })
            relayout()
        }
    }
}

/// One item in the grid: its thumbnail with what kind of media it is.
struct GalleryTile: View {
    @Environment(AppModel.self) private var model
    let entry: GalleryEntryView
    /// Read here and not handed in by the grid, so an item resolving
    /// redraws the tiles on screen and not the grid.
    let media: GalleryMedia
    /// The most pixels to decode the thumbnail at, along its longer side.
    let pixels: Int
    var read: Bool
    @State private var image: NSImage?

    private var mediaSet: MediaSetView? { media.set(for: entry) }
    private var thumbnail: String? { mediaSet?.thumbnail ?? entry.thumbnail }

    private struct Picture: Equatable {
        let url: String?
        let pixels: Int
    }

    var body: some View {
        // The picture fills the size the row gives without taking part in
        // layout: a filled image would otherwise widen the tile past it.
        Rectangle()
            .fill(.quaternary)
            .overlay {
                if let image {
                    Image(nsImage: image).resizable().scaledToFill()
                } else {
                    Image(systemName: "photo").font(.title).foregroundStyle(.tertiary)
                }
            }
            .clipped()
            .contentShape(RoundedRectangle(cornerRadius: 6))
            .overlay(alignment: .bottomLeading) { MediaBadges(mediaSet: mediaSet).padding(6) }
            .overlay(alignment: .topTrailing) {
                if !read {
                    Circle().fill(.tint).frame(width: 8, height: 8).padding(7).accessibilityLabel("Unread")
                }
            }
            .help(entry.item.title)
            .task(id: Picture(url: thumbnail, pixels: pixels)) {
                guard let thumbnail else { return }
                let loaded = await model.image(thumbnail, page: entry.item.url, maxPixelSize: pixels)
                // A picture at another size stays until this one is in.
                if loaded != nil || !Task.isCancelled { image = loaded }
                if let size = loaded?.size, size.height > 0 {
                    media.noteAspect(size.width / size.height, for: entry)
                } else if !Task.isCancelled {
                    Task { await media.verify(entry.item.id) }
                }
            }
    }
}

/// Marks a subscription or folder in the sidebar as shown in the Gallery;
/// fainter when a subscription is there through its folder.
struct GalleryMark: View {
    var throughFolder: String?

    var body: some View {
        Image(systemName: "photo.on.rectangle.angled")
            .font(.caption)
            .foregroundStyle(throughFolder == nil ? .secondary : .tertiary)
            .help(throughFolder.map { "Shown in the Gallery with the folder “\($0)”" } ?? "Shown in the Gallery")
            .accessibilityLabel("In Gallery")
    }
}

extension AppModel {
    /// Why a subscription is in the Gallery: on its own (`""`), through
    /// its folder (the folder's name), or not at all (`nil`).
    func galleryReason(_ feed: SourceView) -> String? {
        if let folder = feed.category, inGallery(folder: folder) { return folder }
        return feed.gallery ? "" : nil
    }
}

extension MediaItemView {
    /// An animated GIF: shown as a picture, but it moves.
    var isGIF: Bool {
        kind == .image && URL(string: src)?.pathExtension.lowercased() == "gif"
    }
}

/// Small labels over a thumbnail: duration, number of pictures, a site
/// that needs signing in, an animated GIF.
struct MediaBadges: View {
    let mediaSet: MediaSetView?

    var body: some View {
        HStack(spacing: 4) {
            if let set = mediaSet {
                switch set.status {
                case .needsSignIn:
                    badge("lock.fill", "Sign in")
                case .failed where set.items.isEmpty:
                    badge("exclamationmark.triangle.fill", nil)
                default:
                    if let first = set.items.first {
                        switch first.kind {
                        case .video: badge("play.fill", first.duration.map(Self.clock))
                        case .embed: badge("play.rectangle.fill", first.provider?.capitalized)
                        case .image: if first.isGIF { badge("play.fill", "GIF") }
                        }
                    }
                    if set.items.count > 1 {
                        badge("square.on.square", "\(set.items.count)")
                    }
                }
            }
        }
    }

    private func badge(_ symbol: String, _ text: String?) -> some View {
        HStack(spacing: 3) {
            Image(systemName: symbol)
            if let text { Text(text).monospacedDigit() }
        }
        .font(.caption2.weight(.semibold))
        .foregroundStyle(.white)
        .padding(.horizontal, 6)
        .padding(.vertical, 3)
        .background(.black.opacity(0.55), in: Capsule())
    }

    static func clock(_ seconds: Double) -> String {
        let s = Int(seconds.rounded())
        return s >= 3600 ? String(format: "%d:%02d:%02d", s / 3600, s / 60 % 60, s % 60) : String(format: "%d:%02d", s / 60, s % 60)
    }
}

/// What the Gallery holds: all of it, marked folders and subscriptions,
/// saved media.
private struct GallerySidebar: View {
    @Environment(AppModel.self) private var model
    @Binding var scopeKey: String
    var focus: FocusState<PaneFocus?>.Binding
    /// Return or Right: on to the grid.
    let enter: () -> Void
    /// Esc.
    let escape: () -> Void

    var body: some View {
        List(selection: Binding<String?>(get: { scopeKey }, set: { if let key = $0 { scopeKey = key } })) {
            SideHeading(title: "Gallery", first: true)
            row(GalleryScopeView.all.key, "photo.on.rectangle.angled", Text("All Media"))
            if model.collection.contains(where: { $0.kind == .media }) {
                row(GalleryScopeView.saved.key, "play.rectangle", Text("Saved Media"), model.galleryUnread(.saved))
            }
            if !model.galleryFolders.isEmpty {
                SideHeading(title: "Folders")
                ForEach(model.galleryFolders, id: \.self) { folder in
                    row(GalleryScopeView.folder(name: folder).key, "folder", Text(verbatim: folder))
                        .contextMenu {
                            Button("Remove from Gallery") { model.setGallery(folder: folder, false) }
                        }
                }
            }
            let feeds = model.galleryFeeds
            if !feeds.isEmpty {
                SideHeading(title: "Subscriptions")
                ForEach(feeds) { feed in
                    row(GalleryScopeView.source(id: feed.id).key, "dot.radiowaves.up.forward", Text(verbatim: feed.title), model.galleryUnread(.source(id: feed.id)))
                        .contextMenu {
                            if feed.gallery {
                                Button("Remove from Gallery") { model.setGallery(source: feed, false) }
                            } else if let folder = feed.category {
                                Button("Remove Folder “\(folder)” from Gallery") { model.setGallery(folder: folder, false) }
                            }
                        }
                }
            }
        }
        .flatList()
        .pane(focus, .sidebar, onReturn: enter, onEscape: escape)
        .onKeyPress(.rightArrow) {
            enter()
            return .handled
        }
    }

    private func row(_ key: String, _ symbol: String, _ title: Text, _ count: Int = 0) -> some View {
        SideRow(symbol: symbol, title: title, count: count, chosen: scopeKey == key)
            .flatRow(chosen: scopeKey == key, active: focus.wrappedValue == .sidebar)
            .tag(key)
    }
}
