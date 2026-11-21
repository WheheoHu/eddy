//! Eddy driven from outside: by the command line, and through it by agents.
//!
//! A [`Request`] is carried out on an engine by [`execute`], and its result
//! is one of the shapes in [`dto`]. The command line runs that itself on a
//! data directory of its own; on the user's data it sends the request over
//! a Unix socket to the app, which is the only process that opens that
//! directory ([`serve`], [`call`]). See docs/adr/0011.

pub mod dto;
mod execute;
mod protocol;

#[cfg(unix)]
mod client;
#[cfg(unix)]
mod server;
#[cfg(unix)]
mod socket;

pub use execute::{ProgressFn, execute};
pub use protocol::{
    CONTROL_VERSION, ControlError, DocFormat, Envelope, Frame, Mark, Progress, ProgressTarget,
    Request, Scope,
};

#[cfg(unix)]
pub use client::call;
#[cfg(unix)]
pub use server::{ServerHandle, serve};
#[cfg(unix)]
pub use socket::{POINTER_NAME, SOCKET_NAME, private_dir, socket_path};
