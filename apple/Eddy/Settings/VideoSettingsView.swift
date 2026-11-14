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
        case s2URL, s2Model, s2Key
        case asrURL, asrModel, asrKey
    }

    var body: some View {
        Form {
            toolchainSection
            generalSection
            stage1Section
            stage2Section
            asrSection
        }
        .formStyle(.grouped)
        .onSubmit { save() }
        .onChange(of: focus) { old, _ in
            switch old {
            case .s1Key: saveKey(.stage1, &stage1Key)
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
    }

    private var toolchainSection: some View {
        Section {
            LabeledContent("ffmpeg") {
                if let ver = toolchain?.ffmpegVersion ?? toolchain?.ffmpegPath {
                    Label(ver, systemImage: "checkmark.circle.fill")
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                } else {
                    Label("Not found (install via brew install ffmpeg)", systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                }
            }
            LabeledContent("yt-dlp") {
                if let ver = toolchain?.ytDlpVersion ?? toolchain?.ytDlpPath {
                    Label(ver, systemImage: "checkmark.circle.fill")
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                } else {
                    Label("Not found (install via brew install yt-dlp)", systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                }
            }
            if model.status?.mode == .server {
                LabeledContent("Eddy Server offload") {
                    Text(model.serverVideoCapable ? "Ready (configured via server .env)" : "Off (server .env not configured)")
                        .foregroundStyle(.secondary)
                }
            }
            TextField("Custom ffmpeg path (optional)", text: $ffmpegPath, prompt: Text("/opt/homebrew/bin/ffmpeg"))
                .focused($focus, equals: .ffmpegPath)
            TextField("Custom yt-dlp path (optional)", text: $ytDlpPath, prompt: Text("/opt/homebrew/bin/yt-dlp"))
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
            Picker("Visual pipeline", selection: $pipeline) {
                Text("Automatic (Gemini Video + Writer)").tag(VideoPipelineModeView.auto)
                Text("Gemini Native Video + Local HD Capture").tag(VideoPipelineModeView.geminiTwoStage)
                Text("Local Scene Detection + VLM").tag(VideoPipelineModeView.localFramesTwoStage)
            }
            .onChange(of: pipeline) { _, _ in save() }

            Picker("Default transcript", selection: $transcriptPolicy) {
                Text("Prefer Any Subtitles, Fallback to ASR").tag(TranscriptPolicyView.preferAnySubtitles)
                Text("Prefer Manual Subtitles Only, Else ASR").tag(TranscriptPolicyView.preferManualSubtitles)
                Text("Always Run Speech Recognition (ASR)").tag(TranscriptPolicyView.alwaysAsr)
            }
            .onChange(of: transcriptPolicy) { _, _ in save() }

            TextField("Book language (blank uses AI setting)", text: $targetLanguage, prompt: Text("English / 简体中文"))
                .focused($focus, equals: .targetLanguage)

            Stepper("Concurrent chapters: \(chapterConcurrency)", value: $chapterConcurrency, in: 1...6)
                .onChange(of: chapterConcurrency) { _, _ in save() }
        } header: {
            Text("Defaults")
        }
    }

    private var stage1Section: some View {
        Section {
            protocolPicker($stage1Protocol)
            TextField("API URL", text: $stage1URL, prompt: Text("https://generativelanguage.googleapis.com/v1beta"))
                .focused($focus, equals: .s1URL)
            TextField("Model", text: $stage1Model, prompt: Text("gemini-3.8-flash"))
                .focused($focus, equals: .s1Model)
            SecureField(hasKey(.stage1) ? "API key (saved in keychain)" : "API key (blank inherits AI key)", text: $stage1Key)
                .focused($focus, equals: .s1Key)
                .onSubmit { saveKey(.stage1, &stage1Key) }
            testRow(testing: testingStage1, note: stage1Note, disabled: stage1Model.isEmpty && stage1URL.isEmpty) {
                await runTest(role: .stage1, proto: stage1Protocol, url: stage1URL, model: stage1Model, key: stage1Key, testing: $testingStage1, note: $stage1Note)
            }
        } header: {
            Text("Stage 1 — Visual Slide Planner")
        } footer: {
            Text("Supports both Gemini native format (/v1beta, recommended for full-video timestamp localization) and OpenAI-compatible VLM endpoints (/v1). Keys stay strictly in this Mac’s keychain.")
                .foregroundStyle(.secondary)
        }
    }

    private var stage2Section: some View {
        Section {
            protocolPicker($stage2Protocol)
            TextField("API URL (blank inherits AI URL)", text: $stage2URL, prompt: Text("https://api.openai.com/v1"))
                .focused($focus, equals: .s2URL)
            TextField("Model (blank inherits AI model)", text: $stage2Model, prompt: Text("claude-opus-5-5"))
                .focused($focus, equals: .s2Model)
            SecureField(hasKey(.stage2) ? "API key (saved in keychain)" : "API key (blank inherits AI key)", text: $stage2Key)
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
            TextField("API URL", text: $asrURL, prompt: Text("https://dashscope.aliyuncs.com/compatible-mode/v1"))
                .focused($focus, equals: .asrURL)
            TextField("Model", text: $asrModel, prompt: Text("qwen3-asr-flash"))
                .focused($focus, equals: .asrModel)
            SecureField(hasKey(.asr) ? "API key (saved in keychain)" : "API key (blank inherits AI key)", text: $asrKey)
                .focused($focus, equals: .asrKey)
                .onSubmit { saveKey(.asr, &asrKey) }
            testRow(testing: testingAsr, note: asrNote, disabled: asrModel.isEmpty) {
                await runTest(role: .asr, proto: asrProtocol, url: asrURL, model: asrModel, key: asrKey, testing: $testingAsr, note: $asrNote)
            }
        } header: {
            Text("Speech Recognition (ASR)")
        } footer: {
            Text("Used for local video files or online videos without subtitles (supports Qwen OpenAI-compatible input_audio, Whisper /audio/transcriptions, and Gemini audio).")
                .foregroundStyle(.secondary)
        }
    }

    private func protocolPicker(_ selection: Binding<VideoProtocolView>) -> some View {
        Picker("Protocol", selection: selection) {
            Text("Automatic").tag(VideoProtocolView.auto)
            Text("OpenAI Compatible (/v1)").tag(VideoProtocolView.openAi)
            Text("Gemini Native (/v1beta)").tag(VideoProtocolView.gemini)
        }
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
        guard loaded else { return }
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
            stage2: VideoEndpointView(
                protocol: stage2Protocol,
                url: stage2URL.trimmingCharacters(in: .whitespacesAndNewlines),
                model: stage2Model.trimmingCharacters(in: .whitespacesAndNewlines)
            )
        )
        try? model.engine?.setVideoSettings(settings: settings)
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
        testing.wrappedValue = true
        note.wrappedValue = ""
        defer { testing.wrappedValue = false }

        let ai = try? model.engine?.aiSettings()
        let effectiveURL = url.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? (ai?.provider?.baseUrl ?? "")
            : url.trimmingCharacters(in: .whitespacesAndNewlines)
        let effectiveModel = modelID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? (ai?.provider?.model ?? "")
            : modelID.trimmingCharacters(in: .whitespacesAndNewlines)
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
