import AppKit
import EddyKit
import SwiftUI

/// One thing the palette can do.
struct PaletteItem: Identifiable {
    enum Group: String {
        case reader = "Reader"
        case list = "List"
        case go = "Go To"
        case collection = "Collection"
        case articles = "Articles"
        case settings = "Settings"
        case search = "Search"
        case results = "Article Text"
    }

    let id: String
    let group: Group
    let title: String
    var subtitle: String?
    let icon: String
    /// A search snippet with the matches in bold.
    var snippet: AttributedString?
    /// Runs with the palette still open (a search lists its results there).
    var keepsOpen = false
    /// Goes somewhere that takes keyboard focus itself (a list, a reader).
    var movesFocus = false
    let run: () -> Void

    func matches(_ query: String) -> Bool {
        title.localizedCaseInsensitiveContains(query)
            || subtitle?.localizedCaseInsensitiveContains(query) == true
            || String(localized: String.LocalizationValue(group.rawValue)).localizedCaseInsensitiveContains(query)
    }
}

/// ⌘K: go to a sidebar entry, a collection entry or an article by title,
/// change a setting, or run a command on what is being read. When nothing
/// matches, Return searches the text of every article and lists what it
/// finds here. Esc closes it.
struct CommandPalette: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openSettings) private var openSettings
    @AppStorage("app.mode") private var mode: AppMode = .threePane
    @AppStorage("reader.appearance") private var appearance: ReaderAppearance = .app
    @AppStorage("reader.serif") private var serif = false
    @AppStorage("settings.tab") private var settingsTab: SettingsTab = .accounts
    @AppStorage("threePane.contents") private var threePaneContents = false
    @AppStorage("library.contents") private var libraryContents = true

    @State private var query = ""
    @State private var highlighted = 0
    @State private var titleMatches: [ItemView] = []
    /// Article text found by the last search; listed while set.
    @State private var results: [SearchResultView]?
    @State private var searching = false
    @State private var listHeight: CGFloat = 0
    @State private var keyMonitor: Any?
    @State private var focusMemory = FocusMemory()
    @FocusState private var fieldFocused: Bool

    private static let maxListHeight: CGFloat = 420

    var body: some View {
        let items = visibleItems
        ZStack(alignment: .top) {
            Color.black.opacity(0.12)
                .ignoresSafeArea()
                .onTapGesture { close() }
            VStack(spacing: 0) {
                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                    TextField("Go to, search, or run a command", text: $query)
                        .textFieldStyle(.plain)
                        .font(.title3)
                        .focused($fieldFocused)
                    if searching { ProgressView().controlSize(.small) }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                Divider()
                list(items)
            }
            .frame(width: 580)
            .background(WindowReader(memory: focusMemory))
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(.separator))
            .shadow(color: .black.opacity(0.25), radius: 24, y: 10)
            .padding(.top, 70)
        }
        .onAppear {
            fieldFocused = true
            installKeys()
        }
        .onDisappear(perform: removeKeys)
        .onChange(of: query) {
            highlighted = 0
            results = nil
        }
        .task(id: query) {
            let text = query.trimmingCharacters(in: .whitespaces)
            guard !text.isEmpty else {
                titleMatches = []
                return
            }
            try? await Task.sleep(for: .milliseconds(120))
            guard !Task.isCancelled else { return }
            titleMatches = model.titleMatches(text)
        }
    }

    // MARK: List

    private func list(_ items: [PaletteItem]) -> some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if items.isEmpty {
                        Text(results != nil ? "No article text matches." : "Nothing matches.")
                            .foregroundStyle(.secondary)
                            .padding(14)
                    }
                    ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                        if index == 0 || items[index - 1].group != item.group {
                            Text(LocalizedStringKey(item.group.rawValue))
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                                .padding(.horizontal, 16)
                                .padding(.top, index == 0 ? 4 : 10)
                                .padding(.bottom, 3)
                        }
                        row(item, highlighted: index == highlighted)
                            .id(item.id)
                            .onTapGesture {
                                highlighted = index
                                perform(item)
                            }
                    }
                }
                .padding(.vertical, 6)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { listHeight = $0 }
            }
            .scrollBounceBehavior(.basedOnSize)
            .frame(height: min(listHeight, Self.maxListHeight))
            .onChange(of: highlighted) { _, new in
                if items.indices.contains(new) { proxy.scrollTo(items[new].id) }
            }
        }
    }

    private func row(_ item: PaletteItem, highlighted: Bool) -> some View {
        HStack(spacing: 10) {
            Image(systemName: item.icon)
                .frame(width: 20)
                .foregroundStyle(highlighted ? AnyShapeStyle(.white) : AnyShapeStyle(.secondary))
            VStack(alignment: .leading, spacing: 2) {
                Text(item.title).lineLimit(1)
                if let snippet = item.snippet {
                    Text(snippet).font(.caption).lineLimit(2)
                        .foregroundStyle(highlighted ? AnyShapeStyle(.white.opacity(0.85)) : AnyShapeStyle(.secondary))
                } else if let subtitle = item.subtitle {
                    Text(subtitle).font(.caption).lineLimit(1)
                        .foregroundStyle(highlighted ? AnyShapeStyle(.white.opacity(0.85)) : AnyShapeStyle(.secondary))
                }
            }
            Spacer(minLength: 0)
        }
        .foregroundStyle(highlighted ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(highlighted ? Color.accentColor : .clear, in: RoundedRectangle(cornerRadius: 6))
        .padding(.horizontal, 6)
        .contentShape(Rectangle())
    }

    // MARK: Items

    /// With nothing typed: the reader's commands and where to go. Typing
    /// narrows those and adds collection entries, article titles, settings
    /// and, last, a search of the article text.
    private var visibleItems: [PaletteItem] {
        if let results { return resultItems(results) }
        let text = query.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { return readerItems + listItems + goItems }
        var items = (readerItems + listItems + goItems).filter { $0.matches(text) }
        items += collectionItems(text)
        items += articleItems
        items += settingsItems.filter { $0.matches(text) }
        items.append(PaletteItem(
            id: "search", group: .search, title: String(localized: "Search Article Text for “\(text)”"),
            icon: "text.magnifyingglass", keepsOpen: true
        ) { searchText(text) })
        return items
    }

    private var readerItems: [PaletteItem] {
        guard let id = model.currentItemID, let item = model.item(id) else { return [] }
        let command = { (action: ReaderCommand.Action) in
            model.readerCommand = PaletteRequest(value: ReaderCommand(itemID: id, action: action))
        }
        var items: [PaletteItem] = [
            PaletteItem(id: "summary", group: .reader, title: String(localized: "Summarize"), icon: "sparkles") { command(.summary) },
            PaletteItem(id: "bilingual", group: .reader, title: String(localized: "Translate: Bilingual"), icon: "character.bubble") { command(.readingMode(.bilingual)) },
            PaletteItem(id: "translation", group: .reader, title: String(localized: "Translate: Translation Only"), icon: "character.bubble.fill") { command(.readingMode(.translation)) },
            PaletteItem(id: "original", group: .reader, title: String(localized: "Show Original"), icon: "text.alignleft") { command(.readingMode(.original)) },
            PaletteItem(id: "star", group: .reader, title: item.starred ? String(localized: "Unstar") : String(localized: "Star"), icon: item.starred ? "star.slash" : "star") {
                model.setStarred(item, !item.starred)
            },
            PaletteItem(id: "read", group: .reader, title: item.hasBeenRead ? String(localized: "Mark as Unread") : String(localized: "Mark as Read"), icon: item.hasBeenRead ? "circle.inset.filled" : "circle") {
                if item.hasBeenRead { model.markUnread([item.id]) } else { model.setRead(item, true) }
            },
        ]
        if item.seriesId != nil || item.kind == .feed {
            let chapter = item.seriesId != nil
            items.append(PaletteItem(id: "previous", group: .reader, title: chapter ? String(localized: "Previous Chapter") : String(localized: "Previous Article"), icon: "chevron.up") { command(.step(.previous)) })
            items.append(PaletteItem(id: "next", group: .reader, title: chapter ? String(localized: "Next Chapter") : String(localized: "Next Article"), icon: "chevron.down") { command(.step(.next)) })
        }
        if item.kind == .feed {
            items.append(PaletteItem(id: "fulltext", group: .reader, title: String(localized: "Fetch Full Text"), icon: "doc.text.magnifyingglass") { command(.fetchFullText) })
        }
        if model.entry(containing: item) != nil {
            let shown = mode == .threePane ? threePaneContents : libraryContents
            items.append(PaletteItem(id: "contents", group: .reader, title: shown ? String(localized: "Hide Contents") : String(localized: "Show Contents"), icon: "sidebar.left") {
                withAnimation(.easeInOut(duration: 0.2)) {
                    if mode == .threePane { threePaneContents.toggle() } else { libraryContents.toggle() }
                }
            })
        }
        if let url = item.url.flatMap(URL.init(string:)) {
            items.append(PaletteItem(id: "web", group: .reader, title: String(localized: "Switch Between Reader and Web Page"), icon: "safari") { command(.toggleWeb) })
            items.append(PaletteItem(id: "browser", group: .reader, title: String(localized: "Open in Browser"), icon: "arrow.up.forward.square") { NSWorkspace.shared.open(url) })
            items.append(PaletteItem(id: "copy", group: .reader, title: String(localized: "Copy Link"), icon: "link") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(url.absoluteString, forType: .string)
            })
        }
        return items
    }

    /// Commands on the article list on screen.
    private var listItems: [PaletteItem] {
        guard let list = model.currentList else { return [] }
        return [
            PaletteItem(id: "markAllRead", group: .list, title: String(localized: "Mark All as Read"), subtitle: list.title, icon: "checkmark.circle") {
                model.requestMarkAllRead(list)
            },
        ]
    }

    /// The current mode's sidebar.
    private var goItems: [PaletteItem] {
        goDestinations.map { item in
            var item = item
            item.movesFocus = true
            return item
        }
    }

    private var goDestinations: [PaletteItem] {
        let kinds = CollectionKindView.all.filter { kind in model.collection.contains { $0.kind == kind } }
        let go = { (destination: Destination) in model.navigation = PaletteRequest(value: destination) }
        var items: [PaletteItem] = []
        switch mode {
        case .threePane:
            for smart in ThreePaneView.Smart.allCases {
                items.append(PaletteItem(id: "smart:\(smart.rawValue)", group: .go, title: String(localized: String.LocalizationValue(smart.rawValue)), icon: smart.icon) { go(.threePane(.smart(smart))) })
            }
            items.append(PaletteItem(id: "saved", group: .go, title: String(localized: "Saved"), icon: "bookmark") { go(.threePane(.saved)) })
            for kind in kinds {
                items.append(PaletteItem(id: "kind:\(kind.key)", group: .go, title: kind.pluralName, icon: kind.icon) { go(.threePane(.kind(kind))) })
            }
            for folder in model.folders {
                items.append(PaletteItem(id: "folder:\(folder)", group: .go, title: folder, subtitle: String(localized: "Folder"), icon: "folder") { go(.threePane(.folder(folder))) })
            }
            for feed in model.feeds {
                items.append(PaletteItem(id: "feed:\(feed.id)", group: .go, title: feed.title, subtitle: feed.category ?? String(localized: "Subscription"), icon: "dot.radiowaves.up.forward") { go(.threePane(.feed(feed.id))) })
            }
            for tag in model.tags {
                items.append(PaletteItem(id: "tag:\(tag.id)", group: .go, title: tag.name, subtitle: String(localized: "Tag"), icon: "number") { go(.threePane(.tag(tag.id))) })
            }
        case .library:
            items.append(PaletteItem(id: "all", group: .go, title: String(localized: "All"), icon: "books.vertical") { go(.library(.all)) })
            items.append(PaletteItem(id: "continue", group: .go, title: String(localized: "Continue Reading"), icon: "book") { go(.library(.continueReading)) })
            items.append(PaletteItem(id: "saved", group: .go, title: String(localized: "Saved"), icon: "bookmark") { go(.library(.saved)) })
            for kind in kinds {
                items.append(PaletteItem(id: "kind:\(kind.key)", group: .go, title: kind.pluralName, icon: kind.icon) { go(.library(.kind(kind))) })
            }
            if !model.feeds.isEmpty {
                items.append(PaletteItem(id: "subscriptions", group: .go, title: String(localized: "All Subscriptions"), icon: "dot.radiowaves.up.forward") { go(.library(.subscriptions)) })
                let categories = Array(Set(model.feeds.map { $0.category ?? "Subscriptions" })).sorted()
                if categories.count > 1 {
                    for category in categories {
                        items.append(PaletteItem(id: "category:\(category)", group: .go, title: category, subtitle: String(localized: "Folder"), icon: "folder") { go(.library(.category(category))) })
                    }
                }
                for feed in model.feeds {
                    items.append(PaletteItem(id: "feed:\(feed.id)", group: .go, title: feed.title, subtitle: feed.category ?? String(localized: "Subscription"), icon: "dot.radiowaves.up.forward") { go(.libraryFeed(sourceID: feed.id)) })
                }
            }
        }
        return items
    }

    private func collectionItems(_ text: String) -> [PaletteItem] {
        model.collection.filter { $0.title.localizedCaseInsensitiveContains(text) }.prefix(8).map { entry in
            PaletteItem(
                id: "entry:\(entry.id)", group: .collection, title: entry.title,
                subtitle: [entry.kind.name, entry.byline].compactMap { $0 }.joined(separator: " · "), icon: entry.kind.icon,
                movesFocus: true
            ) { reveal(entry.resumeItemId ?? entry.id) }
        }
    }

    /// Titles that aren't collection entries already listed.
    private var articleItems: [PaletteItem] {
        let entries = Set(model.collection.map(\.id))
        return titleMatches.filter { !entries.contains($0.id) }.map { item in
            PaletteItem(id: "item:\(item.id)", group: .articles, title: item.title, subtitle: item.sourceName, icon: "doc.text", movesFocus: true) { reveal(item.id) }
        }
    }

    private var settingsItems: [PaletteItem] {
        var items = SettingsTab.allCases.map { tab in
            PaletteItem(id: "settings:\(tab.rawValue)", group: .settings, title: String(localized: "Settings: \(String(localized: String.LocalizationValue(tab.rawValue)))"), icon: tab.icon) {
                settingsTab = tab
                openSettings()
            }
        }
        for option in ReaderAppearance.allCases {
            items.append(PaletteItem(
                id: "appearance:\(option.rawValue)", group: .settings,
                title: String(localized: "Content Appearance: \(String(localized: String.LocalizationValue(option.rawValue)))"),
                subtitle: option == appearance ? String(localized: "Current") : nil, icon: "circle.lefthalf.filled"
            ) { appearance = option })
        }
        items.append(PaletteItem(id: "serif", group: .settings, title: serif ? String(localized: "Use Sans-Serif Text") : String(localized: "Use Serif Text"), icon: "textformat") { serif.toggle() })
        let other: AppMode = mode == .threePane ? .library : .threePane
        items.append(PaletteItem(id: "mode", group: .settings, title: String(localized: "Switch to \(String(localized: String.LocalizationValue(other.rawValue)))"), icon: other.icon) { mode = other })
        return items
    }

    private func resultItems(_ results: [SearchResultView]) -> [PaletteItem] {
        results.map { result in
            PaletteItem(
                id: "result:\(result.item.id)", group: .results, title: result.item.title,
                subtitle: result.item.sourceName, icon: "doc.text.magnifyingglass", snippet: Self.snippet(result),
                movesFocus: true
            ) { reveal(result.item.id) }
        }
    }

    /// The snippet with its matches in bold; ranges are UTF-16 offsets.
    private static func snippet(_ result: SearchResultView) -> AttributedString {
        var text = AttributedString(result.snippet)
        for highlight in result.highlights where highlight.end > highlight.start {
            let range = NSRange(location: Int(highlight.start), length: Int(highlight.end - highlight.start))
            if let found = Range(range, in: text) { text[found].inlinePresentationIntent = .stronglyEmphasized }
        }
        return text
    }

    // MARK: Actions

    private func runHighlighted(_ items: [PaletteItem]) {
        guard items.indices.contains(highlighted) else { return }
        perform(items[highlighted])
    }

    private func perform(_ item: PaletteItem) {
        if !item.keepsOpen { close(restoringFocus: !item.movesFocus) }
        item.run()
    }

    private func reveal(_ id: String) {
        model.revealedItem = PaletteRequest(value: id)
    }

    private func searchText(_ text: String) {
        searching = true
        Task {
            let found = await model.search(text)
            searching = false
            guard query.trimmingCharacters(in: .whitespaces) == text else { return }
            results = found
            highlighted = 0
        }
    }

    private func move(_ delta: Int) {
        let count = visibleItems.count
        guard count > 0 else { return }
        highlighted = min(max(highlighted + delta, 0), count - 1)
    }

    /// Focus goes back at once to what had it. When an action goes on to
    /// a list or a reader, the field only lets go, so they can take focus:
    /// putting a list back first would have it clear the reader's focus.
    private func close(restoringFocus: Bool = true) {
        if let window = focusMemory.window {
            let back = restoringFocus ? (focusMemory.responder as? NSView).flatMap { $0.window === window ? $0 : nil } : nil
            window.makeFirstResponder(back)
        }
        model.paletteVisible = false
    }

    /// Up, Down, Return and Esc while typing. An input method composing text
    /// keeps them, so Return can commit a candidate. Return isn't left to the
    /// field's submit: after one Return the field stops editing and selects
    /// its text, and a second Return would not submit.
    private func installKeys() {
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            MainActor.assumeIsolated {
                if let editor = event.window?.firstResponder as? NSTextView, editor.hasMarkedText() { return event }
                switch event.keyCode {
                case 125:
                    move(1)
                    return nil
                case 126:
                    move(-1)
                    return nil
                case 36, 76:
                    runHighlighted(visibleItems)
                    return nil
                case 53:
                    close()
                    return nil
                default:
                    return event
                }
            }
        }
    }

    private func removeKeys() {
        if let keyMonitor { NSEvent.removeMonitor(keyMonitor) }
        keyMonitor = nil
    }
}

/// The palette's window and what had focus in it before the palette's
/// field took it. A class, so recording them isn't a view state change.
final class FocusMemory {
    weak var window: NSWindow?
    weak var responder: NSResponder?
}

/// Records the window as the palette arrives in it, before its field is
/// focused.
private struct WindowReader: NSViewRepresentable {
    let memory: FocusMemory

    func makeNSView(context: Context) -> NSView {
        let view = Probe()
        view.memory = memory
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {}

    final class Probe: NSView {
        var memory: FocusMemory?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window, let memory, memory.window == nil else { return }
            memory.window = window
            if let editor = window.firstResponder as? NSTextView, editor.isFieldEditor { return }
            memory.responder = window.firstResponder
        }
    }
}
