import AVKit
import EddyKit
import SwiftUI
import WebKit

/// Shows media over the whole window, one item at a time: pictures that
/// zoom, videos that play (muted and looping until unmuted), YouTube and
/// Vimeo in their own players. Gallery items step with Left and Right, the
/// pictures of one item with Up and Down.
struct MediaViewer: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openSettings) private var openSettings
    let entries: [GalleryEntryView]
    @Binding var index: Int
    let media: GalleryMedia
    let close: () -> Void

    @AppStorage("viewer.muted") private var muted = true
    @AppStorage("settings.tab") private var settingsTab: SettingsTab = .accounts
    @State private var page = 0
    @State private var playing = true
    @State private var resolving = false
    @FocusState private var focused: Bool

    private var entry: GalleryEntryView { entries[index] }
    private var mediaSet: MediaSetView? { media.set(for: entry) }
    private var items: [MediaItemView] { mediaSet?.items ?? [] }
    private var current: MediaItemView? { items.indices.contains(page) ? items[page] : nil }
    private var starred: Bool { model.item(entry.item.id)?.starred ?? entry.item.starred }

    var body: some View {
        ZStack {
            Color.black.opacity(0.96).ignoresSafeArea()
            stage
        }
        .overlay(alignment: .top) { topBar }
        .overlay(alignment: .leading) { if index > 0 { stepButton(-1, "chevron.left") } }
        .overlay(alignment: .trailing) { if index < entries.count - 1 { stepButton(1, "chevron.right") } }
        .environment(\.colorScheme, .dark)
        .focusable()
        .focusEffectDisabled()
        .focused($focused)
        .onAppear { focused = true }
        .onKeyPress(.escape) { close(); return .handled }
        .onKeyPress(.leftArrow) { step(-1) }
        .onKeyPress(.rightArrow) { step(1) }
        .onKeyPress(.upArrow) { turn(-1) }
        .onKeyPress(.downArrow) { turn(1) }
        .onKeyPress(.space) { playing.toggle(); return .handled }
        .onKeyPress(characters: CharacterSet(charactersIn: "mMsS")) { press in
            if press.characters.lowercased() == "m" { muted.toggle() } else { toggleStar() }
            return .handled
        }
        .task(id: entry.id) {
            page = 0
            playing = true
            model.markOpened(entry.item.id)
            guard entry.media == nil || mediaSet?.status == .failed else { return }
            resolving = true
            await media.resolve(entry.item.id)
            resolving = false
        }
    }

    @ViewBuilder
    private var stage: some View {
        if let current {
            MediaItemPlayer(item: current, muted: $muted, playing: $playing)
                .id("\(entry.id)#\(page)")
                .padding(.vertical, 44)
        } else if resolving || mediaSet == nil {
            ProgressView().controlSize(.large)
        } else if mediaSet?.status == .needsSignIn {
            message("lock", "This post is only shown to signed-in visitors.") {
                Button("Sign In…") {
                    settingsTab = .accounts
                    openSettings()
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
                Text("\(page + 1) / \(items.count)").monospacedDigit().foregroundStyle(.secondary)
            }
            if current?.kind == .video {
                Button { muted.toggle() } label: { Image(systemName: muted ? "speaker.slash.fill" : "speaker.wave.2.fill") }
                    .help(muted ? "Unmute (M)" : "Mute (M)")
            }
            Button(action: toggleStar) { Image(systemName: starred ? "star.fill" : "star") }
                .help(starred ? "Unstar (S)" : "Star (S)")
            if let url = entry.item.url.flatMap(URL.init(string:)) {
                Button { NSWorkspace.shared.open(url) } label: { Image(systemName: "arrow.up.forward.square") }
                    .help("Open in Browser")
            }
            Button(action: close) { Image(systemName: "xmark") }.help("Close (Esc)")
        }
        .buttonStyle(.borderless)
        .foregroundStyle(.white)
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(.black.opacity(0.4))
    }

    private func stepButton(_ by: Int, _ symbol: String) -> some View {
        Button { _ = step(by) } label: {
            Image(systemName: symbol).font(.title).foregroundStyle(.white.opacity(0.7)).frame(width: 44, height: 88)
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 6)
    }

    private func step(_ by: Int) -> KeyPress.Result {
        let target = index + by
        if entries.indices.contains(target) { index = target }
        return .handled
    }

    private func turn(_ by: Int) -> KeyPress.Result {
        let target = page + by
        if items.indices.contains(target) { page = target }
        return .handled
    }

    private func toggleStar() {
        model.setStarred(model.item(entry.item.id) ?? entry.item, !starred)
    }

    private func retry() async {
        resolving = true
        await media.resolve(entry.item.id, force: true)
        resolving = false
    }
}

/// One picture, video or embedded player.
struct MediaItemPlayer: View {
    @Environment(AppModel.self) private var model
    let item: MediaItemView
    @Binding var muted: Bool
    @Binding var playing: Bool

    var body: some View {
        switch item.kind {
        case .image:
            ZoomableImage(src: item.src)
        case .video:
            if let url = URL(string: item.src) {
                VideoPlayerView(url: url, muted: muted, playing: playing)
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
/// look around.
struct ZoomableImage: View {
    @Environment(AppModel.self) private var model
    let src: String
    @State private var image: NSImage?
    @State private var scale: CGFloat = 1
    @State private var settled: CGFloat = 1
    @State private var offset: CGSize = .zero
    @State private var dragged: CGSize = .zero

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image)
                    .resizable()
                    .scaledToFit()
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
        .task(id: src) { image = await model.image(src) }
    }
}

/// Plays a video file or HLS stream in a loop.
struct VideoPlayerView: NSViewRepresentable {
    let url: URL
    let muted: Bool
    let playing: Bool

    final class Coordinator {
        var looper: AVPlayerLooper?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> AVPlayerView {
        let player = AVQueuePlayer()
        context.coordinator.looper = AVPlayerLooper(player: player, templateItem: AVPlayerItem(url: url))
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
