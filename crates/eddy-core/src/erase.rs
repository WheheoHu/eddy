//! Signing out of a device and erasing what Eddy keeps on it.
//!
//! The data is not deleted under a running engine, which has the database
//! and the search index open and may have work in flight. Signing out
//! leaves a marker in the data directory instead, and the next `open`
//! erases before it opens anything. Once the marker is written the erase
//! happens whatever becomes of this process.

use std::path::Path;
use std::time::Duration;

use crate::engine::Engine;
use crate::{EngineError, LogLevel, Platform, Result};

const MARKER: &str = "erase-on-open";

/// Everything Eddy keeps in its data directory. Only these are removed:
/// the directory may be one a developer pointed `EDDY_DATA_DIR` at.
/// Add an entry here when the engine starts keeping something new there.
const OWNED: &[&str] = &[
    "eddy.sqlite3",
    "eddy.sqlite3-wal",
    "eddy.sqlite3-shm",
    "eddy.sqlite3-journal",
    "blobs",
    "index",
    "rulepacks",
    "spool",
    "video-jobs",
    "control.sock",
    "control.path",
];

/// How long signing out waits for changes that have not been sent yet.
const LAST_UPLOAD: Duration = Duration::from_secs(30);
/// How long it waits for the server to forget this device.
const UNPAIR: Duration = Duration::from_secs(8);

/// What `sign_out_and_erase` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignOut {
    /// Signed out. The data is erased when the engine is next opened, so
    /// drop this one (an app restarts itself).
    Erased,
    /// Nothing was done: this many changes exist only on this device and
    /// could not be sent. Ask again with `discard_unsent` to lose them.
    Unsent(u32),
}

/// Finishes an erase that was asked for, before anything in `dir` is
/// opened. A failure leaves the marker, so the next open tries again
/// rather than opening half of the old data.
pub(crate) fn finish_pending(dir: &Path, platform: &dyn Platform) -> Result<()> {
    let marker = dir.join(MARKER);
    if !marker.exists() {
        return Ok(());
    }
    platform.secret_clear();
    for name in OWNED.iter().chain([&MARKER]) {
        let path = dir.join(name);
        let removed = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&path),
            Ok(_) => std::fs::remove_file(&path),
            Err(_) => continue,
        };
        removed.map_err(|e| EngineError::Invalid(format!("无法清除本机数据（{name}）：{e}")))?;
    }
    platform.log(LogLevel::Info, "erased the data of a signed-out device");
    Ok(())
}

impl Engine {
    /// Changes that exist only on this device: not yet uploaded to the
    /// server, or (standalone) read/star changes FreshRSS has not received.
    pub fn unsent_changes(&self) -> Result<u32> {
        let inner = &self.inner;
        Ok(match inner.upstream.read().as_ref().map(|u| u.kind()) {
            Some("hub") => inner.store.upload_counts()?.0,
            Some(_) => inner.store.upstream_pending()?.len() as u32,
            None => 0,
        })
    }

    /// Signs this device out and erases everything Eddy keeps on it:
    /// library, downloaded content, search index, rule packs, accounts and
    /// every stored secret.
    ///
    /// Changes not sent yet are sent first. If some cannot be (the server
    /// is unreachable, say) nothing happens and `Unsent` says how many,
    /// unless `discard_unsent` accepts losing them. In server mode the
    /// server is told to forget this device; that it could not be told
    /// does not stop the sign-out.
    ///
    /// After `Erased` this engine is signed out and must not be used: the
    /// files go when the data directory is next opened.
    pub async fn sign_out_and_erase(&self, discard_unsent: bool) -> Result<SignOut> {
        let inner = &self.inner;
        if inner.options.server_role {
            return Err(EngineError::Invalid("服务端不能登出。".into()));
        }
        let upstream = inner.upstream.read().clone();
        if let Some(upstream) = upstream {
            match tokio::time::timeout(LAST_UPLOAD, upstream.push(inner)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => inner.platform.log(
                    LogLevel::Warn,
                    &format!("last upload before signing out failed: {e}"),
                ),
                Err(_) => inner
                    .platform
                    .log(LogLevel::Warn, "last upload before signing out timed out"),
            }
        }
        let unsent = self.unsent_changes()?;
        if unsent > 0 && !discard_unsent {
            return Ok(SignOut::Unsent(unsent));
        }

        // From here on the data is gone, whatever happens to this process.
        std::fs::write(inner.data_dir.join(MARKER), b"")
            .map_err(|e| EngineError::Invalid(format!("无法清除本机数据：{e}")))?;
        self.stop_live_sync();
        self.forget_on_server(UNPAIR).await;
        inner.platform.secret_clear();
        *inner.upstream.write() = None;
        *inner.direct.write() = None;
        inner
            .platform
            .log(LogLevel::Info, "signed out; data is erased on next open");
        Ok(SignOut::Erased)
    }

    /// Server mode: asks the server to drop this device and its token, so
    /// it does not stay listed (and valid) after the device left. Best
    /// effort: an unreachable or older server is not an error.
    pub(crate) async fn forget_on_server(&self, wait: Duration) {
        let Some(client) = self.inner.hub_client() else {
            return;
        };
        match tokio::time::timeout(wait, client.unpair()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => self.inner.platform.log(
                LogLevel::Warn,
                &format!("the server was not told this device left: {e}"),
            ),
            Err(_) => self.inner.platform.log(
                LogLevel::Warn,
                "the server was not told this device left: no answer",
            ),
        }
    }
}
