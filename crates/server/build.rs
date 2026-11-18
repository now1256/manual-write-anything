//! The web UI (web/dist) is embedded into the binary. Before the UI is built, a placeholder
//! page is created so the server still compiles and explains what to do.

use std::path::Path;

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
    let index = dist.join("index.html");
    if !index.exists() {
        std::fs::create_dir_all(&dist).expect("create web/dist");
        std::fs::write(
            &index,
            "<!doctype html><meta charset=utf-8><title>mwa</title>\
             <p>웹 화면이 아직 빌드되지 않았습니다. <code>cd web && npm install && npm run build</code> 후 \
             mwa을 다시 빌드하세요.</p>",
        )
        .expect("write placeholder");
    }
    println!("cargo:rerun-if-changed=../../web/dist");
}
