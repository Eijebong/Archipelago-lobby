use std::path::PathBuf;
use std::process::Command;

fn main() {
    let frontend_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("frontend");

    Command::new("yarn")
        .current_dir(&frontend_dir)
        .status()
        .unwrap();

    Command::new("yarn")
        .arg("build")
        .current_dir(&frontend_dir)
        .status()
        .unwrap();

    println!("cargo:rerun-if-changed=frontend/src");
    println!("cargo:rerun-if-changed=frontend/webpack.config.js");
    println!("cargo:rerun-if-changed=frontend/tsconfig.js");
    println!("cargo:rerun-if-changed=frontend/package.json");
    println!("cargo:rerun-if-changed=frontend/yarn.lock");
}
