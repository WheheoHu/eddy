fn main() {
    // eddy-pdf loads pdfium.dll from next to the exe; copy it there whenever
    // the fetch-pdfium.sh output is available. Missing output is not an
    // error (a clean checkout has not run the script yet); a failed copy is.
    if std::env::var("PDFIUM_DLL").is_err() {
        // src-tauri → eddy-win → windows → workspace root.
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let dll = std::path::Path::new(&manifest)
            .join("../../../build/pdfium/windows/release/lib/x64/pdfium.dll");
        if let Ok(resolved) = std::fs::canonicalize(&dll) {
            if let Ok(out_dir) = std::env::var("OUT_DIR") {
                // OUT_DIR = target/<profile>/build/eddy-win-<hash>/out;
                // the exe lives three levels up.
                if let Some(exe_dir) = std::path::Path::new(&out_dir)
                    .ancestors()
                    .nth(3)
                    .map(std::path::Path::to_path_buf)
                {
                    if let Err(e) = std::fs::copy(&resolved, exe_dir.join("pdfium.dll")) {
                        println!("cargo:warning=could not copy pdfium.dll next to the exe: {e}");
                    }
                }
            }
        }
    }
    tauri_build::build()
}
