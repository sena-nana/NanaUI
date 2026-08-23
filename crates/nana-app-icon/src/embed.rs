use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Embed the default mark into this package's Windows binaries.
pub fn embed_windows() {
    embed(None);
}

/// Embed a custom `.ico` into this package's Windows binaries.
pub fn embed_windows_from(icon: impl AsRef<Path>) {
    embed(Some(icon.as_ref().to_path_buf()));
}

fn embed(custom_ico: Option<PathBuf>) {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return;
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let ico_path = out_dir.join("nana-app.ico");
    if let Some(custom) = custom_ico {
        println!("cargo:rerun-if-changed={}", custom.display());
        fs::copy(&custom, &ico_path).unwrap_or_else(|error| {
            panic!("failed to copy icon {}: {error}", custom.display());
        });
    } else {
        fs::write(&ico_path, crate::encode::ico().expect("default ico"))
            .expect("write default ico");
    }
    let rc_path = out_dir.join("nana-app.rc");
    let ico_literal = ico_path.display().to_string().replace('\\', "/");
    fs::write(&rc_path, format!("1 ICON \"{ico_literal}\"\r\n")).expect("write rc");
    let _ = embed_resource::compile(&rc_path, embed_resource::NONE);
}
