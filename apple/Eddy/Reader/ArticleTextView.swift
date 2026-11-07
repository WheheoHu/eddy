import AppKit
import EddyKit
import SwiftUI

/// Where to scroll: a block and an offset in it, with a token so the same
/// target can be requested twice.
struct ScrollTarget: Equatable {
    var blockID: String
    var offset: Int = 0
    /// The end of the text instead of a block.
    var toEnd = false
    var token = UUID()

    static var end: ScrollTarget { ScrollTarget(blockID: "", toEnd: true) }
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
    /// Title of what pulling past an edge opens; nil turns pulls off.
    var pullTarget: ((Direction) -> String?)?
    var onPull: (Pull?) -> Void = { _ in }
    var onPullRelease: (Direction) -> Void = { _ in }
    /// Cmd+Left and Cmd+Right.
    var onStep: ((Direction) -> Void)?
    var onEscape: (() -> Void)?

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
        textView.pulls = context.coordinator.pulls

        let scroll = ReaderScrollView()
        scroll.pulls = context.coordinator.pulls
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
        c.pulls.target = pullTarget
        c.pulls.onChange = onPull
        c.pulls.onRelease = onPullRelease
        guard let textView = scroll.documentView as? ReaderTextView else { return }
        textView.onStep = onStep
        textView.onEscape = onEscape
        textView.lineStep = style.fontSize * style.lineHeight
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

    static func dismantleNSView(_ scroll: NSScrollView, coordinator: Coordinator) {
        coordinator.pulls.onChange = nil
        coordinator.pulls.reset()
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
        let pulls = PullTracker()
        private var lastReported: (String, Int, Bool)?
        private var reportTask: Task<Void, Never>?
        private var imageTask: Task<Void, Never>?
        /// Position set by the app (resume, outline jump, the end of a
        /// previous chapter) that images loading above must not disturb,
        /// until the user scrolls on their own.
        private var anchor: ScrollTarget?
        private var scrollingProgrammatically = false
        private var settlingEnd = false

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
            let anchor = self.anchor ?? currentPosition().map { ScrollTarget(blockID: $0.0, offset: $0.1) }
            attachment.loaded(image)
            var found: NSRange?
            storage.enumerateAttribute(.attachment, in: NSRange(location: 0, length: storage.length)) { value, range, stop in
                if (value as? ImageAttachment) === attachment { found = range; stop.pointee = true }
            }
            guard let range = found else { return }
            storage.edited(.editedAttributes, range: range, changeInLength: 0)
            if let anchor { scroll(to: anchor, animated: false, report: false) }
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
            guard let scroll, let textView, let tlm = textView.textLayoutManager, let article else { return }
            var y: CGFloat
            if target.toEnd {
                tlm.ensureLayout(for: tlm.documentRange)
                y = textView.frame.height
                // The view takes its full height once layout resizes it.
                if !settlingEnd {
                    settlingEnd = true
                    DispatchQueue.main.async { [weak self] in
                        self?.scroll(to: target, animated: false, report: report)
                        self?.settlingEnd = false
                    }
                }
            } else {
                guard let utf16 = article.location(of: target.blockID, offset: target.offset),
                      let location = tlm.location(tlm.documentRange.location, offsetBy: utf16)
                else { return }
                tlm.ensureLayout(for: NSTextRange(location: tlm.documentRange.location, end: location) ?? tlm.documentRange)
                guard let fragment = tlm.textLayoutFragment(for: location) else { return }
                y = fragment.layoutFragmentFrame.minY + textView.textContainerOrigin.y - 12
                // Within the paragraph, land on the line holding the offset.
                if let line = fragment.textLineFragments.first(where: { line in
                    let lineStart = tlm.offset(from: tlm.documentRange.location, to: fragment.rangeInElement.location) + line.characterRange.location
                    return utf16 >= lineStart && utf16 < lineStart + max(line.characterRange.length, 1)
                }) {
                    y += line.typographicBounds.minY
                }
            }
            y = max(0, min(y, textView.frame.height - scroll.contentView.bounds.height))
            let point = NSPoint(x: 0, y: y)
            anchor = target
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
            if !report, !target.toEnd { lastReported = (target.blockID, target.offset, lastReported?.2 ?? false) }
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

/// Passes scrolling on to the pull tracker once it has scrolled, so the
/// system's rubber-band still shows while pulling past an edge.
final class ReaderScrollView: NSScrollView {
    weak var pulls: PullTracker?

    override func scrollWheel(with event: NSEvent) {
        super.scrollWheel(with: event)
        pulls?.scrollWheel(event, in: self)
    }
}

/// Keeps the text column at a readable width, centered. Reads with the
/// keyboard: Up and Down move a line, Left and Right a page, an arrow held
/// at the edge it points past pulls to the previous or next item, Cmd+Left
/// and Cmd+Right go there at once.
final class ReaderTextView: NSTextView {
    var maxWidth: CGFloat = 700
    /// One line of text.
    var lineStep: CGFloat = 24
    weak var pulls: PullTracker?
    var onStep: ((Direction) -> Void)?
    var onEscape: (() -> Void)?

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        updateInsets()
    }

    func updateInsets() {
        let horizontal = max(28, (bounds.width - maxWidth) / 2)
        let inset = NSSize(width: horizontal, height: 36)
        if textContainerInset != inset { textContainerInset = inset }
    }

    override func keyDown(with event: NSEvent) {
        let modifiers = event.modifierFlags.intersection([.command, .option, .control, .shift])
        guard let key = event.specialKey, let scroll = enclosingScrollView else {
            if event.keyCode == 53 { cancelOperation(nil) } else { super.keyDown(with: event) }
            return
        }
        let page = max(scroll.contentView.bounds.height - lineStep, lineStep)
        if modifiers == .command, key == .leftArrow || key == .rightArrow, let onStep {
            pulls?.reset()
            onStep(key == .leftArrow ? .previous : .next)
        } else if modifiers.isEmpty, key == .downArrow || key == .rightArrow {
            if scroll.atBottom {
                pulls?.hold(.next, key: event.keyCode)
            } else {
                scrollBy(key == .downArrow ? lineStep : page, animated: key == .rightArrow)
            }
        } else if modifiers.isEmpty, key == .upArrow || key == .leftArrow {
            if scroll.atTop {
                pulls?.hold(.previous, key: event.keyCode)
            } else {
                scrollBy(key == .upArrow ? -lineStep : -page, animated: key == .leftArrow)
            }
        } else {
            super.keyDown(with: event)
        }
    }

    override func keyUp(with event: NSEvent) {
        pulls?.release(key: event.keyCode)
        super.keyUp(with: event)
    }

    /// Esc closes the find bar, then cancels a pull, then leaves the text.
    override func cancelOperation(_ sender: Any?) {
        if let scroll = enclosingScrollView, scroll.isFindBarVisible {
            scroll.isFindBarVisible = false
        } else if pulls?.active == true {
            pulls?.reset()
        } else if let onEscape {
            onEscape()
        }
    }

    private func scrollBy(_ dy: CGFloat, animated: Bool) {
        guard let scroll = enclosingScrollView else { return }
        let clip = scroll.contentView
        let top = -scroll.contentInsets.top
        let bottom = max(top, frame.height - clip.bounds.height + scroll.contentInsets.bottom)
        let point = NSPoint(x: clip.bounds.minX, y: min(max(clip.bounds.minY + dy, top), bottom))
        if animated {
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0.18
                clip.animator().setBoundsOrigin(point)
            } completionHandler: {
                MainActor.assumeIsolated { scroll.reflectScrolledClipView(clip) }
            }
        } else {
            clip.scroll(to: point)
            scroll.reflectScrolledClipView(clip)
        }
    }
}
