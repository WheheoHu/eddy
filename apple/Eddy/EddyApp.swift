import EddyKit
import SwiftUI
import UniformTypeIdentifiers

/// Brings Eddy to the front when it is launched. Opening the engine can
/// ask for the keychain before launching finishes, and by the time the
/// dialog is answered the activation that came with the launch is spent;
/// without this the window appears behind whatever was in front.
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        Self.comeForward()
        // Once more after the window has had time to appear: activation
        // granted before there is a window doesn't always stick.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
            if !NSApp.isActive { Self.comeForward() }
        }
    }

    static func comeForward() {
        NSApp.activate()
        if !NSApp.isActive { forceForward() }
        NSApp.windows.first { $0.canBecomeKey }?.makeKeyAndOrderFront(nil)
    }

    /// The pre-Sonoma way, which the system still honors for an app the
    /// user just launched. (A deprecated function may call deprecated API
    /// without a warning.)
    @available(macOS, deprecated: 14.0)
    private static func forceForward() {
        NSApp.activate(ignoringOtherApps: true)
    }
}

@main
struct EddyApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate
    @State private var model = AppModel()
    @AppStorage("app.mode") private var mode: AppMode = .threePane
    @AppStorage(GalleryDensity.key) private var galleryRowHeight = GalleryDensity.standard

    init() {
        ReaderFonts.migrate()
    }

    var body: some Scene {
        WindowGroup("Eddy") {
            RootView(mode: mode)
                .environment(model)
                // Three-pane drops columns as the window narrows; the other
                // modes need their width.
                .frame(minWidth: mode == .threePane ? PaneTier.minWindowWidth : 960, minHeight: 620)
        }
        .handlesExternalEvents(matching: ["*"])
        .defaultSize(width: 1360, height: 860)
        // One toolbar height whether or not a screen shows a title.
        .windowToolbarStyle(.unifiedCompact)
        .commands {
            // Settings are a panel in the window, not a window of their own.
            CommandGroup(replacing: .appSettings) {
                Button("Settings…") { model.settingsVisible ? (model.settingsVisible = false) : model.showSettings() }
                    .keyboardShortcut(",", modifiers: .command)
            }
            CommandGroup(after: .newItem) {
                Button("Save Page…") { model.savingURL = true }.keyboardShortcut("l", modifiers: .command)
                Button("Convert Video to Book…") { model.requestVideoConvert() }.keyboardShortcut("L", modifiers: [.command, .shift])
                Button("Import EPUB, PDF, or Video…") { model.importing = true }.keyboardShortcut("o", modifiers: .command)
                Divider()
                Button("Export as PDF…") { model.exportCurrentItem(.pdf) }
                    .disabled(!model.canExportCurrentItem)
                Button("Export as EPUB…") { model.exportCurrentItem(.epub) }
                    .disabled(!model.canExportCurrentItem)
                Divider()
                Button("Refresh") { Task { await model.refresh() } }.keyboardShortcut("r", modifiers: .command)
            }
            CommandGroup(before: .toolbar) {
                ForEach(Array(AppMode.allCases.enumerated()), id: \.element) { index, m in
                    Button(m.rawValue) { mode = m }.keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: .command)
                }
                Divider()
                // The size of the Gallery's thumbnails.
                Button("Zoom In") { GalleryDensity.larger(than: galleryRowHeight).map { galleryRowHeight = $0 } }
                    .keyboardShortcut("+", modifiers: .command)
                    .disabled(mode != .gallery || GalleryDensity.larger(than: galleryRowHeight) == nil)
                Button("Zoom Out") { GalleryDensity.smaller(than: galleryRowHeight).map { galleryRowHeight = $0 } }
                    .keyboardShortcut("-", modifiers: .command)
                    .disabled(mode != .gallery || GalleryDensity.smaller(than: galleryRowHeight) == nil)
                Divider()
            }
            CommandMenu("Go") {
                Button("Go To or Search…") { model.paletteVisible.toggle() }.keyboardShortcut("k", modifiers: .command)
            }
            CommandMenu("Reader") {
                Button("Translate") { model.toggleTranslation() }
                    .keyboardShortcut("a", modifiers: .option)
            }
        }
    }
}

struct RootView: View {
    @Environment(AppModel.self) private var model
    let mode: AppMode
    @State private var keyMonitor: Any?

    static let importTypes: [UTType] = [.epub, .pdf, .mpeg4Movie, .quickTimeMovie, .movie, .audiovisualContent]

    var body: some View {
        @Bindable var model = model
        Group {
            if let error = model.openError {
                ContentUnavailableView("Eddy Couldn’t Open Its Library", systemImage: "exclamationmark.triangle", description: Text(error))
                    .toolbar(removing: .title)
            } else {
                switch mode {
                case .threePane: ThreePaneView()
                case .library: LibraryView()
                case .gallery: GalleryView()
                }
            }
        }
        .background(ModeSwitchInstaller())
        .overlay(alignment: .bottom) { MessageBanner() }
        // Links from the browser go to this window, not a new one.
        .handlesExternalEvents(preferring: ["*"], allowing: ["*"])
        .onOpenURL { link in Task { await model.open(link) } }
        .overlay {
            if let request = model.mediaRequest {
                MediaViewer(entries: [request.entry], current: .constant(request.entry.id), media: model.media) {
                    model.mediaRequest = nil
                }
                .id(request.id)
            }
        }
        .overlay {
            if model.settingsVisible { SettingsPanel().transition(.opacity) }
        }
        .animation(.easeOut(duration: 0.16), value: model.settingsVisible)
        .overlay {
            if model.paletteVisible { CommandPalette() }
        }
        // The window is made after launching finishes; bring it forward too.
        .onAppear {
            AppDelegate.comeForward()
            keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
                // ⌥A translates, wherever focus is: a list would otherwise
                // type-select on the "å" and the web page would keep it.
                let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask).subtracting([.capsLock, .numericPad, .function])
                guard flags == .option, event.charactersIgnoringModifiers?.lowercased() == "a",
                      (NSApp.keyWindow?.firstResponder as? NSTextView)?.isEditable != true else { return event }
                model.toggleTranslation()
                return nil
            }
        }
        .onDisappear {
            if let keyMonitor { NSEvent.removeMonitor(keyMonitor) }
            keyMonitor = nil
        }
        .confirmationDialog(
            Text("Delete “\(model.pendingDeletion?.title ?? "")”?"),
            isPresented: Binding(get: { model.pendingDeletion != nil }, set: { if !$0 { model.pendingDeletion = nil } }),
            presenting: model.pendingDeletion
        ) { entry in
            Button("Delete", role: .destructive) { Task { await model.delete(entry) } }
            Button("Cancel", role: .cancel) {}
        } message: { _ in
            Text("It will be removed from Eddy on all your devices, along with its reading progress. This can’t be undone.")
        }
        .confirmationDialog(
            Text("Mark \(model.pendingMarkAll?.count ?? 0) articles in “\(model.pendingMarkAll?.list.title ?? "")” as read?"),
            isPresented: Binding(get: { model.pendingMarkAll != nil }, set: { if !$0 { model.pendingMarkAll = nil } }),
            presenting: model.pendingMarkAll
        ) { request in
            Button("Mark as Read") { Task { await model.markAllRead(request) } }
            Button("Cancel", role: .cancel) {}
        } message: { _ in
            Text("Articles that arrive from now on stay unread.")
        }
        .fileImporter(isPresented: $model.importing, allowedContentTypes: Self.importTypes, allowsMultipleSelection: true) { result in
            if case let .success(urls) = result { Task { await model.importFiles(urls) } }
        }
        .dropDestination(for: URL.self) { urls, _ in
            let files = urls.filter { url in Self.importTypes.contains { UTType(filenameExtension: url.pathExtension)?.conforms(to: $0) == true } }
            guard !files.isEmpty else { return false }
            Task { await model.importFiles(files) }
            return true
        }
        .sheet(isPresented: $model.savingURL) { SavePageSheet() }
        .sheet(item: $model.pendingVideoConvert) { req in
            VideoConvertSheet(request: req)
        }
        .sheet(item: $model.activeExport) { state in
            ExportProgressSheet(state: state)
        }
        .sheet(item: Binding(get: { model.pendingSeries.map(SeriesProposalBox.init) }, set: { if $0 == nil { model.pendingSeries = nil } })) { box in
            SeriesProposalSheet(proposal: box.proposal)
        }
    }
}

private struct SavePageSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var url = ""

    private var looksLikeVideo: Bool {
        isVideoUrl(url: url.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Save a Page").font(.headline)
            TextField("https://", text: $url).textFieldStyle(.roundedBorder).frame(width: 420).onSubmit(save)
            Text(looksLikeVideo
                 ? "Video link detected: save it as a media bookmark, or convert the presentation into an illustrated multi-chapter book."
                 : "Eddy fetches the article now and keeps it for offline reading. Saving a table of contents offers to add the whole series.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .frame(width: 420, alignment: .leading)
            HStack {
                if looksLikeVideo {
                    Button("Convert to Book…") {
                        let target = url.trimmingCharacters(in: .whitespacesAndNewlines)
                        dismiss()
                        model.requestVideoConvert(source: target)
                    }
                }
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(looksLikeVideo ? "Save Video" : "Save") { save() }.keyboardShortcut(.defaultAction).disabled(url.isEmpty)
            }
        }
        .padding(20)
        .onAppear {
            if let s = NSPasteboard.general.string(forType: .string), s.hasPrefix("http") { url = s }
        }
    }

    private func save() {
        let target = url
        dismiss()
        Task { await model.save(url: target) }
    }
}

private struct SeriesProposalBox: Identifiable {
    let proposal: SeriesProposalView
    var id: String { proposal.tocUrl }
}

private struct SeriesProposalSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let proposal: SeriesProposalView

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Add “\(proposal.title)” as a Web Series?").font(.headline)
            Text("This page looks like a table of contents with \(proposal.chapters.count) chapters. Eddy can track your progress through all of them.")
                .foregroundStyle(.secondary)
            let numbered = numberedChapters
            List(Array(numbered.prefix(80).enumerated()), id: \.offset) { _, row in
                Text(row.number.map { "\($0). \(row.chapter.title)" } ?? row.chapter.title)
                    .lineLimit(1)
                    .foregroundStyle(row.number == nil ? .secondary : .primary)
                    .padding(.leading, CGFloat(row.chapter.level) * 16)
            }
            .frame(width: 460, height: 220)
            HStack {
                Spacer()
                Button("Not Now") { model.pendingSeries = nil; dismiss() }.keyboardShortcut(.cancelAction)
                Button("Add Web Series") { model.createSeries(proposal); dismiss() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
    }

    /// Chapters with their number; sections under a chapter have none.
    private var numberedChapters: [(number: Int?, chapter: ChapterLinkView)] {
        var count = 0
        return proposal.chapters.map { chapter in
            guard chapter.level == 0 else { return (nil, chapter) }
            count += 1
            return (count, chapter)
        }
    }
}
