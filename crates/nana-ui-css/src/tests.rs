use std::sync::Arc;

use nana_ui_core::{
    BackgroundImage, BackgroundPosition, CssGradient, DisplaySpec, FontStyleSpec, LayoutStyle,
    LengthSpec, MediaColorScheme, MediaEnv, MediaOrientation, PaintColor, StyleOp,
};

use crate::{
    CascadeKey, ColorParseOptions, CompileContext, DiagSeverity, LayerOrder, ParsedSheet,
    PseudoElement, Specificity, VarCascadeKey, VarScope, apply_css_text, compile_declarations,
    compile_inline_css, emit_style_ops_rust, parse_color_str, parse_stylesheet,
    parse_stylesheet_with_base,
};

fn layout_of(css: &str) -> LayoutStyle {
    let mut layout = LayoutStyle::default();
    apply_css_text(&mut layout, css);
    layout
}

fn default_ctx() -> CompileContext {
    CompileContext::default()
}

#[test]
fn display_none_undone_by_visible_display_values() {
    for visible in [
        "block",
        "inline-block",
        "flex",
        "inline-flex",
        "grid",
        "inline-grid",
        "inline",
        "layers",
        "free",
    ] {
        let css = format!("display: none; display: {visible};");
        let layout = layout_of(&css);
        assert!(
            !layout.hidden,
            "display: none followed by display: {visible} must clear hidden"
        );
        assert_ne!(
            layout.display,
            Some(DisplaySpec::None),
            "display: none followed by display: {visible} must not remain DisplaySpec::None"
        );
    }

    let hidden = layout_of("display: flex; display: none;");
    assert!(hidden.hidden);
    assert_eq!(hidden.display, Some(DisplaySpec::None));
}

#[test]
fn nested_var_fallbacks_work_across_all_properties() {
    let sheet = parse_stylesheet(
        r#"
        :root {
            --b: 100px;
            --fg: #112233;
            --pad-x: 16px;
        }
        .card {
            width: var(--missing-a, var(--b, 40px));
            height: var(--missing-a, var(--missing-b, 64px));
            padding: 8px var(--unset-x, var(--pad-x, 4px));
            color: var(--unset-fg, var(--fg, #ff0000));
            font-weight: var(--unset-w, var(--also-unset, 550));
            grid-template-columns: var(--unset-cols, 120px 1fr);
        }
        "#,
    );
    let vars = sheet.root_var_scope("dark");
    let rule = sheet
        .rules
        .iter()
        .find(|r| {
            r.selectors
                .0
                .iter()
                .any(|s| format!("{s:?}").contains("card"))
        })
        .expect(".card rule");
    let mut diags = Vec::new();
    let compiled =
        compile_declarations(&rule.declarations, &vars, None, &default_ctx(), &mut diags);
    assert!(
        diags.iter().all(|d| d.severity != DiagSeverity::Error),
        "unexpected error diags: {diags:?}"
    );
    let mut layout = LayoutStyle::default();
    crate::apply_style_ops(&mut layout, &compiled.ops);
    assert_eq!(layout.width, Some(LengthSpec::Px(100.0)));
    assert_eq!(layout.height, Some(LengthSpec::Px(64.0)));
    assert_eq!(layout.padding_top, Some(LengthSpec::Px(8.0)));
    assert_eq!(layout.padding_left, Some(LengthSpec::Px(16.0)));
    assert_eq!(layout.font_weight, Some(550));
    assert!(layout.paint_colors.color.is_some());
}

#[test]
fn empty_values_and_unresolved_vars_never_panic_or_clobber_prior_values() {
    let compiled = compile_inline_css(
        "padding: 12px; padding: ; width: 80px; width: var(--does-not-exist); color: #112233; color: ; background: #abcdef; background: not-a-color-xyz;",
        &VarScope::default(),
        &default_ctx(),
    );
    let mut layout = LayoutStyle::default();
    crate::apply_style_ops(&mut layout, &compiled.ops);
    assert_eq!(layout.padding, Some(LengthSpec::Px(12.0)));
    assert_eq!(layout.width, Some(LengthSpec::Px(80.0)));
    assert!(layout.paint_colors.color.is_some());
    assert!(layout.paint_colors.background.is_some());
    assert!(
        compiled
            .diags
            .iter()
            .any(|d| d.severity == DiagSeverity::Error),
        "malformed declarations must record error diagnostics"
    );
}

#[test]
fn shorthands_split_component_values_with_calc_min_max_clamp() {
    let layout = layout_of(
        "gap: calc(4px + 4px) min(20px, 12px); \
         padding: max(4px, 8px) calc(10px + 6px); \
         margin: clamp(2px, 6px, 10px) auto; \
         inset: calc(10% + 4px) 8px; \
         flex: 1 0 calc(50% - 12px); \
         border-radius: calc(4px + 2px) 10px;",
    );
    assert!(layout.row_gap.is_some());
    assert!(layout.column_gap.is_some());
    assert!(layout.padding_top.is_some());
    assert!(layout.padding_right.is_some());
    assert!(layout.margin_top.is_some());
    assert_eq!(layout.margin_right, Some(LengthSpec::Auto));
    assert!(layout.offset_top.is_some());
    assert_eq!(layout.offset_right, Some(LengthSpec::Px(8.0)));
    assert_eq!(layout.flex_grow, Some(1.0));
    assert_eq!(layout.flex_shrink, Some(0.0));
    assert!(layout.flex_basis.is_some());
    assert!(layout.paint.border_radii.is_some());
}

#[test]
fn gradient_stops_preserve_explicit_first_last_and_reject_px_without_dividing_by_100() {
    let layout = layout_of("background-image: linear-gradient(90deg, red 20%, blue 80%);");
    let Some(BackgroundImage::Gradient(CssGradient::Linear(g))) =
        layout.paint.background_layers.first()
    else {
        panic!("expected linear gradient");
    };
    assert_eq!(g.stops.len(), 2);
    assert!((g.stops[0].position - 0.2).abs() < 1e-4);
    assert!((g.stops[1].position - 0.8).abs() < 1e-4);

    // px stops must NOT silently divide by 100!
    let px_grad = compile_inline_css(
        "background-image: linear-gradient(90deg, red 10px, blue 50px);",
        &VarScope::default(),
        &default_ctx(),
    );
    let mut px_layout = LayoutStyle::default();
    crate::apply_style_ops(&mut px_layout, &px_grad.ops);
    assert!(
        px_layout.paint.background_layers.is_empty(),
        "px gradient stops without line length must be rejected, not divided by 100"
    );
}

#[test]
fn background_position_handles_right_10px_and_center_20px() {
    let a = layout_of("background-position: right 10px;");
    assert_eq!(
        a.paint.background_position_list,
        vec![BackgroundPosition {
            x: LengthSpec::Percent(100.0),
            y: LengthSpec::Px(10.0),
        }]
    );

    let b = layout_of("background-position: center 20px;");
    assert_eq!(
        b.paint.background_position_list,
        vec![BackgroundPosition {
            x: LengthSpec::Percent(50.0),
            y: LengthSpec::Px(20.0),
        }]
    );
}

#[test]
fn transform_supports_translatex_calc_and_scalez() {
    let t1 = layout_of("transform: translateX(calc(10px + 5px));");
    let pt = t1.transforms.transform.expect("2d transform");
    assert!((pt.e - 15.0).abs() < 1e-4, "expected tx=15, got {}", pt.e);

    let t2 = layout_of("transform: scaleZ(2);");
    let m4 = t2.transforms.transform_3d.expect("3d transform");
    assert!(
        (m4.m[10] - 2.0).abs() < 1e-4,
        "expected sz=2, got {}",
        m4.m[10]
    );
}

#[test]
fn parse_color_rejects_empty_string() {
    assert!(parse_color_str("", ColorParseOptions::default()).is_none());
    assert!(parse_color_str("   ", ColorParseOptions::default()).is_none());
}

#[test]
fn negative_relative_lengths_preserved_on_margin_and_inset() {
    let layout = layout_of("margin-top: -10%; margin-left: -2em; top: -5vh; left: -1.5rem;");
    assert_eq!(layout.margin_top, Some(LengthSpec::Percent(-10.0)));
    assert_eq!(layout.margin_left, Some(LengthSpec::Em(-2.0)));
    assert_eq!(
        layout.offset_top,
        Some(LengthSpec::Viewport {
            axis: nana_ui_core::ViewportAxis::Height,
            value: -5.0,
        })
    );
    assert_eq!(layout.offset_left, Some(LengthSpec::Rem(-1.5)));
}

#[test]
fn min_height_100_percent_does_not_mutate_height() {
    let layout = layout_of("min-height: 100%;");
    assert_eq!(layout.min_height, Some(LengthSpec::Percent(100.0)));
    assert_eq!(
        layout.height, None,
        "min-height: 100% must never mutate height"
    );
}

#[test]
fn font_weight_preserves_variable_numeric_weights_and_font_family_preserves_author_order() {
    let layout =
        layout_of(r#"font-weight: 550; font-family: "Inter", "Noto Sans SC", sans-serif;"#);
    assert_eq!(layout.font_weight, Some(550));
    assert_eq!(layout.text_ext.font_family.as_deref(), Some("Inter"));
}

#[test]
fn pseudo_elements_map_to_typed_style_ops() {
    let sheet = parse_stylesheet(
        r#"
        input::placeholder { color: #888888; opacity: 0.65; }
        .doc::selection { background-color: #224488; color: #ffffff; }
        .list::-webkit-scrollbar { width: 10px; }
        .list::-webkit-scrollbar-thumb { background: #555555; }
        .list::-webkit-scrollbar-track { background: #222222; }
        "#,
    );
    let vars = VarScope::default();
    let ctx = default_ctx();
    let mut layout = LayoutStyle::default();
    let mut merged_scrollbar = nana_ui_core::ScrollbarSkin::default();
    let mut has_scrollbar = false;
    let mut diags = Vec::new();
    for rule in &sheet.rules {
        let pseudo = rule
            .selectors
            .0
            .first()
            .and_then(|s| s.subject().pseudo_element.as_ref());
        let compiled = compile_declarations(&rule.declarations, &vars, pseudo, &ctx, &mut diags);
        for op in &compiled.ops {
            if let StyleOp::Scrollbar(Some(skin)) = op {
                crate::merge_scrollbar_skin(&mut merged_scrollbar, skin);
                has_scrollbar = true;
            } else {
                op.apply(&mut layout);
            }
        }
    }
    if has_scrollbar {
        StyleOp::Scrollbar(Some(merged_scrollbar)).apply(&mut layout);
    }
    assert!(layout.text_ext.placeholder_color.is_some());
    assert_eq!(layout.text_ext.placeholder_opacity, Some(0.65));
    assert!(layout.paint_colors.selection_background.is_some());
    assert!(layout.paint_colors.selection_color.is_some());
    let sb = layout.paint.scrollbar.expect("scrollbar skin");
    assert_eq!(sb.thickness, Some(10.0));
    assert!(sb.thumb_color.is_some());
    assert!(sb.track_color.is_some());

    // Direct PseudoElement compile_declarations check.
    let mut direct_diags = Vec::new();
    let ph = parse_stylesheet("a { color: #111111; opacity: 0.5; }");
    let ph_compiled = compile_declarations(
        &ph.rules[0].declarations,
        &vars,
        Some(&PseudoElement::Placeholder),
        &ctx,
        &mut direct_diags,
    );
    assert!(
        ph_compiled
            .ops
            .iter()
            .any(|op| matches!(op, StyleOp::PlaceholderColor(Some(_))))
    );
    assert!(
        ph_compiled
            .ops
            .iter()
            .any(|op| matches!(op, StyleOp::PlaceholderOpacity(Some(_))))
    );
}

#[test]
fn motion_respects_exact_declaration_source_order_between_shorthand_and_longhand() {
    let ctx = default_ctx();
    let a = compile_inline_css(
        "transition-duration: 500ms; transition: opacity 200ms ease-in;",
        &VarScope::default(),
        &ctx,
    );
    assert_eq!(a.transitions.len(), 1);
    assert_eq!(a.transitions[0].duration_ms, 200.0);

    let b = compile_inline_css(
        "transition: opacity 200ms ease-in; transition-duration: 500ms;",
        &VarScope::default(),
        &ctx,
    );
    assert_eq!(b.transitions.len(), 1);
    assert_eq!(b.transitions[0].duration_ms, 500.0);

    let anim_a = compile_inline_css(
        "animation-duration: 800ms; animation: fade 300ms linear;",
        &VarScope::default(),
        &ctx,
    );
    assert_eq!(anim_a.animations.len(), 1);
    assert_eq!(anim_a.animations[0].duration_ms, 300.0);

    let anim_b = compile_inline_css(
        "animation: fade 300ms linear; animation-duration: 800ms;",
        &VarScope::default(),
        &ctx,
    );
    assert_eq!(anim_b.animations.len(), 1);
    assert_eq!(anim_b.animations[0].duration_ms, 800.0);
}

#[test]
fn layer_cascade_key_orders_normal_and_reverses_important() {
    let sheet = parse_stylesheet(
        r#"
        @layer base, theme;
        @layer theme { .btn { color: red; padding: 10px !important; } }
        @layer base { .btn { color: blue; padding: 20px !important; } }
        .btn { color: green; padding: 30px !important; }
        "#,
    );
    assert_eq!(sheet.rules.len(), 3);
    let theme_key = sheet.rules[0].cascade_key(&sheet.rules[0].selectors.0[0], false);
    let base_key = sheet.rules[1].cascade_key(&sheet.rules[1].selectors.0[0], false);
    let unlayered_key = sheet.rules[2].cascade_key(&sheet.rules[2].selectors.0[0], false);

    // Normal: base < theme < unlayered
    assert!(base_key < theme_key);
    assert!(theme_key < unlayered_key);

    // !important reverses layer order: unlayered < theme < base
    let theme_imp = sheet.rules[0].cascade_key(&sheet.rules[0].selectors.0[0], true);
    let base_imp = sheet.rules[1].cascade_key(&sheet.rules[1].selectors.0[0], true);
    let unlayered_imp = sheet.rules[2].cascade_key(&sheet.rules[2].selectors.0[0], true);
    assert!(unlayered_imp < theme_imp);
    assert!(theme_imp < base_imp);

    // Sanity check direct LayerOrder construction
    let _ = CascadeKey {
        layer: LayerOrder::UNLAYERED,
        important: false,
        specificity: Specificity::new(0, 1, 0),
        source_order: 0,
    };
}

#[test]
fn media_and_font_face_lowering_produce_core_types_with_kleene_logic() {
    let sheet: ParsedSheet = parse_stylesheet_with_base(
        r#"
        @font-face {
            font-family: "Brand";
            src: local("Brand Local"), url("./brand.woff2") format("woff2");
            font-weight: 300 700;
            font-style: italic;
        }
        @media (prefers-color-scheme: dark) and (min-width: 800px) {
            .hero { width: 600px; }
        }
        @media not (unknown-feature: 1) {
            .never { width: 10px; }
        }
        "#,
        Some("nana://res/styles/app.css"),
    );

    assert_eq!(sheet.font_faces.len(), 1);
    let ff = &sheet.font_faces[0];
    assert_eq!(ff.family, "Brand");
    assert_eq!(ff.weight, Some(300));
    assert_eq!(ff.weight_end, Some(700));
    assert_eq!(ff.style, Some(FontStyleSpec::Italic));
    assert_eq!(ff.base_href.as_deref(), Some("nana://res/styles/app.css"));
    assert_eq!(ff.sources.len(), 2);

    let dark_wide = MediaEnv {
        width: 1024.0,
        height: 768.0,
        color_scheme: MediaColorScheme::Dark,
        ..MediaEnv::default()
    };
    let light_wide = MediaEnv::desktop(1024.0, 768.0, false);

    // Rule 0: dark && width >= 800
    assert_eq!(sheet.active_rules(&dark_wide).len(), 1);
    // Rule 1: `not (unknown-feature: 1)` must evaluate to None (not true!) in Kleene 3-valued logic.
    assert_eq!(sheet.active_rules(&light_wide).len(), 0);
    assert_eq!(sheet.rules[1].media[0].eval_tri(&dark_wide), None);
    assert_eq!(dark_wide.orientation(), MediaOrientation::Landscape);
}

#[test]
fn element_scoped_custom_props_and_ancestor_var_scope() {
    let sheet = parse_stylesheet(
        r#"
        :root { --accent: #112233; --pad: 8px; }
        .panel { --pad: 20px; }
        .item { padding: var(--pad); color: var(--accent); }
        "#,
    );
    let root_scope = Arc::new(sheet.root_var_scope("dark"));
    let panel_rule = &sheet.rules[1];
    assert_eq!(panel_rule.custom_props.len(), 1);
    assert_eq!(panel_rule.custom_props[0].name, "--pad");

    let mut panel_scope = VarScope::with_parent(root_scope);
    for cp in &panel_rule.custom_props {
        panel_scope.set_with_cascade(
            cp.name.clone(),
            cp.value.clone(),
            VarCascadeKey::unlayered(1),
        );
    }

    let mut diags = Vec::new();
    let compiled = compile_declarations(
        &sheet.rules[2].declarations,
        &panel_scope,
        None,
        &default_ctx(),
        &mut diags,
    );
    let mut layout = LayoutStyle::default();
    crate::apply_style_ops(&mut layout, &compiled.ops);
    assert_eq!(layout.padding, Some(LengthSpec::Px(20.0)));
    assert_eq!(
        layout.paint_colors.color,
        Some(PaintColor::srgb([
            0x11 as f32 / 255.0,
            0x22 as f32 / 255.0,
            0x33 as f32 / 255.0,
            1.0
        ]))
    );
}

#[test]
fn tiered_diagnostics_warn_on_ignorable_vendor_props_and_error_on_unknown() {
    let compiled = compile_inline_css(
        "-moz-osx-font-smoothing: grayscale; -webkit-tap-highlight-color: transparent; totally-bogus-prop: 123px;",
        &VarScope::default(),
        &default_ctx(),
    );
    let warnings: Vec<_> = compiled
        .diags
        .iter()
        .filter(|d| d.severity == DiagSeverity::Warning)
        .collect();
    let errors: Vec<_> = compiled
        .diags
        .iter()
        .filter(|d| d.severity == DiagSeverity::Error)
        .collect();
    assert_eq!(
        warnings.len(),
        2,
        "expected 2 vendor warnings: {compiled:?}"
    );
    assert_eq!(
        errors.len(),
        1,
        "expected 1 unknown-property error: {compiled:?}"
    );
}

#[test]
fn compile_inline_and_emit_rust_roundtrip_smoke() {
    let ctx = default_ctx();
    let mut layout = LayoutStyle::default();
    crate::apply_css_text_with(
        &mut layout,
        "display: flex; gap: 12px; padding: 8px 16px;",
        &VarScope::default(),
        &ctx,
    );
    assert_eq!(layout.display, Some(DisplaySpec::Flex));
    assert_eq!(layout.gap, Some(LengthSpec::Px(12.0)));
    assert_eq!(layout.padding_top, Some(LengthSpec::Px(8.0)));
    assert_eq!(layout.padding_left, Some(LengthSpec::Px(16.0)));

    let inline = compile_inline_css("width: 120px; opacity: 0.75;", &VarScope::default(), &ctx);
    let rust = emit_style_ops_rust(&inline.ops, "::nana_ui_runtime::view::__patch");
    assert!(rust.contains("::nana_ui_runtime::view::__patch::set::width(__nana_layout,"));
    assert!(rust.contains("::nana_ui_runtime::view::__patch::set::opacity(__nana_layout,"));
}
