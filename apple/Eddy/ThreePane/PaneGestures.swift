import AppKit
import SwiftUI

/// Sideways swipes and row clicks over three-pane's columns, where they
/// move between the columns: laid over the container, it takes no clicks
/// itself and watches the window's events instead. The lists are SwiftUI's
/// and can't be subclassed; the reader's own views follow their swipes
/// themselves (`GestureContainer`), so those are left alone.
struct PaneSwipes: NSViewRepresentable {
    /// Off while something lies over the window's content.
    var enabled: Bool
    /// How far from the leading edge the columns reach; nil for all the way.
    var columnsExtent: CGFloat?
    /// The reader beside the columns is in use; else it lies under a scrim.
    var readerLive: Bool
    /// How far from the leading edge the sidebar reaches, where a click on
    /// a row of it goes on; nil for all the way.
    var sidebarExtent: CGFloat?
    /// A click on a row of the list goes on.
    var listLeadsOn: Bool
    /// What a swipe would do, with the pointer over the columns or not.
    var target: (SwipeDirection, _ overColumns: Bool) -> EdgeTarget
    var commit: (SwipeDirection, _ overColumns: Bool) -> Void
    /// A row of the sidebar or the list was clicked, selected already or not.
    var rowClicked: (PanePlace) -> Void

    func makeNSView(context: Context) -> Watcher { Watcher() }

    func updateNSView(_ view: Watcher, context: Context) {
        view.settings = self
    }

    final class Watcher: NSView {
        var settings: PaneSwipes?

        private let tracker = EdgeGestureTracker()
        private let indicator = NSHostingView(rootView: EdgeIndicator(gesture: nil))
        private var indicatorSize: (label: String, size: CGSize)?
        private var monitors: [Any] = []

        /// Whose the gesture in progress is, decided where it starts.
        private enum Territory {
            /// Not ours: it scrolls, or the reader's own view follows it.
            case passing
            /// Over the columns, or over a reader view that follows no
            /// swipes of its own: ours when it is a swipe that leads
            /// somewhere, scrolling otherwise.
            case shared
            /// Over the scrim: nothing under it scrolls.
            case scrim
        }
        private var territory = Territory.passing
        private var overColumns = false
        /// A row pressed, until the button is up again.
        private var pressed: (at: CGPoint, column: PanePlace)?

        override init(frame: NSRect) {
            super.init(frame: frame)
            indicator.isHidden = true
            addSubview(indicator)
            tracker.gestures = EdgeGestures(
                swipeTarget: { [weak self] fingers in
                    guard let self, let settings = self.settings else { return .none }
                    return settings.target(fingers, self.overColumns)
                },
                commit: { [weak self] kind in
                    guard let self, case let .swipe(fingers) = kind else { return }
                    self.settings?.commit(fingers, self.overColumns)
                }
            )
            tracker.onChange = { [weak self] gesture, _ in
                MainActor.assumeIsolated { self?.show(gesture) }
            }
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError() }

        override var isFlipped: Bool { true }

        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            monitors.forEach(NSEvent.removeMonitor)
            monitors = []
            tracker.cancel()
            pressed = nil
            guard window != nil else { return }
            monitors = [
                NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
                    let taken = MainActor.assumeIsolated {
                        guard let self, event.window === self.window else { return false }
                        return self.take(event, at: event.locationInWindow)
                    }
                    return taken ? nil : event
                },
                NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .leftMouseUp]) { [weak self] event in
                    MainActor.assumeIsolated {
                        if event.type == .leftMouseDown { self?.press(event) } else { self?.settle() }
                    }
                    return event
                },
            ].compactMap { $0 }
        }

        // MARK: Swipes

        /// Whether the event, at `location` in the window, is part of a
        /// swipe between the columns, and must not scroll.
        func take(_ event: NSEvent, at location: CGPoint) -> Bool {
            let phased = !event.phase.isEmpty || !event.momentumPhase.isEmpty
            if event.phase.contains(.began) || event.phase.contains(.mayBegin) || (!phased && !tracker.active) {
                territory = territory(at: location)
            }
            switch territory {
            case .passing:
                return false
            case .shared:
                return tracker.handle(event, in: nil)
            case .scrim:
                _ = tracker.handle(event, in: nil)
                return true
            }
        }

        private func territory(at location: CGPoint) -> Territory {
            guard let settings, settings.enabled, let window else { return .passing }
            let point = convert(location, from: nil)
            guard bounds.contains(point) else { return .passing }
            overColumns = settings.columnsExtent.map { point.x < $0 } ?? true
            if overColumns { return .shared }
            guard settings.readerLive else { return .scrim }
            guard let hit = Self.view(at: location, in: window) else { return .passing }
            var view: NSView? = hit
            while let current = view {
                // The text and the web page follow their own swipes.
                if current is GestureContainer { return .passing }
                // A zoomed PDF or a strip of pictures scrolls sideways.
                if let scroll = current as? NSScrollView, let document = scroll.documentView,
                   document.frame.width > scroll.contentView.bounds.width + 1 { return .passing }
                view = current.superview
            }
            return .shared
        }

        private static func view(at location: CGPoint, in window: NSWindow) -> NSView? {
            guard let content = window.contentView else { return nil }
            return content.hitTest(content.superview?.convert(location, from: nil) ?? location)
        }

        /// The capsule, at the edge the columns come from or leave by.
        private func show(_ gesture: EdgeGesture?) {
            guard let gesture, case let .swipe(fingers) = gesture.kind else {
                indicator.isHidden = true
                return
            }
            indicator.rootView = EdgeIndicator(gesture: gesture)
            let label = EdgeIndicator.label(gesture)
            if indicatorSize?.label != label { indicatorSize = (label, indicator.fittingSize) }
            let size = indicatorSize?.size ?? indicator.fittingSize
            let extent = min(settings?.columnsExtent ?? bounds.width, bounds.width)
            let span = overColumns ? 0...max(extent, size.width + 24) : extent...max(bounds.width, extent + size.width + 24)
            let x = fingers == .right ? span.lowerBound + 12 : span.upperBound - size.width - 12
            indicator.frame = CGRect(x: x, y: bounds.midY - size.height / 2, width: size.width, height: size.height)
            indicator.isHidden = false
        }

        // MARK: Row clicks

        /// A list reports a click only when it changes the selection; where
        /// a column covers the next one, a click on the row selected already
        /// goes on to it as well.
        private func press(_ event: NSEvent) {
            pressed = nil
            guard let settings, settings.enabled, let window, event.window === window,
                  !event.modifierFlags.contains(.control)
            else { return }
            let point = convert(event.locationInWindow, from: nil)
            guard bounds.contains(point), let column = column(at: point.x, settings),
                  let hit = Self.view(at: event.locationInWindow, in: window)
            else { return }
            var row: NSTableRowView?
            var view: NSView? = hit
            while let current = view {
                // A folder's disclosure triangle folds it, nothing more.
                if current is NSButton { return }
                if let found = current as? NSTableRowView {
                    row = found
                    break
                }
                view = current.superview
            }
            guard let row, !row.isGroupRowStyle else { return }
            pressed = (NSEvent.mouseLocation, column)
            // A table follows the mouse in a loop of its own and takes the
            // button coming up with it; this runs once that loop is over.
            RunLoop.main.perform(inModes: [.default]) { [weak self] in
                MainActor.assumeIsolated { self?.settle() }
            }
        }

        private func column(at x: CGFloat, _ settings: PaneSwipes) -> PanePlace? {
            if settings.sidebarExtent.map({ x < $0 }) ?? true { return .sidebar }
            if settings.listLeadsOn, settings.columnsExtent.map({ x < $0 }) ?? true { return .list }
            return nil
        }

        /// The button is up: a click, unless the pointer moved (a drag).
        private func settle() {
            guard let pressed, NSEvent.pressedMouseButtons & 1 == 0 else { return }
            self.pressed = nil
            let now = NSEvent.mouseLocation
            guard hypot(now.x - pressed.at.x, now.y - pressed.at.y) < 4 else { return }
            settings?.rowClicked(pressed.column)
        }
    }
}
