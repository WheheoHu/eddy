import EddyKit
import SwiftUI

/// Reading a book from the library: its contents in a collapsible column
/// (chapters or outline, or a subscription's articles), the text beside it.
struct LibraryReader: View {
    @Environment(AppModel.self) private var model
    let book: Book
    @Binding var mode: AppMode
    let close: () -> Void

    @AppStorage("library.contents") private var showContents = true
    @State private var currentID: String?
    @State private var scrollTarget: ScrollTarget?
    @State private var pageTarget: PageTarget?
    /// Back and forward through a subscription's articles; one per book.
    @State private var history = ReadingHistory()
    /// Where the reader slides in from when moving to a neighbor.
    @State private var arrival: Edge = .bottom
    /// A previous chapter, opened at its end.
    @State private var landAtEnd: String?

    /// The entry as it is now, with current progress.
    private var entry: CollectionEntryView? {
        book.entry.map { model.entry($0.id) ?? $0 }
    }

    private var entries: [ItemView] {
        switch book.kind {
        case let .feed(sourceID): model.items(.source(id: sourceID), limit: 300)
        case let .entry(entry): entry.seriesId.map { model.chapters($0) } ?? model.item(entry.id).map { [$0] } ?? []
        }
    }

    var body: some View {
        LeadingColumn(visible: showContents, width: 270) {
            contents
        } content: {
            reader
        }
        .toolbar {
            ColumnToggle(visible: $showContents)
            ToolbarItem(placement: .navigation) {
                Button(action: close) { Label("Home", systemImage: "house") }
                    .help("Back to the library (Esc)")
                    .keyboardShortcut(.cancelAction)
            }
            ModeToolbar(mode: $mode, model: model)
        }
        .navigationTitle(book.title)
        .onAppear(perform: chooseStart)
        .onChange(of: currentID) { _, new in
            history.opened(new)
            if new != landAtEnd { landAtEnd = nil }
        }
    }

    // MARK: Contents column

    @ViewBuilder
    private var contents: some View {
        if let entry {
            EntryContents(entry: entry, currentItemID: currentID, open: select, jump: jump)
        } else {
            FeedContents(title: book.title, items: entries, currentID: currentID, select: select)
        }
    }

    @ViewBuilder
    private var reader: some View {
        if let id = currentID {
            VStack(spacing: 0) {
                ReaderPane(itemID: id, scrollTarget: $scrollTarget, pageTarget: pageTarget, openItem: select, neighbors: neighbors(of: id), startAtEnd: landAtEnd == id)
                    .id(id)
                    .transition(.push(from: arrival))
                if entries.count > 1 {
                    Divider()
                    SequenceBar(title: book.title, items: entries, current: id) { select($0) }
                }
            }
        } else {
            ContentUnavailableView("Nothing to Read", systemImage: "book.closed")
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
        case .entry:
            currentID = entry?.resumeItemId ?? items.first?.id
        case .feed:
            currentID = items.first { $0.inProgress }?.id ?? items.first { !$0.read }?.id ?? items.first?.id
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

    private func jump(_ target: OutlineEntry.Target) {
        switch target {
        case let .block(id): scrollTarget = ScrollTarget(blockID: id)
        case let .page(page): pageTarget = PageTarget(page: page)
        }
    }

    /// A series' chapters or a subscription's articles; nothing to move to
    /// in a page or PDF saved on its own.
    private func neighbors(of id: String) -> Neighbors? {
        switch book.kind {
        case .feed:
            .articles({ entries }, current: id, history: history, item: { model.item($0) }, open: travel)
        case let .entry(entry):
            entry.isSeries ? .chapters({ entries }, current: id, open: travel) : nil
        }
    }

    /// Opens a neighbor, sliding it in from the side it lies on. The edge
    /// is set first so the outgoing reader leaves the same way.
    private func travel(to id: String, _ direction: Direction, _ axis: Axis, _ atEnd: Bool) {
        arrival = .arriving(direction, along: axis)
        landAtEnd = atEnd ? id : nil
        DispatchQueue.main.async {
            withAnimation(.easeInOut(duration: 0.2)) { select(id) }
        }
    }
}

/// A subscription's articles, newest first, under a filter field.
private struct FeedContents: View {
    let title: String
    let items: [ItemView]
    let currentID: String?
    let select: (String) -> Void

    @State private var query = ""

    var body: some View {
        VStack(spacing: 0) {
            FilterField(text: $query, prompt: "Filter Articles")
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
            List(selection: Binding(get: { currentID }, set: { if let id = $0 { select(id) } })) {
                Section(title) {
                    ForEach(items.filter { $0.title.matches(filter: query) }) { item in
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
}
