import EddyKit
import PDFKit
import SwiftUI

/// One line of a page's or PDF's own outline.
struct OutlineEntry: Identifiable, Sendable {
    enum Target: Hashable, Sendable {
        case block(String)
        /// A PDF page, and the block where the reflowed text reaches the
        /// same place (nil: where the page starts).
        case page(Int, block: String?)
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
                    entries.append(OutlineEntry(id: entries.count, title: title, level: level, target: .page(document.index(for: page), block: nil)))
                }
                if level < 3 { walk(child, level: level + 1) }
            }
        }
        walk(root, level: 1)
        return entries
    }

    /// A PDF's contents for both views: its bookmarks, each placed in the
    /// reflowed text at its own heading rather than where its page starts;
    /// without bookmarks, the reflowed text's headings with their pages.
    static func pdf(bookmarks: [OutlineEntry], text doc: DocumentView?) -> [OutlineEntry] {
        guard let doc, !doc.blocks.isEmpty else { return bookmarks }
        var index: [String: Int] = [:]
        for (i, block) in doc.blocks.enumerated() where index[block.id] == nil { index[block.id] = i }
        let pageStart = { (page: Int) -> Int? in doc.pages.indices.contains(page) ? index[doc.pages[page]] : nil }
        if bookmarks.isEmpty { return pageHeadings(of: doc, index: index) }

        var used = Set<Int>()
        return bookmarks.map { mark in
            guard case let .page(page, _) = mark.target, let from = pageStart(page) else { return mark }
            let title = squashed(mark.title)
            guard !title.isEmpty else { return mark }
            // The page and the next: a heading set at the foot of a page
            // can start the next one once reflowed.
            let to = max(pageStart(page + 2) ?? doc.blocks.count, from + 1)
            let window = (from..<min(to, doc.blocks.count)).filter { !used.contains($0) }
            let heading = window.first { i in
                guard case .heading = doc.blocks[i].kind else { return false }
                let text = squashed(text(of: doc.blocks[i]))
                return text.hasPrefix(title) || (text.count >= 6 && title.hasPrefix(text))
            }
            // A run-in heading starts its paragraph; a label in the margin
            // can come apart over a few short blocks.
            let starts = { (i: Int) -> Bool in
                let first = squashed(text(of: doc.blocks[i]))
                if first.hasPrefix(title) { return true }
                guard first.count >= 3, title.hasPrefix(first) else { return false }
                let joined = doc.blocks[i..<min(i + 3, doc.blocks.count)].map { squashed(text(of: $0)) }.joined()
                return joined.hasPrefix(title)
            }
            let hit = heading ?? window.first(where: starts)
            guard let hit else { return mark }
            used.insert(hit)
            return OutlineEntry(id: mark.id, title: mark.title, level: mark.level, target: .page(page, block: doc.blocks[hit].id))
        }
    }

    /// Headings of a reflowed PDF, each with the page it is on; levels
    /// count from the topmost one used.
    private static func pageHeadings(of doc: DocumentView, index: [String: Int]) -> [OutlineEntry] {
        let starts = doc.pages.enumerated().compactMap { page, id in index[id].map { (block: $0, page: page) } }
        var found: [(title: String, level: Int, page: Int, block: String)] = []
        var page = 0
        var next = 0
        for (i, block) in doc.blocks.enumerated() {
            while next < starts.count, starts[next].block <= i {
                page = starts[next].page
                next += 1
            }
            guard case let .heading(level, _) = block.kind, level <= 4 else { continue }
            let title = text(of: block).trimmingCharacters(in: .whitespacesAndNewlines)
            if !title.isEmpty { found.append((title, Int(level), page, block.id)) }
        }
        let top = found.map(\.level).min() ?? 1
        return found.enumerated().map { n, h in
            OutlineEntry(id: n, title: h.title, level: h.level - top + 1, target: .page(h.page, block: h.block))
        }
    }

    private static func text(of block: BlockView) -> String {
        switch block.kind {
        case let .heading(_, runs), let .paragraph(runs), let .listItem(_, _, _, _, runs): runs.map(\.text).joined()
        default: ""
        }
    }

    /// Letters and digits only, lowercased: "3.1 Decoupled  Rendering" and
    /// "3.1. Decoupled Rendering" are the same title.
    private static func squashed(_ s: String) -> String {
        String(s.lowercased().unicodeScalars.filter { CharacterSet.alphanumerics.contains($0) }.map(Character.init))
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

    /// Top-level chapters whose sections are shown. Starts with the one
    /// being read; a series without sections has nothing to fold.
    @State private var expanded: Set<String> = []
    @State private var expandedFor: String?

    private func chapters(_ seriesID: String) -> some View {
        let all = model.chapters(seriesID)
        let nested = all.contains { $0.chapterLevel > 0 }
        let filtering = !query.isEmpty
        let rows = Self.rows(of: all)
        let shown = rows.filter { row in
            if filtering { return row.chapter.title.matches(filter: query) }
            guard let parent = row.parent else { return true }
            return expanded.contains(parent)
        }
        return List(selection: Binding(get: { currentItemID }, set: { if let id = $0 { open(id) } })) {
            Section { header }
            Section("Chapters") {
                ForEach(shown, id: \.chapter.id) { row in
                    let chapter = row.chapter
                    HStack(spacing: 8) {
                        if row.parent == nil {
                            Text("\(row.index)").font(.caption.monospacedDigit()).foregroundStyle(.secondary).frame(width: 22, alignment: .trailing)
                        } else {
                            Color.clear.frame(width: 22 + CGFloat(max(Int(chapter.chapterLevel) - 1, 0)) * 12, height: 1)
                        }
                        Text(row.parent == nil ? Self.displayTitle(chapter.title, number: row.index) : chapter.title)
                            .lineLimit(2)
                            .foregroundStyle(chapter.finished ? .secondary : .primary)
                        Spacer()
                        if chapter.finished {
                            Image(systemName: "checkmark.circle.fill").foregroundStyle(.green).accessibilityLabel("Finished")
                        } else if chapter.inProgress {
                            ProgressRing(value: chapter.progress, size: 12)
                        }
                        if row.hasChildren, !filtering {
                            let open = expanded.contains(chapter.id)
                            Image(systemName: "chevron.right")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                                .rotationEffect(.degrees(open ? 90 : 0))
                                .frame(width: 16, height: 16)
                                .contentShape(Rectangle())
                                .onTapGesture { toggle(chapter.id) }
                                .accessibilityLabel(open ? "Hide sections" : "Show sections")
                                .accessibilityAddTraits(.isButton)
                        }
                    }
                    .padding(.vertical, 2)
                    .tag(chapter.id)
                    .contextMenu {
                        if chapter.hasBeenRead {
                            Button("Mark as Unread") { model.markUnread([chapter.id]) }
                        }
                        if nested, !filtering {
                            Divider()
                            Button("Expand All") { expanded = Set(rows.filter(\.hasChildren).map(\.chapter.id)) }
                            Button("Collapse All") { expanded = [] }
                        }
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .pane(focus, .contents, onReturn: onReturn, onEscape: onEscape)
        .onKeyPress(keys: [.leftArrow, .rightArrow]) { press in
            guard !filtering, let current = currentItemID,
                  let row = rows.first(where: { $0.chapter.id == current })
            else { return .ignored }
            let group = row.parent ?? current
            let open = expanded.contains(group)
            switch press.key {
            case .rightArrow where row.hasChildren && !open:
                expanded.insert(group)
            case .leftArrow where open:
                expanded.remove(group)
                if row.parent != nil { self.open(group) }
            case .leftArrow where row.parent != nil:
                self.open(group)
            default:
                return .ignored
            }
            return .handled
        }
        .onChange(of: currentItemID, initial: true) { _, id in
            // Reading into a folded chapter unfolds it.
            guard let id, let row = rows.first(where: { $0.chapter.id == id }) else { return }
            if expandedFor != entry.id {
                expandedFor = entry.id
                expanded = []
            }
            expanded.insert(row.parent ?? id)
        }
    }

    private struct ChapterRow {
        /// Shown number: the chapter's place among the top-level ones when
        /// the series has sections, else among all.
        let index: Int
        let chapter: ItemView
        /// The top-level chapter a section belongs to; nil for one itself.
        let parent: String?
        let hasChildren: Bool
    }

    private static func rows(of all: [ItemView]) -> [ChapterRow] {
        let nested = all.contains { $0.chapterLevel > 0 }
        var rows: [ChapterRow] = []
        var parent: String?
        var number = 0
        for (i, chapter) in all.enumerated() {
            let top = chapter.chapterLevel == 0 || parent == nil
            if top {
                parent = chapter.id
                number += 1
            }
            let hasChildren = top && i + 1 < all.count && all[i + 1].chapterLevel > 0
            rows.append(ChapterRow(index: nested ? number : i + 1, chapter: chapter, parent: top ? nil : parent, hasChildren: hasChildren))
        }
        return rows
    }

    private func toggle(_ id: String) {
        withAnimation(.easeInOut(duration: 0.15)) {
            if expanded.contains(id) { expanded.remove(id) } else { expanded.insert(id) }
        }
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
                        if case let .page(page, _) = line.target {
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
            var bookmarks: [OutlineEntry] = []
            if let url = await model.contentFile(itemID) {
                bookmarks = await Task.detached { OutlineEntry.bookmarks(of: PDFDocument(url: url)) }.value
            }
            outline = OutlineEntry.pdf(bookmarks: bookmarks, text: await model.document(itemID))
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
