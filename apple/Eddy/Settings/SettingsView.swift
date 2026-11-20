import EddyKit
import SwiftUI
import UniformTypeIdentifiers
import WebKit

/// The settings pages; the command palette opens one by name.
enum SettingsTab: CaseIterable, Identifiable, RawRepresentable {
    case accounts
    case reading
    case saving
    case storage
    case translation
    case summaries
    case video
    case shortcuts
    case about

    init?(rawValue: String) {
        switch rawValue {
        case "Accounts": self = .accounts
        case "Reading": self = .reading
        case "Saving": self = .saving
        case "Storage": self = .storage
        case "Translation", "AI": self = .translation
        case "Summaries": self = .summaries
        case "Video": self = .video
        case "Shortcuts": self = .shortcuts
        case "About": self = .about
        default: return nil
        }
    }

    var rawValue: String {
        switch self {
        case .accounts: "Accounts"
        case .reading: "Reading"
        case .saving: "Saving"
        case .storage: "Storage"
        case .translation: "Translation"
        case .summaries: "Summaries"
        case .video: "Video"
        case .shortcuts: "Shortcuts"
        case .about: "About"
        }
    }

    var id: String { rawValue }

    var icon: String {
        switch self {
        case .accounts: "person.crop.circle"
        case .reading: "textformat"
        case .saving: "arrow.down.to.line"
        case .storage: "internaldrive"
        case .translation: "translate"
        case .summaries: "sparkles"
        case .video: "film.stack"
        case .shortcuts: "keyboard"
        case .about: "info.circle"
        }
    }

    var group: SettingsGroup {
        switch self {
        case .accounts, .reading, .saving, .storage, .shortcuts, .about: .general
        case .translation, .summaries, .video: .intelligence
        }
    }
}

enum SettingsGroup: String, CaseIterable, Identifiable {
    case general = "General", intelligence = "Intelligence"
    var id: String { rawValue }
    var tabs: [SettingsTab] { SettingsTab.allCases.filter { $0.group == self } }
}

/// Settings, as a panel over the window: categories down the left, the
/// chosen page on the right, and what is behind it dimmed. In a narrow
/// window the categories become a rail of icons. Esc or a click outside
/// closes it.
struct SettingsPanel: View {
    @Environment(AppModel.self) private var model
    @AppStorage("settings.tab") private var tab: SettingsTab = .accounts
    @State private var focusMemory = FocusMemory()

    var body: some View {
        GeometryReader { proxy in
            let rail = proxy.size.width < 700
            ZStack {
                Theme.scrim
                    .ignoresSafeArea()
                    .onTapGesture(perform: close)
                HStack(spacing: 0) {
                    sidebar(rail: rail)
                    Hairline(vertical: true)
                    VStack(alignment: .leading, spacing: 0) {
                        Text(LocalizedStringKey(tab.rawValue))
                            .font(Theme.title())
                            .foregroundStyle(Theme.text1)
                            .padding(.horizontal, 28)
                            .padding(.top, 16)
                            .padding(.bottom, 10)
                        detail
                            .settingsPageStyle()
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                    .background(Theme.list)
                }
                .frame(maxWidth: 820, maxHeight: 600)
                .background(FocusRecorder(memory: focusMemory))
                .clipShape(RoundedRectangle(cornerRadius: Theme.Radius.panel, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: Theme.Radius.panel, style: .continuous).strokeBorder(Theme.outline))
                .shadow(color: .black.opacity(0.28), radius: 30, y: 14)
                .padding(.horizontal, 12)
                .padding(.vertical, 24)
            }
            .frame(width: proxy.size.width, height: proxy.size.height)
        }
        .accessibilityAddTraits(.isModal)
        // Whatever had focus behind the panel lets go, so Esc and the other
        // keys come here and not to a list under it.
        .onAppear {
            DispatchQueue.main.async { focusMemory.window?.makeFirstResponder(nil) }
        }
    }

    private func sidebar(rail: Bool) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Button(action: close) { Image(systemName: "xmark") }
                .buttonStyle(FlatButtonStyle())
                .keyboardShortcut(.cancelAction)
                .help("Close Settings (Esc)")
                .accessibilityLabel("Close Settings")
                .frame(height: 48)
                .padding(.horizontal, 12)
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(SettingsGroup.allCases) { group in
                        if rail {
                            if group != SettingsGroup.allCases.first { Hairline().padding(.vertical, 10) }
                        } else {
                            Text(LocalizedStringKey(group.rawValue))
                                .font(Theme.heading)
                                .foregroundStyle(Theme.text1)
                                .padding(.horizontal, 10)
                                .padding(.top, group == SettingsGroup.allCases.first ? 4 : 16)
                                .padding(.bottom, 6)
                        }
                        ForEach(group.tabs) { item in
                            SettingsSidebarRow(tab: item, chosen: item == tab, rail: rail) { tab = item }
                        }
                    }
                }
                .padding(.horizontal, 10)
                .padding(.bottom, 12)
            }
        }
        .frame(width: rail ? 56 : 200)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(Theme.side)
    }

    @ViewBuilder private var detail: some View {
        switch tab {
        case .accounts: AccountsSettings()
        case .translation: TranslationSettingsPage()
        case .summaries: SummariesSettingsPage()
        case .video: VideoSettingsPage()
        case .reading: ReadingSettings()
        case .saving: SavingSettings()
        case .storage: StorageSettings()
        case .shortcuts: ShortcutsSettings()
        case .about: AboutSettings()
        }
    }

    /// Gives focus back to what had it before the panel came up.
    private func close() {
        if let window = focusMemory.window {
            window.makeFirstResponder((focusMemory.responder as? NSView).flatMap { $0.window === window ? $0 : nil })
        }
        model.settingsVisible = false
    }
}

private struct SettingsSidebarRow: View {
    let tab: SettingsTab
    let chosen: Bool
    let rail: Bool
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 9) {
                Image(systemName: tab.icon)
                    .font(.system(size: 14, weight: chosen ? .semibold : .regular))
                    .frame(width: 20)
                if !rail {
                    Text(LocalizedStringKey(tab.rawValue))
                        .font(.system(size: 14, weight: chosen ? .semibold : .regular))
                    Spacer(minLength: 0)
                }
            }
            .foregroundStyle(Theme.text1)
            .padding(.horizontal, rail ? 0 : 10)
            .frame(maxWidth: .infinity, minHeight: 34)
            .background(chosen ? Theme.track : hovering ? Theme.hover : Color.clear, in: RoundedRectangle(cornerRadius: Theme.Radius.row, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(rail ? Text(LocalizedStringKey(tab.rawValue)) : Text(""))
        .accessibilityLabel(Text(LocalizedStringKey(tab.rawValue)))
        .accessibilityAddTraits(chosen ? .isSelected : [])
    }
}

// MARK: - Accounts

private struct AccountsSettings: View {
    @Environment(AppModel.self) private var model
    @State private var freshURL = ""
    @State private var freshUser = ""
    @State private var freshPassword = ""
    @State private var serverURL = "http://127.0.0.1:8787"
    @State private var pairingCode = ""
    @State private var note = ""
    @State private var working = false
    /// What the server says about itself; nil while unknown or unreachable.
    @State private var server: ServerStatusView?
    @State private var confirmingSignOut = false
    /// Changes that could not be sent when signing out was tried; the
    /// question is then whether to lose them.
    @State private var unsentOnSignOut: UInt32?

    private var status: StatusView? { model.status }
    private var serverMode: Bool { status?.mode == .server }

    var body: some View {
        Form {
            Section("Status") {
                LabeledContent("Mode", value: modeText)
                if let account = status?.account {
                    LabeledContent("FreshRSS", value: "\(account.username) · \(account.baseUrl)")
                }
                if serverMode, let status {
                    LabeledContent("Server") {
                        StatusValue(text: status.serverOnline ? String(localized: "Online") : String(localized: "Offline"), good: status.serverOnline)
                    }
                    if let server {
                        LabeledContent("Server version", value: server.version.isEmpty ? "Unknown" : server.version)
                    }
                    if let sync = status.sync {
                        VStack(alignment: .leading, spacing: 4) {
                            if let fraction = sync.fraction {
                                ProgressView(value: fraction)
                            } else {
                                ProgressView().progressViewStyle(.linear)
                            }
                            Text(sync.detail).font(.callout).foregroundStyle(.secondary).monospacedDigit()
                        }
                    }
                    if status.pendingUploads > 0 {
                        LabeledContent("Waiting to upload", value: "\(status.pendingUploads) changes")
                    }
                    if status.rejectedUploads > 0 {
                        LabeledContent("Refused by the server", value: "\(status.rejectedUploads) changes")
                    }
                    if !status.serverOnline {
                        Text(status.offlineRefresh
                            ? "Everything on this Mac keeps working. New articles come straight from FreshRSS, and your changes upload when the server is back."
                            : "Everything on this Mac keeps working, and your changes upload when the server is back. New articles wait for the server.")
                            .foregroundStyle(.secondary).font(.callout)
                    }
                }
                if let error = status?.lastError, !(serverMode && status?.serverOnline == false) {
                    Text(error).foregroundStyle(.red).font(.callout)
                }
            }
            if serverMode {
                serverFreshRSS
            } else {
                Section {
                    SettingsField("Server address", text: $freshURL, prompt: Text("https://rss.example.com"))
                    SettingsField("Username", text: $freshUser)
                    SettingsField("API password", text: $freshPassword, secure: true)
                    HStack {
                        Button("Connect") { connectFresh() }.disabled(working || freshURL.isEmpty || freshUser.isEmpty || freshPassword.isEmpty)
                        if status?.mode == .standalone {
                            Button("Remove Account", role: .destructive) {
                                try? model.engine?.removeAccount()
                                model.reload()
                            }
                        }
                    }
                } header: {
                    Text("FreshRSS")
                } footer: {
                    Text("Use the API password from FreshRSS settings. It is stored in your keychain.").foregroundStyle(.secondary)
                }
            }
            WebSignInSection()
            Section {
                if serverMode {
                    Button("Disconnect from Server", role: .destructive) {
                        working = true
                        Task {
                            defer { working = false }
                            try? await model.engine?.disconnectServer()
                            server = nil
                            model.reload()
                        }
                    }
                    .disabled(working)
                } else {
                    SettingsField("Address", text: $serverURL)
                    SettingsField("Pairing code", text: $pairingCode, prompt: Text("ABCD-EFGH"))
                    Button("Pair and Sync") { connectServer() }.disabled(working || pairingCode.isEmpty)
                }
            } header: {
                Text("Sync Server (optional)")
            } footer: {
                Text(serverMode
                    ? "Disconnecting keeps everything on this Mac. With a FreshRSS account here it carries on directly with FreshRSS."
                    : "Pairing gives your eddy-server your FreshRSS account and uploads everything you read and saved on this Mac. The account also stays here, so new articles still arrive while the server is offline. Create a code with “eddy-server pairing-code”.")
                    .foregroundStyle(.secondary)
            }
            Section {
                Button("Sign Out and Erase Data…", role: .destructive) { confirmingSignOut = true }
                    .disabled(working || model.engine == nil)
            } footer: {
                Text("Removes your accounts, library, downloaded articles, API keys and website sign-ins from this Mac, then restarts Eddy as new. Appearance and layout settings are kept.")
                    .foregroundStyle(.secondary)
            }
            if !note.isEmpty {
                Text(note).foregroundStyle(.secondary)
            }
        }
        .task(id: status?.mode) { await loadServer() }
        .confirmationDialog("Sign out and erase everything Eddy keeps on this Mac?", isPresented: $confirmingSignOut) {
            Button("Sign Out and Erase", role: .destructive) { signOut(discardUnsent: false) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(signOutWarning)
        }
        .confirmationDialog(
            Text("\(unsentOnSignOut ?? 0) changes could not be sent"),
            isPresented: Binding(get: { unsentOnSignOut != nil }, set: { if !$0 { unsentOnSignOut = nil } })
        ) {
            Button("Erase Anyway", role: .destructive) { signOut(discardUnsent: true) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(serverMode
                ? "They exist only on this Mac because the sync server could not be reached. Erasing now loses them. Nothing has been removed yet."
                : "They are read and starred changes FreshRSS has not received. Erasing now loses them. Nothing has been removed yet.")
        }
    }

    /// What is lost and what is kept elsewhere, for the mode this Mac is in.
    private var signOutWarning: String {
        switch status?.mode {
        case .server:
            let waiting = status?.pendingUploads ?? 0
            return "What has synced stays on your server and comes back when you pair again."
                + (waiting > 0 ? " \(waiting) changes are still waiting to upload; Eddy sends them first." : "")
                + " This Mac is removed from the server’s devices."
        case .standalone:
            return "Subscriptions and what you read or starred stay in FreshRSS. Saved pages, books, highlights, tags and reading progress exist only on this Mac and can’t be recovered."
        default:
            return "Saved pages, books, highlights, tags and reading progress exist only on this Mac and can’t be recovered."
        }
    }

    private func signOut(discardUnsent: Bool) {
        guard let engine = model.engine else { return }
        working = true
        note = "Signing out…"
        Task {
            do {
                let outcome = try await engine.signOutAndErase(discardUnsent: discardUnsent)
                guard outcome.erased else {
                    // Nothing was removed; ask whether to lose what is unsent.
                    working = false
                    note = ""
                    model.reload()
                    unsentOnSignOut = outcome.unsent
                    return
                }
                // The library goes when Eddy next opens it; what the app
                // keeps outside the library goes now.
                await AppReset.eraseAppData()
                AppReset.restart()
            } catch {
                working = false
                note = error.localizedDescription
            }
        }
    }

    /// Server mode: the account the server reads, and whether this Mac can
    /// read it too while the server is offline.
    @ViewBuilder private var serverFreshRSS: some View {
        let onServer = server?.account
        Section {
            if let onServer {
                LabeledContent("On the server", value: "\(onServer.username) · \(onServer.baseUrl)")
            } else if server != nil {
                LabeledContent("On the server", value: "Not set")
            }
            if status?.offlineRefresh == true {
                LabeledContent("On this Mac", value: "Used while the server is offline")
                Button("Remove from This Mac", role: .destructive) {
                    try? model.engine?.removeAccount()
                    model.reload()
                }
            } else {
                if onServer == nil {
                    SettingsField("Server address", text: $freshURL, prompt: Text("https://rss.example.com"))
                    SettingsField("Username", text: $freshUser)
                }
                SettingsField("API password", text: $freshPassword, secure: true)
                Button(onServer == nil ? "Connect" : "Use While Offline") { connectFreshInServerMode() }
                    .disabled(working || freshPassword.isEmpty || (onServer == nil && (freshURL.isEmpty || freshUser.isEmpty)))
            }
        } header: {
            Text("FreshRSS")
        } footer: {
            Text(status?.offlineRefresh == true
                ? "The server reads FreshRSS and keeps your devices in step. This Mac keeps the account in its keychain only to fetch new articles while the server is offline."
                : onServer == nil
                    ? "The server has no FreshRSS account yet. Enter one to load your subscriptions."
                    : "Enter the API password to let this Mac fetch new articles from FreshRSS while the server is offline. It is stored in your keychain.")
                .foregroundStyle(.secondary)
        }
    }

    private var modeText: String {
        switch status?.mode {
        case .server: "Syncing through a server"
        case .standalone: "Connected to FreshRSS"
        default: "This Mac only"
        }
    }

    private func loadServer() async {
        guard serverMode, let engine = model.engine else {
            server = nil
            return
        }
        server = try? await engine.serverStatus()
        model.reload()
    }

    private func connectFresh() {
        let (u, n, p) = (freshURL, freshUser, freshPassword)
        freshPassword = ""
        working = true
        note = "Connecting…"
        Task {
            defer { working = false }
            do {
                _ = try await model.engine?.configureFreshrss(url: u, username: n, password: p)
                note = "Connected. Loading subscriptions…"
                await model.refresh()
                note = ""
            } catch {
                note = error.localizedDescription
            }
        }
    }

    /// Gives the server its account if it has none, then keeps the same
    /// account on this Mac for refreshing while the server is offline.
    private func connectFreshInServerMode() {
        let onServer = server?.account
        let (u, n, p) = (onServer?.baseUrl ?? freshURL, onServer?.username ?? freshUser, freshPassword)
        freshPassword = ""
        working = true
        note = "Connecting…"
        Task {
            defer { working = false }
            do {
                if onServer == nil {
                    try await model.engine?.configureServerFreshrss(url: u, username: n, password: p)
                }
                _ = try await model.engine?.configureFreshrss(url: u, username: n, password: p)
                note = ""
                await loadServer()
                await model.refresh()
            } catch {
                note = error.localizedDescription
                await loadServer()
            }
        }
    }

    private func connectServer() {
        let (url, code) = (serverURL, pairingCode)
        pairingCode = ""
        working = true
        note = "Pairing…"
        Task {
            defer { working = false }
            do {
                // Returns once this Mac is connected; the first sync carries
                // on in the background and shows under Status.
                _ = try await model.engine?.connectServer(url: url, pairingCode: code, deviceName: Host.current().localizedName ?? "Mac")
                model.reload()
                note = "Paired. The first sync runs in the background, and you can keep using Eddy."
                await loadServer()
            } catch {
                note = error.localizedDescription
                model.reload()
            }
        }
    }
}

/// The app's side of signing out: what it keeps outside the data
/// directory, and starting again as new.
@MainActor
enum AppReset {
    /// Website sign-ins and cached pages and images. Appearance and layout
    /// preferences stay.
    static func eraseAppData() async {
        // With `EDDY_DATA_DIR` these belong to the real installation too.
        guard !EddyPaths.isOverridden else { return }
        await WKWebsiteDataStore.default().removeData(
            ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(), modifiedSince: .distantPast
        )
        URLCache.shared.removeAllCachedResponses()
    }

    /// Opens a new instance, which erases the library before opening it,
    /// and quits this one.
    static func restart() {
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.createsNewApplicationInstance = true
        if let dir = ProcessInfo.processInfo.environment["EDDY_DATA_DIR"], !dir.isEmpty {
            configuration.environment = ["EDDY_DATA_DIR": dir]
        }
        NSWorkspace.shared.openApplication(at: Bundle.main.bundleURL, configuration: configuration) { _, error in
            DispatchQueue.main.async {
                if error != nil {
                    let alert = NSAlert()
                    alert.messageText = "Eddy has signed out"
                    alert.informativeText = "Open Eddy again to start fresh."
                    alert.runModal()
                }
                NSApp.terminate(nil)
            }
        }
    }
}

// MARK: - About

/// The app and the core are versioned separately: the app can ship an
/// update on its own, and the core's version is what a server is compared
/// with.
private struct AboutSettings: View {
    private var appVersion: String {
        let info = Bundle.main.infoDictionary
        let version = info?["CFBundleShortVersionString"] as? String ?? "?"
        let build = info?["CFBundleVersion"] as? String ?? "?"
        return "\(version) (\(build))"
    }

    var body: some View {
        Form {
            Section {
                LabeledContent("Eddy for Mac", value: appVersion)
                LabeledContent("Core", value: EddyCore.build)
                LabeledContent("Sync protocol", value: "\(coreProtocolVersion())")
            } header: {
                Text("Versions")
            } footer: {
                Text("The core holds the reading, saving and syncing logic and is shared with eddy-server. A server and its devices sync as long as their sync protocols are compatible, whatever their versions.")
                    .foregroundStyle(.secondary)
            }
            Section("Data") {
                LabeledContent("Location") {
                    Text(EddyPaths.dataDirectory.path)
                        .textSelection(.enabled)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .truncationMode(.middle)
                }
            }
        }
    }
}

// MARK: - Shortcuts

/// The keys the app answers to, to look up. They can't be changed yet.
/// Kept by hand: a key added or changed anywhere is added or changed here.
private struct ShortcutsSettings: View {
    private struct Shortcut: Identifiable {
        let action: LocalizedStringKey
        /// Each a way of asking for it: the keys pressed together.
        let keys: [[String]]
        let id = UUID()

        init(_ action: LocalizedStringKey, _ keys: [String]...) {
            self.action = action
            self.keys = keys
        }
    }

    private static let everywhere = [
        Shortcut("Go to or search", ["⌘", "K"]),
        Shortcut("Three-Pane, Library, Gallery", ["⌘", "1"], ["⌘", "2"], ["⌘", "3"]),
        Shortcut("Refresh", ["⌘", "R"]),
        Shortcut("Save a page", ["⌘", "L"]),
        Shortcut("Convert a video to a book", ["⇧", "⌘", "L"]),
        Shortcut("Import an EPUB, PDF or video", ["⌘", "O"]),
        Shortcut("Settings", ["⌘", ","]),
    ]

    private static let threePane = [
        Shortcut("On to the next column", ["→"], ["Return"]),
        Shortcut("Back a column", ["←"], ["Esc"]),
        Shortcut("Up and down a list", ["↑"], ["↓"]),
        Shortcut("Fold or unfold a folder", ["Space"]),
        Shortcut("Next and previous article", ["J"], ["K"]),
        Shortcut("Star", ["S"]),
        Shortcut("Mark as read or unread", ["M"]),
        Shortcut("Delete from the collection", ["⌘", "⌫"]),
    ]

    private static let reading = [
        Shortcut("A page down or up", ["↓"], ["↑"]),
        Shortcut("Next chapter or article", ["⇧", "→"], ["⌘", "→"], ["⌘", "]"]),
        Shortcut("Previous chapter or article", ["⇧", "←"], ["⌘", "←"], ["⌘", "["]),
        Shortcut("On to the next, at the end of the text", ["Hold ↓"]),
        Shortcut("An article’s web page, a book’s next chapter", ["Hold →"]),
        Shortcut("Back to the text from the web page", ["Hold ←"]),
        Shortcut("Show or hide the contents", ["⌘", "B"]),
        Shortcut("Translate", ["⌥", "A"]),
        Shortcut("Mark as read or unread", ["⇧", "⌘", "U"]),
        Shortcut("Mark all as read", ["⇧", "⌘", "K"]),
    ]

    private static let library = [
        Shortcut("Open a book, or read from the contents", ["Return"]),
        Shortcut("Back to the contents, or the shelf", ["Esc"]),
        Shortcut("Along the shelf", ["←"], ["→"], ["↑"], ["↓"]),
    ]

    private static let gallery = [
        Shortcut("On to the pictures", ["→"], ["Return"]),
        Shortcut("Among the pictures", ["←"], ["→"], ["↑"], ["↓"]),
        Shortcut("Open the one framed", ["Space"], ["Return"]),
        Shortcut("Next and previous, in the viewer", ["↓"], ["↑"]),
        Shortcut("Through an item’s pictures, or a video", ["←"], ["→"]),
        Shortcut("Play or pause", ["Space"]),
        Shortcut("Mute", ["M"]),
        Shortcut("Star", ["S"]),
        Shortcut("Close the viewer, back to the sidebar", ["Esc"]),
        Shortcut("Larger and smaller thumbnails", ["⌘", "+"], ["⌘", "−"]),
    ]

    var body: some View {
        Form {
            section("Everywhere", Self.everywhere)
            section("Three-Pane", Self.threePane)
            section("Reading", Self.reading)
            section("Library", Self.library)
            Section {
                rows(Self.gallery)
            } header: {
                Text("Gallery")
            } footer: {
                Text("Shortcuts can’t be changed yet.")
                    .foregroundStyle(Theme.text2)
            }
        }
    }

    private func section(_ title: LocalizedStringKey, _ shortcuts: [Shortcut]) -> some View {
        Section(title) { rows(shortcuts) }
    }

    private func rows(_ shortcuts: [Shortcut]) -> some View {
        ForEach(shortcuts) { shortcut in
            LabeledContent {
                HStack(spacing: 10) {
                    ForEach(Array(shortcut.keys.enumerated()), id: \.offset) { _, combination in
                        HStack(spacing: 3) {
                            ForEach(Array(combination.enumerated()), id: \.offset) { _, key in KeyCap(key) }
                        }
                    }
                }
            } label: {
                Text(shortcut.action)
            }
        }
    }
}

/// One key of a shortcut, as on a keyboard.
private struct KeyCap: View {
    let key: String

    init(_ key: String) { self.key = key }

    var body: some View {
        Text(verbatim: key)
            .font(.system(size: 12, weight: .medium))
            .foregroundStyle(Theme.text1)
            .padding(.horizontal, 6)
            .frame(minWidth: 22, minHeight: 22)
            .background(Theme.track, in: RoundedRectangle(cornerRadius: 5, style: .continuous))
            .accessibilityLabel(Text(verbatim: key))
    }
}

// MARK: - Website sign-ins

/// A site whose pages show more to a signed-in visitor.
private struct SignInSite: Identifiable {
    let name: String
    let loginURL: URL
    let domain: String
    let sessionCookie: String
    var id: String { name }

    static let all = [
        SignInSite(name: "Reddit", loginURL: URL(string: "https://www.reddit.com/login/")!, domain: "reddit.com", sessionCookie: "reddit_session"),
    ]
}

/// Sign-ins live in WebKit's default website data store, which the in-app
/// browser and the background page renderer share.
private struct WebSignInSection: View {
    @State private var signedIn: Set<String> = []
    @State private var signingIn: SignInSite?

    var body: some View {
        Section {
            ForEach(SignInSite.all) { site in
                HStack {
                    Text(site.name).fontWeight(.medium)
                    Spacer()
                    if signedIn.contains(site.id) {
                        Text("Signed in").foregroundStyle(.secondary)
                        Button("Sign Out", role: .destructive) { signOut(site) }
                    } else {
                        Button("Sign In…") { signingIn = site }
                    }
                }
            }
        } header: {
            Text("Website Sign-ins")
        } footer: {
            Text("Eddy fetches pages that need an account with this session, for example Reddit posts marked 18+. The password goes to the site only; Eddy keeps just its cookies. For 18+ posts, turn on “Show mature content” in your Reddit preferences.")
                .foregroundStyle(.secondary)
        }
        .task { await refresh() }
        .sheet(item: $signingIn, onDismiss: { Task { await refresh() } }) { site in
            SignInSheet(site: site)
        }
    }

    private func refresh() async {
        let cookies = await WKWebsiteDataStore.default().httpCookieStore.allCookies()
        signedIn = Set(SignInSite.all.filter { site in
            cookies.contains { $0.name == site.sessionCookie && $0.domain.hasSuffix(site.domain) }
        }.map(\.id))
    }

    private func signOut(_ site: SignInSite) {
        Task {
            let store = WKWebsiteDataStore.default()
            let records = await store.dataRecords(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes())
            await store.removeData(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(), for: records.filter { $0.displayName.hasSuffix(site.domain) })
            await refresh()
        }
    }
}

private struct SignInSheet: View {
    let site: SignInSite
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            SignInWebView(url: site.loginURL)
            Divider()
            HStack {
                Text("Sign in to \(site.name), then click Done.").foregroundStyle(.secondary)
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }
            .padding(12)
        }
        .frame(width: 520, height: 680)
    }
}

private struct SignInWebView: NSViewRepresentable {
    let url: URL

    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        let view = WKWebView(frame: .zero, configuration: config)
        view.load(URLRequest(url: url))
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {}
}

// MARK: - Full-text rules

/// Which subscriptions fetch the full article for new items. Changes save
/// right away and sync.
private struct FullTextRulesSection: View {
    @Environment(AppModel.self) private var model
    @State private var rules: [FullTextRuleView] = []
    @State private var note = ""

    var body: some View {
        Section {
            ForEach(rules.indices, id: \.self) { i in
                FullTextRuleRow(
                    rule: Binding(get: { rules[i] }, set: { rules[i] = $0; save() }),
                    feeds: model.feeds, folders: model.folders,
                    moveUp: i > 0 ? { move(i, by: -1) } : nil,
                    moveDown: i < rules.count - 1 ? { move(i, by: 1) } : nil
                ) {
                    rules.remove(at: i)
                    save()
                }
            }
            .onMove { from, to in
                rules.move(fromOffsets: from, toOffset: to)
                save()
            }
            HStack {
                Button("Add Rule") {
                    rules.append(FullTextRuleView(scope: .all, mode: .always))
                    save()
                }
                Text(note).foregroundStyle(.secondary)
            }
        } header: {
            Text("Full-Text Rules")
        } footer: {
            Text("Whether new articles from a subscription are fetched in full from the original page. Rules higher in the list take priority; Automatic fetches when the feed looks cut short. Choosing a mode from a subscription’s menu adds a rule at the top.")
                .foregroundStyle(.secondary)
        }
        .onAppear { rules = (try? model.engine?.fullTextRules()) ?? [] }
    }

    private func move(_ i: Int, by offset: Int) {
        rules.swapAt(i, i + offset)
        save()
    }

    private func save() {
        do {
            try model.engine?.setFullTextRules(rules: rules)
            note = ""
            model.reload()
        } catch {
            note = error.localizedDescription
        }
    }
}

private struct FullTextRuleRow: View {
    @Binding var rule: FullTextRuleView
    let feeds: [SourceView]
    let folders: [String]
    let moveUp: (() -> Void)?
    let moveDown: (() -> Void)?
    let remove: () -> Void

    var body: some View {
        HStack {
            Picker("", selection: Binding(get: { scopeTag }, set: { rule = FullTextRuleView(scope: scope(from: $0), mode: rule.mode) })) {
                Text("All subscriptions").tag("all")
                if !folders.isEmpty {
                    Divider()
                    ForEach(folders, id: \.self) { Text("Folder: \($0)").tag("category:\($0)") }
                }
                Divider()
                ForEach(feeds) { Text($0.title).tag("source:\($0.id)") }
                if case let .source(id) = rule.scope, !feeds.contains(where: { $0.id == id }) {
                    Text("Removed subscription").tag(scopeTag)
                }
            }
            .labelsHidden()
            .flatPopup()
            Spacer(minLength: 8)
            SwitchBar(
                selection: Binding(get: { rule.mode }, set: { rule = FullTextRuleView(scope: rule.scope, mode: $0) }),
                options: [.text(.auto, String(localized: "Automatic")), .text(.always, String(localized: "Always")), .text(.never, String(localized: "Never"))]
            )
            Button { moveUp?() } label: { Image(systemName: "chevron.up") }
                .buttonStyle(FlatButtonStyle(size: 26)).disabled(moveUp == nil).help("Higher priority")
            Button { moveDown?() } label: { Image(systemName: "chevron.down") }
                .buttonStyle(FlatButtonStyle(size: 26)).disabled(moveDown == nil).help("Lower priority")
            Button(role: .destructive, action: remove) { Image(systemName: "minus") }
                .buttonStyle(FlatButtonStyle(size: 26)).help("Remove")
        }
    }

    private var scopeTag: String {
        switch rule.scope {
        case .all: "all"
        case let .source(id): "source:\(id)"
        case let .category(name): "category:\(name)"
        }
    }

    private func scope(from tag: String) -> FullTextScopeView {
        if tag.hasPrefix("source:") { return .source(id: String(tag.dropFirst(7))) }
        if tag.hasPrefix("category:") { return .category(name: String(tag.dropFirst(9))) }
        return .all
    }
}

extension FullTextModeView {
    static let allCases: [FullTextModeView] = [.auto, .always, .never]

    var title: LocalizedStringKey {
        switch self {
        case .auto: "Automatic"
        case .always: "Always"
        case .never: "Never"
        }
    }
}

// MARK: - Reading

private struct ReadingSettings: View {
    @AppStorage("app.mode") private var mode: AppMode = .threePane
    @AppStorage("reader.fontSize") private var fontSize = 17.0
    @AppStorage(ReaderFonts.latinKey) private var latinFont = ReaderFonts.systemSans
    @AppStorage(ReaderFonts.cjkKey) private var cjkFont = ReaderFonts.systemSans
    @AppStorage(ReaderFonts.spacingKey) private var cjkLatinSpacing = true
    @AppStorage(ReaderFonts.justifyKey) private var justify = false
    @AppStorage("reader.width") private var width = 700.0
    @AppStorage("reader.lineHeight") private var lineHeight = 1.45
    @AppStorage("reader.appearance") private var appearance: ReaderAppearance = .app
    @AppStorage("reader.openLinksInApp") private var openLinksInApp = true

    var body: some View {
        Form {
            Section {
                SettingsChoice("Open in", selection: $mode, options: AppMode.allCases.map { ($0, String(localized: String.LocalizationValue($0.rawValue))) })
            } footer: {
                Text("Three-Pane suits browsing subscriptions; Library suits books, tutorials and long reads. Switch any time with ⌘1 and ⌘2.")
            }
            Section("Text") {
                SettingsSlider("Size", value: $fontSize, in: 13...26, step: 1) { "\(Int($0)) pt" }
                SettingsSlider("Line width", value: $width, in: 520...900, step: 20) { "\(Int($0)) pt" }
                SettingsSlider("Line spacing", value: $lineHeight, in: 1.2...1.9, step: 0.05) { String(format: "%.2f", $0) }
            }
            Section {
                LabeledContent("Latin font") {
                    ReaderFontPicker(title: "Latin font", selection: $latinFont, families: ReaderFonts.latinFamilies).labelsHidden().flatPopup()
                }
                LabeledContent("Chinese font") {
                    ReaderFontPicker(title: "Chinese font", selection: $cjkFont, families: ReaderFonts.cjkFamilies).labelsHidden().flatPopup()
                }
                Toggle("Space between Chinese and Latin text", isOn: $cjkLatinSpacing)
                Toggle("Justify text", isOn: $justify)
            } header: {
                Text("Typeface")
            } footer: {
                Text("Fonts come from the ones installed on this Mac; add more in Font Book. Exported PDFs use the same fonts.").foregroundStyle(.secondary)
            }
            Section {
                SettingsChoice("Content appearance", selection: $appearance, options: ReaderAppearance.allCases.map { ($0, String(localized: String.LocalizationValue($0.rawValue))) })
            } footer: {
                Text("Articles, pages and PDFs can stay light or dark whatever the rest of Eddy uses.").foregroundStyle(.secondary)
            }
            Section {
                Toggle("Open links in Eddy", isOn: $openLinksInApp)
            } header: {
                Text("Links")
            } footer: {
                Text("Links in articles and web pages slide in beside the reader, where you can save them. Swipe right or press Esc to go back; ⌘-click opens a link in your browser.").foregroundStyle(.secondary)
            }
        }
    }
}

// MARK: - Saving

private struct SavingSettings: View {
    @Environment(AppModel.self) private var model
    @State private var packs: RulePackSummaryView?
    @State private var note = ""
    @State private var importing = false

    var body: some View {
        Form {
            Section {
                LabeledContent("Extension ID", value: chromeExtensionId())
                HStack {
                    Button("Connect Chrome") { installHost() }
                    Button("Show Extension in Finder") { revealExtension() }
                }
            } header: {
                Text("Chrome Extension")
            } footer: {
                Text("In chrome://extensions, turn on Developer mode and load the extension folder unpacked. Pages you save from Chrome arrive here even while Eddy is closed.")
                    .foregroundStyle(.secondary)
            }
            FullTextRulesSection()
            Section {
                if let packs, !packs.packs.isEmpty || packs.ftrSites > 0 {
                    ForEach(packs.packs, id: \.name) { pack in
                        HStack {
                            Text(pack.name).fontWeight(.medium)
                            Spacer()
                            Text("\(pack.sites) sites").foregroundStyle(Theme.text2)
                            Button(role: .destructive) { remove(pack.name) } label: { Image(systemName: "trash") }
                                .buttonStyle(FlatButtonStyle(size: 26)).help("Remove")
                        }
                    }
                    if packs.ftrSites > 0 { LabeledContent("Full-Text RSS site files", value: "\(packs.ftrSites)") }
                } else {
                    Text("No rule packs").foregroundStyle(.secondary)
                }
                Button("Import…") { importing = true }
            } header: {
                Text("Private Rule Packs")
            } footer: {
                Text("Site rules for fetching full text stay on this device (and your own server when syncing). See the rule pack format in the eddy-fetch documentation.")
                    .foregroundStyle(.secondary)
            }
            if !note.isEmpty { Text(note).foregroundStyle(.secondary) }
        }
        .onAppear { packs = model.engine?.rulePacks() }
        .fileImporter(isPresented: $importing, allowedContentTypes: [UTType(filenameExtension: "toml") ?? .plainText, .plainText], allowsMultipleSelection: true) { result in
            guard case let .success(urls) = result else { return }
            for url in urls { importPack(url) }
        }
    }

    private func installHost() {
        let helper = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/eddy-capture-host").path
        do {
            let files = try installNativeHost(hostPath: helper)
            note = "Connected \(files.count) browser\(files.count == 1 ? "" : "s")."
        } catch {
            note = error.localizedDescription
        }
    }

    private func revealExtension() {
        if let url = Bundle.main.url(forResource: "chrome", withExtension: nil) {
            NSWorkspace.shared.activateFileViewerSelecting([url])
        }
    }

    private func importPack(_ url: URL) {
        let access = url.startAccessingSecurityScopedResource()
        defer { if access { url.stopAccessingSecurityScopedResource() } }
        guard let text = try? String(contentsOf: url, encoding: .utf8) else { return }
        let name = url.deletingPathExtension().lastPathComponent
        do {
            packs = url.pathExtension == "txt"
                ? try model.engine?.importFtrRule(domain: name, contents: text)
                : try model.engine?.importRulePack(name: name, contents: text)
            note = "Imported \(url.lastPathComponent)"
        } catch {
            note = error.localizedDescription
        }
    }

    private func remove(_ name: String) {
        packs = try? model.engine?.removeRulePack(name: name)
    }
}

// MARK: - Storage

private struct StorageSettings: View {
    @Environment(AppModel.self) private var model
    @State private var usage: UInt64 = 0
    @State private var limit: UInt64 = 2 << 30
    @State private var note = ""

    var body: some View {
        Form {
            Section {
                LabeledContent("Offline cache", value: ByteCountFormatter.string(fromByteCount: Int64(usage), countStyle: .file))
                SettingsChoice("Keep up to", selection: $limit, options: [UInt64(1), 2, 5, 10, 20].map { ($0 << 30, "\($0) GB") })
                    .onChange(of: limit) { _, new in
                        try? model.engine?.setCacheLimit(bytes: new)
                        usage = (try? model.engine?.cacheUsage()) ?? usage
                    }
            } footer: {
                Text("Unread, starred, saved and in-progress articles are always kept.")
            }
            Section("Search") {
                LabeledContent("Search index") {
                    HStack(spacing: 10) {
                        if !note.isEmpty { Text(note) }
                        Button("Rebuild Search Index") {
                            let n = (try? model.engine?.rebuildSearchIndex()) ?? 0
                            note = "Indexed \(n) articles."
                        }
                    }
                }
            }
        }
        .onAppear {
            usage = (try? model.engine?.cacheUsage()) ?? 0
            limit = (try? model.engine?.cacheLimit()) ?? limit
        }
    }
}
