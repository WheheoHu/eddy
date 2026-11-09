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
    /// Translates the page in place when a translation is asked for.
    var translator: WebTranslator?
    /// Takes the links the user clicks to other pages; nil follows them in
    /// this view.
    var onLink: ((URL) -> Void)?
    /// The address the page is at and its title, as they change.
    var onPage: ((URL?, String?) -> Void)?

    func makeNSView(context: Context) -> GestureContainer {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        config.userContentController.addUserScript(WebTranslator.script)
        let view = ReaderWebView(frame: .zero, configuration: config)
        view.allowsBackForwardNavigationGestures = false
        view.navigationDelegate = view
        view.uiDelegate = view
        view.observePage()
        let container = GestureContainer(content: view)
        view.tracker = container.tracker
        view.show(url)
        return container
    }

    func updateNSView(_ container: GestureContainer, context: Context) {
        container.tracker.gestures = gestures
        guard let view = container.content as? ReaderWebView else { return }
        view.onEscape = onEscape
        view.onFocus = onFocus
        view.onLink = onLink
        view.onPage = onPage
        if let translator {
            translator.webView = view
            view.onLoad = { [weak translator] in translator?.pageLoaded() }
        }
        view.wantsFocus = focused
        if view.appearance?.name != appearance?.name { view.appearance = appearance }
        // Only another item's page is loaded: a page that redirected or
        // rewrote its own address stays, however often the view updates.
        if view.shown != url { view.show(url) }
    }

    static func dismantleNSView(_ container: GestureContainer, coordinator: ()) {
        container.tracker.cancel()
    }
}

final class ReaderWebView: WKWebView, WKNavigationDelegate, WKUIDelegate {
    weak var tracker: EdgeGestureTracker?
    var onEscape: (() -> Void)?
    var onFocus: (() -> Void)?
    /// A page finished loading.
    var onLoad: (() -> Void)?
    var onLink: ((URL) -> Void)?
    var onPage: ((URL?, String?) -> Void)?
    /// The address asked for, which the page may have moved on from.
    private(set) var shown: URL?
    private var observations: [NSKeyValueObservation] = []

    func show(_ url: URL) {
        shown = url
        load(URLRequest(url: url))
    }

    var wantsFocus = false {
        didSet { if wantsFocus, !oldValue { claimFocus(self) } }
    }

    func observePage() {
        observations = [
            observe(\.url) { view, _ in MainActor.assumeIsolated { view.onPage?(view.url, view.title) } },
            observe(\.title) { view, _ in MainActor.assumeIsolated { view.onPage?(view.url, view.title) } },
        ]
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        onLoad?()
    }

    /// Links in frames, jumps within the page and links that open a new
    /// window (handled below) are followed as WebKit does.
    func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction, decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
        guard navigationAction.navigationType == .linkActivated,
              navigationAction.targetFrame?.isMainFrame == true,
              let url = navigationAction.request.url,
              !isSamePage(url)
        else { return decisionHandler(.allow) }
        guard onLink != nil || !url.isWebLink || navigationAction.modifierFlags.contains(.command) else {
            return decisionHandler(.allow)
        }
        decisionHandler(.cancel)
        open(url, navigationAction.modifierFlags)
    }

    /// `target="_blank"` links: no second window, the link opens as any
    /// other does.
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if navigationAction.navigationType == .linkActivated, let url = navigationAction.request.url {
            open(url, navigationAction.modifierFlags)
        }
        return nil
    }

    private func open(_ url: URL, _ modifiers: NSEvent.ModifierFlags) {
        if let onLink, url.isWebLink, !modifiers.contains(.command) {
            onLink(url)
        } else {
            NSWorkspace.shared.open(url)
        }
    }

    private func isSamePage(_ target: URL) -> Bool {
        guard target.fragment != nil, let current = url else { return false }
        return target.withoutFragment == current.withoutFragment
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

extension URL {
    /// A page a web view can show.
    var isWebLink: Bool { scheme == "http" || scheme == "https" }

    var withoutFragment: URL {
        guard var components = URLComponents(url: self, resolvingAgainstBaseURL: false) else { return self }
        components.fragment = nil
        return components.url ?? self
    }
}
