import EddyKit
import SwiftUI
import UniformTypeIdentifiers
import WebKit

/// The settings pages; the command palette opens one by name.
enum SettingsTab: String, CaseIterable, Identifiable {
    case accounts = "Accounts", reading = "Reading", saving = "Saving", storage = "Storage", ai = "AI"
    var id: String { rawValue }

    var icon: String {
        switch self {
        case .accounts: "person.crop.circle"
        case .ai: "sparkles"
        case .reading: "textformat"
        case .saving: "square.and.arrow.down"
        case .storage: "internaldrive"
        }
    }

    /// Background of the icon tile in the sidebar.
    var tint: Color {
        switch self {
        case .accounts: .blue
        case .reading: .orange
        case .saving: .green
        case .storage: .gray
        case .ai: .purple
        }
    }

    var group: SettingsGroup {
        switch self {
        case .accounts, .reading, .saving, .storage: .general
        case .ai: .intelligence
        }
    }
}

enum SettingsGroup: String, CaseIterable, Identifiable {
    case general = "General", intelligence = "Intelligence"
    var id: String { rawValue }
    var tabs: [SettingsTab] { SettingsTab.allCases.filter { $0.group == self } }
}

/// System Settings style: categories down the left, the chosen page on the
/// right. A plain list rather than `NavigationSplitView`, which would add a
/// sidebar toggle there is no use for here.
struct SettingsView: View {
    @AppStorage("settings.tab") private var tab: SettingsTab = .accounts

    var body: some View {
        HStack(spacing: 0) {
            List(selection: $tab) {
                // Room for the window buttons, which sit over the sidebar.
                Color.clear.frame(height: 28).listRowInsets(EdgeInsets()).listRowBackground(Color.clear)
                ForEach(SettingsGroup.allCases) { group in
                    Section(group.rawValue) {
                        ForEach(group.tabs) { tab in
                            SettingsSidebarRow(tab: tab).tag(tab)
                        }
                    }
                }
            }
            .listStyle(.sidebar)
            .scrollContentBackground(.hidden)
            .background(.ultraThinMaterial)
            .frame(width: 200)
            // Only the sidebar runs under the (transparent) title bar; the
            // page keeps its top inset so nothing scrolls out of reach.
            .ignoresSafeArea()
            Divider().ignoresSafeArea()
            detail
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(minWidth: 760, idealWidth: 820, minHeight: 520, idealHeight: 600)
        .background(SettingsWindowStyler())
    }

    @ViewBuilder private var detail: some View {
        switch tab {
        case .accounts: AccountsSettings()
        case .ai: AISettings()
        case .reading: ReadingSettings()
        case .saving: SavingSettings()
        case .storage: StorageSettings()
        }
    }
}

/// Takes the title bar off the settings window: the content runs to the
/// top, the window buttons float over the sidebar, like System Settings.
private struct SettingsWindowStyler: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { Probe() }
    func updateNSView(_ view: NSView, context: Context) {}

    private final class Probe: NSView {
        private var observer: Any?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window else { return }
            // SwiftUI finishes setting the window up after the view is in
            // it, and sets the title again on every page change; style it
            // after that, and again whenever it comes to the front.
            DispatchQueue.main.async { Self.style(window) }
            observer = NotificationCenter.default.addObserver(forName: NSWindow.didBecomeKeyNotification, object: window, queue: .main) { note in
                if let window = note.object as? NSWindow { Self.style(window) }
            }
        }

        deinit {
            if let observer { NotificationCenter.default.removeObserver(observer) }
        }

        static func style(_ window: NSWindow) {
            window.titleVisibility = .hidden
            window.titlebarAppearsTransparent = true
            window.styleMask.insert(.fullSizeContentView)
            window.isMovableByWindowBackground = true
        }
    }
}

private struct SettingsSidebarRow: View {
    let tab: SettingsTab

    var body: some View {
        Label {
            Text(tab.rawValue)
        } icon: {
            Image(systemName: tab.icon)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(.white)
                .frame(width: 24, height: 24)
                .background(tab.tint.gradient, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
        }
        .padding(.vertical, 2)
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

    var body: some View {
        Form {
            Section("Status") {
                LabeledContent("Mode", value: modeText)
                if let account = model.status?.account {
                    LabeledContent("FreshRSS", value: "\(account.username) · \(account.baseUrl)")
                }
                if let error = model.status?.lastError {
                    Text(error).foregroundStyle(.red).font(.callout)
                }
            }
            if model.status?.mode != .server {
                Section {
                    TextField("Server address", text: $freshURL, prompt: Text("https://rss.example.com"))
                    TextField("Username", text: $freshUser)
                    SecureField("API password", text: $freshPassword)
                    HStack {
                        Button("Connect") { connectFresh() }.disabled(working || freshURL.isEmpty || freshUser.isEmpty || freshPassword.isEmpty)
                        if model.status?.mode == .standalone {
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
                if model.status?.mode == .server {
                    Button("Disconnect from Server", role: .destructive) {
                        try? model.engine?.disconnectServer()
                        model.reload()
                    }
                } else {
                    TextField("Address", text: $serverURL)
                    TextField("Pairing code", text: $pairingCode, prompt: Text("ABCD-EFGH"))
                    Button("Pair and Sync") { connectServer() }.disabled(working || pairingCode.isEmpty)
                }
            } header: {
                Text("Sync Server (optional)")
            } footer: {
                Text("Pairing moves your FreshRSS account and everything you read and saved on this Mac to your eddy-server. Create a code with “eddy-server pairing-code”.")
                    .foregroundStyle(.secondary)
            }
            if !note.isEmpty {
                Text(note).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }

    private var modeText: String {
        switch model.status?.mode {
        case .server: "Syncing through a server"
        case .standalone: "Connected to FreshRSS"
        default: "This Mac only"
        }
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

    private func connectServer() {
        let (url, code) = (serverURL, pairingCode)
        pairingCode = ""
        working = true
        note = "Pairing…"
        Task {
            defer { working = false }
            do {
                _ = try await model.engine?.connectServer(url: url, pairingCode: code, deviceName: Host.current().localizedName ?? "Mac")
                note = "Paired. Everything on this Mac now syncs."
                model.reload()
            } catch {
                note = error.localizedDescription
            }
        }
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
                    Text(site.name)
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

// MARK: - AI

/// Every change saves on its own: toggles, steppers and rules at once, text
/// fields when submitted or left. The API key goes to the keychain the same
/// way and the field clears.
private struct AISettings: View {
    @Environment(AppModel.self) private var model
    @State private var baseURL = ""
    @State private var modelID = ""
    @State private var key = ""
    @State private var target = "English"
    @State private var glossary = ""
    @State private var syncTranslations = true
    @State private var syncSummaries = true
    @State private var dailyLimit = 100
    @State private var rules: [AutoRuleView] = []
    @State private var concurrency = 1
    @State private var testing = false
    @State private var testNote = ""
    @State private var note = ""
    @State private var noteFade: Task<Void, Never>?
    @State private var loaded = false
    @FocusState private var focus: Field?

    private enum Field: Hashable { case baseURL, model, key, target, glossary }

    var body: some View {
        Form {
            Section {
                TextField("API base URL", text: $baseURL, prompt: Text("https://api.openai.com/v1"))
                    .focused($focus, equals: .baseURL)
                TextField("Model", text: $modelID, prompt: Text("gpt-4.1-mini"))
                    .focused($focus, equals: .model)
                SecureField(model.engine?.hasAiKey() == true ? "API key (saved in keychain)" : "API key", text: $key)
                    .focused($focus, equals: .key)
                    .onSubmit(saveKey)
                Stepper("Concurrent requests: \(concurrency)", value: $concurrency, in: 1...Int(model.engine?.maxAiConcurrency() ?? 8))
                    .onChange(of: concurrency) { _, _ in save() }
                HStack {
                    Button("Test") { Task { await test() } }
                        .disabled(baseURL.isEmpty || modelID.isEmpty || testing)
                        .help("Send this many translation requests at once to check the model handles them")
                    if testing { ProgressView().controlSize(.small) }
                    Text(testNote).foregroundStyle(.secondary).textSelection(.enabled)
                }
            } header: {
                Text("Provider")
            } footer: {
                Text("Any OpenAI-compatible Chat Completions endpoint. The key stays in this Mac’s keychain and is never synced. Concurrent requests apply to all AI work together; raise it only if the service allows it.").foregroundStyle(.secondary)
            }

            Section {
                TextField("Translate into", text: $target)
                    .focused($focus, equals: .target)
                TextField("Glossary (term = translation, one per line)", text: $glossary, axis: .vertical)
                    .lineLimit(2...4)
                    .focused($focus, equals: .glossary)
                Toggle("Sync translations across devices", isOn: $syncTranslations)
                    .onChange(of: syncTranslations) { _, _ in save() }
                rulesList(for: .translate)
            } header: {
                Text("Translation")
            } footer: {
                Text("Paragraphs already in the target language are left as they are. Rules translate new articles as they arrive; “Foreign only” skips articles already in the target language.").foregroundStyle(.secondary)
            }

            Section {
                Toggle("Sync summaries across devices", isOn: $syncSummaries)
                    .onChange(of: syncSummaries) { _, _ in save() }
                rulesList(for: .summarize)
            } header: {
                Text("Summaries")
            } footer: {
                Text("Rules summarize new articles as they arrive.").foregroundStyle(.secondary)
            }

            Section {
                rulesList(for: .tag)
            } header: {
                Text("Tagging")
            } footer: {
                Text("Rules tag new articles from your own tags; articles you have tagged already are left alone.").foregroundStyle(.secondary)
            }

            Section {
                Stepper("Automatic requests per day: \(dailyLimit)", value: $dailyLimit, in: 0...2000, step: 25)
                    .onChange(of: dailyLimit) { _, _ in save() }
            } header: {
                Text("Automatic Work")
            } footer: {
                Text("Caps what the rules above spend in a day; what you ask for yourself is never held back.").foregroundStyle(.secondary)
            }

            if !note.isEmpty {
                Text(note).foregroundStyle(.secondary).font(.callout)
            }
        }
        .formStyle(.grouped)
        .onSubmit { save() }
        .onChange(of: focus) { old, _ in
            // Leaving a field commits it.
            switch old {
            case .key: saveKey()
            case .some: save()
            case nil: break
            }
        }
        .onAppear(perform: load)
    }

    // MARK: Rules

    @ViewBuilder private func rulesList(for action: AutoActionView) -> some View {
        ForEach(rules.indices.filter { rules[$0].action == action }, id: \.self) { i in
            RuleRow(
                rule: Binding(get: { rules[i] }, set: { rules[i] = $0; save() }),
                feeds: model.feeds, folders: model.folders
            ) {
                rules.remove(at: i)
                save()
            }
        }
        Button(action == .translate ? "Add Translation Rule" : action == .summarize ? "Add Summary Rule" : "Add Tagging Rule") {
            rules.append(AutoRuleView(scope: .all, action: action, minChars: 2000, onlyForeign: false))
            save()
        }
    }

    // MARK: Persistence

    private func load() {
        guard let s = try? model.engine?.aiSettings() else { return }
        baseURL = s.provider?.baseUrl ?? ""
        modelID = s.provider?.model ?? ""
        concurrency = Int(s.provider?.maxConcurrency ?? 1)
        target = s.targetLanguage
        glossary = s.glossary
        syncTranslations = s.syncTranslations
        syncSummaries = s.syncSummaries
        dailyLimit = Int(s.dailyAutoLimit)
        rules = s.rules
        loaded = true
    }

    private func save() {
        guard loaded, let engine = model.engine else { return }
        let settings = AiSettingsView(
            provider: baseURL.isEmpty ? nil : providerView,
            targetLanguage: target,
            glossary: glossary,
            syncTranslations: syncTranslations,
            syncSummaries: syncSummaries,
            dailyAutoLimit: UInt32(dailyLimit),
            rules: rules
        )
        // Nothing changed: no write, no sync, no "Saved".
        if let current = try? engine.aiSettings(), current == settings { return }
        do {
            try engine.setAiSettings(settings: settings)
            flash("Saved")
        } catch {
            flash(error.localizedDescription, fade: false)
        }
    }

    private func saveKey() {
        guard !key.isEmpty, let engine = model.engine else { return }
        do {
            try engine.setAiKey(key: key)
            key = ""
            flash("API key saved")
        } catch {
            flash(error.localizedDescription, fade: false)
        }
    }

    private func flash(_ text: String, fade: Bool = true) {
        noteFade?.cancel()
        note = text
        guard fade else { return }
        noteFade = Task {
            try? await Task.sleep(for: .seconds(2))
            if !Task.isCancelled { note = "" }
        }
    }

    private var providerView: ProviderView {
        ProviderView(baseUrl: baseURL, model: modelID, maxConcurrency: UInt32(concurrency))
    }

    /// Tries the settings as entered, before they are saved.
    private func test() async {
        guard let engine = model.engine else { return }
        testing = true
        testNote = ""
        defer { testing = false }
        do {
            let r = try await engine.testAiProvider(provider: providerView, key: key.isEmpty ? nil : key, targetLanguage: target)
            let seconds = (Double(r.elapsedMs) / 1000).formatted(.number.precision(.fractionLength(1)))
            testNote = String(localized: "\(r.passed)/\(r.total) passed in \(seconds) s")
            if let error = r.error { testNote += " — " + error }
        } catch {
            testNote = error.localizedDescription
        }
    }
}

/// One automatic rule: where it applies, and for translation whether it
/// skips articles already in the target language. The action is the section
/// the row sits in.
private struct RuleRow: View {
    @Binding var rule: AutoRuleView
    let feeds: [SourceView]
    let folders: [String]
    let remove: () -> Void

    var body: some View {
        HStack {
            Picker("", selection: Binding(get: { scopeTag }, set: { rule = AutoRuleView(scope: scope(from: $0), action: rule.action, minChars: rule.minChars, onlyForeign: rule.onlyForeign) })) {
                Text("All articles").tag("all")
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
            if rule.action == .translate {
                Toggle("Foreign only", isOn: Binding(get: { rule.onlyForeign }, set: { rule = AutoRuleView(scope: rule.scope, action: rule.action, minChars: rule.minChars, onlyForeign: $0) }))
            }
            Button(role: .destructive, action: remove) { Image(systemName: "minus.circle") }.buttonStyle(.borderless)
        }
    }

    private var scopeTag: String {
        switch rule.scope {
        case .all: "all"
        case let .source(id): "source:\(id)"
        case let .category(name): "category:\(name)"
        }
    }

    private func scope(from tag: String) -> RuleScopeView {
        if tag.hasPrefix("source:") { return .source(id: String(tag.dropFirst(7))) }
        if tag.hasPrefix("category:") { return .category(name: String(tag.dropFirst(9))) }
        return .all
    }
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
            Picker("", selection: Binding(get: { rule.mode }, set: { rule = FullTextRuleView(scope: rule.scope, mode: $0) })) {
                ForEach(FullTextModeView.allCases, id: \.self) { Text($0.title).tag($0) }
            }
            .labelsHidden()
            .frame(width: 120)
            Button { moveUp?() } label: { Image(systemName: "chevron.up") }
                .buttonStyle(.borderless).disabled(moveUp == nil).help("Higher priority")
            Button { moveDown?() } label: { Image(systemName: "chevron.down") }
                .buttonStyle(.borderless).disabled(moveDown == nil).help("Lower priority")
            Button(role: .destructive, action: remove) { Image(systemName: "minus.circle") }.buttonStyle(.borderless)
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
    @AppStorage("reader.serif") private var serif = false
    @AppStorage("reader.width") private var width = 700.0
    @AppStorage("reader.lineHeight") private var lineHeight = 1.45
    @AppStorage("reader.appearance") private var appearance: ReaderAppearance = .app
    @AppStorage("reader.openLinksInApp") private var openLinksInApp = true

    var body: some View {
        Form {
            Picker("Open in", selection: $mode) {
                ForEach(AppMode.allCases) { Text($0.rawValue).tag($0) }
            }
            Text("Three-Pane suits browsing subscriptions; Library suits books, tutorials and long reads. Switch any time with ⌘1 and ⌘2.")
                .foregroundStyle(.secondary)
            Section("Text") {
                Slider(value: $fontSize, in: 13...26, step: 1) { Text("Size: \(Int(fontSize)) pt") }
                Slider(value: $width, in: 520...900, step: 20) { Text("Line width") }
                Slider(value: $lineHeight, in: 1.2...1.9, step: 0.05) { Text("Line spacing") }
                Toggle("Serif typeface", isOn: $serif)
            }
            Section {
                Picker("Content appearance", selection: $appearance) {
                    ForEach(ReaderAppearance.allCases) { Text($0.rawValue).tag($0) }
                }
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
        .formStyle(.grouped)
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
                            Text(pack.name)
                            Spacer()
                            Text("\(pack.sites) sites").foregroundStyle(.secondary)
                            Button(role: .destructive) { remove(pack.name) } label: { Image(systemName: "trash") }.buttonStyle(.borderless)
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
        .formStyle(.grouped)
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
            LabeledContent("Offline cache", value: ByteCountFormatter.string(fromByteCount: Int64(usage), countStyle: .file))
            Picker("Keep up to", selection: $limit) {
                ForEach([UInt64(1), 2, 5, 10, 20], id: \.self) { gb in Text("\(gb) GB").tag(gb << 30) }
            }
            .onChange(of: limit) { _, new in
                try? model.engine?.setCacheLimit(bytes: new)
                usage = (try? model.engine?.cacheUsage()) ?? usage
            }
            Text("Unread, starred, saved and in-progress articles are always kept.").foregroundStyle(.secondary)
            Button("Rebuild Search Index") {
                let n = (try? model.engine?.rebuildSearchIndex()) ?? 0
                note = "Indexed \(n) articles."
            }
            if !note.isEmpty { Text(note).foregroundStyle(.secondary) }
        }
        .formStyle(.grouped)
        .onAppear {
            usage = (try? model.engine?.cacheUsage()) ?? 0
            limit = (try? model.engine?.cacheLimit()) ?? limit
        }
    }
}
