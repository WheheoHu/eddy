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

    func resolve(_ itemID: String, force: Bool = false) async {
        guard force || sets[itemID] == nil else { return }
        if let set = await model.resolveMedia(itemID, force: force) { sets[itemID] = set }
    }

    /// Resolves what is about to scroll into view, in the background.
    func prefetch(_ entries: some Sequence<GalleryEntryView>) {
        let ids = entries.filter { $0.media == nil && sets[$0.item.id] == nil && prefetched.insert($0.item.id).inserted }.map(\.item.id)
        model.prefetchMedia(ids)
    }

    /// Width over height of thumbnails as they load, for items whose media
    /// doesn't say.
    private(set) var loadedAspects: [String: CGFloat] = [:]

    func noteAspect(_ aspect: CGFloat, for itemID: String) {
        guard aspect.isFinite, aspect > 0, loadedAspects[itemID] == nil else { return }
        loadedAspects[itemID] = aspect
    }

    /// The shape an item takes in the grid: its media's own size, players
    /// at 16:9, else its thumbnail once loaded; square until known.
    func aspect(for entry: GalleryEntryView) -> CGFloat {
        let first = set(for: entry)?.items.first
        let known: CGFloat? = if let w = first?.width, let h = first?.height, w > 0, h > 0 {
            CGFloat(w) / CGFloat(h)
        } else if first?.kind == .embed {
            16 / 9
        } else {
            loadedAspects[entry.item.id]
        }
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

/// Gallery mode: media from marked subscriptions and folders and from
/// saved media links, as a grid; a tile opens the media viewer.
struct GalleryView: View {
    @Environment(AppModel.self) private var model
    @AppStorage("app.mode") private var mode: AppMode = .threePane
    @AppStorage("gallery.sidebar") private var showSidebar = true
    @AppStorage("gallery.unreadOnly") private var unreadOnly = false
    @AppStorage("gallery.scope") private var scopeKey = "all"
    @State private var entries: [GalleryEntryView] = []
    @State private var exhausted = false
    @State private var media: GalleryMedia?
    /// Index in `visible` of the item open in the viewer.
    @State private var viewing: Int?
    /// Lists changed while the viewer was open; reload when it closes so
    /// the items being looked at don't move.
    @State private var stale = false

    @State private var gridWidth: CGFloat = 0

    private static let pageSize = 200
    private static let prefetchAhead = 20
    private static let rowHeight: CGFloat = 220
    private static let spacing: CGFloat = 6
    private static let gridPadding: CGFloat = 8

    private var scope: GalleryScopeView { GalleryScopeView(key: scopeKey) }

    /// Items found to have no media drop out as soon as that is known.
    private var visible: [GalleryEntryView] {
        guard let media else { return entries }
        return entries.filter { media.set(for: $0)?.status != .noMedia }
    }

    var body: some View {
        LeadingColumn(visible: showSidebar, width: 220) {
            GallerySidebar(scopeKey: $scopeKey)
        } content: {
            grid
        }
        .toolbar {
            ColumnToggle(visible: $showSidebar)
            ModeToolbar(mode: $mode, model: model)
            ToolbarItem {
                Toggle(isOn: $unreadOnly) { Label("Unread Only", systemImage: unreadOnly ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle") }
                    .help("Show only media you haven’t opened")
            }
            ToolbarItem {
                Button { Task { await model.refresh() } } label: { Label("Refresh", systemImage: "arrow.clockwise") }.disabled(model.busy)
            }
        }
        .toolbar(removing: .title)
        .overlay {
            if let index = viewing, let media, visible.indices.contains(index) {
                MediaViewer(entries: visible, index: Binding(get: { index }, set: { viewing = $0 }), media: media) {
                    viewing = nil
                }
            }
        }
        .onAppear {
            if media == nil { media = GalleryMedia(model: model) }
            load()
        }
        .onChange(of: scopeKey) { load(reset: true) }
        .onChange(of: unreadOnly) { load(reset: true) }
        .onChange(of: model.revision) {
            if viewing == nil { load() } else { stale = true }
        }
        .onChange(of: viewing) { _, now in
            if now == nil, stale {
                stale = false
                load()
            }
        }
        .onChange(of: model.navigation) { _, request in
            guard case let .gallery(destination) = request?.value else { return }
            model.navigation = nil
            scopeKey = destination.key
        }
    }

    @ViewBuilder
    private var grid: some View {
        let items = visible
        if items.isEmpty {
            if model.galleryFeeds.isEmpty && !model.collection.contains(where: { $0.kind == .media }) {
                ContentUnavailableView("No Media Yet", systemImage: "photo.on.rectangle.angled", description: Text("In Three-Pane, right-click a subscription or folder and choose Show in Gallery. Saved YouTube, RedGIFs and image links appear here too."))
            } else {
                ContentUnavailableView(unreadOnly ? "Nothing Unread" : "Nothing Here", systemImage: "photo.on.rectangle")
            }
        } else if let media {
            let rows = JustifiedRow.rows(items, width: gridWidth - 2 * Self.gridPadding, targetHeight: Self.rowHeight, spacing: Self.spacing, aspect: media.aspect(for:))
            ScrollView {
                LazyVStack(alignment: .leading, spacing: Self.spacing) {
                    ForEach(rows) { row in
                        HStack(spacing: Self.spacing) {
                            ForEach(row.cells, id: \.entry.id) { cell in
                                tile(cell, in: items, media: media)
                                    .frame(width: cell.width, height: row.height)
                            }
                        }
                    }
                }
                .padding(Self.gridPadding)
            }
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { gridWidth = $0 }
        }
    }

    private func tile(_ cell: JustifiedRow.Cell, in items: [GalleryEntryView], media: GalleryMedia) -> some View {
        let (entry, index) = (cell.entry, cell.index)
        return GalleryTile(entry: entry, mediaSet: media.set(for: entry)) { aspect in
            media.noteAspect(aspect, for: entry.item.id)
        }
        .onTapGesture { open(index) }
        .contextMenu { menu(for: entry.item) }
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(Text(entry.item.title))
        .task(id: entry.id) {
            media.prefetch(items[min(index + 1, items.count)..<min(index + 1 + Self.prefetchAhead, items.count)])
            if index >= items.count - Self.prefetchAhead { loadMore() }
            await media.resolve(entry.item.id)
        }
    }

    @ViewBuilder
    private func menu(for item: ItemView) -> some View {
        Button(item.starred ? "Unstar" : "Star") { model.setStarred(item, !item.starred) }
        if item.read {
            Button("Mark as Unread") { model.markUnread([item.id]) }
        } else {
            Button("Mark as Read") { model.setRead(item, true) }
        }
        if let url = item.url.flatMap(URL.init(string:)) {
            Button("Open in Browser") { NSWorkspace.shared.open(url) }
            Button("Copy Link") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(url.absoluteString, forType: .string)
            }
        }
    }

    private func open(_ index: Int) {
        viewing = index
    }

    /// Reloads the list, keeping as many items as are loaded unless `reset`.
    private func load(reset: Bool = false) {
        let count = reset ? Self.pageSize : max(Self.pageSize, entries.count)
        entries = model.galleryItems(scope, unreadOnly: unreadOnly, offset: 0, limit: count)
        exhausted = entries.count < count
    }

    private func loadMore() {
        guard !exhausted else { return }
        let more = model.galleryItems(scope, unreadOnly: unreadOnly, offset: entries.count, limit: Self.pageSize)
        exhausted = more.count < Self.pageSize
        let known = Set(entries.map(\.id))
        entries.append(contentsOf: more.filter { !known.contains($0.id) })
    }
}

/// One item in the grid: its thumbnail with what kind of media it is.
struct GalleryTile: View {
    @Environment(AppModel.self) private var model
    let entry: GalleryEntryView
    let mediaSet: MediaSetView?
    /// The thumbnail's width over height, once loaded.
    var onAspect: (CGFloat) -> Void = { _ in }
    @State private var image: NSImage?

    private var thumbnail: String? { mediaSet?.thumbnail ?? entry.thumbnail }

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
            .contentShape(RoundedRectangle(cornerRadius: 6))
            .overlay(alignment: .bottomLeading) { MediaBadges(mediaSet: mediaSet).padding(6) }
            .overlay(alignment: .topTrailing) {
                if !entry.item.read {
                    Circle().fill(.tint).frame(width: 8, height: 8).padding(7).accessibilityLabel("Unread")
                }
            }
            .help(entry.item.title)
            .task(id: thumbnail) {
                guard let thumbnail else { return }
                image = await model.image(thumbnail)
                if let size = image?.size, size.height > 0 { onAspect(size.width / size.height) }
            }
    }
}

/// Small labels over a thumbnail: duration, number of pictures, a site
/// that needs signing in.
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
                        case .image: EmptyView()
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

    var body: some View {
        List(selection: Binding<String?>(get: { scopeKey }, set: { if let key = $0 { scopeKey = key } })) {
            Section("Gallery") {
                Label("All Media", systemImage: "photo.on.rectangle.angled")
                    .badge(model.galleryCount(.all))
                    .tag(GalleryScopeView.all.key)
                if model.collection.contains(where: { $0.kind == .media }) {
                    Label("Saved Media", systemImage: "play.rectangle").tag(GalleryScopeView.saved.key)
                }
            }
            if !model.galleryFolders.isEmpty {
                Section("Folders") {
                    ForEach(model.galleryFolders, id: \.self) { folder in
                        Label(folder, systemImage: "folder")
                            .tag(GalleryScopeView.folder(name: folder).key)
                            .contextMenu {
                                Button("Remove from Gallery") { model.setGallery(folder: folder, false) }
                            }
                    }
                }
            }
            let feeds = model.galleryFeeds
            if !feeds.isEmpty {
                Section("Subscriptions") {
                    ForEach(feeds) { feed in
                        Label(feed.title, systemImage: "dot.radiowaves.up.forward")
                            .tag(GalleryScopeView.source(id: feed.id).key)
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
        }
        .listStyle(.sidebar)
    }
}
