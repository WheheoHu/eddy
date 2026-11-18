import AppKit
import EddyKit
import SwiftUI

struct VideoSettingsPage: View {
    @Environment(AppModel.self) private var model

    @State private var pipeline: VideoPipelineModeView = .auto
    @State private var transcriptPolicy: TranscriptPolicyView = .preferAnySubtitles
    @State private var targetLanguage = ""
    @State private var chapterConcurrency = 2
    @State private var ffmpegPath = ""
    @State private var ytDlpPath = ""

    @State private var stage1Protocol: VideoProtocolView = .auto
    @State private var stage1URL = ""
    @State private var stage1Model = ""
    @State private var stage1Key = ""
    @State private var testingStage1 = false
    @State private var stage1Note = ""

    @State private var vlmProtocol: VideoProtocolView = .auto
    @State private var vlmURL = ""
    @State private var vlmModel = ""
    @State private var vlmKey = ""
    @State private var testingVlm = false
    @State private var vlmNote = ""

    @State private var stage2Protocol: VideoProtocolView = .auto
    @State private var stage2URL = ""
    @State private var stage2Model = ""
    @State private var stage2Key = ""
    @State private var testingStage2 = false
    @State private var stage2Note = ""

    @State private var asrProtocol: VideoProtocolView = .auto
    @State private var asrURL = ""
    @State private var asrModel = ""
    @State private var asrKey = ""
    @State private var testingAsr = false
    @State private var asrNote = ""

    @State private var toolchain: VideoToolchainView?
    @State private var loaded = false
    @FocusState private var focus: Field?

    private enum Field: Hashable {
        case targetLanguage, ffmpegPath, ytDlpPath
        case s1URL, s1Model, s1Key
        case vlmURL, vlmModel, vlmKey
        case s2URL, s2Model, s2Key
        case asrURL, asrModel, asrKey
    }

    var body: some View {
        Form {
            toolchainSection
            generalSection
            stage1Section
            vlmSection
            stage2Section
            asrSection
        }
        .onSubmit {
            commitPendingKeys()
            save()
        }
        .onChange(of: focus) { old, _ in
            switch old {
            case .s1Key: saveKey(.stage1, &stage1Key)
            case .vlmKey: saveKey(.vlm, &vlmKey)
            case .s2Key: saveKey(.stage2, &stage2Key)
            case .asrKey: saveKey(.asr, &asrKey)
            case .ffmpegPath, .ytDlpPath:
                save()
                refreshToolchain()
            case .some:
                save()
            case nil:
                break
            }
        }
        .onAppear(perform: load)
        .onDisappear {
            commitPendingKeys()
            save()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.willCloseNotification)) { _ in
            commitPendingKeys()
            save()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.willResignActiveNotification)) { _ in
            commitPendingKeys()
            save()
        }
    }

    private var toolchainSection: some View {
        Section {
            LabeledContent("ffmpeg") {
                if let ver = toolchain?.ffmpegVersion ?? toolchain?.ffmpegPath {
                    StatusValue(text: ver, good: true)
                } else {
                    StatusValue(text: String(localized: "Not found (install via brew install ffmpeg)"), good: false)
                }
            }
            LabeledContent("yt-dlp") {
                if let ver = toolchain?.ytDlpVersion ?? toolchain?.ytDlpPath {
                    StatusValue(text: ver, good: true)
                } else {
                    StatusValue(text: String(localized: "Not found (install via brew install yt-dlp)"), good: false)
                }
            }
            if model.status?.mode == .server {
                LabeledContent("Eddy Server offload") {
                    Text(model.serverVideoCapable ? "Ready (configured via server .env)" : "Off (server .env not configured)")
                        .foregroundStyle(.secondary)
                }
            }
            SettingsField("Custom ffmpeg path (optional)", text: $ffmpegPath, prompt: Text("/opt/homebrew/bin/ffmpeg"))
                .focused($focus, equals: .ffmpegPath)
            SettingsField("Custom yt-dlp path (optional)", text: $ytDlpPath, prompt: Text("/opt/homebrew/bin/yt-dlp"))
                .focused($focus, equals: .ytDlpPath)
            Button("Re-check Toolchain") {
                save()
                refreshToolchain()
                Task { await model.refreshServerCapability() }
            }
        } header: {
            Text("Toolchain & Server")
        } footer: {
            Text("Local video files always use hardware-accelerated decoding on This Mac. Online video links can also run on your paired Eddy Server when EDDY_VIDEO_* keys are set in its deploy/.env.")
                .foregroundStyle(.secondary)
        }
    }

    private var generalSection: some View {
        Section {
            SettingsPicker("Default visual pipeline", selection: $pipeline) {
                Text("Automatic (Gemini Video if set, else VLM)").tag(VideoPipelineModeView.auto)
                Text("Gemini Native Video + Local HD Capture").tag(VideoPipelineModeView.geminiTwoStage)
                Text("Local Scene Detection + VLM").tag(VideoPipelineModeView.localFramesTwoStage)
            }
            .onChange(of: pipeline) { _, _ in save() }

            SettingsPicker("Default transcript", selection: $transcriptPolicy) {
                Text("Prefer Any Subtitles, Fallback to ASR").tag(TranscriptPolicyView.preferAnySubtitles)
                Text("Prefer Manual Subtitles Only, Else ASR").tag(TranscriptPolicyView.preferManualSubtitles)
                Text("Always Run Speech Recognition (ASR)").tag(TranscriptPolicyView.alwaysAsr)
            }
            .onChange(of: transcriptPolicy) { _, _ in save() }

            SettingsField("Book language", text: $targetLanguage, prompt: Text("English / 简体中文"), note: "Blank uses the Summaries language")
                .focused($focus, equals: .targetLanguage)

            SettingsStepper("Concurrent chapters", value: $chapterConcurrency, in: 1...6)
                .onChange(of: chapterConcurrency) { _, _ in save() }
        } header: {
            Text("Defaults")
        }
    }

    private var stage1Section: some View {
        Section {
            protocolPicker($stage1Protocol)
            SettingsField("API URL", text: $stage1URL, prompt: Text("https://generativelanguage.googleapis.com/v1beta"), width: 320)
                .focused($focus, equals: .s1URL)
            SettingsField("Model", text: $stage1Model, prompt: Text("gemini-3.8-flash"), width: 320)
                .focused($focus, equals: .s1Model)
            SettingsField("API key", text: $stage1Key, prompt: Text(hasKey(.stage1) ? "Saved in keychain" : "Blank inherits Stage 2 / Summaries key"), secure: true)
                .focused($focus, equals: .s1Key)
                .onSubmit { saveKey(.stage1, &stage1Key) }
            testRow(testing: testingStage1, note: stage1Note, disabled: stage1Model.isEmpty && stage1URL.isEmpty) {
                await runTest(role: .stage1, proto: stage1Protocol, url: stage1URL, model: stage1Model, key: stage1Key, testing: $testingStage1, note: $stage1Note)
            }
        } header: {
            Text("Stage 1A — Gemini Video Planner (Native Video Stream)")
        } footer: {
            Text("Used by “Gemini Native Video + Local HD Capture” to scan the full video stream and pinpoint exact slide timestamps.")
                .foregroundStyle(.secondary)
        }
    }

    private var vlmSection: some View {
        Section {
            protocolPicker($vlmProtocol)
            SettingsField("API URL", text: $vlmURL, prompt: Text("https://api.openai.com/v1"), note: "Blank inherits Stage 2 / Summaries URL", width: 320)
                .focused($focus, equals: .vlmURL)
            SettingsField("Model", text: $vlmModel, prompt: Text("qwen3-vl-plus / claude-opus-5-5"), note: "Blank inherits Stage 2 model", width: 320)
                .focused($focus, equals: .vlmModel)
            SettingsField("API key", text: $vlmKey, prompt: Text(hasKey(.vlm) ? "Saved in keychain" : "Blank inherits Stage 2 / Summaries key"), secure: true)
                .focused($focus, equals: .vlmKey)
                .onSubmit { saveKey(.vlm, &vlmKey) }
            testRow(testing: testingVlm, note: vlmNote, disabled: false) {
                await runTest(role: .vlm, proto: vlmProtocol, url: vlmURL, model: vlmModel, key: vlmKey, testing: $testingVlm, note: $vlmNote)
            }
        } header: {
            Text("Stage 1B — Vision Model / VLM (Local Scene Detection)")
        } footer: {
            Text("Used when running “Local Scene Detection + VLM” to inspect locally deduplicated candidate frames, filter out talking heads, and plan chapters. Keeps your VLM separate from Gemini Video above.")
                .foregroundStyle(.secondary)
        }
    }

    private var stage2Section: some View {
        Section {
            protocolPicker($stage2Protocol)
            SettingsField("API URL", text: $stage2URL, prompt: Text("https://api.openai.com/v1"), note: "Blank inherits Summaries URL", width: 320)
                .focused($focus, equals: .s2URL)
            SettingsField("Model", text: $stage2Model, prompt: Text("claude-opus-5-5"), note: "Blank inherits Summaries model", width: 320)
                .focused($focus, equals: .s2Model)
            SettingsField("API key", text: $stage2Key, prompt: Text(hasKey(.stage2) ? "Saved in keychain" : "Blank inherits Summaries key"), secure: true)
                .focused($focus, equals: .s2Key)
                .onSubmit { saveKey(.stage2, &stage2Key) }
            testRow(testing: testingStage2, note: stage2Note, disabled: false) {
                await runTest(role: .stage2, proto: stage2Protocol, url: stage2URL, model: stage2Model, key: stage2Key, testing: $testingStage2, note: $stage2Note)
            }
        } header: {
            Text("Stage 2 — Chapter Writer")
        } footer: {
            Text("Writes deep, reflowable chapters from each transcript slice and its captured 1080p slides.")
                .foregroundStyle(.secondary)
        }
    }

    private var asrSection: some View {
        Section {
            protocolPicker($asrProtocol)
            SettingsField("API URL", text: $asrURL, prompt: Text("https://dashscope.aliyuncs.com/compatible-mode/v1"), width: 320)
                .focused($focus, equals: .asrURL)
            SettingsField("Model", text: $asrModel, prompt: Text("qwen3-asr-flash"), width: 320)
                .focused($focus, equals: .asrModel)
            SettingsField("API key", text: $asrKey, prompt: Text(hasKey(.asr) ? "Saved in keychain" : "Blank inherits Summaries key"), secure: true)
                .focused($focus, equals: .asrKey)
                .onSubmit { saveKey(.asr, &asrKey) }
            testRow(testing: testingAsr, note: asrNote, disabled: asrModel.isEmpty) {
                await runTest(role: .asr, proto: asrProtocol, url: asrURL, model: asrModel, key: asrKey, testing: $testingAsr, note: $asrNote)
            }
        } header: {
            Text("Speech Recognition (ASR)")
        } footer: {
            Text("Sends ~2-minute speech segments with automatic 429 rate-limit backoff. Clicking Test sends a real 0.45s audio frame to verify the model.")
                .foregroundStyle(.secondary)
        }
    }

    private func protocolPicker(_ selection: Binding<VideoProtocolView>) -> some View {
        SettingsChoice("Protocol", selection: selection, options: [
            (VideoProtocolView.auto, String(localized: "Automatic")),
            (.openAi, String(localized: "OpenAI Compatible (/v1)")),
            (.gemini, String(localized: "Gemini Native (/v1beta)")),
        ])
        .onChange(of: selection.wrappedValue) { _, _ in save() }
    }

    private func testRow(testing: Bool, note: String, disabled: Bool, action: @escaping () async -> Void) -> some View {
        HStack {
            Button("Test") { Task { await action() } }
                .disabled(testing || disabled)
            if testing { ProgressView().controlSize(.small) }
            if !note.isEmpty {
                Text(note)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
        }
    }

    private func hasKey(_ role: VideoStageRoleView) -> Bool {
        model.engine?.hasVideoStageKey(role: role) == true
    }

    private func load() {
        guard let vs = try? model.engine?.videoSettings() else { return }
        pipeline = vs.pipeline
        transcriptPolicy = vs.transcriptPolicy
        targetLanguage = vs.targetLanguage
        chapterConcurrency = Int(vs.chapterConcurrency)
        ffmpegPath = vs.ffmpegPath
        ytDlpPath = vs.ytDlpPath

        stage1Protocol = vs.stage1.protocol
        stage1URL = vs.stage1.url
        stage1Model = vs.stage1.model

        vlmProtocol = vs.vlm.protocol
        vlmURL = vs.vlm.url
        vlmModel = vs.vlm.model

        stage2Protocol = vs.stage2.protocol
        stage2URL = vs.stage2.url
        stage2Model = vs.stage2.model

        asrProtocol = vs.asr.protocol
        asrURL = vs.asr.url
        asrModel = vs.asr.model

        refreshToolchain()
        Task { await model.refreshServerCapability() }
        loaded = true
    }

    private func refreshToolchain() {
        toolchain = try? model.engine?.checkVideoToolchain()
    }

    private func save() {
        guard loaded, let engine = model.engine else { return }
        let settings = VideoSettingsView(
            pipeline: pipeline,
            transcriptPolicy: transcriptPolicy,
            targetLanguage: targetLanguage.trimmingCharacters(in: .whitespacesAndNewlines),
            chapterConcurrency: UInt32(chapterConcurrency),
            ffmpegPath: ffmpegPath.trimmingCharacters(in: .whitespacesAndNewlines),
            ytDlpPath: ytDlpPath.trimmingCharacters(in: .whitespacesAndNewlines),
            asr: VideoEndpointView(
                protocol: asrProtocol,
                url: asrURL.trimmingCharacters(in: .whitespacesAndNewlines),
                model: asrModel.trimmingCharacters(in: .whitespacesAndNewlines)
            ),
            stage1: VideoEndpointView(
                protocol: stage1Protocol,
                url: stage1URL.trimmingCharacters(in: .whitespacesAndNewlines),
                model: stage1Model.trimmingCharacters(in: .whitespacesAndNewlines)
            ),
            vlm: VideoEndpointView(
                protocol: vlmProtocol,
                url: vlmURL.trimmingCharacters(in: .whitespacesAndNewlines),
                model: vlmModel.trimmingCharacters(in: .whitespacesAndNewlines)
            ),
            stage2: VideoEndpointView(
                protocol: stage2Protocol,
                url: stage2URL.trimmingCharacters(in: .whitespacesAndNewlines),
                model: stage2Model.trimmingCharacters(in: .whitespacesAndNewlines)
            )
        )
        if let current = try? engine.videoSettings(), current == settings { return }
        try? engine.setVideoSettings(settings: settings)
        if let saved = try? engine.videoSettings(), saved.asr.model != asrModel {
            asrModel = saved.asr.model
        }
    }

    private func commitPendingKeys() {
        if !stage1Key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            saveKey(.stage1, &stage1Key)
        }
        if !vlmKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            saveKey(.vlm, &vlmKey)
        }
        if !stage2Key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            saveKey(.stage2, &stage2Key)
        }
        if !asrKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            saveKey(.asr, &asrKey)
        }
    }

    private func saveKey(_ role: VideoStageRoleView, _ field: inout String) {
        let trimmed = field.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        try? model.engine?.setVideoStageKey(role: role, key: trimmed)
        field = ""
    }

    private func runTest(
        role: VideoStageRoleView,
        proto: VideoProtocolView,
        url: String,
        model modelID: String,
        key: String,
        testing: Binding<Bool>,
        note: Binding<String>
    ) async {
        save()
        if !key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            try? model.engine?.setVideoStageKey(role: role, key: key.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        testing.wrappedValue = true
        note.wrappedValue = ""
        defer { testing.wrappedValue = false }

        let ai = try? model.engine?.aiSettings()
        let trimmedURL = url.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedModel = modelID.trimmingCharacters(in: .whitespacesAndNewlines)
        let s2URL = stage2URL.trimmingCharacters(in: .whitespacesAndNewlines)
        let s2Model = stage2Model.trimmingCharacters(in: .whitespacesAndNewlines)

        let effectiveURL: String = if !trimmedURL.isEmpty {
            trimmedURL
        } else if role == .vlm, !s2URL.isEmpty {
            s2URL
        } else {
            ai?.summaryProvider?.baseUrl ?? ""
        }

        let effectiveModel: String = if !trimmedModel.isEmpty {
            trimmedModel
        } else if role == .vlm, !s2Model.isEmpty {
            s2Model
        } else {
            ai?.summaryProvider?.model ?? ""
        }

        let pendingKey = key.trimmingCharacters(in: .whitespacesAndNewlines)

        do {
            let ms = try await model.engine?.testVideoStage(
                role: role,
                protocol: proto,
                url: effectiveURL,
                model: effectiveModel,
                key: pendingKey.isEmpty ? nil : pendingKey
            ) ?? 0
            note.wrappedValue = "Connected (\(ms) ms)"
        } catch {
            note.wrappedValue = error.localizedDescription
        }
    }
}
