import EddyKit
import SwiftUI
import WebKit

/// Everything a renderer needs to show one item.
struct RenderContext {
    let item: ItemView
    let document: DocumentView?
    /// The item belongs in the Gallery: its subscription is shown there,
    /// or it is a saved media link.
    var inGallery = false
}

/// A way to present an item. Renderers register here; new content types
/// (EPUB, PDF) add entries without touching the reader.
struct RendererEntry: Identifiable {
    let id: String
    let title: String
    let icon: String
    let supports: (RenderContext) -> Bool
}

enum RendererRegistry {
    /// The item's pictures and videos, played in place: Gallery items and
    /// posts whose link is media.
    static let media = RendererEntry(id: "media", title: "Media", icon: "play.rectangle", supports: { $0.inGallery || $0.item.url.map(isMediaUrl(url:)) == true })
    static let pdf = RendererEntry(id: "pdf", title: "PDF", icon: "doc.richtext", supports: { $0.item.format == .pdf })
    static let article = RendererEntry(id: "article", title: "Reader", icon: "doc.plaintext", supports: { $0.document?.blocks.isEmpty == false })
    static let web = RendererEntry(id: "web", title: "Web Page", icon: "safari", supports: { $0.item.url != nil })

    static let all: [RendererEntry] = [media, pdf, article, web]

    static func available(for context: RenderContext) -> [RendererEntry] {
        all.filter { $0.supports(context) }
    }

    /// What an item opens in: a PDF as itself, a Gallery item as its
    /// media, everything else as text.
    static func preferred(for item: ItemView?, inGallery: Bool = false) -> RendererEntry {
        if item?.format == .pdf { return pdf }
        return inGallery ? media : article
    }
}

/// The original page, for content the reflowed reader cannot express.
/// A sideways swipe goes back to the reader, so WebKit's own back and
/// forward swipes are off.
struct WebPageView: NSViewRepresentable {
    let url: URL
    var gestures = EdgeGestures()
    var onEscape: (() -> Void)?
    /// Keyboard focus belongs in the page.
    var focused = false
    var onFocus: (() -> Void)?
    /// Pages that style themselves for dark mode follow it; nil inherits
    /// the app's appearance.
    var appearance: NSAppearance?

    func makeNSView(context: Context) -> GestureContainer {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        let view = ReaderWebView(frame: .zero, configuration: config)
        view.allowsBackForwardNavigationGestures = false
        let container = GestureContainer(content: view)
        view.tracker = container.tracker
        view.load(URLRequest(url: url))
        return container
    }

    func updateNSView(_ container: GestureContainer, context: Context) {
        container.tracker.gestures = gestures
        guard let view = container.content as? ReaderWebView else { return }
        view.onEscape = onEscape
        view.onFocus = onFocus
        view.wantsFocus = focused
        if view.appearance?.name != appearance?.name { view.appearance = appearance }
        if view.url?.absoluteString != url.absoluteString, !view.isLoading {
            view.load(URLRequest(url: url))
        }
    }

    static func dismantleNSView(_ container: GestureContainer, coordinator: ()) {
        container.tracker.cancel()
    }
}

final class ReaderWebView: WKWebView {
    weak var tracker: EdgeGestureTracker?
    var onEscape: (() -> Void)?
    var onFocus: (() -> Void)?
    var wantsFocus = false {
        didSet { if wantsFocus, !oldValue { claimFocus(self) } }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if wantsFocus { claimFocus(self) }
    }

    override func becomeFirstResponder() -> Bool {
        let became = super.becomeFirstResponder()
        if became { onFocus?() }
        return became
    }

    override func scrollWheel(with event: NSEvent) {
        if tracker?.handle(event, in: nil) == true { return }
        super.scrollWheel(with: event)
    }

    /// Esc cancels a swipe, else leaves the page for the column before it.
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53, tracker?.active == true {
            tracker?.cancel()
        } else if event.keyCode == 53, let onEscape {
            onEscape()
        } else {
            super.keyDown(with: event)
        }
    }
}
