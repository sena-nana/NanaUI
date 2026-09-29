//! A view's styles, compiled with the CSS engine while the application
//! builds: the `.vue` compiler's `<style>`, `view!`'s `<style>`, and
//! `css!`.
//!
//! Rules whose selector is a class or a compound of classes (`.card`,
//! `.card.active`) are matched against each element's `class="…"` and
//! `class:name="condition"` here, in cascade order (`!important`,
//! specificity, source order). Each rule becomes a patch: the Style Model
//! fields its declarations write and their values, as `nana-ui-css`
//! reports them ([`written_layout`]), including a field written with the
//! value the default layout has. The running view receives the patches and which
//! conditional classes each needs; it parses no CSS and matches no
//! selector. `transition` becomes the element's implicit animations.
//!
//! Everything else in the sheet (other selectors, `:hover` and the other
//! interactive states, `@media`, `@keyframes`, a declaration the Style
//! Model has no field for) is a warning, not silently dropped. Each warning
//! points at what it is about: a range of the sheet's text, or the element
//! or `class` attribute in the template.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::{Attr, AttrName, AttrValue, Element, Node};
use nana_ui_core::Easing;
use nana_ui_css::css_motion::{
    css_list_at, easing_from_css_keyword, first_timing_token, parse_css_time_token,
    parse_transition_shorthand, split_css_comma_list,
};
use nana_ui_css::{
    CompoundSelector, DeclarationEntry, LayoutStyleCss, MotionDeclarations, Selector, Specificity,
    collect_document_custom_properties_from_rules, parse_stylesheet_full, written_layout,
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
    pub(crate) warnings: Vec<StyleWarning>,
}

/// What a style warning is about.
#[derive(Debug, Clone)]
pub enum StyleAt {
    /// A byte range of the sheet's text: a selector, an at-rule, a
    /// declaration.
    Sheet(Range<usize>),
    /// An element of the template, or its `class` attribute.
    Template(Span),
}

#[derive(Debug, Clone)]
pub struct StyleWarning {
    pub at: StyleAt,
    pub message: String,
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

/// The custom properties `var()` resolves against: the sheet's own, so a
/// view's style is a build-time constant.
fn sheet_vars(sheet: &nana_ui_css::ParsedStylesheet) -> BTreeMap<String, String> {
    collect_document_custom_properties_from_rules(&sheet.static_rules, "light")
}

/// The Style Model fields `entries` write, as a patch: the value of each,
/// keyed by its dotted path. A field written with the value the default
/// layout has is still in it, so the patch resets what an element was
/// built with.
fn patch(entries: &[&DeclarationEntry]) -> Map<String, Value> {
    written_layout(|layout| {
        for entry in entries {
            layout.apply_css_property(&entry.property, &entry.value, None, None);
        }
    })
    .patch()
}

pub(crate) fn parse(css: &str) -> Sheet {
    let (sheet, _) = parse_stylesheet_full(css, 0);
    let vars = sheet_vars(&sheet);
    let mut rules = Vec::new();
    for rule in &sheet.static_rules {
        let (normal, important): (Vec<&DeclarationEntry>, Vec<&DeclarationEntry>) = rule
            .declaration_entries
            .iter()
            .filter(|entry| !entry.property.starts_with("--"))
            .partition(|entry| !entry.important);
        for (important, entries) in [(false, normal), (true, important)] {
            if entries.is_empty() {
                continue;
            }
            let patch = nana_ui_css::css_map::with_active_css_vars(&vars, || patch(&entries));
            if patch.is_empty() {
                continue;
            }
            let patch = Value::Object(patch);
            for selector in &rule.selectors {
                if let Some(classes) = class_selector(selector) {
                    rules.push(Rule {
                        classes: classes.to_vec(),
                        rank: rank(important, selector.specificity, rule.source_order),
                        patch: patch.to_string(),
                    });
                }
            }
        }
    }
    let mut transitions = Vec::new();
    for rule in &sheet.motion_rules {
        let items = transition_items(&rule.motion, &mut Vec::new());
        if items.is_empty() {
            continue;
        }
        for selector in &rule.selectors {
            if let Some(classes) = class_selector(selector) {
                transitions.push(TransitionRule {
                    classes: classes.to_vec(),
                    rank: rank(false, selector.specificity, rule.source_order),
                    items: items.clone(),
                });
            }
        }
    }
    Sheet {
        rules,
        transitions,
        warnings: check(css, &vars),
    }
}

/// One top-level block of a sheet, `prelude { body }` or `prelude;`, as
/// byte ranges of the sheet: the prelude trimmed, and for a rule each
/// `;`-separated segment of its body trimmed (empty ones included, as the
/// parser counts them).
struct Block {
    range: Range<usize>,
    prelude: Range<usize>,
    segments: Vec<Range<usize>>,
}

/// Skip a comment or a quoted string starting at `at`; the index after it,
/// or `None` when `at` starts neither.
fn skip_opaque(css: &[u8], at: usize) -> Option<usize> {
    match css[at] {
        b'/' if css.get(at + 1) == Some(&b'*') => Some(
            css[at + 2..]
                .windows(2)
                .position(|pair| pair == b"*/")
                .map_or(css.len(), |end| at + 2 + end + 2),
        ),
        quote @ (b'"' | b'\'') => {
            let mut index = at + 1;
            while index < css.len() && css[index] != quote {
                index += if css[index] == b'\\' { 2 } else { 1 };
            }
            Some((index + 1).min(css.len()))
        }
        _ => None,
    }
}

/// `range` without surrounding whitespace and leading comments.
fn trim(css: &[u8], mut range: Range<usize>) -> Range<usize> {
    loop {
        while range.start < range.end && css[range.start].is_ascii_whitespace() {
            range.start += 1;
        }
        if range.start + 1 < range.end && css[range.start] == b'/' && css[range.start + 1] == b'*' {
            range.start = skip_opaque(css, range.start)
                .unwrap_or(range.end)
                .min(range.end);
            continue;
        }
        break;
    }
    while range.end > range.start && css[range.end - 1].is_ascii_whitespace() {
        range.end -= 1;
    }
    range
}

fn outline(css: &str) -> Vec<Block> {
    let bytes = css.as_bytes();
    let mut blocks = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        let mut depth = 0usize;
        // The prelude: up to `{` or `;` outside brackets.
        let mut stop = None;
        while at < bytes.len() {
            if let Some(next) = skip_opaque(bytes, at) {
                at = next;
                continue;
            }
            match bytes[at] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth = depth.saturating_sub(1),
                b'{' | b';' if depth == 0 => {
                    stop = Some(bytes[at]);
                    break;
                }
                _ => {}
            }
            at += 1;
        }
        let prelude = trim(bytes, start..at);
        if stop != Some(b'{') {
            at = (at + 1).min(bytes.len());
            if !prelude.is_empty() {
                blocks.push(Block {
                    range: prelude.clone(),
                    prelude,
                    segments: Vec::new(),
                });
            }
            continue;
        }
        let body = at + 1;
        at = body;
        let mut braces = 1usize;
        let mut parens = 0usize;
        let mut segments = Vec::new();
        let mut segment = body;
        while at < bytes.len() {
            if let Some(next) = skip_opaque(bytes, at) {
                at = next;
                continue;
            }
            match bytes[at] {
                b'{' => braces += 1,
                b'}' => {
                    braces -= 1;
                    if braces == 0 {
                        break;
                    }
                }
                b'(' | b'[' => parens += 1,
                b')' | b']' => parens = parens.saturating_sub(1),
                b';' if braces == 1 && parens == 0 => {
                    segments.push(trim(bytes, segment..at));
                    segment = at + 1;
                }
                _ => {}
            }
            at += 1;
        }
        let end = at.min(bytes.len());
        let last = trim(bytes, segment..end);
        if !last.is_empty() {
            segments.push(last);
        }
        if bytes.get(prelude.start) == Some(&b'@') {
            segments.clear();
        }
        at = (end + 1).min(bytes.len());
        blocks.push(Block {
            range: start..at,
            prelude,
            segments,
        });
    }
    blocks
}

/// The property a declaration segment sets, lowercased.
fn property_of(text: &str) -> String {
    text.split(':')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// Each block's warnings, found by parsing it alone and pointed at its
/// prelude or the declaration they are about.
fn check(css: &str, vars: &BTreeMap<String, String>) -> Vec<StyleWarning> {
    let mut warnings = Vec::new();
    for block in outline(css) {
        let (sheet, _) = parse_stylesheet_full(&css[block.range.clone()], 0);
        let prelude = &css[block.prelude.clone()];
        let mut warn = |range: Range<usize>, message: String| {
            warnings.push(StyleWarning {
                at: StyleAt::Sheet(range),
                message,
            })
        };
        let declaration = |index: Option<u32>, matches: &dyn Fn(&str) -> bool| {
            index
                .and_then(|index| block.segments.get(index as usize))
                .filter(|segment| matches(&property_of(&css[(*segment).clone()])))
                .or_else(|| {
                    block
                        .segments
                        .iter()
                        .find(|segment| matches(&property_of(&css[(*segment).clone()])))
                })
                .cloned()
                .unwrap_or_else(|| block.prelude.clone())
        };
        let skipped = [
            (
                !sheet.interactive_rules.is_empty(),
                "`:hover` / `:focus` / `:active` rules",
            ),
            (!sheet.media_rules.is_empty(), "`@media` blocks"),
            (!sheet.keyframes.is_empty(), "`@keyframes`"),
            (!sheet.font_faces.is_empty(), "`@font-face` rules"),
            (
                !sheet.generated_pseudo_rules.is_empty()
                    || !sheet.scrollbar_pseudo_rules.is_empty(),
                "pseudo-element rules",
            ),
        ];
        for (skipped, what) in skipped {
            if skipped {
                warn(
                    block.prelude.clone(),
                    format!("{what} are not compiled for L3 views; `{prelude}` is ignored"),
                );
            }
        }
        for rule in &sheet.static_rules {
            let entries: Vec<&DeclarationEntry> = rule
                .declaration_entries
                .iter()
                .filter(|entry| !entry.property.starts_with("--"))
                .collect();
            if entries.is_empty() {
                continue;
            }
            if rule.selectors.iter().any(|s| class_selector(s).is_none()) {
                warn(
                    block.prelude.clone(),
                    format!(
                        "the rule for `{prelude}` is ignored: L3 views compile class \
                         selectors (`.a`, `.a.b`) only"
                    ),
                );
            }
            nana_ui_css::css_map::with_active_css_vars(vars, || {
                for entry in entries {
                    if patch(&[entry]).is_empty() {
                        let property = entry.property.to_ascii_lowercase();
                        warn(
                            declaration(Some(entry.index), &|p| p == property),
                            format!(
                                "`{}` sets nothing in the Style Model and is ignored",
                                entry.text()
                            ),
                        );
                    }
                }
            });
        }
        for rule in &sheet.motion_rules {
            if rule.motion.animation.is_some() || rule.motion.animation_name.is_some() {
                warn(
                    declaration(None, &|p| p.starts_with("animation")),
                    "`animation` is not compiled for L3 views and is ignored".into(),
                );
            }
            let mut messages = Vec::new();
            let items = transition_items(&rule.motion, &mut messages);
            let at = declaration(None, &|p| p.starts_with("transition"));
            for message in messages {
                warn(at.clone(), message);
            }
            if !items.is_empty() && rule.selectors.iter().any(|s| class_selector(s).is_none()) {
                warn(
                    block.prelude.clone(),
                    format!(
                        "the `transition` for `{prelude}` is ignored: L3 views compile class \
                         selectors only"
                    ),
                );
            }
        }
    }
    warnings
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

/// Blocks that build no node of their own to style. The others (`Block`,
/// `Virtual`, `Transition`, `TransitionGroup`, `KeepAlive`) style the
/// container of the list or chain they hold.
const BLOCKS: &[&str] = &["Suspense", "Teleport", "ErrorBoundary"];

/// A sheet's rules and transitions as the runtime's `view::Sheet`: patch
/// statics, then the sheet, both in cascade order. `classes` numbers every
/// class a rule or transition names.
struct SheetItems {
    classes: Vec<String>,
    items: Vec<TokenStream>,
}

impl Sheet {
    fn class_index(classes: &mut Vec<String>, class: &str) -> u16 {
        match classes.iter().position(|known| known == class) {
            Some(at) => at as u16,
            None => {
                classes.push(class.to_owned());
                (classes.len() - 1) as u16
            }
        }
    }

    /// The statics of this sheet, `name` the sheet's.
    fn items(&self, name: &syn::Ident, runtime: &TokenStream) -> SheetItems {
        let mut classes = Vec::new();
        let mut patches: BTreeMap<&str, usize> = BTreeMap::new();
        let mut items = Vec::new();
        let mut ordered: Vec<&Rule> = self.rules.iter().collect();
        ordered.sort_by_key(|rule| rule.rank);
        let mut rules = Vec::new();
        for rule in ordered {
            let next = patches.len();
            let index = *patches.entry(&rule.patch).or_insert(next);
            let patch = format_ident!("{name}_PATCH_{index}");
            if index == next {
                let json = &rule.patch;
                items.push(quote! {
                    static #patch: #runtime::view::StylePatch =
                        #runtime::view::StylePatch::new(#json);
                });
            }
            let needs = rule
                .classes
                .iter()
                .map(|class| Self::class_index(&mut classes, class));
            rules.push(quote! {
                #runtime::view::SheetRule { classes: &[#(#needs),*], patch: &#patch }
            });
        }
        let mut ordered: Vec<&TransitionRule> = self.transitions.iter().collect();
        ordered.sort_by_key(|rule| rule.rank);
        let mut transitions = Vec::new();
        for rule in ordered {
            let needs = rule
                .classes
                .iter()
                .map(|class| Self::class_index(&mut classes, class));
            let animate = animate_tokens(&rule.items, runtime);
            transitions.push(quote! {
                #runtime::view::SheetTransition { classes: &[#(#needs),*], animate: &#animate }
            });
        }
        items.push(quote! {
            static #name: #runtime::view::Sheet =
                #runtime::view::Sheet::new(&[#(#rules),*], &[#(#transitions),*]);
        });
        SheetItems { classes, items }
    }
}

fn animate_tokens(items: &[(&'static str, f32, Easing)], runtime: &TokenStream) -> TokenStream {
    let items = items.iter().map(|(property, ms, easing)| {
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
    quote!([#(#items),*])
}

/// Turns the classes of a view's elements into `.class(…)` and
/// `.class_when(…)` on its sheet, the calls the Rust spelling writes.
pub(crate) struct Styler<'a> {
    sheet: &'a Sheet,
    name: syn::Ident,
    runtime: &'a TokenStream,
    classes: Vec<String>,
    /// Whether an element uses the sheet, so its statics are needed.
    used: bool,
    pub(crate) warnings: Vec<StyleWarning>,
}

impl<'a> Styler<'a> {
    fn new(
        sheet: &'a Sheet,
        classes: Vec<String>,
        name: syn::Ident,
        runtime: &'a TokenStream,
    ) -> Self {
        Self {
            sheet,
            name,
            runtime,
            classes,
            used: false,
            warnings: Vec::new(),
        }
    }

    fn warn(&mut self, span: Span, message: String) {
        self.warnings.push(StyleWarning {
            at: StyleAt::Template(span),
            message,
        });
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
        let tag = element.tag();
        let at_element = element.name.span();
        let mut fixed: Vec<(String, Span)> = Vec::new();
        let mut conditional: Vec<(String, Expr, Span)> = Vec::new();
        let mut kept = Vec::new();
        for attr in std::mem::take(&mut element.attrs) {
            match (&attr.name, &attr.value) {
                (AttrName::Plain(name), AttrValue::Lit(Expr::Lit(literal))) if name == "class" => {
                    if let Lit::Str(text) = &literal.lit {
                        for class in text.value().split_whitespace() {
                            fixed.push((class.to_owned(), name.span()));
                        }
                    }
                }
                (AttrName::Plain(name), _) if name == "class" => {
                    self.warn(
                        name.span(),
                        format!(
                            "`<{tag}>`: a bound `:class` is not compiled; write \
                             `class:name=\"condition\"` for each class"
                        ),
                    );
                }
                (AttrName::Directive(directive, span), AttrValue::Expr(condition))
                    if directive.starts_with("class:") =>
                {
                    let class = directive["class:".len()..].to_owned();
                    conditional.push((class, condition.clone(), *span));
                }
                _ => kept.push(attr),
            }
        }
        element.attrs = kept;
        if fixed.is_empty() && conditional.is_empty() {
            return;
        }
        if !crate::is_builtin(&tag) || BLOCKS.contains(&tag.as_str()) {
            self.warn(
                at_element,
                format!("`<{tag}>` takes no class: style the elements inside it"),
            );
            return;
        }
        if conditional.len() > 64 {
            self.warn(
                at_element,
                format!("`<{tag}>` has more than 64 conditional classes"),
            );
            return;
        }
        let holds = |classes: &[String], conditional_too: bool| {
            classes.iter().all(|class| {
                fixed.iter().any(|(name, _)| name == class)
                    || (conditional_too && conditional.iter().any(|(name, _, _)| name == class))
            })
        };
        if self
            .sheet
            .transitions
            .iter()
            .any(|rule| holds(&rule.classes, true) && !holds(&rule.classes, false))
        {
            self.warn(
                at_element,
                format!(
                    "`<{tag}>`: a `transition` behind a conditional class is not compiled; put \
                     it on a class the element always has"
                ),
            );
        }
        let runtime = self.runtime;
        let name = self.name.clone();
        let mut injected = Vec::new();
        let index = |classes: &[String], class: &str| {
            classes
                .iter()
                .position(|known| known == class)
                .map(|at| at as u16)
        };
        for (class, span) in fixed {
            match index(&self.classes, &class) {
                Some(at) => injected.push(Attr {
                    name: AttrName::Directive("class".into(), span),
                    value: AttrValue::Verbatim(quote!(#runtime::view::Class::new(&#name, #at))),
                }),
                None => self.warn(
                    span,
                    format!("`<{tag}>`: no rule in `<style>` uses class `{class}`"),
                ),
            }
        }
        for (class, condition, span) in conditional {
            let Some(at) = index(&self.classes, &class) else {
                self.warn(
                    span,
                    format!("`<{tag}>`: no rule in `<style>` uses class `{class}`"),
                );
                continue;
            };
            let condition = match condition {
                Expr::Path(_) => quote!(#condition),
                condition => quote!(move || #condition),
            };
            injected.push(Attr {
                name: AttrName::Directive("class_when".into(), span),
                value: AttrValue::Verbatim(
                    quote!(#runtime::view::Class::new(&#name, #at), #condition),
                ),
            });
        }
        self.used |= !injected.is_empty();
        injected.append(&mut element.attrs);
        element.attrs = injected;
    }
}

/// What compiling a view's styles adds: statics to put before the view's
/// code, and warnings about what was not compiled.
pub struct CompiledStyles {
    pub items: Vec<TokenStream>,
    pub warnings: Vec<StyleWarning>,
}

/// Compile `css` against the elements of `nodes`: each element's
/// `class="…"` and `class:name="…"` become `.class(…)` and
/// `.class_when(…)` on the view's sheet, and are removed from the element.
pub fn compile_styles(css: &str, nodes: &mut [Node], runtime: &TokenStream) -> CompiledStyles {
    let sheet = parse(css);
    let name = format_ident!("__NANA_SHEET");
    let SheetItems { classes, items } = sheet.items(&name, runtime);
    let mut styler = Styler::new(&sheet, classes, name, runtime);
    styler.nodes(nodes);
    let mut warnings = sheet.warnings.clone();
    warnings.extend(styler.warnings);
    CompiledStyles {
        items: if styler.used { items } else { Vec::new() },
        warnings,
    }
}

/// `stylesheet! { mod name; … }`: a module holding the sheet and one
/// `view::Class` constant per class, named as the class with `-` as `_`.
/// `class_at` gives the span a class is written at: an unused class then
/// warns there.
pub fn compile_stylesheet(
    css: &str,
    module: &syn::Ident,
    public: bool,
    runtime: &TokenStream,
    class_at: &dyn Fn(&str) -> Span,
) -> (TokenStream, Vec<StyleWarning>) {
    let sheet = parse(css);
    let name = format_ident!("SHEET");
    let SheetItems { classes, items } = sheet.items(&name, runtime);
    let visibility = if public {
        quote!(pub)
    } else {
        quote!(pub(super))
    };
    let constants = classes.iter().enumerate().map(|(index, class)| {
        let ident = class.replace('-', "_");
        let span = class_at(class);
        let ident = match syn::parse_str::<syn::Ident>(&ident) {
            Ok(_) => syn::Ident::new(&ident, span),
            Err(_) => syn::Ident::new_raw(&ident, span),
        };
        let index = index as u16;
        quote! {
            #visibility const #ident: #runtime::view::Class = #runtime::view::Class::new(&#name, #index);
        }
    });
    let module_visibility = if public { quote!(pub) } else { quote!() };
    (
        quote! {
            #[allow(non_upper_case_globals)]
            #module_visibility mod #module {
                #(#items)*
                #(#constants)*
            }
        },
        sheet.warnings.clone(),
    )
}

/// `css! { padding: 8px; transition: opacity 120ms }`: one declaration
/// block as a compiled style, an expression of type
/// `view::InlineStyle` for `El::css`. Warnings point into `declarations`.
pub fn compile_inline(
    declarations: &str,
    runtime: &TokenStream,
) -> (TokenStream, Vec<StyleWarning>) {
    const OPEN: &str = ".__nana_inline { ";
    let css = format!("{OPEN}{declarations} }}");
    let sheet = parse(&css);
    // The wrapper is not the author's: what points at it points at the
    // whole block.
    let inner = |range: Range<usize>| {
        let clamp = |at: usize| at.saturating_sub(OPEN.len()).min(declarations.len());
        let range = clamp(range.start)..clamp(range.end);
        if range.is_empty() {
            0..declarations.len()
        } else {
            range
        }
    };
    let warnings = sheet
        .warnings
        .iter()
        .cloned()
        .map(|warning| StyleWarning {
            at: match warning.at {
                StyleAt::Sheet(range) => StyleAt::Sheet(inner(range)),
                StyleAt::Template(_) => StyleAt::Sheet(0..declarations.len()),
            },
            message: warning.message,
        })
        .collect();
    let mut rules: Vec<&Rule> = sheet.rules.iter().collect();
    rules.sort_by_key(|rule| rule.rank);
    let patches = rules.iter().enumerate().map(|(index, rule)| {
        let name = format_ident!("__NANA_PATCH_{index}");
        let json = &rule.patch;
        quote! {
            static #name: #runtime::view::StylePatch = #runtime::view::StylePatch::new(#json);
        }
    });
    let entries = (0..rules.len()).map(|index| {
        let name = format_ident!("__NANA_PATCH_{index}");
        quote!((0u64, &#name))
    });
    let site = if rules.is_empty() {
        quote!(::core::option::Option::None)
    } else {
        quote! {{
            static __NANA_SITE: #runtime::view::StyleSite =
                #runtime::view::StyleSite::new(&[#(#entries),*]);
            ::core::option::Option::Some(&__NANA_SITE)
        }}
    };
    let animate = sheet
        .transitions
        .iter()
        .max_by_key(|rule| rule.rank)
        .map_or_else(|| quote!([]), |rule| animate_tokens(&rule.items, runtime));
    (
        quote! {{
            #(#patches)*
            static __NANA_ANIMATE: &[#runtime::view::Implicit] = &#animate;
            #runtime::view::InlineStyle::new(#site, __NANA_ANIMATE)
        }},
        warnings,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each sheet warning as the text it points at and its message.
    fn located(css: &str) -> Vec<(&str, String)> {
        parse(css)
            .warnings
            .into_iter()
            .map(|warning| match warning.at {
                StyleAt::Sheet(range) => (&css[range], warning.message),
                StyleAt::Template(_) => unreachable!("a sheet alone has no template"),
            })
            .collect()
    }

    fn at<'a>(warnings: &[(&'a str, String)], needle: &str) -> &'a str {
        warnings
            .iter()
            .find(|(_, message)| message.contains(needle))
            .unwrap_or_else(|| panic!("no warning about {needle}: {warnings:?}"))
            .0
    }

    #[test]
    fn each_warning_points_at_what_it_is_about() {
        let warnings = located(
            "/* lead */ .a { padding: 4px; frobnicate: 3; transition: color 1s; animation: spin 1s }\n\
             @media (min-width: 1px) { .b { opacity: 1 } }\n\
             .c::before { opacity: 1 }\n\
             .d:hover { opacity: 0.5; }\n\
             .e > .f { opacity: 1; }",
        );
        assert_eq!(at(&warnings, "frobnicate"), "frobnicate: 3");
        assert_eq!(at(&warnings, "not animated"), "transition: color 1s");
        assert_eq!(at(&warnings, "`animation`"), "animation: spin 1s");
        assert_eq!(at(&warnings, "@media"), "@media (min-width: 1px)");
        assert_eq!(at(&warnings, "pseudo-element"), ".c::before");
        assert_eq!(at(&warnings, ":hover"), ".d:hover");
        assert_eq!(at(&warnings, "class selectors"), ".e > .f");
        assert_eq!(warnings.len(), 7, "{warnings:?}");
    }

    #[test]
    fn a_clean_sheet_warns_of_nothing() {
        let css =
            ".a { padding: 4px 8px; background: url(\"x;{y}.png\"); }\n.a.b { opacity: 0.5; }";
        assert!(located(css).is_empty(), "{:?}", located(css));
    }

    #[test]
    fn the_outline_skips_strings_comments_and_brackets() {
        let css =
            ".a[title=\"}{;\"] { content: \"a;b\"; /* ; } */ padding: 1px }\n@import \"x.css\";";
        let blocks = outline(css);
        assert_eq!(blocks.len(), 2);
        assert_eq!(&css[blocks[0].prelude.clone()], ".a[title=\"}{;\"]");
        let segments: Vec<_> = blocks[0]
            .segments
            .iter()
            .map(|segment| &css[segment.clone()])
            .collect();
        assert_eq!(segments, ["content: \"a;b\"", "padding: 1px"]);
        assert_eq!(&css[blocks[1].prelude.clone()], "@import \"x.css\"");
    }
}
