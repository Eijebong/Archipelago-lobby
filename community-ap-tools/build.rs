use sha2::{Digest, Sha256};
use walkdir::WalkDir;

fn main() {
    let mut hasher = Sha256::new();
    for entry in WalkDir::new("static") {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            let path = entry.path().display().to_string();
            if path.ends_with(".css") || path.ends_with(".js") {
                hasher.update(std::fs::read(entry.path()).unwrap());
            }
        }
    }
    let hash = hasher.finalize();
    println!("cargo:rustc-env=STATIC_VERSION={hash:x}");
    println!("cargo:rerun-if-changed=static");
}
