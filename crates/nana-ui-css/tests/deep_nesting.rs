//! Hostile nesting: ten thousand levels of every nesting construct parse on
//! a 512 KiB thread, skipped or rejected past the nesting limit, while
//! shallow input reads as before.

use std::collections::BTreeMap;

use nana_ui_css::css_map::with_active_css_vars;
use nana_ui_css::{
    ContainerQueryUnsupported, evaluate_supports_condition, parse_container_prelude,
    parse_css_color, parse_css_font_size, parse_css_length_px, parse_media_query_list,
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
        vars
    };
    let (short, long, cycle) = on_small_stack(move || {
        let short = with_active_css_vars(&chain(3), || parse_css_font_size("var(--v0)"));
        let long = with_active_css_vars(&chain(DEEP), || parse_css_font_size("var(--v0)"));
        let cycle = with_active_css_vars(&chain(0), || parse_css_font_size("var(--self, 2px)"));
        (short, long, cycle)
    });
    assert_eq!(short, Some(4.0));
    assert_eq!(long, None);
    assert_eq!(cycle, Some(2.0));
    let (_sheet, _report) = parse_stylesheet_full(":root{--a: var(--a)} a{width: var(--a)}", 0);
}

#[test]
fn deep_cascade_and_owned_match_tree_survive_small_stack() {
    use nana_ui_css::{
        MatchContext, MatchNode, MediaEnv, OwnedMatchTree, apply_stylesheet_to_layout,
        parse_stylesheet,
    };

    const CHAIN_DEPTH: usize = 600;
    std::thread::Builder::new()
        .stack_size(384 * 1024)
        .spawn(|| {
            let rules = parse_stylesheet(".root .branch > .leaf { gap: 12px; width: 100%; }", 0);
            let mut tree = OwnedMatchTree {
                tag: "div".into(),
                id: String::new(),
                classes: vec!["leaf".into()],
                attrs: BTreeMap::new(),
                children: Vec::new(),
            };
            for level in (0..CHAIN_DEPTH).rev() {
                let class = if level == 0 {
                    "root"
                } else if level == CHAIN_DEPTH - 1 {
                    "branch"
                } else {
                    "mid"
                };
                tree = OwnedMatchTree {
                    tag: "div".into(),
                    id: String::new(),
                    classes: vec![class.into()],
                    attrs: BTreeMap::new(),
                    children: vec![tree],
                };
            }
            let empty = BTreeMap::new();
            let branch_cls = [String::from("branch")];
            let mid_cls = [String::from("mid")];
            let root_cls = [String::from("root")];
            let mut ancestors = Vec::with_capacity(CHAIN_DEPTH);
            ancestors.push(MatchNode {
                tag: "div",
                id: "",
                classes: &branch_cls,
                attrs: &empty,
                is_empty: false,
                checked: false,
            });
            for _ in 1..CHAIN_DEPTH - 1 {
                ancestors.push(MatchNode {
                    tag: "div",
                    id: "",
                    classes: &mid_cls,
                    attrs: &empty,
                    is_empty: false,
                    checked: false,
                });
            }
            ancestors.push(MatchNode {
                tag: "div",
                id: "",
                classes: &root_cls,
                attrs: &empty,
                is_empty: false,
                checked: false,
            });
            let leaf_cls = [String::from("leaf")];
            let ctx = MatchContext {
                tag: "div",
                id: "",
                classes: &leaf_cls,
                attrs: &empty,
                ancestors: &ancestors,
                preceding_siblings: &[],
                sibling_index: 0,
                sibling_count: 1,
                of_type_index: 0,
                of_type_count: 1,
                has_bits: 0,
                has_args: &[],
                focus_within: false,
                is_empty: true,
                checked: false,
                media: MediaEnv::default(),
                children: &[],
                following_siblings: &[],
                all_siblings: &[],
                ancestor_subtrees: &[],
                owned_children: std::slice::from_ref(&tree),
                owned_following: &[],
                owned_ancestor_trees: &[],
                relative: None,
                relative_id: 0,
            };
            let mut layout = nana_ui_core::LayoutStyle::default();
            apply_stylesheet_to_layout(&mut layout, &rules, &ctx, None, None);
            assert_eq!(layout.gap, Some(nana_ui_core::LengthSpec::Px(12.0)));
            assert_eq!(layout.width, Some(nana_ui_core::LengthSpec::Fill));
            drop(tree);
        })
        .unwrap()
        .join()
        .unwrap();
}
