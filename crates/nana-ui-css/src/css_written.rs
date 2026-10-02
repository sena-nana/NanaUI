//! Which Style Model fields a block of declarations writes.
//!
//! A build-time consumer (the L3 view compiler) turns a rule into a patch
//! over whatever layout an element is built with. The patch must hold every
//! field a declaration writes, including a write of the value
//! [`LayoutStyle::default`] happens to have: `align-items: flex-start` on a
//! row built centered has to reset it. Comparing the result with the default
//! cannot see such a write, so the declarations are applied twice: once over
//! the default and once over a witness layout in which every field differs
//! from the default. A field is written when either application changes it
//! and, for the witness, only when it lands on the same value as over the
//! default: a field whose new value depends on what was there before (a
//! physical edge re-derived from the base's logical edges after `direction`
//! changes) is not the declaration's to set.
//!
//! Fields are addressed by their path through the Style Model's structs
//! (`align_items`, `paint.outline.width`); an enum or an `Option` is one
//! field, replaced whole.

use std::sync::OnceLock;

use nana_ui_core::{LayoutFieldMask, LayoutStyle};
use serde_json::{Map, Value};

use crate::css_map::LayoutStyleCss;

/// The layout the declarations give a default one, and the fields they
/// write, as dotted paths into the Style Model.
#[derive(Debug, Clone)]
pub struct WrittenLayout {
    pub layout: LayoutStyle,
    pub written: Vec<String>,
    /// Coarse ownership groups written by the declaration set. The original
    /// paths remain available for code generation; the mask lets a resolver
    /// classify invalidation without reparsing CSS.
    pub field_mask: LayoutFieldMask,
}

impl WrittenLayout {
    /// The written fields and their values, as a JSON object keyed by
    /// dotted path.
    pub fn patch(&self) -> Map<String, Value> {
        let layout = layout_json(&self.layout);
        self.written
            .iter()
            .map(|path| (path.clone(), leaf(&layout, path).clone()))
            .collect()
    }
}

/// Apply `declarations` (e.g. a loop of
/// [`LayoutStyleCss::apply_css_property`]) and report what they wrote.
/// Custom properties, the viewport and anything else the declarations read
/// are the caller's to install around this call.
pub fn written_layout(declarations: impl Fn(&mut LayoutStyle)) -> WrittenLayout {
    let (default, witness_json) = reference();
    let mut layout = LayoutStyle::default();
    declarations(&mut layout);
    let mut over_witness = witness();
    declarations(&mut over_witness);
    let applied = layout_json(&layout);
    let applied_witness = layout_json(&over_witness);
    let written: Vec<String> = leaf_paths(default)
        .into_iter()
        .filter(|path| {
            let value = leaf(&applied, path);
            value != leaf(default, path)
                || (leaf(&applied_witness, path) != leaf(witness_json, path)
                    && leaf(&applied_witness, path) == value)
        })
        .collect();
    let field_mask = written.iter().fold(LayoutFieldMask::NONE, |mask, path| {
        mask.union(LayoutFieldMask::from_property(path))
    });
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
    ("font-feature-settings", "\"liga\" 0"),
    ("pointer-events", "none"),
    ("border-image", "linear-gradient(red, blue) 1"),
    ("border-image", "url(a.png) 30 round"),
    ("border-radius", "4px"),
    ("text-shadow", "1px 1px red"),
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
    ("transform", "translate(1px, 2px)"),
    ("transform", "rotateX(30deg)"),
    ("transform", "nonsense(1)"),
    ("transform-origin", "10px 10px"),
    ("transform-box", "border-box"),
    ("perspective", "100px"),
    ("transform-style", "preserve-3d"),
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
    ("float", "left"),
    ("clear", "both"),
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
    ("color", "red"),
    ("grid-template-columns", "repeat(auto-fill, 10px)"),
    ("grid-template-rows", "repeat(auto-fill, 10px)"),
    ("grid-template-columns", "subgrid"),
    ("grid-template-rows", "subgrid"),
    ("grid-template-columns", "repeat(2, repeat(2, 10px))"),
    ("grid-template-rows", "repeat(2, repeat(2, 10px))"),
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

fn layout_json(layout: &LayoutStyle) -> Value {
    serde_json::to_value(layout).expect("a layout serializes")
}

/// The default layout's JSON and the witness's.
fn reference() -> &'static (Value, Value) {
    static REFERENCE: OnceLock<(Value, Value)> = OnceLock::new();
    REFERENCE.get_or_init(|| {
        let default = layout_json(&LayoutStyle::default());
        let mut witness = default.clone();
        for (property, value) in WITNESS_DECLARATIONS {
            let mut alone = LayoutStyle::default();
            alone.apply_css_property(property, value, None, None);
            let alone = layout_json(&alone);
            for path in leaf_paths(&default) {
                if leaf(&witness, &path) == leaf(&default, &path)
                    && leaf(&alone, &path) != leaf(&default, &path)
                {
                    *leaf_mut(&mut witness, &path) = leaf(&alone, &path).clone();
                }
            }
        }
        let mut layout: LayoutStyle =
            serde_json::from_value(witness).expect("the witness is a layout");
        // What no declaration writes (class hints, replaced elements and
        // pseudo-elements set these) still gets a value of its own, so the
        // witness stays complete should a declaration ever write one.
        layout.border_radius = Some(1.0);
        layout.layout_isolation = true;
        layout.placeholder_color = Some([0.5; 4]);
        layout.placeholder_opacity = Some(0.5);
        layout.selection_background = Some([0.5; 4]);
        layout.selection_color = Some([0.5; 4]);
        crate::css_paint::apply_img_replaced_content(&mut layout, "witness.png");
        layout.paint.skipped_replaced = Some("video".into());
        layout.paint.scrollbar = Some(nana_ui_core::ScrollbarSkin {
            thickness: Some(1.0),
            ..Default::default()
        });
        // Applying anything ends by resolving logical edges; start from a
        // layout that is already resolved, so that alone changes nothing.
        layout.resolve_logical_box_edges();
        (default, layout_json(&layout))
    })
}

fn witness() -> LayoutStyle {
    serde_json::from_value(reference().1.clone()).expect("the witness is a layout")
}

/// The paths of the Style Model's fields: a struct is an object in the
/// default layout's JSON (an enum or `Option` there is a string or null), so
/// every object is descended into and everything else is a field.
fn leaf_paths(default: &Value) -> Vec<String> {
    fn walk(value: &Value, prefix: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(fields) => {
                for (key, field) in fields {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    walk(field, &path, out);
                }
            }
            _ => out.push(prefix.to_owned()),
        }
    }
    let mut out = Vec::new();
    walk(default, "", &mut out);
    out
}

fn leaf<'a>(value: &'a Value, path: &str) -> &'a Value {
    path.split('.')
        .try_fold(value, |value, key| value.get(key))
        .unwrap_or(&Value::Null)
}

fn leaf_mut<'a>(value: &'a mut Value, path: &str) -> &'a mut Value {
    path.split('.').fold(value, |value, key| {
        value
            .as_object_mut()
            .expect("a Style Model path runs through structs")
            .entry(key)
            .or_insert(Value::Null)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let (default, witness) = reference();
        let same: Vec<String> = leaf_paths(default)
            .into_iter()
            .filter(|path| leaf(default, path) == leaf(witness, path))
            .collect();
        assert!(same.is_empty(), "the witness keeps the default of {same:?}");
    }

    #[test]
    fn a_declaration_that_sets_nothing_writes_nothing() {
        assert!(written(&[("unicode-bidi", "isolate")]).is_empty());
        assert!(written(&[("align-items", "nonsense")]).is_empty());
    }

    #[test]
    fn a_write_of_the_default_value_is_a_write() {
        assert_eq!(written(&[("align-items", "flex-start")]), ["align_items"]);
        assert_eq!(written(&[("position", "static")]), ["position"]);
        assert_eq!(written(&[("flex-wrap", "nowrap")]), ["flex_wrap"]);
        assert_eq!(written(&[("outline-width", "0")]), ["paint.outline.width"]);
    }

    #[test]
    fn a_field_derived_from_what_was_there_is_not_written() {
        let fields = written(&[("direction", "rtl")]);
        assert!(fields.contains(&"dir".to_owned()), "{fields:?}");
        assert!(
            !fields.iter().any(|path| path.starts_with("padding")),
            "{fields:?}"
        );
    }
}
