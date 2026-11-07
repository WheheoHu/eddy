uniffi::setup_scaffolding!();

#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn ping(delay_ms: u64) -> String {
    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    format!("pong from eddy-core {}", core_version())
}
