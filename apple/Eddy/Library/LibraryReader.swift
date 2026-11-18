import EddyKit
import SwiftUI

/// Reading a book from the library: its contents in a collapsible column
/// (chapters or outline, or a subscription's articles), the text beside it.
struct LibraryReader: View {
    @Environment(AppModel.self) private var model
    let book: Book
    @Binding var mode: AppMode
    /// Whether the window has room for the contents beside the text.
    let dock: ColumnDock
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
    @FocusState private var focus: PaneFocus?
    /// Keyboard focus is in the text; opening a book is going to read it.
    @State private var readerFocused = true

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
        LeadingColumn(shown: contentsShown, over: dock.over, width: 270, drag: dock.drag, gesturesEnabled: !model.overlayVisible) {
            VStack(spacing: 0) {
                ColumnStrip(leads: true) { home } trailing: { AddMenu() }
                contents
                // The way to another mode without going back to the shelf.
                StatusFooter()
                ModeBar()
            }
        } content: {
            reader
        }
        .onAppear {
            chooseStart()
            model.currentList = articleList
        }
        .onDisappear { model.currentList = nil }
        .onChange(of: currentID) { _, new in
            history.opened(new)
            if new != landAtEnd { landAtEnd = nil }
        }
        .onChange(of: focus) { _, new in if new != nil { readerFocused = false } }
        // An item of this book picked elsewhere (the command palette).
        .onChange(of: model.currentItemID) { _, new in
            guard let new, new != currentID, entries.contains(where: { $0.id == new }) else { return }
            select(new)
            readerFocused = true
        }
    }

    // MARK: Contents column

    /// The contents show: beside the text as last left, or pulled in over it.
    private var contentsShown: Binding<Bool> { dock.shown($showContents, model: model) }

    /// Picking from contents that lie over the text goes to read: they go
    /// away again.
    private func pick(_ id: String) {
        select(id)
        if dock.over {
            withAnimation(.easeOut(duration: 0.24)) { model.sidebarPulled = false }
        }
    }

    /// A swipe to the right on the text brings the contents in over it.
    private var readerBack: ReaderBack? {
        guard dock.over, !contentsShown.wrappedValue else { return nil }
        return ReaderBack(
            commit: PaneDrag.commit,
            follow: { [drag = dock.drag] distance in
                if let distance { drag.move(to: .sidebar, distance) } else { drag.cancel() }
            },
            go: {
                withAnimation(.easeOut(duration: 0.24)) {
                    model.sidebarPulled = true
                    dock.drag.end()
                }
            }
        )
    }

    /// Return reads from here; Esc goes back to the shelf.
    @ViewBuilder
    private var contents: some View {
        if let entry {
            EntryContents(
                entry: entry, currentItemID: currentID, open: pick, jump: jump,
                focus: $focus, onReturn: { readerFocused = true }, onEscape: close
            )
        } else {
            FeedContents(
                title: book.title, items: entries, currentID: currentID, select: pick,
                focus: $focus, onReturn: { readerFocused = true }, onEscape: close,
                onMarkAllRead: articleList.map { list in { model.requestMarkAllRead(list) } }
            )
        }
    }

    /// A subscription's articles, for "Mark All as Read".
    private var articleList: ArticleList? {
        guard case let .feed(sourceID) = book.kind else { return nil }
        return ArticleList(filter: .source(id: sourceID), title: book.title)
    }

    /// Esc in the text: to the contents column when it shows, else back to
    /// the shelf.
    private func leaveReader() {
        readerFocused = false
        if contentsShown.wrappedValue { focus = .contents } else { close() }
    }

    /// Back to the shelf: first in the strip that leads the window.
    private var home: some View {
        Button(action: close) { Label("Library", systemImage: "chevron.left") }
            .help("Back to the library (Esc)")
    }

    @ViewBuilder
    private var reader: some View {
        if let id = currentID {
            VStack(spacing: 0) {
                ReaderPane(
                    itemID: id, scrollTarget: $scrollTarget, pageTarget: pageTarget, openItem: select,
                    showContents: contentsShown,
                    neighbors: neighbors(of: id), startAtEnd: landAtEnd == id,
                    onEscape: leaveReader, focused: readerFocused, onFocus: { readerFocused = true },
                    onBack: readerBack,
                    stripLeads: !contentsShown.wrappedValue, stripLeading: contentsShown.wrappedValue ? nil : AnyView(home)
                )
                    .id(id)
                    .transition(.push(from: arrival))
                if entries.count > 1 {
                    Hairline()
                    SequenceBar(title: book.title, items: entries, current: id) { select($0) }
                }
            }
            .background(Theme.page)
        } else {
            VStack(spacing: 0) {
                ColumnStrip(leads: !contentsShown.wrappedValue) { if !contentsShown.wrappedValue { home } } trailing: { EmptyView() }
                ContentUnavailableView("Nothing to Read", systemImage: "book.closed").frame(maxHeight: .infinity)
            }
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
        case let .page(page, block): pageTarget = PageTarget(page: page, block: block)
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
    @Environment(AppModel.self) private var model
    let title: String
    let items: [ItemView]
    let currentID: String?
    let select: (String) -> Void
    var focus: FocusState<PaneFocus?>.Binding?
    var onReturn: (() -> Void)?
    var onEscape: (() -> Void)?
    var onMarkAllRead: (() -> Void)?

    @State private var query = ""

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                FilterField(text: $query, prompt: "Filter Articles")
                if let onMarkAllRead {
                    Button(action: onMarkAllRead) { Image(systemName: "checkmark.circle") }
                        .buttonStyle(FlatButtonStyle(size: 28))
                        .help("Mark every unread article here as read")
                        .accessibilityLabel("Mark All as Read")
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            List(selection: Binding(get: { currentID }, set: { if let id = $0 { select(id) } })) {
                Text(verbatim: title)
                    .font(Theme.title(20))
                    .foregroundStyle(Theme.text1)
                    .lineLimit(2)
                    .padding(.vertical, 6)
                    .flatRow(chosen: false)
                    .selectionDisabled()
                Group {
                    ForEach(items.filter { $0.title.matches(filter: query) }) { item in
                        VStack(alignment: .leading, spacing: 3) {
                            HStack(spacing: 6) {
                                RoundedRectangle(cornerRadius: 2).fill(item.read ? Color.clear : Theme.accent).frame(width: 6, height: 6)
                                Text(item.date, format: .relative(presentation: .named)).font(.system(size: 11)).foregroundStyle(Theme.text3)
                                if item.inProgress { ProgressRing(value: item.progress, size: 10) }
                            }
                            Text(item.title).font(.system(size: 13.5, weight: item.read ? .regular : .semibold)).foregroundStyle(item.read ? Theme.text2 : Theme.text1).lineLimit(3)
                        }
                        .padding(.vertical, 6)
                        .flatRow(chosen: item.id == currentID)
                        .tag(item.id)
                        .contextMenu {
                            if item.hasBeenRead {
                                Button("Mark as Unread") { model.markUnread([item.id]) }
                            } else {
                                Button("Mark as Read") { model.setRead(item, true) }
                            }
                            if model.canExport(item.id, fromReader: false) {
                                Menu("Export") {
                                    Button("Export as PDF…") { model.beginExport(targetID: item.id, format: .pdf, fromReader: false) }
                                    Button("Export as EPUB…") { model.beginExport(targetID: item.id, format: .epub, fromReader: false) }
                                }
                            }
                        }
                    }
                }
            }
            .flatList()
            .pane(focus, .contents, onReturn: onReturn, onEscape: onEscape)
        }
    }
}
