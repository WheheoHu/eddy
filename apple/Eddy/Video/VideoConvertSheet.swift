import EddyKit
import SwiftUI
import UniformTypeIdentifiers

struct VideoConvertSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let request: VideoConvertRequest

    @State private var source = ""
    @State private var target: VideoJobTargetView = .local
    @State private var pipeline: VideoPipelineModeView = .auto
    @State private var transcriptPolicy: TranscriptPolicyView = .preferAnySubtitles
    @State private var targetLanguage = ""
    @State private var useCookies = false
    @State private var cookieBrowser = "chrome"

    @State private var probing = false
    @State private var probe: VideoProbeView?
    @State private var probeError: String?
    @State private var pickingFile = false
    @State private var probeTask: Task<Void, Never>?

    private var isLocalFile: Bool {
        !source.isEmpty && isVideoFile(path: source) && FileManager.default.fileExists(atPath: source)
    }

    private var canRunOnServer: Bool {
        !isLocalFile && model.status?.mode == .server && model.serverVideoCapable
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Convert Presentation Video to Book")
                .font(.headline)

            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    TextField("https://www.youtube.com/watch?v=… or local video path", text: $source)
                        .textFieldStyle(.roundedBorder)
                        .onChange(of: source) { _, newValue in
                            scheduleProbe(newValue)
                        }
                    Button("Choose File…") { pickingFile = true }
                }

                probeSummaryView
            }

            Divider()

            Form {
                if canRunOnServer {
                    Picker("Run on", selection: $target) {
                        Text("Eddy Server (runs while Mac sleeps)").tag(VideoJobTargetView.server)
                        Text("This Mac").tag(VideoJobTargetView.local)
                    }
                    .pickerStyle(.radioGroup)
                } else {
                    LabeledContent("Run on", value: isLocalFile ? "This Mac (local hardware decode)" : "This Mac")
                }

                Picker("Visual pipeline", selection: $pipeline) {
                    Text("Automatic (Gemini Video + Writer)").tag(VideoPipelineModeView.auto)
                    Text("Gemini Native Video + Local HD Capture").tag(VideoPipelineModeView.geminiTwoStage)
                    Text("Local Scene Detection + VLM").tag(VideoPipelineModeView.localFramesTwoStage)
                }

                Picker("Transcript", selection: $transcriptPolicy) {
                    Text("Prefer Any Subtitles, Fallback to ASR").tag(TranscriptPolicyView.preferAnySubtitles)
                    Text("Prefer Manual Subtitles Only, Else ASR").tag(TranscriptPolicyView.preferManualSubtitles)
                    Text("Always Run Speech Recognition (ASR)").tag(TranscriptPolicyView.alwaysAsr)
                }

                TextField("Book language", text: $targetLanguage, prompt: Text("English / 简体中文"))

                if !isLocalFile && target == .local {
                    HStack {
                        Toggle("Use browser cookies (for signed-in videos)", isOn: $useCookies)
                        if useCookies {
                            Picker("", selection: $cookieBrowser) {
                                Text("Chrome").tag("chrome")
                                Text("Safari").tag("safari")
                                Text("Arc").tag("arc")
                                Text("Firefox").tag("firefox")
                            }
                            .labelsHidden()
                            .frame(width: 110)
                        }
                    }
                }
            }
            .formStyle(.grouped)
            .scrollDisabled(true)

            Text("Conversion takes a few minutes in the background. You can track chapter progress or retry interrupted steps from the toolbar activity popover; the finished multi-chapter book lands in Books.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Start Conversion") { submit() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(source.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 540)
        .onAppear(perform: loadDefaults)
        .onDisappear { probeTask?.cancel() }
        .fileImporter(
            isPresented: $pickingFile,
            allowedContentTypes: [.mpeg4Movie, .quickTimeMovie, .movie, .audiovisualContent],
            allowsMultipleSelection: false
        ) { result in
            if case let .success(urls) = result, let url = urls.first {
                source = url.path
                target = .local
            }
        }
    }

    @ViewBuilder
    private var probeSummaryView: some View {
        if probing {
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Inspecting video metadata and subtitle tracks…")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        } else if let probe {
            HStack(spacing: 8) {
                Image(systemName: probe.isLocalFile ? "film" : "play.rectangle.fill")
                    .foregroundStyle(.secondary)
                VStack(alignment: .leading, spacing: 2) {
                    Text(probe.title)
                        .font(.callout.weight(.medium))
                        .lineLimit(1)
                    HStack(spacing: 6) {
                        if let author = probe.author, !author.isEmpty {
                            Text(author)
                            Text("·")
                        }
                        Text(Self.formatDuration(ms: probe.durationMs))
                        Text("·")
                        Text(subtitleLabel(for: probe))
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
            .padding(8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 8))
        } else if let probeError {
            Text(probeError)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
        }
    }

    private func subtitleLabel(for p: VideoProbeView) -> String {
        if p.hasManualSubs { return "Manual subtitles available" }
        if p.hasAutoSubs { return "Auto captions available" }
        return "No subtitles (ASR will run)"
    }

    private static func formatDuration(ms: UInt64) -> String {
        let totalSec = Int((ms + 500) / 1000)
        let h = totalSec / 3600
        let m = (totalSec % 3600) / 60
        let s = totalSec % 60
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, s) : String(format: "%02d:%02d", m, s)
    }

    private func loadDefaults() {
        let vs = try? model.engine?.videoSettings()
        let ai = try? model.engine?.aiSettings()
        pipeline = vs?.pipeline ?? .auto
        transcriptPolicy = vs?.transcriptPolicy ?? .preferAnySubtitles
        let lang = vs?.targetLanguage.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        targetLanguage = lang.isEmpty ? (ai?.targetLanguage ?? "English") : lang

        if !request.source.isEmpty {
            source = request.source
        } else if let clip = NSPasteboard.general.string(forType: .string)?.trimmingCharacters(in: .whitespacesAndNewlines),
                  isVideoUrl(url: clip) {
            source = clip
        }

        Task {
            await model.refreshServerCapability()
            if canRunOnServer {
                target = .server
            }
        }
        if !source.isEmpty {
            scheduleProbe(source)
        }
    }

    private func scheduleProbe(_ value: String) {
        probeTask?.cancel()
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            probe = nil
            probeError = nil
            probing = false
            return
        }
        if isVideoFile(path: trimmed) && FileManager.default.fileExists(atPath: trimmed) {
            target = .local
        }
        guard trimmed.hasPrefix("http://") || trimmed.hasPrefix("https://") || FileManager.default.fileExists(atPath: trimmed) else {
            probe = nil
            probeError = nil
            return
        }
        probing = true
        probeError = nil
        let cookies = (useCookies && !isLocalFile) ? cookieBrowser : nil
        probeTask = Task {
            try? await Task.sleep(for: .milliseconds(350))
            if Task.isCancelled { return }
            do {
                let result = try await model.engine?.probeVideo(source: trimmed, cookiesFromBrowser: cookies)
                if Task.isCancelled { return }
                probe = result
                // Default to ASR when no manual subtitles exist and ASR key is configured
                if let result, !result.hasManualSubs, model.engine?.hasVideoStageKey(role: .asr) == true {
                    transcriptPolicy = .alwaysAsr
                }
            } catch {
                if Task.isCancelled { return }
                probe = nil
                probeError = error.localizedDescription
            }
            probing = false
        }
    }

    private func submit() {
        let trimmed = source.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        let effectiveTarget: VideoJobTargetView = canRunOnServer ? target : .local
        let cookies = (effectiveTarget == .local && !isLocalFile && useCookies) ? cookieBrowser : nil
        let lang = targetLanguage.trimmingCharacters(in: .whitespacesAndNewlines)

        let req = StartVideoJobRequestView(
            source: trimmed,
            target: effectiveTarget,
            pipeline: pipeline,
            transcriptPolicy: transcriptPolicy,
            targetLanguage: lang.isEmpty ? nil : lang,
            cookiesFromBrowser: cookies
        )
        dismiss()
        Task { await model.startVideoJob(req) }
    }
}
