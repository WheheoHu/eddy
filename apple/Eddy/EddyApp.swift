import EddyKit
import SwiftUI
import UniformTypeIdentifiers

@main
struct EddyApp: App {
    @State private var model = AppModel()
    @AppStorage("app.mode") private var mode: AppMode = .threePane

    var body: some Scene {
        WindowGroup("Eddy") {
            RootView(mode: mode)
                .environment(model)
                .frame(minWidth: 960, minHeight: 620)
        }
        .defaultSize(width: 1360, height: 860)
        // One toolbar height whether or not a screen shows a title.
        .windowToolbarStyle(.unifiedCompact)
        .commands {
            CommandGroup(after: .newItem) {
                Button("Save Page…") { model.savingURL = true }.keyboardShortcut("l", modifiers: .command)
                Button("Import EPUB or PDF…") { model.importing = true }.keyboardShortcut("o", modifiers: .command)
                Button("Refresh") { Task { await model.refresh() } }.keyboardShortcut("r", modifiers: .command)
            }
            CommandGroup(before: .toolbar) {
                ForEach(Array(AppMode.allCases.enumerated()), id: \.element) { index, m in
                    Button(m.rawValue) { mode = m }.keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: .command)
                }
                Divider()
            }
        }
        Settings {
            SettingsView().environment(model)
        }
    }
}

struct RootView: View {
    @Environment(AppModel.self) private var model
    let mode: AppMode

    static let importTypes: [UTType] = [.epub, .pdf]

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
                }
            }
        }
        .overlay(alignment: .bottom) { MessageBanner() }
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
        .sheet(item: Binding(get: { model.pendingSeries.map(SeriesProposalBox.init) }, set: { if $0 == nil { model.pendingSeries = nil } })) { box in
            SeriesProposalSheet(proposal: box.proposal)
        }
    }
}

private struct SavePageSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var url = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Save a Page").font(.headline)
            TextField("https://", text: $url).textFieldStyle(.roundedBorder).frame(width: 420).onSubmit(save)
            Text("Eddy fetches the article now and keeps it for offline reading. Saving a table of contents offers to add the whole series.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .frame(width: 420, alignment: .leading)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Save") { save() }.keyboardShortcut(.defaultAction).disabled(url.isEmpty)
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
            Text("Add “\(proposal.title)” as a Publication?").font(.headline)
            Text("This page looks like a table of contents with \(proposal.chapters.count) chapters. Eddy can track your progress through all of them.")
                .foregroundStyle(.secondary)
            List(Array(proposal.chapters.prefix(50).enumerated()), id: \.offset) { index, chapter in
                Text("\(index + 1). \(chapter.title)").lineLimit(1)
            }
            .frame(width: 460, height: 220)
            HStack {
                Spacer()
                Button("Not Now") { model.pendingSeries = nil; dismiss() }.keyboardShortcut(.cancelAction)
                Button("Add Publication") { model.createSeries(proposal); dismiss() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
    }
}
