use sha2::{Digest, Sha256};
use walkdir::WalkDir;

fn main() -> anyhow::Result<()> {
    let mut hasher = Sha256::new();
    for entry in WalkDir::new("static/css") {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            hasher.update(std::fs::read_to_string(entry.path())?);
        }
    }

    let hash = hasher.finalize();
    println!("cargo:rustc-env=CSS_VERSION={:x}", hash);

    Ok(())
}
