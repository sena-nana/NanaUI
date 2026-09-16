//! Platform system-font acceptance for the Issue #90 font layer.
//!
//! These read whatever fonts the machine has installed, so they are
//! `#[ignore]`d: they are acceptance evidence for a real desktop, not a gate a
//! headless CI image can meaningfully pass. Run them on the target platform:
//!
//! ```text
//! cargo test -p nana-text --test font_system_platform_acceptance -- --ignored --nocapture
//! ```
//!
//! Hermetic behaviour is covered by `font_system_is_hermetic_and_deterministic.rs`.

use nana_text::font::{
    FamilyList, FontChoiceReason, FontOrigin, FontQuery, FontSystem, LanguageTag,
};

#[test]
#[ignore = "reads the machine's installed fonts; run on the target platform"]
fn system_fonts_resolve_ui_text_cjk_and_emoji() {
    let started = std::time::Instant::now();
    let mut system = FontSystem::with_system_fonts();
    let scan = started.elapsed();
    let faces = system.face_count();
    assert!(faces > 0, "the platform scan found no fonts");
    println!("system faces: {faces} (scan {scan:?})");

    let query = FontQuery {
        families: FamilyList::parse_css("system-ui, sans-serif"),
        language: LanguageTag::new("zh-CN"),
        ..FontQuery::default()
    };
    let selection = system.select(&query);
    let primary = selection
        .primary
        .expect("the platform policy names an installed UI family");
    let described = system.describe(primary).unwrap();
    assert_eq!(described.origin, FontOrigin::System);
    println!(
        "primary: {:?} {}",
        described.families, described.post_script_name
    );

    let text = "Hello, 世界。こんにちは 한국어 🔥";
    let started = std::time::Instant::now();
    let assignments = system.resolve_text(&selection, text, None);
    println!(
        "resolve_text (cold: reads faces from disk): {:?}",
        started.elapsed()
    );
    let started = std::time::Instant::now();
    let warm = system.resolve_text(&selection, text, None);
    println!(
        "resolve_text (warm: coverage cached): {:?}",
        started.elapsed()
    );
    assert_eq!(warm, assignments, "a warm pass resolves identically");
    for assignment in &assignments {
        let family = assignment
            .font
            .and_then(|font| system.describe(font))
            .map(|face| face.families[0].to_string());
        println!(
            "  {:?} {:?} -> {family:?} ({:?})",
            &text[assignment.range.clone()],
            assignment.script.map(|script| script.as_str().to_string()),
            assignment.reason
        );
    }
    println!("{:#?}", system.counters());

    let missing: Vec<&str> = assignments
        .iter()
        .filter(|assignment| assignment.reason == FontChoiceReason::Missing)
        .map(|assignment| &text[assignment.range.clone()])
        .collect();
    assert!(
        missing.is_empty(),
        "a desktop platform policy should cover Latin, Han, kana, hangul and emoji; missing {missing:?}"
    );
}
