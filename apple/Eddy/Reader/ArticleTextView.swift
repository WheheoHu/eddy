import AppKit
import EddyKit
import SwiftUI

/// Where to scroll: a block and an offset in it, with a token so the same
/// target can be requested twice.
struct ScrollTarget: Equatable {
    var blockID: String
    var offset: Int = 0
    var token = UUID()
}

/// The reflowed article in a TextKit 2 text view: native layout, selection
/// across paragraphs, find, and reading positions mapped to engine blocks.
struct ArticleTextView: NSViewRepresentable {
    let article: BuiltArticle
    /// Changes whenever `article` is rebuilt; the text is only replaced then.
    let version: String
    let style: ReaderStyle
    let scrollTarget: ScrollTarget?
    let loadImage: (String) async -> NSImage?
    /// The block at the top of the view, and whether the end of the text is
    /// on screen.
    let onPosition: (_ blockID: String, _ offset: Int, _ atEnd: Bool) -> Void
    /// Links inside an imported book (`epub://`); others open in the browser.
    var onBookLink: (URL) -> Void = { _ in }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSScrollView {
        let textView = ReaderTextView(usingTextLayoutManager: true)
        textView.isEditable = false
        textView.isSelectable = true
        textView.drawsBackground = true
        textView.backgroundColor = .textBackgroundColor
        textView.usesFindBar = true
        textView.isIncrementalSearchingEnabled = true
        textView.isAutomaticLinkDetectionEnabled = false
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true
        textView.textContainer?.lineFragmentPadding = 0
        textView.maxWidth = style.maxWidth
        textView.linkTextAttributes = [.foregroundColor: NSColor.linkColor, .cursor: NSCursor.pointingHand]
        textView.textLayoutManager?.delegate = context.coordinator
        textView.delegate = context.coordinator

        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = true
        scroll.backgroundColor = .textBackgroundColor
        scroll.documentView = textView
        scroll.contentView.postsBoundsChangedNotifications = true
        context.coordinator.attach(scroll: scroll, textView: textView)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        let c = context.coordinator
        c.onPosition = onPosition
        c.loadImage = loadImage
        c.onBookLink = onBookLink
        guard let textView = scroll.documentView as? ReaderTextView else { return }
        if textView.maxWidth != style.maxWidth {
            textView.maxWidth = style.maxWidth
            textView.updateInsets()
        }
        if c.version != version {
            c.show(article, version: version, keepPosition: c.version.map { Self.sameDocument($0, version) } ?? false)
        }
        if let target = scrollTarget, target != c.lastTarget {
            // The first target after opening is the resume point: jump there.
            let animated = c.lastTarget != nil
            c.lastTarget = target
            c.scroll(to: target, animated: animated)
        }
    }

    /// Versions share their prefix (document hash) when only the reading
    /// mode or style changed; the position then carries over.
    private static func sameDocument(_ a: String, _ b: String) -> Bool {
        a.split(separator: "|").first == b.split(separator: "|").first
    }

    @MainActor
    final class Coordinator: NSObject, NSTextLayoutManagerDelegate, NSTextViewDelegate {
        weak var scroll: NSScrollView?
        weak var textView: ReaderTextView?
        var article: BuiltArticle?
        var version: String?
        var lastTarget: ScrollTarget?
        var hasShown = false
        var onPosition: ((String, Int, Bool) -> Void)?
        var loadImage: ((String) async -> NSImage?)?
        var onBookLink: ((URL) -> Void)?
        private var lastReported: (String, Int, Bool)?
        private var reportTask: Task<Void, Never>?
        private var imageTask: Task<Void, Never>?
        /// Position set by the app (resume, outline jump) that images loading
        /// above must not disturb, until the user scrolls on their own.
        private var anchor: (String, Int)?
        private var scrollingProgrammatically = false

        func attach(scroll: NSScrollView, textView: ReaderTextView) {
            self.scroll = scroll
            self.textView = textView
            NotificationCenter.default.addObserver(self, selector: #selector(scrolled), name: NSView.boundsDidChangeNotification, object: scroll.contentView)
        }

        func show(_ article: BuiltArticle, version: String, keepPosition: Bool) {
            guard let textView else { return }
            let anchor = keepPosition ? currentPosition() : nil
            self.article = article
            self.version = version
            imageTask?.cancel()
            textView.textStorage?.setAttributedString(article.text)
            textView.updateInsets()
            hasShown = true
            if let anchor {
                DispatchQueue.main.async { self.scroll(to: ScrollTarget(blockID: anchor.0, offset: anchor.1), animated: false) }
            } else if lastTarget == nil {
                scroll?.contentView.scroll(to: .zero)
            }
            loadImages(article.images)
            report(onlyAtEnd: true)
        }

        private func loadImages(_ images: [ImageAttachment]) {
            guard let loadImage else { return }
            imageTask = Task { [weak self] in
                for attachment in images {
                    if Task.isCancelled { return }
                    guard let image = await loadImage(attachment.url) else { continue }
                    if Task.isCancelled { return }
                    self?.replaceImage(attachment, with: image)
                }
            }
        }

        /// Swaps in a loaded image, keeping the reading position steady.
        private func replaceImage(_ attachment: ImageAttachment, with image: NSImage) {
            guard let textView, let storage = textView.textStorage else { return }
            let anchor = self.anchor ?? currentPosition()
            attachment.loaded(image)
            var found: NSRange?
            storage.enumerateAttribute(.attachment, in: NSRange(location: 0, length: storage.length)) { value, range, stop in
                if (value as? ImageAttachment) === attachment { found = range; stop.pointee = true }
            }
            guard let range = found else { return }
            storage.edited(.editedAttributes, range: range, changeInLength: 0)
            if let anchor { scroll(to: ScrollTarget(blockID: anchor.0, offset: anchor.1), animated: false, report: false) }
            report(onlyAtEnd: true)
        }

        /// Block and offset at the top of the visible area.
        func currentPosition() -> (String, Int)? {
            guard let scroll, let textView, let tlm = textView.textLayoutManager, let article else { return nil }
            let y = scroll.contentView.bounds.minY - textView.textContainerOrigin.y + 2
            guard let fragment = tlm.textLayoutFragment(for: CGPoint(x: 0, y: max(0, y))) else { return nil }
            let location = tlm.offset(from: tlm.documentRange.location, to: fragment.rangeInElement.location)
            return article.block(at: location)
        }

        @objc func scrolled() {
            if !scrollingProgrammatically { anchor = nil }
            report(onlyAtEnd: false)
        }

        /// Whether the last line of the text is inside the visible area,
        /// which includes text shorter than the view.
        var atEnd: Bool {
            guard let scroll, let textView else { return false }
            return scroll.contentView.bounds.maxY >= textView.frame.height - textView.textContainerInset.height - 8
        }

        /// Reports the position once the view settles. Text that never
        /// scrolls reports from `show` and image loads, but only when its
        /// end is on screen: opening an article is not reading it.
        private func report(onlyAtEnd: Bool) {
            reportTask?.cancel()
            reportTask = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(250))
                guard let self, !Task.isCancelled, let position = self.currentPosition() else { return }
                let atEnd = self.atEnd
                if onlyAtEnd, !atEnd { return }
                if let last = self.lastReported, last.0 == position.0, last.1 == position.1, last.2 == atEnd { return }
                self.lastReported = (position.0, position.1, atEnd)
                self.onPosition?(position.0, position.1, atEnd)
            }
        }

        func scroll(to target: ScrollTarget, animated: Bool, report: Bool = true) {
            guard let scroll, let textView, let tlm = textView.textLayoutManager, let article,
                  let utf16 = article.location(of: target.blockID, offset: target.offset),
                  let location = tlm.location(tlm.documentRange.location, offsetBy: utf16)
            else { return }
            tlm.ensureLayout(for: NSTextRange(location: tlm.documentRange.location, end: location) ?? tlm.documentRange)
            guard let fragment = tlm.textLayoutFragment(for: location) else { return }
            var y = fragment.layoutFragmentFrame.minY + textView.textContainerOrigin.y - 12
            // Within the paragraph, land on the line holding the offset.
            if let line = fragment.textLineFragments.first(where: { line in
                let lineStart = tlm.offset(from: tlm.documentRange.location, to: fragment.rangeInElement.location) + line.characterRange.location
                return utf16 >= lineStart && utf16 < lineStart + max(line.characterRange.length, 1)
            }) {
                y += line.typographicBounds.minY
            }
            y = max(0, min(y, textView.frame.height - scroll.contentView.bounds.height))
            let point = NSPoint(x: 0, y: y)
            anchor = (target.blockID, target.offset)
            scrollingProgrammatically = true
            if animated {
                NSAnimationContext.runAnimationGroup { ctx in
                    ctx.duration = 0.25
                    scroll.contentView.animator().setBoundsOrigin(point)
                } completionHandler: { [weak self] in
                    MainActor.assumeIsolated { self?.scrollingProgrammatically = false }
                }
            } else {
                scroll.contentView.scroll(to: point)
                scroll.reflectScrolledClipView(scroll.contentView)
                DispatchQueue.main.async { [weak self] in self?.scrollingProgrammatically = false }
            }
            if !report { lastReported = (target.blockID, target.offset, lastReported?.2 ?? false) }
        }

        // Code backgrounds and quote bars.
        nonisolated func textLayoutManager(_ textLayoutManager: NSTextLayoutManager, textLayoutFragmentFor location: any NSTextLocation, in textElement: NSTextElement) -> NSTextLayoutFragment {
            if let paragraph = textElement as? NSTextParagraph, paragraph.attributedString.length > 0 {
                let attrs = paragraph.attributedString.attributes(at: 0, effectiveRange: nil)
                if attrs[.eddyCode] != nil {
                    let f = DecoratedFragment(textElement: textElement, range: textElement.elementRange)
                    f.kind = .code
                    return f
                }
                if let depth = attrs[.eddyQuote] as? Int, depth > 0 {
                    let f = DecoratedFragment(textElement: textElement, range: textElement.elementRange)
                    f.kind = .quote(depth)
                    return f
                }
            }
            return NSTextLayoutFragment(textElement: textElement, range: textElement.elementRange)
        }

        func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
            if let url = link as? URL ?? (link as? String).flatMap(URL.init(string:)) {
                if url.scheme == "epub" {
                    onBookLink?(url)
                } else {
                    NSWorkspace.shared.open(url)
                }
                return true
            }
            return false
        }
    }
}

/// Keeps the text column at a readable width, centered.
final class ReaderTextView: NSTextView {
    var maxWidth: CGFloat = 700

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        updateInsets()
    }

    func updateInsets() {
        let horizontal = max(28, (bounds.width - maxWidth) / 2)
        let inset = NSSize(width: horizontal, height: 36)
        if textContainerInset != inset { textContainerInset = inset }
    }
}
