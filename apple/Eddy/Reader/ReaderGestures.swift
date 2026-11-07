import AppKit
import SwiftUI

/// A pull past the top or bottom of the text, towards the previous or next
/// chapter or article.
struct Pull: Equatable {
    var direction: Direction
    /// 0 when the pull starts, 1 once letting go would move on.
    var progress: Double
    /// What letting go opens; nil at the first or last item.
    var target: String?

    var armed: Bool { progress >= 1 && target != nil }
}

/// Follows pulls past the edges of the reader's scroll view. On a trackpad
/// or Magic Mouse only finger movement after the edge is reached counts,
/// never momentum, and the move happens when the fingers lift. A plain wheel
/// moves on once it has been still for a moment. A held arrow key fills
/// over a fixed time and moves on when released.
@MainActor
final class PullTracker {
    static let threshold: CGFloat = 90
    static let holdDuration: TimeInterval = 0.7
    static let wheelIdle: TimeInterval = 0.4
    /// Points per line for wheels that scroll by lines.
    static let lineDelta: CGFloat = 12

    /// Title of what a pull would open; nil turns pulls off.
    var target: ((Direction) -> String?)?
    var onChange: ((Pull?) -> Void)?
    var onRelease: ((Direction) -> Void)?

    private var pull: Pull? {
        didSet { if pull != oldValue { onChange?(pull) } }
    }
    private var distance: CGFloat = 0
    private var feltThreshold = false
    private var idleTimer: Timer?
    private var holdTimer: Timer?
    private var holdStart: Date?
    private var holdKey: UInt16?

    var active: Bool { pull != nil }

    // MARK: Wheel and trackpad

    /// Called after the scroll view has scrolled, so its edges are current.
    func scrollWheel(_ event: NSEvent, in scroll: NSScrollView) {
        guard target != nil, holdKey == nil, event.momentumPhase.isEmpty else { return }
        let phased = !event.phase.isEmpty
        if phased {
            if event.phase.contains(.began) || event.phase.contains(.mayBegin) {
                reset()
                return
            }
            if event.phase.contains(.cancelled) {
                reset()
                return
            }
            if event.phase.contains(.ended) {
                finish()
                return
            }
        }
        var dy = event.scrollingDeltaY
        if !event.hasPreciseScrollingDeltas { dy *= Self.lineDelta }
        guard dy != 0 else { return }
        if pull == nil {
            let towards: Direction = dy < 0 ? .next : .previous
            let atEdge = towards == .next ? scroll.atBottom : scroll.atTop
            guard atEdge, abs(event.scrollingDeltaY) >= abs(event.scrollingDeltaX) else { return }
            start(towards)
        }
        guard let direction = pull?.direction else { return }
        distance += direction == .next ? -dy : dy
        if distance <= 0 {
            reset()
            return
        }
        update(Double(distance / Self.threshold), haptic: phased)
        if !phased {
            idleTimer?.invalidate()
            let timer = Timer(timeInterval: Self.wheelIdle, repeats: false) { [weak self] _ in
                MainActor.assumeIsolated { self?.finish() }
            }
            RunLoop.main.add(timer, forMode: .common)
            idleTimer = timer
        }
    }

    // MARK: Keys

    /// An arrow key pressed at the edge it points past. Repeats of the same
    /// key keep the hold going; it fills on a timer, not with the repeats.
    func hold(_ direction: Direction, key: UInt16) {
        guard target != nil else { return }
        if holdKey == key, pull?.direction == direction { return }
        start(direction)
        holdKey = key
        holdStart = Date()
        holdTimer?.invalidate()
        let timer = Timer(timeInterval: 1.0 / 60, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tickHold() }
        }
        RunLoop.main.add(timer, forMode: .common)
        holdTimer = timer
    }

    /// Letting go of a held key moves on when the pull is full.
    func release(key: UInt16) {
        guard holdKey == key else { return }
        finish()
    }

    private func tickHold() {
        guard let holdStart else { return }
        update(Date().timeIntervalSince(holdStart) / Self.holdDuration, haptic: false)
    }

    // MARK: State

    private func start(_ direction: Direction) {
        reset()
        pull = Pull(direction: direction, progress: 0, target: target?(direction))
    }

    private func update(_ progress: Double, haptic: Bool) {
        guard var pull else { return }
        pull.progress = min(progress, 1)
        if pull.armed, !feltThreshold {
            feltThreshold = true
            if haptic { NSHapticFeedbackManager.defaultPerformer.perform(.levelChange, performanceTime: .now) }
        } else if !pull.armed {
            feltThreshold = false
        }
        self.pull = pull
    }

    private func finish() {
        let armed = pull?.armed == true ? pull?.direction : nil
        reset()
        if let armed { onRelease?(armed) }
    }

    func reset() {
        idleTimer?.invalidate()
        idleTimer = nil
        holdTimer?.invalidate()
        holdTimer = nil
        holdStart = nil
        holdKey = nil
        distance = 0
        feltThreshold = false
        pull = nil
    }
}

/// Which way the fingers move in a horizontal swipe.
enum SwipeDirection {
    case left, right
}

/// Recognizes horizontal swipes in scroll events, for the reader and the
/// web view. A gesture is horizontal when it starts out clearly sideways;
/// it then fires once, past a threshold, and none of it scrolls.
@MainActor
final class SwipeTracker {
    static let threshold: CGFloat = 90
    /// How long a plain wheel's sideways movement adds up.
    static let wheelWindow: TimeInterval = 0.3

    var onSwipe: ((SwipeDirection) -> Void)?

    private enum Axis { case undecided, horizontal, vertical }
    private var axis = Axis.undecided
    private var dx: CGFloat = 0
    private var dy: CGFloat = 0
    private var fired = false
    private var lastWheel: Date?

    /// Whether the event belongs to a swipe, and must not scroll.
    func handle(_ event: NSEvent) -> Bool {
        guard onSwipe != nil else { return false }
        if !event.momentumPhase.isEmpty { return axis == .horizontal }
        let phased = !event.phase.isEmpty
        if phased {
            if event.phase.contains(.began) || event.phase.contains(.mayBegin) { reset() }
            if event.phase.contains(.ended) || event.phase.contains(.cancelled) {
                return axis == .horizontal
            }
        } else if lastWheel.map({ Date().timeIntervalSince($0) > Self.wheelWindow }) ?? true {
            reset()
        }
        if !phased { lastWheel = Date() }
        var x = event.scrollingDeltaX
        var y = event.scrollingDeltaY
        if !event.hasPreciseScrollingDeltas {
            x *= PullTracker.lineDelta
            y *= PullTracker.lineDelta
        }
        dx += x
        dy += y
        if axis == .undecided, abs(dx) + abs(dy) > 8 {
            axis = abs(dx) > 2 * abs(dy) ? .horizontal : .vertical
        }
        guard axis == .horizontal else { return false }
        if !fired, abs(dx) >= Self.threshold {
            fired = true
            // Natural scrolling reports content movement, which follows the
            // fingers; a plain wheel scrolling right reports negative x.
            let fingersRight = phased ? (event.isDirectionInvertedFromDevice ? dx > 0 : dx < 0) : dx > 0
            onSwipe?(fingersRight ? .right : .left)
        }
        return true
    }

    private func reset() {
        axis = .undecided
        dx = 0
        dy = 0
        fired = false
    }
}

extension NSScrollView {
    /// The top of the document is in view (or pulled past).
    var atTop: Bool {
        contentView.bounds.minY <= -contentInsets.top + 1
    }

    /// The bottom of the document is in view (or pulled past); always true
    /// for a document shorter than the view.
    var atBottom: Bool {
        guard let document = documentView else { return false }
        return contentView.bounds.maxY >= document.frame.height + contentInsets.bottom - 1
    }
}

/// Shows a pull at the edge it goes past: how far it has come, what letting
/// go opens, or that there is nothing more.
struct PullIndicator: View {
    let pull: Pull
    let kind: Neighbors.Kind

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: pull.direction == .next ? "arrow.down.circle" : "arrow.up.circle")
                .symbolVariant(pull.armed ? .fill : .none)
                .foregroundStyle(pull.armed ? Color.accentColor : .secondary)
            Text(text).lineLimit(1)
        }
        .font(.callout)
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background {
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Rectangle().fill(.regularMaterial)
                    if pull.target != nil {
                        Rectangle()
                            .fill(Color.accentColor.opacity(pull.armed ? 0.25 : 0.15))
                            .frame(width: geo.size.width * pull.progress)
                    }
                }
            }
        }
        .clipShape(Capsule())
        .shadow(color: .black.opacity(0.1), radius: 8, y: 2)
        .frame(maxWidth: 460)
        .padding(16)
        .accessibilityElement(children: .combine)
    }

    private var text: String {
        guard let target = pull.target else {
            switch (kind, pull.direction) {
            case (.chapter, .next): return String(localized: "This is the last chapter")
            case (.chapter, .previous): return String(localized: "This is the first chapter")
            case (.article, .next): return String(localized: "This is the last article")
            case (.article, .previous): return String(localized: "This is the first article")
            }
        }
        switch (pull.direction, pull.armed) {
        case (.next, true): return String(localized: "Release for next: \(target)")
        case (.next, false): return String(localized: "Next: \(target)")
        case (.previous, true): return String(localized: "Release for previous: \(target)")
        case (.previous, false): return String(localized: "Previous: \(target)")
        }
    }
}
