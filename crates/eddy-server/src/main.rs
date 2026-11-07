//! eddy-server [serve]            run the sync hub
//! eddy-server pairing-code [MIN] print a one-time pairing code (default 15 minutes)
//! eddy-server devices            list paired devices
//! eddy-server remove-device ID   revoke a device
//!
//! Environment: EDDY_DATA_DIR (default /data), EDDY_BIND (default
//! 0.0.0.0:8080), EDDY_MASTER_KEY (base64, 32 bytes), EDDY_REFRESH_MINUTES
//! (default 15).

use std::path::PathBuf;
use std::time::Duration;

use eddy_server::{
    AppState, ServerConfig, create_pairing_code, router, secrets::parse_master_key, spawn_poller,
};
use eddy_store::Store;

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn fail(message: &str) -> ! {
    eprintln!("eddy-server: {message}");
    std::process::exit(1)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let data_dir = PathBuf::from(env("EDDY_DATA_DIR", "/data"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str).unwrap_or("serve") {
        "serve" => serve(data_dir).await,
        "pairing-code" => {
            let minutes: u64 = args.get(1).and_then(|m| m.parse().ok()).unwrap_or(15);
            let store = Store::open(&data_dir).unwrap_or_else(|e| fail(&e.to_string()));
            let code = create_pairing_code(&store, Duration::from_secs(minutes * 60))
                .unwrap_or_else(|e| fail(&e));
            println!("{code}");
            eprintln!("valid for {minutes} minutes, single use");
        }
        "devices" => {
            let store = Store::open(&data_dir).unwrap_or_else(|e| fail(&e.to_string()));
            for d in store.devices().unwrap_or_else(|e| fail(&e.to_string())) {
                println!("{}  {}  last seen {:?}", d.id, d.name, d.last_seen);
            }
        }
        "remove-device" => {
            let id = args
                .get(1)
                .unwrap_or_else(|| fail("usage: eddy-server remove-device ID"));
            let store = Store::open(&data_dir).unwrap_or_else(|e| fail(&e.to_string()));
            store
                .remove_device(id)
                .unwrap_or_else(|e| fail(&e.to_string()));
        }
        other => fail(&format!("unknown command {other}")),
    }
}

async fn serve(data_dir: PathBuf) {
    let key = std::env::var("EDDY_MASTER_KEY")
        .unwrap_or_else(|_| fail("EDDY_MASTER_KEY is not set (openssl rand -base64 32)"));
    let master_key = parse_master_key(&key).unwrap_or_else(|e| fail(&e));
    let state = AppState::open(&ServerConfig {
        data_dir,
        master_key,
    })
    .unwrap_or_else(|e| fail(&e));
    let minutes: u64 = env("EDDY_REFRESH_MINUTES", "15")
        .parse()
        .unwrap_or(15)
        .max(1);
    spawn_poller(state.clone(), Duration::from_secs(minutes * 60));
    let bind = env("EDDY_BIND", "0.0.0.0:8080");
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .unwrap_or_else(|e| fail(&format!("bind {bind}: {e}")));
    tracing::info!("eddy-server listening on {bind}");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap_or_else(|e| fail(&e.to_string()));
}
