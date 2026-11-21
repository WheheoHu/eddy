//! `eddy skill`: the skill that tells agents about this command, installed
//! where they look. The work is `eddy_control::skill`, which the app's
//! Settings use too; this is how it reads in a terminal.

use std::path::Path;

use eddy_control::ControlError;
use eddy_control::skill::{self, AgentSkill, Home, SkillChange, SkillState, SkillWrite};
use serde_json::json;

use crate::cli::SkillCommand;
use crate::mode::Surroundings;

pub fn run(
    command: SkillCommand,
    as_json: bool,
    around: &Surroundings,
) -> Result<(), ControlError> {
    let home = Home::current()
        .ok_or_else(|| ControlError::invalid("HOME is not set, so agents cannot be found."))?;
    // Agents are sent to this executable when `eddy` is not on their PATH.
    let program = around
        .exe
        .clone()
        .ok_or_else(|| ControlError::new("internal", "cannot tell where this command is"))?;
    let failed = |e: std::io::Error| ControlError::new("io", e.to_string());
    let tilde = |path: &str| match Path::new(path).strip_prefix(&home.home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.to_string(),
    };
    let print = |value: serde_json::Value| {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_default()
        )
    };
    match command {
        SkillCommand::Show => {
            let text = skill::stub(&program);
            if as_json {
                print(json!({ "name": skill::SKILL_NAME, "skill": text }));
            } else {
                print!("{text}");
            }
        }
        SkillCommand::Status => {
            let agents = skill::status(&home, &program);
            if as_json {
                print(json!({ "agents": agents }));
            } else {
                println!("{}", status_text(&agents, &tilde));
            }
        }
        SkillCommand::Setup { dry_run, force } => {
            let changes = skill::setup(&home, &program, force, dry_run).map_err(failed)?;
            if as_json {
                let agents = skill::status(&home, &program);
                print(json!({ "dry_run": dry_run, "changes": changes, "agents": agents }));
            } else {
                println!("{}", setup_text(&changes, dry_run, &tilde));
                if around.bundle().is_none() {
                    eprintln!(
                        "note: this eddy is not inside Eddy.app; agents are sent to {}",
                        program.display()
                    );
                }
            }
        }
        SkillCommand::Remove => {
            let removed = skill::remove(&home).map_err(failed)?;
            if as_json {
                print(json!({ "removed": removed }));
            } else if removed.is_empty() {
                println!("nothing to remove");
            } else {
                for path in &removed {
                    println!("removed  {}", tilde(path));
                }
            }
        }
    }
    Ok(())
}

/// One line for each agent found here: its name, how the skill stands with
/// it, and where the skill goes.
fn status_text(agents: &[AgentSkill], tilde: &dyn Fn(&str) -> String) -> String {
    let word = |state: SkillState| match state {
        SkillState::NotInstalled => "not installed",
        SkillState::Current => "installed",
        SkillState::Outdated => "out of date",
        SkillState::Conflict => "another skill is named eddy",
    };
    let found: Vec<&AgentSkill> = agents.iter().filter(|a| a.found).collect();
    if found.is_empty() {
        return "No agent was found on this Mac. `eddy skill setup` still installs the skill in ~/.agents/skills, where most agents look.".into();
    }
    found
        .iter()
        .map(|a| format!("{:<15} {:<14} {}", a.agent, word(a.state), tilde(&a.path)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn setup_text(changes: &[SkillWrite], dry_run: bool, tilde: &dyn Fn(&str) -> String) -> String {
    let mut lines: Vec<String> = changes
        .iter()
        .map(|w| {
            let word = match (w.change, dry_run) {
                (SkillChange::Installed, false) => "installed",
                (SkillChange::Installed, true) => "would install",
                (SkillChange::Updated, false) => "updated",
                (SkillChange::Updated, true) => "would update",
                (SkillChange::Replaced, false) => "replaced",
                (SkillChange::Replaced, true) => "would replace",
                (SkillChange::Unchanged, _) => "up to date",
                (SkillChange::Conflict, _) => "left alone",
            };
            format!("{word:<13} {}", tilde(&w.path))
        })
        .collect();
    if changes.iter().any(|w| w.change == SkillChange::Conflict) {
        lines.push(
            "A skill named eddy that Eddy did not install was left alone; --force replaces it."
                .into(),
        );
    }
    let wrote = changes.iter().any(|w| {
        matches!(
            w.change,
            SkillChange::Installed | SkillChange::Updated | SkillChange::Replaced
        )
    });
    if wrote && !dry_run {
        lines.push("Agents pick it up in their next session.".into());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(name: &str, found: bool, state: SkillState, path: &str) -> AgentSkill {
        AgentSkill {
            id: name.to_lowercase(),
            agent: name.into(),
            found,
            path: path.into(),
            state,
        }
    }

    #[test]
    fn status_lists_the_agents_that_are_here() {
        let plain = |path: &str| path.to_string();
        let text = status_text(
            &[
                agent(
                    "Claude Code",
                    true,
                    SkillState::Current,
                    "~/.claude/skills/eddy",
                ),
                agent(
                    "Codex",
                    true,
                    SkillState::NotInstalled,
                    "~/.agents/skills/eddy",
                ),
                agent(
                    "Cursor",
                    false,
                    SkillState::NotInstalled,
                    "~/.agents/skills/eddy",
                ),
            ],
            &plain,
        );
        assert_eq!(
            text,
            "Claude Code     installed      ~/.claude/skills/eddy\nCodex           not installed  ~/.agents/skills/eddy"
        );
        let nobody = status_text(
            &[agent("Cursor", false, SkillState::NotInstalled, "x")],
            &plain,
        );
        assert!(nobody.starts_with("No agent was found"));
    }

    #[test]
    fn setup_says_what_it_did_or_would_do() {
        let plain = |path: &str| path.to_string();
        let changes = [
            SkillWrite {
                path: "a".into(),
                change: SkillChange::Installed,
            },
            SkillWrite {
                path: "b".into(),
                change: SkillChange::Conflict,
            },
        ];
        let done = setup_text(&changes, false, &plain);
        assert!(done.starts_with("installed     a\nleft alone    b\n"));
        assert!(done.contains("--force replaces it"));
        assert!(done.ends_with("Agents pick it up in their next session."));
        let planned = setup_text(&changes, true, &plain);
        assert!(planned.starts_with("would install a\n"));
        assert!(!planned.contains("next session"));
    }
}
