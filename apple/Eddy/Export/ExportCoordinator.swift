import AppKit
import EddyKit
import Observation
import SwiftUI
import UniformTypeIdentifiers

@Observable
@MainActor
final class ActiveExportState: Identifiable {
    let id: String
    let title: String
    let format: ExportFormatView
    var phase: String
    var completed: UInt32 = 0
    var total: UInt32 = 0
    var cancelled = false

    init(id: String, title: String, format: ExportFormatView, phase: String = "Preparing export…") {
        self.id = id
        self.title = title
        self.format = format
        self.phase = phase
    }
}

final class ExportProgressBridge: ExportProgressListener, @unchecked Sendable {
    private let handler: @MainActor (ExportProgressView) -> Void

    init(_ handler: @escaping @MainActor (ExportProgressView) -> Void) {
        self.handler = handler
    }

    func onProgress(progress: ExportProgressView) {
        let handler = handler
        Task { @MainActor in handler(progress) }
    }
}

@Observable
@MainActor
final class ExportPanelChoices {
    var scope: ExportScopeView
    var mode: ExportContentModeView
    var includeSummary: Bool

    init(scope: ExportScopeView = .whole, mode: ExportContentModeView = .original, includeSummary: Bool = false) {
        self.scope = scope
        self.mode = mode
        self.includeSummary = includeSummary
    }
}

struct ExportAccessoryView: View {
    @Bindable var choices: ExportPanelChoices
    let format: ExportFormatView
    let options: ExportOptionsView
    let translationConfigured: Bool
    let onScopeChanged: (ExportScopeView) -> Void

    private var hasSummary: Bool {
        choices.scope == .whole ? options.hasCachedSummaryWhole : options.hasCachedSummaryChapter
    }

    private var isLosslessPassthrough: Bool {
        guard choices.mode == .original, choices.scope == .whole, !choices.includeSummary else {
            return false
        }
        return switch format {
        case .epub: options.canPassthroughEpub
        case .pdf: options.canPassthroughPdf
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 8) {
                GridRow {
                    Text("Content:")
                        .foregroundStyle(.secondary)
                        .gridColumnAlignment(.trailing)
                    Picker("", selection: $choices.mode) {
                        Text("Original").tag(ExportContentModeView.original)
                        Text("Bilingual").tag(ExportContentModeView.bilingual)
                        Text("Translation Only").tag(ExportContentModeView.translation)
                    }
                    .labelsHidden()
                    .pickerStyle(.segmented)
                    .frame(width: 280)
                }

                if options.canChooseScope {
                    GridRow {
                        Text("Scope:")
                            .foregroundStyle(.secondary)
                        Picker("", selection: $choices.scope) {
                            Text(options.isBook
                                 ? "Entire Book (\(options.chapterCount) chapters)"
                                 : "Entire Series (\(options.chapterCount) chapters)")
                                .tag(ExportScopeView.whole)
                            Text("Current Chapter Only")
                                .tag(ExportScopeView.currentChapter)
                        }
                        .labelsHidden()
                        .frame(width: 280)
                        .onChange(of: choices.scope) { _, newScope in
                            onScopeChanged(newScope)
                        }
                    }
                }

                if hasSummary {
                    GridRow {
                        Color.clear.frame(width: 1, height: 1)
                        Toggle("Include AI summary", isOn: $choices.includeSummary)
                    }
                }
            }

            if isLosslessPassthrough {
                Text(format == .pdf
                     ? "Exports the original PDF file directly."
                     : "Exports the original EPUB archive directly.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else if choices.mode != .original, !translationConfigured {
                Text("Untranslated sections will require a provider in Settings → Translation.")
                    .font(.caption)
                    .foregroundStyle(.orange)
            } else if choices.mode != .original {
                Text("Any untranslated sections will be translated automatically during export.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 12)
        .frame(width: 440)
    }
}

struct ExportProgressSheet: View {
    @Environment(AppModel.self) private var model
    @Bindable var state: ActiveExportState

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(state.format == .pdf ? "Exporting PDF" : "Exporting EPUB")
                .font(.headline)

            Text(state.title)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(1)

            if state.total > 1 {
                ProgressView(value: Double(state.completed), total: Double(state.total))
            } else {
                ProgressView()
                    .progressViewStyle(.linear)
            }

            HStack {
                Text(state.phase)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer()
                Button("Cancel") {
                    model.cancelActiveExport()
                }
                .keyboardShortcut(.cancelAction)
            }
        }
        .padding(20)
        .frame(width: 380)
        .interactiveDismissDisabled()
    }
}

extension ExportContentModeView {
    init(readingMode: ReadingMode) {
        switch readingMode {
        case .original: self = .original
        case .bilingual: self = .bilingual
        case .translation: self = .translation
        }
    }
}

extension AppModel {
    func canExport(_ targetID: String, fromReader: Bool) -> Bool {
        guard let engine else { return false }
        return (try? engine.exportOptions(targetId: targetID, fromReader: fromReader))?.exportable == true
    }

    var canExportCurrentItem: Bool {
        guard let id = currentItemID else { return false }
        return canExport(id, fromReader: true)
    }

    func exportCurrentItem(_ format: ExportFormatView) {
        guard let id = currentItemID else { return }
        readerCommand = PaletteRequest(value: ReaderCommand(itemID: id, action: .export(format)))
    }

    func cancelActiveExport() {
        guard let active = activeExport else { return }
        active.cancelled = true
        active.phase = "Canceling…"
        engine?.cancelExport(jobId: active.id)
    }

    func beginExport(
        targetID: String,
        format: ExportFormatView,
        fromReader: Bool,
        preferredMode: ReadingMode = .original
    ) {
        guard let engine else { return }
        let options: ExportOptionsView
        do {
            options = try engine.exportOptions(targetId: targetID, fromReader: fromReader)
        } catch {
            report(error)
            return
        }
        guard options.exportable else {
            message = "This item doesn’t support Reader export."
            return
        }

        let ext = format == .pdf ? "pdf" : "epub"
        let utType: UTType = format == .pdf ? .pdf : .epub
        let choices = ExportPanelChoices(
            scope: .whole,
            mode: ExportContentModeView(readingMode: preferredMode),
            includeSummary: false
        )

        let panel = NSSavePanel()
        panel.title = format == .pdf ? String(localized: "Export as PDF") : String(localized: "Export as EPUB")
        panel.prompt = String(localized: "Export")
        panel.allowedContentTypes = [utType]
        panel.canCreateDirectories = true
        panel.isExtensionHidden = false

        let wholeDefault = "\(options.wholeFilename).\(ext)"
        let chapterDefault = options.chapterFilename.map { "\($0).\(ext)" } ?? wholeDefault
        panel.nameFieldStringValue = wholeDefault

        let accessory = ExportAccessoryView(
            choices: choices,
            format: format,
            options: options,
            translationConfigured: translationConfigured,
            onScopeChanged: { [weak panel] newScope in
                guard let panel else { return }
                let current = panel.nameFieldStringValue
                if newScope == .currentChapter, current == wholeDefault {
                    panel.nameFieldStringValue = chapterDefault
                } else if newScope == .whole, current == chapterDefault {
                    panel.nameFieldStringValue = wholeDefault
                }
            }
        )
        let hosting = NSHostingView(rootView: accessory)
        hosting.frame = CGRect(origin: .zero, size: hosting.fittingSize)
        panel.accessoryView = hosting

        let handleCompletion: @MainActor (NSApplication.ModalResponse) -> Void = { [weak self] response in
            guard let self, response == .OK, let url = panel.url else { return }
            Task { @MainActor in
                await self.performExport(
                    targetID: targetID,
                    destination: url,
                    format: format,
                    options: options,
                    choices: choices
                )
            }
        }

        if let window = NSApp.keyWindow ?? NSApp.mainWindow {
            panel.beginSheetModal(for: window, completionHandler: handleCompletion)
        } else {
            handleCompletion(panel.runModal())
        }
    }

    private func performExport(
        targetID: String,
        destination: URL,
        format: ExportFormatView,
        options: ExportOptionsView,
        choices: ExportPanelChoices
    ) async {
        guard let engine else { return }
        let hasSummary = choices.scope == .whole ? options.hasCachedSummaryWhole : options.hasCachedSummaryChapter
        let includeSummary = hasSummary && choices.includeSummary
        let displayTitle = choices.scope == .whole ? (options.seriesTitle ?? options.itemTitle) : options.itemTitle
        let jobID = UUID().uuidString

        let state = ActiveExportState(id: jobID, title: displayTitle, format: format)
        activeExport = state

        let access = destination.startAccessingSecurityScopedResource()
        defer {
            if access { destination.stopAccessingSecurityScopedResource() }
            if activeExport?.id == jobID { activeExport = nil }
        }

        let listener = ExportProgressBridge { [weak state] progress in
            guard let state, !state.cancelled else { return }
            state.phase = progress.phase
            state.completed = progress.completed
            state.total = progress.total
        }

        let request = ExportRequestView(
            jobId: jobID,
            targetId: targetID,
            destinationPath: destination.path,
            scope: choices.scope,
            mode: choices.mode,
            includeSummary: includeSummary
        )

        do {
            switch format {
            case .epub:
                try await engine.exportEpub(request: request, listener: listener)

            case .pdf:
                let outcome = try await engine.exportOrPreparePdf(request: request, listener: listener)
                switch outcome {
                case .copiedOriginal:
                    break
                case let .prepared(payload):
                    if state.cancelled { throw PDFExporter.ExportRenderError.cancelled }
                    state.phase = "Typesetting PDF…"
                    state.completed = 0
                    state.total = 1

                    var imageMap: [String: Data] = [:]
                    for ch in payload.chapters {
                        for block in ch.blocks {
                            if case let .image(src, _, _, _, _) = block.kind, imageMap[src] == nil {
                                if state.cancelled { throw PDFExporter.ExportRenderError.cancelled }
                                if let img = try? await engine.image(url: src) {
                                    imageMap[src] = Data(img.bytes)
                                }
                            }
                        }
                    }

                    let serif = UserDefaults.standard.bool(forKey: "reader.serif")
                    let flagBox = CancelFlagBox(state)
                    try await Task.detached(priority: .userInitiated) {
                        try PDFExporter.render(
                            payload: payload,
                            images: imageMap,
                            destination: destination,
                            serif: serif,
                            translatedRuns: { text in
                                engine.translatedRuns(text: text)
                            },
                            isCancelled: { flagBox.isCancelled }
                        )
                    }.value
                }
            }

            if state.cancelled {
                try? FileManager.default.removeItem(at: destination)
                return
            }

            notice = Notice(
                text: String(localized: "Exported “\(destination.lastPathComponent)”"),
                kind: .exported(path: destination.path)
            )
        } catch {
            if state.cancelled || error.localizedDescription.localizedCaseInsensitiveContains("canceled") {
                try? FileManager.default.removeItem(at: destination)
                return
            }
            report(error)
        }
    }
}

private final class CancelFlagBox: @unchecked Sendable {
    private let lock = NSLock()
    private var cancelled = false

    @MainActor
    init(_ state: ActiveExportState) {
        cancelled = state.cancelled
        observe(state)
    }

    @MainActor
    private func observe(_ state: ActiveExportState) {
        withObservationTracking {
            _ = state.cancelled
        } onChange: { [weak self, weak state] in
            Task { @MainActor in
                guard let self, let state else { return }
                self.lock.lock()
                self.cancelled = state.cancelled
                self.lock.unlock()
                if !state.cancelled {
                    self.observe(state)
                }
            }
        }
    }

    var isCancelled: Bool {
        lock.lock()
        defer { lock.unlock() }
        return cancelled
    }
}
