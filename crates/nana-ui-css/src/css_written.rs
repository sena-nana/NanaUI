//! Records which [`LayoutStyle`] leaves a CSS declaration block wrote
//! and emits Rust [`StylePatch`](nana_ui_core::StylePatch) statements for them.
//!
//! Comparing an applied layout against [`LayoutStyle::default`] alone misses a
//! declaration that writes the default value itself (`opacity: 1`,
//! `border-width: 0`) to override an earlier rule or component preset. To catch
//! those too, declarations are also applied onto a *witness* layout where
//! every leaf starts away from its default: any leaf whose value changes to
//! match what was applied was written.

use std::sync::OnceLock;

use nana_ui_core::LayoutStyleChange;

use crate::LayoutStyle;
use crate::css_cascade::DeclarationEntry;
use crate::css_map::{CssVarScope, LayoutStyleCss, with_dynamic_light_dark};
use crate::rust_expr::emit_patch_stmt;

/// The layout built by a declaration block and the leaves it wrote.
#[derive(Debug, Clone)]
pub struct WrittenLayout {
    pub layout: LayoutStyle,
    pub written: Vec<String>,
    /// Ownership groups written by the declaration set. The original
    /// paths remain available for code generation; the mask lets a resolver
    /// classify invalidation without reparsing CSS.
    pub field_mask: LayoutStyleChange,
}

impl WrittenLayout {
    /// Emit Rust setter statements (`{patch_mod}::set::<path>(__nana_layout, ...);`)
    /// for every leaf path recorded in `self.written`.
    pub fn patch_rust_stmts(&self, patch_mod: &str) -> String {
        let mut out = String::new();
        for path in &self.written {
            emit_patch_stmt(&self.layout, path, patch_mod, &mut out);
        }
        out
    }

    /// Compile declaration entries against light and dark [`CssVarScope`]s,
    /// merging differing theme colors into [`nana_ui_core::PaintColor::light_dark`].
    pub fn compile_dual(
        entries: &[&DeclarationEntry],
        vars_light: &CssVarScope,
        vars_dark: &CssVarScope,
    ) -> Self {
        let run = |vars: &CssVarScope| {
            with_dynamic_light_dark(true, || {
                written_layout_in(vars, |layout, vars| {
                    for entry in entries {
                        layout.apply_css_property_in(
                            &entry.property,
                            &entry.value,
                            None,
                            None,
                            vars,
                        );
                    }
                })
            })
        };
        let light = run(vars_light);
        if vars_light == vars_dark {
            return light;
        }
        let dark = run(vars_dark);
        light.merge_light_dark(dark)
    }

    /// Combine a light-mode and dark-mode [`WrittenLayout`] compiled from the
    /// same declarations, promoting any color that differs across `:root` /
    /// `:root[data-theme="dark"]` into [`nana_ui_core::PaintColor::light_dark`].
    pub fn merge_light_dark(mut self, dark: Self) -> Self {
        use nana_ui_core::{BackgroundImage, CssGradient, MaskImage, PaintColor};

        if self.layout == dark.layout && self.written == dark.written {
            return self;
        }
        let add_written = |written: &mut Vec<String>, path: &str| {
            if !written.iter().any(|p| p == path) {
                written.push(path.to_owned());
            }
        };
        let merge_slot = |l_pc: &mut Option<PaintColor>,
                          d_pc: Option<PaintColor>,
                          paint_path: &str,
                          written: &mut Vec<String>| {
            if *l_pc == d_pc {
                return;
            }
            let merged = PaintColor::light_dark(
                (*l_pc).unwrap_or(PaintColor::srgb([0.0; 4])),
                d_pc.unwrap_or(PaintColor::srgb([0.0; 4])),
            );
            *l_pc = Some(merged);
            add_written(written, paint_path);
        };

        let d_slots = &*dark.layout.paint_colors;
        let l_slots = &mut *self.layout.paint_colors;
        merge_slot(
            &mut l_slots.color,
            d_slots.color,
            "paint_colors.color",
            &mut self.written,
        );
        merge_slot(
            &mut l_slots.background,
            d_slots.background,
            "paint_colors.background",
            &mut self.written,
        );
        merge_slot(
            &mut l_slots.border,
            d_slots.border,
            "paint_colors.border",
            &mut self.written,
        );
        merge_slot(
            &mut l_slots.border_top,
            d_slots.border_top,
            "paint_colors.border_top",
            &mut self.written,
        );
        merge_slot(
            &mut l_slots.border_right,
            d_slots.border_right,
            "paint_colors.border_right",
            &mut self.written,
        );
        merge_slot(
            &mut l_slots.border_bottom,
            d_slots.border_bottom,
            "paint_colors.border_bottom",
            &mut self.written,
        );
        merge_slot(
            &mut l_slots.border_left,
            d_slots.border_left,
            "paint_colors.border_left",
            &mut self.written,
        );
        merge_slot(
            &mut l_slots.outline,
            d_slots.outline,
            "paint_colors.outline",
            &mut self.written,
        );
        merge_slot(
            &mut l_slots.selection_background,
            d_slots.selection_background,
            "paint_colors.selection_background",
            &mut self.written,
        );
        merge_slot(
            &mut l_slots.selection_color,
            d_slots.selection_color,
            "paint_colors.selection_color",
            &mut self.written,
        );

        let merge_pair = |l_pc: &mut PaintColor, d_pc: PaintColor| {
            if *l_pc != d_pc {
                *l_pc = PaintColor::light_dark(*l_pc, d_pc);
            }
        };
        let merge_gradient = |lg: &mut CssGradient, dg: &CssGradient| {
            for (ls, ds) in lg.stops_mut().iter_mut().zip(dg.stops().iter()) {
                if ls.color != ds.color {
                    ls.color = PaintColor::light_dark(ls.color, ds.color);
                }
            }
        };

        if !self.layout.paint.is_shared_default() || !dark.layout.paint.is_shared_default() {
            let dp = &*dark.layout.paint;
            let lp = &mut *self.layout.paint;
            for (l_s, d_s) in lp.box_shadows.iter_mut().zip(dp.box_shadows.iter()) {
                merge_pair(&mut l_s.color, d_s.color);
            }
            for (l_s, d_s) in lp.text_shadows.iter_mut().zip(dp.text_shadows.iter()) {
                merge_pair(&mut l_s.color, d_s.color);
            }
            if let (Some(ls), Some(ds)) = (&mut lp.text_stroke, &dp.text_stroke) {
                match (&mut ls.color, ds.color) {
                    (Some(lc), Some(dc)) => merge_pair(lc, dc),
                    (None, Some(dc)) => {
                        let lc = PaintColor::srgb([0.0; 4]);
                        if lc != dc {
                            ls.color = Some(PaintColor::light_dark(lc, dc));
                        }
                    }
                    (Some(lc), None) => {
                        let dc = PaintColor::srgb([0.0; 4]);
                        if *lc != dc {
                            *lc = PaintColor::light_dark(*lc, dc);
                        }
                    }
                    (None, None) => {}
                }
            }
            if let (Some(lf), Some(df)) = (&mut lp.filter, &dp.filter)
                && let (Some(ls), Some(ds)) = (&mut lf.drop_shadow, &df.drop_shadow)
            {
                merge_pair(&mut ls.color, ds.color);
            }
            for (ll, dl) in lp
                .background_layers
                .iter_mut()
                .zip(dp.background_layers.iter())
            {
                if let (BackgroundImage::Gradient(lg), BackgroundImage::Gradient(dg)) = (ll, dl) {
                    merge_gradient(lg, dg);
                }
            }
            if let (Some(MaskImage::Gradient(lg)), Some(MaskImage::Gradient(dg))) =
                (&mut lp.mask, &dp.mask)
            {
                merge_gradient(lg, dg);
            }
        }
        self
    }
}

/// Apply `declarations` (e.g. a loop of
/// [`LayoutStyleCss::apply_css_property`]) and report what they wrote.
pub fn written_layout(declarations: impl Fn(&mut LayoutStyle)) -> WrittenLayout {
    written_layout_in(CssVarScope::EMPTY, |layout, _| declarations(layout))
}

/// Apply `declarations` with an explicit [`CssVarScope`] and report what they wrote.
pub fn written_layout_in(
    vars: &CssVarScope,
    declarations: impl Fn(&mut LayoutStyle, &CssVarScope),
) -> WrittenLayout {
    let (default, witness_layout) = reference();
    let mut layout = LayoutStyle::default();
    declarations(&mut layout, vars);
    let mut over_witness = witness_layout.clone();
    declarations(&mut over_witness, vars);
    let written = layout.collect_written_leaves(default, &over_witness, witness_layout);
    let field_mask = default
        .changed_fields(&layout)
        .union(witness_layout.changed_fields(&over_witness));
    WrittenLayout {
        layout,
        written,
        field_mask,
    }
}

/// Every declaration here is valid and each leaves at least one field away
/// from its default; the witness takes, for each field, the value of the
/// first one that moves it.
const WITNESS_DECLARATIONS: &[(&str, &str)] = &[
    ("display", "grid"),
    ("grid-template-columns", "[a] 10px [b] 20px"),
    ("grid-template-rows", "[c] 10px [d]"),
    ("display", "flex"),
    ("display", "none"),
    ("flex-direction", "column-reverse"),
    ("flex-wrap", "wrap"),
    ("flex", "2 3 10px"),
    ("order", "2"),
    ("gap", "3px"),
    ("row-gap", "4px"),
    ("column-gap", "5px"),
    ("padding", "1px"),
    ("padding-top", "1px"),
    ("padding-right", "1px"),
    ("padding-bottom", "1px"),
    ("padding-left", "1px"),
    ("margin", "1px"),
    ("margin-top", "1px"),
    ("margin-right", "1px"),
    ("margin-bottom", "1px"),
    ("margin-left", "1px"),
    ("top", "1px"),
    ("right", "1px"),
    ("bottom", "1px"),
    ("left", "1px"),
    ("width", "10px"),
    ("height", "10px"),
    ("min-width", "0"),
    ("max-width", "10px"),
    ("min-height", "5px"),
    ("max-height", "10px"),
    ("align-items", "center"),
    ("align-self", "center"),
    ("align-content", "center"),
    ("justify-content", "center"),
    ("justify-items", "center"),
    ("justify-self", "center"),
    ("overflow", "hidden"),
    ("box-sizing", "content-box"),
    ("background-color", "red"),
    ("background-image", "linear-gradient(red, blue)"),
    (
        "background-image",
        "linear-gradient(red, blue), linear-gradient(blue, red)",
    ),
    ("background-size", "10px 20px"),
    ("background-size", "cover"),
    ("background-position", "10px 20px"),
    ("background-repeat", "no-repeat"),
    ("object-fit", "cover"),
    ("object-position", "10px 20px"),
    ("mask-image", "linear-gradient(red, blue)"),
    ("clip-path", "circle(50%)"),
    ("filter", "blur(2px)"),
    ("backdrop-filter", "blur(4px)"),
    ("box-shadow", "0 1px 2px red"),
    ("outline", "2px solid red"),
    ("mix-blend-mode", "multiply"),
    ("line-clamp", "2"),
    ("text-decoration", "underline"),
    ("text-emphasis-style", "filled sesame"),
    ("text-emphasis-position", "under left"),
    ("text-emphasis-color", "red"),
    ("font-feature-settings", "\"liga\" 0"),
    ("pointer-events", "none"),
    ("interactivity", "inert"),
    ("border-image", "linear-gradient(red, blue) 1"),
    ("border-image", "url(a.png) 30 round"),
    ("border-radius", "4px"),
    ("text-shadow", "1px 1px red"),
    ("-webkit-text-stroke", "1px red"),
    ("paint-order", "stroke"),
    ("border-width", "1px"),
    ("border-top-width", "1px"),
    ("border-right-width", "1px"),
    ("border-bottom-width", "1px"),
    ("border-left-width", "1px"),
    ("border-color", "red"),
    ("border-top-color", "red"),
    ("border-right-color", "red"),
    ("border-bottom-color", "red"),
    ("border-left-color", "red"),
    ("border-style", "solid"),
    ("border-top-style", "solid"),
    ("border-right-style", "solid"),
    ("border-bottom-style", "solid"),
    ("border-left-style", "solid"),
    ("position", "absolute"),
    ("z-index", "3"),
    ("isolation", "isolate"),
    ("container-type", "size"),
    ("container-name", "witness"),
    ("transform", "translate(1px, 2px)"),
    ("transform", "rotateX(30deg)"),
    ("transform", "nonsense(1)"),
    ("transform-origin", "10px 10px"),
    ("transform-box", "border-box"),
    ("perspective", "100px"),
    ("perspective-origin", "10px 10px"),
    ("transform-style", "preserve-3d"),
    ("backface-visibility", "hidden"),
    ("padding-inline", "2px 3px"),
    ("padding-block", "4px 5px"),
    ("margin-inline", "2px 3px"),
    ("margin-block", "4px 5px"),
    ("inset-inline", "2px 3px"),
    ("inset-block", "4px 5px"),
    ("text-overflow", "ellipsis"),
    ("white-space", "nowrap"),
    ("word-break", "break-all"),
    ("overflow-wrap", "anywhere"),
    ("aspect-ratio", "2"),
    ("text-align", "center"),
    ("direction", "rtl"),
    ("writing-mode", "vertical-rl"),
    ("writing-mode", "sideways-lr"),
    ("text-orientation", "upright"),
    ("vertical-align", "top"),
    ("font-size", "13px"),
    ("font-weight", "700"),
    ("font-style", "italic"),
    ("font-variation-settings", "\"wght\" 400"),
    ("font-variation-settings", "\"wght\" var(--x)"),
    ("font-family", "Inter"),
    ("line-height", "2"),
    ("letter-spacing", "1px"),
    ("font-kerning", "none"),
    ("line-break", "anywhere"),
    ("text-spacing-trim", "trim-both"),
    ("text-autospace", "normal"),
    ("text-justify", "inter-word"),
    ("text-wrap-style", "pretty"),
    ("color", "red"),
    ("grid-template-columns", "repeat(auto-fill, 10px)"),
    ("grid-template-rows", "repeat(auto-fill, 10px)"),
    ("grid-template-columns", "repeat(2, repeat(2, 10px))"),
    ("grid-template-rows", "repeat(2, repeat(2, 10px))"),
    ("grid-template-columns", "subgrid"),
    ("grid-template-rows", "subgrid"),
    ("grid-auto-columns", "10px"),
    ("grid-auto-rows", "10px"),
    ("grid-auto-flow", "column"),
    ("grid-template-areas", "\"a b\""),
    ("grid-column", "1 / 3"),
    ("grid-row", "2 / 4"),
    ("grid-area", "a"),
    ("visibility", "hidden"),
    ("opacity", "0.5"),
    ("cursor", "pointer"),
    ("user-select", "none"),
];

/// The default layout and the witness layout (with every leaf moved off default).
fn reference() -> &'static (LayoutStyle, LayoutStyle) {
    static REFERENCE: OnceLock<(LayoutStyle, LayoutStyle)> = OnceLock::new();
    REFERENCE.get_or_init(|| {
        let default = LayoutStyle::default();
        let mut layout = default.clone();
        for (property, value) in WITNESS_DECLARATIONS {
            let mut alone = LayoutStyle::default();
            alone.apply_css_property(property, value, None, None);
            layout = alone.overlaid(&layout);
        }
        // What no declaration writes (class hints, replaced elements and
        // pseudo-elements set these) still gets a value of its own, so the
        // witness stays complete should a declaration ever write one.
        layout.border_radius = Some(1.0);
        // Controls anchor their surfaces; no declaration writes this.
        layout.anchor = Some(std::sync::Arc::new(nana_ui_core::AnchorSpec::point(
            1.0,
            1.0,
            nana_ui_core::AnchoredMenuPlacement::BottomStart,
        )));
        let theme_expr = nana_ui_core::ThemeExpr::token(nana_ui_core::ThemeToken::Space(
            nana_ui_core::SpacingStep::Md,
        ));
        layout.theme.gap = Some(theme_expr.clone());
        layout.theme.row_gap = Some(theme_expr.clone());
        layout.theme.column_gap = Some(theme_expr.clone());
        layout.theme.padding = Some(theme_expr.clone());
        layout.theme.padding_top = Some(theme_expr.clone());
        layout.theme.padding_right = Some(theme_expr.clone());
        layout.theme.padding_bottom = Some(theme_expr.clone());
        layout.theme.padding_left = Some(theme_expr.clone());
        layout.theme.margin = Some(theme_expr.clone());
        layout.theme.margin_top = Some(theme_expr.clone());
        layout.theme.margin_right = Some(theme_expr.clone());
        layout.theme.margin_bottom = Some(theme_expr.clone());
        layout.theme.margin_left = Some(theme_expr.clone());
        layout.theme.offset_top = Some(theme_expr.clone());
        layout.theme.offset_right = Some(theme_expr.clone());
        layout.theme.offset_bottom = Some(theme_expr.clone());
        layout.theme.offset_left = Some(theme_expr.clone());
        layout.theme.width = Some(theme_expr.clone());
        layout.theme.height = Some(theme_expr.clone());
        layout.theme.min_width = Some(theme_expr.clone());
        layout.theme.max_width = Some(theme_expr.clone());
        layout.theme.min_height = Some(theme_expr.clone());
        layout.theme.max_height = Some(theme_expr.clone());
        layout.theme.flex_basis = Some(theme_expr.clone());
        layout.theme.font_size = Some(theme_expr.clone());
        layout.theme.font_weight = Some(theme_expr.clone());
        layout.theme.line_height = Some(theme_expr.clone());
        layout.theme.letter_spacing = Some(theme_expr.clone());
        layout.theme.border_width = Some(theme_expr.clone());
        layout.theme.border_top_width = Some(theme_expr.clone());
        layout.theme.border_right_width = Some(theme_expr.clone());
        layout.theme.border_bottom_width = Some(theme_expr.clone());
        layout.theme.border_left_width = Some(theme_expr.clone());
        layout.theme.border_radius = Some(theme_expr.clone());
        layout.theme.scrollbar_room = Some(theme_expr);
        layout.layout_isolation = true;
        layout.adaptation = Some(nana_ui_core::dynamic_layout::AdaptationProfile::RIGID.into());
        layout.equal_wrap = true;
        let material = nana_ui_core::MaterialRef::new(nana_ui_core::MaterialProgram::new(
            "fn material(input: MaterialInput) -> vec4<f32> { return input.color; }",
        ));
        layout.paint.background_material = Some(material.clone());
        layout.paint.border_material = Some(material.clone());
        layout.paint.text_material = Some(material);
        layout.text_ext.placeholder_color = Some([0.5; 4]);
        layout.text_ext.placeholder_opacity = Some(0.5);
        layout.paint_colors.selection_background =
            Some(nana_ui_core::PaintColor::Srgb { rgba: [0.5; 4] });
        layout.paint_colors.selection_color =
            Some(nana_ui_core::PaintColor::Srgb { rgba: [0.25; 4] });
        layout.paint.content_image = Some(nana_ui_core::BackgroundImage::url_with_fit(
            "witness.png",
            nana_ui_core::BackgroundImageFit::Stretch,
        ));
        layout.paint.skipped_replaced = Some("video".into());
        layout.paint.scrollbar = Some(nana_ui_core::ScrollbarSkin {
            thickness: Some(1.0),
            ..Default::default()
        });
        layout.paint.scroll_edge_fade = Some(nana_ui_core::ScrollEdgeFade {
            top: 1.0,
            bottom: 1.0,
        });
        // Rust-only, like the edge fade: no CSS property writes them.
        layout.paint.wide_filter_blur = true;
        layout.paint.motion_shutter = 0.5;
        // Applying anything ends by resolving logical edges; start from a
        // layout that is already resolved, so that alone changes nothing.
        layout.resolve_logical_box_edges();
        (default, layout)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `Overlay` path list is the authority for leaf emission.
    #[test]
    fn the_typed_overlay_names_every_leaf_path_and_rust_emitter_covers_them() {
        let (_, witness_layout) = reference();
        let paths = LayoutStyle::overlay_leaf_paths();
        assert_eq!(paths.len(), LayoutStyle::OVERLAY_LEAVES);
        for path in &paths {
            let mut stmt = String::new();
            emit_patch_stmt(witness_layout, path, "__patch", &mut stmt);
            assert!(
                stmt.starts_with("__patch::set::") && stmt.ends_with(");"),
                "unexpected emitted statement for {path}: {stmt}"
            );
        }
    }

    /// A declaration against the default is mirrored in its emitted Rust patch.
    #[test]
    fn every_path_a_declaration_writes_emits_a_rust_patch_statement() {
        let (default, _) = reference();
        for &(property, value) in WITNESS_DECLARATIONS {
            let result = written_layout(|layout| {
                layout.apply_css_property(property, value, None, None);
            });
            let expected = result.layout.collect_written_leaves(
                default,
                &{
                    let mut over = reference().1.clone();
                    over.apply_css_property(property, value, None, None);
                    over
                },
                &reference().1,
            );
            assert_eq!(result.written, expected, "{property}: {value}");
            let stmts = result.patch_rust_stmts("__patch");
            assert!(
                !stmts.contains(":::"),
                "{property}: {value} emitted ':::' token glue in {stmts}"
            );
            for path in &result.written {
                let expected_prefix = format!("__patch::set::{}(", path.replace('.', "::"));
                assert!(
                    stmts.contains(&expected_prefix),
                    "{property}: {value} missing {expected_prefix} in {stmts}"
                );
            }
        }
    }

    fn written(declarations: &[(&str, &str)]) -> Vec<String> {
        written_layout(|layout| {
            for (property, value) in declarations {
                layout.apply_css_property(property, value, None, None);
            }
        })
        .written
    }

    #[test]
    fn every_field_of_the_witness_differs_from_the_default() {
        let (default, witness_layout) = reference();
        let unchanged = witness_layout.unchanged_leaf_paths(default);
        assert!(
            unchanged.is_empty(),
            "add to WITNESS_DECLARATIONS: {unchanged:?}"
        );
    }

    #[test]
    fn a_declaration_that_sets_nothing_writes_nothing() {
        let w = written(&[("frobnicate", "10px")]);
        assert!(w.is_empty(), "got {w:?}");
    }

    #[test]
    fn a_write_of_the_default_value_is_a_write() {
        // The defaults are not kept out of a patch, or writing
        // `opacity: 1` could not cancel an earlier rule's `opacity: 0.5`.
        assert!(written(&[("opacity", "1")]).contains(&"opacity".to_owned()));
        assert!(written(&[("flex-direction", "column")]).contains(&"direction".to_owned()));
        assert!(written(&[("flex-wrap", "nowrap")]).contains(&"flex_wrap".to_owned()));
        // Same inside nested groups (`paint`, `grid.placement`).
        assert!(written(&[("visibility", "visible")]).contains(&"paint.visibility".to_owned()));
        assert!(
            written(&[("grid-column", "auto")]).contains(&"grid.placement.column_start".to_owned())
        );
    }

    #[test]
    fn a_field_derived_from_what_was_there_is_not_written() {
        // A physical padding sets `padding_top` directly; nothing in
        // `logical_padding` has a logical edge, so re-resolving at the end
        // does not rewrite any other edge.
        let w = written(&[("padding-top", "4px")]);
        assert!(!w.contains(&"padding_right".to_owned()));
        assert!(!w.contains(&"padding_bottom".to_owned()));
        assert!(!w.contains(&"padding_left".to_owned()));
    }

    #[test]
    fn var_nana_tokens_preserve_theme_role_and_merge_light_dark() {
        use nana_ui_core::{PaintColor, SemanticColorRole};
        use std::collections::BTreeMap;

        let res = crate::with_dynamic_light_dark(true, || {
            written_layout(|layout| {
                layout.apply_css_property("background", "var(--nana-surface)", None, None);
                layout.apply_css_property("color", "var(--nana-accent)", None, None);
            })
        });
        assert_eq!(
            res.layout.paint_colors.background,
            Some(PaintColor::theme_role(SemanticColorRole::Surface))
        );
        assert_eq!(
            res.layout.paint_colors.color,
            Some(PaintColor::theme_role(SemanticColorRole::Accent))
        );

        let light_vars = CssVarScope::from_map(BTreeMap::from([(
            "--tint".to_owned(),
            "#112233".to_owned(),
        )]));
        let dark_vars = CssVarScope::from_map(BTreeMap::from([(
            "--tint".to_owned(),
            "#ddeeff".to_owned(),
        )]));
        let run = |vars: &CssVarScope| {
            crate::with_dynamic_light_dark(true, || {
                written_layout_in(vars, |layout, vars| {
                    layout.apply_css_property_in("background", "var(--tint)", None, None, vars);
                })
            })
        };
        let merged = run(&light_vars).merge_light_dark(run(&dark_vars));
        assert!(matches!(
            merged.layout.paint_colors.background,
            Some(PaintColor::ThemePair(_))
        ));
        let rust = merged.patch_rust_stmts("__patch");
        assert!(rust.contains("PaintColor::light_dark"), "{rust}");
    }
}
