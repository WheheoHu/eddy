//! The app's side of the control socket.

use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use eddy_core::{Engine, core_version};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};

use crate::execute::{ProgressFn, execute};
use crate::protocol::{CONTROL_VERSION, ControlError, Envelope, Frame};
use crate::socket::{self, POINTER_NAME};

/// Room for the longest text `add.text` takes, escaped as JSON.
const LONGEST_REQUEST: u64 = 64 * 1024 * 1024;

/// A running control server. Dropping it stops taking requests and removes
/// the socket; requests already being carried out finish.
pub struct ServerHandle {
    socket: PathBuf,
    pointer: Option<PathBuf>,
    inode: u64,
    task: tokio::task::JoinHandle<()>,
}

impl ServerHandle {
    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.task.abort();
        // Only our own: a newer instance may have put its socket there.
        if std::fs::metadata(&self.socket).is_ok_and(|m| m.ino() == self.inode) {
            let _ = std::fs::remove_file(&self.socket);
            if let Some(pointer) = &self.pointer {
                let _ = std::fs::remove_file(pointer);
            }
        }
    }
}

/// Starts answering requests for `engine` on the control socket of
/// `data_dir`. Call it inside the runtime the engine's own work runs on.
///
/// Only the user's own processes get an answer: the socket is theirs alone
/// to open, and the other end is checked to be them.
pub fn serve(engine: Engine, data_dir: &Path) -> io::Result<ServerHandle> {
    let place = socket::place(data_dir)?;
    // What an instance that is gone left behind. The data directory has one
    // app at a time, and the one that opened it last answers.
    let _ = std::fs::remove_file(&place.socket);
    let listener = UnixListener::bind(&place.socket)?;
    std::fs::set_permissions(&place.socket, std::fs::Permissions::from_mode(0o600))?;
    let meta = std::fs::metadata(&place.socket)?;
    match &place.pointer {
        Some(pointer) => std::fs::write(pointer, place.socket.as_os_str().as_encoded_bytes())?,
        None => {
            let _ = std::fs::remove_file(data_dir.join(POINTER_NAME));
        }
    }
    let owner = meta.uid();
    let runtime = tokio::runtime::Handle::current();
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };
            if !stream.peer_cred().is_ok_and(|peer| peer.uid() == owner) {
                continue;
            }
            // On a thread of its own. The app's runtime has a single thread
            // for everything it spawns, and a request that reads the
            // database or converts a page would hold up its network I/O.
            let (engine, runtime) = (engine.clone(), runtime.clone());
            tokio::task::spawn_blocking(move || runtime.block_on(answer(stream, engine)));
        }
    });
    Ok(ServerHandle {
        socket: place.socket,
        pointer: place.pointer,
        inode: meta.ino(),
        task,
    })
}

async fn answer(stream: UnixStream, engine: Engine) {
    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    let mut reader = BufReader::new(read).take(LONGEST_REQUEST);
    let frame = match reader.read_line(&mut line).await {
        Ok(_) if line.ends_with('\n') => carry_out(&engine, &line, &mut write).await,
        _ => Frame::Error(ControlError::new(
            "bad_request",
            "the request did not arrive whole",
        )),
    };
    let _ = send(&mut write, &frame).await;
    let _ = write.shutdown().await;
}

async fn carry_out(engine: &Engine, line: &str, write: &mut OwnedWriteHalf) -> Frame {
    let envelope: Envelope = match serde_json::from_str(line) {
        Ok(envelope) => envelope,
        Err(e) => {
            // A request written for another version may not read at all.
            let theirs = serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| v.get("v")?.as_u64());
            return Frame::Error(match theirs {
                Some(v) if v != u64::from(CONTROL_VERSION) => mismatch("another version"),
                _ => ControlError::new("bad_request", e.to_string()),
            });
        }
    };
    if envelope.v != CONTROL_VERSION {
        return Frame::Error(mismatch(&envelope.core));
    }
    let mutates = envelope.request.mutates();
    let (sender, mut waiting) = tokio::sync::mpsc::unbounded_channel();
    let progress: ProgressFn = Arc::new(move |p| {
        let _ = sender.send(p);
    });
    let work = execute(engine, envelope.request, progress);
    tokio::pin!(work);
    let result = loop {
        tokio::select! {
            result = &mut work => break result,
            Some(p) = waiting.recv() => {
                let _ = send(write, &Frame::Progress(p)).await;
            }
        }
    };
    // Whether it worked or stopped part-way, open lists may be stale.
    if mutates {
        engine.notify_changed();
    }
    match result {
        Ok(value) => Frame::Ok(value),
        Err(e) => Frame::Error(e),
    }
}

fn mismatch(theirs: &str) -> ControlError {
    ControlError::new(
        "version_mismatch",
        format!(
            "the running Eddy is {} and this command is {theirs}: quit and reopen Eddy so both are the same",
            core_version()
        ),
    )
}

async fn send(write: &mut OwnedWriteHalf, frame: &Frame) -> io::Result<()> {
    let mut line = serde_json::to_vec(frame)?;
    line.push(b'\n');
    write.write_all(&line).await
}
