// No build script needed for server-side rendering
fn main() {
    println!("cargo:rerun-if-changed=templates/");
    println!("cargo:rerun-if-changed=static/");
}
