import EddyKit
import SwiftUI
import UniformTypeIdentifiers

struct SettingsView: View {
    var body: some View {
        TabView {
            AccountsSettings().tabItem { Label("Accounts", systemImage: "person.crop.circle") }
            AISettings().tabItem { Label("AI", systemImage: "sparkles") }
            ReadingSettings().tabItem { Label("Reading", systemImage: "textformat") }
            SavingSettings().tabItem { Label("Saving", systemImage: "square.and.arrow.down") }
            StorageSettings().tabItem { Label("Storage", systemImage: "internaldrive") }
        }
        .frame(width: 560)
        .padding(20)
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

// MARK: - AI

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
    @State private var note = ""

    var body: some View {
        Form {
            Section {
                TextField("API base URL", text: $baseURL, prompt: Text("https://api.openai.com/v1"))
                TextField("Model", text: $modelID, prompt: Text("gpt-4.1-mini"))
                SecureField(model.engine?.hasAiKey() == true ? "API key (saved in keychain)" : "API key", text: $key)
            } header: {
                Text("Provider")
            } footer: {
                Text("Any OpenAI-compatible Chat Completions endpoint. The key stays in this Mac’s keychain and is never synced.").foregroundStyle(.secondary)
            }
            Section("Translation") {
                TextField("Translate into", text: $target)
                TextField("Glossary (term = translation, one per line)", text: $glossary, axis: .vertical).lineLimit(2...4)
            }
            Section("Sync") {
                Toggle("Sync translations across devices", isOn: $syncTranslations)
                Toggle("Sync summaries across devices", isOn: $syncSummaries)
                Stepper("Automatic requests per day: \(dailyLimit)", value: $dailyLimit, in: 0...2000, step: 25)
            }
            Section("Automatic Rules") {
                ForEach(rules.indices, id: \.self) { i in
                    RuleRow(rule: $rules[i], feeds: model.feeds) { rules.remove(at: i) }
                }
                Button("Add Rule") {
                    rules.append(AutoRuleView(scope: .all, action: .summarize, minChars: 2000, onlyForeign: false))
                }
            }
            HStack {
                Button("Save") { save() }.keyboardShortcut(.defaultAction)
                Text(note).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .onAppear(perform: load)
    }

    private func load() {
        guard let s = try? model.engine?.aiSettings() else { return }
        baseURL = s.provider?.baseUrl ?? ""
        modelID = s.provider?.model ?? ""
        target = s.targetLanguage
        glossary = s.glossary
        syncTranslations = s.syncTranslations
        syncSummaries = s.syncSummaries
        dailyLimit = Int(s.dailyAutoLimit)
        rules = s.rules
    }

    private func save() {
        guard let engine = model.engine else { return }
        do {
            try engine.setAiSettings(settings: AiSettingsView(
                provider: baseURL.isEmpty ? nil : ProviderView(baseUrl: baseURL, model: modelID),
                targetLanguage: target,
                glossary: glossary,
                syncTranslations: syncTranslations,
                syncSummaries: syncSummaries,
                dailyAutoLimit: UInt32(dailyLimit),
                rules: rules
            ))
            if !key.isEmpty { try engine.setAiKey(key: key) }
            key = ""
            note = "Saved"
            load()
        } catch {
            note = error.localizedDescription
        }
    }
}

private struct RuleRow: View {
    @Binding var rule: AutoRuleView
    let feeds: [SourceView]
    let remove: () -> Void

    var body: some View {
        HStack {
            Picker("", selection: Binding(get: { actionTag }, set: { rule = AutoRuleView(scope: rule.scope, action: $0, minChars: rule.minChars, onlyForeign: rule.onlyForeign) })) {
                Text("Summarize").tag(AutoActionView.summarize)
                Text("Translate").tag(AutoActionView.translate)
                Text("Tag").tag(AutoActionView.tag)
            }
            .labelsHidden()
            .frame(width: 110)
            Picker("", selection: Binding(get: { scopeTag }, set: { rule = AutoRuleView(scope: scope(from: $0), action: rule.action, minChars: rule.minChars, onlyForeign: rule.onlyForeign) })) {
                Text("All articles").tag("all")
                ForEach(feeds) { Text($0.title).tag("source:\($0.id)") }
            }
            .labelsHidden()
            Toggle("Foreign only", isOn: Binding(get: { rule.onlyForeign }, set: { rule = AutoRuleView(scope: rule.scope, action: rule.action, minChars: rule.minChars, onlyForeign: $0) }))
            Button(role: .destructive, action: remove) { Image(systemName: "minus.circle") }.buttonStyle(.borderless)
        }
    }

    private var actionTag: AutoActionView { rule.action }

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

// MARK: - Reading

private struct ReadingSettings: View {
    @AppStorage("app.mode") private var mode: AppMode = .threePane
    @AppStorage("reader.fontSize") private var fontSize = 17.0
    @AppStorage("reader.serif") private var serif = false
    @AppStorage("reader.width") private var width = 700.0
    @AppStorage("reader.lineHeight") private var lineHeight = 1.45
    @AppStorage("reader.appearance") private var appearance: ReaderAppearance = .app

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
