//! The command line's side of the control socket.

use std::path::Path;

use eddy_core::core_version;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::protocol::{CONTROL_VERSION, ControlError, Frame, Progress, Request};

#[derive(Serialize)]
struct Outgoing<'a> {
    v: u32,
    core: String,
    request: &'a Request,
}

/// Sends `request` to the app listening at `socket` and waits for its
/// result. `app_unreachable` when nothing is listening there.
pub async fn call(
    socket: &Path,
    request: &Request,
    mut on_progress: impl FnMut(Progress),
) -> Result<serde_json::Value, ControlError> {
    let lost = |e: std::io::Error| ControlError::new("internal", format!("lost Eddy: {e}"));
    let stream = UnixStream::connect(socket).await.map_err(|e| {
        ControlError::new(
            "app_unreachable",
            format!("Eddy is not answering at {}: {e}", socket.display()),
        )
    })?;
    let (read, mut write) = stream.into_split();
    let mut line = serde_json::to_vec(&Outgoing {
        v: CONTROL_VERSION,
        core: core_version(),
        request,
    })?;
    line.push(b'\n');
    write.write_all(&line).await.map_err(lost)?;
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await.map_err(lost)? {
        match serde_json::from_str::<Frame>(&line)? {
            Frame::Progress(p) => on_progress(p),
            Frame::Ok(value) => return Ok(value),
            Frame::Error(e) => return Err(e),
        }
    }
    Err(ControlError::new(
        "internal",
        "Eddy closed the connection without an answer",
    ))
}
