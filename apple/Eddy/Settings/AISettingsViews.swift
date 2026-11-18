import AppKit
import EddyKit
import SwiftUI

// MARK: - Translation Settings

struct TranslationSettingsPage: View {
    @Environment(AppModel.self) private var model
    @State private var proto: AiProtocolView = .auto
    @State private var baseURL = ""
    @State private var modelID = ""
    @State private var key = ""
    @State private var concurrency = 1
    @State private var target = "English"
    @State private var glossary = ""
    @State private var syncTranslations = true
    @State private var dailyLimit = 100
    @State private var rules: [AutoRuleView] = []
    @State private var hasSavedKey = false

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
                aiProtocolPicker($proto) { save() }
                SettingsField("API URL", text: $baseURL, prompt: Text(proto == .gemini ? "https://generativelanguage.googleapis.com/v1beta" : "https://api.openai.com/v1"), width: 320)
                    .focused($focus, equals: .baseURL)
                SettingsField("Model", text: $modelID, prompt: Text(proto == .gemini ? "gemini-2.5-flash" : "gpt-4.1-mini"))
                    .focused($focus, equals: .model)
                HStack {
                    SettingsField("API key", text: $key, prompt: hasSavedKey ? Text("Saved in keychain") : nil, secure: true)
                        .focused($focus, equals: .key)
                        .onSubmit(saveKey)
                    if hasSavedKey {
                        Button("Remove Key", role: .destructive, action: removeKey)
                    }
                }
                SettingsStepper("Concurrent requests", value: $concurrency, in: 1...Int(model.engine?.maxAiConcurrency() ?? 8))
                    .onChange(of: concurrency) { _, _ in save() }
                HStack {
                    Button("Test") { Task { await test() } }
                        .disabled(baseURL.trimmingCharacters(in: .whitespaces).isEmpty || modelID.trimmingCharacters(in: .whitespaces).isEmpty || testing)
                        .help("Send this many translation requests at once to verify the endpoint and model")
                    if testing { ProgressView().controlSize(.small) }
                    Text(testNote).foregroundStyle(.secondary).textSelection(.enabled)
                }
            } header: {
                Text("Provider")
            } footer: {
                Text("Supports OpenAI-compatible Chat Completions (/v1) and Gemini native (/v1beta) endpoints. The API key stays in this Mac’s keychain and is never synced.")
                    .foregroundStyle(.secondary)
            }

            Section {
                SettingsField("Translate into", text: $target)
                    .focused($focus, equals: .target)
                SettingsField("Glossary", text: $glossary, prompt: Text("term = translation, one per line"), lines: 2...4, width: 320)
                    .focused($focus, equals: .glossary)
                Toggle("Sync translations across devices", isOn: $syncTranslations)
                    .onChange(of: syncTranslations) { _, _ in save() }
                ForEach(rules.indices, id: \.self) { i in
                    AIRuleRow(
                        rule: Binding(get: { rules[i] }, set: { rules[i] = $0; save() }),
                        feeds: model.feeds,
                        folders: model.folders
                    ) {
                        rules.remove(at: i)
                        save()
                    }
                }
                Button("Add Translation Rule") {
                    rules.append(AutoRuleView(scope: .all, action: .translate, minChars: 2000, onlyForeign: false))
                    save()
                }
            } header: {
                Text("Translation")
            } footer: {
                Text("Paragraphs already in the target language are left as they are. Rules translate new articles as they arrive; “Foreign only” skips articles already in the target language.")
                    .foregroundStyle(.secondary)
            }

            Section {
                SettingsStepper("Automatic requests per day", value: $dailyLimit, in: 0...2000, step: 25)
                    .onChange(of: dailyLimit) { _, _ in save() }
            } header: {
                Text("Automatic Work")
            } footer: {
                Text("Caps what automatic translation rules spend in a day; translations you ask for yourself are never held back.")
                    .foregroundStyle(.secondary)
            }

            if !note.isEmpty {
                Text(note).foregroundStyle(.secondary).font(.callout)
            }
        }
        .onSubmit {
            commitPendingKey()
            save()
        }
        .onChange(of: focus) { old, _ in
            switch old {
            case .key: saveKey()
            case .some: save()
            case nil: break
            }
        }
        .onAppear(perform: load)
        .onDisappear {
            commitPendingKey()
            save()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)) { _ in
            commitPendingKey()
            save()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.willResignActiveNotification)) { _ in
            commitPendingKey()
            save()
        }
    }

    private var providerView: ProviderView {
        ProviderView(
            protocol: proto,
            baseUrl: baseURL.trimmingCharacters(in: .whitespacesAndNewlines),
            model: modelID.trimmingCharacters(in: .whitespacesAndNewlines),
            maxConcurrency: UInt32(concurrency)
        )
    }

    private func load() {
        guard let engine = model.engine, let s = try? engine.aiSettings() else { return }
        proto = s.translationProvider?.protocol ?? .auto
        baseURL = s.translationProvider?.baseUrl ?? ""
        modelID = s.translationProvider?.model ?? ""
        concurrency = Int(s.translationProvider?.maxConcurrency ?? 1)
        target = s.targetLanguage
        glossary = s.glossary
        syncTranslations = s.syncTranslations
        dailyLimit = Int(s.translationDailyLimit)
        rules = s.rules.filter { $0.action == .translate }
        hasSavedKey = engine.hasAiRoleKey(role: .translation)
        loaded = true
    }

    private func save() {
        guard loaded, let engine = model.engine, let current = try? engine.aiSettings() else { return }
        let trimmedURL = baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedModel = modelID.trimmingCharacters(in: .whitespacesAndNewlines)
        let newProvider: ProviderView? = if trimmedURL.isEmpty || trimmedModel.isEmpty {
            nil
        } else {
            providerView
        }
        let otherRules = current.rules.filter { $0.action != .translate }
        let updated = AiSettingsView(
            translationProvider: newProvider,
            targetLanguage: target,
            glossary: glossary,
            syncTranslations: syncTranslations,
            translationDailyLimit: UInt32(dailyLimit),
            summaryProvider: current.summaryProvider,
            summaryLanguage: current.summaryLanguage,
            syncSummaries: current.syncSummaries,
            summaryDailyLimit: current.summaryDailyLimit,
            rules: rules + otherRules
        )
        if current == updated { return }
        do {
            try engine.setAiSettings(settings: updated)
            if let normalized = try? engine.aiSettings().translationProvider {
                baseURL = normalized.baseUrl
                modelID = normalized.model
            }
            flash("Saved")
        } catch {
            flash(error.localizedDescription, fade: false)
        }
    }

    private func commitPendingKey() {
        if !key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            saveKey()
        }
    }

    private func saveKey() {
        let trimmed = key.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let engine = model.engine else { return }
        do {
            try engine.setAiRoleKey(role: .translation, key: trimmed)
            key = ""
            hasSavedKey = engine.hasAiRoleKey(role: .translation)
            flash("API key saved")
        } catch {
            flash(error.localizedDescription, fade: false)
        }
    }

    private func removeKey() {
        guard let engine = model.engine else { return }
        do {
            try engine.setAiRoleKey(role: .translation, key: nil)
            key = ""
            hasSavedKey = false
            flash("API key removed")
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

    private func test() async {
        guard let engine = model.engine else { return }
        commitPendingKey()
        save()
        testing = true
        testNote = ""
        defer { testing = false }
        let pendingKey = key.trimmingCharacters(in: .whitespacesAndNewlines)
        do {
            let r = try await engine.testTranslationProvider(
                provider: providerView,
                key: pendingKey.isEmpty ? nil : pendingKey,
                targetLanguage: target
            )
            let seconds = (Double(r.elapsedMs) / 1000).formatted(.number.precision(.fractionLength(1)))
            testNote = String(localized: "\(r.passed)/\(r.total) passed in \(seconds) s")
            if let error = r.error { testNote += " — " + error }
        } catch {
            testNote = error.localizedDescription
        }
    }
}

// MARK: - Summaries & Tagging Settings

struct SummariesSettingsPage: View {
    @Environment(AppModel.self) private var model
    @State private var proto: AiProtocolView = .auto
    @State private var baseURL = ""
    @State private var modelID = ""
    @State private var key = ""
    @State private var concurrency = 1
    @State private var summaryLanguage = "English"
    @State private var syncSummaries = true
    @State private var dailyLimit = 100
    @State private var rules: [AutoRuleView] = []
    @State private var hasSavedKey = false

    @State private var testing = false
    @State private var testNote = ""
    @State private var note = ""
    @State private var noteFade: Task<Void, Never>?
    @State private var loaded = false
    @FocusState private var focus: Field?

    private enum Field: Hashable { case baseURL, model, key, summaryLanguage }

    var body: some View {
        Form {
            Section {
                aiProtocolPicker($proto) { save() }
                SettingsField("API URL", text: $baseURL, prompt: Text(proto == .gemini ? "https://generativelanguage.googleapis.com/v1beta" : "https://api.openai.com/v1"), width: 320)
                    .focused($focus, equals: .baseURL)
                SettingsField("Model", text: $modelID, prompt: Text(proto == .gemini ? "gemini-2.5-pro" : "gpt-4.1"))
                    .focused($focus, equals: .model)
                HStack {
                    SettingsField("API key", text: $key, prompt: hasSavedKey ? Text("Saved in keychain") : nil, secure: true)
                        .focused($focus, equals: .key)
                        .onSubmit(saveKey)
                    if hasSavedKey {
                        Button("Remove Key", role: .destructive, action: removeKey)
                    }
                }
                SettingsStepper("Concurrent requests", value: $concurrency, in: 1...Int(model.engine?.maxAiConcurrency() ?? 8))
                    .onChange(of: concurrency) { _, _ in save() }
                HStack {
                    Button("Test") { Task { await test() } }
                        .disabled(baseURL.trimmingCharacters(in: .whitespaces).isEmpty || modelID.trimmingCharacters(in: .whitespaces).isEmpty || testing)
                        .help("Send this many summary requests at once to verify the endpoint and model")
                    if testing { ProgressView().controlSize(.small) }
                    Text(testNote).foregroundStyle(.secondary).textSelection(.enabled)
                }
            } header: {
                Text("Provider")
            } footer: {
                Text("Used for article summaries and automatic tagging, and inherited by Video stages left blank. The API key stays in this Mac’s keychain and is never synced.")
                    .foregroundStyle(.secondary)
            }

            Section {
                SettingsField("Summary language", text: $summaryLanguage)
                    .focused($focus, equals: .summaryLanguage)
                Toggle("Sync summaries across devices", isOn: $syncSummaries)
                    .onChange(of: syncSummaries) { _, _ in save() }
                rulesList(for: .summarize)
            } header: {
                Text("Summaries")
            } footer: {
                Text("Rules summarize new articles as they arrive.")
                    .foregroundStyle(.secondary)
            }

            Section {
                rulesList(for: .tag)
            } header: {
                Text("Tagging")
            } footer: {
                Text("Rules tag new articles from your own tags using the Summaries provider above; articles you have tagged already are left alone.")
                    .foregroundStyle(.secondary)
            }

            Section {
                SettingsStepper("Automatic requests per day", value: $dailyLimit, in: 0...2000, step: 25)
                    .onChange(of: dailyLimit) { _, _ in save() }
            } header: {
                Text("Automatic Work")
            } footer: {
                Text("Caps what summary and tagging rules spend in a day; summaries you ask for yourself are never held back.")
                    .foregroundStyle(.secondary)
            }

            if !note.isEmpty {
                Text(note).foregroundStyle(.secondary).font(.callout)
            }
        }
        .onSubmit {
            commitPendingKey()
            save()
        }
        .onChange(of: focus) { old, _ in
            switch old {
            case .key: saveKey()
            case .some: save()
            case nil: break
            }
        }
        .onAppear(perform: load)
        .onDisappear {
            commitPendingKey()
            save()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)) { _ in
            commitPendingKey()
            save()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.willResignActiveNotification)) { _ in
            commitPendingKey()
            save()
        }
    }

    @ViewBuilder
    private func rulesList(for action: AutoActionView) -> some View {
        ForEach(rules.indices.filter { rules[$0].action == action }, id: \.self) { i in
            AIRuleRow(
                rule: Binding(get: { rules[i] }, set: { rules[i] = $0; save() }),
                feeds: model.feeds,
                folders: model.folders
            ) {
                rules.remove(at: i)
                save()
            }
        }
        Button(action == .summarize ? "Add Summary Rule" : "Add Tagging Rule") {
            rules.append(AutoRuleView(scope: .all, action: action, minChars: 2000, onlyForeign: false))
            save()
        }
    }

    private var providerView: ProviderView {
        ProviderView(
            protocol: proto,
            baseUrl: baseURL.trimmingCharacters(in: .whitespacesAndNewlines),
            model: modelID.trimmingCharacters(in: .whitespacesAndNewlines),
            maxConcurrency: UInt32(concurrency)
        )
    }

    private func load() {
        guard let engine = model.engine, let s = try? engine.aiSettings() else { return }
        proto = s.summaryProvider?.protocol ?? .auto
        baseURL = s.summaryProvider?.baseUrl ?? ""
        modelID = s.summaryProvider?.model ?? ""
        concurrency = Int(s.summaryProvider?.maxConcurrency ?? 1)
        summaryLanguage = s.summaryLanguage
        syncSummaries = s.syncSummaries
        dailyLimit = Int(s.summaryDailyLimit)
        rules = s.rules.filter { $0.action == .summarize || $0.action == .tag }
        hasSavedKey = engine.hasAiRoleKey(role: .summary)
        loaded = true
    }

    private func save() {
        guard loaded, let engine = model.engine, let current = try? engine.aiSettings() else { return }
        let trimmedURL = baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedModel = modelID.trimmingCharacters(in: .whitespacesAndNewlines)
        let newProvider: ProviderView? = if trimmedURL.isEmpty || trimmedModel.isEmpty {
            nil
        } else {
            providerView
        }
        let translationRules = current.rules.filter { $0.action == .translate }
        let updated = AiSettingsView(
            translationProvider: current.translationProvider,
            targetLanguage: current.targetLanguage,
            glossary: current.glossary,
            syncTranslations: current.syncTranslations,
            translationDailyLimit: current.translationDailyLimit,
            summaryProvider: newProvider,
            summaryLanguage: summaryLanguage,
            syncSummaries: syncSummaries,
            summaryDailyLimit: UInt32(dailyLimit),
            rules: translationRules + rules
        )
        if current == updated { return }
        do {
            try engine.setAiSettings(settings: updated)
            if let normalized = try? engine.aiSettings().summaryProvider {
                baseURL = normalized.baseUrl
                modelID = normalized.model
            }
            flash("Saved")
        } catch {
            flash(error.localizedDescription, fade: false)
        }
    }

    private func commitPendingKey() {
        if !key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            saveKey()
        }
    }

    private func saveKey() {
        let trimmed = key.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let engine = model.engine else { return }
        do {
            try engine.setAiRoleKey(role: .summary, key: trimmed)
            key = ""
            hasSavedKey = engine.hasAiRoleKey(role: .summary)
            flash("API key saved")
        } catch {
            flash(error.localizedDescription, fade: false)
        }
    }

    private func removeKey() {
        guard let engine = model.engine else { return }
        do {
            try engine.setAiRoleKey(role: .summary, key: nil)
            key = ""
            hasSavedKey = false
            flash("API key removed")
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

    private func test() async {
        guard let engine = model.engine else { return }
        commitPendingKey()
        save()
        testing = true
        testNote = ""
        defer { testing = false }
        let pendingKey = key.trimmingCharacters(in: .whitespacesAndNewlines)
        do {
            let r = try await engine.testSummaryProvider(
                provider: providerView,
                key: pendingKey.isEmpty ? nil : pendingKey,
                summaryLanguage: summaryLanguage
            )
            let seconds = (Double(r.elapsedMs) / 1000).formatted(.number.precision(.fractionLength(1)))
            testNote = String(localized: "\(r.passed)/\(r.total) passed in \(seconds) s")
            if let error = r.error { testNote += " — " + error }
        } catch {
            testNote = error.localizedDescription
        }
    }
}

// MARK: - Shared Controls

@ViewBuilder
private func aiProtocolPicker(_ selection: Binding<AiProtocolView>, onChange: @escaping () -> Void) -> some View {
    SettingsChoice("Protocol", selection: selection, options: [
        (AiProtocolView.auto, String(localized: "Automatic")),
        (.openAi, String(localized: "OpenAI Compatible (/v1)")),
        (.gemini, String(localized: "Gemini Native (/v1beta)")),
    ])
    .onChange(of: selection.wrappedValue) { _, _ in onChange() }
}

/// One automatic rule: where it applies, and for translation whether it
/// skips articles already in the target language.
struct AIRuleRow: View {
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
            .flatPopup()
            Spacer(minLength: 8)
            if rule.action == .translate {
                Toggle("Foreign only", isOn: Binding(get: { rule.onlyForeign }, set: { rule = AutoRuleView(scope: rule.scope, action: rule.action, minChars: rule.minChars, onlyForeign: $0) }))
                    .toggleStyle(FlatToggleStyle(compact: true))
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.text2)
            }
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

    private func scope(from tag: String) -> RuleScopeView {
        if tag.hasPrefix("source:") { return .source(id: String(tag.dropFirst(7))) }
        if tag.hasPrefix("category:") { return .category(name: String(tag.dropFirst(9))) }
        return .all
    }
}
