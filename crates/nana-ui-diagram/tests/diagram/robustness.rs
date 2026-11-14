//! Streaming and hostile input: every line prefix of every fixture and a
//! seeded set of byte mutations parse and lay out without panicking.

use crate::common;

use nana_ui_diagram::{DiagramError, DiagramMetrics, FixedMeasure, LimitKind, Limits, render};

fn run(source: &str) {
    let _ = render(
        source,
        &FixedMeasure,
        &DiagramMetrics::DEFAULT,
        &Limits::DEFAULT,
    );
}

#[test]
fn every_line_prefix_renders_or_fails_cleanly() {
    for (_, _, source) in common::fixtures() {
        let lines: Vec<&str> = source.lines().collect();
        for n in 0..=lines.len() {
            run(&lines[..n].join("\n"));
        }
        // Mid-line cuts, as a stream delivers them.
        for (i, _) in source.char_indices().step_by(7) {
            run(&source[..i]);
        }
    }
}

#[test]
fn byte_mutations_never_panic() {
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let alphabet = b"[](){}<>|&;:-=.~ox\"`@#\n AZaz09";
    for (_, _, source) in common::fixtures() {
        for _ in 0..40 {
            let mut bytes = source.clone().into_bytes();
            for _ in 0..(1 + next() % 4) {
                if bytes.is_empty() {
                    break;
                }
                let at = (next() as usize) % bytes.len();
                match next() % 3 {
                    0 => bytes[at] = alphabet[(next() as usize) % alphabet.len()],
                    1 => {
                        bytes.remove(at);
                    }
                    _ => bytes.insert(at, alphabet[(next() as usize) % alphabet.len()]),
                }
            }
            if let Ok(text) = String::from_utf8(bytes) {
                run(&text);
            }
        }
    }
}

#[test]
fn limits_end_runaway_sources() {
    let limits = Limits {
        nodes: 50,
        ..Limits::DEFAULT
    };
    let mut source = String::from("flowchart TD\n");
    for i in 0..60 {
        source.push_str(&format!("n{i} --> n{}\n", i + 1));
    }
    let err = render(&source, &FixedMeasure, &DiagramMetrics::DEFAULT, &limits).unwrap_err();
    assert_eq!(err, DiagramError::TooLarge(LimitKind::Nodes));
    let huge = "flowchart TD\n".to_string() + &"A-->B\n".repeat(30_000);
    assert_eq!(
        render(
            &huge,
            &FixedMeasure,
            &DiagramMetrics::DEFAULT,
            &Limits::DEFAULT
        )
        .unwrap_err(),
        DiagramError::TooLarge(LimitKind::SourceBytes)
    );
}
