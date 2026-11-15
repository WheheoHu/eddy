//! Stamps the build with the commit it was made from. Docker builds have no
//! `.git`; they pass `EDDY_GIT_HASH` instead.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=EDDY_GIT_HASH");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    let hash = std::env::var("EDDY_GIT_HASH")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .or_else(|| {
            let out = Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=EDDY_GIT_HASH={}", hash.trim());
}
