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
//! A [`Sheet`] is a stylesheet's class rules in cascade order. An element
//! names its classes with [`El::class`] and [`El::class_when`]; the sheet
//! picks the rules those classes can match, once per distinct set of
//! classes, into a [`StyleSite`]. The site composes the element's base
//! layout with the patches whose classes are active, once per distinct base
//! and active set, and hands out the same shared layout to every instance
//! after that.
//!
//! A rule inside a CSS `@container` block carries its query as data
//! ([`SheetQuery`]): the container it reads, the axis, and the extents where
//! it holds. Composing leaves those rules out. Under the same active set,
//! the site cuts the container's extent at every bound their queries use and
//! composes each bucket with the rules that hold there, in cascade order, a
//! later plain rule still winning; what a bucket writes over the authored
//! composition is its [`StyleVariant`]. The element follows the result as a
//! [`ResponsiveRule`] on the nearest query container, which the runtime
//! measures; nothing here does.

use std::collections::HashMap;
use std::panic::Location;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use nana_ui_core::LayoutStyle;
use serde_json::Value;

use super::controls::StyledComponent;
use super::node::{El, NodeBindings};
use super::prop::{FieldWrite, IntoProp, PropSource};
use super::transition::Implicit;
use crate::{
    ComponentView, MAX_RESPONSIVE_BREAKPOINTS, MutationQueue, ResponsiveAxis, ResponsiveContainer,
    ResponsiveRule, StableNodeId, StyleVariant,
};

/// The Style Model fields one rule's declarations write, as a JSON object
/// from each field's dotted path (`align_items`, `paint.outline.width`) to
/// its value: produced by the build, read once. A field written with the
/// value the default layout has is in it too, so a class can reset what
/// the element was built with.
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

/// The CSS `@container` query of a rule, compiled at build time: the
/// container it reads (the nearest one named `name`, or the nearest query
/// container eligible for `axis`) and the extents of `axis` where it holds,
/// half-open `[lo, hi)`, ascending and disjoint. `lo` may be
/// `f32::NEG_INFINITY` and `hi` `f32::INFINITY`; an inclusive bound such as
/// `max-width: 480px` is already `480f32.next_up()`.
#[doc(hidden)]
#[derive(Debug, PartialEq)]
pub struct SheetQuery {
    name: Option<&'static str>,
    axis: ResponsiveAxis,
    intervals: &'static [(f32, f32)],
}

impl SheetQuery {
    #[doc(hidden)]
    pub const fn new(
        name: Option<&'static str>,
        axis: ResponsiveAxis,
        intervals: &'static [(f32, f32)],
    ) -> Self {
        Self {
            name,
            axis,
            intervals,
        }
    }

    /// Whether the query holds for a container measuring `extent` on its
    /// axis.
    fn holds_at(&self, extent: f32) -> bool {
        self.intervals
            .iter()
            .any(|&(lo, hi)| lo <= extent && extent < hi)
    }
}

/// One compiled rule of a site: the conditional classes it needs (bit `i`
/// is the element's `i`-th conditional class), its patch, and the
/// `@container` query that guards it.
type SiteRule = (u64, &'static StylePatch, Option<&'static SheetQuery>);

/// One element's compiled rules, in cascade order: each patch applies
/// while every conditional class in its mask is active, and a guarded one
/// only in the buckets of its container where its query holds.
pub struct StyleSite {
    rules: &'static [SiteRule],
    /// What every instance of the site shares.
    composed: Mutex<Option<Composed>>,
}

/// A site's bases, compositions and responsive rules. Held weakly by base:
/// a base no instance keeps any more takes what was made from it with it at
/// the next prune.
#[derive(Default)]
struct Composed {
    /// The distinct bases seen, one per value.
    bases: Vec<Weak<LayoutStyle>>,
    /// `(base pointer, active classes) -> composed`.
    by_base: HashMap<(usize, u64), (Weak<LayoutStyle>, Arc<LayoutStyle>)>,
    /// `(base pointer, active classes) -> the rule its guarded rules give`.
    responsive: HashMap<(usize, u64), (Weak<LayoutStyle>, Option<Arc<ResponsiveRule>>)>,
    /// Entries at the last prune; the next runs when that doubles.
    pruned_at: usize,
}

impl Composed {
    /// The live base equal to `base`, `base` itself when none is. Pointers
    /// first: a value comparison of a whole layout runs only for a base
    /// pointer not seen before.
    fn canonical(&mut self, base: &Arc<LayoutStyle>) -> Arc<LayoutStyle> {
        let known = |weak: &Weak<LayoutStyle>| weak.upgrade();
        if let Some(found) = self
            .bases
            .iter()
            .filter_map(known)
            .find(|known| Arc::ptr_eq(known, base))
        {
            return found;
        }
        if let Some(found) = self
            .bases
            .iter()
            .filter_map(known)
            .find(|known| **known == **base)
        {
            return found;
        }
        self.bases.push(Arc::downgrade(base));
        Arc::clone(base)
    }

    /// Drop what belongs to bases nothing holds, once the maps have doubled.
    fn prune(&mut self) {
        let entries = self.by_base.len() + self.responsive.len();
        if entries < (self.pruned_at * 2).max(64) {
            return;
        }
        self.bases.retain(|base| base.strong_count() > 0);
        self.by_base.retain(|_, (base, _)| base.strong_count() > 0);
        self.responsive
            .retain(|_, (base, _)| base.strong_count() > 0);
        self.pruned_at = self.by_base.len() + self.responsive.len();
    }
}

/// The container `queries` read and the finite bounds they use, ascending
/// and unique: every bound is a breakpoint, so each query holds across a
/// whole bucket or nowhere in it. `None` without a query; also `None`, and a
/// diagnostic naming where the element's classes were given (`at`), when
/// they ask more than one container or axis or use more than
/// [`MAX_RESPONSIVE_BREAKPOINTS`] bounds.
fn plan(
    queries: &[&SheetQuery],
    at: &'static Location<'static>,
) -> Option<(Option<&'static str>, ResponsiveAxis, Vec<f32>)> {
    let first = queries.first()?;
    let mut breakpoints: Vec<f32> = queries
        .iter()
        .flat_map(|query| query.intervals.iter())
        .flat_map(|&(lo, hi)| [lo, hi])
        .filter(|bound| bound.is_finite())
        .collect();
    breakpoints.sort_by(f32::total_cmp);
    breakpoints.dedup();
    let why = if queries
        .iter()
        .any(|query| query.name != first.name || query.axis != first.axis)
    {
        "ask more than one container or axis"
    } else if breakpoints.len() > MAX_RESPONSIVE_BREAKPOINTS {
        "use more breakpoints than one responsive rule holds"
    } else {
        return Some((first.name, first.axis, breakpoints));
    };
    nana_diagnostics::fault!(
        nana_diagnostics::framework::runtime::VIEW_CONTAINER_QUERY_UNSUPPORTED,
        breakpoints = breakpoints.len() as u64;
        "the @container rules of the element whose classes are given at {at} {why}: none of \
         them apply"
    );
    None
}

impl StyleSite {
    #[doc(hidden)]
    pub const fn new(rules: &'static [SiteRule]) -> Self {
        Self {
            rules,
            composed: Mutex::new(None),
        }
    }

    /// Whether any rule of the site is guarded by an `@container` query.
    fn has_queries(&self) -> bool {
        self.rules.iter().any(|(_, _, query)| query.is_some())
    }

    /// The base layout equal to `base` that this site has composed from
    /// before, or `base` itself. A binding that keeps its base holds this
    /// one: every instance then shares it, and composing finds it by
    /// pointer instead of comparing whole layouts.
    fn shared_base(&self, base: &Arc<LayoutStyle>) -> Arc<LayoutStyle> {
        let mut composed = self.composed.lock().unwrap_or_else(PoisonError::into_inner);
        composed
            .get_or_insert_with(Composed::default)
            .canonical(base)
    }

    /// `base` with the patches that apply under `active`, in order. A rule
    /// guarded by an `@container` query is left out: it applies through
    /// [`Self::responsive`].
    pub fn compose(&self, base: &Arc<LayoutStyle>, active: u64) -> Arc<LayoutStyle> {
        let mut guard = self.composed.lock().unwrap_or_else(PoisonError::into_inner);
        let composed = guard.get_or_insert_with(Composed::default);
        let base = composed.canonical(base);
        let layout = self.compose_in(composed, &base, active);
        composed.prune();
        layout
    }

    /// [`Self::compose`] over a canonical `base`, with the site's state held.
    fn compose_in(
        &self,
        composed: &mut Composed,
        base: &Arc<LayoutStyle>,
        active: u64,
    ) -> Arc<LayoutStyle> {
        let key = (Arc::as_ptr(base) as usize, active);
        if let Some((known, layout)) = composed.by_base.get(&key)
            && known
                .upgrade()
                .is_some_and(|known| Arc::ptr_eq(&known, base))
        {
            return Arc::clone(layout);
        }
        let applying = self
            .rules
            .iter()
            .filter(|(needs, _, query)| query.is_none() && needs & !active == 0)
            .map(|(_, patch, _)| patch.value());
        let layout = Arc::new(apply(base, applying));
        composed
            .by_base
            .insert(key, (Arc::downgrade(base), Arc::clone(&layout)));
        layout
    }

    /// The rule the site's `@container` rules give an element over `base`
    /// under `active`: one bucket per stretch of the container's extent
    /// between the bounds their queries use, each the composition with the
    /// guarded rules that hold there written over the authored one, in
    /// cascade order. `None` when no guarded rule applies under `active`,
    /// when none changes anything, or when they cannot make one rule: they
    /// ask more than one container or axis, or use more than
    /// [`MAX_RESPONSIVE_BREAKPOINTS`] bounds. The last is a diagnostic,
    /// naming where the element's classes were given (`at`). One rule per
    /// base and active set: an equal call returns the same one.
    pub(crate) fn responsive(
        &self,
        base: &Arc<LayoutStyle>,
        active: u64,
        at: &'static Location<'static>,
    ) -> Option<Arc<ResponsiveRule>> {
        if !self.has_queries() {
            return None;
        }
        let mut guard = self.composed.lock().unwrap_or_else(PoisonError::into_inner);
        let composed = guard.get_or_insert_with(Composed::default);
        let base = composed.canonical(base);
        let key = (Arc::as_ptr(&base) as usize, active);
        if let Some((known, rule)) = composed.responsive.get(&key)
            && known
                .upgrade()
                .is_some_and(|known| Arc::ptr_eq(&known, &base))
        {
            return rule.clone();
        }
        let rule = self.plan_in(composed, &base, active, at).map(Arc::new);
        composed
            .responsive
            .insert(key, (Arc::downgrade(&base), rule.clone()));
        composed.prune();
        rule
    }

    /// [`Self::responsive`], uncached.
    fn plan_in(
        &self,
        composed: &mut Composed,
        base: &Arc<LayoutStyle>,
        active: u64,
        at: &'static Location<'static>,
    ) -> Option<ResponsiveRule> {
        let applying: Vec<&SiteRule> = self
            .rules
            .iter()
            .filter(|(needs, _, _)| needs & !active == 0)
            .collect();
        let queries: Vec<&SheetQuery> = applying.iter().filter_map(|rule| rule.2).collect();
        let (name, axis, breakpoints) = plan(&queries, at)?;
        let authored = self.compose_in(composed, base, active);
        let variants: Vec<Option<StyleVariant>> = (0..=breakpoints.len())
            .map(|bucket| {
                // The bucket's lower bound decides: no query changes inside
                // a bucket.
                let lower = bucket
                    .checked_sub(1)
                    .map_or(f32::NEG_INFINITY, |below| breakpoints[below]);
                let styled = apply(
                    base,
                    applying
                        .iter()
                        .filter(|(_, _, query)| query.is_none_or(|query| query.holds_at(lower)))
                        .map(|(_, patch, _)| patch.value()),
                );
                StyleVariant::between(&authored, &styled)
            })
            .collect();
        if variants.iter().all(Option::is_none) {
            return None;
        }
        ResponsiveRule::from_buckets(
            ResponsiveContainer::Nearest {
                name: name.map(str::to_owned),
            },
            axis,
            breakpoints,
            variants,
        )
    }

    #[cfg(test)]
    fn composed_entries(&self) -> usize {
        self.composed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or(0, |composed| composed.by_base.len())
    }
}

/// A stylesheet compiled at build time (`stylesheet!`, a view's `<style>`):
/// its class rules and transitions, each in cascade order.
pub struct Sheet {
    rules: &'static [SheetRule],
    transitions: &'static [SheetTransition],
    /// `(fixed classes, conditional classes) -> (site, animations)`.
    resolved: Mutex<Vec<Resolved>>,
}

type Resolved = (Vec<u16>, Vec<u16>, &'static StyleSite, &'static [Implicit]);

/// One rule: the classes its selector needs, its patch, and the
/// `@container` query of the block it is in (`None` outside one).
#[doc(hidden)]
pub struct SheetRule {
    pub classes: &'static [u16],
    pub patch: &'static StylePatch,
    pub query: Option<&'static SheetQuery>,
}

/// One `transition`: the classes its selector needs and what it animates.
#[doc(hidden)]
pub struct SheetTransition {
    pub classes: &'static [u16],
    pub animate: &'static [Implicit],
}

impl Sheet {
    #[doc(hidden)]
    pub const fn new(rules: &'static [SheetRule], transitions: &'static [SheetTransition]) -> Self {
        Self {
            rules,
            transitions,
            resolved: Mutex::new(Vec::new()),
        }
    }

    /// The rules an element with these classes can match, with the bit of
    /// each conditional class they need, and the winning transition among
    /// those its fixed classes match.
    fn resolve(
        &self,
        fixed: &[u16],
        conditional: &[u16],
    ) -> (&'static StyleSite, &'static [Implicit]) {
        let mut resolved = self.resolved.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, _, site, animate)) = resolved
            .iter()
            .find(|(f, c, _, _)| f.as_slice() == fixed && c.as_slice() == conditional)
        {
            return (site, animate);
        }
        let bit = |class: &u16| {
            conditional
                .iter()
                .position(|c| c == class)
                .map_or(0, |at| 1u64 << at)
        };
        let rules: Vec<SiteRule> = self
            .rules
            .iter()
            .filter(|rule| {
                rule.classes
                    .iter()
                    .all(|class| fixed.contains(class) || conditional.contains(class))
            })
            .map(|rule| {
                let needs = rule
                    .classes
                    .iter()
                    .filter(|class| !fixed.contains(class))
                    .fold(0, |needs, class| needs | bit(class));
                (needs, rule.patch, rule.query)
            })
            .collect();
        // One per distinct class set an element is written with: bounded
        // by the program's source, not by how many nodes it builds.
        let site: &'static StyleSite = Box::leak(Box::new(StyleSite::new(Box::leak(
            rules.into_boxed_slice(),
        ))));
        let animate = self
            .transitions
            .iter()
            .rfind(|t| t.classes.iter().all(|class| fixed.contains(class)))
            .map_or(&[][..], |t| t.animate);
        resolved.push((fixed.to_vec(), conditional.to_vec(), site, animate));
        (site, animate)
    }
}

/// One class of a [`Sheet`], as `stylesheet!` names it: `styles::card`.
#[derive(Clone, Copy)]
pub struct Class {
    sheet: &'static Sheet,
    index: u16,
}

impl Class {
    #[doc(hidden)]
    pub const fn new(sheet: &'static Sheet, index: u16) -> Self {
        Self { sheet, index }
    }
}

/// The classes [`El::class`] and [`El::class_when`] gave an element.
pub(crate) struct Classes<C> {
    sheet: &'static Sheet,
    fixed: Vec<u16>,
    conditional: Vec<(u16, PropSource<bool>)>,
    apply: ApplySite<C>,
    at: &'static Location<'static>,
}

type ApplySite<C> = fn(
    &mut C,
    &mut NodeBindings<C>,
    &'static StyleSite,
    Vec<PropSource<bool>>,
    &'static Location<'static>,
);

impl<C> Classes<C> {
    /// Write the element's styles into it, and return its animations.
    pub(crate) fn apply(
        self,
        component: &mut C,
        bindings: &mut NodeBindings<C>,
    ) -> &'static [Implicit] {
        let indices: Vec<u16> = self.conditional.iter().map(|(class, _)| *class).collect();
        let (site, animate) = self.sheet.resolve(&self.fixed, &indices);
        let sources = self
            .conditional
            .into_iter()
            .map(|(_, source)| source)
            .collect();
        (self.apply)(component, bindings, site, sources, self.at);
        animate
    }
}

/// `site` over the component's layout as built; the `i`-th condition
/// switches the site's `i`-th conditional class. A site with `@container`
/// rules also gives the element its responsive rule under the active
/// classes, kept in `bindings` for the node to send.
fn apply_site<C: StyledComponent + ComponentView>(
    component: &mut C,
    bindings: &mut NodeBindings<C>,
    site: &'static StyleSite,
    conditions: Vec<PropSource<bool>>,
    at: &'static Location<'static>,
) {
    let base = &component.node_style().layout;
    let queried = site.has_queries();
    if conditions.is_empty() {
        if queried {
            bindings.responsive = Some(ResponsiveBinding::new(site.responsive(base, 0, at)));
        }
        let composed = site.compose(base, 0);
        <ComposedLayout as FieldWrite<C, Arc<LayoutStyle>>>::write(component, composed);
        return;
    }
    let base = site.shared_base(base);
    // The binding computes the active classes once per run, for both.
    let pending = queried.then(|| {
        let binding = ResponsiveBinding::default();
        let pending = Arc::clone(&binding.pending);
        bindings.responsive = Some(binding);
        pending
    });
    let composed = move || {
        let active = conditions
            .iter()
            .enumerate()
            .fold(0u64, |mask, (bit, class)| match class.get() {
                true => mask | 1 << bit,
                false => mask,
            });
        if let Some(pending) = &pending {
            let rule = site.responsive(&base, active, at);
            *pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(rule);
        }
        site.compose(&base, active)
    };
    composed.bind_field::<C, ComposedLayout>(component, bindings, at);
}

/// The responsive rule an element's compiled `@container` rules give it,
/// sent beside its other changes: in the commit that builds the node, then
/// whenever the active classes give another one.
#[derive(Default)]
pub(crate) struct ResponsiveBinding {
    /// What the last evaluation of the element's classes gave and is not
    /// sent yet; shared with that binding.
    pending: Arc<Mutex<Option<Option<Arc<ResponsiveRule>>>>>,
    /// What was sent last.
    sent: Option<Arc<ResponsiveRule>>,
}

impl ResponsiveBinding {
    fn new(rule: Option<Arc<ResponsiveRule>>) -> Self {
        Self {
            pending: Arc::new(Mutex::new(Some(rule))),
            sent: None,
        }
    }

    /// The rule to send in the commit that creates the node: what the
    /// first evaluation gave, the absence of a rule included, so a node
    /// built again over one that had a rule loses it.
    pub(crate) fn initial(&mut self) -> Option<Option<Arc<ResponsiveRule>>> {
        let rule = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()?;
        self.sent.clone_from(&rule);
        Some(rule)
    }

    /// Queue the rule the last evaluation gave when it differs from the one
    /// sent. A cached rule is the same `Arc`, so a run that keeps the active
    /// classes compares one pointer.
    pub(crate) fn send(&mut self, node: StableNodeId, mutations: &mut MutationQueue) {
        let Some(rule) = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let same = match (&rule, &self.sent) {
            (Some(rule), Some(sent)) => Arc::ptr_eq(rule, sent) || rule == sent,
            (None, None) => true,
            _ => false,
        };
        if !same {
            mutations.set_responsive(node, rule.clone());
            self.sent = rule;
        }
    }
}

fn apply<'a>(base: &LayoutStyle, patches: impl Iterator<Item = &'a Value>) -> LayoutStyle {
    let mut value = serde_json::to_value(base).expect("a layout serializes");
    for patch in patches {
        let Value::Object(fields) = patch else {
            continue;
        };
        for (path, field) in fields {
            *field_mut(&mut value, path) = field.clone();
        }
    }
    serde_json::from_value(value).expect("a patched layout deserializes")
}

/// The field a patch key names: a dotted path through the Style Model's
/// structs (`paint.outline.width`). The field itself, an enum or an
/// `Option` included, is replaced whole: merging a `{"Px": 12}` into a
/// `{"Percent": 50}` would be neither.
fn field_mut<'a>(layout: &'a mut Value, path: &str) -> &'a mut Value {
    path.split('.').fold(layout, |value, key| {
        if !value.is_object() {
            *value = Value::Object(Default::default());
        }
        value
            .as_object_mut()
            .expect("made an object above")
            .entry(key)
            .or_insert(Value::Null)
    })
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
    /// Give the element `class` of a stylesheet:
    ///
    /// ```ignore
    /// stylesheet! {
    ///     mod styles;
    ///     .card { padding: 12px; }
    ///     .card.done { opacity: 0.5; }
    /// }
    /// widget(card).class(styles::card).class_when(styles::done, done)
    /// ```
    ///
    /// Every class of one element comes from one sheet. Its rules apply
    /// over the layout the element was built with, in the sheet's cascade
    /// order; a `transition` on its fixed classes animates what bindings
    /// change.
    #[track_caller]
    pub fn class(self, class: Class) -> Self {
        self.class_at(class, Location::caller())
    }

    /// `class` while `condition` holds (Vue's `:class="{ done: … }"`).
    #[track_caller]
    pub fn class_when(self, class: Class, condition: impl IntoProp<bool>) -> Self {
        self.class_when_at(class, condition.into_source(), Location::caller())
    }

    fn class_at(self, class: Class, at: &'static Location<'static>) -> Self {
        self.with_classes(class, at, |classes, index| classes.fixed.push(index))
    }

    fn class_when_at(
        self,
        class: Class,
        condition: PropSource<bool>,
        at: &'static Location<'static>,
    ) -> Self {
        self.with_classes(class, at, move |classes, index| {
            if classes.conditional.len() < 64 {
                classes.conditional.push((index, condition));
            }
        })
    }

    fn with_classes(
        mut self,
        class: Class,
        at: &'static Location<'static>,
        add: impl FnOnce(&mut Classes<C>, u16),
    ) -> Self {
        let classes = self.classes_mut().get_or_insert_with(|| Classes {
            sheet: class.sheet,
            fixed: Vec::new(),
            conditional: Vec::new(),
            apply: apply_site::<C>,
            at,
        });
        assert!(
            std::ptr::eq(classes.sheet, class.sheet),
            "an element's classes come from one stylesheet"
        );
        add(classes, class.index);
        self
    }

    /// A compiled site over the element's layout as built, now (`css!`).
    fn styles(mut self, site: &'static StyleSite, at: &'static Location<'static>) -> Self {
        let (component, bindings) = self.parts_mut();
        apply_site(component, bindings, site, Vec::new(), at);
        self
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
    /// A declaration block compiled by `css!`: `column().gap(8.0).children(rows)
    /// .css(css! { padding: 12px; transition: opacity 150ms })`. Call it
    /// before other layout props.
    #[track_caller]
    pub fn css(self, style: InlineStyle) -> Self {
        self.css_at(style, Location::caller())
    }

    fn css_at(self, style: InlineStyle, at: &'static Location<'static>) -> Self {
        let styled = match style.site {
            Some(site) => self.styles(site, at),
            None => self,
        };
        styled.animate(style.animate.iter().copied())
    }
}

/// What [`El::class`], [`El::class_when`] and [`El::css`] give the
/// container of a structural view (`each`, `when`, `dynamic`,
/// `each_virtual`), kept until the view builds its container.
#[derive(Default)]
pub(crate) struct ContainerStyle(Vec<ContainerStyleOp>);

enum ContainerStyleOp {
    Class(Class, &'static Location<'static>),
    ClassWhen(Class, PropSource<bool>, &'static Location<'static>),
    Css(InlineStyle, &'static Location<'static>),
    Visible(PropSource<bool>, &'static Location<'static>),
}

impl ContainerStyle {
    /// `element` with these styles, in the order they were given.
    pub(crate) fn apply<C: StyledComponent + ComponentView>(self, element: El<C>) -> El<C> {
        self.0.into_iter().fold(element, |element, op| match op {
            ContainerStyleOp::Class(class, at) => element.class_at(class, at),
            ContainerStyleOp::ClassWhen(class, condition, at) => {
                element.class_when_at(class, condition, at)
            }
            ContainerStyleOp::Css(style, at) => element.css_at(style, at),
            ContainerStyleOp::Visible(visible, at) => element.visible_at(visible, at),
        })
    }
}

/// `.class`, `.class_when`, `.css` and `.visible` on a structural view: they
/// style the container its rows or branches are built in, as they style an
/// element.
macro_rules! container_styles {
    ($([$($generics:tt)*] $view:ty),* $(,)?) => {$(
        impl<$($generics)*> $view {
            /// Give the container the rows or branches are built in `class`
            /// of a stylesheet, as [`El::class`] gives an element one.
            #[track_caller]
            pub fn class(mut self, class: $crate::view::Class) -> Self {
                self.container_style_mut().push_class(class, ::std::panic::Location::caller());
                self
            }

            /// `class` on the container while `condition` holds.
            #[track_caller]
            pub fn class_when(
                mut self,
                class: $crate::view::Class,
                condition: impl $crate::view::IntoProp<bool>,
            ) -> Self {
                self.container_style_mut().push_class_when(
                    class,
                    condition.into_source(),
                    ::std::panic::Location::caller(),
                );
                self
            }

            /// A `css!` block on the container, as [`El::css`] on an
            /// element.
            #[track_caller]
            pub fn css(mut self, style: $crate::view::InlineStyle) -> Self {
                self.container_style_mut().push_css(style, ::std::panic::Location::caller());
                self
            }

            /// Keep the container, and what is built in it, but take it out
            /// of layout, paint and hit testing while `visible` is false
            /// (`v-show`), as [`El::visible`] does an element's.
            #[track_caller]
            pub fn visible(mut self, visible: impl $crate::view::IntoProp<bool>) -> Self {
                self.container_style_mut().push_visible(
                    visible.into_source(),
                    ::std::panic::Location::caller(),
                );
                self
            }
        }
    )*};
}
pub(crate) use container_styles;

impl ContainerStyle {
    pub(crate) fn push_class(&mut self, class: Class, at: &'static Location<'static>) {
        self.0.push(ContainerStyleOp::Class(class, at));
    }

    pub(crate) fn push_class_when(
        &mut self,
        class: Class,
        condition: PropSource<bool>,
        at: &'static Location<'static>,
    ) {
        self.0
            .push(ContainerStyleOp::ClassWhen(class, condition, at));
    }

    pub(crate) fn push_css(&mut self, style: InlineStyle, at: &'static Location<'static>) {
        self.0.push(ContainerStyleOp::Css(style, at));
    }

    pub(crate) fn push_visible(
        &mut self,
        visible: PropSource<bool>,
        at: &'static Location<'static>,
    ) {
        self.0.push(ContainerStyleOp::Visible(visible, at));
    }
}

#[cfg(test)]
mod site_tests {
    use super::*;
    use nana_ui_core::LengthSpec;

    static SITE: StyleSite = StyleSite::new(&[]);
    static SHARED: StyleSite = StyleSite::new(&[]);

    /// Equal bases at different addresses share one composition; bases no
    /// instance holds any more do not keep theirs alive for ever.
    #[test]
    fn a_site_shares_equal_bases_and_forgets_dropped_ones() {
        let one = Arc::new(LayoutStyle::default());
        let other = Arc::new(LayoutStyle::default());
        assert!(Arc::ptr_eq(
            &SHARED.compose(&one, 0),
            &SHARED.compose(&other, 0)
        ));

        for width in 0..500 {
            let base = Arc::new(LayoutStyle {
                width: Some(nana_ui_core::LengthSpec::Px(width as f32)),
                ..LayoutStyle::default()
            });
            let _ = SITE.compose(&base, 0);
        }
        assert!(
            SITE.composed_entries() <= 128,
            "{} entries for bases nothing holds",
            SITE.composed_entries()
        );
    }

    static ROW: StylePatch = StylePatch::new(r#"{"height":{"Px":20.0}}"#);
    static NARROW: StylePatch = StylePatch::new(r#"{"height":{"Px":40.0},"opacity":0.5}"#);
    static LATER: StylePatch = StylePatch::new(r#"{"height":{"Px":30.0}}"#);
    /// `@container card (width < 300px)`.
    static CARD: SheetQuery = SheetQuery::new(
        Some("card"),
        ResponsiveAxis::Width,
        &[(f32::NEG_INFINITY, 300.0)],
    );
    static QUERIED: StyleSite = StyleSite::new(&[
        (0, &ROW, None),
        (0, &NARROW, Some(&CARD)),
        (1, &LATER, None),
    ]);

    fn here() -> &'static Location<'static> {
        Location::caller()
    }

    /// Composing leaves a guarded rule out; the responsive rule carries it
    /// in the buckets where its query holds, written over the authored
    /// composition, and a later plain rule still wins over it there.
    #[test]
    fn guarded_rules_become_buckets_and_a_later_plain_rule_still_wins() {
        let base = Arc::new(LayoutStyle::default());
        let authored = QUERIED.compose(&base, 0);
        assert_eq!(authored.height, Some(LengthSpec::Px(20.0)));
        assert_eq!(authored.opacity, None, "the guarded rule is not composed");

        let rule = QUERIED.responsive(&base, 0, here()).expect("a rule");
        assert_eq!(
            rule.container(),
            &ResponsiveContainer::Nearest {
                name: Some("card".into())
            }
        );
        assert_eq!(rule.axis(), ResponsiveAxis::Width);
        assert_eq!(rule.bucket_for(299.9), 0);
        assert_eq!(rule.bucket_for(300.0), 1, "`<` leaves its bound out");
        let mut narrow = (*authored).clone();
        rule.variant(0).expect("below 300").apply(&mut narrow);
        assert_eq!(narrow.height, Some(LengthSpec::Px(40.0)));
        assert_eq!(narrow.opacity, Some(0.5));
        assert!(rule.variant(1).is_none(), "the authored style above");
        assert!(
            Arc::ptr_eq(&rule, &QUERIED.responsive(&base, 0, here()).unwrap()),
            "one rule per base and active set"
        );

        let on = QUERIED.responsive(&base, 1, here()).expect("a rule");
        let mut styled = (*QUERIED.compose(&base, 1)).clone();
        assert_eq!(styled.height, Some(LengthSpec::Px(30.0)));
        on.variant(0).expect("below 300").apply(&mut styled);
        assert_eq!(styled.height, Some(LengthSpec::Px(30.0)), "the later rule");
        assert_eq!(styled.opacity, Some(0.5));
    }

    static ELSEWHERE: SheetQuery =
        SheetQuery::new(None, ResponsiveAxis::Width, &[(f32::NEG_INFINITY, 100.0)]);
    static TWO_CONTAINERS: StyleSite =
        StyleSite::new(&[(0, &NARROW, Some(&CARD)), (1, &LATER, Some(&ELSEWHERE))]);
    static SEVENTEEN: SheetQuery = SheetQuery::new(
        None,
        ResponsiveAxis::Inline,
        &[
            (0.0, 1.0),
            (2.0, 3.0),
            (4.0, 5.0),
            (6.0, 7.0),
            (8.0, 9.0),
            (10.0, 11.0),
            (12.0, 13.0),
            (14.0, 15.0),
            (16.0, f32::INFINITY),
        ],
    );
    static TOO_MANY: StyleSite = StyleSite::new(&[(0, &NARROW, Some(&SEVENTEEN))]);
    static SAME: StyleSite = StyleSite::new(&[(0, &ROW, None), (0, &ROW, Some(&CARD))]);

    /// An element follows one container on one axis, through at most 16
    /// breakpoints; past that its guarded rules make no rule. Nor does a
    /// guarded rule that changes nothing.
    #[test]
    fn guarded_rules_that_cannot_share_one_plan_make_no_rule() {
        let base = Arc::new(LayoutStyle::default());
        assert!(TWO_CONTAINERS.responsive(&base, 0, here()).is_some());
        assert!(
            TWO_CONTAINERS.responsive(&base, 1, here()).is_none(),
            "two containers once the class is on"
        );
        assert!(TOO_MANY.responsive(&base, 0, here()).is_none());
        assert!(SAME.responsive(&base, 0, here()).is_none());
        assert!(
            SITE.responsive(&base, 0, here()).is_none(),
            "no guarded rule"
        );
    }
}
