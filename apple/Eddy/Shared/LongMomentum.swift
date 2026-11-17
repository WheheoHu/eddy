import AppKit
import SwiftUI

/// Carries a vertical scroll on after the fingers lift, for longer than the
/// system does, and springs back from the ends of the content.
///
/// Once the fingers lift, the system sends its own momentum as scroll
/// events. This takes those: the first few say how fast the view was
/// moving, and from there the view glides on its own, losing speed more
/// slowly, a frame at a time. Fingers back on the trackpad stop it.
@MainActor
final class LongMomentum: NSObject {
    /// How long the glide takes to lose about two thirds of its speed, in
    /// seconds. The system's own is near half a second; the distance
    /// travelled grows with it.
    static let timeConstant: Double = 0.85
    /// Points per second lost every second on top of that. It matters only
    /// at the end, where it brings the glide to rest instead of leaving it
    /// to creep for seconds.
    private static let friction: CGFloat = 90
    /// Points per second under which the glide is over.
    private static let restSpeed: CGFloat = 10
    /// The spring that brings the view back from past an end: stiffer comes
    /// back sooner and goes less far.
    private static let stiffness: CGFloat = 190
    /// The fastest the view goes past an end, so a hard flick near the top
    /// doesn't throw the text half a screen away.
    private static let edgeSpeed: CGFloat = 2400

    /// Called when a glide starts and when it ends.
    var onActiveChange: ((Bool) -> Void)?
    private(set) var isActive = false

    private weak var scrollView: NSScrollView?
    private var link: CADisplayLink?
    /// Where the view is, in points down its content (past an end while it
    /// springs back), and how fast that is changing per second.
    private var position: CGFloat = 0
    private var velocity: CGFloat = 0
    private var lastStep: CFTimeInterval = 0
    /// Where the view was last put, and how tall its content was. The
    /// view somewhere else means something else scrolled it: with the
    /// content a new height (an image loaded above), to keep the text in
    /// place, and the glide goes on from there; otherwise a jump (to a
    /// heading, to a restored position), and the glide gives way.
    private var placed: CGFloat?
    private var contentHeight: CGFloat = 0

    /// The system's momentum, as far as it has been read.
    private enum Momentum {
        case none
        /// Sideways, or in a view with nowhere to go: the view's own.
        case passing
        /// Reading the speed: since when, how far, over how many events.
        case measuring(since: TimeInterval, moved: CGFloat, events: Int)
        case gliding
    }

    private var momentum = Momentum.none

    /// Whether the event was taken; one that wasn't goes on to the view.
    func handle(_ event: NSEvent, in scrollView: NSScrollView) -> Bool {
        self.scrollView = scrollView
        guard !event.momentumPhase.isEmpty else {
            // Fingers on the trackpad, or a wheel: the view is theirs.
            stop()
            momentum = .none
            return false
        }
        // Content moves with the fingers: down the page as they go up.
        let flipped = scrollView.documentView?.isFlipped ?? true
        let delta = flipped ? -event.scrollingDeltaY : event.scrollingDeltaY
        if event.momentumPhase.contains(.began) {
            stop()
            let (low, high) = limits(of: scrollView)
            guard abs(event.scrollingDeltaY) >= abs(event.scrollingDeltaX), high - low > 1 else {
                momentum = .passing
                return false
            }
            position = scrollView.contentView.bounds.minY
            move(by: delta, in: scrollView)
            momentum = .measuring(since: event.timestamp, moved: 0, events: 0)
            return true
        }
        switch momentum {
        case .none, .passing:
            return false
        case let .measuring(since, moved, events):
            guard !event.momentumPhase.contains(.ended) else {
                momentum = .none
                return true
            }
            move(by: delta, in: scrollView)
            let (moved, events, elapsed) = (moved + delta, events + 1, event.timestamp - since)
            // Two events can come a moment apart or a frame apart; a few
            // of them give a speed that doesn't jump about.
            if events >= 3 || elapsed >= 0.03, elapsed > 0.001 {
                momentum = .gliding
                glide(at: moved / CGFloat(elapsed), in: scrollView)
            } else {
                momentum = .measuring(since: since, moved: moved, events: events)
            }
            return true
        case .gliding:
            if event.momentumPhase.contains(.ended) { momentum = .none }
            return true
        }
    }

    func stop() {
        link?.invalidate()
        link = nil
        velocity = 0
        if let scrollView, placed != nil { stretch(0, in: scrollView) }
        placed = nil
        guard isActive else { return }
        isActive = false
        onActiveChange?(false)
    }

    // MARK: The glide

    private func glide(at speed: CGFloat, in scrollView: NSScrollView) {
        guard abs(speed) > Self.restSpeed else { return }
        velocity = speed
        lastStep = CACurrentMediaTime()
        let link = scrollView.displayLink(target: self, selector: #selector(step))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
        link.add(to: .main, forMode: .common)
        self.link = link
        scrollView.flashScrollers()
        isActive = true
        onActiveChange?(true)
    }

    @objc private func step(_ link: CADisplayLink) {
        guard let scrollView, let placed, scrollView.window != nil else {
            stop()
            return
        }
        let moved = scrollView.contentView.bounds.minY - placed
        if abs(moved) >= 1 {
            guard scrollView.documentView?.frame.height != contentHeight else {
                stop()
                return
            }
            position += moved
        }
        let now = CACurrentMediaTime()
        // A stalled frame doesn't become a leap.
        let dt = CGFloat(min(now - lastStep, 1.0 / 30))
        lastStep = now
        let (low, high) = limits(of: scrollView)
        let edge = min(max(position, low), high)
        let past = position - edge
        if past != 0 {
            guard scrollView.verticalScrollElasticity != .none else {
                place(edge, in: scrollView)
                stop()
                return
            }
            // Past an end: a spring pulls back to it, damped so that it
            // arrives without swinging through.
            let damping = 2 * Self.stiffness.squareRoot()
            velocity += (-Self.stiffness * past - damping * velocity) * dt
            position += velocity * dt
            let stillPast = (position - edge) * past > 0
            if !stillPast || (abs(position - edge) < 0.5 && abs(velocity) < Self.restSpeed * 2) {
                place(edge, in: scrollView)
                stop()
                return
            }
        } else {
            position += velocity * dt
            velocity *= CGFloat(exp(-Double(dt) / Self.timeConstant))
            velocity -= min(abs(velocity), Self.friction * dt) * (velocity < 0 ? -1 : 1)
            if position < low || position > high {
                velocity = min(max(velocity, -Self.edgeSpeed), Self.edgeSpeed)
            } else if abs(velocity) < Self.restSpeed {
                place(position, in: scrollView)
                stop()
                return
            }
        }
        place(position, in: scrollView)
    }

    /// While the speed is being read: follows the system's own momentum,
    /// within the content.
    private func move(by delta: CGFloat, in scrollView: NSScrollView) {
        let (low, high) = limits(of: scrollView)
        position = min(max(position + delta, low), high)
        place(position, in: scrollView)
    }

    /// The first and last places the view scrolls to, insets included.
    private func limits(of scrollView: NSScrollView) -> (CGFloat, CGFloat) {
        let clip = scrollView.contentView
        var bounds = clip.bounds
        bounds.origin.y = -1_000_000
        let low = clip.constrainBoundsRect(bounds).minY
        bounds.origin.y = 1_000_000
        let high = clip.constrainBoundsRect(bounds).minY
        return (min(low, high), max(low, high))
    }

    /// Puts the view at `y`, on whole pixels so text stays sharp while it
    /// moves. The view itself goes no further than the ends of its
    /// content; the rest of the way is drawn by `stretch`.
    private func place(_ y: CGFloat, in scrollView: NSScrollView) {
        let scale = scrollView.window?.backingScaleFactor ?? 2
        let clip = scrollView.contentView
        let (low, high) = limits(of: scrollView)
        let within = min(max(y, low), high)
        clip.setBoundsOrigin(CGPoint(x: clip.bounds.minX, y: (within * scale).rounded() / scale))
        scrollView.reflectScrolledClipView(clip)
        placed = clip.bounds.minY
        contentHeight = scrollView.documentView?.frame.height ?? 0
        stretch(((y - within) * scale).rounded() / scale, in: scrollView)
    }

    /// Shows the content `past` points beyond an end. A clip view holds
    /// its bounds inside the content, so this moves only what its layer
    /// shows: nothing is laid out, and 0 puts the layer back with the view.
    private func stretch(_ past: CGFloat, in scrollView: NSScrollView) {
        let clip = scrollView.contentView
        guard let layer = clip.layer else { return }
        let y = clip.bounds.minY + past
        guard layer.bounds.minY != y else { return }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        layer.bounds.origin.y = y
        // The view's background is drawn only as far as its content goes;
        // the gap past it takes the same colour for as long as it shows.
        if scrollView.drawsBackground {
            if past == 0 {
                layer.backgroundColor = nil
            } else if layer.backgroundColor == nil {
                scrollView.effectiveAppearance.performAsCurrentDrawingAppearance {
                    layer.backgroundColor = scrollView.backgroundColor.cgColor
                }
            }
        }
        CATransaction.commit()
    }
}

/// Gives a SwiftUI `ScrollView` the long glide: put it in the background of
/// the scroll view's content. SwiftUI's scroll view can't be subclassed, so
/// this watches the window's scroll events for the ones over it.
struct LongMomentumScrolling: NSViewRepresentable {
    /// A glide started or ended.
    var onGlide: (Bool) -> Void = { _ in }

    func makeNSView(context: Context) -> Watcher { Watcher() }

    func updateNSView(_ view: Watcher, context: Context) {
        view.momentum.onActiveChange = onGlide
    }

    final class Watcher: NSView {
        let momentum = LongMomentum()
        private var monitor: Any?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
            guard window != nil else {
                momentum.stop()
                return
            }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
                let taken = MainActor.assumeIsolated { self?.take(event) == true }
                return taken ? nil : event
            }
        }

        /// Whether the event is for this scroll view and the glide took it.
        /// Not for it: another window's, or over something lying on top
        /// (the media viewer).
        private func take(_ event: NSEvent) -> Bool {
            guard let window, event.window === window, let scroll = enclosingScrollView,
                  let content = window.contentView
            else { return false }
            let point = content.superview?.convert(event.locationInWindow, from: nil) ?? event.locationInWindow
            guard let hit = content.hitTest(point), hit.isDescendant(of: scroll) else {
                // Fingers down anywhere end a glide; the event stays theirs.
                if event.momentumPhase.isEmpty { momentum.stop() }
                return false
            }
            return momentum.handle(event, in: scroll)
        }
    }
}
