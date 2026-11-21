//! `eddy`: Eddy from the terminal, for people and for agents.
//!
//! The copy inside Eddy.app works on the user's library, by asking the app:
//! the app is the only process that opens that data directory, and it is
//! started in the background when it is not running. With `--data DIR` (and
//! always for a copy built in the repository, whose default is
//! `build/cli-data`) the engine runs in this process on a directory of its
//! own, with its secrets in a file there, apart from the app's data and the
//! system keychain. Either way a command becomes a request that
//! `eddy-control` carries out. See docs/adr/0011.

mod cli;
#[cfg(unix)]
mod launch;
mod local;
mod mode;
mod render;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::{CommandFactory, FromArgMatches};
use eddy_control::{ControlError, Progress, Request, execute};
use eddy_core::Engine;
use serde_json::Value;

use cli::{Action, Cli};
use mode::{Mode, Surroundings};

/// Not understood: the arguments, or what they ask for where they ask it.
const USAGE: u8 = 2;
/// The app did not answer.
const UNREACHABLE: u8 = 3;

struct Output {
    json: bool,
    quiet: bool,
}

enum Backend {
    Here(Engine),
    #[cfg(unix)]
    App(launch::App),
}

impl Backend {
    async fn send(&self, request: &Request, quiet: bool) -> Result<Value, ControlError> {
        // A long sync says how far it is, about once a second.
        let printed = Mutex::new(None::<Instant>);
        let report = move |p: Progress| {
            let mut printed = printed.lock().unwrap();
            if quiet || printed.is_some_and(|at| at.elapsed() < Duration::from_secs(1)) {
                return;
            }
            *printed = Some(Instant::now());
            eprintln!("{}", render::progress(&p));
        };
        match self {
            Backend::Here(engine) => execute(engine, request.clone(), Arc::new(report)).await,
            #[cfg(unix)]
            Backend::App(app) => app.send(request, report).await,
        }
    }

    /// A directory the other side can read and write, when the other side
    /// is the app; `None` when the work happens in this process.
    fn staging(&self) -> Result<Option<PathBuf>, ControlError> {
        match self {
            Backend::Here(_) => Ok(None),
            #[cfg(unix)]
            Backend::App(app) => app.staging().map(Some),
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let version: &'static str = Box::leak(
        format!(
            "{} (sync protocol {})",
            eddy_core::core_version(),
            eddy_core::SYNC_PROTOCOL_VERSION
        )
        .into_boxed_str(),
    );
    let matches = Cli::command().version(version).get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let output = Output {
        json: cli.json,
        quiet: cli.quiet,
    };
    let usage = |message: String| {
        eprintln!("error: {message}");
        ExitCode::from(USAGE)
    };
    let action = match cli.command.action() {
        Ok(action) => action,
        Err(message) => return usage(message),
    };
    if let Action::Guide = action {
        print!("{}", include_str!("guide.md"));
        return ExitCode::SUCCESS;
    }
    let mode = match mode::choose(cli.data.as_deref(), cli.app, &Surroundings::current()) {
        Ok(mode) => mode,
        Err(message) => return usage(message),
    };
    match run(mode, action, Duration::from_secs(cli.timeout), &output).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if output.json {
                println!("{}", serde_json::json!({ "error": e }));
            } else {
                eprintln!("error: {e}");
            }
            ExitCode::from(if e.code == "app_unreachable" {
                UNREACHABLE
            } else {
                1
            })
        }
    }
}

async fn run(
    mode: Mode,
    action: Action,
    timeout: Duration,
    output: &Output,
) -> Result<(), ControlError> {
    let backend = match mode {
        Mode::Standalone { data } => {
            if app_has_open(&data).await {
                return Err(ControlError::invalid(format!(
                    "Eddy is running on {}, and only one process opens a data directory. Use --app with EDDY_DATA_DIR set to it.",
                    data.display()
                )));
            }
            Backend::Here(local::open(&data).map_err(|e| ControlError::new("store", e))?)
        }
        #[cfg(unix)]
        Mode::App { data, launch } => Backend::App(launch::App {
            data,
            launch,
            timeout,
            quiet: output.quiet,
        }),
        #[cfg(not(unix))]
        Mode::App { .. } => {
            let _ = timeout;
            return Err(ControlError::invalid(
                "working through the app is not available on this platform; use --data DIR",
            ));
        }
    };
    match action {
        Action::Guide => Ok(()),
        Action::Local(command) => match &backend {
            Backend::Here(engine) => Ok(local::run(engine, command).await?),
            #[cfg(unix)]
            Backend::App(_) => Err(ControlError::invalid(
                "this command works on a data directory given with --data. Your library's accounts are set up in Eddy's Settings.",
            )),
        },
        Action::Request(request) => {
            let value = backend.send(&request, output.quiet).await?;
            print(&request, value, output)
        }
        Action::AddText {
            title,
            html,
            url,
            file,
        } => {
            let unread =
                |e: std::io::Error| ControlError::invalid(format!("cannot read the text: {e}"));
            let body = match &file {
                Some(path) => std::fs::read_to_string(path).map_err(unread)?,
                None => {
                    let mut text = String::new();
                    std::io::stdin().read_to_string(&mut text).map_err(unread)?;
                    text
                }
            };
            let request = Request::AddText {
                title,
                body,
                html,
                url,
            };
            let value = backend.send(&request, output.quiet).await?;
            print(&request, value, output)
        }
        Action::AddFile(path) => {
            let unread = |e: std::io::Error| {
                ControlError::invalid(format!("cannot read {}: {e}", path.display()))
            };
            let mut path = std::path::absolute(&path).map_err(unread)?;
            let staging = backend.staging()?;
            if let Some(dir) = &staging {
                let staged = dir.join(path.file_name().unwrap_or("file".as_ref()));
                if let Err(e) = std::fs::copy(&path, &staged) {
                    let _ = std::fs::remove_dir_all(dir);
                    return Err(unread(e));
                }
                path = staged;
            }
            let request = Request::AddFile {
                path: path.to_string_lossy().into_owned(),
            };
            let answer = backend.send(&request, output.quiet).await;
            if let Some(dir) = &staging {
                let _ = std::fs::remove_dir_all(dir);
            }
            print(&request, answer?, output)
        }
        Action::ExportEpub { id, output: wanted } => {
            let unwritten = |e: std::io::Error| {
                ControlError::invalid(format!("cannot write {}: {e}", wanted.display()))
            };
            let wanted_full = std::path::absolute(&wanted).map_err(unwritten)?;
            let staging = backend.staging()?;
            let written = match &staging {
                Some(dir) => dir.join(wanted_full.file_name().unwrap_or("export.epub".as_ref())),
                None => wanted_full.clone(),
            };
            let request = Request::ExportEpub {
                id,
                path: written.to_string_lossy().into_owned(),
            };
            let answer = backend.send(&request, output.quiet).await;
            let moved = match (&answer, &staging) {
                (Ok(_), Some(_)) => move_file(&written, &wanted_full).map_err(unwritten),
                _ => Ok(()),
            };
            if let Some(dir) = &staging {
                let _ = std::fs::remove_dir_all(dir);
            }
            let mut value = answer?;
            moved?;
            value["path"] = Value::String(wanted_full.to_string_lossy().into_owned());
            print(&request, value, output)
        }
    }
}

/// Whether an app answers on the control socket of `data`.
async fn app_has_open(data: &Path) -> bool {
    #[cfg(unix)]
    {
        let socket = eddy_control::socket_path(data);
        socket.exists()
            && eddy_control::call(&socket, &Request::Hello, |_| {})
                .await
                .is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = data;
        false
    }
}

/// Renames, or copies when the two are on different volumes.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

fn print(request: &Request, value: Value, output: &Output) -> Result<(), ControlError> {
    if output.json {
        let text = serde_json::to_string_pretty(&value)
            .map_err(|e| ControlError::new("internal", e.to_string()))?;
        println!("{text}");
        return Ok(());
    }
    // A list that goes on says so, where it does not get in the way of
    // reading the lines.
    let more = match request {
        Request::ItemsList { offset, .. } => {
            let shown = value["items"].as_array().map_or(0, Vec::len) as u64;
            let total = value["total"].as_u64().unwrap_or(0);
            let next = u64::from(*offset) + shown;
            (shown > 0 && next < total)
                .then(|| format!("{shown} of {total}; --offset {next} for the next ones"))
        }
        _ => None,
    };
    let text = render::text(request, value)?;
    if !text.is_empty() {
        println!("{text}");
    }
    if let Some(more) = more.filter(|_| !output.quiet) {
        eprintln!("{more}");
    }
    Ok(())
}
