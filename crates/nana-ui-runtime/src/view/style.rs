//! Styles compiled from CSS at build time.
//!
//! The `.vue` compiler (and `css!`) parse and match a view's stylesheet
//! while the application builds, with the CSS engine of `nana-ui-css`. What
//! reaches the running program is data: for each element, the rules that
//! apply to it in cascade order, each as a [`StylePatch`] (the Style Model
//! fields its declarations set) and the conditional classes it needs. No
//! CSS is parsed and no selector is matched at run time, and this crate has
//! no CSS in it.
//!
//! A [`StyleSite`] composes an element's base layout with the patches whose
//! classes are active, once per distinct base and class set, and hands out
//! the same shared layout to every instance after that.

use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use nana_ui_core::LayoutStyle;
use serde_json::Value;

use super::controls::StyledComponent;
use super::node::El;
use super::prop::{FieldWrite, PropSource};
use crate::ComponentView;

/// The Style Model fields one rule's declarations set, as the JSON of
/// those fields: produced by the build, read once.
pub struct StylePatch {
    json: &'static str,
    value: OnceLock<Value>,
}

impl StylePatch {
    #[doc(hidden)]
    pub const fn new(json: &'static str) -> Self {
        Self {
            json,
            value: OnceLock::new(),
        }
    }

    fn value(&self) -> &Value {
        self.value.get_or_init(|| {
            serde_json::from_str(self.json).expect("a compiled style patch is valid JSON")
        })
    }
}

/// One element's compiled rules, in cascade order: each patch applies
/// while every conditional class in its mask is active (bit `i` is the
/// element's `i`-th conditional class).
pub struct StyleSite {
    rules: &'static [(u64, &'static StylePatch)],
    /// `(base, active classes) -> composed`, shared by every instance.
    composed: Mutex<Vec<(Arc<LayoutStyle>, u64, Arc<LayoutStyle>)>>,
}

impl StyleSite {
    #[doc(hidden)]
    pub const fn new(rules: &'static [(u64, &'static StylePatch)]) -> Self {
        Self {
            rules,
            composed: Mutex::new(Vec::new()),
        }
    }

    /// The base layout equal to `base` that this site has composed from
    /// before, or `base` itself. A binding that keeps its base holds this
    /// one: every instance then shares it, and composing finds it by
    /// pointer instead of comparing whole layouts.
    fn shared_base(&self, base: &Arc<LayoutStyle>) -> Arc<LayoutStyle> {
        let composed = self.composed.lock().unwrap_or_else(PoisonError::into_inner);
        composed
            .iter()
            .find(|(known, _, _)| Arc::ptr_eq(known, base) || **known == **base)
            .map_or_else(|| Arc::clone(base), |(known, _, _)| Arc::clone(known))
    }

    /// `base` with the patches that apply under `active`, in order.
    pub fn compose(&self, base: &Arc<LayoutStyle>, active: u64) -> Arc<LayoutStyle> {
        let mut composed = self.composed.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, _, layout)) = composed.iter().find(|(known, mask, _)| {
            *mask == active && (Arc::ptr_eq(known, base) || **known == **base)
        }) {
            return Arc::clone(layout);
        }
        let applying = self
            .rules
            .iter()
            .filter(|(needs, _)| needs & !active == 0)
            .map(|(_, patch)| patch.value());
        let layout = Arc::new(apply(base, applying));
        composed.push((Arc::clone(base), active, Arc::clone(&layout)));
        layout
    }
}

fn apply<'a>(base: &LayoutStyle, patches: impl Iterator<Item = &'a Value>) -> LayoutStyle {
    let mut value = serde_json::to_value(base).expect("a layout serializes");
    for patch in patches {
        merge(&mut value, patch);
    }
    serde_json::from_value(value).expect("a patched layout deserializes")
}

/// Objects merge key by key; anything else is replaced.
fn merge(into: &mut Value, patch: &Value) {
    match (into, patch) {
        (Value::Object(into), Value::Object(patch)) => {
            for (key, value) in patch {
                match into.get_mut(key) {
                    Some(slot) => merge(slot, value),
                    None => {
                        into.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (into, patch) => *into = patch.clone(),
    }
}

/// Writes a composed layout, keeping what `.visible` decided.
#[doc(hidden)]
pub struct ComposedLayout;

impl<C: StyledComponent> FieldWrite<C, Arc<LayoutStyle>> for ComposedLayout {
    const FIELD: &'static str = "style.layout";

    fn write(target: &mut C, mut layout: Arc<LayoutStyle>) {
        let style = target.node_style_mut();
        if style.layout.hidden != layout.hidden {
            Arc::make_mut(&mut layout).hidden = style.layout.hidden;
        }
        style.layout = layout;
    }

    fn differs(target: &C, layout: &Arc<LayoutStyle>) -> bool {
        let current = &target.node_style().layout;
        !Arc::ptr_eq(current, layout) && (current.hidden != layout.hidden || **current != **layout)
    }
}

impl<C: StyledComponent + ComponentView, K> El<C, K> {
    /// Compiled styles (the `.vue` compiler writes these): the site's rules
    /// over the element's layout as built, the `i`-th entry of `classes`
    /// switching the element's `i`-th conditional class. Call it before
    /// other layout props.
    #[doc(hidden)]
    #[track_caller]
    pub fn styles(self, site: &'static StyleSite, classes: Vec<PropSource<bool>>) -> Self {
        let base = &self.component_ref().node_style().layout;
        if classes.is_empty() {
            let composed = site.compose(base, 0);
            return self.prop::<Arc<LayoutStyle>, ComposedLayout>(super::Fixed(composed));
        }
        let base = site.shared_base(base);
        self.prop::<Arc<LayoutStyle>, ComposedLayout>(move || {
            let active =
                classes
                    .iter()
                    .enumerate()
                    .fold(0u64, |mask, (bit, class)| match class.get() {
                        true => mask | 1 << bit,
                        false => mask,
                    });
            site.compose(&base, active)
        })
    }
}

/// One declaration block compiled at build time (`css! { … }`): its patch
/// and its implicit animations. See [`El::css`].
#[derive(Clone, Copy)]
pub struct InlineStyle {
    site: Option<&'static StyleSite>,
    animate: &'static [super::Implicit],
}

impl InlineStyle {
    #[doc(hidden)]
    pub const fn new(
        site: Option<&'static StyleSite>,
        animate: &'static [super::Implicit],
    ) -> Self {
        Self { site, animate }
    }
}

impl<C: StyledComponent + ComponentView, K> El<C, K> {
    /// A declaration block compiled by `css!`: `column(8.0, rows)
    /// .css(css! { padding: 12px; transition: opacity 150ms })`. Call it
    /// before other layout props.
    #[track_caller]
    pub fn css(self, style: InlineStyle) -> Self {
        let styled = match style.site {
            Some(site) => self.styles(site, Vec::new()),
            None => self,
        };
        styled.animate(style.animate.iter().copied())
    }
}
