use sha2::{Digest, Sha256};
use walkdir::WalkDir;

fn main() -> anyhow::Result<()> {
    let mut css_hasher = Sha256::new();
    for entry in WalkDir::new("static/css") {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            css_hasher.update(std::fs::read_to_string(entry.path())?);
        }
    }

    let css_hash = css_hasher.finalize();
    println!("cargo:rustc-env=CSS_VERSION={:x}", css_hash);

    let mut js_hasher = Sha256::new();
    for entry in WalkDir::new("static/js") {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            js_hasher.update(std::fs::read_to_string(entry.path())?);
        }
    }

    let js_hash = js_hasher.finalize();
    println!("cargo:rustc-env=JS_VERSION={:x}", js_hash);

    Ok(())
}
