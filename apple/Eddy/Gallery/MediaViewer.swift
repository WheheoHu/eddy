import EddyKit
import SwiftUI

/// Shows Gallery items over the whole window, one at a time.
struct MediaViewer: View {
    @Environment(AppModel.self) private var model
    let entries: [GalleryEntryView]
    @Binding var index: Int
    let media: GalleryMedia
    let close: () -> Void
    @State private var image: NSImage?
    @FocusState private var focused: Bool

    private var entry: GalleryEntryView { entries[index] }

    private var picture: String? {
        let set = media.set(for: entry)
        return set?.items.first?.src(asPicture: true) ?? set?.thumbnail ?? entry.thumbnail
    }

    var body: some View {
        ZStack {
            Color.black.opacity(0.94).ignoresSafeArea()
            if let image {
                Image(nsImage: image).resizable().scaledToFit().padding(40)
            } else {
                ProgressView().controlSize(.large)
            }
        }
        .overlay(alignment: .top) {
            HStack {
                Text(entry.item.title).lineLimit(1).foregroundStyle(.white.opacity(0.85))
                Spacer()
                Button(action: close) { Image(systemName: "xmark") }.buttonStyle(.borderless).foregroundStyle(.white)
            }
            .padding(12)
        }
        .focusable()
        .focusEffectDisabled()
        .focused($focused)
        .onAppear { focused = true }
        .onKeyPress(.escape) { close(); return .handled }
        .onKeyPress(.leftArrow) { step(-1) }
        .onKeyPress(.rightArrow) { step(1) }
        .task(id: entry.id) {
            model.markOpened(entry.item.id)
            await media.resolve(entry.item.id)
            image = nil
            if let picture { image = await model.image(picture) }
        }
    }

    private func step(_ by: Int) -> KeyPress.Result {
        let target = index + by
        guard entries.indices.contains(target) else { return .handled }
        index = target
        return .handled
    }
}

extension MediaItemView {
    /// The address of a picture standing for this item.
    func src(asPicture: Bool) -> String? {
        kind == .image ? src : poster
    }
}
