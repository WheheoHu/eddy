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
    static let article = RendererEntry(id: "article", title: "Reader", icon: "doc.plaintext", supports: { $0.document != nil })
    static let web = RendererEntry(id: "web", title: "Web Page", icon: "safari", supports: { $0.item.url != nil })

    static let all: [RendererEntry] = [article, web]

    static func available(for context: RenderContext) -> [RendererEntry] {
        all.filter { $0.supports(context) }
    }
}

/// The original page, for content the reflowed reader cannot express.
struct WebPageView: NSViewRepresentable {
    let url: URL

    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        let view = WKWebView(frame: .zero, configuration: config)
        view.allowsBackForwardNavigationGestures = true
        view.load(URLRequest(url: url))
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        if view.url?.absoluteString != url.absoluteString, !view.isLoading {
            view.load(URLRequest(url: url))
        }
    }
}
