import EddyKit
import SwiftUI

/// Settings → Agents: what an agent (Claude Code, Codex, …) needs to work
/// on the library. The `eddy` command, and a skill that tells agents the
/// command is there. Nothing is installed until a button is pressed.
struct AgentsSettingsPage: View {
    @State private var agents: [AgentSkillView] = []
    @State private var note = ""

    var body: some View {
        Form {
            CommandLineSection()
            Section {
                if found.isEmpty {
                    Text("No agents found on this Mac.").foregroundStyle(Theme.text2)
                }
                ForEach(found, id: \.id) { agent in
                    LabeledContent(agent.agent) {
                        StatusValue(text: status(agent.state), good: agent.state == .current)
                    }
                }
                HStack {
                    if let title = setupTitle {
                        Button(title) { apply { try agentSkillSetup(programPath: $0, force: false) } }
                    }
                    if relevant.contains(where: { $0.state == .conflict }) {
                        Button("Replace") { apply { try agentSkillSetup(programPath: $0, force: true) } }
                            .help("Replace the skill named eddy that Eddy did not install")
                    }
                    if agents.contains(where: { $0.state == .current || $0.state == .outdated }) {
                        Button("Remove") { apply { try agentSkillRemove(programPath: $0) } }
                    }
                    if !note.isEmpty { Text(note).foregroundStyle(Theme.text2) }
                }
            } header: {
                Text("Agent Skill")
            } footer: {
                Text("A skill named eddy tells agents how to use the eddy command on your library. It goes into ~/.agents/skills, which Codex, Gemini CLI, Cursor, GitHub Copilot and most other agents read, and into Claude Code’s own ~/.claude/skills. Agents see it in their next session, and it works whether or not the command is installed above.")
                    .foregroundStyle(Theme.text2)
            }
        }
        .onAppear(perform: refresh)
    }

    /// The agents that have been used on this Mac.
    private var found: [AgentSkillView] { agents.filter(\.found) }

    /// The ones setting up is about: the agents here, or with none here
    /// the shared directory an agent installed later would read.
    private var relevant: [AgentSkillView] {
        found.isEmpty ? Array(agents.filter { !$0.path.contains("/.claude/") }.prefix(1)) : found
    }

    private var setupTitle: String? {
        if relevant.contains(where: { $0.state == .notInstalled }) { return String(localized: "Set Up Agents") }
        if relevant.contains(where: { $0.state == .outdated }) { return String(localized: "Update") }
        return nil
    }

    private func status(_ state: AgentSkillState) -> String {
        switch state {
        case .current: String(localized: "Set up")
        case .notInstalled: String(localized: "Not set up")
        case .outdated: String(localized: "Needs updating")
        case .conflict: String(localized: "Has another skill named eddy")
        }
    }

    private func refresh() {
        agents = (try? agentSkillStatus(programPath: CommandLineTool.program.path)) ?? []
    }

    private func apply(_ change: (String) throws -> [AgentSkillView]) {
        do {
            agents = try change(CommandLineTool.program.path)
            note = ""
        } catch {
            note = error.localizedDescription
        }
    }
}
