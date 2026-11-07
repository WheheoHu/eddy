import PDFKit
import SwiftUI

/// A page to show, with a token so the same page can be requested twice.
struct PageTarget: Equatable {
    var page: Int
    var token = UUID()
}

/// One bookmark of a PDF's outline.
struct PDFOutlineEntry: Identifiable {
    let id: Int
    let title: String
    let level: Int
    let page: Int
}

/// An imported PDF in PDFKit's view, laid out as the file is. Reports the
/// current page for progress; the first target is the resume point.
struct PDFReaderView: NSViewRepresentable {
    let url: URL
    let target: PageTarget?
    let onPage: (_ page: Int, _ count: Int) -> Void
    var onOutline: ([PDFOutlineEntry]) -> Void = { _ in }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> PDFView {
        let view = PDFView()
        view.autoScales = true
        view.displayMode = .singlePageContinuous
        view.displaysPageBreaks = true
        view.backgroundColor = .underPageBackgroundColor
        context.coordinator.attach(view)
        return view
    }

    func updateNSView(_ view: PDFView, context: Context) {
        let c = context.coordinator
        c.onPage = onPage
        if c.url != url {
            c.url = url
            c.settling = true
            let document = PDFDocument(url: url)
            view.document = document
            let outline = Self.outline(of: document)
            let initial = target
            c.lastTarget = target
            // Jump after the first layout; page changes until then are not reading.
            DispatchQueue.main.async {
                if let initial, let page = document?.page(at: initial.page) { view.go(to: page) }
                c.settling = false
                onOutline(outline)
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

    /// Bookmarks down to three levels deep.
    static func outline(of document: PDFDocument?) -> [PDFOutlineEntry] {
        guard let document, let root = document.outlineRoot else { return [] }
        var entries: [PDFOutlineEntry] = []
        func walk(_ node: PDFOutline, level: Int) {
            for i in 0..<node.numberOfChildren {
                guard let child = node.child(at: i) else { continue }
                if let title = child.label?.trimmingCharacters(in: .whitespacesAndNewlines), !title.isEmpty,
                   let page = child.destination?.page {
                    entries.append(PDFOutlineEntry(id: entries.count, title: title, level: level, page: document.index(for: page)))
                }
                if level < 3 { walk(child, level: level + 1) }
            }
        }
        walk(root, level: 1)
        return entries
    }
}
