import AVKit
import EddyKit
import SwiftUI
import WebKit

/// Shows media over the whole window, one item at a time: pictures that
/// zoom, GIFs that move, videos that play (muted and looping until
/// unmuted), YouTube and Vimeo in their own players.
///
/// Scrolling up and down (in the system's direction, so with natural
/// scrolling the next item comes up from below) and Up and Down step
/// between items. Scrolling sideways and Left and Right go through the
/// pictures of an item, which also show as a strip of thumbnails, or seek
/// in a video.
///
/// The item on screen is named by ID: `entries` may grow underneath the
/// viewer, and items found to have no media stay in it and are stepped
/// over, so the one being looked at never changes by itself.
struct MediaViewer: View {
    @Environment(AppModel.self) private var model
    let entries: [GalleryEntryView]
    @Binding var current: String
    let media: GalleryMedia
    let close: () -> Void

    @AppStorage("viewer.muted") private var muted = true
    @State private var playback = MediaPlayback()
    @State private var scroll = ViewerScroll()
    /// How far the items are dragged up (negative) or down.
    @State private var dragOffset: CGFloat = 0
    @State private var stageHeight: CGFloat = 600
    /// An item is sliding into place; drags and steps wait.
    @State private var sliding = false
    @FocusState private var focused: Bool

    /// Seconds a Left or Right press moves a video.
    private static let seekStep = 5.0

    private var index: Int { entries.firstIndex { $0.id == current } ?? 0 }
    private var entry: GalleryEntryView { entries[index] }
    private var items: [MediaItemView] { media.set(for: entry)?.items ?? [] }
    private var currentItem: MediaItemView? { items.indices.contains(playback.page) ? items[playback.page] : nil }
    private var starred: Bool { model.item(entry.item.id)?.starred ?? entry.item.starred }
    private var scrubs: Bool { items.count <= 1 && currentItem?.kind == .video }

    var body: some View {
        ZStack {
            Color.black.opacity(0.96).ignoresSafeArea()
            ZStack {
                // The items before and after wait just out of view and come
                // in with the drag.
                if dragOffset != 0 {
                    if let before = neighbor(-1) { NeighborPreview(entry: entries[before], media: media).offset(y: dragOffset - stageHeight) }
                    if let after = neighbor(1) { NeighborPreview(entry: entries[after], media: media).offset(y: dragOffset + stageHeight) }
                }
                MediaStage(entry: entry, media: media, playback: playback, muted: $muted, insets: EdgeInsets(top: 44, leading: 0, bottom: items.count > 1 ? 96 : 44, trailing: 0))
                    .offset(y: dragOffset)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .clipped()
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { stageHeight = max($0, 1) }
        }
        .overlay(alignment: .top) { topBar }
        .overlay(alignment: .bottom) { if items.count > 1 { MediaStrip(items: items, page: $playback.page).background(.black.opacity(0.4)) } }
        .overlay(alignment: .trailing) { itemButtons }
        .overlay(alignment: .leading) { if items.count > 1, playback.page > 0 { MediaPageButton(by: -1, turn: turn) } }
        .overlay(alignment: .trailing) { if items.count > 1, playback.page < items.count - 1 { MediaPageButton(by: 1, turn: turn).padding(.trailing, 52) } }
        .background(WindowReader { scroll.window = $0 })
        .environment(\.colorScheme, .dark)
        .focusable()
        .focusEffectDisabled()
        .focused($focused)
        .onAppear {
            focused = true
            scroll.start()
        }
        .onDisappear { scroll.stop() }
        .onKeyPress(.escape) { close(); return .handled }
        .onKeyPress(.upArrow) { step(-1) }
        .onKeyPress(.downArrow) { step(1) }
        .onKeyPress(.leftArrow) { sideways(-1) }
        .onKeyPress(.rightArrow) { sideways(1) }
        .onKeyPress(.space) { playback.playing.toggle(); return .handled }
        .onKeyPress(characters: CharacterSet(charactersIn: "mMsS")) { press in
            if press.characters.lowercased() == "m" { muted.toggle() } else { toggleStar() }
            return .handled
        }
        .onChange(of: scrubs, initial: true) { configureScroll() }
        .onChange(of: entries.map(\.id)) { configureScroll() }
    }

    private var topBar: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 1) {
                Text(entry.item.title).lineLimit(1)
                if let source = entry.item.sourceTitle {
                    Text(source).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            Spacer()
            if items.count > 1 {
                Text("\(playback.page + 1) / \(items.count)").monospacedDigit().foregroundStyle(.secondary)
            }
            if currentItem?.kind == .video {
                Button { muted.toggle() } label: { Image(systemName: muted ? "speaker.slash.fill" : "speaker.wave.2.fill") }
                    .help(muted ? "Unmute (M)" : "Mute (M)")
            }
            Button(action: toggleStar) { Image(systemName: starred ? "star.fill" : "star") }
                .help(starred ? "Unstar (S)" : "Star (S)")
            if let url = entry.item.url.flatMap(URL.init(string:)) {
                Button { NSWorkspace.shared.open(url) } label: { BrowserIcon() }
                    .help("Open in Browser")
                    .accessibilityLabel("Open in Browser")
            }
            Button(action: close) { Image(systemName: "xmark") }.help("Close (Esc)")
        }
        .buttonStyle(.borderless)
        .foregroundStyle(.white)
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(.black.opacity(0.4))
    }

    /// Previous and next item, one above the other.
    private var itemButtons: some View {
        VStack(spacing: 8) {
            Button { _ = step(-1) } label: { Image(systemName: "chevron.up").frame(width: 36, height: 36) }
                .disabled(neighbor(-1) == nil)
                .help("Previous (Up, or scroll up)")
            Button { _ = step(1) } label: { Image(systemName: "chevron.down").frame(width: 36, height: 36) }
                .disabled(neighbor(1) == nil)
                .help("Next (Down, or scroll down)")
        }
        .buttonStyle(.plain)
        .font(.title2)
        .foregroundStyle(.white.opacity(0.7))
        .padding(.trailing, 10)
    }

    /// The nearest item `by` steps away that still has media, if any.
    private func neighbor(_ by: Int) -> Int? {
        var i = index + by
        while entries.indices.contains(i) {
            if media.set(for: entries[i])?.status != .noMedia { return i }
            i += by
        }
        return nil
    }

    private func step(_ by: Int) -> KeyPress.Result {
        slide(by)
        return .handled
    }

    /// The items follow the fingers; past the ends they give a little.
    private func drag(_ travelled: CGFloat) {
        guard !sliding else { return }
        let toward = travelled < 0 ? 1 : -1
        dragOffset = neighbor(toward) != nil ? travelled : travelled * 0.25
    }

    /// Lets go: on to the neighbor when dragged far or flicked, else back.
    /// `speed` is in points per second.
    private func release(_ travelled: CGFloat, speed: CGFloat) {
        guard !sliding else { return }
        let far = abs(travelled) > stageHeight * 0.15
        let flicked = abs(speed) > 600 && abs(travelled) > 30
        if far || flicked {
            slide(travelled < 0 ? 1 : -1)
        } else {
            withAnimation(.spring(duration: 0.3)) { dragOffset = 0 }
        }
    }

    /// The system started scrolling on by itself after the fingers left:
    /// that was a flick, whatever the speed at release looked like. Moves
    /// on if the release didn't already.
    private func momentum(_ travelled: CGFloat) {
        guard !sliding, abs(travelled) > 30 else { return }
        slide(travelled < 0 ? 1 : -1)
    }

    /// Slides the next (`by` 1) or previous item into place, from wherever
    /// a drag left things; at the ends the items spring back.
    private func slide(_ by: Int) {
        guard !sliding else { return }
        guard let target = neighbor(by) else {
            withAnimation(.spring(duration: 0.3)) { dragOffset = 0 }
            return
        }
        sliding = true
        let id = entries[target].id
        withAnimation(.easeOut(duration: 0.22)) {
            dragOffset = by > 0 ? -stageHeight : stageHeight
        } completion: {
            var still = Transaction()
            still.disablesAnimations = true
            withTransaction(still) {
                current = id
                dragOffset = 0
            }
            sliding = false
        }
    }

    private func turn(_ by: Int) {
        playback.turn(by, of: items.count)
    }

    /// Left and Right: through the pictures, else seeking a video, else to
    /// the next item, so a single picture can still be left.
    private func sideways(_ by: Int) -> KeyPress.Result {
        if items.count > 1 {
            turn(by)
        } else if currentItem?.kind == .video {
            playback.seek = playback.seek.by(Double(by) * Self.seekStep)
        } else {
            _ = step(by)
        }
        return .handled
    }

    private func configureScroll() {
        scroll.onVertical = { slide($0) }
        scroll.onDrag = { drag($0) }
        scroll.onRelease = { release($0, speed: $1) }
        scroll.onMomentum = { momentum($0) }
        scroll.onSideways = { turn($0) }
        scroll.onScrub = scrubs ? { playback.seek = playback.seek.by($0) } : nil
    }

    private func toggleStar() {
        model.setStarred(model.item(entry.item.id) ?? entry.item, !starred)
    }
}

/// What is going on with the item on screen: which of its pictures shows,
/// whether its video plays, a seek asked for. Shared by the stage and the
/// controls around it.
@Observable
@MainActor
final class MediaPlayback {
    var page = 0
    var playing = true
    var seek = SeekRequest()
    var resolving = false

    func turn(_ by: Int, of count: Int) {
        let target = page + by
        if (0..<count).contains(target) { page = target }
    }
}

/// One item's media, as the full-window viewer and the reader's media pane
/// both show it: the picture or player, or why there is none. Resolves the
/// item's media when it isn't known yet.
struct MediaStage: View {
    @Environment(AppModel.self) private var model
    let entry: GalleryEntryView
    let media: GalleryMedia
    let playback: MediaPlayback
    @Binding var muted: Bool
    /// Room left for the controls laid over the stage.
    var insets = EdgeInsets()

    private var mediaSet: MediaSetView? { media.set(for: entry) }
    private var items: [MediaItemView] { mediaSet?.items ?? [] }
    private var current: MediaItemView? { items.indices.contains(playback.page) ? items[playback.page] : nil }

    var body: some View {
        @Bindable var playback = playback
        Group {
            if let current {
                MediaItemPlayer(item: current, muted: $muted, playing: $playback.playing, seek: playback.seek)
                    .id("\(entry.id)#\(playback.page)")
                    .padding(insets)
            } else if playback.resolving || mediaSet == nil {
                ProgressView().controlSize(.large)
            } else if mediaSet?.status == .needsSignIn {
                message("lock", "This post is only shown to signed-in visitors.") {
                    Button("Sign In…") {
                        model.showSettings(.accounts)
                    }
                    Button("Try Again") { Task { await retry() } }
                }
            } else if mediaSet?.status == .failed {
                message("exclamationmark.triangle", "The site didn’t answer.") {
                    Button("Try Again") { Task { await retry() } }
                }
            } else {
                message("photo", "Nothing to show here.") { EmptyView() }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task(id: entry.id) {
            playback.page = 0
            playback.playing = true
            model.markOpened(entry.item.id)
            guard entry.media == nil || mediaSet?.status == .failed else { return }
            playback.resolving = true
            await media.resolve(entry.item.id)
            playback.resolving = false
        }
    }

    private func message<Actions: View>(_ symbol: String, _ text: LocalizedStringKey, @ViewBuilder actions: () -> Actions) -> some View {
        VStack(spacing: 14) {
            Image(systemName: symbol).font(.largeTitle).foregroundStyle(.secondary)
            Text(text).foregroundStyle(.secondary)
            HStack { actions() }
            if let url = entry.item.url.flatMap(URL.init(string:)) {
                Link("Open in Browser", destination: url)
            }
        }
    }

    private func retry() async {
        playback.resolving = true
        await media.resolve(entry.item.id, force: true)
        playback.resolving = false
    }
}

/// The pictures of an item as a row of thumbnails; the one shown is
/// outlined, and clicking one shows it.
struct MediaStrip: View {
    let items: [MediaItemView]
    @Binding var page: Int

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(items.indices, id: \.self) { i in
                        StripThumbnail(item: items[i], selected: i == page)
                            .id(i)
                            .onTapGesture { page = i }
                            .accessibilityAddTraits(.isButton)
                            .accessibilityLabel(Text("Picture \(i + 1)"))
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                .frame(minWidth: 0)
            }
            .frame(maxWidth: .infinity)
            .onChange(of: page, initial: true) { _, now in
                withAnimation(.easeInOut(duration: 0.15)) { proxy.scrollTo(now, anchor: .center) }
            }
        }
    }
}

/// A chevron at the side of the stage that turns to the previous (`by`
/// -1) or next picture.
struct MediaPageButton: View {
    let by: Int
    let turn: (Int) -> Void

    var body: some View {
        Button { turn(by) } label: {
            Image(systemName: by < 0 ? "chevron.left" : "chevron.right").font(.title).foregroundStyle(.white.opacity(0.7)).frame(width: 44, height: 88)
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 6)
    }
}

/// A relative seek for the video on screen; each new one gets a new token.
struct SeekRequest: Equatable {
    var seconds = 0.0
    var token = 0

    func by(_ seconds: Double) -> SeekRequest {
        SeekRequest(seconds: seconds, token: token + 1)
    }
}

/// Scrolling over the viewer. Up and down on a trackpad drags the items
/// (`onDrag` with the distance so far, in the direction content moves) and
/// `onRelease` decides where they land, like flicking through short
/// videos; the gesture's momentum is swallowed, so one flick is one item,
/// but its arrival is reported (`onMomentum`), since the system only sends
/// it after a flick. A mouse wheel steps a notch at a time. Sideways
/// scrolling steps through pictures, or scrubs a video.
@MainActor
final class ViewerScroll {
    weak var window: NSWindow?
    var onDrag: (CGFloat) -> Void = { _ in }
    /// Distance travelled and the speed at the end, in points per second.
    var onRelease: (CGFloat, CGFloat) -> Void = { _, _ in }
    /// The system began scrolling on after the fingers left a vertical
    /// drag, with the distance it had travelled.
    var onMomentum: (CGFloat) -> Void = { _ in }
    /// A mouse wheel notch.
    var onVertical: (Int) -> Void = { _ in }
    var onSideways: (Int) -> Void = { _ in }
    /// Seconds to seek by; set when sideways scrolling scrubs.
    var onScrub: ((Double) -> Void)?

    private var monitor: Any?
    private var travelled = CGSize.zero
    private var axis: Axis?
    private var stepped = false
    /// A vertical drag has begun and not been released.
    private var dragging = false
    /// Vertical movement of the last moments, for the speed at release.
    private var recent: [(time: TimeInterval, delta: CGFloat)] = []
    private var lastWheelStep = Date.distantPast

    /// Points a trackpad gesture travels before it steps.
    private static let threshold: CGFloat = 40
    /// Video seconds per point of sideways travel.
    private static let scrubRate = 0.1
    /// How far back the speed at release looks.
    private static let speedWindow: TimeInterval = 0.1

    func start() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
            MainActor.assumeIsolated {
                guard let self else { return event }
                return self.handle(event)
            }
        }
    }

    func stop() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
    }

    private func handle(_ event: NSEvent) -> NSEvent? {
        guard let window, event.window === window else { return event }
        if !event.momentumPhase.isEmpty {
            if event.momentumPhase.contains(.began), axis == .vertical {
                // The release has been reported; one momentum per gesture.
                axis = nil
                onMomentum(travelled.height)
            }
            return nil
        }
        guard event.hasPreciseScrollingDeltas else {
            wheel(event)
            return nil
        }
        if event.phase.contains(.began) {
            // A new gesture before the last was let go of (its end was
            // lost): settle the last first, so nothing is left half-way.
            if dragging { finishDrag() }
            travelled = .zero
            axis = nil
            stepped = false
            recent = []
        }
        travelled.width += event.scrollingDeltaX
        travelled.height += event.scrollingDeltaY
        if axis == nil, max(abs(travelled.width), abs(travelled.height)) > 4 {
            axis = abs(travelled.width) > abs(travelled.height) ? .horizontal : .vertical
        }
        let ended = event.phase.contains(.ended) || event.phase.contains(.cancelled)
        switch axis {
        case .vertical:
            dragging = true
            if event.scrollingDeltaY != 0 {
                recent.append((event.timestamp, event.scrollingDeltaY))
                recent.removeAll { event.timestamp - $0.time > Self.speedWindow }
            }
            onDrag(travelled.height)
            if ended { finishDrag() }
        case .horizontal:
            if let onScrub {
                onScrub(-Double(event.scrollingDeltaX) * Self.scrubRate)
            } else if !stepped, abs(travelled.width) > Self.threshold {
                stepped = true
                onSideways(travelled.width < 0 ? 1 : -1)
            }
            if ended { axis = nil }
        case nil:
            break
        }
        return nil
    }

    /// Reports the release of a vertical drag with its speed: the distance
    /// of the last moments over the time they took.
    private func finishDrag() {
        dragging = false
        var speed: CGFloat = 0
        if let first = recent.first, let last = recent.last, last.time > first.time {
            speed = recent.dropFirst().reduce(0) { $0 + $1.delta } / CGFloat(last.time - first.time)
        }
        onRelease(travelled.height, speed)
        // `axis` stays vertical until momentum arrives, or the next gesture.
    }

    private func wheel(_ event: NSEvent) {
        let (dx, dy) = (event.scrollingDeltaX, event.scrollingDeltaY)
        if abs(dx) > abs(dy), let onScrub {
            onScrub(-Double(dx) * 2)
            return
        }
        guard Date().timeIntervalSince(lastWheelStep) > 0.3, max(abs(dx), abs(dy)) > 0 else { return }
        lastWheelStep = Date()
        if abs(dy) >= abs(dx) {
            onVertical(dy < 0 ? 1 : -1)
        } else {
            onSideways(dx < 0 ? 1 : -1)
        }
    }
}

/// Hands the window a view is in to `found`.
private struct WindowReader: NSViewRepresentable {
    let found: (NSWindow?) -> Void

    func makeNSView(context: Context) -> NSView { Probe(found: found) }
    func updateNSView(_ view: NSView, context: Context) {}

    private final class Probe: NSView {
        let found: (NSWindow?) -> Void
        init(found: @escaping (NSWindow?) -> Void) {
            self.found = found
            super.init(frame: .zero)
        }

        required init?(coder: NSCoder) { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            found(window)
        }
    }
}

/// The item above or below, as its picture, while the items are dragged.
private struct NeighborPreview: View {
    @Environment(AppModel.self) private var model
    let entry: GalleryEntryView
    let media: GalleryMedia
    @State private var image: NSImage?

    private var src: String? {
        let set = media.set(for: entry)
        return set?.items.first?.preview ?? set?.thumbnail ?? entry.thumbnail
    }

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image).resizable().scaledToFit()
            } else {
                Color.clear
            }
        }
        .padding(.vertical, 44)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task(id: src) {
            if let src { image = await model.image(src) }
        }
    }
}

/// One picture of an item in the viewer's strip.
private struct StripThumbnail: View {
    @Environment(AppModel.self) private var model
    let item: MediaItemView
    let selected: Bool
    @State private var image: NSImage?

    var body: some View {
        RoundedRectangle(cornerRadius: 4)
            .fill(.white.opacity(0.08))
            .overlay {
                if let image { Image(nsImage: image).resizable().scaledToFill() }
            }
            .frame(width: 64, height: 64)
            .clipShape(RoundedRectangle(cornerRadius: 4))
            .overlay {
                RoundedRectangle(cornerRadius: 4).strokeBorder(selected ? Color.white : .clear, lineWidth: 2)
            }
            .opacity(selected ? 1 : 0.6)
            .task(id: item.src) {
                if let src = item.preview { image = await model.image(src) }
            }
    }
}

extension MediaItemView {
    /// The picture that stands for it.
    var preview: String? { kind == .image ? src : poster }
}

/// One picture, video or embedded player.
struct MediaItemPlayer: View {
    @Environment(AppModel.self) private var model
    let item: MediaItemView
    @Binding var muted: Bool
    @Binding var playing: Bool
    var seek = SeekRequest()

    var body: some View {
        switch item.kind {
        case .image:
            ZoomableImage(src: item.src, animated: item.isGIF)
        case .video:
            if let url = URL(string: item.src) {
                VideoPlayerView(url: url, muted: muted, playing: playing, seek: seek)
            }
        case .embed:
            if let url = URL(string: item.src) {
                EmbedPlayerView(url: url)
                    .aspectRatio(16 / 9, contentMode: .fit)
                    .padding(.horizontal, 60)
            }
        }
    }
}

/// A picture that fits the window; pinch or double-click to zoom, drag to
/// look around. An animated GIF plays.
struct ZoomableImage: View {
    @Environment(AppModel.self) private var model
    let src: String
    var animated = false
    @State private var image: NSImage?
    @State private var scale: CGFloat = 1
    @State private var settled: CGFloat = 1
    @State private var offset: CGSize = .zero
    @State private var dragged: CGSize = .zero

    var body: some View {
        Group {
            if let image {
                picture(image)
                    .scaleEffect(scale)
                    .offset(offset)
                    .gesture(MagnifyGesture()
                        .onChanged { scale = max(1, settled * $0.magnification) }
                        .onEnded { _ in
                            settled = scale
                            if scale == 1 { offset = .zero; dragged = .zero }
                        })
                    .simultaneousGesture(DragGesture()
                        .onChanged { value in
                            guard scale > 1 else { return }
                            offset = CGSize(width: dragged.width + value.translation.width, height: dragged.height + value.translation.height)
                        }
                        .onEnded { _ in dragged = offset })
                    .onTapGesture(count: 2) {
                        withAnimation(.easeInOut(duration: 0.2)) {
                            scale = scale > 1 ? 1 : 2.5
                            settled = scale
                            if scale == 1 { offset = .zero; dragged = .zero }
                        }
                    }
            } else {
                ProgressView().controlSize(.large)
            }
        }
        .task(id: src) { image = animated ? await model.animatedImage(src) : await model.image(src) }
    }

    @ViewBuilder
    private func picture(_ image: NSImage) -> some View {
        if animated {
            AnimatedImageView(image: image)
                .aspectRatio(image.size.width / max(image.size.height, 1), contentMode: .fit)
        } else {
            Image(nsImage: image).resizable().scaledToFit()
        }
    }
}

/// Plays an animated image; SwiftUI's `Image` shows only its first frame.
private struct AnimatedImageView: NSViewRepresentable {
    let image: NSImage

    func makeNSView(context: Context) -> NSImageView {
        let view = NSImageView()
        view.animates = true
        view.imageScaling = .scaleProportionallyUpOrDown
        view.setContentHuggingPriority(.defaultLow, for: .horizontal)
        view.setContentHuggingPriority(.defaultLow, for: .vertical)
        view.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        view.setContentCompressionResistancePriority(.defaultLow, for: .vertical)
        view.image = image
        return view
    }

    func updateNSView(_ view: NSImageView, context: Context) {
        if view.image !== image { view.image = image }
    }
}

/// Plays a video file or HLS stream in a loop.
struct VideoPlayerView: NSViewRepresentable {
    let url: URL
    let muted: Bool
    let playing: Bool
    var seek = SeekRequest()

    final class Coordinator {
        var looper: AVPlayerLooper?
        var seekToken = 0
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> AVPlayerView {
        let player = AVQueuePlayer()
        context.coordinator.looper = AVPlayerLooper(player: player, templateItem: AVPlayerItem(url: url))
        context.coordinator.seekToken = seek.token
        player.isMuted = muted
        let view = AVPlayerView()
        view.player = player
        view.controlsStyle = .floating
        view.videoGravity = .resizeAspect
        if playing { player.play() }
        return view
    }

    func updateNSView(_ view: AVPlayerView, context: Context) {
        guard let player = view.player else { return }
        player.isMuted = muted
        if playing, player.timeControlStatus == .paused { player.play() }
        if !playing, player.timeControlStatus != .paused { player.pause() }
        if seek.token != context.coordinator.seekToken {
            context.coordinator.seekToken = seek.token
            Self.seek(player, by: seek.seconds)
        }
    }

    /// Within the current loop of the clip.
    private static func seek(_ player: AVPlayer, by seconds: Double) {
        guard let item = player.currentItem else { return }
        let duration = item.duration.seconds
        var target = player.currentTime().seconds + seconds
        if duration.isFinite, duration > 0 { target = min(max(0, target), duration - 0.05) } else { target = max(0, target) }
        let tolerance = CMTime(seconds: 0.1, preferredTimescale: 600)
        player.seek(to: CMTime(seconds: target, preferredTimescale: 600), toleranceBefore: tolerance, toleranceAfter: tolerance)
    }

    static func dismantleNSView(_ view: AVPlayerView, coordinator: Coordinator) {
        view.player?.pause()
        coordinator.looper?.disableLooping()
        view.player = nil
    }
}

/// A third-party player page (YouTube, Vimeo). It is framed by a page of
/// Eddy's own so the player sees an embedding site, as it expects; the
/// default website data store carries the user's sign-ins.
struct EmbedPlayerView: NSViewRepresentable {
    let url: URL

    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        config.mediaTypesRequiringUserActionForPlayback = []
        let view = WKWebView(frame: .zero, configuration: config)
        view.setValue(false, forKey: "drawsBackground")
        let html = """
        <!doctype html><html><head><meta name="viewport" content="width=device-width">
        <style>html,body{margin:0;height:100%;background:#000}iframe{border:0;width:100%;height:100%}</style></head>
        <body><iframe src="\(url.absoluteString)" allow="autoplay; encrypted-media; picture-in-picture; fullscreen" allowfullscreen></iframe></body></html>
        """
        view.loadHTMLString(html, baseURL: URL(string: "https://eddy.local/"))
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {}

    static func dismantleNSView(_ view: WKWebView, coordinator: ()) {
        view.loadHTMLString("", baseURL: nil)
    }
}
