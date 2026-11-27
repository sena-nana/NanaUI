use std::cmp::Ordering;
use std::sync::Arc;

use nana_ui_core::theme::SpacingStep;
use nana_ui_core::{
    BackgroundPosition, CalcExpr, LengthSpec, PaintColor, SemanticColorRole, ThemeToken,
    ViewportAxis,
};

use super::*;

fn cvs(src: &str) -> Vec<ComponentValue> {
    let mut diags = Vec::new();
    parse_component_values(src, &mut diags)
}

// ============================================================================
// 1. Span & Diagnostics
// ============================================================================

#[test]
fn span_line_col_handles_crlf_and_utf8() {
    let src = "/* 中文 */\r\n.按钮 { color: red; }";
    let btn_offset = src.find(".按钮").unwrap();
    let span = Span::new(btn_offset, btn_offset + ".按钮".len());
    assert_eq!(span.slice(src), ".按钮");
    assert_eq!(span.line_col(src), (2, 1));

    let color_offset = src.find("color").unwrap();
    let color_span = Span::new(color_offset, color_offset + 5);
    // `.`(1) `按`(2) `钮`(3) ` `(4) `{`(5) ` `(6) `c`(7)
    assert_eq!(color_span.line_col(src), (2, 7));
}

#[test]
fn diag_tiers_vendor_prefix_warning_vs_unknown_error() {
    let d1 = diagnose_unknown_property("-moz-appearance", Span::new(0, 15));
    assert_eq!(d1.severity, DiagSeverity::Warning);
    assert_eq!(d1.code, DiagCode::VendorPrefixedProperty);

    let d2 = diagnose_unknown_property("-webkit-font-smoothing", Span::new(0, 22));
    assert_eq!(d2.severity, DiagSeverity::Warning);

    // Supported -webkit-* properties are NOT ignorable vendor prefixes.
    assert!(!is_ignorable_vendor_property("-webkit-backdrop-filter"));
    assert!(!is_ignorable_vendor_property("-webkit-line-clamp"));

    let d3 = diagnose_unknown_property("not-a-real-prop", Span::new(0, 15));
    assert_eq!(d3.severity, DiagSeverity::Error);
    assert_eq!(d3.code, DiagCode::UnknownProperty);
}

// ============================================================================
// 2. Tokenizer (W3C CSS Syntax Level 3)
// ============================================================================

#[test]
fn tokenizer_utf8_comments_escapes_numbers_and_urls() {
    let src = r#"
        /* comment with 中文 */
        .按钮[data-label="中文 /* not a comment */"] {
            opacity: 1e-2;
            width: +2.5E+1px;
            content: "}";
            background: url(data:image/svg+xml;base64,PHN2Zz4=);
            --esc-\31 23: "\1F600";
        }
    "#;
    let mut diags = Vec::new();
    let toks = tokenize(src, &mut diags);
    assert!(diags.is_empty(), "unexpected diags: {diags:?}");

    assert!(toks.iter().any(|t| t.token == Token::Ident("按钮".into())));
    assert!(
        toks.iter()
            .any(|t| t.token == Token::String("中文 /* not a comment */".into()))
    );
    assert!(toks.iter().any(|t| matches!(
        &t.token,
        Token::Number { value, int: None, .. } if (*value - 0.01).abs() < 1e-9
    )));
    assert!(toks.iter().any(|t| matches!(
        &t.token,
        Token::Dimension { value, unit } if (*value - 25.0).abs() < 1e-9 && unit == "px"
    )));
    assert!(toks.iter().any(|t| t.token == Token::String("}".into())));
    assert!(
        toks.iter()
            .any(|t| t.token == Token::Url("data:image/svg+xml;base64,PHN2Zz4=".into()))
    );
    assert!(
        toks.iter()
            .any(|t| t.token == Token::Ident("--esc-123".into()))
    );
    assert!(
        toks.iter()
            .any(|t| t.token == Token::String("\u{1F600}".into()))
    );
}

#[test]
fn tokenizer_quoted_url_and_bad_tokens() {
    let mut diags = Vec::new();
    let quoted = parse_component_values(r#"url("data:image/svg+xml;base64,PHN2Zz4=")"#, &mut diags);
    assert!(diags.is_empty());
    assert_eq!(quoted.len(), 1);
    assert_eq!(
        quoted[0].as_url(),
        Some("data:image/svg+xml;base64,PHN2Zz4=")
    );

    let mut bad_diags = Vec::new();
    let bad_toks = tokenize("url(foo bar) \"unterminated\nnext", &mut bad_diags);
    assert!(bad_toks.iter().any(|t| t.token == Token::BadUrl));
    assert!(bad_toks.iter().any(|t| t.token == Token::BadString));
    assert!(
        bad_toks
            .iter()
            .any(|t| t.token == Token::Ident("next".into()))
    );
    assert!(!bad_diags.is_empty());
}

// ============================================================================
// 3. Parser, Error Recovery, CSS Nesting, Helpers & Stack Safety
// ============================================================================

#[test]
fn parser_content_rbrace_and_declaration_error_recovery() {
    let css = r#"
        .a {
            content: "}";
            broken_without_colon;
            123invalid: foo;
            color: red !IMPORTANT;
            padding: 10px calc(10px + 2px);
        }
        .b { display: flex; }
    "#;
    let sheet = parse_stylesheet(css);
    assert_eq!(sheet.rules.len(), 2);

    let Rule::Qualified(rule_a) = &sheet.rules[0] else {
        panic!("expected qualified rule .a");
    };
    let mut diags = Vec::new();
    let decls = parse_declarations_from_values(&rule_a.block, &mut diags);
    // Two malformed declarations produced errors, but the 3 valid ones survived intact.
    assert_eq!(diags.len(), 2);
    assert!(diags.iter().all(|d| d.code == DiagCode::InvalidDeclaration));
    assert_eq!(decls.len(), 3);
    assert_eq!(decls[0].name, "content");
    assert_eq!(
        decls[0].value[0].as_token(),
        Some(&Token::String("}".into()))
    );
    assert_eq!(decls[1].name, "color");
    assert!(decls[1].important);
    assert_eq!(decls[2].name, "padding");

    // `split_whitespace_items` keeps `calc(10px + 2px)` as ONE item!
    let pad_items = split_whitespace_items(&decls[2].value);
    assert_eq!(pad_items.len(), 2);
    assert!(matches!(
        pad_items[1],
        [ComponentValue::Function { name, .. }] if name == "calc"
    ));

    // Second rule `.b` was never poisoned by `content: "}"`!
    let Rule::Qualified(rule_b) = &sheet.rules[1] else {
        panic!("expected qualified rule .b");
    };
    let mut diags_b = Vec::new();
    let decls_b = parse_declarations_from_values(&rule_b.block, &mut diags_b);
    assert!(diags_b.is_empty());
    assert_eq!(decls_b.len(), 1);
    assert_eq!(decls_b[0].name, "display");
}

#[test]
fn parser_css_nesting_style_block() {
    let css = r#"
        .card {
            padding: 8px;
            &:hover { opacity: 0.9; }
            @media (min-width: 600px) {
                .title { font-size: 20px; }
            }
            color: blue;
        }
    "#;
    let sheet = parse_stylesheet(css);
    assert!(sheet.diags.is_empty());
    let Rule::Qualified(q) = &sheet.rules[0] else {
        panic!("expected qualified rule");
    };
    let mut diags = Vec::new();
    let items = parse_style_block(&q.block, &mut diags);
    assert!(diags.is_empty());
    assert_eq!(items.len(), 4);
    assert!(matches!(items[0], StyleBlockItem::Declaration(ref d) if d.name == "padding"));
    assert!(matches!(
        items[1],
        StyleBlockItem::NestedRule(Rule::Qualified(_))
    ));
    assert!(matches!(items[2], StyleBlockItem::NestedRule(Rule::At(ref a)) if a.name == "media"));
    assert!(matches!(items[3], StyleBlockItem::Declaration(ref d) if d.name == "color"));
}

#[test]
fn parser_helpers_comma_whitespace_eof() {
    let vals = cvs("  rgb(1, 2, 3), calc(10px + 2px) 20px , red  ");
    let groups = split_comma_groups(&vals);
    assert_eq!(groups.len(), 3);
    assert_eq!(split_whitespace_items(groups[0]).len(), 1);
    assert_eq!(split_whitespace_items(groups[1]).len(), 2);
    assert_eq!(split_whitespace_items(groups[2]).len(), 1);
    assert!(expect_eof(cvs("   ").as_slice()).is_ok());
    assert!(expect_eof(&vals).is_err());
}

#[test]
fn parser_deep_nesting_stack_safety() {
    // 2,000 deep parens, curlies, and calc() — must never overflow stack and must report NestingLimitExceeded.
    for (open, close) in [("(", ")"), ("{", "}"), ("[", "]"), ("calc(", ")")] {
        let deep = format!("{}{}{}", open.repeat(2000), "1px", close.repeat(2000));
        let mut diags = Vec::new();
        let _ = parse_component_values(&deep, &mut diags);
        assert!(
            diags
                .iter()
                .any(|d| d.code == DiagCode::NestingLimitExceeded
                    && d.severity == DiagSeverity::Error),
            "expected NestingLimitExceeded for {open}...{close}"
        );
    }
}

// ============================================================================
// 4. Token-Stream Vars & Cycle / Depth Safety
// ============================================================================

#[test]
fn vars_nested_fallbacks_and_preserve_nana_tokens() {
    let mut root = VarScope::new();
    root.set("--b", cvs("100px"));
    root.set("--alias-theme", cvs("var(--nana-accent)"));
    root.set("--fb-color", cvs("#112233"));

    let mut child = VarScope::with_parent(Arc::new(root));
    child.set("--local", cvs("12px"));

    // 1. Nested fallback when outer var is missing works with BOTH preserve_nana_tokens = true and false.
    for preserve in [false, true] {
        let mut diags = Vec::new();
        let expanded = expand_vars(
            &cvs("calc(var(--missing, var(--b, 50px)) + var(--local))"),
            &child,
            preserve,
            &mut diags,
        )
        .expect("nested var expansion must succeed");
        assert!(diags.is_empty());
        let len = parse_length_spec(&expanded).expect("expanded calc must parse as LengthSpec");
        assert_eq!(len.resolve_px(None), Some(112.0));
    }

    // 2. preserve_nana_tokens = true preserves `--nana-*` while still expanding nested fallback vars!
    let mut diags = Vec::new();
    let preserved = expand_vars(
        &cvs("var(--nana-accent, var(--fb-color, #000))"),
        &child,
        true,
        &mut diags,
    )
    .expect("nana token preserved");
    assert!(diags.is_empty());
    assert_eq!(
        serialize_component_values(&preserved),
        "var(--nana-accent, #112233)"
    );

    // 3. Ordinary custom property pointing to `--nana-*` preserves the `--nana-*` token!
    let via_alias = expand_vars(&cvs("var(--alias-theme)"), &child, true, &mut diags).unwrap();
    assert_eq!(serialize_component_values(&via_alias), "var(--nana-accent)");

    // 4. preserve_nana_tokens = false resolves fallback for undefined `--nana-*`.
    let resolved_fb = expand_vars(
        &cvs("var(--nana-accent, var(--fb-color))"),
        &child,
        false,
        &mut diags,
    )
    .unwrap();
    assert_eq!(serialize_component_values(&resolved_fb), "#112233");
}

#[test]
fn vars_cascade_specificity_layer_and_important() {
    let mut reg = LayerRegistry::new();
    let base = reg.declare(&LayerName(vec!["base".into()]));
    let theme = reg.declare(&LayerName(vec!["theme".into()]));

    let mut scope = VarScope::new();
    // Normal: theme (later layer) beats base.
    scope.set_with_cascade(
        "--c",
        cvs("red"),
        VarCascadeKey {
            important: false,
            layer: base,
            specificity: Specificity::ID,
            source_order: 1,
        },
    );
    scope.set_with_cascade(
        "--c",
        cvs("blue"),
        VarCascadeKey {
            important: false,
            layer: theme,
            specificity: Specificity::TYPE,
            source_order: 2,
        },
    );
    assert_eq!(
        serialize_component_values(scope.get("--c").unwrap()),
        "blue"
    );

    // Normal unlayered beats layered.
    scope.set_with_cascade(
        "--c",
        cvs("green"),
        VarCascadeKey {
            important: false,
            layer: LayerOrder::UNLAYERED,
            specificity: Specificity::ZERO,
            source_order: 3,
        },
    );
    assert_eq!(
        serialize_component_values(scope.get("--c").unwrap()),
        "green"
    );

    // !important in earlier layer `base` beats !important unlayered AND !important in `theme`!
    scope.set_with_cascade(
        "--c",
        cvs("yellow"),
        VarCascadeKey {
            important: true,
            layer: LayerOrder::UNLAYERED,
            specificity: Specificity::ID,
            source_order: 4,
        },
    );
    scope.set_with_cascade(
        "--c",
        cvs("purple"),
        VarCascadeKey {
            important: true,
            layer: base,
            specificity: Specificity::TYPE,
            source_order: 5,
        },
    );
    scope.set_with_cascade(
        "--c",
        cvs("orange"),
        VarCascadeKey {
            important: true,
            layer: theme,
            specificity: Specificity::ID,
            source_order: 6,
        },
    );
    assert_eq!(
        serialize_component_values(scope.get("--c").unwrap()),
        "purple"
    );
}

#[test]
fn vars_cycle_and_depth_detection() {
    let mut scope = VarScope::new();
    scope.set("--a", cvs("var(--b)"));
    scope.set("--b", cvs("var(--a)"));
    let mut diags = Vec::new();
    let err = expand_vars(&cvs("var(--a)"), &scope, true, &mut diags).unwrap_err();
    assert!(matches!(err, VarExpandError::Cycle { ref name, .. } if name == "--a"));
    assert!(diags.iter().any(|d| d.code == DiagCode::VarCycle));

    // Deep non-cyclic chain exceeding MAX_NESTING.
    let mut deep_scope = VarScope::new();
    for i in 0..80 {
        deep_scope.set(format!("--v{i}"), cvs(&format!("var(--v{})", i + 1)));
    }
    deep_scope.set("--v80", cvs("1px"));
    let mut diags2 = Vec::new();
    let err2 = expand_vars(&cvs("var(--v0)"), &deep_scope, false, &mut diags2).unwrap_err();
    assert!(matches!(err2, VarExpandError::DepthExceeded { .. }));
    assert!(diags2.iter().any(|d| d.code == DiagCode::VarDepthExceeded));
}

// ============================================================================
// 5. Selectors & Specificity
// ============================================================================

#[test]
fn selector_utf8_quoted_attrs_combinators_and_pseudos() {
    let mut diags = Vec::new();
    let list = parse_selector_list(
        &cvs(
            r#".按钮[href="https://example.com/a:b"][data-x="a*=b" i] > span + em ~ strong:hover::before"#,
        ),
        &mut diags,
    )
    .expect("selector list should parse");
    assert!(diags.is_empty());
    assert_eq!(list.0.len(), 1);
    let sel = &list.0[0];
    assert!(sel.has_sibling_combinator());
    assert_eq!(sel.head.classes, vec!["按钮"]);
    assert_eq!(sel.head.attrs.len(), 2);
    assert_eq!(sel.head.attrs[0].name, "href");
    assert_eq!(sel.head.attrs[0].op, Some(AttrOperator::Equals));
    assert_eq!(
        sel.head.attrs[0].value.as_deref(),
        Some("https://example.com/a:b")
    );
    assert_eq!(sel.head.attrs[1].name, "data-x");
    assert_eq!(sel.head.attrs[1].op, Some(AttrOperator::Equals));
    assert_eq!(sel.head.attrs[1].value.as_deref(), Some("a*=b"));
    assert_eq!(
        sel.head.attrs[1].case_sensitivity,
        Some(AttrCase::Insensitive)
    );

    assert_eq!(sel.rest.len(), 3);
    assert_eq!(sel.rest[0].0, Combinator::Child);
    assert_eq!(sel.rest[1].0, Combinator::NextSibling);
    assert_eq!(sel.rest[2].0, Combinator::SubsequentSibling);
    assert_eq!(sel.subject().pseudos, vec![PseudoClass::Hover]);
    assert_eq!(sel.subject().pseudo_element, Some(PseudoElement::Before));
}

#[test]
fn selector_multiple_is_where_not_preserved_as_and_with_exact_specificity() {
    let mut diags = Vec::new();
    let list = parse_selector_list(
        &cvs("button:is(.a, #main):is(.c, .d):where(#ignored, .zero):not(.disabled, .hidden)"),
        &mut diags,
    )
    .expect("selector should parse");
    assert!(diags.is_empty());
    let comp = &list.0[0].head;
    // Four distinct functional pseudo-classes kept separate (AND composition, NEVER flattened!).
    assert_eq!(comp.pseudos.len(), 4);
    assert!(matches!(comp.pseudos[0], PseudoClass::Is(ref l) if l.0.len() == 2));
    assert!(matches!(comp.pseudos[1], PseudoClass::Is(ref l) if l.0.len() == 2));
    assert!(matches!(comp.pseudos[2], PseudoClass::Where(ref l) if l.0.len() == 2));
    assert!(matches!(comp.pseudos[3], PseudoClass::Not(ref l) if l.0.len() == 2));

    // Specificity:
    // button (0,0,1)
    // + :is(.a, #main) -> max((0,1,0), (1,0,0)) = (1,0,0)
    // + :is(.c, .d) -> (0,1,0)
    // + :where(#ignored, .zero) -> (0,0,0)
    // + :not(.disabled, .hidden) -> (0,1,0)
    // Total = (1, 2, 1)
    assert_eq!(list.0[0].specificity(), Specificity::new(1, 2, 1));
}

#[test]
fn selector_nth_child_and_pseudo_elements() {
    let mut diags = Vec::new();
    let s1 = parse_selector_list(&cvs("li:nth-child(2n + 1)"), &mut diags).unwrap();
    let PseudoClass::NthChild(ab) = s1.0[0].head.pseudos[0] else {
        panic!("expected NthChild");
    };
    assert_eq!(ab, AnPlusB { a: 2, b: 1 });
    assert!(ab.matches(1));
    assert!(!ab.matches(2));
    assert!(ab.matches(5));

    let s2 = parse_selector_list(&cvs("li:nth-child(-n+3)"), &mut diags).unwrap();
    let PseudoClass::NthChild(ab2) = s2.0[0].head.pseudos[0] else {
        panic!("expected NthChild");
    };
    assert!(ab2.matches(1) && ab2.matches(3) && !ab2.matches(4));

    for (raw, expected) in [
        ("input::placeholder", PseudoElement::Placeholder),
        ("p::selection", PseudoElement::Selection),
        ("div::-webkit-scrollbar", PseudoElement::WebkitScrollbar),
        (
            "div::-webkit-scrollbar-thumb",
            PseudoElement::WebkitScrollbarThumb,
        ),
        (
            "div::-webkit-scrollbar-track",
            PseudoElement::WebkitScrollbarTrack,
        ),
        ("div::after", PseudoElement::After),
    ] {
        let parsed = parse_selector_list(&cvs(raw), &mut diags).unwrap();
        assert_eq!(parsed.0[0].head.pseudo_element, Some(expected));
    }
}

// ============================================================================
// 6. At-Rules (@layer, @keyframes, @container, @media, @supports, @font-face)
// ============================================================================

#[test]
fn at_rule_layer_ordering_and_important_reversal() {
    let sheet = parse_stylesheet(
        r#"
        @layer reset, base, components;
        @layer base.sub { .a { color: red; } }
        @layer { .b { color: blue; } }
        "#,
    );
    assert!(sheet.diags.is_empty());
    let mut reg = LayerRegistry::new();
    let mut diags = Vec::new();

    let Rule::At(stmt) = &sheet.rules[0] else {
        panic!();
    };
    let LayerPrelude::Statement(names) = parse_layer_prelude(stmt, &mut diags).unwrap() else {
        panic!();
    };
    assert_eq!(names.len(), 3);
    let r = reg.declare(&names[0]);
    let b = reg.declare(&names[1]);
    let c = reg.declare(&names[2]);
    // Re-declaring `reset` is idempotent.
    assert_eq!(reg.declare(&names[0]), r);
    assert!(r < b && b < c && c < LayerOrder::UNLAYERED);

    let Rule::At(blk1) = &sheet.rules[1] else {
        panic!();
    };
    let LayerPrelude::Block(Some(sub)) = parse_layer_prelude(blk1, &mut diags).unwrap() else {
        panic!();
    };
    assert_eq!(sub.full_name(), "base.sub");

    let Rule::At(blk2) = &sheet.rules[2] else {
        panic!();
    };
    assert_eq!(
        parse_layer_prelude(blk2, &mut diags),
        Some(LayerPrelude::Block(None))
    );

    // Normal cascade: reset < base < components < unlayered
    assert_eq!(
        LayerRegistry::compare_cascade(false, r, false, b),
        Ordering::Less
    );
    assert_eq!(
        LayerRegistry::compare_cascade(false, c, false, LayerOrder::UNLAYERED),
        Ordering::Less
    );

    // !important cascade: unlayered < components < base < reset (REVERSED!)
    assert_eq!(
        LayerRegistry::compare_cascade(true, r, true, b),
        Ordering::Greater
    );
    assert_eq!(
        LayerRegistry::compare_cascade(true, c, true, LayerOrder::UNLAYERED),
        Ordering::Greater
    );
    assert_eq!(
        LayerRegistry::compare_cascade(true, LayerOrder::UNLAYERED, false, LayerOrder::UNLAYERED),
        Ordering::Greater
    );
}

#[test]
fn at_rule_keyframes_container_media_supports_font_face() {
    let sheet = parse_stylesheet(
        r#"
        @keyframes fade {
            from, 25% { opacity: 0; }
            to { opacity: 1; }
        }
        @container card (300px < inline-size <= 600px) and (min-height: 200px) {
            .inner { display: grid; }
        }
        @media not (unknown-feature), (width >= 600px) and (color) {
            .wide { width: 100%; }
        }
        @supports ((display: block) or (display: grid)) and (not (foo: bar)) {
            .ok { display: block; }
        }
        @font-face {
            font-family: "Inter Variable";
            src: local("Inter"), url(fonts/inter.woff2) format("woff2");
            font-weight: 100 900;
            font-style: oblique -10deg 20deg;
            font-stretch: condensed 125%;
            font-display: swap;
            unicode-range: U+0025-00FF, U+4??, U+0026;
        }
        "#,
    );
    assert!(sheet.diags.is_empty(), "diags: {:?}", sheet.diags);
    let mut diags = Vec::new();

    // @keyframes
    let Rule::At(kf_at) = &sheet.rules[0] else {
        panic!();
    };
    let kf = parse_keyframes_rule(kf_at, &mut diags).unwrap();
    assert_eq!(kf.name, "fade");
    assert_eq!(kf.blocks.len(), 2);
    assert_eq!(
        kf.blocks[0].stops,
        vec![KeyframeStop::From, KeyframeStop::Percentage(25.0)]
    );

    // @container
    let Rule::At(ct_at) = &sheet.rules[1] else {
        panic!();
    };
    let ct = parse_container_rule(ct_at, &mut diags).unwrap();
    assert_eq!(ct.name.as_deref(), Some("card"));
    let ContainerCondition::And(parts) = &ct.condition else {
        panic!("expected And condition, got {:?}", ct.condition);
    };
    assert_eq!(parts.len(), 2);
    assert!(matches!(
        &parts[0],
        ContainerCondition::Feature(ContainerSizeQuery::Range {
            feature: ContainerFeatureName::InlineSize,
            bounds,
        }) if bounds.len() == 2
            && bounds[0].0 == CompareOp::Gt
            && bounds[1].0 == CompareOp::Le
    ));

    // @media with Kleene 3-valued logic
    let Rule::At(mq_at) = &sheet.rules[2] else {
        panic!();
    };
    let mql = parse_media_query_list(&mq_at.prelude, &mut diags);
    assert_eq!(mql.0.len(), 2);

    // Test `@media not (unknown)` specifically -> Kleene3::Unknown -> does NOT match!
    let not_unknown = parse_media_query_list(&cvs("not (unknown-feature)"), &mut diags);
    let eval_feat = |f: &MediaFeature| match f {
        MediaFeature::Plain { name, .. } if name == "color" => Kleene3::True,
        MediaFeature::Range { name, bounds } if name == "width" => {
            let ok = bounds.iter().all(|(op, val)| {
                parse_length_spec(val)
                    .and_then(|l| l.resolve_px(None))
                    .is_some_and(|rhs| op.eval_f32(800.0, rhs))
            });
            Kleene3::from_bool(ok)
        }
        _ => Kleene3::Unknown,
    };
    assert!(
        !not_unknown.matches(&|_| true, &eval_feat),
        "Kleene3: `not (unknown)` must evaluate to Unknown (false), NEVER true!"
    );
    // `(unknown) or (color)` -> Unknown OR True == True!
    let unknown_or_true = parse_media_query_list(&cvs("(unknown-feature) or (color)"), &mut diags);
    assert!(unknown_or_true.matches(&|_| true, &eval_feat));
    // Full query list matches because second query `(width >= 600px) and (color)` is True.
    assert!(mql.matches(&|_| true, &eval_feat));

    // @supports with `or` and `display: block`
    let Rule::At(sup_at) = &sheet.rules[3] else {
        panic!();
    };
    let sup = parse_supports_condition(&sup_at.prelude, &mut diags).unwrap();
    let matched = sup.evaluate(
        &|prop, val| {
            prop == "display"
                && val
                    .first()
                    .and_then(ComponentValue::as_ident)
                    .is_some_and(|v| v == "block")
        },
        &|_| false,
    );
    assert!(matched);

    // @font-face
    let Rule::At(ff_at) = &sheet.rules[4] else {
        panic!();
    };
    let ff = parse_font_face_rule(ff_at, &mut diags).unwrap();
    assert!(diags.is_empty(), "diags: {diags:?}");
    assert_eq!(ff.font_family.as_deref(), Some("Inter Variable"));
    assert_eq!(ff.src.len(), 2);
    assert_eq!(ff.src[0].kind, FontFaceSrcKind::Local);
    assert_eq!(ff.src[1].kind, FontFaceSrcKind::Url);
    assert_eq!(ff.src[1].value, "fonts/inter.woff2");
    assert_eq!(ff.src[1].format.as_deref(), Some("woff2"));
    assert_eq!(ff.font_weight, Some(FontWeightRange { min: 100, max: 900 }));
    assert_eq!(
        ff.font_style,
        Some(FontFaceStyle::Oblique {
            min_deg: Some(-10.0),
            max_deg: Some(20.0),
        })
    );
    assert_eq!(
        ff.font_stretch,
        Some(FontStretchRange {
            min_percent: 75.0,
            max_percent: 125.0,
        })
    );
    assert_eq!(ff.font_display, Some(FontDisplay::Swap));
    assert_eq!(
        ff.unicode_range,
        vec![
            UnicodeRange {
                start: 0x0025,
                end: 0x00FF
            },
            UnicodeRange {
                start: 0x400,
                end: 0x4FF
            },
            UnicodeRange {
                start: 0x0026,
                end: 0x0026
            },
        ]
    );
}

// ============================================================================
// 7. Unified Value Parsers (Angles, Negative Lengths, Calc, ThemeExpr, Colors,
//    Positions, Gradient Stop Fixup, Stack Safety)
// ============================================================================

#[test]
fn values_angles_and_negative_lengths() {
    assert!((parse_angle_deg(&cvs("90deg")).unwrap() - 90.0).abs() < 1e-4);
    assert!((parse_angle_deg(&cvs("0.5turn")).unwrap() - 180.0).abs() < 1e-4);
    assert!((parse_angle_deg(&cvs("100grad")).unwrap() - 90.0).abs() < 1e-4);
    assert!((parse_angle_deg(&cvs("3.14159265rad")).unwrap() - 180.0).abs() < 1e-3);

    // Negative lengths & percentages MUST be preserved without clamping to 0!
    assert_eq!(
        parse_length_spec(&cvs("-50%")),
        Some(LengthSpec::Percent(-50.0))
    );
    assert_eq!(
        parse_length_spec(&cvs("-20vh")),
        Some(LengthSpec::Viewport {
            axis: ViewportAxis::Height,
            value: -20.0,
        })
    );
    assert_eq!(
        parse_length_spec(&cvs("-10vw")),
        Some(LengthSpec::Viewport {
            axis: ViewportAxis::Width,
            value: -10.0,
        })
    );
    assert_eq!(
        parse_length_spec(&cvs("-1.5em")),
        Some(LengthSpec::Em(-1.5))
    );
    assert_eq!(
        parse_length_spec(&cvs("-2rem")),
        Some(LengthSpec::Rem(-2.0))
    );
    assert_eq!(parse_length_spec(&cvs("-8px")), Some(LengthSpec::Px(-8.0)));
}

#[test]
fn values_calc_theme_expr_and_stack_safety() {
    // calc / min / max / clamp
    let calc = parse_calc_expr(&cvs("calc(100% - min(20px, 2em) * 2)")).unwrap();
    assert!(matches!(calc, CalcExpr::Binary { .. }));
    let clamp_len = parse_length_spec(&cvs("clamp(10px, 50%, 100px)")).unwrap();
    assert_eq!(clamp_len.resolve_px(Some(120.0)), Some(60.0));
    assert_eq!(clamp_len.resolve_px(Some(10.0)), Some(10.0));
    assert_eq!(clamp_len.resolve_px(Some(400.0)), Some(100.0));

    // ThemeExpr supports BOTH `theme(...)` and `var(--nana-*)`!
    let te1 = parse_theme_expr(&cvs("calc(theme(space-md) * 2 + 4px)")).unwrap();
    assert_eq!(te1.terms, vec![(ThemeToken::Space(SpacingStep::Md), 2.0)]);
    assert_eq!(te1.constant, 4.0);

    let te2 = parse_theme_expr(&cvs("calc(var(--nana-space-lg) - 6px)")).unwrap();
    assert_eq!(te2.terms, vec![(ThemeToken::Space(SpacingStep::Lg), 1.0)]);
    assert_eq!(te2.constant, -6.0);

    // Stack safety: 5,000 unary `-` and `+` signs, deep `calc(`, and deep `(`
    let unary_spam = format!("calc({}1px)", "- ".repeat(5000));
    assert!(parse_calc_expr(&cvs(&unary_spam)).is_none());

    let theme_unary_spam = format!("{}{}", "+ ".repeat(5000), "theme(space-md)");
    assert!(parse_theme_expr(&cvs(&theme_unary_spam)).is_none());

    let deep_parens = format!("calc({}1px{})", "(".repeat(200), ")".repeat(200));
    assert!(parse_calc_expr(&cvs(&deep_parens)).is_none());

    let deep_theme_calc = format!("{}theme(space-md){}", "calc(".repeat(200), ")".repeat(200));
    assert!(parse_theme_expr(&cvs(&deep_theme_calc)).is_none());
}

#[test]
fn values_colors_reject_empty_and_parse_all_formats() {
    let opts = ColorParseOptions::default();

    // Empty & whitespace input MUST be rejected (not turned into transparent!).
    assert_eq!(parse_color_str("", opts), None);
    assert_eq!(parse_color_str("   ", opts), None);
    assert_eq!(parse_color(&[], opts), None);

    // Hex 3/4/6/8
    assert_eq!(
        parse_color_str("#f00", opts),
        Some(ParsedColor::Paint(PaintColor::srgb([1.0, 0.0, 0.0, 1.0])))
    );
    assert_eq!(
        parse_color_str("#00ff0080", opts)
            .unwrap()
            .to_srgb_opt()
            .unwrap()[1],
        1.0
    );

    // rgb / rgba legacy & modern
    assert_eq!(
        parse_color_str("rgb(255, 0, 128)", opts)
            .unwrap()
            .to_srgb_opt()
            .unwrap()[0],
        1.0
    );
    assert_eq!(
        parse_color_str("rgb(255 0 0 / 50%)", opts)
            .unwrap()
            .to_srgb_opt()
            .unwrap()[3],
        0.5
    );
    // Reject mixed comma + slash
    assert_eq!(parse_color_str("rgb(255, 0, 0 / 50%)", opts), None);

    // hsl / hsla
    let green = parse_color_str("hsl(120deg 100% 50% / 0.5)", opts)
        .unwrap()
        .to_srgb_opt()
        .unwrap();
    assert!((green[1] - 1.0).abs() < 1e-4 && (green[3] - 0.5).abs() < 1e-4);

    // oklch
    assert!(matches!(
        parse_color_str("oklch(0.7 0.15 180deg / 80%)", opts),
        Some(ParsedColor::Paint(PaintColor::Oklch { .. }))
    ));

    // color-mix
    let mixed = parse_color_str("color-mix(in srgb, red 50%, blue 50%)", opts)
        .unwrap()
        .to_srgb_opt()
        .unwrap();
    assert!((mixed[0] - 0.5).abs() < 1e-4 && (mixed[2] - 0.5).abs() < 1e-4);

    // named, transparent, currentcolor, theme token
    assert_eq!(
        parse_color_str("transparent", opts),
        Some(ParsedColor::Paint(PaintColor::srgb([0.0, 0.0, 0.0, 0.0])))
    );
    assert_eq!(
        parse_color_str("currentColor", opts),
        Some(ParsedColor::CurrentColor)
    );
    assert_eq!(
        parse_color_str("rebeccapurple", opts)
            .unwrap()
            .to_srgb_opt()
            .unwrap()[0],
        102.0 / 255.0
    );
    assert_eq!(
        parse_color_str("var(--nana-accent)", opts),
        Some(ParsedColor::Paint(PaintColor::theme_role(
            SemanticColorRole::Accent
        )))
    );
}

#[test]
fn values_position_1_2_3_4_fixes_right_10px_and_center_20px() {
    // 2-value fixes from audit: `right 10px` and `center 20px`!
    assert_eq!(
        parse_position(&cvs("right 10px")),
        Some(BackgroundPosition {
            x: LengthSpec::Percent(100.0),
            y: LengthSpec::Px(10.0),
        })
    );
    assert_eq!(
        parse_position(&cvs("center 20px")),
        Some(BackgroundPosition {
            x: LengthSpec::Percent(50.0),
            y: LengthSpec::Px(20.0),
        })
    );
    assert_eq!(
        parse_position(&cvs("top right")),
        Some(BackgroundPosition {
            x: LengthSpec::Percent(100.0),
            y: LengthSpec::Percent(0.0),
        })
    );
    // Invalid 2-value ordering rejected per CSS Backgrounds 3
    assert_eq!(parse_position(&cvs("top 10px")), None);
    assert_eq!(parse_position(&cvs("10px left")), None);

    // 3-value & 4-value edge-offset syntax
    let pos3 = parse_position(&cvs("right 10px bottom")).unwrap();
    assert_eq!(pos3.x.resolve_px(Some(200.0)), Some(190.0));
    assert_eq!(pos3.y, LengthSpec::Percent(100.0));

    let pos4 = parse_position(&cvs("right 10px bottom 20px")).unwrap();
    assert_eq!(pos4.x.resolve_px(Some(200.0)), Some(190.0));
    assert_eq!(pos4.y.resolve_px(Some(100.0)), Some(80.0));
}

#[test]
fn values_gradient_stops_preserve_explicit_ends_and_never_fake_px_div_100() {
    let opts = ColorParseOptions::default();
    let resolve = |c: &ParsedColor| c.into_paint().unwrap();

    // Explicit first (20%) and last (80%) MUST be preserved (never overwritten to 0.0 / 1.0)!
    let raw = vec![
        parse_gradient_stop(&cvs("red 20%"), opts).unwrap(),
        parse_gradient_stop(&cvs("green"), opts).unwrap(),
        parse_gradient_stop(&cvs("blue 80%"), opts).unwrap(),
    ];
    let stops = normalize_gradient_stops(&raw, None, resolve).unwrap();
    assert_eq!(stops.len(), 3);
    assert!((stops[0].position - 0.2).abs() < 1e-5);
    assert!((stops[1].position - 0.5).abs() < 1e-5);
    assert!((stops[2].position - 0.8).abs() < 1e-5);

    // Px stops MUST NOT divide by hardcoded 100.0: fail closed when gradient line length is unknown!
    let px_raw = vec![
        parse_gradient_stop(&cvs("red 50px"), opts).unwrap(),
        parse_gradient_stop(&cvs("blue 150px"), opts).unwrap(),
    ];
    assert!(
        normalize_gradient_stops(&px_raw, None, resolve).is_none(),
        "px gradient stops must fail closed when gradient line length is unknown"
    );
    // With concrete gradient line length (200px): 50/200 = 0.25, 150/200 = 0.75.
    let px_stops = normalize_gradient_stops(&px_raw, Some(200.0), resolve).unwrap();
    assert!((px_stops[0].position - 0.25).abs() < 1e-5);
    assert!((px_stops[1].position - 0.75).abs() < 1e-5);
}
