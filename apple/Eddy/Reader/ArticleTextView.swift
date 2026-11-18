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
    /// Videos of the article (`eddy-media:` links), for the media viewer.
    var onMediaLink: (URL) -> Void = { _ in }
    /// Web links clicked without ⌘; nil, or ⌘-clicked, they open in the
    /// browser.
    var onWebLink: ((URL) -> Void)?
    /// Pulls past the top and bottom of the text, and sideways swipes.
    var gestures = EdgeGestures()
    /// Cmd+Left and Cmd+Right.
    var onStep: ((Direction) -> Void)?
    var onEscape: (() -> Void)?
    /// Keyboard focus belongs in the text; it takes it when this turns on,
    /// or when it appears with it on.
    var focused = false
    var onFocus: (() -> Void)?
    /// The content's own appearance; nil inherits the app's.
    var appearance: NSAppearance?
    /// Languages the code block menu offers.
    var codeLanguages: [String] = []
    /// A language picked for a code block from the label in its corner;
    /// nil goes back to the declared or guessed one.
    var onCodeLanguage: ((CodeBlockMark, String?) -> Void)?
    /// The side the item's text comes in from, the first time it is shown.
    var arrival: ArrivalSide = .below
    /// Whether that first time is still to come; nil never plays it.
    var arriving: Arriving?

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> GestureContainer {
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
        // A layer of its own, for the arrival to play on.
        textView.wantsLayer = true

        let scroll = ReaderScrollView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = true
        scroll.backgroundColor = .textBackgroundColor
        scroll.documentView = textView
        scroll.contentView.postsBoundsChangedNotifications = true
        let container = GestureContainer(content: scroll)
        scroll.tracker = container.tracker
        textView.tracker = container.tracker
        context.coordinator.attach(scroll: scroll, textView: textView)
        textView.onReflow = { [weak coordinator = context.coordinator] done in
            if done { coordinator?.reflowed() } else { coordinator?.willReflow() }
        }
        return container
    }

    func updateNSView(_ container: GestureContainer, context: Context) {
        let c = context.coordinator
        c.onPosition = onPosition
        c.loadImage = loadImage
        c.onBookLink = onBookLink
        c.onMediaLink = onMediaLink
        c.onWebLink = onWebLink
        container.tracker.gestures = gestures
        if container.appearance?.name != appearance?.name { container.appearance = appearance }
        guard let scroll = c.scroll, let textView = scroll.documentView as? ReaderTextView else { return }
        // With somewhere to go, the article itself follows a pull past the
        // ends instead of the system's bounce.
        scroll.verticalScrollElasticity = gestures.pullTarget != nil ? .none : .automatic
        textView.onStep = onStep
        textView.onEscape = onEscape
        textView.onFocus = onFocus
        textView.wantsFocus = focused
        textView.codeLanguages = codeLanguages
        textView.onCodeLanguage = onCodeLanguage
        textView.lineStep = style.fontSize * style.lineHeight
        if textView.maxWidth != style.maxWidth {
            textView.maxWidth = style.maxWidth
            textView.updateInsets()
        }
        if c.version != version {
            // The item's first text: it arrives. Text rebuilt after (a
            // translation, a style, the full text) replaces it in place.
            if c.version == nil, arriving?.claim() == true { textView.arrive(from: arrival) }
            c.show(article, version: version, keepPosition: c.version.map { Self.sameDocument($0, version) } ?? false)
        }
        if let target = scrollTarget, target != c.lastTarget {
            // The first target after opening is the resume point: jump there.
            let animated = c.lastTarget != nil
            c.lastTarget = target
            c.scroll(to: target, animated: animated)
        }
    }

    static func dismantleNSView(_ container: GestureContainer, coordinator: Coordinator) {
        container.tracker.cancel()
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
        var onMediaLink: ((URL) -> Void)?
        var onWebLink: ((URL) -> Void)?
        private var lastReported: (String, Int, Bool)?
        private var reportTask: Task<Void, Never>?
        private var imageTasks: [Task<Void, Never>] = []
        private var pendingImages: [ImageAttachment] = []
        private var imageRanges: [ObjectIdentifier: NSRange] = [:]
        /// Images loaded for the document being shown, by URL.
        private var loadedImages: [String: NSImage] = [:]
        private let imageConcurrency = 2
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
            let held = keepPosition ? holdPosition() : nil
            self.article = article
            self.version = version
            imageTasks.forEach { $0.cancel() }
            // A rebuilt article (a translation arriving, a style change)
            // comes with fresh attachments; the images already shown go
            // straight back in at their size, so nothing above the reader
            // changes height after the position is put back.
            if !keepPosition { loadedImages = [:] }
            for attachment in article.images where !attachment.isLoaded {
                if let image = loadedImages[attachment.url] { attachment.loaded(image) }
            }
            textView.textStorage?.setAttributedString(article.text)
            textView.updateInsets()
            hasShown = true
            if let held {
                restore(held)
            } else if lastTarget == nil {
                scroll?.contentView.scroll(to: .zero)
            }
            loadImages(article.images)
            report(onlyAtEnd: true)
        }

        /// The line at the top of the view, and how far into it the view
        /// starts, to put back after the text is rebuilt (a translation batch
        /// arriving, a change of reading mode or style).
        private struct HeldPosition {
            /// nil: the view is at the header above the first block, which
            /// has no spot of its own; it stays at the top.
            let spot: BuiltArticle.Spot?
            let offset: Int
            let delta: CGFloat
            /// Where the line starts in the text as it is, and how far into
            /// the line the view starts: exact for as long as the text
            /// itself doesn't change.
            let location: Int
            let lineDelta: CGFloat
        }

        private func holdPosition() -> HeldPosition? {
            guard let scroll, let textView, let tlm = textView.textLayoutManager, let article else { return nil }
            let top = scroll.contentView.bounds.minY
            let y = max(0, top - textView.textContainerOrigin.y)
            guard let fragment = tlm.textLayoutFragment(for: CGPoint(x: 0, y: y)) else { return nil }
            let fragmentStart = tlm.offset(from: tlm.documentRange.location, to: fragment.rangeInElement.location)
            let within = y - fragment.layoutFragmentFrame.minY
            let line = fragment.textLineFragments.last { $0.typographicBounds.minY <= within } ?? fragment.textLineFragments.first
            let lineTop = fragment.layoutFragmentFrame.minY + (line?.typographicBounds.minY ?? 0) + textView.textContainerOrigin.y
            let location = fragmentStart + (line?.characterRange.location ?? 0)
            // In the header (title, byline): anchoring to the first block
            // would scroll the header away when the text is rebuilt.
            if let first = article.blocks.first, location < first.range.location {
                return HeldPosition(spot: nil, offset: 0, delta: top, location: location, lineDelta: top - lineTop)
            }
            guard let place = article.spot(at: location) else { return nil }
            return HeldPosition(spot: place.spot, offset: place.offset, delta: top - lineTop, location: location, lineDelta: top - lineTop)
        }

        /// The line at the top while the view's width changes: a window
        /// being resized, or the columns beside the reader coming and going.
        /// Narrower lines break anew, and the same distance down the text
        /// would be another place in it.
        private var heldForReflow: HeldPosition?

        func willReflow() {
            guard hasShown, heldForReflow == nil else { return }
            heldForReflow = holdPosition()
        }

        func reflowed() {
            guard let held = heldForReflow else { return }
            heldForReflow = nil
            restore(held, sameText: true)
        }

        /// Puts the held line back where it was, before anything is drawn.
        /// `sameText`: the text is the one the position was held in, only
        /// laid out anew, so the line is found where it was; a rebuilt text
        /// is searched for the line's block instead.
        private func restore(_ held: HeldPosition, sameText: Bool = false, retry: Bool = true) {
            guard let scroll, let textView, let tlm = textView.textLayoutManager, let article else { return }
            guard sameText || held.spot != nil else {
                scrollingProgrammatically = true
                scroll.contentView.scroll(to: NSPoint(x: 0, y: max(0, held.delta)))
                scroll.reflectScrolledClipView(scroll.contentView)
                DispatchQueue.main.async { [weak self] in self?.scrollingProgrammatically = false }
                return
            }
            let delta = sameText ? held.lineDelta : held.delta
            guard let utf16 = sameText ? held.location : held.spot.flatMap({ article.location(of: $0, offset: held.offset) }),
                  let location = tlm.location(tlm.documentRange.location, offsetBy: utf16)
            else { return }
            tlm.ensureLayout(for: NSTextRange(location: tlm.documentRange.location, end: location) ?? tlm.documentRange)
            guard let fragment = tlm.textLayoutFragment(for: location) else { return }
            let fragmentStart = tlm.offset(from: tlm.documentRange.location, to: fragment.rangeInElement.location)
            var lineTop = fragment.layoutFragmentFrame.minY + textView.textContainerOrigin.y
            if let line = fragment.textLineFragments.first(where: { line in
                let start = fragmentStart + line.characterRange.location
                return utf16 >= start && utf16 < start + max(line.characterRange.length, 1)
            }) {
                lineTop += line.typographicBounds.minY
            }
            let target = max(0, lineTop + delta)
            scrollingProgrammatically = true
            scroll.contentView.scroll(to: NSPoint(x: 0, y: target))
            scroll.reflectScrolledClipView(scroll.contentView)
            // The view's height may lag the new text and cut the scroll
            // short; once it has caught up, go there again.
            let cutShort = retry && abs(scroll.contentView.bounds.minY - target) > 0.5
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                if cutShort { self.restore(held, sameText: sameText, retry: false) }
                self.scrollingProgrammatically = false
            }
        }

        /// Loads the article's images a couple at a time, those on or near
        /// the screen first, so what the reader is looking at fills in
        /// before the rest of a long document.
        private func loadImages(_ images: [ImageAttachment]) {
            imageTasks.forEach { $0.cancel() }
            imageTasks = []
            pendingImages = images.filter { !$0.isLoaded }
            imageRanges = [:]
            guard loadImage != nil, !pendingImages.isEmpty, let storage = textView?.textStorage else { return }
            storage.enumerateAttribute(.attachment, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
                if let attachment = value as? ImageAttachment { imageRanges[ObjectIdentifier(attachment)] = range }
            }
            imageTasks = (0..<imageConcurrency).map { _ in
                Task { [weak self] in await self?.loadNextImages() }
            }
        }

        private func loadNextImages() async {
            while !Task.isCancelled, let loadImage, let attachment = nextImage() {
                let image = await loadImage(attachment.url)
                if Task.isCancelled { return }
                if let image { replaceImage(attachment, with: image) }
            }
        }

        /// The next image to load: one laid out within a screen of the
        /// visible area if there is one, else the first in reading order.
        private func nextImage() -> ImageAttachment? {
            guard !pendingImages.isEmpty else { return nil }
            var pick = 0
            if let scroll, let textView, let tlm = textView.textLayoutManager {
                let visible = scroll.contentView.bounds.insetBy(dx: 0, dy: -scroll.contentView.bounds.height)
                for (i, attachment) in pendingImages.enumerated() {
                    guard let range = imageRanges[ObjectIdentifier(attachment)],
                          let location = tlm.location(tlm.documentRange.location, offsetBy: range.location),
                          let fragment = tlm.textLayoutFragment(for: location), fragment.state != .none
                    else { continue }
                    if fragment.layoutFragmentFrame.offsetBy(dx: 0, dy: textView.textContainerOrigin.y).intersects(visible) {
                        pick = i
                        break
                    }
                }
            }
            return pendingImages.remove(at: pick)
        }

        /// Swaps in a loaded image without disturbing the reader. An image
        /// that keeps its size (PDF figures, pictures with dimensions in
        /// the HTML) is only redrawn; one that changes height relays out
        /// its paragraph, and the view moves with the text when that
        /// paragraph is above what is on screen.
        private func replaceImage(_ attachment: ImageAttachment, with image: NSImage) {
            guard let scroll, let textView, let storage = textView.textStorage, let tlm = textView.textLayoutManager,
                  let range = imageRanges[ObjectIdentifier(attachment)], range.location + range.length <= storage.length
            else { return }
            let lineWidth = textView.textContainer?.size.width ?? textView.bounds.width
            let before = attachment.displayHeight(forLineWidth: lineWidth)
            attachment.loaded(image)
            loadedImages[attachment.url] = image
            let after = attachment.displayHeight(forLineWidth: lineWidth)
            let origin = textView.textContainerOrigin
            let location = tlm.location(tlm.documentRange.location, offsetBy: range.location)
            let fragment = location.flatMap { tlm.textLayoutFragment(for: $0) }
            let frame = fragment.map { $0.layoutFragmentFrame.offsetBy(dx: origin.x, dy: origin.y) }
            if abs(after - before) < 0.5 {
                // Not laid out yet: it comes up with the image when it is.
                guard let fragment, fragment.state != .none, let frame else { return }
                tlm.invalidateLayout(for: fragment.rangeInElement)
                textView.setNeedsDisplay(frame.insetBy(dx: -8, dy: -8))
                return
            }
            let top = scroll.contentView.bounds.minY
            let above = frame.map { $0.maxY <= top } ?? false
            storage.edited(.editedAttributes, range: range, changeInLength: 0)
            if let anchor, anchor.toEnd || article?.location(of: anchor.blockID, offset: anchor.offset).map({ range.location < $0 }) == true {
                // Still at a position the app chose: keep it there.
                self.scroll(to: anchor, animated: false, report: false)
            } else if above {
                scrollingProgrammatically = true
                scroll.contentView.scroll(to: NSPoint(x: 0, y: max(0, top + after - before)))
                scroll.reflectScrolledClipView(scroll.contentView)
                DispatchQueue.main.async { [weak self] in self?.scrollingProgrammatically = false }
            }
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
                // TextKit 2 estimates the height of text it hasn't laid out,
                // and the estimate moves; laying out around the last
                // character puts the end where the view's height says it is.
                textView.scrollRangeToVisible(NSRange(location: textView.textStorage?.length ?? 0, length: 0))
                y = textView.frame.height
                // Before the view is in a window it has no width to lay out in.
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
                let quote = attrs[.eddyQuote] as? Int ?? 0
                if attrs[.eddyCode] != nil || quote > 0 {
                    let f = DecoratedFragment(textElement: textElement, range: textElement.elementRange)
                    f.kind = attrs[.eddyCode].map { .code($0 as? CodeBlockMark) } ?? .plain
                    f.quoteDepth = quote
                    f.nextQuoteDepth = attrs[.eddyQuoteNext] as? Int ?? 0
                    return f
                }
            }
            return NSTextLayoutFragment(textElement: textElement, range: textElement.elementRange)
        }

        func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
            if let url = link as? URL ?? (link as? String).flatMap(URL.init(string:)) {
                if url.scheme == "epub" {
                    onBookLink?(url)
                } else if url.scheme == "eddy-media" {
                    onMediaLink?(url)
                } else if let onWebLink, url.isWebLink, !(NSApp.currentEvent?.modifierFlags.contains(.command) ?? false) {
                    onWebLink(url)
                } else {
                    NSWorkspace.shared.open(url)
                }
                return true
            }
            return false
        }
    }
}

/// Hands scrolling to the edge gesture tracker first: a pull past an end or
/// a sideways swipe doesn't scroll. What is left glides on after the fingers
/// lift.
final class ReaderScrollView: NSScrollView {
    weak var tracker: EdgeGestureTracker?
    let momentum = LongMomentum()

    override func scrollWheel(with event: NSEvent) {
        if tracker?.handle(event, in: self) == true {
            momentum.stop()
            return
        }
        if momentum.handle(event, in: self) { return }
        super.scrollWheel(with: event)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { momentum.stop() }
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
    weak var tracker: EdgeGestureTracker?
    var onStep: ((Direction) -> Void)?
    var onEscape: (() -> Void)?
    var onFocus: (() -> Void)?
    var codeLanguages: [String] = []
    var onCodeLanguage: ((CodeBlockMark, String?) -> Void)?
    /// Takes keyboard focus when this turns on or the view reaches a window.
    var wantsFocus = false {
        didSet { if wantsFocus, !oldValue { claimFocus(self) } }
    }

    // MARK: Code block languages

    /// A click on the language label in a code block's corner opens the
    /// menu of languages instead of placing the insertion point.
    override func mouseDown(with event: NSEvent) {
        if let mark = codeMark(at: event) {
            showLanguageMenu(for: mark, with: event)
            return
        }
        super.mouseDown(with: event)
    }

    private func codeMark(at event: NSEvent) -> CodeBlockMark? {
        guard let fragment = codeFragment(at: event), let mark = fragment.codeMark else { return nil }
        let inView = convert(event.locationInWindow, from: nil)
        let local = CGPoint(
            x: inView.x - textContainerOrigin.x - fragment.layoutFragmentFrame.minX,
            y: inView.y - textContainerOrigin.y - fragment.layoutFragmentFrame.minY
        )
        return fragment.labelRect.contains(local) ? mark : nil
    }

    /// The code block under the pointer, if any.
    private func codeFragment(at event: NSEvent) -> DecoratedFragment? {
        guard let tlm = textLayoutManager else { return nil }
        let inView = convert(event.locationInWindow, from: nil)
        let inContainer = CGPoint(x: inView.x - textContainerOrigin.x, y: inView.y - textContainerOrigin.y)
        guard let fragment = tlm.textLayoutFragment(for: inContainer) as? DecoratedFragment, fragment.codeMark != nil,
              fragment.layoutFragmentFrame.minY <= inContainer.y, inContainer.y < fragment.layoutFragmentFrame.maxY
        else { return nil }
        return fragment
    }

    // MARK: Sideways scrolling in code blocks

    /// The code block a sideways gesture is scrolling; the gesture stays
    /// with it until it ends, like the direction it started with.
    private var scrollingCode: DecoratedFragment?

    /// Code longer than its block scrolls sideways under the pointer; up
    /// and down, and anything else, scroll the article as usual. A gesture
    /// is sorted when it starts (or every event, for a mouse wheel) and
    /// its momentum follows it.
    override func scrollWheel(with event: NSEvent) {
        if event.phase == .began || (event.phase.isEmpty && event.momentumPhase.isEmpty) {
            let sideways = abs(event.scrollingDeltaX) > abs(event.scrollingDeltaY)
            scrollingCode = sideways ? codeFragment(at: event).flatMap { $0.codeOverflow > 0 ? $0 : nil } : nil
        }
        guard let fragment = scrollingCode else {
            super.scrollWheel(with: event)
            return
        }
        if fragment.scrollCode(by: -event.scrollingDeltaX) {
            redraw(fragment)
        }
    }

    /// Has a fragment drawn again where it is. The text view keeps each
    /// fragment's drawing in a view of its own under its content view;
    /// invalidating the layout leaves those alone, so they are marked
    /// directly (any neighbour sharing the rows redraws too, harmlessly).
    private func redraw(_ fragment: NSTextLayoutFragment) {
        let frame = fragment.layoutFragmentFrame.offsetBy(dx: textContainerOrigin.x, dy: textContainerOrigin.y)
        for content in subviews where content.frame.intersects(frame) {
            for element in content.subviews where element.frame.intersects(frame) {
                element.needsDisplay = true
            }
        }
    }

    private final class LanguagePick: NSObject {
        let mark: CodeBlockMark
        let language: String?
        init(mark: CodeBlockMark, language: String?) {
            self.mark = mark
            self.language = language
        }
    }

    private func showLanguageMenu(for mark: CodeBlockMark, with event: NSEvent) {
        let menu = NSMenu()
        let current = mark.language?.lowercased()
        func add(_ title: String, _ token: String?, checked: Bool) {
            let item = NSMenuItem(title: title, action: #selector(pickLanguage(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = LanguagePick(mark: mark, language: token)
            item.state = checked ? .on : .off
            menu.addItem(item)
        }
        add(String(localized: "Automatic"), nil, checked: false)
        add(String(localized: "Plain Text"), "plaintext", checked: CodeBlockMark.isPlain(current))
        menu.addItem(.separator())
        var languages = codeLanguages
        // The block's own language, when it isn't one offered (a grammar
        // the page named that is highlighted by its nearest relative).
        if let current, !CodeBlockMark.isPlain(current), !languages.contains(current) {
            languages.insert(current, at: 0)
        }
        for language in languages {
            add(CodeBlockMark.displayName(language), language, checked: language == current)
        }
        NSMenu.popUpContextMenu(menu, with: event, for: self)
    }

    @objc private func pickLanguage(_ sender: NSMenuItem) {
        guard let pick = sender.representedObject as? LanguagePick else { return }
        onCodeLanguage?(pick.mark, pick.language)
    }

    /// The side to arrive from, kept until the view is in a window.
    private var arrivingFrom: ArrivalSide?

    /// The text is about to be shown for the first time: it comes in from
    /// `side`, now or once the view is in a window.
    func arrive(from side: ArrivalSide) {
        if window != nil {
            ReaderMotion.float(self, from: side)
        } else {
            arrivingFrom = side
        }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil, let side = arrivingFrom {
            arrivingFrom = nil
            ReaderMotion.float(self, from: side)
        }
        if wantsFocus { claimFocus(self) }
        // An article is given its text before the view is in a window, and
        // text laid out then gets no views for its attachments: a table on
        // the first screen stayed blank until it was scrolled away and
        // back. Laying the visible part out again, once in the window,
        // brings them in.
        guard window != nil else { return }
        DispatchQueue.main.async { [weak self] in
            self?.textLayoutManager?.textViewportLayoutController.layoutViewport()
        }
    }

    override func becomeFirstResponder() -> Bool {
        let became = super.becomeFirstResponder()
        if became { onFocus?() }
        return became
    }

    /// The width left for the lines is about to change (false), or has
    /// (true): they break anew.
    var onReflow: ((_ done: Bool) -> Void)?

    override func setFrameSize(_ newSize: NSSize) {
        // Wider than the longest line allowed, only the margins change.
        let lineWidth = { (width: CGFloat) in min(width - 56, self.maxWidth) }
        let reflows = abs(lineWidth(newSize.width) - lineWidth(frame.width)) > 0.5
        if reflows { onReflow?(false) }
        super.setFrameSize(newSize)
        updateInsets()
        if reflows { onReflow?(true) }
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
            tracker?.cancel()
            onStep(key == .leftArrow ? .previous : .next)
        } else if modifiers.isEmpty, key == .downArrow || key == .rightArrow {
            if scroll.atBottom {
                tracker?.hold(.next, key: event.keyCode)
            } else {
                scrollBy(key == .downArrow ? lineStep : page, animated: key == .rightArrow)
            }
        } else if modifiers.isEmpty, key == .upArrow || key == .leftArrow {
            if scroll.atTop {
                tracker?.hold(.previous, key: event.keyCode)
            } else {
                scrollBy(key == .upArrow ? -lineStep : -page, animated: key == .leftArrow)
            }
        } else {
            super.keyDown(with: event)
        }
    }

    override func keyUp(with event: NSEvent) {
        tracker?.release(key: event.keyCode)
        super.keyUp(with: event)
    }

    /// Esc closes the find bar, then cancels a pull, then goes to the reader
    /// (which cancels a translation, else leaves the text).
    override func cancelOperation(_ sender: Any?) {
        if let scroll = enclosingScrollView, scroll.isFindBarVisible {
            scroll.isFindBarVisible = false
        } else if tracker?.active == true {
            tracker?.cancel()
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
