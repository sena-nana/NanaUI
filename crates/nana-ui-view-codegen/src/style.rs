//! A view's styles, compiled with the CSS engine while the application
//! builds: the `.vue` compiler's `<style>`, `view!`'s `style = "…"`, and
//! `css!`.
//!
//! Rules whose selector is a class or a compound of classes (`.card`,
//! `.card.active`) are matched against each element's `class="…"` and
//! `class:name="condition"` here, in cascade order (`!important`,
//! specificity, source order). Each rule becomes a patch: the Style Model
//! fields its declarations set, found by applying them with `nana-ui-css`
//! to a default layout. The running view receives the patches and which
//! conditional classes each needs; it parses no CSS and matches no
//! selector. `transition` becomes the element's implicit animations.
//!
//! Everything else in the sheet (other selectors, `:hover` and the other
//! interactive states, `@media`, `@keyframes`, a declaration the Style
//! Model has no field for) is a warning, not silently dropped.

use std::collections::BTreeMap;

use crate::{Attr, AttrName, AttrValue, Element, Node};
use nana_ui_core::{Easing, LayoutStyle};
use nana_ui_css::css_motion::{
    css_list_at, easing_from_css_keyword, first_timing_token, parse_css_time_token,
    parse_transition_shorthand, split_css_comma_list,
};
use nana_ui_css::{
    CompoundSelector, DeclarationEntry, LayoutStyleCss, MotionDeclarations, Selector, Specificity,
    collect_document_custom_properties_from_rules, parse_stylesheet_full,
};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use serde_json::{Map, Value};
use syn::{Expr, Lit};

/// Where a rule stands in the cascade: later wins.
type Rank = (bool, (u16, u16, u16), u32);

struct Rule {
    classes: Vec<String>,
    rank: Rank,
    /// The patch's JSON.
    patch: String,
}

struct TransitionRule {
    classes: Vec<String>,
    rank: Rank,
    /// `(property, milliseconds, easing)`; property names the
    /// `AnimatableProperty` variant.
    items: Vec<(&'static str, f32, Easing)>,
}

pub(crate) struct Sheet {
    rules: Vec<Rule>,
    transitions: Vec<TransitionRule>,
    pub(crate) warnings: Vec<String>,
}

/// The classes of a class-only selector; `None` for any other.
fn class_selector(selector: &Selector) -> Option<&[String]> {
    let classes_only = CompoundSelector {
        classes: selector.subject.classes.clone(),
        ..CompoundSelector::default()
    };
    (selector.ancestors.is_empty()
        && !selector.subject.classes.is_empty()
        && selector.subject == classes_only)
        .then_some(selector.subject.classes.as_slice())
}

fn rank(important: bool, specificity: Specificity, order: u32) -> Rank {
    (
        important,
        (
            specificity.ids,
            specificity.classes_attrs,
            specificity.types,
        ),
        order,
    )
}

/// The fields of `value` that differ from `default`, recursively.
fn diff(value: &Value, default: &Value) -> Option<Value> {
    match (value, default) {
        (Value::Object(value), Value::Object(default)) => {
            let changed: Map<String, Value> = value
                .iter()
                .filter_map(|(key, field)| {
                    let base = default.get(key).unwrap_or(&Value::Null);
                    diff(field, base).map(|field| (key.clone(), field))
                })
                .collect();
            (!changed.is_empty()).then_some(Value::Object(changed))
        }
        (value, default) => (value != default).then(|| value.clone()),
    }
}

fn layout_json(layout: &LayoutStyle) -> Value {
    serde_json::to_value(layout).expect("a layout serializes")
}

/// The CSS properties an implicit animation can follow, as
/// `AnimatableProperty` variants.
fn animatable(property: &str) -> &'static [&'static str] {
    match property.trim().to_ascii_lowercase().as_str() {
        "all" => &["Opacity", "Transform", "Width", "Height", "Background"],
        "opacity" => &["Opacity"],
        "transform" => &["Transform"],
        "width" => &["Width"],
        "height" => &["Height"],
        "background" | "background-color" => &["Background"],
        _ => &[],
    }
}

fn transition_items(
    motion: &MotionDeclarations,
    warnings: &mut Vec<String>,
) -> Vec<(&'static str, f32, Easing)> {
    let shorthand = motion
        .transition
        .as_deref()
        .and_then(parse_transition_shorthand);
    let pick = |longhand: &Option<String>, short: Option<&String>| {
        longhand
            .clone()
            .or_else(|| short.cloned())
            .unwrap_or_default()
    };
    let properties = pick(
        &motion.transition_property,
        shorthand.as_ref().map(|s| &s.property),
    );
    let durations = split_css_comma_list(&pick(
        &motion.transition_duration,
        shorthand.as_ref().map(|s| &s.duration),
    ));
    let timings = split_css_comma_list(&pick(
        &motion.transition_timing_function,
        shorthand.as_ref().map(|s| &s.timing_function),
    ));
    let mut items = Vec::new();
    for (index, property) in split_css_comma_list(&properties).iter().enumerate() {
        let duration = if durations.is_empty() {
            0.0
        } else {
            parse_css_time_token(css_list_at(&durations, index)).unwrap_or(0.0)
        };
        let easing = if timings.is_empty() {
            easing_from_css_keyword("ease")
        } else {
            easing_from_css_keyword(
                &first_timing_token(css_list_at(&timings, index)).to_ascii_lowercase(),
            )
        };
        let variants = animatable(property);
        if variants.is_empty() {
            warnings.push(format!(
                "`transition: {property}` is not animated: only opacity, transform, width, \
                 height and background are"
            ));
        }
        items.extend(variants.iter().map(|variant| (*variant, duration, easing)));
    }
    items
}

pub(crate) fn parse(css: &str) -> Sheet {
    let (sheet, _) = parse_stylesheet_full(css, 0);
    let mut warnings = Vec::new();
    let skipped = |what: &str, count: usize, warnings: &mut Vec<String>| {
        if count > 0 {
            warnings.push(format!(
                "{count} {what} in `<style>` are not compiled for L3 views and are ignored"
            ));
        }
    };
    skipped(
        "`:hover` / `:focus` / `:active` rules",
        sheet.interactive_rules.len(),
        &mut warnings,
    );
    skipped("`@media` blocks", sheet.media_rules.len(), &mut warnings);
    skipped("`@keyframes`", sheet.keyframes.len(), &mut warnings);
    skipped("`@font-face` rules", sheet.font_faces.len(), &mut warnings);
    skipped(
        "pseudo-element rules",
        sheet.generated_pseudo_rules.len() + sheet.scrollbar_pseudo_rules.len(),
        &mut warnings,
    );
    // `var()` resolves against the sheet's own custom properties: a view's
    // style is a build-time constant.
    let vars: BTreeMap<String, String> =
        collect_document_custom_properties_from_rules(&sheet.static_rules, "light");
    let default = layout_json(&LayoutStyle::default());
    let mut rules = Vec::new();
    for rule in &sheet.static_rules {
        let selectors: Vec<&Selector> = rule.selectors.iter().collect();
        let (normal, important): (Vec<&DeclarationEntry>, Vec<&DeclarationEntry>) = rule
            .declaration_entries
            .iter()
            .filter(|entry| !entry.property.starts_with("--"))
            .partition(|entry| !entry.important);
        if normal.is_empty() && important.is_empty() {
            continue;
        }
        for (important, entries) in [(false, normal), (true, important)] {
            if entries.is_empty() {
                continue;
            }
            let (layout, inert) = nana_ui_css::css_map::with_active_css_vars(&vars, || {
                let mut layout = LayoutStyle::default();
                let mut inert = Vec::new();
                for entry in &entries {
                    let mut alone = LayoutStyle::default();
                    alone.apply_css_property(&entry.property, &entry.value, None, None);
                    if layout_json(&alone) == default {
                        inert.push(entry.text());
                    }
                    layout.apply_css_property(&entry.property, &entry.value, None, None);
                }
                (layout, inert)
            });
            for declaration in inert {
                warnings.push(format!(
                    "`{declaration}` sets nothing in the Style Model and is ignored"
                ));
            }
            let Some(patch) = diff(&layout_json(&layout), &default) else {
                continue;
            };
            for selector in &selectors {
                match class_selector(selector) {
                    Some(classes) => rules.push(Rule {
                        classes: classes.to_vec(),
                        rank: rank(important, selector.specificity, rule.source_order),
                        patch: patch.to_string(),
                    }),
                    None => warnings.push(format!(
                        "a rule for `{}` is ignored: L3 views compile class selectors \
                         (`.a`, `.a.b`) only",
                        rule.declarations.trim()
                    )),
                }
            }
        }
    }
    let mut transitions = Vec::new();
    for rule in &sheet.motion_rules {
        if rule.motion.animation.is_some() || rule.motion.animation_name.is_some() {
            warnings.push("`animation` is not compiled for L3 views and is ignored".into());
        }
        let items = transition_items(&rule.motion, &mut warnings);
        if items.is_empty() {
            continue;
        }
        for selector in &rule.selectors {
            match class_selector(selector) {
                Some(classes) => transitions.push(TransitionRule {
                    classes: classes.to_vec(),
                    rank: rank(false, selector.specificity, rule.source_order),
                    items: items.clone(),
                }),
                None => warnings.push(
                    "a `transition` rule is ignored: L3 views compile class selectors only".into(),
                ),
            }
        }
    }
    Sheet {
        rules,
        transitions,
        warnings,
    }
}

fn easing_tokens(easing: Easing, runtime: &TokenStream) -> TokenStream {
    match easing {
        Easing::Linear => quote!(#runtime::Easing::Linear),
        Easing::EaseOutCubic => quote!(#runtime::Easing::EaseOutCubic),
        Easing::EaseInOutCubic => quote!(#runtime::Easing::EaseInOutCubic),
        Easing::CubicBezier([a, b, c, d]) => {
            quote!(#runtime::Easing::CubicBezier([#a, #b, #c, #d]))
        }
    }
}

/// Built-in elements that are blocks around other elements, not nodes that
/// hold a style.
const BLOCKS: &[&str] = &[
    "Virtual",
    "Transition",
    "TransitionGroup",
    "KeepAlive",
    "Suspense",
    "Teleport",
    "ErrorBoundary",
];

/// Compiles the classes of a view's elements against its sheet.
pub(crate) struct Styler<'a> {
    pub(crate) sheet: &'a Sheet,
    pub(crate) runtime: &'a TokenStream,
    /// Patch JSON → its static's index.
    patches: BTreeMap<String, usize>,
    sites: usize,
    pub(crate) items: Vec<TokenStream>,
    pub(crate) warnings: Vec<String>,
}

impl<'a> Styler<'a> {
    pub(crate) fn new(sheet: &'a Sheet, runtime: &'a TokenStream) -> Self {
        Self {
            sheet,
            runtime,
            patches: BTreeMap::new(),
            sites: 0,
            items: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub(crate) fn nodes(&mut self, nodes: &mut [Node]) {
        for node in nodes {
            if let Node::Element(element) = node {
                self.element(element);
            }
        }
    }

    fn element(&mut self, element: &mut Element) {
        self.nodes(&mut element.children);
        for attr in &mut element.attrs {
            if let AttrValue::View(slot) = &mut attr.value {
                self.nodes(slot);
            }
        }
        let tag = element.name.to_string();
        let mut fixed: Vec<String> = Vec::new();
        let mut conditional: Vec<(String, Expr)> = Vec::new();
        let mut kept = Vec::new();
        for attr in std::mem::take(&mut element.attrs) {
            match (&attr.name, &attr.value) {
                (AttrName::Plain(name), AttrValue::Lit(Expr::Lit(literal))) if name == "class" => {
                    if let Lit::Str(text) = &literal.lit {
                        fixed.extend(text.value().split_whitespace().map(str::to_owned));
                    }
                }
                (AttrName::Plain(name), _) if name == "class" => {
                    self.warnings.push(format!(
                        "`<{tag}>`: a bound `:class` is not compiled; write \
                         `class:name=\"condition\"` for each class"
                    ));
                }
                (AttrName::Directive(directive, _), AttrValue::Expr(condition))
                    if directive.starts_with("class:") =>
                {
                    let class = directive["class:".len()..].to_owned();
                    conditional.push((class, condition.clone()));
                }
                _ => kept.push(attr),
            }
        }
        element.attrs = kept;
        if fixed.is_empty() && conditional.is_empty() {
            return;
        }
        if !crate::is_builtin(&tag) || BLOCKS.contains(&tag.as_str()) {
            self.warnings.push(format!(
                "`<{tag}>` takes no class: style the elements inside it"
            ));
            return;
        }
        if conditional.len() > 64 {
            self.warnings
                .push(format!("`<{tag}>` has more than 64 conditional classes"));
            return;
        }
        let bit = |class: &str| {
            conditional
                .iter()
                .position(|(name, _)| name == class)
                .map(|at| 1u64 << at)
        };
        let applies = |classes: &[String]| {
            classes
                .iter()
                .all(|class| fixed.contains(class) || bit(class).is_some())
        };
        let mask = |classes: &[String]| {
            classes
                .iter()
                .filter(|class| !fixed.contains(class))
                .filter_map(|class| bit(class))
                .fold(0u64, |mask, bit| mask | bit)
        };
        let mut matched: Vec<&Rule> = self
            .sheet
            .rules
            .iter()
            .filter(|rule| applies(&rule.classes))
            .collect();
        matched.sort_by_key(|rule| rule.rank);
        for class in fixed
            .iter()
            .chain(conditional.iter().map(|(class, _)| class))
        {
            let known = self
                .sheet
                .rules
                .iter()
                .any(|rule| rule.classes.contains(class))
                || self
                    .sheet
                    .transitions
                    .iter()
                    .any(|rule| rule.classes.contains(class));
            if !known {
                self.warnings.push(format!(
                    "`<{tag}>`: no rule in `<style>` uses class `{class}`"
                ));
            }
        }
        let runtime = self.runtime;
        let mut injected = Vec::new();
        if !matched.is_empty() {
            let mut entries = Vec::new();
            for rule in matched {
                let next = self.patches.len();
                let index = *self.patches.entry(rule.patch.clone()).or_insert(next);
                if index == next {
                    let name = format_ident!("__NANA_PATCH_{index}");
                    let json = &rule.patch;
                    self.items.push(quote! {
                        static #name: #runtime::view::StylePatch =
                            #runtime::view::StylePatch::new(#json);
                    });
                }
                let name = format_ident!("__NANA_PATCH_{index}");
                let needs = mask(&rule.classes);
                entries.push(quote!((#needs, &#name)));
            }
            let site = format_ident!("__NANA_STYLE_{}", self.sites);
            self.sites += 1;
            self.items.push(quote! {
                static #site: #runtime::view::StyleSite =
                    #runtime::view::StyleSite::new(&[#(#entries),*]);
            });
            let conditions = conditional.iter().map(|(_, condition)| match condition {
                Expr::Path(_) => {
                    quote!(#runtime::view::IntoProp::<bool>::into_source(#condition))
                }
                condition => {
                    quote!(#runtime::view::IntoProp::<bool>::into_source(move || #condition))
                }
            });
            injected.push(Attr {
                name: AttrName::Directive("styles".into(), Span::call_site()),
                value: AttrValue::Verbatim(quote!(&#site, ::std::vec![#(#conditions),*])),
            });
        }
        // `transition` is single-valued: the winning rule among those that
        // hold whatever the conditions.
        let transition = self
            .sheet
            .transitions
            .iter()
            .filter(|rule| rule.classes.iter().all(|class| fixed.contains(class)))
            .max_by_key(|rule| rule.rank);
        if self.sheet.transitions.iter().any(|rule| {
            applies(&rule.classes) && !rule.classes.iter().all(|class| fixed.contains(class))
        }) {
            self.warnings.push(format!(
                "`<{tag}>`: a `transition` behind a conditional class is not compiled; put it \
                 on a class the element always has"
            ));
        }
        if let Some(transition) = transition {
            let items = transition.items.iter().map(|(property, ms, easing)| {
                let property = format_ident!("{property}");
                let easing = easing_tokens(*easing, runtime);
                let nanos = (f64::from(*ms) * 1_000_000.0).round() as u64;
                quote! {
                    #runtime::view::Implicit::new(
                        #runtime::AnimatableProperty::#property,
                        ::std::time::Duration::from_nanos(#nanos),
                    )
                    .ease(#easing)
                }
            });
            injected.push(Attr {
                name: AttrName::Directive("animate".into(), Span::call_site()),
                value: AttrValue::Verbatim(quote!([#(#items),*])),
            });
        }
        // Compiled styles go first, so later layout props land on top.
        injected.append(&mut element.attrs);
        element.attrs = injected;
    }
}

/// What compiling a view's styles adds: statics to put before the view's
/// code, and warnings about what was not compiled.
pub struct CompiledStyles {
    pub items: Vec<TokenStream>,
    pub warnings: Vec<String>,
}

/// Compile `css` against the elements of `nodes`: each element's
/// `class="…"` and `class:name="…"` become compiled styles and implicit
/// animations, and are removed from the element.
pub fn compile_styles(css: &str, nodes: &mut [Node], runtime: &TokenStream) -> CompiledStyles {
    let sheet = parse(css);
    let mut styler = Styler::new(&sheet, runtime);
    styler.nodes(nodes);
    let mut warnings = styler.warnings;
    warnings.extend(sheet.warnings.iter().cloned());
    CompiledStyles {
        items: styler.items,
        warnings,
    }
}

/// `css!("padding: 8px; transition: opacity 120ms")`: one declaration
/// block as a compiled style, an expression of type
/// `view::InlineStyle` for `El::css`.
pub fn compile_inline(declarations: &str, runtime: &TokenStream) -> (TokenStream, Vec<String>) {
    let css = format!(".__nana_inline {{ {declarations} }}");
    let sheet = parse(&css);
    let mut element = Element {
        name: syn::Ident::new("Widget", Span::call_site()),
        attrs: vec![Attr {
            name: AttrName::Plain(syn::Ident::new("class", Span::call_site())),
            value: AttrValue::Lit(syn::parse_quote!("__nana_inline")),
        }],
        children: Vec::new(),
    };
    let mut styler = Styler::new(&sheet, runtime);
    styler.element(&mut element);
    let mut warnings = styler.warnings;
    warnings.extend(sheet.warnings.iter().cloned());
    let find = |name: &str| {
        element
            .attrs
            .iter()
            .find_map(|attr| match (&attr.name, &attr.value) {
                (AttrName::Directive(directive, _), AttrValue::Verbatim(tokens))
                    if directive == name =>
                {
                    Some(tokens.clone())
                }
                _ => None,
            })
    };
    let site = match find("styles") {
        // `&SITE, vec![]`: the site alone.
        Some(tokens) => {
            let site = tokens.into_iter().take_while(|token| {
                !matches!(token, proc_macro2::TokenTree::Punct(punct) if punct.as_char() == ',')
            });
            let site: TokenStream = site.collect();
            quote!(::core::option::Option::Some(#site))
        }
        None => quote!(::core::option::Option::None),
    };
    let animate = find("animate").unwrap_or_else(|| quote!([]));
    let items = styler.items;
    (
        quote! {{
            #(#items)*
            static __NANA_ANIMATE: &[#runtime::view::Implicit] = &#animate;
            #runtime::view::InlineStyle::new(#site, __NANA_ANIMATE)
        }},
        warnings,
    )
}
