import AppKit
import SwiftUI

/// Which way the fingers move in a sideways swipe.
enum SwipeDirection {
    case left, right
}

/// What a gesture past an edge would do.
enum EdgeTarget: Equatable {
    /// Nothing handles it: the gesture scrolls as usual.
    case none
    /// Nothing lies that way; the capsule says so and letting go does nothing.
    case end(String)
    /// Letting go opens something; `release` is shown once it would.
    case item(String, release: String)
    /// Letting go brings something in over the view, which stays where it
    /// is: only the capsule shows the gesture.
    case reveal(String, release: String)

    /// Letting go past the threshold does something.
    var acts: Bool {
        switch self {
        case .item, .reveal: true
        case .none, .end: false
        }
    }
}

/// A pull past the top or bottom of the text, or a sideways swipe, in
/// progress.
struct EdgeGesture: Equatable {
    enum Kind: Equatable {
        case pull(Direction)
        case swipe(SwipeDirection)
    }

    var kind: Kind
    /// 0 when it starts, 1 once letting go would act.
    var progress: Double
    var target: EdgeTarget

    var armed: Bool { progress >= 1 && target.acts }
}

/// What a reader view does with gestures past its edges. A nil target
/// function turns that kind of gesture off.
struct EdgeGestures {
    var pullTarget: ((Direction) -> EdgeTarget)?
    var swipeTarget: ((SwipeDirection) -> EdgeTarget)?
    var commit: (EdgeGesture.Kind) -> Void = { _ in }
}

/// Follows pulls past the top and bottom of the text and sideways swipes.
/// Each gesture is classified once: sideways when it starts out clearly
/// sideways, a pull when it pushes past an edge it has reached. On a
/// trackpad or Magic Mouse only finger movement counts, never momentum, and
/// the gesture acts when the fingers lift; a plain wheel acts once it has
/// been still for a moment; a held arrow key fills over a fixed time and
/// acts when released. Nothing reaches SwiftUI until a gesture acts.
@MainActor
final class EdgeGestureTracker {
    static let threshold: CGFloat = 180
    static let holdDuration: TimeInterval = 0.7
    static let wheelIdle: TimeInterval = 0.4
    /// Points per line for wheels that scroll by lines.
    static let lineDelta: CGFloat = 12

    var gestures = EdgeGestures()
    /// The gesture as it moves; nil when it ends, with whether it acted.
    var onChange: ((EdgeGesture?, _ committed: Bool) -> Void)?

    private var gesture: EdgeGesture? {
        didSet { if let gesture, gesture != oldValue { onChange?(gesture, false) } }
    }
    private enum Axis { case undecided, horizontal, vertical }
    private var axis = Axis.undecided
    private var classifyX: CGFloat = 0
    private var classifyY: CGFloat = 0
    private var distance: CGFloat = 0
    private var feltThreshold = false
    /// The rest of a trackpad gesture that was ours, momentum included.
    private var ownsGesture = false
    private var lastWheel: Date?
    private var idleTimer: Timer?
    private var holdTimer: Timer?
    private var holdStart: Date?
    private var holdKey: UInt16?

    var active: Bool { gesture != nil }

    /// A gesture is following the fingers somewhere; heavy work waits.
    private(set) static var anyActive = false

    // MARK: Wheel and trackpad

    /// Whether the event belongs to an edge gesture, and must not scroll.
    /// Called before the view scrolls; `scroll` gives its edges, nil when
    /// pulls don't apply.
    func handle(_ event: NSEvent, in scroll: NSScrollView?) -> Bool {
        guard holdKey == nil else { return false }
        if !event.momentumPhase.isEmpty { return ownsGesture }
        let phased = !event.phase.isEmpty
        if phased {
            if event.phase.contains(.began) || event.phase.contains(.mayBegin) {
                startOver()
            }
            if event.phase.contains(.ended) || event.phase.contains(.cancelled) {
                if gesture != nil { finish(commit: event.phase.contains(.ended)) }
                // The view saw the gesture begin; it sees it end too, or it
                // stays in the middle of a scroll.
                return false
            }
        } else {
            if gesture == nil, lastWheel.map({ Date().timeIntervalSince($0) > Self.wheelIdle }) ?? true {
                startOver()
            }
            lastWheel = Date()
        }
        var dx = event.scrollingDeltaX
        var dy = event.scrollingDeltaY
        if !event.hasPreciseScrollingDeltas {
            dx *= Self.lineDelta
            dy *= Self.lineDelta
        }
        // Natural scrolling reports content movement, which follows the
        // fingers; a plain wheel scrolling right reports negative x and acts
        // like fingers moving left.
        let rightward = phased ? (event.isDirectionInvertedFromDevice ? dx : -dx) : dx
        if gesture == nil, !begin(rightward: rightward, dy: dy, scroll: scroll) { return false }
        guard let kind = gesture?.kind else { return false }
        switch kind {
        case let .pull(direction): distance += direction == .next ? -dy : dy
        case let .swipe(fingers): distance += fingers == .right ? rightward : -rightward
        }
        if distance <= 0 {
            // Back past where it started: an early end, and scrolling resumes,
            // momentum included.
            finish(commit: false)
            ownsGesture = false
            return false
        }
        update(Double(distance / Self.threshold), haptic: phased)
        if !phased { restartIdleTimer() }
        return true
    }

    /// Classifies a gesture that isn't one yet; true once it is.
    private func begin(rightward: CGFloat, dy: CGFloat, scroll: NSScrollView?) -> Bool {
        classifyX += rightward
        classifyY += dy
        if axis == .undecided, abs(classifyX) + abs(classifyY) > 8 {
            axis = abs(classifyX) > 2 * abs(classifyY) ? .horizontal : .vertical
        }
        switch axis {
        case .horizontal:
            guard let swipeTarget = gestures.swipeTarget else { return false }
            let fingers: SwipeDirection = classifyX > 0 ? .right : .left
            let target = swipeTarget(fingers)
            guard target != .none else { return false }
            start(.swipe(fingers), target: target)
            distance = abs(classifyX)
            return true
        case .vertical, .undecided:
            guard let pullTarget = gestures.pullTarget, let scroll, dy != 0 else { return false }
            let towards: Direction = dy < 0 ? .next : .previous
            guard towards == .next ? scroll.atBottom : scroll.atTop else { return false }
            let target = pullTarget(towards)
            guard target != .none else { return false }
            start(.pull(towards), target: target)
            return true
        }
    }

    private func restartIdleTimer() {
        idleTimer?.invalidate()
        let timer = Timer(timeInterval: Self.wheelIdle, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.finish(commit: true) }
        }
        RunLoop.main.add(timer, forMode: .common)
        idleTimer = timer
    }

    // MARK: Keys

    /// An arrow key pressed at the edge it points past. Repeats of the same
    /// key keep the hold going; it fills on a timer, not with the repeats.
    func hold(_ direction: Direction, key: UInt16) {
        guard let pullTarget = gestures.pullTarget else { return }
        if holdKey == key, gesture?.kind == .pull(direction) { return }
        let target = pullTarget(direction)
        guard target != .none else { return }
        start(.pull(direction), target: target)
        holdKey = key
        holdStart = Date()
        let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tickHold() }
        }
        RunLoop.main.add(timer, forMode: .common)
        holdTimer = timer
    }

    /// Letting go of a held key acts when the pull is full.
    func release(key: UInt16) {
        guard holdKey == key else { return }
        finish(commit: true)
    }

    private func tickHold() {
        guard let holdStart else { return }
        distance = CGFloat(Date().timeIntervalSince(holdStart) / Self.holdDuration) * Self.threshold
        update(Double(distance / Self.threshold), haptic: false)
    }

    // MARK: State

    private func startOver() {
        if gesture != nil { finish(commit: false) }
        axis = .undecided
        classifyX = 0
        classifyY = 0
        ownsGesture = false
    }

    private func start(_ kind: EdgeGesture.Kind, target: EdgeTarget) {
        stopTimers()
        distance = 0
        feltThreshold = false
        ownsGesture = true
        Self.anyActive = true
        gesture = EdgeGesture(kind: kind, progress: 0, target: target)
    }

    private func update(_ progress: Double, haptic: Bool) {
        guard var gesture else { return }
        gesture.progress = max(progress, 0)
        if gesture.armed, !feltThreshold {
            feltThreshold = true
            if haptic { NSHapticFeedbackManager.defaultPerformer.perform(.levelChange, performanceTime: .now) }
        } else if !gesture.armed {
            feltThreshold = false
        }
        self.gesture = gesture
    }

    private func finish(commit: Bool) {
        let acted = commit && gesture?.armed == true ? gesture?.kind : nil
        stopTimers()
        Self.anyActive = false
        gesture = nil
        distance = 0
        onChange?(nil, acted != nil)
        if let acted { gestures.commit(acted) }
    }

    private func stopTimers() {
        idleTimer?.invalidate()
        idleTimer = nil
        holdTimer?.invalidate()
        holdTimer = nil
        holdStart = nil
        holdKey = nil
    }

    /// Ends any gesture without acting, as Esc does.
    func cancel() {
        guard gesture != nil else { return }
        finish(commit: false)
    }
}

/// Holds a reader view (text or web page) and moves it with an edge
/// gesture: up or down past the end of the text, sideways for a swipe. The
/// capsule sits in the space that opens. Only the view's origin moves, so
/// nothing is laid out again while the fingers move.
final class GestureContainer: NSView {
    let content: NSView
    let tracker = EdgeGestureTracker()
    private let indicator = NSHostingView(rootView: EdgeIndicator(gesture: nil))
    private var offset = CGPoint.zero

    init(content: NSView) {
        self.content = content
        super.init(frame: .zero)
        wantsLayer = true
        layer?.masksToBounds = true
        addSubview(content)
        indicator.isHidden = true
        addSubview(indicator)
        tracker.onChange = { [weak self] gesture, committed in
            MainActor.assumeIsolated { self?.follow(gesture, committed: committed) }
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    /// Once the content has moved, the gap it leaves (and the capsule in it)
    /// can be under the pointer; scrolling there still belongs to the
    /// gesture, or the gesture never hears the fingers lift.
    override func scrollWheel(with event: NSEvent) {
        if tracker.handle(event, in: content as? NSScrollView) { return }
        content.scrollWheel(with: event)
    }

    /// Sliding in with a transition can leave a strip of the text undrawn;
    /// redraw once the view has come to rest.
    override func setFrameOrigin(_ newOrigin: NSPoint) {
        super.setFrameOrigin(newOrigin)
        redrawTask?.cancel()
        redrawTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(120))
            guard let self, !Task.isCancelled else { return }
            Self.markNeedsDisplay(self.content)
        }
    }

    private var redrawTask: Task<Void, Never>?

    private static func markNeedsDisplay(_ view: NSView) {
        view.needsDisplay = true
        view.subviews.forEach(markNeedsDisplay)
    }

    override func layout() {
        super.layout()
        content.frame = bounds.offsetBy(dx: offset.x, dy: offset.y)
        placeIndicator()
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.underPageBackgroundColor.setFill()
        dirtyRect.fill()
    }

    /// Almost 1:1 up to the threshold, then with growing resistance.
    private static func travel(_ progress: Double) -> CGFloat {
        let full = EdgeGestureTracker.threshold * 0.8
        let p = CGFloat(progress)
        return p <= 1 ? p * full : full + 60 * (1 - 1 / (1 + (p - 1) * 2))
    }

    /// The gesture as the tracker last saw it, and as last drawn. Scroll
    /// events can come faster than a frame; each only records the gesture,
    /// and the display link draws the latest once per frame.
    private var wanted: EdgeGesture?
    private var drawn: EdgeGesture?
    private var link: CADisplayLink?
    /// The capsule is measured only when its wording changes.
    private var indicatorSize: (label: String, size: CGSize)?

    private func follow(_ gesture: EdgeGesture?, committed: Bool) {
        guard let gesture else {
            // What comes in over the view covers it; nothing to wait for.
            var stays = false
            if case .reveal = (wanted ?? drawn)?.target { stays = true }
            stopFollowing()
            if committed, !stays {
                // The view leaves with the reader's transition; should it stay,
                // it comes back.
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) { [weak self] in self?.springBack() }
            } else {
                springBack()
            }
            return
        }
        if wanted == nil, drawn == nil {
            // The first frame of a gesture shows at once.
            draw(gesture)
        }
        wanted = gesture
        if link == nil {
            let link = displayLink(target: self, selector: #selector(step))
            link.add(to: .main, forMode: .common)
            self.link = link
        }
    }

    @objc private func step(_ link: CADisplayLink) {
        guard let wanted, wanted != drawn else { return }
        draw(wanted)
    }

    private func stopFollowing() {
        if let wanted, wanted != drawn { draw(wanted) }
        link?.invalidate()
        link = nil
        wanted = nil
        drawn = nil
    }

    private func draw(_ gesture: EdgeGesture) {
        drawn = gesture
        var travel = Self.travel(gesture.progress)
        if case .reveal = gesture.target { travel = 0 }
        offset = switch gesture.kind {
        case .pull(.next): CGPoint(x: 0, y: -travel)
        case .pull(.previous): CGPoint(x: 0, y: travel)
        case .swipe(.left): CGPoint(x: -travel, y: 0)
        case .swipe(.right): CGPoint(x: travel, y: 0)
        }
        content.setFrameOrigin(offset)
        indicator.rootView = EdgeIndicator(gesture: gesture)
        indicator.isHidden = false
        placeIndicator()
    }

    private func springBack() {
        guard offset != .zero || !indicator.isHidden else { return }
        offset = .zero
        indicator.isHidden = true
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.22
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            content.animator().frame = bounds
        }
    }

    /// In the middle of the space the content has left, kept on screen.
    private func placeIndicator() {
        guard !indicator.isHidden, let gesture = indicator.rootView.gesture else { return }
        let label = EdgeIndicator.label(gesture)
        let size: CGSize
        if let cached = indicatorSize, cached.label == label {
            size = cached.size
        } else {
            size = indicator.fittingSize
            indicatorSize = (label, size)
        }
        let gapX = abs(offset.x), gapY = abs(offset.y)
        var origin = CGPoint(x: bounds.midX - size.width / 2, y: bounds.midY - size.height / 2)
        switch gesture.kind {
        case .pull(.next): origin.y = min(bounds.maxY - gapY / 2 - size.height / 2, bounds.maxY - size.height - 12)
        case .pull(.previous): origin.y = max(gapY / 2 - size.height / 2, 12)
        case .swipe(.left): origin.x = min(bounds.maxX - gapX / 2 - size.width / 2, bounds.maxX - size.width - 12)
        case .swipe(.right): origin.x = max(gapX / 2 - size.width / 2, 12)
        }
        indicator.frame = CGRect(origin: origin, size: size)
    }
}

extension NSScrollView {
    /// The top of the document is in view.
    var atTop: Bool {
        contentView.bounds.minY <= -contentInsets.top + 1
    }

    /// The bottom of the document is in view; always true for a document
    /// shorter than the view.
    var atBottom: Bool {
        guard let document = documentView else { return false }
        return contentView.bounds.maxY >= document.frame.height + contentInsets.bottom - 1
    }
}

/// Shows an edge gesture: how far it has come, what letting go does, or
/// that there is nothing more.
struct EdgeIndicator: View {
    let gesture: EdgeGesture?

    var body: some View {
        if let gesture {
            HStack(spacing: 8) {
                Image(systemName: icon(gesture.kind))
                    .symbolVariant(gesture.armed ? .fill : .none)
                    .foregroundStyle(gesture.armed ? Color.accentColor : .secondary)
                Text(Self.label(gesture)).lineLimit(1)
            }
            .font(.callout)
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background {
                GeometryReader { geo in
                    ZStack(alignment: .leading) {
                        Rectangle().fill(.regularMaterial)
                        if gesture.target.acts {
                            Rectangle()
                                .fill(Color.accentColor.opacity(gesture.armed ? 0.25 : 0.15))
                                .frame(width: geo.size.width * min(gesture.progress, 1))
                        }
                    }
                }
            }
            .clipShape(Capsule())
            .frame(maxWidth: 420)
            .accessibilityElement(children: .combine)
        }
    }

    private func icon(_ kind: EdgeGesture.Kind) -> String {
        switch kind {
        case .pull(.next): "arrow.down.circle"
        case .pull(.previous): "arrow.up.circle"
        case .swipe(.left): "arrow.left.circle"
        case .swipe(.right): "arrow.right.circle"
        }
    }

    static func label(_ gesture: EdgeGesture) -> String {
        switch gesture.target {
        case .none: ""
        case let .end(message): message
        case let .item(label, release), let .reveal(label, release): gesture.armed ? release : label
        }
    }
}
