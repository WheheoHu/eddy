import EddyKit
import SwiftUI

/// Reading a book from the library: its contents on the left (chapters, a
/// subscription's articles, or a saved page's outline), the text on the right.
struct LibraryReader: View {
    @Environment(AppModel.self) private var model
    let book: Book
    let close: () -> Void

    @State private var currentID: String?
    @State private var scrollTarget: ScrollTarget?
    @State private var outline: [(id: String, level: Int, title: String)] = []
    @State private var pdfOutline: [PDFOutlineEntry] = []
    @State private var pageTarget: PageTarget?
    @State private var showContents = true

    private var entries: [ItemView] {
        switch book.kind {
        case let .feed(sourceID): model.items(.source(id: sourceID), limit: 300)
        case let .publication(seriesID): model.chapters(seriesID)
        case let .saved(itemID): model.item(itemID).map { [$0] } ?? []
        }
    }

    var body: some View {
        HStack(spacing: 0) {
            if showContents {
                contents
                    .frame(width: 270)
                    .background(.background.secondary)
                Divider()
            }
            VStack(spacing: 0) {
                if let id = currentID {
                    ReaderPane(itemID: id, scrollTarget: $scrollTarget, onDocument: { doc in
                        outline = Self.outline(of: doc)
                    }, pageTarget: pageTarget, onPDFOutline: { pdfOutline = $0 }, openItem: select)
                    .id(id)
                    if case .saved = book.kind {} else if entries.count > 1 {
                        Divider()
                        SequenceBar(title: book.title, items: entries, current: id) { next in
                            scrollTarget = nil
                            select(next)
                        }
                    }
                } else {
                    ContentUnavailableView("Nothing to Read", systemImage: "book.closed")
                }
            }
        }
        .toolbar {
            ToolbarItem(placement: .navigation) {
                Button(action: close) { Label("Library", systemImage: "chevron.left") }
                    .help("Back to the library (Esc)")
                    .keyboardShortcut(.cancelAction)
            }
            ToolbarItem(placement: .navigation) {
                Button { withAnimation(.easeInOut(duration: 0.2)) { showContents.toggle() } } label: {
                    Label("Contents", systemImage: "sidebar.left")
                }
                .help("Show or hide contents")
            }
        }
        .navigationTitle(book.title)
        .onAppear(perform: chooseStart)
    }

    // MARK: Contents column

    @ViewBuilder
    private var contents: some View {
        switch book.kind {
        case let .saved(itemID) where model.item(itemID)?.format == .pdf:
            List {
                Section("Contents") {
                    if pdfOutline.isEmpty {
                        Text("This PDF has no outline.").foregroundStyle(.secondary)
                    }
                    ForEach(pdfOutline) { entry in
                        Button { pageTarget = PageTarget(page: entry.page) } label: {
                            HStack {
                                Text(entry.title).lineLimit(2)
                                Spacer()
                                Text("\(entry.page + 1)").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                            }
                            .padding(.leading, CGFloat(entry.level - 1) * 12)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            .listStyle(.sidebar)
        case .saved:
            List {
                Section("Contents") {
                    if outline.isEmpty {
                        Text("This page has no headings.").foregroundStyle(.secondary)
                    }
                    ForEach(outline, id: \.id) { entry in
                        Button { scrollTarget = ScrollTarget(blockID: entry.id) } label: {
                            Text(entry.title)
                                .lineLimit(2)
                                .padding(.leading, CGFloat(max(entry.level - 1, 0)) * 12)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            .listStyle(.sidebar)
        case .publication:
            List(selection: Binding(get: { currentID }, set: { if let id = $0 { select(id) } })) {
                Section(book.title) {
                    ForEach(Array(entries.enumerated()), id: \.element.id) { index, chapter in
                        HStack(spacing: 8) {
                            Text("\(index + 1)").font(.caption.monospacedDigit()).foregroundStyle(.secondary).frame(width: 20, alignment: .trailing)
                            Text(ChapterList.displayTitle(chapter.title, number: index + 1)).lineLimit(2)
                                .foregroundStyle(chapter.finished ? .secondary : .primary)
                            Spacer()
                            if chapter.finished {
                                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                            } else if chapter.inProgress {
                                ProgressRing(value: chapter.progress, size: 12)
                            }
                        }
                        .tag(chapter.id)
                    }
                }
            }
            .listStyle(.sidebar)
        case .feed:
            List(selection: Binding(get: { currentID }, set: { if let id = $0 { select(id) } })) {
                Section(book.title) {
                    ForEach(entries) { item in
                        VStack(alignment: .leading, spacing: 3) {
                            HStack(spacing: 6) {
                                Circle().fill(item.read ? Color.clear : Color.accentColor).frame(width: 6, height: 6)
                                Text(item.date, format: .relative(presentation: .named)).font(.caption2).foregroundStyle(.secondary)
                                if item.inProgress { ProgressRing(value: item.progress, size: 10) }
                            }
                            Text(item.title).font(.callout.weight(item.read ? .regular : .semibold)).lineLimit(3)
                        }
                        .padding(.vertical, 2)
                        .tag(item.id)
                    }
                }
            }
            .listStyle(.sidebar)
        }
    }

    // MARK: Navigation

    private func chooseStart() {
        let items = entries
        if let current = model.currentItemID, items.contains(where: { $0.id == current }) {
            currentID = current
            return
        }
        switch book.kind {
        case let .publication(seriesID):
            currentID = model.series.first { $0.id == seriesID }?.nextItemId ?? items.first?.id
        case .feed:
            currentID = items.first { $0.inProgress }?.id ?? items.first { !$0.read }?.id ?? items.first?.id
        case let .saved(itemID):
            currentID = itemID
        }
        model.currentItemID = currentID
    }

    private func select(_ id: String) {
        guard id != currentID else { return }
        scrollTarget = nil
        pageTarget = nil
        currentID = id
        model.currentItemID = id
    }

    static func outline(of doc: DocumentView?) -> [(id: String, level: Int, title: String)] {
        guard let doc else { return [] }
        return doc.blocks.compactMap { block in
            guard case let .heading(level, runs) = block.kind, level <= 4 else { return nil }
            let title = runs.map { $0.math ? $0.text : $0.text }.joined().trimmingCharacters(in: .whitespacesAndNewlines)
            return title.isEmpty ? nil : (block.id, Int(level), title)
        }
    }
}
