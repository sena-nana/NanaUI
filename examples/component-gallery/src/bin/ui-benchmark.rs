#[path = "ui_benchmark/report.rs"]
mod report;
#[path = "ui_benchmark/runner.rs"]
mod runner;

fn main() {
    let report = runner::run();
    let json = serde_json::to_string_pretty(&report).expect("benchmark report must serialize");
    let mut arguments = std::env::args_os().skip(1);
    match arguments.next() {
        None => println!("{json}"),
        Some(flag) if flag == "--output" => {
            let path = arguments
                .next()
                .expect("--output requires a destination path");
            assert!(
                arguments.next().is_none(),
                "unexpected arguments after --output destination"
            );
            let path = std::path::PathBuf::from(path);
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)
                    .expect("benchmark output directory must be writable");
            }
            std::fs::write(&path, format!("{json}\n"))
                .expect("benchmark report destination must be writable");
            println!("{}", path.display());
        }
        Some(argument) => panic!(
            "unsupported argument `{}`; expected --output <path>",
            argument.to_string_lossy()
        ),
    }
}
