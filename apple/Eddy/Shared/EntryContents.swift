import EddyKit
import PDFKit
import SwiftUI

/// One line of a page's or PDF's own outline.
struct OutlineEntry: Identifiable, Sendable {
    enum Target: Hashable, Sendable {
        case block(String)
        case page(Int)
    }

    let id: Int
    let title: String
    let level: Int
    let target: Target

    /// Headings down to level four.
    static func headings(of doc: DocumentView?) -> [OutlineEntry] {
        guard let doc else { return [] }
        var entries: [OutlineEntry] = []
        for block in doc.blocks {
            guard case let .heading(level, runs) = block.kind, level <= 4 else { continue }
            let title = runs.map(\.text).joined().trimmingCharacters(in: .whitespacesAndNewlines)
            if !title.isEmpty {
                entries.append(OutlineEntry(id: entries.count, title: title, level: Int(level), target: .block(block.id)))
            }
        }
        return entries
    }

    /// Bookmarks down to three levels deep.
    static func bookmarks(of document: PDFDocument?) -> [OutlineEntry] {
        guard let document, let root = document.outlineRoot else { return [] }
        var entries: [OutlineEntry] = []
        func walk(_ node: PDFOutline, level: Int) {
            for i in 0..<node.numberOfChildren {
                guard let child = node.child(at: i) else { continue }
                if let title = child.label?.trimmingCharacters(in: .whitespacesAndNewlines), !title.isEmpty,
                   let page = child.destination?.page {
                    entries.append(OutlineEntry(id: entries.count, title: title, level: level, target: .page(document.index(for: page))))
                }
                if level < 3 { walk(child, level: level + 1) }
            }
        }
        walk(root, level: 1)
        return entries
    }
}

/// The contents of a collection entry: a series' chapters, or the outline
/// of a single page or PDF, under a filter field. Used by the three-pane
/// reader's contents panel and the library's contents column.
struct EntryContents: View {
    @Environment(AppModel.self) private var model
    let entry: CollectionEntryView
    let currentItemID: String?
    /// Opens a chapter.
    let open: (String) -> Void
    /// Moves within the open page or PDF.
    let jump: (OutlineEntry.Target) -> Void
    var focus: FocusState<PaneFocus?>.Binding?
    /// Return: read from here.
    var onReturn: (() -> Void)?
    var onEscape: (() -> Void)?

    @State private var query = ""
    @State private var outline: [OutlineEntry] = []
    @State private var outlineSelection: OutlineEntry.ID?

    var body: some View {
        VStack(spacing: 0) {
            FilterField(text: $query, prompt: "Filter Contents")
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
            if let seriesID = entry.seriesId {
                chapters(seriesID)
            } else {
                outlineList
            }
        }
        .task(id: entry.id) { await loadOutline() }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(entry.title).font(.headline).lineLimit(3)
            if let byline = entry.byline { Text(byline).font(.callout).foregroundStyle(.secondary).lineLimit(1) }
            ProgressView(value: entry.progress)
            HStack {
                Text(entry.progressText).font(.caption).foregroundStyle(.secondary)
                Spacer()
                if let next = entry.resumeItemId, next != currentItemID {
                    Button("Continue") { open(next) }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                }
            }
        }
        .padding(.vertical, 4)
    }

    private func chapters(_ seriesID: String) -> some View {
        let chapters = Array(model.chapters(seriesID).enumerated()).filter { $0.element.title.matches(filter: query) }
        return List(selection: Binding(get: { currentItemID }, set: { if let id = $0 { open(id) } })) {
            Section { header }
            Section("Chapters") {
                ForEach(chapters, id: \.element.id) { index, chapter in
                    HStack(spacing: 8) {
                        Text("\(index + 1)").font(.caption.monospacedDigit()).foregroundStyle(.secondary).frame(width: 22, alignment: .trailing)
                        Text(Self.displayTitle(chapter.title, number: index + 1))
                            .lineLimit(2)
                            .foregroundStyle(chapter.finished ? .secondary : .primary)
                        Spacer()
                        if chapter.finished {
                            Image(systemName: "checkmark.circle.fill").foregroundStyle(.green).accessibilityLabel("Finished")
                        } else if chapter.inProgress {
                            ProgressRing(value: chapter.progress, size: 12)
                        }
                    }
                    .padding(.vertical, 2)
                    .tag(chapter.id)
                    .contextMenu {
                        if chapter.hasBeenRead {
                            Button("Mark as Unread") { model.markUnread([chapter.id]) }
                        }
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .pane(focus, .contents, onReturn: onReturn, onEscape: onEscape)
    }

    /// Selecting a heading jumps to it, by click or with the arrow keys.
    private var outlineList: some View {
        List(selection: Binding(get: { outlineSelection }, set: { id in
            outlineSelection = id
            if let line = outline.first(where: { $0.id == id }) { jump(line.target) }
        })) {
            Section { header }
            Section("Contents") {
                if outline.isEmpty {
                    Label(entry.title, systemImage: entry.kind.icon).lineLimit(2)
                }
                ForEach(outline.filter { $0.title.matches(filter: query) }) { line in
                    HStack {
                        Text(line.title).lineLimit(2)
                        Spacer()
                        if case let .page(page) = line.target {
                            Text("\(page + 1)").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                        }
                    }
                    .padding(.leading, CGFloat(max(line.level - 1, 0)) * 12)
                    .tag(line.id)
                }
            }
        }
        .listStyle(.sidebar)
        .pane(focus, .contents, onReturn: onReturn, onEscape: onEscape)
    }

    private func loadOutline() async {
        guard !entry.isSeries, let itemID = entry.resumeItemId else {
            outline = []
            return
        }
        if entry.kind == .pdf {
            guard let url = await model.contentFile(itemID) else {
                outline = []
                return
            }
            outline = await Task.detached { OutlineEntry.bookmarks(of: PDFDocument(url: url)) }.value
        } else {
            outline = OutlineEntry.headings(of: await model.document(itemID))
        }
    }

    /// Chapter titles often carry their own number; the list shows it already.
    static func displayTitle(_ title: String, number: Int) -> String {
        for prefix in ["\(number). ", "\(number) ", "Chapter \(number): ", "Chapter \(number). "] where title.hasPrefix(prefix) {
            return String(title.dropFirst(prefix.count))
        }
        return title
    }
}
