//! Requests sent to the app, which is started when it is not running.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eddy_control::{ControlError, Progress, Request, call, socket_path};
use serde_json::Value;

/// How the app is reached.
pub struct App {
    /// The data directory it has open.
    pub data: PathBuf,
    /// The bundle to start when nothing answers.
    pub launch: Option<PathBuf>,
    pub timeout: Duration,
    pub quiet: bool,
}

impl App {
    pub async fn send(
        &self,
        request: &Request,
        mut on_progress: impl FnMut(Progress),
    ) -> Result<Value, ControlError> {
        match call(&socket_path(&self.data), request, &mut on_progress).await {
            Err(e) if e.code == "app_unreachable" => {}
            answer => return answer,
        }
        let Some(bundle) = &self.launch else {
            return Err(ControlError::new(
                "app_unreachable",
                format!(
                    "no Eddy is running on {}. Start one on it first (open --env EDDY_DATA_DIR=… Eddy.app).",
                    self.data.display()
                ),
            ));
        };
        self.start(bundle).await?;
        call(&socket_path(&self.data), request, &mut on_progress).await
    }

    /// Starts the app in the background and waits until it answers.
    async fn start(&self, bundle: &Path) -> Result<(), ControlError> {
        let unreachable = |why: String| ControlError::new("app_unreachable", why);
        if !cfg!(target_os = "macos") {
            return Err(unreachable("Eddy is not running.".into()));
        }
        // -g and -j: behind what is in front, and hidden. `--background`
        // asks the app not to come forward on its own either.
        let opened = std::process::Command::new("/usr/bin/open")
            .args(["-g", "-j"])
            .arg(bundle)
            .args(["--args", "--background"])
            .status();
        if !opened.is_ok_and(|status| status.success()) {
            return Err(unreachable(format!("could not start {}", bundle.display())));
        }
        let begun = Instant::now();
        let mut said = false;
        loop {
            // Looked up each time: the app writes where its socket is as it
            // starts.
            if call(&socket_path(&self.data), &Request::Hello, |_| {})
                .await
                .is_ok()
            {
                return Ok(());
            }
            if begun.elapsed() >= self.timeout {
                return Err(unreachable(format!(
                    "Eddy was started but did not answer within {} seconds. It may be waiting for you (a keychain prompt), or be a version from before this command existed.",
                    self.timeout.as_secs()
                )));
            }
            if !said && !self.quiet && begun.elapsed() >= Duration::from_secs(3) {
                eprintln!("waiting for Eddy to start…");
                said = true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// A directory both this process and the app can use, for files handed
    /// from one to the other: the app is not asked to read the user's
    /// folders (which would put a permission prompt on a hidden app), and
    /// does not write into them.
    pub fn staging(&self) -> Result<PathBuf, ControlError> {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let failed = |e: std::io::Error| ControlError::new("internal", format!("staging: {e}"));
        let dir = eddy_control::private_dir(&self.data)
            .map_err(failed)?
            .join(format!(
                "staged-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
        std::fs::create_dir(&dir).map_err(failed)?;
        Ok(dir)
    }
}
