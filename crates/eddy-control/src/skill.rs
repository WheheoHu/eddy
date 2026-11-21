//! The skill that tells agents the `eddy` command exists, and putting it
//! where they look.
//!
//! What is installed is a stub: a few lines that send the agent to
//! `eddy guide`. The guide comes from the installed command, so what agents
//! are told always matches the commands that version has, and nothing here
//! goes stale when Eddy is updated.
//!
//! Agents read skills from a directory of their own or from the shared
//! `~/.agents/skills`, which nearly all of them read. Claude Code reads only
//! its own. Writing those two places reaches every agent in [`AGENTS`].

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What `eddy guide` prints: the commands, written for an agent.
pub const GUIDE: &str = include_str!("guide.md");

/// The skill's name, which is also the name of its directory.
pub const SKILL_NAME: &str = "eddy";

/// When an agent should reach for the skill. Agents load this line for
/// every session and the rest only when it applies.
const DESCRIPTION: &str = "Work with the user's Eddy reading library through the eddy command. It lists and searches their articles, saved pages, books and PDFs, reads the text as Markdown, saves pages, files and notes, manages subscriptions and tags, and reads or sets reading progress. Use when the user mentions Eddy, their reading list or library, saved articles, feeds or subscriptions, or asks to save something to read later.";

/// Marks a `SKILL.md` as one Eddy wrote, so it can be updated and removed
/// without touching a skill someone else called `eddy`.
const MARK: &str = "eddy-stub:";

/// Raised when the stub's wording changes; older ones then count as out of
/// date.
const STUB_VERSION: u32 = 1;

/// The directories agents are looked for and skills are written under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Home {
    pub home: PathBuf,
    /// `CLAUDE_CONFIG_DIR`, where Claude Code keeps what is otherwise in
    /// `~/.claude`.
    pub claude_config_dir: Option<PathBuf>,
}

impl Home {
    /// From the environment; `None` when there is no home directory.
    pub fn current() -> Option<Home> {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Some(Home {
            home: var("HOME").or_else(|| var("USERPROFILE"))?,
            claude_config_dir: var("CLAUDE_CONFIG_DIR"),
        })
    }

    fn claude(&self) -> PathBuf {
        self.claude_config_dir
            .clone()
            .unwrap_or_else(|| self.home.join(".claude"))
    }

    /// `~/.agents/skills`, the directory agents share.
    fn shared(&self) -> PathBuf {
        self.home.join(".agents").join("skills")
    }
}

/// Where an agent reads user-level skills from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reads {
    /// `<its configuration directory>/skills`, and not the shared one.
    Claude,
    Shared,
}

struct Agent {
    id: &'static str,
    name: &'static str,
    /// Under the home directory; there when the agent has been used on
    /// this machine.
    sign: &'static str,
    reads: Reads,
}

/// The agents known to read skills, the ones most people use first.
const AGENTS: &[Agent] = &[
    Agent {
        id: "claude",
        name: "Claude Code",
        sign: ".claude",
        reads: Reads::Claude,
    },
    Agent {
        id: "codex",
        name: "Codex",
        sign: ".codex",
        reads: Reads::Shared,
    },
    Agent {
        id: "gemini",
        name: "Gemini CLI",
        sign: ".gemini",
        reads: Reads::Shared,
    },
    Agent {
        id: "cursor",
        name: "Cursor",
        sign: ".cursor",
        reads: Reads::Shared,
    },
    Agent {
        id: "copilot",
        name: "GitHub Copilot",
        sign: ".copilot",
        reads: Reads::Shared,
    },
    Agent {
        id: "opencode",
        name: "opencode",
        sign: ".config/opencode",
        reads: Reads::Shared,
    },
    Agent {
        id: "amp",
        name: "Amp",
        sign: ".config/amp",
        reads: Reads::Shared,
    },
    Agent {
        id: "goose",
        name: "Goose",
        sign: ".config/goose",
        reads: Reads::Shared,
    },
    Agent {
        id: "cline",
        name: "Cline",
        sign: ".cline",
        reads: Reads::Shared,
    },
    Agent {
        id: "roo",
        name: "Roo Code",
        sign: ".roo",
        reads: Reads::Shared,
    },
    Agent {
        id: "kilo",
        name: "Kilo Code",
        sign: ".kilo",
        reads: Reads::Shared,
    },
    Agent {
        id: "qwen",
        name: "Qwen Code",
        sign: ".qwen",
        reads: Reads::Shared,
    },
    Agent {
        id: "droid",
        name: "Droid",
        sign: ".factory",
        reads: Reads::Shared,
    },
    Agent {
        id: "devin",
        name: "Devin",
        sign: ".config/devin",
        reads: Reads::Shared,
    },
];

impl Agent {
    fn found(&self, home: &Home) -> bool {
        match self.reads {
            Reads::Claude => home.claude().is_dir(),
            Reads::Shared => home.home.join(self.sign).is_dir(),
        }
    }

    /// The directory the skill goes in for this agent.
    fn skill_dir(&self, home: &Home) -> PathBuf {
        match self.reads {
            Reads::Claude => home.claude().join("skills").join(SKILL_NAME),
            Reads::Shared => home.shared().join(SKILL_NAME),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillState {
    NotInstalled,
    /// Installed, and what would be installed now.
    Current,
    /// Installed by Eddy, but by another version or for a copy of the app
    /// that is somewhere else now.
    Outdated,
    /// A skill of this name is there that Eddy did not install.
    Conflict,
}

/// One agent and how the skill stands with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSkill {
    pub id: String,
    pub agent: String,
    /// The agent has been used on this machine.
    pub found: bool,
    /// The skill's directory for this agent.
    pub path: String,
    pub state: SkillState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillChange {
    Installed,
    Updated,
    Unchanged,
    /// Another skill of this name was there and has been replaced.
    Replaced,
    /// Another skill of this name is there and was left alone.
    Conflict,
}

/// What `setup` did, or with `dry_run` would do, in one skill directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillWrite {
    pub path: String,
    pub change: SkillChange,
}

/// The `SKILL.md` to install. `program` is the `eddy` executable, named for
/// the agent to fall back on when the command is not on the `PATH`.
pub fn stub(program: &Path) -> String {
    format!(
        "---\n\
name: {SKILL_NAME}\n\
description: {DESCRIPTION}\n\
metadata:\n  {MARK} \"{STUB_VERSION}\"\n\
---\n\
<!-- Installed by Eddy (Settings → Agents, or \"eddy skill setup\"). Remove it there, or with \"eddy skill remove\". -->\n\
\n\
Run `eddy guide` and follow what it prints. The guide comes from the installed Eddy, so it describes the commands that version has.\n\
\n\
If `eddy` is not found, the command is at `{program}`: use that path in its place.\n\
\n\
If that is not there either, Eddy has been moved or removed. Say so to the user, who can set this up again in Eddy's Settings → Agents.\n",
        program = program.display()
    )
}

/// How the skill stands with every known agent.
pub fn status(home: &Home, program: &Path) -> Vec<AgentSkill> {
    let wanted = stub(program);
    AGENTS
        .iter()
        .map(|agent| {
            let dir = agent.skill_dir(home);
            AgentSkill {
                id: agent.id.to_string(),
                agent: agent.name.to_string(),
                found: agent.found(home),
                state: state(&dir, &wanted),
                path: dir.to_string_lossy().into_owned(),
            }
        })
        .collect()
}

fn state(dir: &Path, wanted: &str) -> SkillState {
    match std::fs::read_to_string(dir.join("SKILL.md")) {
        Ok(found) if found == wanted => SkillState::Current,
        Ok(found) if found.contains(MARK) => SkillState::Outdated,
        Ok(_) => SkillState::Conflict,
        // Something by this name that is not a skill we can read.
        Err(_) if std::fs::symlink_metadata(dir).is_ok() => SkillState::Conflict,
        Err(_) => SkillState::NotInstalled,
    }
}

/// The directories `setup` writes: the shared one always, and Claude Code's
/// when Claude Code is there to read it.
fn targets(home: &Home) -> Vec<PathBuf> {
    let mut dirs = vec![home.shared().join(SKILL_NAME)];
    for agent in AGENTS.iter().filter(|a| a.reads != Reads::Shared) {
        let dir = agent.skill_dir(home);
        if agent.found(home) && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Installs the skill for the agents on this machine, or brings it up to
/// date. A skill named `eddy` that Eddy did not install is left alone
/// unless `force` is set. With `dry_run` nothing is written.
pub fn setup(
    home: &Home,
    program: &Path,
    force: bool,
    dry_run: bool,
) -> io::Result<Vec<SkillWrite>> {
    let wanted = stub(program);
    let mut done = Vec::new();
    for dir in targets(home) {
        let change = match state(&dir, &wanted) {
            SkillState::Current => SkillChange::Unchanged,
            SkillState::NotInstalled => SkillChange::Installed,
            SkillState::Outdated => SkillChange::Updated,
            SkillState::Conflict if force => SkillChange::Replaced,
            SkillState::Conflict => SkillChange::Conflict,
        };
        if !dry_run {
            match change {
                SkillChange::Unchanged | SkillChange::Conflict => {}
                SkillChange::Replaced => {
                    discard(&dir)?;
                    write(&dir, &wanted)?;
                }
                SkillChange::Installed | SkillChange::Updated => write(&dir, &wanted)?,
            }
        }
        done.push(SkillWrite {
            path: dir.to_string_lossy().into_owned(),
            change,
        });
    }
    Ok(done)
}

/// Removes the skill wherever Eddy installed it, and returns where that
/// was. Skills of the same name that are someone else's stay.
pub fn remove(home: &Home) -> io::Result<Vec<String>> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for agent in AGENTS {
        let dir = agent.skill_dir(home);
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    let mut removed = Vec::new();
    for dir in dirs {
        let file = dir.join("SKILL.md");
        let ours = std::fs::read_to_string(&file).is_ok_and(|text| text.contains(MARK));
        if !ours {
            continue;
        }
        std::fs::remove_file(&file)?;
        // The directory too, unless something else was put in it.
        let _ = std::fs::remove_dir(&dir);
        removed.push(dir.to_string_lossy().into_owned());
    }
    Ok(removed)
}

/// Written whole or not at all: an agent starting up never reads half of it.
fn write(dir: &Path, text: &str) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let partial = dir.join(format!(".SKILL.md.{}", std::process::id()));
    let written = std::fs::write(&partial, text)
        .and_then(|()| std::fs::rename(&partial, dir.join("SKILL.md")));
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    written
}

/// Takes away whatever has the skill's name: a directory, or a link to one
/// (the link only, never what it points to).
fn discard(dir: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(dir),
        Ok(_) => std::fs::remove_file(dir),
        Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM: &str = "/Applications/Eddy.app/Contents/Helpers/eddy";

    /// A home directory with these agents in it.
    fn home_with(signs: &[&str]) -> (tempfile::TempDir, Home) {
        let dir = tempfile::tempdir().unwrap();
        for sign in signs {
            std::fs::create_dir_all(dir.path().join(sign)).unwrap();
        }
        let home = Home {
            home: dir.path().to_path_buf(),
            claude_config_dir: None,
        };
        (dir, home)
    }

    fn of<'a>(all: &'a [AgentSkill], id: &str) -> &'a AgentSkill {
        all.iter().find(|a| a.id == id).unwrap()
    }

    fn changes(done: &[SkillWrite]) -> Vec<SkillChange> {
        done.iter().map(|w| w.change).collect()
    }

    #[test]
    fn the_stub_is_a_skill_every_agent_accepts() {
        let text = stub(Path::new(PROGRAM));
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("---"));
        let front: Vec<&str> = lines.by_ref().take_while(|l| *l != "---").collect();
        assert_eq!(front[0], "name: eddy");
        let description = front[1].strip_prefix("description: ").unwrap();
        // The limits of the Agent Skills specification, which the stricter
        // agents enforce: a name of lowercase letters, digits and hyphens
        // that is also the directory's name, and a description of at most
        // 1024 characters. Angle brackets are refused by some.
        assert!(
            SKILL_NAME
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        );
        assert!(
            description.chars().count() <= 1024,
            "{}",
            description.chars().count()
        );
        assert!(!description.contains(['<', '>']));
        // Plain YAML: a colon followed by a space would start a mapping.
        assert!(!description.contains(": "));
        // Only fields the specification has.
        assert_eq!(&front[2..], ["metadata:", "  eddy-stub: \"1\""]);
        assert!(text.contains("Run `eddy guide`"));
        assert!(text.contains(&format!("`{PROGRAM}`")));
        assert!(GUIDE.starts_with("# eddy"));
    }

    #[test]
    fn setup_writes_the_shared_directory_and_claude_codes() {
        let (dir, home) = home_with(&[".claude", ".codex"]);
        let before = status(&home, Path::new(PROGRAM));
        assert!(of(&before, "claude").found && of(&before, "codex").found);
        assert!(!of(&before, "cursor").found);
        assert!(before.iter().all(|a| a.state == SkillState::NotInstalled));

        // A dry run says what would happen and writes nothing.
        let planned = setup(&home, Path::new(PROGRAM), false, true).unwrap();
        assert_eq!(
            changes(&planned),
            [SkillChange::Installed, SkillChange::Installed]
        );
        assert!(!dir.path().join(".agents").exists());

        let done = setup(&home, Path::new(PROGRAM), false, false).unwrap();
        assert_eq!(done, planned);
        let shared = dir.path().join(".agents/skills/eddy/SKILL.md");
        let claude = dir.path().join(".claude/skills/eddy/SKILL.md");
        assert_eq!(
            std::fs::read_to_string(&shared).unwrap(),
            stub(Path::new(PROGRAM))
        );
        assert_eq!(
            std::fs::read_to_string(&claude).unwrap(),
            stub(Path::new(PROGRAM))
        );
        // Nothing but the skill is left in its directory.
        assert_eq!(
            std::fs::read_dir(shared.parent().unwrap()).unwrap().count(),
            1
        );

        let after = status(&home, Path::new(PROGRAM));
        assert!(after.iter().all(|a| a.state == SkillState::Current));
        assert_eq!(of(&after, "codex").path, of(&after, "cursor").path);
        assert_ne!(of(&after, "codex").path, of(&after, "claude").path);

        // Again changes nothing.
        let again = setup(&home, Path::new(PROGRAM), false, false).unwrap();
        assert_eq!(
            changes(&again),
            [SkillChange::Unchanged, SkillChange::Unchanged]
        );
    }

    #[test]
    fn claude_codes_directory_is_not_made_for_it() {
        let (dir, home) = home_with(&[".codex"]);
        let done = setup(&home, Path::new(PROGRAM), false, false).unwrap();
        assert_eq!(changes(&done), [SkillChange::Installed]);
        assert!(!dir.path().join(".claude").exists());
        // With no agent at all the shared directory is still written: an
        // agent installed later finds the skill there.
        let (bare, nobody) = home_with(&[]);
        let done = setup(&nobody, Path::new(PROGRAM), false, false).unwrap();
        assert_eq!(changes(&done), [SkillChange::Installed]);
        assert!(bare.path().join(".agents/skills/eddy/SKILL.md").exists());
    }

    #[test]
    fn claude_code_is_followed_to_where_its_configuration_was_moved() {
        let (dir, mut home) = home_with(&["elsewhere"]);
        home.claude_config_dir = Some(dir.path().join("elsewhere"));
        setup(&home, Path::new(PROGRAM), false, false).unwrap();
        assert!(dir.path().join("elsewhere/skills/eddy/SKILL.md").exists());
        assert!(!dir.path().join(".claude").exists());
    }

    #[test]
    fn a_stub_for_an_app_that_moved_is_out_of_date_and_updated() {
        let (_dir, home) = home_with(&[".claude"]);
        setup(
            &home,
            Path::new("/Users/reader/Downloads/Eddy.app/Contents/Helpers/eddy"),
            false,
            false,
        )
        .unwrap();
        let moved = status(&home, Path::new(PROGRAM));
        assert!(moved.iter().all(|a| a.state == SkillState::Outdated));
        let done = setup(&home, Path::new(PROGRAM), false, false).unwrap();
        assert_eq!(changes(&done), [SkillChange::Updated, SkillChange::Updated]);
        assert!(
            status(&home, Path::new(PROGRAM))
                .iter()
                .all(|a| a.state == SkillState::Current)
        );
    }

    #[test]
    fn someone_elses_skill_of_the_same_name_is_left_alone_unless_forced() {
        let (dir, home) = home_with(&[".claude"]);
        let theirs = dir.path().join(".claude/skills/eddy");
        std::fs::create_dir_all(theirs.join("references")).unwrap();
        std::fs::write(
            theirs.join("SKILL.md"),
            "---\nname: eddy\ndescription: Mine.\n---\nHello.\n",
        )
        .unwrap();
        std::fs::write(theirs.join("references/notes.md"), "notes").unwrap();

        assert_eq!(
            of(&status(&home, Path::new(PROGRAM)), "claude").state,
            SkillState::Conflict
        );
        let done = setup(&home, Path::new(PROGRAM), false, false).unwrap();
        assert_eq!(
            changes(&done),
            [SkillChange::Installed, SkillChange::Conflict]
        );
        assert!(
            std::fs::read_to_string(theirs.join("SKILL.md"))
                .unwrap()
                .contains("Mine.")
        );
        // Removing takes ours away and not theirs.
        let removed = remove(&home).unwrap();
        assert_eq!(removed.len(), 1);
        assert!(theirs.join("references/notes.md").exists());
        assert!(!dir.path().join(".agents/skills/eddy").exists());

        let forced = setup(&home, Path::new(PROGRAM), true, false).unwrap();
        assert_eq!(
            changes(&forced),
            [SkillChange::Installed, SkillChange::Replaced]
        );
        assert!(!theirs.join("references").exists());
        assert_eq!(
            of(&status(&home, Path::new(PROGRAM)), "claude").state,
            SkillState::Current
        );
    }

    #[test]
    fn remove_takes_the_skill_out_everywhere_and_keeps_what_was_added_beside_it() {
        let (dir, home) = home_with(&[".claude"]);
        setup(&home, Path::new(PROGRAM), false, false).unwrap();
        std::fs::write(dir.path().join(".claude/skills/eddy/mine.md"), "kept").unwrap();
        let removed = remove(&home).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(!dir.path().join(".agents/skills/eddy").exists());
        assert!(dir.path().join(".agents/skills").exists());
        assert!(dir.path().join(".claude/skills/eddy/mine.md").exists());
        assert!(!dir.path().join(".claude/skills/eddy/SKILL.md").exists());
        assert!(remove(&home).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn replacing_a_link_removes_the_link_and_not_what_it_points_to() {
        let (dir, home) = home_with(&[]);
        let elsewhere = dir.path().join("their-skills/eddy");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(
            elsewhere.join("SKILL.md"),
            "---\nname: eddy\ndescription: Theirs.\n---\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join(".agents/skills")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.path().join(".agents/skills/eddy")).unwrap();

        let forced = setup(&home, Path::new(PROGRAM), true, false).unwrap();
        assert_eq!(changes(&forced), [SkillChange::Replaced]);
        assert!(
            std::fs::read_to_string(elsewhere.join("SKILL.md"))
                .unwrap()
                .contains("Theirs.")
        );
        let ours = dir.path().join(".agents/skills/eddy");
        assert!(std::fs::symlink_metadata(&ours).unwrap().is_dir());
    }
}
