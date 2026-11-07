import EddyKit
import SwiftUI
import WebKit

/// Everything a renderer needs to show one item.
struct RenderContext {
    let item: ItemView
    let document: DocumentView?
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
    static let pdf = RendererEntry(id: "pdf", title: "PDF", icon: "doc.richtext", supports: { $0.item.format == .pdf })
    static let article = RendererEntry(id: "article", title: "Reader", icon: "doc.plaintext", supports: { $0.document?.blocks.isEmpty == false })
    static let web = RendererEntry(id: "web", title: "Web Page", icon: "safari", supports: { $0.item.url != nil })

    static let all: [RendererEntry] = [pdf, article, web]

    static func available(for context: RenderContext) -> [RendererEntry] {
        all.filter { $0.supports(context) }
    }

    /// What an item opens in.
    static func preferred(for item: ItemView?) -> RendererEntry {
        item?.format == .pdf ? pdf : article
    }
}

/// The original page, for content the reflowed reader cannot express.
/// A sideways swipe goes back to the reader, so WebKit's own back and
/// forward swipes are off.
struct WebPageView: NSViewRepresentable {
    let url: URL
    var onSwipe: ((SwipeDirection) -> Void)?
    var onEscape: (() -> Void)?
    /// Keyboard focus belongs in the page.
    var focused = false
    var onFocus: (() -> Void)?

    func makeCoordinator() -> SwipeTracker { SwipeTracker() }

    func makeNSView(context: Context) -> ReaderWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        let view = ReaderWebView(frame: .zero, configuration: config)
        view.allowsBackForwardNavigationGestures = false
        view.swipes = context.coordinator
        view.load(URLRequest(url: url))
        return view
    }

    func updateNSView(_ view: ReaderWebView, context: Context) {
        context.coordinator.onSwipe = onSwipe
        view.onEscape = onEscape
        view.onFocus = onFocus
        view.wantsFocus = focused
        if view.url?.absoluteString != url.absoluteString, !view.isLoading {
            view.load(URLRequest(url: url))
        }
    }
}

final class ReaderWebView: WKWebView {
    weak var swipes: SwipeTracker?
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
        if swipes?.handle(event) == true { return }
        super.scrollWheel(with: event)
    }

    /// Esc leaves the page for the column before it.
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53, let onEscape {
            onEscape()
        } else {
            super.keyDown(with: event)
        }
    }
}
