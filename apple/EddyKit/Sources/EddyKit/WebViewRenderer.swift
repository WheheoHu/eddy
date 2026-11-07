import Foundation
import WebKit

/// Renders pages in an offscreen WKWebView for the `webview` fetch strategy.
/// It shares the default website data store, so sites the user signed in to
/// from Eddy's in-app browser render with that session.
public final class WebViewRenderer: NSObject, WebPageRenderer, @unchecked Sendable {
    private let timeout: TimeInterval
    private let settleDelay: TimeInterval

    public init(timeout: TimeInterval = 30, settleDelay: TimeInterval = 1.5) {
        self.timeout = timeout
        self.settleDelay = settleDelay
    }

    public func render(url: String) async -> String? {
        guard let target = URL(string: url) else { return nil }
        return await MainActor.run { RenderJob(timeout: timeout, settleDelay: settleDelay) }.run(target)
    }
}

@MainActor
private final class RenderJob: NSObject, WKNavigationDelegate {
    private let timeout: TimeInterval
    private let settleDelay: TimeInterval
    private var webView: WKWebView?
    private var continuation: CheckedContinuation<String?, Never>?

    init(timeout: TimeInterval, settleDelay: TimeInterval) {
        self.timeout = timeout
        self.settleDelay = settleDelay
    }

    func run(_ url: URL) async -> String? {
        await withCheckedContinuation { continuation in
            self.continuation = continuation
            let config = WKWebViewConfiguration()
            config.websiteDataStore = .default()
            let view = WKWebView(frame: CGRect(x: 0, y: 0, width: 1280, height: 2000), configuration: config)
            view.navigationDelegate = self
            webView = view
            view.load(URLRequest(url: url, timeoutInterval: timeout))
            DispatchQueue.main.asyncAfter(deadline: .now() + timeout) { [weak self] in self?.finish(nil) }
        }
    }

    private func finish(_ html: String?) {
        guard let continuation else { return }
        self.continuation = nil
        webView?.stopLoading()
        webView = nil
        continuation.resume(returning: html)
    }

    nonisolated func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        Task { @MainActor in
            // Bot checks ("Checking your browser…") resolve themselves after a
            // few seconds and navigate on; keep waiting until the real page is
            // there or time runs out.
            for _ in 0..<12 {
                try? await Task.sleep(for: .seconds(settleDelay))
                guard continuation != nil else { return }
                let html = try? await webView.evaluateJavaScript("'<!doctype html>\\n' + document.documentElement.outerHTML") as? String
                if let html, !Self.looksLikeChallenge(html) {
                    finish(html)
                    return
                }
            }
            finish(nil)
        }
    }

    private static func looksLikeChallenge(_ html: String) -> Bool {
        ["Checking your browser", "Just a moment...", "challenge-platform", "cf-browser-verification", "captcha-delivery.com"].contains { html.contains($0) }
    }

    nonisolated func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        Task { @MainActor in finish(nil) }
    }

    nonisolated func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        Task { @MainActor in finish(nil) }
    }
}

/// Watches the capture spool directory and calls `onChange` when the browser
/// host drops a new page.
public final class SpoolWatcher: @unchecked Sendable {
    private var source: DispatchSourceFileSystemObject?

    public init?(path: String, onChange: @escaping @Sendable () -> Void) {
        try? FileManager.default.createDirectory(atPath: path, withIntermediateDirectories: true)
        let fd = open(path, O_EVTONLY)
        guard fd >= 0 else { return nil }
        let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: fd, eventMask: .write, queue: .main)
        source.setEventHandler(handler: onChange)
        source.setCancelHandler { close(fd) }
        source.resume()
        self.source = source
    }

    deinit { source?.cancel() }
}
