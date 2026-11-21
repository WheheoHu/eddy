//! Whose data a command works on: a directory of its own, opened in this
//! process, or the user's library, through the app.

use std::path::{Component, Path, PathBuf};

/// Where the copy built from the repository keeps its data, relative to the
/// directory it is run in: never the user's.
pub const DEVELOPMENT_DATA: &str = "build/cli-data";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Open `data` in this process.
    Standalone { data: PathBuf },
    /// Ask the app that has `data` open.
    App {
        data: PathBuf,
        /// The app to start when nothing answers. `None` for a data
        /// directory someone pointed an app at themselves: starting an app
        /// here would start it on the user's own library.
        launch: Option<PathBuf>,
    },
}

/// What the choice depends on, gathered by [`Surroundings::current`].
#[derive(Clone, Debug, Default)]
pub struct Surroundings {
    /// This executable, with symbolic links followed.
    pub exe: Option<PathBuf>,
    /// `EDDY_DATA_DIR`, as the app reads it.
    pub data_override: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl Surroundings {
    pub fn current() -> Self {
        Surroundings {
            exe: std::env::current_exe()
                .ok()
                .map(|exe| std::fs::canonicalize(&exe).unwrap_or(exe)),
            data_override: std::env::var_os("EDDY_DATA_DIR")
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from),
            home: std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .map(PathBuf::from),
        }
    }

    /// The directory the app keeps the user's library in.
    pub fn library(&self) -> Option<PathBuf> {
        self.home
            .as_ref()
            .map(|home| home.join("Library/Application Support/Eddy"))
    }

    /// The `.app` this executable is inside of, if it is in one.
    pub fn bundle(&self) -> Option<PathBuf> {
        let exe = self.exe.as_ref()?;
        let inside = exe.parent()?;
        let contents = inside.parent()?;
        let bundle = contents.parent()?;
        let named = |path: &Path, names: &[&str]| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| names.contains(&n))
        };
        (named(inside, &["Helpers", "MacOS"])
            && named(contents, &["Contents"])
            && bundle.extension().is_some_and(|e| e == "app"))
        .then(|| bundle.to_path_buf())
    }
}

/// Chooses the mode from `--data`, `--app` and where the command is.
///
/// The copy inside the app works on the user's library unless told
/// otherwise. A copy anywhere else (one built in the repository) never
/// does on its own, and even with `--app` only reaches an app that was
/// started on a data directory named by `EDDY_DATA_DIR`.
pub fn choose(data: Option<&Path>, app: bool, around: &Surroundings) -> Result<Mode, String> {
    if let Some(data) = data {
        if around
            .library()
            .is_some_and(|library| same_place(&library, data))
        {
            return Err(format!(
                "{} is Eddy's own library: only the app opens it. Leave out --data to work on it through the app.",
                data.display()
            ));
        }
        return Ok(Mode::Standalone {
            data: data.to_path_buf(),
        });
    }
    let bundle = around.bundle();
    if !app && bundle.is_none() {
        return Ok(Mode::Standalone {
            data: PathBuf::from(DEVELOPMENT_DATA),
        });
    }
    match (&around.data_override, bundle) {
        (Some(data), _) => Ok(Mode::App {
            data: data.clone(),
            launch: None,
        }),
        (None, Some(bundle)) => match around.library() {
            Some(data) => Ok(Mode::App {
                data,
                launch: Some(bundle),
            }),
            None => Err("HOME is not set, so Eddy's library cannot be found.".into()),
        },
        (None, None) => Err(
            "--app from outside Eddy.app needs EDDY_DATA_DIR, naming the data directory a test copy of the app was started on. This copy does not touch your own library."
                .into(),
        ),
    }
}

/// Whether two paths name the same directory, without it having to exist.
fn same_place(a: &Path, b: &Path) -> bool {
    resolved(a) == resolved(b)
}

fn resolved(path: &Path) -> PathBuf {
    if let Ok(full) = std::fs::canonicalize(path) {
        return full;
    }
    let absolute = match std::env::current_dir() {
        Ok(here) if path.is_relative() => here.join(path),
        _ => path.to_path_buf(),
    };
    let mut out = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn around(exe: &str, data_override: Option<&str>) -> Surroundings {
        Surroundings {
            exe: Some(PathBuf::from(exe)),
            data_override: data_override.map(PathBuf::from),
            home: Some(PathBuf::from("/Users/reader")),
        }
    }

    const BUNDLED: &str = "/Applications/Eddy.app/Contents/Helpers/eddy";
    const BUILT: &str = "/Users/reader/Code/eddy/target/debug/eddy-cli";
    const LIBRARY: &str = "/Users/reader/Library/Application Support/Eddy";

    #[test]
    fn the_copy_in_the_app_works_on_the_library_and_can_start_the_app() {
        assert_eq!(
            choose(None, false, &around(BUNDLED, None)),
            Ok(Mode::App {
                data: PathBuf::from(LIBRARY),
                launch: Some(PathBuf::from("/Applications/Eddy.app")),
            })
        );
        // A test copy of the app, on its own data: found, never started.
        assert_eq!(
            choose(None, false, &around(BUNDLED, Some("/tmp/app-test"))),
            Ok(Mode::App {
                data: PathBuf::from("/tmp/app-test"),
                launch: None,
            })
        );
    }

    #[test]
    fn a_copy_built_in_the_repository_never_reaches_the_library() {
        assert_eq!(
            choose(None, false, &around(BUILT, None)),
            Ok(Mode::Standalone {
                data: PathBuf::from(DEVELOPMENT_DATA)
            })
        );
        // EDDY_DATA_DIR alone changes nothing: it is the app's variable.
        assert_eq!(
            choose(None, false, &around(BUILT, Some("/tmp/app-test"))),
            Ok(Mode::Standalone {
                data: PathBuf::from(DEVELOPMENT_DATA)
            })
        );
        assert!(choose(None, true, &around(BUILT, None)).is_err());
        assert_eq!(
            choose(None, true, &around(BUILT, Some("/tmp/app-test"))),
            Ok(Mode::App {
                data: PathBuf::from("/tmp/app-test"),
                launch: None,
            })
        );
    }

    #[test]
    fn a_data_directory_is_opened_here_unless_it_is_the_library() {
        for exe in [BUNDLED, BUILT] {
            assert_eq!(
                choose(Some(Path::new("build/smoke/a")), false, &around(exe, None)),
                Ok(Mode::Standalone {
                    data: PathBuf::from("build/smoke/a")
                })
            );
            assert!(choose(Some(Path::new(LIBRARY)), false, &around(exe, None)).is_err());
            let roundabout = format!("{LIBRARY}/../Eddy/.");
            assert!(choose(Some(Path::new(&roundabout)), false, &around(exe, None)).is_err());
        }
    }

    #[test]
    fn only_a_helper_inside_an_app_counts_as_bundled() {
        for (exe, bundle) in [
            (BUNDLED, Some("/Applications/Eddy.app")),
            (
                "/Users/reader/build/Eddy.app/Contents/MacOS/eddy",
                Some("/Users/reader/build/Eddy.app"),
            ),
            (BUILT, None),
            ("/usr/local/bin/eddy", None),
            ("/Applications/Eddy.app/Contents/Resources/eddy", None),
            ("/tmp/Helpers/eddy", None),
        ] {
            assert_eq!(
                around(exe, None).bundle(),
                bundle.map(PathBuf::from),
                "{exe}"
            );
        }
    }
}
