import PDFKit
import SwiftUI

/// A page to show, with a token so the same page can be requested twice.
struct PageTarget: Equatable {
    var page: Int
    var token = UUID()
}

/// An imported PDF in PDFKit's view, laid out as the file is. Reports the
/// current page for progress; the first target is the resume point.
struct PDFReaderView: NSViewRepresentable {
    let url: URL
    let target: PageTarget?
    let onPage: (_ page: Int, _ count: Int) -> Void
    /// Keyboard focus belongs in the PDF.
    var focused = false
    var onFocus: (() -> Void)?
    var onEscape: (() -> Void)?
    /// The area around the pages; nil inherits the app's appearance.
    var appearance: NSAppearance?

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> ReaderPDFView {
        let view = ReaderPDFView()
        view.autoScales = true
        view.displayMode = .singlePageContinuous
        view.displaysPageBreaks = true
        view.backgroundColor = .underPageBackgroundColor
        context.coordinator.attach(view)
        return view
    }

    func updateNSView(_ view: ReaderPDFView, context: Context) {
        let c = context.coordinator
        c.onPage = onPage
        view.onFocus = onFocus
        view.onEscape = onEscape
        view.wantsFocus = focused
        if view.appearance?.name != appearance?.name { view.appearance = appearance }
        if c.url != url {
            c.url = url
            c.settling = true
            let document = PDFDocument(url: url)
            view.document = document
            let initial = target
            c.lastTarget = target
            // Jump after the first layout; page changes until then are not reading.
            DispatchQueue.main.async {
                if let initial, let page = document?.page(at: initial.page) { view.go(to: page) }
                c.settling = false
            }
        } else if let target, target != c.lastTarget, let page = view.document?.page(at: target.page) {
            c.lastTarget = target
            view.go(to: page)
        }
    }

    @MainActor
    final class Coordinator: NSObject {
        var url: URL?
        var lastTarget: PageTarget?
        var settling = false
        var onPage: ((Int, Int) -> Void)?

        func attach(_ view: PDFView) {
            NotificationCenter.default.addObserver(self, selector: #selector(pageChanged(_:)), name: .PDFViewPageChanged, object: view)
        }

        @objc func pageChanged(_ note: Notification) {
            guard !settling, let view = note.object as? PDFView, let document = view.document, let page = view.currentPage else { return }
            // Layout passes while the view is detached or collapsed move the page too.
            guard view.window != nil, view.visibleRect.height > 100 else { return }
            onPage?(document.index(for: page), document.pageCount)
        }
    }
}

/// Left and Right turn pages, Esc leaves the PDF for the column before it.
final class ReaderPDFView: PDFView {
    var onFocus: (() -> Void)?
    var onEscape: (() -> Void)?
    var wantsFocus = false {
        didSet { if wantsFocus, !oldValue { claimFocus(self) } }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if wantsFocus { claimFocus(self) }
    }

    override func becomeFirstResponder() -> Bool {
        let became = super.becomeFirstResponder()
        if became { onFocus?() }
        return became
    }

    override func keyDown(with event: NSEvent) {
        let modifiers = event.modifierFlags.intersection([.command, .option, .control, .shift])
        if event.keyCode == 53, let onEscape {
            onEscape()
        } else if modifiers.isEmpty, event.specialKey == .leftArrow {
            goToPreviousPage(nil)
        } else if modifiers.isEmpty, event.specialKey == .rightArrow {
            goToNextPage(nil)
        } else {
            super.keyDown(with: event)
        }
    }
}
