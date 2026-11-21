import AppKit
import SwiftUI

/// The `eddy` command on the user's `PATH`: a symbolic link in
/// `/usr/local/bin` to the program inside this app. A link rather than a
/// copy, so the command is always the one that came with the app it talks
/// to, and where it sits tells it to work through the app.
enum CommandLineTool {
    static let link = URL(fileURLWithPath: "/usr/local/bin/eddy")

    /// The program inside this copy of the app.
    static var program: URL {
        Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/eddy").standardizedFileURL
    }

    enum State: Equatable {
        case notInstalled
        case installed
        /// The link leads somewhere else: another copy of Eddy, or where
        /// this one used to be before it was moved.
        case elsewhere(String)
        /// Something that is not a link has the name.
        case occupied
        /// macOS is running the app from a temporary place because it was
        /// never moved out of where it was downloaded to; a link to here
        /// would stop working.
        case translocated
        /// A build without the program in it.
        case missing
    }

    static func state() -> State {
        let files = FileManager.default
        guard files.isExecutableFile(atPath: program.path) else { return .missing }
        if let target = try? files.destinationOfSymbolicLink(atPath: link.path) {
            let leads = URL(fileURLWithPath: target, relativeTo: link.deletingLastPathComponent()).standardizedFileURL.path
            return leads == program.path ? .installed : .elsewhere(leads)
        }
        // Does not follow links, so anything at all with the name counts.
        if (try? files.attributesOfItem(atPath: link.path)) != nil { return .occupied }
        return Bundle.main.bundlePath.contains("/AppTranslocation/") ? .translocated : .notInstalled
    }

    /// Makes the link, or points it at this copy. Asks for an
    /// administrator's password when `/usr/local/bin` is not the user's to
    /// write, as it is not on a Mac out of the box. Returns false when the
    /// user cancelled that.
    static func install() throws -> Bool {
        let files = FileManager.default
        let directory = link.deletingLastPathComponent().path
        if files.isWritableFile(atPath: directory) {
            if (try? files.destinationOfSymbolicLink(atPath: link.path)) != nil { try files.removeItem(at: link) }
            try files.createSymbolicLink(atPath: link.path, withDestinationPath: program.path)
            return true
        }
        return try asAdministrator(
            "\"/bin/mkdir -p /usr/local/bin && /bin/ln -sfh \" & quoted form of (item 1 of argv) & \" /usr/local/bin/eddy\"",
            prompt: "Eddy wants to add the eddy command to /usr/local/bin.",
            argument: program.path
        )
    }

    /// Removes the link. Returns false when the user cancelled.
    static func remove() throws -> Bool {
        if FileManager.default.isWritableFile(atPath: link.deletingLastPathComponent().path) {
            try FileManager.default.removeItem(at: link)
            return true
        }
        return try asAdministrator(
            "\"/bin/rm -f /usr/local/bin/eddy\"",
            prompt: "Eddy wants to remove the eddy command from /usr/local/bin.",
            argument: ""
        )
    }

    /// Runs a shell command, given as an AppleScript expression, with the
    /// system's own password dialog. The one path that varies is handed
    /// over as an argument and quoted by AppleScript, never spliced into
    /// the script's text.
    private static func asAdministrator(_ command: String, prompt: String, argument: String) throws -> Bool {
        let script = "on run argv\ndo shell script \(command) with prompt \"\(prompt)\" with administrator privileges\nend run"
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", script, "--", argument]
        let errors = Pipe()
        process.standardError = errors
        process.standardOutput = Pipe()
        try process.run()
        let said = String(data: errors.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        process.waitUntilExit()
        if process.terminationStatus == 0 { return true }
        // -128: the user pressed Cancel.
        if said.contains("-128") { return false }
        throw NSError(domain: "CommandLineTool", code: Int(process.terminationStatus), userInfo: [
            NSLocalizedDescriptionKey: said.trimmingCharacters(in: .whitespacesAndNewlines),
        ])
    }
}

/// Settings → Saving: puts the `eddy` command on the `PATH` and takes it
/// off again. Nothing is installed unless the button is pressed.
struct CommandLineSection: View {
    @State private var state = CommandLineTool.state()
    @State private var working = false
    @State private var note = ""

    var body: some View {
        Section {
            LabeledContent("eddy") { StatusValue(text: status, good: state == .installed) }
            HStack {
                if let install {
                    Button(install) { run(CommandLineTool.install, done: String(localized: "Open a new Terminal window and type eddy.")) }
                }
                if removable {
                    Button("Remove") { run(CommandLineTool.remove, done: "") }
                }
                if !note.isEmpty { Text(note).foregroundStyle(Theme.text2) }
            }
            .disabled(working)
        } header: {
            Text("Command Line Tool")
        } footer: {
            Text("The eddy command works on your library from Terminal, and is how an agent can: eddy items --unread, eddy add url …, eddy doc …. It asks Eddy to do the work, starting it in the background when needed; eddy guide explains it to an agent. Installing links it into /usr/local/bin, which macOS asks an administrator’s password for.")
                .foregroundStyle(Theme.text2)
        }
        .onAppear { state = CommandLineTool.state() }
    }

    private var status: String {
        switch state {
        case .notInstalled: String(localized: "Not installed")
        case .installed: String(localized: "Installed in /usr/local/bin")
        case .elsewhere: String(localized: "Installed for another copy of Eddy")
        case .occupied: String(localized: "Another program is called eddy there")
        case .translocated: String(localized: "Move Eddy to Applications first")
        case .missing: String(localized: "Not part of this build")
        }
    }

    /// The install button's title, when installing is possible.
    private var install: String? {
        switch state {
        case .notInstalled: String(localized: "Install Command Line Tool")
        case .elsewhere: String(localized: "Use This Copy")
        case .installed, .occupied, .translocated, .missing: nil
        }
    }

    private var removable: Bool {
        switch state {
        case .installed, .elsewhere: true
        default: false
        }
    }

    /// The password dialog blocks whoever asked for it, so not the main
    /// thread.
    private func run(_ action: @escaping @Sendable () throws -> Bool, done: String) {
        working = true
        note = ""
        Task.detached {
            let result = Result { try action() }
            await MainActor.run {
                working = false
                state = CommandLineTool.state()
                switch result {
                case .success(true): note = done
                case .success(false): note = ""
                case let .failure(error): note = error.localizedDescription
                }
            }
        }
    }
}
