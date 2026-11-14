import EddyKit
import SwiftUI

/// Toolbar button & popover showing active, failed, and recently completed Video-to-Book jobs.
struct VideoJobsToolbarItem: ToolbarContent {
    @Environment(AppModel.self) private var model

    var body: some ToolbarContent {
        if !model.videoJobs.isEmpty {
            ToolbarItem {
                @Bindable var model = model
                let active = model.activeVideoJobsCount
                let hasFailed = model.videoJobs.contains { $0.status == "failed" }
                Button {
                    model.showingVideoJobs.toggle()
                } label: {
                    HStack(spacing: 5) {
                        if active > 0 {
                            ProgressView().controlSize(.small)
                            Text("\(active)")
                                .font(.caption.monospacedDigit())
                        } else if hasFailed {
                            Image(systemName: "exclamationmark.circle.fill")
                                .foregroundStyle(.orange)
                        } else {
                            Image(systemName: "checkmark.circle")
                        }
                    }
                }
                .help(active > 0 ? "\(active) video conversion(s) in progress" : "Video conversion tasks")
                .popover(isPresented: $model.showingVideoJobs, arrowEdge: .bottom) {
                    VideoJobsPopover()
                        .environment(model)
                }
            }
        }
    }
}

struct VideoJobsPopover: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Video Conversions")
                    .font(.headline)
                Spacer()
                if model.videoJobs.contains(where: { $0.status == "succeeded" || $0.status == "canceled" }) {
                    Button("Clear Finished") {
                        Task {
                            for job in model.videoJobs where job.status == "succeeded" || job.status == "canceled" {
                                await model.dismissVideoJob(job)
                            }
                        }
                    }
                    .buttonStyle(.borderless)
                    .font(.caption)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)

            Divider()

            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(model.videoJobs, id: \.id) { job in
                        VideoJobRow(job: job)
                        if job.id != model.videoJobs.last?.id {
                            Divider()
                        }
                    }
                }
                .padding(14)
            }
            .frame(maxHeight: 360)
        }
        .frame(width: 380)
        .task {
            // Poll server job updates smoothly while the popover or active server jobs are running
            while !Task.isCancelled {
                await model.refreshVideoJobs(initial: false)
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }
}

private struct VideoJobRow: View {
    @Environment(AppModel.self) private var model
    let job: VideoJobView

    private var isRunning: Bool {
        job.status == "queued" || job.status == "running"
    }

    private var progressFraction: Double? {
        guard isRunning else { return nil }
        switch job.phase {
        case .queued, .probing: return 0.06
        case .transcribing: return 0.20
        case .planningVisuals: return 0.36
        case .capturingFrames: return 0.48
        case .writingChapters:
            guard job.totalChapters > 0 else { return 0.55 }
            return 0.50 + 0.45 * (Double(job.completedChapters) / Double(max(job.totalChapters, 1)))
        case .packaging: return 0.97
        case .completed: return 1.0
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top, spacing: 6) {
                Image(systemName: job.target == .server ? "server.rack" : "laptopcomputer")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.top, 2)
                    .help(job.target == .server ? "Running on Eddy Server" : "Running on This Mac")

                Text(job.title)
                    .font(.callout.weight(.medium))
                    .lineLimit(2)

                Spacer(minLength: 6)

                actions
            }

            if let fraction = progressFraction {
                ProgressView(value: fraction)
                    .controlSize(.small)
            }

            if let err = job.error, job.status == "failed" {
                Text(err)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .lineLimit(3)
                    .textSelection(.enabled)
            } else {
                Text(phaseText)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }

    private var phaseText: String {
        if job.status == "succeeded" {
            return job.detail.isEmpty ? "Completed" : job.detail
        }
        if job.status == "canceled" {
            return "Canceled"
        }
        if !job.detail.isEmpty {
            return job.detail
        }
        return switch job.phase {
        case .queued: "Queued…"
        case .probing: "1/4 Inspecting video metadata…"
        case .transcribing: "1/4 Extracting transcript / ASR…"
        case .planningVisuals: "2/4 Scanning presentation slides…"
        case .capturingFrames: "3/4 Capturing 1080p frames…"
        case .writingChapters:
            job.totalChapters > 0
                ? "4/4 Writing chapter \(job.completedChapters) of \(job.totalChapters)…"
                : "4/4 Writing chapters…"
        case .packaging: "Packaging multi-chapter book…"
        case .completed: "Completed"
        }
    }

    @ViewBuilder
    private var actions: some View {
        if isRunning {
            Button {
                Task { await model.cancelVideoJob(job) }
            } label: {
                Image(systemName: "xmark.circle")
            }
            .buttonStyle(.borderless)
            .help("Cancel")
        } else if job.status == "failed" || job.status == "canceled" {
            HStack(spacing: 6) {
                Button("Retry") {
                    Task { await model.retryVideoJob(job) }
                }
                .controlSize(.small)
                .help("Resume from saved stage checkpoints")

                Button {
                    Task { await model.dismissVideoJob(job) }
                } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(.borderless)
                .help("Remove")
            }
        } else if job.status == "succeeded" {
            HStack(spacing: 6) {
                if let itemID = job.resultItemId {
                    Button("Open") {
                        model.showingVideoJobs = false
                        model.reveal(itemID: itemID)
                    }
                    .controlSize(.small)
                }
                Button {
                    Task { await model.dismissVideoJob(job) }
                } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.borderless)
                .help("Dismiss")
            }
        }
    }
}
