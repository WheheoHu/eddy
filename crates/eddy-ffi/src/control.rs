//! The app answering the command line (`eddy`), which works on the user's
//! data by asking the app rather than opening the data directory itself.

use eddy_core::EngineError;

use crate::{EddyEngine, R};

#[uniffi::export(async_runtime = "tokio")]
impl EddyEngine {
    /// Starts answering requests on the control socket of the data
    /// directory and returns the socket's path. Changes made for a request
    /// are announced through the sync listener's `on_change`, like changes
    /// that arrive from the server.
    ///
    /// Async only to run inside the runtime, where the listener lives.
    pub async fn start_control_server(&self) -> R<String> {
        let server = eddy_control::serve(self.engine.clone(), &self.data_dir)
            .map_err(|e| EngineError::Invalid(format!("control socket: {e}")))?;
        let socket = server.socket().to_string_lossy().into_owned();
        *self.control.lock().unwrap() = Some(server);
        Ok(socket)
    }

    /// Stops taking requests and removes the socket. Requests already being
    /// carried out finish.
    pub fn stop_control_server(&self) {
        self.control.lock().unwrap().take();
    }
}
