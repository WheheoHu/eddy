//! The app answering the command line (`eddy`), which works on the user's
//! data by asking the app rather than opening the data directory itself.

use eddy_core::EngineError;

use crate::{EddyEngine, R};

#[uniffi::export(async_runtime = "tokio")]
impl EddyEngine {
    /// Starts answering requests on the control socket of the data
    /// directory and returns the socket's path. Changes made for a request
    /// are announced through the sync listener's `on_change`, like changes
    /// that arrive from the server.
    ///
    /// Async only to run inside the runtime, where the listener lives.
    pub async fn start_control_server(&self) -> R<String> {
        let server = eddy_control::serve(self.engine.clone(), &self.data_dir)
            .map_err(|e| EngineError::Invalid(format!("control socket: {e}")))?;
        let socket = server.socket().to_string_lossy().into_owned();
        *self.control.lock().unwrap() = Some(server);
        Ok(socket)
    }

    /// Stops taking requests and removes the socket. Requests already being
    /// carried out finish.
    pub fn stop_control_server(&self) {
        self.control.lock().unwrap().take();
    }
}

/// How the skill that tells an agent about `eddy` stands with it.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentSkillState {
    NotInstalled,
    Current,
    /// Installed by Eddy, for another version or a copy that has moved.
    Outdated,
    /// A skill named `eddy` is there that Eddy did not install.
    Conflict,
}

/// An agent (Claude Code, Codex, …) and the skill's state for it.
#[derive(uniffi::Record, Clone, Debug)]
pub struct AgentSkillView {
    pub id: String,
    pub agent: String,
    /// The agent has been used on this machine.
    pub found: bool,
    /// The skill's directory for this agent; several agents share one.
    pub path: String,
    pub state: AgentSkillState,
}

fn skill_views(home: &eddy_control::skill::Home, program: &std::path::Path) -> Vec<AgentSkillView> {
    use eddy_control::skill::SkillState;
    eddy_control::skill::status(home, program)
        .into_iter()
        .map(|a| AgentSkillView {
            id: a.id,
            agent: a.agent,
            found: a.found,
            path: a.path,
            state: match a.state {
                SkillState::NotInstalled => AgentSkillState::NotInstalled,
                SkillState::Current => AgentSkillState::Current,
                SkillState::Outdated => AgentSkillState::Outdated,
                SkillState::Conflict => AgentSkillState::Conflict,
            },
        })
        .collect()
}

fn skill_home() -> R<eddy_control::skill::Home> {
    Ok(eddy_control::skill::Home::current()
        .ok_or_else(|| EngineError::Invalid("找不到用户目录。".into()))?)
}

/// Every known agent and how the `eddy` skill stands with it.
/// `program_path` is the `eddy` executable inside the app, which the skill
/// names for agents that do not have the command on their `PATH`.
#[uniffi::export]
pub fn agent_skill_status(program_path: String) -> R<Vec<AgentSkillView>> {
    Ok(skill_views(
        &skill_home()?,
        std::path::Path::new(&program_path),
    ))
}

/// Installs the skill for the agents on this machine, or brings it up to
/// date, and returns how things stand afterwards. A skill named `eddy` that
/// Eddy did not install is left alone unless `force` is set.
#[uniffi::export]
pub fn agent_skill_setup(program_path: String, force: bool) -> R<Vec<AgentSkillView>> {
    let home = skill_home()?;
    let program = std::path::Path::new(&program_path);
    eddy_control::skill::setup(&home, program, force, false)
        .map_err(|e| EngineError::Invalid(e.to_string()))?;
    Ok(skill_views(&home, program))
}

/// Removes the skill wherever Eddy installed it.
#[uniffi::export]
pub fn agent_skill_remove(program_path: String) -> R<Vec<AgentSkillView>> {
    let home = skill_home()?;
    eddy_control::skill::remove(&home).map_err(|e| EngineError::Invalid(e.to_string()))?;
    Ok(skill_views(&home, std::path::Path::new(&program_path)))
}
