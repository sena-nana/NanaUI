use std::path::PathBuf;

fn main() {
    // `.vue` files in `views/` become `$OUT_DIR/nana_views.rs`; a mistake in
    // one fails the build with its file, line and column.
    // Debug builds keep static text replaceable while running (hot reload).
    let debug = std::env::var("PROFILE").as_deref() == Ok("debug");
    if let Err(error) = nana_ui_sfc::Compiler::new("::nana_ui::runtime")
        .hot(debug)
        .build("views")
    {
        panic!("{error}");
    }
    bench_views();
    test_views();
}

/// `tests/views/` for the integration tests: what the app does not show
/// (localized text, locale scopes), compiled as its views are.
fn test_views() {
    println!("cargo:rerun-if-changed=tests/views");
    let mut sources = Vec::new();
    for entry in std::fs::read_dir("tests/views").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "vue") {
            println!("cargo:rerun-if-changed={}", path.display());
            let source = std::fs::read_to_string(&path).unwrap();
            sources.push((path.display().to_string(), source));
        }
    }
    sources.sort();
    let output = nana_ui_sfc::Compiler::new("::nana_ui::runtime")
        .compile(&sources)
        .unwrap_or_else(|error| panic!("{error}"));
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::write(out.join("nana_test_views.rs"), output.code).unwrap();
    for warning in output.warnings {
        println!("cargo:warning={warning}");
    }
}

/// `bench/` for `sfc-benchmark`: once as a release build compiles it and
/// once in hot mode, so the benchmark can compare the two in one binary.
fn bench_views() {
    println!("cargo:rerun-if-changed=bench");
    let mut files: Vec<PathBuf> = std::fs::read_dir("bench")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "vue"))
        .collect();
    files.sort();
    let sources: Vec<(String, String)> = files
        .iter()
        .map(|path| {
            println!("cargo:rerun-if-changed={}", path.display());
            let source = std::fs::read_to_string(path).unwrap();
            (path.display().to_string(), source)
        })
        .collect();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    for (hot, file) in [
        (false, "nana_bench_views.rs"),
        (true, "nana_bench_views_hot.rs"),
    ] {
        let output = nana_ui_sfc::Compiler::new("::nana_ui::runtime")
            .hot(hot)
            .compile(&sources)
            .unwrap_or_else(|error| panic!("{error}"));
        std::fs::write(out.join(file), output.code).unwrap();
        if !hot {
            for warning in output.warnings {
                println!("cargo:warning={warning}");
            }
        }
    }
}
