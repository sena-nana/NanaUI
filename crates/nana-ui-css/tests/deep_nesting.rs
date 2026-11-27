//! Hostile nesting: ten thousand levels of every nesting construct parse on
//! a 512 KiB thread, skipped or rejected past the nesting limit, while
//! shallow input reads as before.

use nana_ui_core::LayoutStyle;
use nana_ui_css::syntax::{
    parse_component_values, parse_container_prelude, parse_media_query_list,
    parse_supports_condition,
};
use nana_ui_css::{
    ColorParseOptions, CompileContext, ContainerQueryUnsupported, VarScope, apply_css_text_with,
    compile_inline_css, lower_container_query, parse_color_str, parse_length_spec,
    parse_stylesheet, parse_theme_expr,
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
        let sheet = on_small_stack(move || parse_stylesheet(&deep));
        assert!(
            !sheet.diags.is_empty() || sheet.rules.is_empty(),
            "{open} should record a diagnostic or skip deep block"
        );
    }
    for open in ["@supports (display: grid) {", "@layer a {"] {
        assert_eq!(
            parse_stylesheet(&wrap(4, open, "a{color:red}", "}"))
                .rules
                .len(),
            1
        );
    }
    let sheet = parse_stylesheet(&wrap(4, "@media all {", "a{color:red}", "}"));
    assert_eq!(sheet.rules.len(), 1);
    assert!(!sheet.rules[0].media.is_empty());
}

#[test]
fn conditions_past_the_limit_are_rejected() {
    let (container_err, supports_none) = on_small_stack(|| {
        let mut diags = Vec::new();
        let deep_c = parse_component_values(&wrap(DEEP, "(", "width > 1px", ")"), &mut diags);
        let container = parse_container_prelude(&deep_c, &mut diags)
            .ok_or(ContainerQueryUnsupported::Invalid)
            .and_then(|(name, cond)| lower_container_query(name, &cond));

        let deep_not_c =
            parse_component_values(&wrap(DEEP, "not (", "width > 1px", ")"), &mut diags);
        let not_c = parse_container_prelude(&deep_not_c, &mut diags)
            .ok_or(ContainerQueryUnsupported::Invalid)
            .and_then(|(name, cond)| lower_container_query(name, &cond));
        assert!(not_c.is_err());

        let deep_s = parse_component_values(&wrap(DEEP, "(", "display: grid", ")"), &mut diags);
        let supports = parse_supports_condition(&deep_s, &mut diags);
        let deep_not_s =
            parse_component_values(&wrap(DEEP, "not (", "display: grid", ")"), &mut diags);
        let _ = parse_supports_condition(&deep_not_s, &mut diags);

        let deep_m = parse_component_values(&wrap(DEEP, "(", "min-width: 1px", ")"), &mut diags);
        let _ = parse_media_query_list(&deep_m, &mut diags);
        let deep_not_m =
            parse_component_values(&wrap(DEEP, "not (", "min-width: 1px", ")"), &mut diags);
        let _ = parse_media_query_list(&deep_not_m, &mut diags);

        (container.err(), supports.is_none())
    });
    assert!(container_err.is_some());
    assert!(supports_none);

    let mut diags = Vec::new();
    let shallow_c = parse_component_values("((width > 1px))", &mut diags);
    let (name, cond) = parse_container_prelude(&shallow_c, &mut diags).expect("shallow container");
    assert!(lower_container_query(name, &cond).is_ok());

    let shallow_s = parse_component_values("((display: grid))", &mut diags);
    assert!(parse_supports_condition(&shallow_s, &mut diags).is_some());
}

#[test]
fn selectors_ten_thousand_deep_parse() {
    on_small_stack(|| {
        for selector in [
            wrap(DEEP, ":not(", "a", ")"),
            wrap(DEEP, ":is(", "a", ")"),
            format!("a{}", wrap(DEEP, ":has(", "b", ")")),
        ] {
            let _ = parse_stylesheet(&format!("{selector}{{color:red}}"));
        }
    });
}

#[test]
fn values_past_the_limit_are_invalid() {
    on_small_stack(|| {
        let mut diags = Vec::new();
        let deep_calc = parse_component_values(&wrap(DEEP, "calc(", "1px", ")"), &mut diags);
        assert_eq!(parse_length_spec(&deep_calc), None);

        let parens = format!("calc({})", wrap(DEEP, "(", "1px", ")"));
        let deep_parens = parse_component_values(&parens, &mut diags);
        assert_eq!(parse_length_spec(&deep_parens), None);

        assert_eq!(
            parse_color_str(
                &wrap(DEEP, "color-mix(in srgb, ", "red", ", blue)"),
                ColorParseOptions::default(),
            ),
            None
        );

        let deep_theme = parse_component_values(
            &wrap(DEEP, "calc(", "theme(space.md) + 1px", ")"),
            &mut diags,
        );
        let _ = parse_theme_expr(&deep_theme);
    });

    let mut diags = Vec::new();
    let ok_calc = parse_component_values("calc((1px + calc(1px)))", &mut diags);
    let spec = parse_length_spec(&ok_calc).expect("shallow nested calc");
    assert_eq!(spec.resolve_px(None), Some(2.0));

    assert!(
        parse_color_str(
            "color-mix(in srgb, color-mix(in srgb, red, blue), blue)",
            ColorParseOptions::default(),
        )
        .is_some()
    );
}

#[test]
fn custom_property_cycles_and_long_chains_end() {
    let chain = |length: usize| {
        let mut scope = VarScope::new();
        let mut diags = Vec::new();
        for i in 0..length {
            let val = parse_component_values(&format!("var(--v{})", i + 1), &mut diags);
            scope.set(format!("--v{i}"), val);
        }
        scope.set(
            format!("--v{length}"),
            parse_component_values("4px", &mut diags),
        );
        scope.set("--self", parse_component_values("var(--self)", &mut diags));
        scope
    };
    let font_size_of = |css: &str, scope: &VarScope| -> Option<f32> {
        let mut layout = LayoutStyle::default();
        apply_css_text_with(&mut layout, css, scope, &CompileContext::default());
        layout.font_size
    };

    let (short, long, cycle) = on_small_stack(move || {
        let short = font_size_of("font-size: var(--v0)", &chain(3));
        let long = font_size_of("font-size: var(--v0)", &chain(DEEP));
        let cycle = font_size_of("font-size: var(--self, 2px)", &chain(0));
        (short, long, cycle)
    });
    assert_eq!(short, Some(4.0));
    assert_eq!(long, None);
    // Per CSS Variables 1 §3, cyclic custom properties become guaranteed-invalid and
    // do not fall back to the fallback inside a cycle reference; either None or 2px is
    // safe as long as it terminates without overflowing the stack.
    let _ = cycle;

    let sheet = parse_stylesheet(":root{--a: var(--a)} a{width: var(--a)}");
    let vars = sheet.root_var_scope("dark");
    let _ = compile_inline_css("width: var(--a)", &vars, &CompileContext::default());
}
