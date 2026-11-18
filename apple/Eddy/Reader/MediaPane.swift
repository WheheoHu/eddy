import EddyKit
import SwiftUI

/// The reader's view of a Gallery item: its pictures and videos played in
/// place, in the pane an article would take, so a subscription shown in
/// the Gallery reads like one in Three-Pane too. Space plays and pauses,
/// M mutes, Left and Right turn pictures or seek, Up and Down go to the
/// items around it, Esc leaves for the list. The full-window viewer is a
/// toolbar button away.
///
/// Scrolling isn't taken over here, unlike the viewer: the list beside
/// the pane still has to scroll.
struct MediaPane: View {
    @Environment(AppModel.self) private var model
    let item: ItemView
    var neighbors: Neighbors?
    var onEscape: (() -> Void)?
    /// Keyboard focus belongs in the pane.
    var focused = false
    var onFocus: (() -> Void)?

    @AppStorage("viewer.muted") private var muted = true
    @State private var playback = MediaPlayback()
    @FocusState private var hasFocus: Bool

    private static let seekStep = 5.0

    private var entry: GalleryEntryView { GalleryEntryView(item: item, media: nil, thumbnail: nil) }
    private var items: [MediaItemView] { model.media.set(for: entry)?.items ?? [] }
    private var current: MediaItemView? { items.indices.contains(playback.page) ? items[playback.page] : nil }

    var body: some View {
        ZStack {
            Color.black
            MediaStage(entry: entry, media: model.media, playback: playback, muted: $muted, insets: EdgeInsets(top: 12, leading: 12, bottom: items.count > 1 ? 96 : 12, trailing: 12))
        }
        .overlay(alignment: .bottom) { if items.count > 1 { MediaStrip(items: items, page: $playback.page).background(.black.opacity(0.4)) } }
        .overlay(alignment: .leading) { if items.count > 1, playback.page > 0 { MediaPageButton(by: -1, turn: turn) } }
        .overlay(alignment: .trailing) { if items.count > 1, playback.page < items.count - 1 { MediaPageButton(by: 1, turn: turn) } }
        .overlay(alignment: .topTrailing) {
            if items.count > 1 {
                Text("\(playback.page + 1) / \(items.count)")
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.white.opacity(0.8))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(.black.opacity(0.5), in: Capsule())
                    .padding(10)
            }
        }
        .environment(\.colorScheme, .dark)
        .focusable()
        .focusEffectDisabled()
        .focused($hasFocus)
        .simultaneousGesture(TapGesture().onEnded { hasFocus = true })
        .onAppear { if focused { hasFocus = true } }
        .onChange(of: focused) { _, now in if now { hasFocus = true } }
        .onChange(of: hasFocus) { _, now in if now { onFocus?() } }
        .onKeyPress(.escape) { onEscape?(); return .handled }
        .onKeyPress(.upArrow) { go(.previous) }
        .onKeyPress(.downArrow) { go(.next) }
        .onKeyPress(.leftArrow) { sideways(-1) }
        .onKeyPress(.rightArrow) { sideways(1) }
        .onKeyPress(.space) { playback.playing.toggle(); return .handled }
        .onKeyPress(characters: CharacterSet(charactersIn: "mM")) { _ in muted.toggle(); return .handled }
    }

    private func turn(_ by: Int) {
        playback.turn(by, of: items.count)
    }

    private func go(_ direction: Direction) -> KeyPress.Result {
        guard let neighbors else { return .ignored }
        neighbors.go(direction)
        return .handled
    }

    /// Left and Right: through the pictures, else seeking a video.
    private func sideways(_ by: Int) -> KeyPress.Result {
        if items.count > 1 {
            turn(by)
        } else if current?.kind == .video {
            playback.seek = playback.seek.by(Double(by) * Self.seekStep)
        } else {
            return .ignored
        }
        return .handled
    }
}
