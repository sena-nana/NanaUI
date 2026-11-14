//! Display-list goldens: every fixture laid out with the fixed
//! measure, dumped as text. Re-bless with `NANA_DIAGRAM_BLESS=1`.

use crate::common;

use nana_ui_diagram::{DiagramMetrics, FixedMeasure, Limits, dump, render};

#[test]
fn display_lists_match_their_goldens() {
    let bless = std::env::var_os("NANA_DIAGRAM_BLESS").is_some();
    let mut failures = Vec::new();
    let fixtures = common::fixtures();
    assert!(!fixtures.is_empty());
    for (family, name, source) in fixtures {
        let scene = render(
            &source,
            &FixedMeasure,
            &DiagramMetrics::DEFAULT,
            &Limits::DEFAULT,
        )
        .unwrap_or_else(|e| panic!("{family}/{name}: {e}"));
        let text = dump(&scene);
        let path = common::root()
            .join("golden")
            .join(&family)
            .join(format!("{name}.txt"));
        if bless {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &text).unwrap();
            continue;
        }
        // A Windows checkout may hand the golden back with CRLF endings.
        match std::fs::read_to_string(&path) {
            Ok(expected) if expected.replace("\r\n", "\n") == text => {}
            Ok(_) => failures.push(format!("{family}/{name}: differs from {}", path.display())),
            Err(_) => failures.push(format!("{family}/{name}: no golden at {}", path.display())),
        }
    }
    assert!(
        failures.is_empty(),
        "{}\nre-bless with NANA_DIAGRAM_BLESS=1",
        failures.join("\n")
    );
}
