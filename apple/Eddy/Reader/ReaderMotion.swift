import AppKit
import EddyKit
import SwiftUI

/// How the text of an item comes into the reader: it fades in, rising from
/// a little below or settling from a little above, by where the item lies
/// from the one read before. Only the text moves; the strip over it and the
/// columns beside it stay put.
///
/// The move is played on the text view's own layer the moment its text is
/// in: a transition on the pane would move the strip with it, and would
/// play on the progress indicator that stands there until the text comes.
@MainActor
enum ReaderMotion {
    /// How far the text travels.
    static let rise: CGFloat = 14
    static let move: CFTimeInterval = 0.32
    static let fade: CFTimeInterval = 0.22
    /// Before it starts, with the text not showing yet: the scroll to where
    /// reading stopped lands in this.
    static let hold: CFTimeInterval = 0.03
    /// What was open fades out.
    static let leave = Animation.easeOut(duration: 0.14)
    /// Items following closer than this (a key held down) come in plainly.
    static let settle: TimeInterval = 0.3
    /// The wait before a progress indicator shows; most text is in sooner.
    static let spinnerDelay = 0.3

    private static var lastArrival: TimeInterval = 0
    private static var lastSwitch: TimeInterval = 0
    private static var now: TimeInterval { ProcessInfo.processInfo.systemUptime }

    /// Another item was opened a moment ago.
    static var hurrying: Bool { now - lastSwitch < settle }

    static func switched() { lastSwitch = now }

    /// Plays the arrival on a view whose text has just been put in. Nothing
    /// of the view itself changes, so text that misses its animation is
    /// simply there.
    static func float(_ view: NSView, from side: ArrivalSide) {
        let previous = lastArrival
        lastArrival = now
        guard lastArrival - previous >= settle, let layer = view.layer else { return }
        let ease = CAMediaTimingFunction(controlPoints: 0.2, 0.8, 0.2, 1)
        let fadeIn = CABasicAnimation(keyPath: "opacity")
        fadeIn.fromValue = 0
        fadeIn.toValue = 1
        fadeIn.duration = fade
        fadeIn.timingFunction = CAMediaTimingFunction(name: .easeOut)
        var animations: [CAAnimation] = [fadeIn]
        if !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            // Down the screen is up the layer's y axis unless the view it
            // sits in is flipped.
            let down: CGFloat = view.superview?.isFlipped == true ? 1 : -1
            let moveIn = CABasicAnimation(keyPath: "transform.translation.y")
            moveIn.fromValue = (side == .below ? rise : -rise) * down
            moveIn.toValue = 0
            moveIn.isAdditive = true
            moveIn.duration = move
            moveIn.timingFunction = ease
            animations.append(moveIn)
        }
        let group = CAAnimationGroup()
        group.animations = animations
        group.duration = move
        group.beginTime = layer.convertTime(CACurrentMediaTime(), from: nil) + hold
        group.fillMode = .backwards
        layer.add(group, forKey: "arrival")
    }
}

/// The side the text of a newly opened item comes in from.
enum ArrivalSide {
    case below, above

    /// Moving on to a neighbor: the next one lies below.
    init(_ direction: Direction) {
        self = direction == .next ? .below : .above
    }

    /// By where `new` stands from `old` in the list both are in; from below
    /// when there was nothing open, or the list doesn't hold them both.
    static func from(_ old: String?, to new: String, in items: [ItemView]) -> ArrivalSide {
        guard let old,
              let was = items.firstIndex(where: { $0.id == old }),
              let now = items.firstIndex(where: { $0.id == new })
        else { return .below }
        return now < was ? .above : .below
    }
}

/// The side an item comes in from, decided as it is opened.
struct Arrival: Equatable {
    let item: String
    let side: ArrivalSide

    /// The side for `id`: the one decided for it, else from below.
    static func side(_ arrival: Arrival?, for id: String) -> ArrivalSide {
        arrival.flatMap { $0.item == id ? $0.side : nil } ?? .below
    }
}

/// An item's arrival, played once: by the first text shown for it. Text
/// that comes after another renderer has been on screen is a switch of
/// renderer, which has its own slide.
@MainActor
final class Arriving {
    private var spent = false

    /// True the first time only.
    func claim() -> Bool {
        defer { spent = true }
        return !spent
    }
}

/// The pane leaving stays over the one that replaces it while it fades.
private struct OlderOnTop: ViewModifier {
    @State private var born = Date.timeIntervalSinceReferenceDate

    func body(content: Content) -> some View {
        content.zIndex(-born)
    }
}

extension View {
    /// The reader for one item, in the place the next item's takes: the one
    /// before fades out over it, and the text arrives on its own
    /// (`ReaderMotion.float`).
    func readerSlot(_ id: String) -> some View {
        ZStack {
            modifier(OlderOnTop())
                .id(id)
                .transition(.asymmetric(insertion: .identity, removal: .opacity))
        }
        .animation(ReaderMotion.hurrying ? nil : ReaderMotion.leave, value: id)
        .onChange(of: id) { ReaderMotion.switched() }
    }
}

/// A progress indicator that waits a moment before it shows, so text that
/// is there at once isn't preceded by a flash of it.
struct LateProgress: View {
    @State private var shown = false

    var body: some View {
        ProgressView()
            .opacity(shown ? 1 : 0)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .task {
                try? await Task.sleep(for: .seconds(ReaderMotion.spinnerDelay))
                shown = true
            }
    }
}
