//! Where the control socket of a data directory is.

use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const SOCKET_NAME: &str = "control.sock";
/// Holds the socket's path when it could not be in the data directory.
pub const POINTER_NAME: &str = "control.path";

/// A socket address holds 104 bytes on macOS (108 on Linux), the
/// terminator included.
const LONGEST: usize = 100;

pub(crate) struct Place {
    pub socket: PathBuf,
    /// Where to write the socket's path for clients to find.
    pub pointer: Option<PathBuf>,
}

/// Where the server of `data_dir` listens: in the directory, or when that
/// path is too long for a socket, in the user's private directory under
/// `/tmp`, named after the data directory.
pub(crate) fn place(data_dir: &Path) -> io::Result<Place> {
    let direct = data_dir.join(SOCKET_NAME);
    if direct.as_os_str().len() <= LONGEST {
        return Ok(Place {
            socket: direct,
            pointer: None,
        });
    }
    let full = std::fs::canonicalize(data_dir)?;
    // FNV-1a: the same data directory gets the same name every time.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in full.as_os_str().as_encoded_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
    }
    Ok(Place {
        socket: private_dir(data_dir)?.join(format!("{hash:016x}.sock")),
        pointer: Some(data_dir.join(POINTER_NAME)),
    })
}

/// The socket to connect to for `data_dir`: the one its pointer file names,
/// or the one in it.
pub fn socket_path(data_dir: &Path) -> PathBuf {
    std::fs::read_to_string(data_dir.join(POINTER_NAME))
        .ok()
        .map(|path| PathBuf::from(path.trim()))
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| data_dir.join(SOCKET_NAME))
}

/// `/tmp/eddy-<uid>`, for the owner of `owned` alone: created when missing,
/// refused when it is anyone else's or open to others. `/tmp` rather than
/// the temporary directory of the session, whose path can be longer than a
/// socket address.
pub fn private_dir(owned: &Path) -> io::Result<PathBuf> {
    let uid = std::fs::metadata(owned)?.uid();
    let dir = PathBuf::from(format!("/tmp/eddy-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(&dir)?;
    if !meta.is_dir() || meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a private directory", dir.display()),
        ));
    }
    Ok(dir)
}
