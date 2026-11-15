fn main() {
    // Dev runs load pdfium.dll from next to the exe; copy it there whenever it
    // is available. Bundled builds ship it as a resource instead.
    if std::env::var("PDFIUM_DLL").is_err() {
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let dll = std::path::Path::new(&manifest)
            .join("../../build/pdfium/windows/release/lib/x64/pdfium.dll");
        if let Ok(resolved) = std::fs::canonicalize(&dll) {
            if let Ok(out_dir) = std::env::var("OUT_DIR") {
                // OUT_DIR = target/<profile>/build/eddy-win-<hash>/out;
                // the exe lives three levels up.
                let exe_dir = std::path::Path::new(&out_dir)
                    .ancestors()
                    .nth(3)
                    .map(std::path::Path::to_path_buf);
                if let Some(dir) = exe_dir {
                    let _ = std::fs::copy(&resolved, dir.join("pdfium.dll"));
                }
            }
        }
    }
    tauri_build::build()
}
