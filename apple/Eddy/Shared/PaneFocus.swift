import AppKit
import SwiftUI

/// Makes a reader view first responder once it is in a window: on the next
/// turn of the run loop, since SwiftUI adds it after creating it. A text
/// field being typed in keeps its focus.
@MainActor
func claimFocus(_ view: NSView) {
    DispatchQueue.main.async { [weak view] in
        guard let view, let window = view.window, window.firstResponder !== view else { return }
        if let editor = window.firstResponder as? NSTextView, editor.isFieldEditor { return }
        window.makeFirstResponder(view)
    }
}

/// The columns keyboard focus moves between. Return goes to read, Esc steps
/// back a column. The reader is AppKit, so it isn't here: each mode keeps a
/// flag for it and the reader's views claim first responder themselves.
enum PaneFocus: Hashable {
    case sidebar, list, contents
}

extension View {
    /// Binds a column's list to the pane focus. `onReturn` goes on to the
    /// next column, `onEscape` back to the one before.
    @ViewBuilder
    func pane(
        _ focus: FocusState<PaneFocus?>.Binding?,
        _ pane: PaneFocus,
        onReturn: (() -> Void)? = nil,
        onEscape: (() -> Void)? = nil
    ) -> some View {
        if let focus {
            focused(focus, equals: pane)
                .onKeyPress(.return) {
                    guard let onReturn else { return .ignored }
                    onReturn()
                    return .handled
                }
                .onExitCommand(perform: onEscape)
        } else {
            self
        }
    }
}
