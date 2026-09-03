//! Assemble classic-script API families in one scope and a fixed order.
use std::{env, fs, path::PathBuf};
fn main() {
    let source = PathBuf::from("src/shim");
    println!("cargo:rerun-if-changed=src/shim/manifest.txt");
    let manifest = fs::read_to_string(source.join("manifest.txt")).expect("shim manifest");
    let mut combined = String::new();
    for name in manifest.lines().filter(|line| !line.is_empty()) {
        let path = source.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        combined.push_str(&fs::read_to_string(path).expect("shim API family"));
        combined.push('\n');
    }
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("shim.js"),
        combined,
    )
    .expect("assembled shim");
}
