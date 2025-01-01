use anyhow::Result;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

fn main() -> Result<()> {
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

    println!("cargo:rustc-env=GIT_VERSION={}", derive_git_version()?);

    Ok(())
}

fn derive_git_version() -> Result<String> {
    let mut branch_name = "unknown";

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    let repo = git2::Repository::open(manifest_dir)?;
    let head = repo.head()?;
    if head.is_branch() {
        branch_name = head.name().unwrap().strip_prefix("refs/heads/").unwrap();
    }

    let mut walk = repo.revwalk()?;
    walk.push(head.target().unwrap())?;
    let number = walk.count();

    Ok(format!("{}-{}", branch_name, number))
}
