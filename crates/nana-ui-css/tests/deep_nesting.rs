//! Hostile nesting: ten thousand levels of every nesting construct parse on
//! a 512 KiB thread, skipped or rejected past the nesting limit, while
//! shallow input reads as before.

use std::collections::BTreeMap;

use nana_ui_css::{
    ContainerQueryUnsupported, CssVarScope, evaluate_supports_condition, parse_container_prelude,
    parse_css_color, parse_css_font_size_in, parse_css_length_px, parse_media_query_list,
    parse_stylesheet, parse_stylesheet_full, parse_theme_expr,
};

const DEEP: usize = 10_000;

fn on_small_stack<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

fn wrap(depth: usize, open: &str, inner: &str, close: &str) -> String {
    format!("{}{inner}{}", open.repeat(depth), close.repeat(depth))
}

#[test]
fn at_rule_blocks_past_the_limit_are_skipped() {
    for open in [
        "@media all {",
        "@container (width > 1px) {",
        "@supports (display: grid) {",
        "@layer a {",
    ] {
        let deep = wrap(DEEP, open, "a{color:red}", "}");
        let (sheet, report) = on_small_stack(move || parse_stylesheet_full(&deep, 0));
        assert!(report.skipped_at_rules > 0, "{open}");
        drop(sheet);
    }
    for open in ["@supports (display: grid) {", "@layer a {"] {
        assert_eq!(
            parse_stylesheet(&wrap(4, open, "a{color:red}", "}"), 0).len(),
            1
        );
    }
    let (sheet, _) = parse_stylesheet_full(&wrap(4, "@media all {", "a{color:red}", "}"), 0);
    assert_eq!(sheet.media_rules.len(), 1);
}

#[test]
fn conditions_past_the_limit_are_rejected() {
    let (container, supports) = on_small_stack(|| {
        let container = parse_container_prelude(&wrap(DEEP, "(", "width > 1px", ")"));
        let not = parse_container_prelude(&wrap(DEEP, "not (", "width > 1px", ")"));
        assert!(not.is_err());
        let supports = evaluate_supports_condition(&wrap(DEEP, "(", "display: grid", ")"));
        assert_eq!(
            evaluate_supports_condition(&wrap(DEEP, "not (", "display: grid", ")")),
            None
        );
        let _ = parse_media_query_list(&wrap(DEEP, "(", "min-width: 1px", ")"));
        let _ = parse_media_query_list(&wrap(DEEP, "not (", "min-width: 1px", ")"));
        (container.err(), supports)
    });
    assert_eq!(container, Some(ContainerQueryUnsupported::Invalid));
    assert_eq!(supports, None);
    assert!(parse_container_prelude("((width > 1px))").is_ok());
    assert_eq!(evaluate_supports_condition("((display: grid))"), Some(true));
}

#[test]
fn selectors_ten_thousand_deep_parse() {
    on_small_stack(|| {
        for selector in [
            wrap(DEEP, ":not(", "a", ")"),
            wrap(DEEP, ":is(", "a", ")"),
            format!("a{}", wrap(DEEP, ":has(", "b", ")")),
        ] {
            let _ = parse_stylesheet(&format!("{selector}{{color:red}}"), 0);
        }
    });
}

#[test]
fn values_past_the_limit_are_invalid() {
    on_small_stack(|| {
        assert_eq!(
            parse_css_length_px(&wrap(DEEP, "calc(", "1px", ")"), None),
            None
        );
        let parens = format!("calc({})", wrap(DEEP, "(", "1px", ")"));
        assert_eq!(parse_css_length_px(&parens, None), None);
        assert_eq!(
            parse_css_color(&wrap(DEEP, "color-mix(in srgb, ", "red", ", blue)")),
            None
        );
        let _ = parse_theme_expr(&wrap(DEEP, "mix(", "accent", ", surface, 50%)"));
    });
    assert_eq!(
        parse_css_length_px("calc((1px + calc(1px)))", None),
        Some(2.0)
    );
    assert!(parse_css_color("color-mix(in srgb, color-mix(in srgb, red, blue), blue)").is_some());
}

#[test]
fn custom_property_cycles_and_long_chains_end() {
    let chain = |length: usize| {
        let mut vars: BTreeMap<String, String> = (0..length)
            .map(|i| (format!("--v{i}"), format!("var(--v{})", i + 1)))
            .collect();
        vars.insert(format!("--v{length}"), "4px".into());
        vars.insert("--self".into(), "var(--self)".into());
        CssVarScope::from_map(vars)
    };
    let (short, long, cycle) = on_small_stack(move || {
        let short = parse_css_font_size_in("var(--v0)", &chain(3));
        let long = parse_css_font_size_in("var(--v0)", &chain(DEEP));
        let cycle = parse_css_font_size_in("var(--self, 2px)", &chain(0));
        (short, long, cycle)
    });
    assert_eq!(short, Some(4.0));
    assert_eq!(long, None);
    assert_eq!(cycle, Some(2.0));
    let (_sheet, _report) = parse_stylesheet_full(":root{--a: var(--a)} a{width: var(--a)}", 0);
}
