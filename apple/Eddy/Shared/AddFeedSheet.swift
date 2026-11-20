import EddyKit
import SwiftUI

/// What the Add Feed sheet starts from: nothing, or the feed the browser
/// extension found on a page.
struct AddFeedRequest: Identifiable, Equatable {
    let id = UUID()
    var url = ""
    /// The feed's name where it was found, shown above the address.
    var title: String?
}

/// Subscribes to a feed: an address, and the folder to file it in. FreshRSS
/// does the subscribing (through the sync server when there is one), so the
/// sheet stays up until it has answered and says what it said.
struct AddFeedSheet: View {
    private enum Folder: Hashable {
        case none, existing(String), new
    }

    private enum Field {
        case url, folder
    }

    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let request: AddFeedRequest
    @State private var url = ""
    @State private var folder = Folder.none
    @State private var newFolder = ""
    @State private var adding = false
    @State private var problem: String?
    @FocusState private var focus: Field?

    private var address: String { url.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var folderName: String { newFolder.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var unconfigured: Bool { model.status?.mode == .unconfigured }

    private var ready: Bool {
        !adding && !unconfigured && !address.isEmpty && (folder != .new || !folderName.isEmpty)
    }

    /// What the sheet says under the fields while nothing has gone wrong.
    private var hint: LocalizedStringKey {
        if unconfigured { return "Connect FreshRSS or a sync server in Settings to add feeds." }
        if model.status?.mode == .server, model.status?.serverOnline == false {
            return "The sync server is offline. Feeds can be added when it is back."
        }
        return "FreshRSS subscribes to the feed. Given a website’s address, it looks for the site’s feed."
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 3) {
                Text("Add Feed").font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.text1)
                if let title = request.title, !title.isEmpty {
                    Text(title).font(.system(size: 13)).foregroundStyle(Theme.text2).lineLimit(1)
                }
            }
            TextField("Feed or website address", text: $url)
                .flatField(focused: focus == .url)
                .focused($focus, equals: .url)
                .onSubmit(add)
            HStack(spacing: 10) {
                Text("Folder").font(.system(size: 13)).foregroundStyle(Theme.text2)
                Picker("Folder", selection: $folder) {
                    Text("No Folder").tag(Folder.none)
                    if !model.folders.isEmpty {
                        Divider()
                        ForEach(model.folders, id: \.self) { Text($0).tag(Folder.existing($0)) }
                    }
                    Divider()
                    Text("New Folder…").tag(Folder.new)
                }
                .labelsHidden()
                .flatPopup()
                // Always there, so the row keeps its height as the choice changes.
                TextField("Folder name", text: $newFolder)
                    .flatField(focused: focus == .folder)
                    .focused($focus, equals: .folder)
                    .onSubmit(add)
                    .opacity(folder == .new ? 1 : 0)
                    .disabled(folder != .new)
            }
            Group {
                if let problem {
                    Text(problem).foregroundStyle(Theme.danger).textSelection(.enabled)
                } else {
                    Text(hint).foregroundStyle(Theme.text2)
                }
            }
            .font(.system(size: 12))
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, minHeight: 32, alignment: .topLeading)
            HStack(spacing: 8) {
                if adding {
                    ProgressView().controlSize(.small)
                    Text("Subscribing…").font(.system(size: 12)).foregroundStyle(Theme.text2)
                }
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Add") { add() }.keyboardShortcut(.defaultAction).disabled(!ready)
            }
            .buttonStyle(KeyButtonStyle())
        }
        .padding(20)
        .frame(width: 440)
        .background(Theme.page)
        .onAppear {
            if !request.url.isEmpty {
                url = request.url
            } else if let copied = NSPasteboard.general.string(forType: .string)?.trimmingCharacters(in: .whitespacesAndNewlines),
                      copied.hasPrefix("http"), !copied.contains(where: \.isWhitespace) {
                url = copied
            }
            focus = .url
        }
        .onChange(of: folder) { _, chosen in
            if chosen == .new { focus = .folder }
        }
        .onChange(of: url) { problem = nil }
    }

    private func add() {
        guard ready else { return }
        let chosen: String? = switch folder {
        case .none: nil
        case let .existing(name): name
        case .new: folderName
        }
        adding = true
        problem = nil
        Task {
            do {
                try await model.addFeed(url: address, folder: chosen)
                dismiss()
            } catch EddyError.Offline {
                problem = String(localized: "The sync server is offline, and it is the one that subscribes. Try again when it is back.")
            } catch EddyError.NotConfigured {
                problem = String(localized: "Connect FreshRSS or a sync server in Settings to add feeds.")
            } catch let EddyError.Failed(message), let EddyError.UpgradeRequired(message), let EddyError.Unauthorized(message) {
                // The core's words, or FreshRSS's: not a feed, subscribed already.
                problem = message
            } catch {
                problem = error.localizedDescription
            }
            adding = false
        }
    }
}
