//! Stylesheet parser and selector / declaration AST (CSS subset).
//!
//! **Adapter internals.** Application hosts should use [`crate::prelude`], not this
//! parser surface.
//!
//! Aligns with MDN / CSS Selectors Level 4 + CSS Cascade:
//! type / class / id / attribute (`[attr]`, `=`, `~=`, `|=`, `^=`, `$=`, `*=`,
//! optional `i`/`s`); combinators ` ` / `>` / `+` / `~`;
//! `:root` / `:first-child` / `:last-child` / `:only-child` /
//! `:nth-child()` / `:nth-of-type()` / `:nth-last-child()` /
//! `:first-of-type` / `:last-of-type` (An+B, `odd`/`even`);
//! simple `:not()` / `:is()` / `:where()`; author-layer `!important` tracking.
//!
//! Parsed into separate buckets: `:hover`/`:focus`/`:active`,
//! `::before`/`::after`, `@keyframes`, `@font-face`, and transition/animation
//! longhands — see [`crate::css_interactive::ParsedStylesheet`].
//!
//! `@import` / `@media` (width/height/orientation/prefers-color-scheme) /
//! `@font-face` / matching `@supports` / `@layer { }` (author source order,
//! names recorded) merge into the same [`ParsedStylesheet`].
//! `@container` blocks stay in [`ParsedStylesheet::container_rules`]
//! (see [`crate::css_container`]).

use std::collections::{BTreeMap, HashMap};

use crate::css_at_rule::{
    ImportPrelude, MAX_IMPORT_DEPTH, ParseStylesheetOptions, evaluate_supports_condition,
    font_face_from_pairs, is_blocked_href, parse_import_prelude, parse_layer_prelude,
    parse_media_query_list,
};

pub use crate::css_at_rule::MediaQuery;
use crate::css_container::{
    ContainerQueryUnsupported, ContainerRule, parse_container_prelude, take_nested_container_rules,
};
use crate::css_interactive::{
    GeneratedPseudo, GeneratedPseudoRule, InteractivePseudo, InteractiveSelector,
    InteractiveStyleRule, MediaRule, MotionDeclarations, MotionStyleRule, ParsedStylesheet,
    ScrollbarPseudo, ScrollbarPseudoRule, merge_parsed_stylesheet, offset_source_order,
    parse_keyframes_at_rule, partition_motion_entries,
};
use crate::css_map::split_important_flag;

/// One parsed declaration from a rule block (`property: value`, `!important` stripped).
///
/// Built once in [`parse_stylesheet`] and reused on every match so cascade does not
/// re-split the raw declaration string per node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclarationEntry {
    /// Index among `;`-separated segments in the original block (including empties).
    pub index: u32,
    pub important: bool,
    pub property: String,
    pub value: String,
}

impl DeclarationEntry {
    /// Reconstruct `property: value` (same shape as the former string-split path).
    pub fn text(&self) -> String {
        format!("{}: {}", self.property, self.value)
    }
}

/// One stylesheet rule after parse (selector list + declaration block).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleRule {
    pub selectors: Vec<Selector>,
    /// Raw declaration block text (diagnostics / debug; not re-parsed on match).
    pub declarations: String,
    /// Parsed once at stylesheet ingest; cascade hot path reads this only.
    pub declaration_entries: Vec<DeclarationEntry>,
    pub source_order: u32,
}

/// A single selector (compound chain with combinators toward the subject).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    /// Rightmost / subject compound (the element that receives declarations).
    pub subject: CompoundSelector,
    /// Leftward relatives toward the document start: stored
    /// outermost→…→closest-to-subject, each with the combinator linking it
    /// to the compound on its right.
    pub ancestors: Vec<(Combinator, CompoundSelector)>,
    pub specificity: Specificity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// Descendant combinator (` `).
    Descendant,
    /// Child combinator (`>`).
    Child,
    /// Adjacent sibling combinator (`+`).
    AdjacentSibling,
    /// Subsequent-sibling combinator (`~`).
    SubsequentSibling,
}

/// Simple compound used inside `:is()` / `:where()` / `:not()` argument lists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SimpleCompound {
    pub type_name: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSelector>,
    /// `:empty` / `:not(:empty)` — see [`MatchNode::is_empty`].
    pub empty: bool,
    /// `:checked` / `:not(:checked)` — see [`MatchNode::checked`].
    pub checked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompoundSelector {
    pub type_name: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSelector>,
    /// Negated simple compounds from `:not(...)` (must match none).
    pub not_alts: Vec<SimpleCompound>,
    /// `:is(...)` alternatives — match any; specificity = max arg (MDN).
    pub is_alts: Vec<SimpleCompound>,
    /// `:where(...)` alternatives — match any; specificity contribution 0.
    pub where_alts: Vec<SimpleCompound>,
    /// Structural / document pseudos.
    pub first_child: bool,
    pub last_child: bool,
    pub only_child: bool,
    pub first_of_type: bool,
    pub last_of_type: bool,
    /// `:empty` — see [`MatchContext::is_empty`].
    pub empty: bool,
    /// `:checked` — see [`MatchContext::checked`].
    pub checked: bool,
    /// `:root` — matches the document root (no parent in [`MatchContext`]).
    pub root: bool,
    /// `:nth-child(An+B)` — 1-based index among all siblings.
    pub nth_child: Option<AnPlusB>,
    /// `:nth-of-type(An+B)` — 1-based index among same-tag siblings.
    pub nth_of_type: Option<AnPlusB>,
    /// `:nth-last-child(An+B)` — 1-based index counting from the last sibling.
    pub nth_last_child: Option<AnPlusB>,
    /// `:hover` / `:focus` / `:active` when parsed for interactive buckets.
    pub interactive: Option<InteractivePseudo>,
    /// Cheap `:has()` descendant-present queries (OR inside each list, AND across
    /// lists). Combinators inside `:has()` fail parse.
    pub has_queries: Vec<Vec<SimpleCompound>>,
    /// Subject `:focus-within` — the element or a descendant has focus.
    pub focus_within: bool,
}

/// CSS An+B microsyntax (`odd`/`even`/`2n+1`/…) for `:nth-child` / `:nth-of-type`.
///
/// Matches 1-based sibling indices per MDN / Selectors Level 4: there exists
/// integer `n ≥ 0` such that `a·n + b = index`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnPlusB {
    pub a: i32,
    pub b: i32,
}

impl AnPlusB {
    pub const fn odd() -> Self {
        Self { a: 2, b: 1 }
    }

    pub const fn even() -> Self {
        Self { a: 2, b: 0 }
    }

    /// `index` is **1-based**. Index `0` never matches.
    pub fn matches_index(self, index: usize) -> bool {
        if index == 0 {
            return false;
        }
        let index = index as i32;
        if self.a == 0 {
            return index == self.b;
        }
        let an = index - self.b;
        an % self.a == 0 && an / self.a >= 0
    }
}

/// Attribute-selector comparison operator (MDN / Selectors Level 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrOperator {
    /// `[attr]` — attribute present (any value).
    Present,
    /// `[attr=value]` — exact value.
    Equal,
    /// `[attr~=value]` — whitespace-separated word list contains value.
    Includes,
    /// `[attr|=value]` — exact or `value-*` hyphen prefix.
    DashMatch,
    /// `[attr^=value]` — value is a prefix.
    Prefix,
    /// `[attr$=value]` — value is a suffix.
    Suffix,
    /// `[attr*=value]` — value is a substring.
    Substring,
}

/// Optional ASCII case flag before `]` (`i` / `s`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttrCase {
    /// Document-language default; values compared case-sensitively here.
    #[default]
    Default,
    /// Explicit `i` / `I` — ASCII case-insensitive value compare.
    Insensitive,
    /// Explicit `s` / `S` — ASCII case-sensitive value compare.
    Sensitive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrSelector {
    pub name: String,
    pub op: AttrOperator,
    /// Comparison operand; `None` only when [`AttrOperator::Present`].
    pub value: Option<String>,
    pub case: AttrCase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Specificity {
    pub ids: u16,
    pub classes_attrs: u16,
    pub types: u16,
}

impl Specificity {
    fn saturating_add_assign(&mut self, other: Specificity) {
        self.ids = self.ids.saturating_add(other.ids);
        self.classes_attrs = self.classes_attrs.saturating_add(other.classes_attrs);
        self.types = self.types.saturating_add(other.types);
    }
}

/// Skipped-content counters for one stylesheet parse. Lets callers surface
/// how much of a sheet was dropped (malformed blocks, unsupported selectors,
/// at-rules) instead of styles silently going missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StylesheetParseReport {
    pub rules: usize,
    /// Malformed blocks recovered by skipping to the next `}`.
    pub skipped_rules: usize,
    /// Rules dropped because no declaration survived parsing.
    pub skipped_declarations: usize,
    /// Selectors that failed to parse (deferred/unsupported syntax).
    pub skipped_selectors: usize,
    /// At-rule blocks skipped entirely (unknown `@supports` predicates, failed
    /// `@import`, unknown at-rules, `@container` blocks whose query is
    /// unsupported or nested in another). Applied `@supports` / `@layer` and
    /// supported `@container` blocks do not increment this.
    pub skipped_at_rules: usize,
    /// Successfully loaded `@import` stylesheets (cycle / depth / missing are skipped).
    pub imported_sheets: usize,
}

impl StylesheetParseReport {
    /// Sum two reports (accumulated across stylesheet injections).
    pub fn combine(self, other: Self) -> Self {
        Self {
            rules: self.rules + other.rules,
            skipped_rules: self.skipped_rules + other.skipped_rules,
            skipped_declarations: self.skipped_declarations + other.skipped_declarations,
            skipped_selectors: self.skipped_selectors + other.skipped_selectors,
            skipped_at_rules: self.skipped_at_rules + other.skipped_at_rules,
            imported_sheets: self.imported_sheets + other.imported_sheets,
        }
    }
}

/// Parse stylesheet text into static cascade rules (interactive/pseudo/keyframes omitted).
pub fn parse_stylesheet(css: &str, order_base: u32) -> Vec<StyleRule> {
    parse_stylesheet_with_report(css, order_base).0
}

/// Full parse: static rules plus interactive, generated-pseudo, keyframes, and motion buckets.
pub fn parse_stylesheet_full(
    css: &str,
    order_base: u32,
) -> (ParsedStylesheet, StylesheetParseReport) {
    parse_stylesheet_full_with_options(css, order_base, &mut ParseStylesheetOptions::default())
}

/// Compatibility wrapper: origin tracks `@layer` inside [`ParseStylesheetOptions`].
pub fn parse_stylesheet_full_with_layers(
    css: &str,
    order_base: u32,
    _layer_order: &mut Vec<String>,
) -> (ParsedStylesheet, StylesheetParseReport) {
    parse_stylesheet_full(css, order_base)
}

/// Like [`parse_stylesheet_full`], with `@import` loader / media environment.
pub fn parse_stylesheet_full_with_options(
    css: &str,
    order_base: u32,
    options: &mut ParseStylesheetOptions<'_>,
) -> (ParsedStylesheet, StylesheetParseReport) {
    let loader = options.loader;
    let base_href = options.base_href.map(str::to_string);
    let mut local_cache = HashMap::new();
    if let Some(cache) = options.import_cache.as_mut() {
        parse_stylesheet_full_with_cache(css, order_base, loader, base_href.as_deref(), cache)
    } else {
        parse_stylesheet_full_with_cache(
            css,
            order_base,
            loader,
            base_href.as_deref(),
            &mut local_cache,
        )
    }
}

fn parse_stylesheet_full_with_cache(
    css: &str,
    order_base: u32,
    loader: Option<&dyn crate::css_at_rule::StylesheetLoader>,
    base_href: Option<&str>,
    cache: &mut HashMap<String, ParsedStylesheet>,
) -> (ParsedStylesheet, StylesheetParseReport) {
    let stripped = strip_css_comments(css);
    let mut report = StylesheetParseReport::default();
    let mut sheet = ParsedStylesheet::default();
    let mut order = order_base;
    let mut stack = Vec::new();
    if let Some(href) = base_href {
        stack.push(href.to_string());
    }
    parse_stylesheet_into(
        &stripped,
        &mut order,
        &mut sheet,
        &mut report,
        &mut ImportParseCtx {
            loader,
            stack: &mut stack,
            cache,
            nesting: 0,
        },
        true,
    );
    (sheet, report)
}

struct ImportParseCtx<'a> {
    loader: Option<&'a dyn crate::css_at_rule::StylesheetLoader>,
    stack: &'a mut Vec<String>,
    cache: &'a mut HashMap<String, ParsedStylesheet>,
    /// At-rule blocks open around the text being parsed.
    nesting: usize,
}

/// Parses the body of a nested at-rule block, or skips it past
/// [`crate::MAX_NESTING`]: `false` when skipped.
fn parse_nested_block(
    css: &str,
    order: &mut u32,
    sheet: &mut ParsedStylesheet,
    report: &mut StylesheetParseReport,
    ctx: &mut ImportParseCtx<'_>,
) -> bool {
    if ctx.nesting >= crate::MAX_NESTING {
        report.skipped_at_rules += 1;
        return false;
    }
    ctx.nesting += 1;
    parse_stylesheet_into(css, order, sheet, report, ctx, false);
    ctx.nesting -= 1;
    true
}

/// `Some(rest)` when the at-rule was consumed; `None` means unknown.
fn parse_known_at_rule<'a>(
    rest: &'a str,
    order: &mut u32,
    sheet: &mut ParsedStylesheet,
    report: &mut StylesheetParseReport,
    ctx: &mut ImportParseCtx<'_>,
) -> Option<&'a str> {
    let (name, after_name) = at_rule_ident(rest)?;
    if name.eq_ignore_ascii_case("charset") {
        return Some(skip_at_rule(rest));
    }
    if name.eq_ignore_ascii_case("keyframes") || name.eq_ignore_ascii_case("-webkit-keyframes") {
        let (keyframes, next) = parse_keyframes_at_rule(rest, *order)?;
        sheet.keyframes.insert(keyframes.name.clone(), keyframes);
        *order = order.saturating_add(1);
        return Some(next);
    }
    if name.eq_ignore_ascii_case("import") {
        let (prelude, body, next) = split_at_rule_tail(after_name)?;
        if body.is_some() {
            report.skipped_at_rules += 1;
            return Some(next);
        }
        apply_import(prelude, order, sheet, report, ctx);
        return Some(next);
    }
    if name.eq_ignore_ascii_case("media") {
        let (prelude, body, next) = split_at_rule_tail(after_name)?;
        let Some(inner_css) = body else {
            report.skipped_at_rules += 1;
            return Some(next);
        };
        let query = parse_media_query_list(prelude);
        let mut inner = ParsedStylesheet::default();
        // Nested `@import` is invalid (CSS spec); reuse the late-import skip.
        if !parse_nested_block(inner_css, order, &mut inner, report, ctx) {
            return Some(next);
        }
        sheet.media_rules.push(MediaRule {
            query,
            sheet: inner,
        });
        return Some(next);
    }
    if name.eq_ignore_ascii_case("container") {
        let (prelude, body, next) = split_at_rule_tail(after_name)?;
        let Some(inner_css) = body else {
            report.skipped_at_rules += 1;
            return Some(next);
        };
        let query = parse_container_prelude(prelude);
        let mut inner = ParsedStylesheet::default();
        // Inner rules share the global order: they interleave with the rest
        // of the sheet in the cascade.
        if !parse_nested_block(inner_css, order, &mut inner, report, ctx) {
            return Some(next);
        }
        let nested = take_nested_container_rules(&mut inner);
        if query.is_err() {
            report.skipped_at_rules += 1;
        }
        sheet.container_rules.push(ContainerRule {
            query,
            sheet: inner,
        });
        // A block inside another would have to hold with it: not evaluated.
        for mut rule in nested {
            if rule.query.is_ok() {
                report.skipped_at_rules += 1;
                rule.query = Err(ContainerQueryUnsupported::Nested);
            }
            sheet.container_rules.push(rule);
        }
        return Some(next);
    }
    if name.eq_ignore_ascii_case("font-face") {
        let (_prelude, body, next) = split_at_rule_tail(after_name)?;
        let Some(inner_css) = body else {
            report.skipped_at_rules += 1;
            return Some(next);
        };
        let entries = parse_declaration_entries(inner_css);
        let pairs = entries
            .iter()
            .map(|e| (e.property.as_str(), e.value.as_str()));
        if let Some(mut face) = font_face_from_pairs(pairs) {
            face.base_href = ctx.stack.last().cloned();
            sheet.font_faces.push(face);
        } else {
            report.skipped_declarations += 1;
        }
        return Some(next);
    }
    if name.eq_ignore_ascii_case("supports") {
        let (prelude, body, next) = split_at_rule_tail(after_name)?;
        let Some(inner_css) = body else {
            report.skipped_at_rules += 1;
            return Some(next);
        };
        match evaluate_supports_condition(prelude) {
            Some(true) => {
                parse_nested_block(inner_css, order, sheet, report, ctx);
            }
            _ => {
                report.skipped_at_rules += 1;
            }
        }
        return Some(next);
    }
    if name.eq_ignore_ascii_case("layer") {
        let (prelude, body, next) = split_at_rule_tail(after_name)?;
        let Some(parsed) = parse_layer_prelude(prelude) else {
            report.skipped_at_rules += 1;
            return Some(next);
        };
        record_layer_names(sheet, &parsed.names);
        if let Some(inner_css) = body {
            parse_nested_block(inner_css, order, sheet, report, ctx);
        }
        return Some(next);
    }
    None
}

fn apply_import(
    prelude: &str,
    order: &mut u32,
    sheet: &mut ParsedStylesheet,
    report: &mut StylesheetParseReport,
    ctx: &mut ImportParseCtx<'_>,
) {
    let Some(parsed) = parse_import_prelude(prelude) else {
        report.skipped_at_rules += 1;
        return;
    };
    let (href, media) = match parsed {
        ImportPrelude::Unsupported => {
            report.skipped_at_rules += 1;
            return;
        }
        ImportPrelude::Ready { href, media } => (href, media),
    };
    if is_blocked_href(&href) {
        report.skipped_at_rules += 1;
        return;
    }
    if ctx.stack.len() as u32 >= MAX_IMPORT_DEPTH {
        report.skipped_at_rules += 1;
        return;
    }
    let Some(loader) = ctx.loader else {
        report.skipped_at_rules += 1;
        return;
    };
    let from = ctx.stack.last().map(String::as_str);
    // A packaged sheet's canonical key is known without reading it: an
    // already parsed import is not read, authenticated and decoded again.
    let cached = crate::css_at_rule::packaged_canonical(&href, from)
        .filter(|key| ctx.cache.contains_key(key));
    let (css, canonical) = match cached {
        Some(key) => (String::new(), key),
        None => match loader.load(&href, from) {
            Some(loaded) => loaded,
            None => {
                report.skipped_at_rules += 1;
                return;
            }
        },
    };
    // Filesystem hrefs compare case-insensitively (case-insensitive disks);
    // packaged entries are case-sensitive keys.
    let packaged = nana_ui_core::is_packaged_url(&canonical);
    if ctx.stack.iter().any(|h| {
        if packaged {
            *h == canonical
        } else {
            h.eq_ignore_ascii_case(&canonical)
        }
    }) {
        report.skipped_at_rules += 1;
        return;
    }
    let imported = if let Some(cached) = ctx.cache.get(&canonical) {
        let mut cloned = cached.clone();
        offset_source_order(&mut cloned, *order);
        cloned
    } else {
        ctx.stack.push(canonical.clone());
        let mut nested = ParsedStylesheet::default();
        let mut nested_order = 0u32;
        let stripped = strip_css_comments(&css);
        parse_stylesheet_into(&stripped, &mut nested_order, &mut nested, report, ctx, true);
        ctx.stack.pop();
        ctx.cache.insert(canonical, nested.clone());
        offset_source_order(&mut nested, *order);
        nested
    };
    if let Some(max) = imported.max_source_order() {
        *order = max.saturating_add(1);
    }
    report.imported_sheets += 1;
    if media.is_unconditional() {
        merge_parsed_stylesheet(sheet, imported);
    } else {
        sheet.media_rules.push(MediaRule {
            query: media,
            sheet: imported,
        });
    }
}

fn record_layer_names(sheet: &mut ParsedStylesheet, names: &[String]) {
    for name in names {
        if !name.is_empty() && !sheet.layer_names.iter().any(|existing| existing == name) {
            sheet.layer_names.push(name.clone());
        }
    }
}

fn at_rule_ident(s: &str) -> Option<(&str, &str)> {
    let s = s.strip_prefix('@')?;
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    Some((&s[..end], s[end..].trim_start()))
}

fn split_at_rule_tail(after_name: &str) -> Option<(&str, Option<&str>, &str)> {
    let bytes = after_name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == quote {
                        i += 1;
                        break;
                    }
                    if bytes[i] == b'\\' {
                        i = (i + 2).min(bytes.len());
                        continue;
                    }
                    i += 1;
                }
            }
            b'{' | b';' => break,
            _ => i += 1,
        }
    }
    if i >= bytes.len() {
        return Some((after_name.trim(), None, ""));
    }
    let prelude = after_name[..i].trim();
    if bytes[i] == b';' {
        return Some((prelude, None, &after_name[i + 1..]));
    }
    let mut depth = 0i32;
    let mut j = i;
    while j < bytes.len() {
        match bytes[j] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let body = &after_name[i + 1..j];
                    return Some((prelude, Some(body), &after_name[j + 1..]));
                }
            }
            _ => {}
        }
        j += 1;
    }
    Some((prelude, Some(&after_name[i + 1..]), ""))
}

fn parse_stylesheet_into(
    css: &str,
    order: &mut u32,
    sheet: &mut ParsedStylesheet,
    report: &mut StylesheetParseReport,
    ctx: &mut ImportParseCtx<'_>,
    mut allow_import: bool,
) {
    let mut rest = css;
    // CSS ignores `@import` after any style rule or non-charset/import at-rule,
    // and any `@import` nested inside `@media { }` (not a valid import position).
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if rest.starts_with('@') {
            let at_name = at_rule_ident(rest).map(|(name, _)| name);
            if at_name.is_some_and(|name| name.eq_ignore_ascii_case("import")) && !allow_import {
                rest = skip_at_rule(rest);
                report.skipped_at_rules += 1;
                continue;
            }
            if let Some(next) = parse_known_at_rule(rest, order, sheet, report, ctx) {
                if let Some(name) = at_name
                    && !name.eq_ignore_ascii_case("import")
                    && !name.eq_ignore_ascii_case("charset")
                {
                    allow_import = false;
                }
                rest = next;
            } else {
                allow_import = false;
                rest = skip_at_rule(rest);
                report.skipped_at_rules += 1;
            }
            continue;
        }
        allow_import = false;
        let Some((selector_text, body, next)) = split_rule(rest) else {
            report.skipped_rules += 1;
            rest = match rest.find('}') {
                Some(end) => &rest[end + 1..],
                None => "",
            };
            continue;
        };
        rest = next;
        parse_style_rule(selector_text, body, order, sheet, report);
    }
}

/// One style rule: its selectors and declarations into `sheet`. Kept out of
/// [`parse_stylesheet_into`] so the frame that recurses into nested at-rule
/// blocks stays small.
#[inline(never)]
fn parse_style_rule(
    selector_text: &str,
    body: &str,
    order: &mut u32,
    sheet: &mut ParsedStylesheet,
    report: &mut StylesheetParseReport,
) {
    let declarations = body.trim().to_string();
    if declarations.is_empty() {
        report.skipped_declarations += 1;
        return;
    }
    let declaration_entries = parse_declaration_entries(&declarations);
    if declaration_entries.is_empty() {
        report.skipped_declarations += 1;
        return;
    }
    let (layout_entries, motion) = partition_motion_entries(&declaration_entries);
    if layout_entries.is_empty() && motion.is_empty() {
        report.skipped_declarations += 1;
        return;
    }
    let layout_declarations = entries_to_declaration_text(&layout_entries);

    let mut static_selectors = Vec::new();
    let mut interactive_selectors = Vec::new();
    let mut generated = Vec::new();
    let mut scrollbar = Vec::new();
    for part in split_selector_list(selector_text) {
        if let Some((originating, pseudo)) = parse_scrollbar_pseudo_selector(part) {
            scrollbar.push((originating, pseudo));
        } else if let Some((originating, pseudo)) = parse_generated_pseudo_selector(part) {
            generated.push((originating, pseudo));
        } else if let Some(sel) = parse_interactive_selector(part) {
            interactive_selectors.push(sel);
        } else if let Some(sel) = parse_selector(part) {
            static_selectors.push(sel);
        } else {
            report.skipped_selectors += 1;
        }
    }

    if static_selectors.is_empty()
        && interactive_selectors.is_empty()
        && generated.is_empty()
        && scrollbar.is_empty()
    {
        return;
    }

    let has_layout = !layout_entries.is_empty();
    let has_motion = !motion.is_empty();
    let motion_for_rules = if has_motion {
        motion.clone()
    } else {
        MotionDeclarations::default()
    };

    let mut consumed = false;
    if !static_selectors.is_empty() && has_layout {
        report.rules += 1;
        consumed = true;
        sheet.static_rules.push(StyleRule {
            selectors: static_selectors.clone(),
            declarations: layout_declarations.clone(),
            declaration_entries: layout_entries.clone(),
            source_order: *order,
        });
    }
    if !interactive_selectors.is_empty() && (has_layout || has_motion) {
        consumed = true;
        for sel in interactive_selectors {
            sheet.interactive_rules.push(InteractiveStyleRule {
                selector: sel,
                declarations: layout_declarations.clone(),
                declaration_entries: layout_entries.clone(),
                motion: motion_for_rules.clone(),
                source_order: *order,
                layer: None,
                media: Vec::new(),
            });
        }
    }
    if !generated.is_empty() && (has_layout || has_motion) {
        consumed = true;
        for (originating_selector, pseudo) in generated {
            sheet.generated_pseudo_rules.push(GeneratedPseudoRule {
                originating_selector,
                pseudo,
                declarations: layout_declarations.clone(),
                declaration_entries: layout_entries.clone(),
                motion: motion_for_rules.clone(),
                source_order: *order,
                layer: None,
                media: Vec::new(),
            });
        }
    }
    if !scrollbar.is_empty() && has_layout {
        consumed = true;
        for (originating_selector, pseudo) in scrollbar {
            sheet.scrollbar_pseudo_rules.push(ScrollbarPseudoRule {
                originating_selector,
                pseudo,
                declaration_entries: layout_entries.clone(),
                source_order: *order,
                layer: None,
                media: Vec::new(),
            });
        }
    }
    if !static_selectors.is_empty() && has_motion {
        consumed = true;
        sheet.motion_rules.push(MotionStyleRule {
            selectors: static_selectors,
            motion,
            source_order: *order,
            layer: None,
            media: Vec::new(),
        });
    }
    if consumed {
        *order = order.saturating_add(1);
    }
}

fn entries_to_declaration_text(entries: &[DeclarationEntry]) -> String {
    entries
        .iter()
        .map(|entry| {
            if entry.important {
                format!("{}: {} !important", entry.property, entry.value)
            } else {
                entry.text()
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// [`parse_stylesheet`] plus skipped-content diagnostics.
///
/// Matching `@media` (default [`crate::css_at_rule::MediaEnvironment`]) is
/// flattened into the returned rules so the simple parse API affects cascade.
pub fn parse_stylesheet_with_report(
    css: &str,
    order_base: u32,
) -> (Vec<StyleRule>, StylesheetParseReport) {
    let (sheet, report) = parse_stylesheet_full(css, order_base);
    (
        sheet
            .flatten(&crate::css_at_rule::MediaEnvironment::default())
            .static_rules,
        report,
    )
}

/// Split a declaration block into structured entries (once per rule at parse).
pub fn parse_declaration_entries(block: &str) -> Vec<DeclarationEntry> {
    let mut out = Vec::new();
    for (i, decl) in crate::css_font_face::split_decls(block)
        .into_iter()
        .enumerate()
    {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        let Some((raw_key, raw_val)) = decl.split_once(':') else {
            continue;
        };
        let key = raw_key.trim();
        if key.is_empty() {
            continue;
        }
        let (value, important) = split_important_flag(raw_val.trim());
        if value.is_empty() {
            continue;
        }
        out.push(DeclarationEntry {
            index: i as u32,
            important,
            property: key.to_string(),
            value,
        });
    }
    out
}

/// Document-level `--*` from parsed rules for the active theme (source order; last wins).
///
/// Replaces re-scraping raw stylesheet text on inject / theme change: same theme
/// filter as [`crate::css_map::collect_document_css_custom_properties`], but reads
/// cached [`DeclarationEntry`] values from rules that already survived parse.
pub fn collect_document_custom_properties_from_rules(
    rules: &[StyleRule],
    theme: &str,
) -> crate::css_map::CssVarScope {
    let mut map = BTreeMap::new();
    for rule in rules {
        if !rule_contributes_document_vars(rule, theme) {
            continue;
        }
        for entry in &rule.declaration_entries {
            if let Some(name) = entry.property.strip_prefix("--")
                && !name.is_empty()
            {
                map.insert(entry.property.clone(), entry.value.clone());
            }
        }
    }
    crate::css_map::CssVarScope::from_map(crate::css_map::merge_css_custom_properties(
        &map,
        &BTreeMap::new(),
    ))
}

fn rule_contributes_document_vars(rule: &StyleRule, theme: &str) -> bool {
    let theme = theme.trim();
    rule.selectors.iter().any(|sel| {
        if !selector_is_document_var_scope(sel) {
            return false;
        }
        match data_theme_constraint_from_compound(&sel.subject) {
            Some(want) => want.eq_ignore_ascii_case(theme),
            None => true,
        }
    })
}

/// `Some(theme)` when subject requires `data-theme=…`; `None` when unconstrained.
fn data_theme_constraint_from_compound(c: &CompoundSelector) -> Option<String> {
    for attr in &c.attrs {
        if !attr.name.eq_ignore_ascii_case("data-theme") {
            continue;
        }
        match attr.op {
            AttrOperator::Equal => return attr.value.clone(),
            // Presence-only `[data-theme]` — unconstrained (matches css_map scrape).
            AttrOperator::Present => return None,
            _ => {}
        }
    }
    None
}

/// Subject is a bare document selector that receives theme-aware `--*` via
/// [`collect_document_custom_properties_from_rules`].
fn selector_is_document_var_scope(sel: &Selector) -> bool {
    if !sel.ancestors.is_empty() {
        return false;
    }
    compound_is_document_var_scope(&sel.subject)
}

fn compound_is_document_var_scope(c: &CompoundSelector) -> bool {
    // Class / id makes the subject element-scoped (e.g. `:root.theme-shell`).
    if !c.classes.is_empty() || c.id.is_some() {
        return false;
    }
    if c.root {
        return true;
    }
    let tag = c.type_name.as_deref().map(|t| t.to_ascii_lowercase());
    match tag.as_deref() {
        Some("html") | Some("body") | Some("*") => true,
        None => c
            .attrs
            .iter()
            .any(|a| a.name.eq_ignore_ascii_case("data-theme")),
        _ => false,
    }
}

fn parse_selector(raw: &str) -> Option<Selector> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if selector_has_deferred_pseudo(s) || selector_has_interactive_pseudo(s) {
        return None;
    }
    parse_selector_chain(s)
}

fn parse_interactive_selector(raw: &str) -> Option<InteractiveSelector> {
    let s = raw.trim();
    if s.is_empty() || !selector_has_interactive_pseudo(s) {
        return None;
    }
    if selector_has_deferred_pseudo(s) {
        return None;
    }
    let tokens = tokenize_selector_chain(s)?;
    if tokens.is_empty() {
        return None;
    }
    let mut ancestors = Vec::new();
    let mut i = 0;
    while i + 1 < tokens.len() {
        let compound = parse_compound(&tokens[i].0, ParseCompoundMode::Interactive)?;
        let comb = tokens[i].1.unwrap_or(Combinator::Descendant);
        ancestors.push((comb, compound));
        i += 1;
    }
    let subject = parse_compound(&tokens[i].0, ParseCompoundMode::Interactive)?;
    if ancestors
        .iter()
        .any(|(_, c)| !c.has_queries.is_empty() || c.focus_within)
    {
        return None;
    }
    let mut interactive_count = 0usize;
    let mut interactive_at = None::<usize>;
    let mut interactive_pseudo = None::<InteractivePseudo>;
    for (idx, (_, compound)) in ancestors.iter().enumerate() {
        if let Some(pseudo) = compound.interactive {
            interactive_count += 1;
            interactive_at = Some(idx);
            interactive_pseudo = Some(pseudo);
        }
    }
    if let Some(pseudo) = subject.interactive {
        interactive_count += 1;
        interactive_at = Some(ancestors.len());
        interactive_pseudo = Some(pseudo);
    }
    if interactive_count != 1 {
        return None;
    }
    let interactive_at = interactive_at?;
    let pseudo = interactive_pseudo?;
    if interactive_at == ancestors.len() {
        for (comb, _) in &ancestors {
            if !matches!(*comb, Combinator::Descendant | Combinator::Child) {
                return None;
            }
        }
    } else {
        let (_, compound) = &ancestors[interactive_at];
        if compound.interactive != Some(pseudo) {
            return None;
        }
        if interactive_at + 1 != ancestors.len() {
            return None;
        }
        let (comb, _) = &ancestors[interactive_at];
        if *comb != Combinator::Descendant {
            return None;
        }
    }
    let mut specificity = Specificity::default();
    for (_, c) in &ancestors {
        add_specificity(&mut specificity, c);
    }
    add_specificity(&mut specificity, &subject);
    Some(InteractiveSelector {
        subject,
        ancestors,
        interactive_at,
        pseudo,
        specificity,
    })
}

fn parse_generated_pseudo_selector(raw: &str) -> Option<(Selector, GeneratedPseudo)> {
    let s = raw.trim();
    let (base, pseudo) = strip_subject_generated_pseudo(s)?;
    if selector_has_interactive_pseudo(s) || selector_has_deferred_pseudo(&base) {
        return None;
    }
    let originating = if base.is_empty() { "*" } else { base.as_str() };
    let originating_selector = parse_selector_chain(originating)?;
    Some((originating_selector, pseudo))
}

fn parse_scrollbar_pseudo_selector(raw: &str) -> Option<(Selector, ScrollbarPseudo)> {
    let s = raw.trim();
    let (base, pseudo) = strip_subject_scrollbar_pseudo(s)?;
    if selector_has_interactive_pseudo(s) || selector_has_deferred_pseudo(&base) {
        return None;
    }
    let originating = if base.is_empty() { "*" } else { base.as_str() };
    let originating_selector = parse_selector_chain(originating)?;
    Some((originating_selector, pseudo))
}

fn strip_subject_scrollbar_pseudo(s: &str) -> Option<(String, ScrollbarPseudo)> {
    let lower = s.to_ascii_lowercase();
    for (suffix, pseudo) in [
        ("::-webkit-scrollbar-thumb", ScrollbarPseudo::Thumb),
        ("::-webkit-scrollbar", ScrollbarPseudo::Scrollbar),
    ] {
        if lower.ends_with(suffix) {
            let base = s[..s.len().wrapping_sub(suffix.len())].trim().to_string();
            return Some((base, pseudo));
        }
    }
    None
}

fn parse_selector_chain(s: &str) -> Option<Selector> {
    let tokens = tokenize_selector_chain(s)?;
    if tokens.is_empty() {
        return None;
    }
    let mut ancestors = Vec::new();
    let mut i = 0;
    while i + 1 < tokens.len() {
        let compound = parse_compound(&tokens[i].0, ParseCompoundMode::Static)?;
        let comb = tokens[i].1.unwrap_or(Combinator::Descendant);
        ancestors.push((comb, compound));
        i += 1;
    }
    let subject = parse_compound(&tokens[i].0, ParseCompoundMode::Static)?;
    // Cheap subset: `:has()` / `:focus-within` only on the subject. Ancestor
    // forms would need per-ancestor bits/flags and are skipped.
    if ancestors
        .iter()
        .any(|(_, c)| !c.has_queries.is_empty() || c.focus_within)
    {
        return None;
    }
    let mut specificity = Specificity::default();
    for (_, c) in &ancestors {
        add_specificity(&mut specificity, c);
    }
    add_specificity(&mut specificity, &subject);
    Some(Selector {
        subject,
        ancestors,
        specificity,
    })
}

fn strip_subject_generated_pseudo(s: &str) -> Option<(String, GeneratedPseudo)> {
    let lower = s.to_ascii_lowercase();
    for (suffix, pseudo) in [
        ("::-moz-selection", GeneratedPseudo::Selection),
        ("::selection", GeneratedPseudo::Selection),
        ("::before", GeneratedPseudo::Before),
        ("::after", GeneratedPseudo::After),
        ("::placeholder", GeneratedPseudo::Placeholder),
        (":before", GeneratedPseudo::Before),
        (":after", GeneratedPseudo::After),
    ] {
        if lower.ends_with(suffix) {
            let base = s[..s.len().wrapping_sub(suffix.len())].trim();
            // `::selection` may be bare (originating `*`). Boxes still need a subject.
            if base.is_empty() && !matches!(pseudo, GeneratedPseudo::Selection) {
                continue;
            }
            return Some((base.to_string(), pseudo));
        }
    }
    None
}

fn selector_has_interactive_pseudo(s: &str) -> bool {
    scan_selector_pseudos(s, |name| InteractivePseudo::from_ident(name).is_some())
}

fn selector_has_deferred_pseudo(s: &str) -> bool {
    scan_selector_pseudos(s, |name| {
        if InteractivePseudo::from_ident(name).is_some() {
            return false;
        }
        if GeneratedPseudo::from_ident(name).is_some() {
            return true;
        }
        !matches!(
            name,
            "where"
                | "is"
                | "not"
                | "nth-child"
                | "nth-of-type"
                | "nth-last-child"
                | "has"
                | "root"
                | "first-child"
                | "last-child"
                | "only-child"
                | "first-of-type"
                | "last-of-type"
                | "empty"
                | "checked"
                | "disabled"
                | "focus-within"
        )
    }) || s.to_ascii_lowercase().contains("::")
        || has_legacy_pseudo_element(s)
}

fn has_legacy_pseudo_element(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    [":before", ":after"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
        && !lower.ends_with("::before")
        && !lower.ends_with("::after")
}

fn scan_selector_pseudos(s: &str, mut classify: impl FnMut(&str) -> bool) -> bool {
    let lower = s.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(idx) = rest.find(':') {
        let after = &rest[idx + 1..];
        if let Some(stripped) = after.strip_prefix(':') {
            rest = stripped;
            continue;
        }
        if let Some(end) = skip_ident(after) {
            let name = &after[..end];
            let rem = &after[end..];
            match name {
                "where" | "is" | "not" | "nth-child" | "nth-of-type" | "nth-last-child" | "has" => {
                    if !rem.starts_with('(') {
                        return true;
                    }
                    let Some(close) = balanced_paren_end(rem) else {
                        return true;
                    };
                    if matches!(name, "nth-child" | "nth-of-type" | "nth-last-child") {
                        let inner = &rem[1..close];
                        if nth_arg_has_of_clause(inner) {
                            return true;
                        }
                    }
                    rest = &rem[close + 1..];
                    continue;
                }
                "root" | "first-child" | "last-child" | "only-child" | "first-of-type"
                | "last-of-type" | "empty" | "checked" | "disabled" | "focus-within" => {
                    rest = rem;
                    continue;
                }
                _ if classify(name) => return true,
                _ => {
                    rest = rem;
                    continue;
                }
            }
        }
        return true;
    }
    false
}

fn skip_ident(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let c0 = bytes[0];
    if !(c0.is_ascii_alphabetic() || c0 == b'_' || c0 == b'-') {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' {
            i += 1;
        } else {
            break;
        }
    }
    Some(i)
}

fn balanced_paren_end(s: &str) -> Option<usize> {
    // s starts with '('
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    for (idx, ch) in bytes.iter().enumerate() {
        match *ch {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn tokenize_selector_chain(s: &str) -> Option<Vec<(String, Option<Combinator>)>> {
    // Split on combinators outside of [] / ().
    let mut parts: Vec<(String, Option<Combinator>)> = Vec::new();
    let mut cur = String::new();
    let mut depth_br = 0i32;
    let mut depth_paren = 0i32;
    let bytes = s.as_bytes();
    let mut i = 0;

    let push_compound = |parts: &mut Vec<(String, Option<Combinator>)>,
                         cur: &mut String,
                         comb: Combinator|
     -> Option<()> {
        let compound = cur.trim().to_string();
        if compound.is_empty() {
            return None;
        }
        parts.push((compound, Some(comb)));
        cur.clear();
        Some(())
    };

    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '[' => {
                depth_br += 1;
                cur.push(c);
                i += 1;
            }
            ']' => {
                depth_br -= 1;
                cur.push(c);
                i += 1;
            }
            '(' => {
                depth_paren += 1;
                cur.push(c);
                i += 1;
            }
            ')' => {
                depth_paren -= 1;
                cur.push(c);
                i += 1;
            }
            '>' | '+' | '~' if depth_br == 0 && depth_paren == 0 => {
                let comb = match c {
                    '>' => Combinator::Child,
                    '+' => Combinator::AdjacentSibling,
                    _ => Combinator::SubsequentSibling,
                };
                push_compound(&mut parts, &mut cur, comb)?;
                i += 1;
                while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                    i += 1;
                }
            }
            c if c.is_whitespace() && depth_br == 0 && depth_paren == 0 => {
                while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                    i += 1;
                }
                if i < bytes.len() {
                    let n = bytes[i] as char;
                    if n == '>' || n == '+' || n == '~' {
                        continue; // handled as explicit combinator next loop
                    }
                }
                let compound = cur.trim().to_string();
                if compound.is_empty() {
                    continue;
                }
                parts.push((compound, Some(Combinator::Descendant)));
                cur.clear();
            }
            _ => {
                cur.push(c);
                i += 1;
            }
        }
    }
    let last = cur.trim().to_string();
    if last.is_empty() {
        return None;
    }
    parts.push((last, None));
    Some(parts)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseCompoundMode {
    Static,
    Interactive,
}

fn parse_compound(raw: &str, mode: ParseCompoundMode) -> Option<CompoundSelector> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let mut out = CompoundSelector::default();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    if chars
        .first()
        .is_some_and(|c| c.is_ascii_alphabetic() || *c == '*' || *c == '_')
    {
        let start = i;
        i += 1;
        while i < chars.len()
            && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_')
        {
            i += 1;
        }
        let name: String = chars[start..i].iter().collect();
        if name != "*" {
            out.type_name = Some(name.to_ascii_lowercase());
        } else {
            out.type_name = Some("*".into());
        }
    }
    while i < chars.len() {
        match chars[i] {
            '.' => {
                i += 1;
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_')
                {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                out.classes.push(chars[start..i].iter().collect::<String>());
            }
            '#' => {
                i += 1;
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_')
                {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                out.id = Some(chars[start..i].iter().collect());
            }
            '[' => {
                let close = chars[i..].iter().position(|c| *c == ']')? + i;
                let inner: String = chars[i + 1..close].iter().collect();
                i = close + 1;
                out.attrs.push(parse_attr_inner(inner.trim())?);
            }
            ':' => {
                if i + 1 < chars.len() && chars[i + 1] == ':' {
                    return None;
                }
                i += 1;
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_')
                {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                let name: String = chars[start..i]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase();
                if mode == ParseCompoundMode::Interactive
                    && let Some(pseudo) = InteractivePseudo::from_ident(&name)
                {
                    if out.interactive.is_some() {
                        return None;
                    }
                    out.interactive = Some(pseudo);
                    continue;
                }
                match name.as_str() {
                    "first-child" => out.first_child = true,
                    "last-child" => out.last_child = true,
                    "only-child" => out.only_child = true,
                    "first-of-type" => out.first_of_type = true,
                    "last-of-type" => out.last_of_type = true,
                    "empty" => out.empty = true,
                    "checked" => out.checked = true,
                    "disabled" => out.attrs.push(present_attr("disabled")),
                    "root" => out.root = true,
                    "focus-within" => out.focus_within = true,
                    "nth-child" | "nth-of-type" | "nth-last-child" => {
                        if i >= chars.len() || chars[i] != '(' {
                            return None;
                        }
                        i += 1;
                        let inner_start = i;
                        let mut depth = 1i32;
                        while i < chars.len() {
                            match chars[i] {
                                '(' => depth += 1,
                                ')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            i += 1;
                        }
                        if depth != 0 || i >= chars.len() {
                            return None;
                        }
                        let inner: String = chars[inner_start..i].iter().collect();
                        i += 1; // skip ')'
                        if nth_arg_has_of_clause(&inner) {
                            return None;
                        }
                        let anb = parse_an_plus_b(inner.trim())?;
                        match name.as_str() {
                            "nth-child" => out.nth_child = Some(anb),
                            "nth-of-type" => out.nth_of_type = Some(anb),
                            "nth-last-child" => out.nth_last_child = Some(anb),
                            _ => unreachable!(),
                        }
                    }
                    "has" => {
                        if i >= chars.len() || chars[i] != '(' {
                            return None;
                        }
                        i += 1;
                        let inner_start = i;
                        let mut depth = 1i32;
                        while i < chars.len() {
                            match chars[i] {
                                '(' => depth += 1,
                                ')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            i += 1;
                        }
                        if depth != 0 || i >= chars.len() {
                            return None;
                        }
                        let inner: String = chars[inner_start..i].iter().collect();
                        i += 1; // skip ')'
                        let alts = parse_simple_selector_list(inner.trim())?;
                        out.has_queries.push(alts);
                    }
                    "not" | "is" | "where" => {
                        if i >= chars.len() || chars[i] != '(' {
                            return None;
                        }
                        i += 1;
                        let inner_start = i;
                        let mut depth = 1i32;
                        while i < chars.len() {
                            match chars[i] {
                                '(' => depth += 1,
                                ')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            i += 1;
                        }
                        if depth != 0 || i >= chars.len() {
                            return None;
                        }
                        let inner: String = chars[inner_start..i].iter().collect();
                        i += 1; // skip ')'
                        let alts = parse_simple_selector_list(inner.trim())?;
                        if alts.is_empty() {
                            return None;
                        }
                        match name.as_str() {
                            "not" => out.not_alts.extend(alts),
                            "is" => out.is_alts.extend(alts),
                            "where" => out.where_alts.extend(alts),
                            _ => unreachable!(),
                        }
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
    if compound_is_empty(&out) {
        return None;
    }
    Some(out)
}

fn compound_is_empty(out: &CompoundSelector) -> bool {
    out.type_name.is_none()
        && out.id.is_none()
        && out.classes.is_empty()
        && out.attrs.is_empty()
        && out.not_alts.is_empty()
        && out.is_alts.is_empty()
        && out.where_alts.is_empty()
        && !out.first_child
        && !out.last_child
        && !out.only_child
        && !out.first_of_type
        && !out.last_of_type
        && !out.empty
        && !out.checked
        && !out.root
        && out.nth_child.is_none()
        && out.nth_of_type.is_none()
        && out.nth_last_child.is_none()
        && out.has_queries.is_empty()
        && !out.focus_within
}

fn parse_attr_inner(inner: &str) -> Option<AttrSelector> {
    if inner.is_empty() {
        return None;
    }
    // Optional trailing ` i` / ` s` case flag (Selectors Level 4).
    let (body, case) = {
        let t = inner.trim_end();
        let bytes = t.as_bytes();
        if bytes.len() >= 2 && bytes[bytes.len() - 2] == b' ' {
            match bytes[bytes.len() - 1].to_ascii_lowercase() {
                b'i' => (&t[..t.len() - 2], AttrCase::Insensitive),
                b's' => (&t[..t.len() - 2], AttrCase::Sensitive),
                _ => (t, AttrCase::Default),
            }
        } else {
            (t, AttrCase::Default)
        }
    };
    let body = body.trim();
    // Operators longest-first: ~= |= ^= $= *= then =
    for (sym, op) in [
        ("~=", AttrOperator::Includes),
        ("|=", AttrOperator::DashMatch),
        ("^=", AttrOperator::Prefix),
        ("$=", AttrOperator::Suffix),
        ("*=", AttrOperator::Substring),
        ("=", AttrOperator::Equal),
    ] {
        if let Some((k, v)) = body.split_once(sym) {
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            return Some(AttrSelector {
                name: k.trim().to_ascii_lowercase(),
                op,
                value: Some(v.to_string()),
                case,
            });
        }
    }
    Some(AttrSelector {
        name: body.to_ascii_lowercase(),
        op: AttrOperator::Present,
        value: None,
        case,
    })
}

/// Parse a forgiving-ish list of **simple** compounds for `:is`/`:where`/`:not`.
/// Combinators / nested functional pseudos inside an alt ⇒ whole list rejected
/// (honest defer — do not partially match). Cheap `:disabled` is allowed and
/// stored as `[disabled]` presence.
fn parse_simple_selector_list(inner: &str) -> Option<Vec<SimpleCompound>> {
    let mut out = Vec::new();
    for part in split_selector_list(inner) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // Reject combinators / nested pseudos outside `[]` (so `[attr~=x]` is ok).
        if simple_alt_has_combinator_or_pseudo(part) {
            return None;
        }
        out.push(parse_simple_compound(part)?);
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

fn simple_alt_has_combinator_or_pseudo(s: &str) -> bool {
    let mut depth_br = 0i32;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => {
                depth_br += 1;
                i += 1;
            }
            b']' => {
                depth_br -= 1;
                i += 1;
            }
            b':' if depth_br == 0 => {
                let rest = &s[i + 1..];
                let Some(end) = skip_ident(rest) else {
                    return true;
                };
                if !matches!(
                    rest[..end].to_ascii_lowercase().as_str(),
                    "disabled" | "checked" | "empty"
                ) {
                    return true;
                }
                i += 1 + end;
            }
            b'>' | b'+' if depth_br == 0 => return true,
            b'~' if depth_br == 0 => return true,
            c if c.is_ascii_whitespace() && depth_br == 0 => return true,
            _ => i += 1,
        }
    }
    false
}

fn parse_simple_compound(raw: &str) -> Option<SimpleCompound> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let mut out = SimpleCompound::default();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    if chars
        .first()
        .is_some_and(|c| c.is_ascii_alphabetic() || *c == '*' || *c == '_')
    {
        let start = i;
        i += 1;
        while i < chars.len()
            && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_')
        {
            i += 1;
        }
        let name: String = chars[start..i].iter().collect();
        if name != "*" {
            out.type_name = Some(name.to_ascii_lowercase());
        } else {
            out.type_name = Some("*".into());
        }
    }
    while i < chars.len() {
        match chars[i] {
            '.' => {
                i += 1;
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_')
                {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                out.classes.push(chars[start..i].iter().collect::<String>());
            }
            '#' => {
                i += 1;
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_')
                {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                out.id = Some(chars[start..i].iter().collect());
            }
            '[' => {
                let close = chars[i..].iter().position(|c| *c == ']')? + i;
                let inner: String = chars[i + 1..close].iter().collect();
                i = close + 1;
                out.attrs.push(parse_attr_inner(inner.trim())?);
            }
            ':' => {
                i += 1;
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_')
                {
                    i += 1;
                }
                if start == i {
                    return None;
                }
                let name: String = chars[start..i]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase();
                match name.as_str() {
                    "disabled" => out.attrs.push(present_attr("disabled")),
                    "checked" => out.checked = true,
                    "empty" => out.empty = true,
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
    if out.type_name.is_none()
        && out.id.is_none()
        && out.classes.is_empty()
        && out.attrs.is_empty()
        && !out.empty
        && !out.checked
    {
        return None;
    }
    Some(out)
}

fn present_attr(name: &str) -> AttrSelector {
    AttrSelector {
        name: name.into(),
        op: AttrOperator::Present,
        value: None,
        case: AttrCase::Default,
    }
}

fn add_simple_specificity(spec: &mut Specificity, simple: &SimpleCompound) {
    if simple.id.is_some() {
        spec.ids = spec.ids.saturating_add(1);
    }
    spec.classes_attrs = spec
        .classes_attrs
        .saturating_add(simple.classes.len() as u16)
        .saturating_add(simple.attrs.len() as u16)
        .saturating_add(u16::from(simple.empty))
        .saturating_add(u16::from(simple.checked));
    if simple.type_name.as_ref().is_some_and(|t| t != "*") {
        spec.types = spec.types.saturating_add(1);
    }
}

fn max_alts_specificity(alts: &[SimpleCompound]) -> Specificity {
    let mut max = Specificity::default();
    for alt in alts {
        let mut s = Specificity::default();
        add_simple_specificity(&mut s, alt);
        if s > max {
            max = s;
        }
    }
    max
}

fn add_specificity(spec: &mut Specificity, compound: &CompoundSelector) {
    if compound.id.is_some() {
        spec.ids = spec.ids.saturating_add(1);
    }
    let structural = u16::from(compound.first_child)
        + u16::from(compound.last_child)
        + u16::from(compound.only_child)
        + u16::from(compound.first_of_type)
        + u16::from(compound.last_of_type)
        + u16::from(compound.empty)
        + u16::from(compound.checked)
        + u16::from(compound.root)
        + u16::from(compound.nth_child.is_some())
        + u16::from(compound.nth_of_type.is_some())
        + u16::from(compound.nth_last_child.is_some())
        + u16::from(compound.interactive.is_some())
        + u16::from(compound.focus_within);
    spec.classes_attrs = spec
        .classes_attrs
        .saturating_add(compound.classes.len() as u16)
        .saturating_add(compound.attrs.len() as u16)
        .saturating_add(structural);
    if compound.type_name.as_ref().is_some_and(|t| t != "*") {
        spec.types = spec.types.saturating_add(1);
    }
    // :not() / :is() — specificity of the most specific argument (MDN).
    if !compound.not_alts.is_empty() {
        spec.saturating_add_assign(max_alts_specificity(&compound.not_alts));
    }
    if !compound.is_alts.is_empty() {
        spec.saturating_add_assign(max_alts_specificity(&compound.is_alts));
    }
    // :has() — specificity of the most specific argument (Selectors L4).
    for query in &compound.has_queries {
        spec.saturating_add_assign(max_alts_specificity(query));
    }
    // :where() — always 0 (intentionally omitted).
}

/// True when `:nth-child` / `:nth-of-type` args use the `of <selector-list>` form
/// (Selectors L4) — deferred; do not partially match.
fn nth_arg_has_of_clause(inner: &str) -> bool {
    // Whitespace-delimited `of` token (e.g. `2n+1 of .noted`). `odd`/`even`
    // do not contain a separate `of` token.
    inner
        .split_whitespace()
        .any(|tok| tok.eq_ignore_ascii_case("of"))
}

/// Parse CSS `<An+B> | even | odd` (MDN / CSS Syntax An+B microsyntax).
fn parse_an_plus_b(raw: &str) -> Option<AnPlusB> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    match lower.as_str() {
        "odd" => return Some(AnPlusB::odd()),
        "even" => return Some(AnPlusB::even()),
        _ => {}
    }

    // Strip whitespace for token scanning (CSS allows `2n + 1`).
    let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return None;
    }

    // Plain integer: `3`, `+3`, `-3` → 0n+B
    if let Some(b) = parse_signed_int_full(&compact) {
        return Some(AnPlusB { a: 0, b });
    }

    // Forms with `n`: [±]?[A]?n[±B]?, `n`, `-n`, `+n`, `3n`, `3n+1`, `-n+3`, …
    let n_pos = compact.find('n')?;
    let a_part = &compact[..n_pos];
    let b_part = &compact[n_pos + 1..];

    let a = if a_part.is_empty() || a_part == "+" {
        1
    } else if a_part == "-" {
        -1
    } else {
        parse_signed_int_full(a_part)?
    };

    let b = if b_part.is_empty() {
        0
    } else {
        // Must start with + or - (CSS: `n+2`, `n-2`); bare `n2` is invalid.
        let first = b_part.as_bytes()[0];
        if first != b'+' && first != b'-' {
            return None;
        }
        parse_signed_int_full(b_part)?
    };

    Some(AnPlusB { a, b })
}

fn parse_signed_int_full(s: &str) -> Option<i32> {
    if s.is_empty() {
        return None;
    }
    s.parse::<i32>().ok()
}

pub(crate) fn strip_css_comments(css: &str) -> String {
    // Copy non-comment byte ranges as slices so multi-byte UTF-8 payloads
    // (Chinese comments, `content` strings, family names) survive verbatim.
    let bytes = css.as_bytes();
    let mut out = String::with_capacity(css.len());
    let mut copy_from = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            out.push_str(&css[copy_from..i]);
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            copy_from = i;
            continue;
        }
        i += 1;
    }
    out.push_str(&css[copy_from..]);
    out
}

fn skip_at_rule(s: &str) -> &str {
    // @media ... { ... } or @keyframes ... { ... } or @import ...;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] != b'{' && bytes[i] != b';' {
        i += 1;
    }
    if i >= bytes.len() {
        return "";
    }
    if bytes[i] == b';' {
        return &s[i + 1..];
    }
    let mut depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &s[i + 1..];
                }
            }
            _ => {}
        }
        i += 1;
    }
    ""
}

fn split_rule(s: &str) -> Option<(&str, &str, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut depth_br = 0i32;
    let mut depth_paren = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => depth_br += 1,
            b']' => depth_br -= 1,
            b'(' => depth_paren += 1,
            b')' => depth_paren -= 1,
            b'{' if depth_br == 0 && depth_paren == 0 => break,
            _ => {}
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let selector = s[..i].trim();
    i += 1; // skip '{'
    let body_start = i;
    let mut depth = 1i32;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let body = &s[body_start..i];
                    let next = &s[i + 1..];
                    return Some((selector, body, next));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn split_selector_list(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth_br = 0i32;
    let mut depth_paren = 0i32;
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        match *b {
            b'[' => depth_br += 1,
            b']' => depth_br -= 1,
            b'(' => depth_paren += 1,
            b')' => depth_paren -= 1,
            b',' if depth_br == 0 && depth_paren == 0 => {
                let part = s[start..i].trim();
                if !part.is_empty() {
                    out.push(part);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let part = s[start..].trim();
    if !part.is_empty() {
        out.push(part);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn malformed_rule_does_not_truncate_following_rules() {
        // An unclosed outer block used to abort the whole parse; recovery must
        // keep the rules that follow the first `}`.
        let css = ".x { .a { color: red } .b { color: blue }";
        let (rules, report) = parse_stylesheet_with_report(css, 0);
        assert_eq!(report.skipped_rules, 1);
        let selectors: Vec<String> = rules
            .iter()
            .filter_map(|rule| {
                rule.selectors
                    .first()
                    .and_then(|sel| sel.subject.classes.first().cloned())
            })
            .collect();
        assert_eq!(selectors, vec!["b".to_string()]);
    }

    #[test]
    fn skipped_content_counters_report_every_drop_kind() {
        let css = concat!(
            "@supports (color: lab(0% 0 0)) { .lab { color: red } }",
            ".empty { }",
            "input::file-selector-button { color: gray }",
            ".kept { color: blue }"
        );
        let (rules, report) = parse_stylesheet_with_report(css, 0);
        assert_eq!(report.skipped_at_rules, 1);
        assert_eq!(report.skipped_declarations, 1);
        assert_eq!(report.skipped_selectors, 1);
        assert_eq!(report.rules, 1);
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn cheap_has_descendant_parses_subject() {
        let rules = parse_stylesheet(".card:has(.badge) { width: 80px }", 0);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selectors[0].subject.has_queries.len(), 1);
    }

    #[test]
    fn has_with_combinator_is_skipped() {
        let (_, report) = parse_stylesheet_with_report(
            ".card:has(.a > .b) { color: red } input::placeholder { color: gray }",
            0,
        );
        assert_eq!(report.skipped_selectors, 1);
        assert_eq!(report.rules, 0);
    }

    #[test]
    fn placeholder_pseudo_parses_as_generated_not_skipped() {
        let (sheet, report) =
            parse_stylesheet_full("input::placeholder { color: gray; opacity: 0.5 }", 0);
        assert_eq!(report.skipped_selectors, 0);
        assert_eq!(sheet.generated_pseudo_rules.len(), 1);
        assert_eq!(
            sheet.generated_pseudo_rules[0].pseudo,
            crate::css_interactive::GeneratedPseudo::Placeholder
        );
        assert!(
            sheet.generated_pseudo_rules[0]
                .declaration_entries
                .iter()
                .any(|e| e.property == "color")
        );
    }

    #[test]
    fn selection_pseudo_parses_background_and_color() {
        let (sheet, report) = parse_stylesheet_full(
            "::selection { background: #ff0000; color: #ffffff; caret-color: blue } ::-moz-selection { background: #00ff00 }",
            0,
        );
        assert_eq!(report.skipped_selectors, 0);
        assert_eq!(sheet.generated_pseudo_rules.len(), 2);
        assert!(
            sheet
                .generated_pseudo_rules
                .iter()
                .all(|rule| rule.pseudo == crate::css_interactive::GeneratedPseudo::Selection)
        );
        assert!(
            sheet.generated_pseudo_rules[0]
                .declaration_entries
                .iter()
                .any(|e| e.property == "background")
        );
        assert!(
            sheet.generated_pseudo_rules[0]
                .declaration_entries
                .iter()
                .any(|e| e.property == "color")
        );
    }

    #[test]
    fn ancestor_has_is_skipped() {
        let (_, report) = parse_stylesheet_with_report(".card:has(.x) .child { color: red }", 0);
        assert_eq!(report.skipped_selectors, 1);
    }

    #[test]
    fn strip_css_comments_preserves_utf8_payloads() {
        let css = "/* 中文注释 */ .card { color: red; content: \"中文提示\" } /* 尾注 */";
        assert_eq!(
            strip_css_comments(css),
            " .card { color: red; content: \"中文提示\" } "
        );
        // Unterminated comment swallows the rest, byte-identically for ASCII.
        assert_eq!(strip_css_comments("a { } /* 未闭合"), "a { } ");
    }

    #[test]
    fn non_ascii_comments_and_content_survive_parsing() {
        let with_comments = "/* 深色主题 */ .card { color: red; content: \"按钮文字\" } /* 杂项 */";
        let without = " .card { color: red; content: \"按钮文字\" }  ";
        let commented = parse_stylesheet(with_comments, 0);
        let plain = parse_stylesheet(without, 0);
        assert_eq!(commented.len(), 1);
        assert_eq!(plain.len(), 1);
        assert_eq!(commented[0].declarations, plain[0].declarations);
        assert!(commented[0].declarations.contains("按钮文字"));
    }

    #[test]
    fn focus_within_subject_parses() {
        let rules = parse_stylesheet(".field:focus-within { width: 12px; }", 0);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].selectors[0].subject.focus_within);
    }

    #[test]
    fn ancestor_focus_within_is_skipped() {
        let (_, report) =
            parse_stylesheet_with_report(".field:focus-within .child { color: red }", 0);
        assert_eq!(report.skipped_selectors, 1);
        assert_eq!(report.rules, 0);
    }

    #[test]
    fn parses_hover_pseudo_and_keyframes_into_buckets() {
        let (sheet, report) = parse_stylesheet_full(
            r#"
            .ok { height: 100%; }
            .ok:hover { height: 50%; }
            @keyframes spin { to { transform: rotate(1turn); } }
            .ok::before { content: ""; }
            "#,
            0,
        );
        assert_eq!(sheet.static_rules.len(), 1);
        assert_eq!(sheet.interactive_rules.len(), 1);
        assert_eq!(sheet.generated_pseudo_rules.len(), 1);
        assert!(sheet.keyframes.contains_key("spin"));
        assert_eq!(report.skipped_at_rules, 0);
        assert!(
            sheet.interactive_rules[0]
                .declaration_entries
                .iter()
                .any(|e| e.property == "height" && e.value == "50%")
        );
        assert_eq!(
            sheet.generated_pseudo_rules[0].pseudo,
            crate::css_interactive::GeneratedPseudo::Before
        );
    }

    #[test]
    fn interactive_descendant_and_motion_parse_only() {
        let (sheet, _) = parse_stylesheet_full(
            r#"
            .card { color: blue; }
            .card:hover .icon { width: 24px; }
            .card:focus .icon { width: 32px; }
            .card { transition: opacity 0.2s ease; animation-name: fade; }
            "#,
            0,
        );
        assert_eq!(sheet.static_rules.len(), 1);
        assert_eq!(sheet.interactive_rules.len(), 2);
        assert_eq!(sheet.motion_rules.len(), 1);
        assert_eq!(
            sheet.motion_rules[0].motion.transition.as_deref(),
            Some("opacity 0.2s ease")
        );
        assert_eq!(
            sheet.motion_rules[0].motion.animation_name.as_deref(),
            Some("fade")
        );
    }

    #[test]
    fn legacy_single_colon_before_pseudo_parses() {
        let (sheet, _) = parse_stylesheet_full(".chip:before { content: \"*\"; width: 2px; }", 0);
        assert_eq!(sheet.generated_pseudo_rules.len(), 1);
        let rule = &sheet.generated_pseudo_rules[0];
        assert_eq!(rule.pseudo, crate::css_interactive::GeneratedPseudo::Before);
        assert!(
            rule.declaration_entries
                .iter()
                .any(|e| e.property == "content" && e.value == "\"*\"")
        );
        assert!(
            rule.originating_selector
                .subject
                .classes
                .contains(&"chip".to_string())
        );
    }

    #[test]
    fn interactive_subject_accepts_child_and_rejects_sibling_combinators() {
        let (sheet, report) = parse_stylesheet_full(
            r#"
            .card > .btn:hover { color: red; }
            .card + .btn:hover { color: red; }
            .card ~ .btn:hover { color: red; }
            .btn:hover { color: green; }
            "#,
            0,
        );
        assert_eq!(sheet.interactive_rules.len(), 2);
        assert_eq!(report.skipped_selectors, 2);
        assert_eq!(
            sheet.interactive_rules[0].selector.ancestors[0].0,
            Combinator::Child
        );
        assert_eq!(
            sheet.interactive_rules[0].selector.ancestors[0].1.classes,
            ["card"]
        );
        assert_eq!(sheet.interactive_rules[0].selector.subject.classes, ["btn"]);
        assert_eq!(sheet.interactive_rules[1].selector.subject.classes, ["btn"]);
    }

    #[test]
    fn motion_on_hover_rule_is_stored_on_interactive_rule() {
        let (sheet, _) = parse_stylesheet_full(
            ".btn:hover { transition: opacity 0.2s ease; color: red; }",
            0,
        );
        assert!(sheet.static_rules.is_empty());
        assert_eq!(sheet.interactive_rules.len(), 1);
        let rule = &sheet.interactive_rules[0];
        assert_eq!(rule.motion.transition.as_deref(), Some("opacity 0.2s ease"));
        assert!(
            rule.declaration_entries
                .iter()
                .any(|e| e.property == "color")
        );
    }

    #[test]
    fn motion_on_generated_pseudo_is_stored() {
        let (sheet, _) =
            parse_stylesheet_full(".ok::before { animation: spin 1s; content: \"\"; }", 0);
        assert_eq!(sheet.generated_pseudo_rules.len(), 1);
        assert_eq!(
            sheet.generated_pseudo_rules[0].motion.animation.as_deref(),
            Some("spin 1s")
        );
    }

    #[test]
    fn parse_stylesheet_omits_interactive_buckets() {
        let rules = parse_stylesheet(".ok:hover { height: 50%; } .ok { height: 100%; }", 0);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].declaration_entries[0].value, "100%");

        let (rules2, report) = parse_stylesheet_with_report(
            ".card:hover .icon { width: 1px; } .card { width: 2px; }",
            0,
        );
        assert_eq!(rules2.len(), 1);
        assert_eq!(rules2[0].declaration_entries[0].value, "2px");
        assert_eq!(report.skipped_selectors, 0);
    }

    #[test]
    fn interactive_important_preserved_in_entries() {
        let (sheet, _) = parse_stylesheet_full(".btn:hover { color: red !important; }", 0);
        assert_eq!(sheet.interactive_rules.len(), 1);
        let entry = &sheet.interactive_rules[0].declaration_entries[0];
        assert!(entry.important);
        assert_eq!(
            sheet.interactive_rules[0].declarations,
            "color: red !important"
        );
    }

    #[test]
    fn webkit_keyframes_and_bad_stop_skipped() {
        let css =
            "@-webkit-keyframes fade { from { opacity: 0 } bad { color: red } 50% { opacity: 1 } }";
        let (rule, _) = crate::css_interactive::parse_keyframes_at_rule(css, 0).expect("keyframes");
        assert_eq!(rule.name, "fade");
        assert_eq!(rule.blocks.len(), 2);
    }

    #[test]
    fn keyframe_bare_number_without_percent_rejected() {
        assert!(
            crate::css_interactive::parse_keyframes_at_rule(
                "@keyframes x { 50 { opacity: 1 } }",
                0
            )
            .is_none()
        );
    }

    #[test]
    fn multiple_interactive_pseudos_skipped() {
        let (sheet, report) = parse_stylesheet_full(
            ".btn:hover:focus { color: red; } .btn:hover { color: blue; }",
            0,
        );
        assert_eq!(sheet.interactive_rules.len(), 1);
        assert_eq!(report.skipped_selectors, 1);
    }

    #[test]
    fn root_and_data_theme_specificity() {
        let rules = parse_stylesheet(
            r#"
            :root { gap: 4px; }
            :root[data-theme="dark"] { padding: 16px; }
            :root[data-theme="light"] { padding: 8px; }
            "#,
            0,
        );
        assert_eq!(rules.len(), 3);
        // Specificity: :root[data-theme] (0,2,0) > :root (0,1,0).
        let sel_root = &rules[0].selectors[0];
        let sel_dark = &rules[1].selectors[0];
        assert!(sel_dark.specificity > sel_root.specificity);
    }

    #[test]
    fn is_and_where_specificity_per_mdn() {
        // :is() takes max argument specificity; :where() is always 0.
        let rules = parse_stylesheet(
            r#"
            :where(.anon-a, #anon-id) { gap: 1px; }
            :is(.anon-a, #anon-id) { gap: 2px; }
            .anon-a { gap: 3px; }
            "#,
            0,
        );
        assert_eq!(rules.len(), 3);
        let where_spec = rules[0].selectors[0].specificity;
        let is_spec = rules[1].selectors[0].specificity;
        let class_spec = rules[2].selectors[0].specificity;
        assert_eq!(where_spec, Specificity::default());
        // :is(.anon-a, #anon-id) → max = (1,0,0)
        assert_eq!(
            is_spec,
            Specificity {
                ids: 1,
                classes_attrs: 0,
                types: 0
            }
        );
        assert_eq!(
            class_spec,
            Specificity {
                ids: 0,
                classes_attrs: 1,
                types: 0
            }
        );
    }

    #[test]
    fn not_comma_list_uses_max_specificity() {
        let rules = parse_stylesheet(".a:not(.b, #c) { width: 10px; }", 0);
        assert_eq!(rules.len(), 1);
        // .a (0,1,0) + max(.b, #c)=(1,0,0) ⇒ (1,1,0)
        assert_eq!(
            rules[0].selectors[0].specificity,
            Specificity {
                ids: 1,
                classes_attrs: 1,
                types: 0
            }
        );
    }

    #[test]
    fn important_flag_case_and_whitespace() {
        assert_eq!(
            crate::css_map::split_important_flag("10px !important"),
            ("10px".into(), true)
        );
        assert_eq!(
            crate::css_map::split_important_flag("10px!IMPORTANT"),
            ("10px".into(), true)
        );
        assert_eq!(
            crate::css_map::split_important_flag("10px ! Important"),
            ("10px".into(), true)
        );
        assert_eq!(
            crate::css_map::split_important_flag("10px"),
            ("10px".into(), false)
        );
        assert_eq!(
            crate::css_map::split_important_flag("calc(1px + 2px)"),
            ("calc(1px + 2px)".into(), false)
        );

        let rules = parse_stylesheet(".anon { gap: 8px ! IMPORTANT; }", 0);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].declaration_entries[0].important);
        assert_eq!(rules[0].declaration_entries[0].value, "8px");
    }

    #[test]
    fn an_plus_b_parse_and_match_index() {
        assert_eq!(parse_an_plus_b("odd"), Some(AnPlusB::odd()));
        assert_eq!(parse_an_plus_b("even"), Some(AnPlusB::even()));
        assert_eq!(parse_an_plus_b("3"), Some(AnPlusB { a: 0, b: 3 }));
        assert_eq!(parse_an_plus_b("2n+1"), Some(AnPlusB { a: 2, b: 1 }));
        assert_eq!(parse_an_plus_b("2n + 1"), Some(AnPlusB { a: 2, b: 1 }));
        assert_eq!(parse_an_plus_b("-n+3"), Some(AnPlusB { a: -1, b: 3 }));
        assert_eq!(parse_an_plus_b("n"), Some(AnPlusB { a: 1, b: 0 }));
        assert_eq!(parse_an_plus_b("3n"), Some(AnPlusB { a: 3, b: 0 }));
        assert!(parse_an_plus_b("n2").is_none());
        assert!(parse_an_plus_b("").is_none());

        let odd = AnPlusB::odd();
        assert!(odd.matches_index(1));
        assert!(!odd.matches_index(2));
        assert!(odd.matches_index(5));
        let first_three = AnPlusB { a: -1, b: 3 };
        assert!(first_three.matches_index(1));
        assert!(first_three.matches_index(3));
        assert!(!first_three.matches_index(4));
    }

    #[test]
    fn nth_child_and_nth_of_type_parse_specificity() {
        let rules = parse_stylesheet(
            r#"
            .row > :nth-child(odd) { width: 10px; }
            .row > :nth-child(2n) { height: 20px; }
            .row > p:nth-of-type(2) { padding: 8px; }
            .row > :nth-child(-n+2) { gap: 4px; }
            "#,
            0,
        );
        assert_eq!(rules.len(), 4);
        // Specificity: :nth-child counts as one class-level pseudo.
        let sel = &rules[0].selectors[0];
        assert_eq!(
            sel.specificity,
            Specificity {
                ids: 0,
                classes_attrs: 2, // .row + :nth-child
                types: 0
            }
        );
    }

    #[test]
    fn nth_of_clause_and_invalid_deferred() {
        assert!(parse_stylesheet("li:nth-child(even of .noted) { gap: 1px; }", 0).is_empty());
        assert!(parse_stylesheet("p:nth-of-type(2 of .x) { gap: 1px; }", 0).is_empty());
        let last = parse_stylesheet(":nth-last-child(2) { gap: 1px; }", 0);
        assert_eq!(last.len(), 1);
        assert_eq!(
            last[0].selectors[0].subject.nth_last_child,
            Some(AnPlusB { a: 0, b: 2 })
        );
        let ok = parse_stylesheet(":nth-child(2n+1) { gap: 1px; }", 0);
        assert_eq!(ok.len(), 1);
        assert_eq!(
            ok[0].selectors[0].subject.nth_child,
            Some(AnPlusB { a: 2, b: 1 })
        );
    }

    #[test]
    fn attribute_operators_and_case_flags_parse() {
        let rules = parse_stylesheet(
            r##"
            [title] { gap: 1px; }
            [data-role="admin"] { width: 10px; }
            [class~="logo"] { height: 20px; }
            [lang|="zh"] { padding: 8px; }
            [href^="#"] { margin: 4px; }
            [href$=".org"] { border-radius: 2px; }
            [href*="example"] { min-width: 40px; }
            [href*="org" i] { max-width: 50px; }
            [data-id="AbC" s] { min-height: 12px; }
            "##,
            0,
        );
        assert_eq!(rules.len(), 9);
        assert_eq!(
            rules[2].selectors[0].subject.attrs[0].op,
            AttrOperator::Includes
        );
        assert_eq!(
            rules[7].selectors[0].subject.attrs[0].case,
            AttrCase::Insensitive
        );
        assert_eq!(
            rules[8].selectors[0].subject.attrs[0].case,
            AttrCase::Sensitive
        );
    }

    #[test]
    fn declaration_entries_cached_at_parse() {
        let rules = parse_stylesheet(
            ".anon { width: 10px; height: 20px !important; color: red; }",
            0,
        );
        assert_eq!(rules.len(), 1);
        let entries = &rules[0].declaration_entries;
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].property, "width");
        assert_eq!(entries[0].value, "10px");
        assert!(!entries[0].important);
        assert_eq!(entries[1].property, "height");
        assert_eq!(entries[1].value, "20px");
        assert!(entries[1].important);
        assert_eq!(entries[2].text(), "color: red");

        // Re-parse of raw block must match cached entries (index / important / text).
        let again = parse_declaration_entries(&rules[0].declarations);
        assert_eq!(again, *entries);
    }

    #[test]
    fn document_vars_from_rules_match_theme_scrape() {
        let css = r#"
            :root { --bg: #111; --fg: #eee; }
            :root[data-theme="light"] { --bg: #fff; }
            :root[data-theme="dark"] { --bg: #000; }
            .surface { --row-h: 28px; }
            :root, .surface { --shared: 1px; }
        "#;
        let rules = parse_stylesheet(css, 0);
        let from_rules = collect_document_custom_properties_from_rules(&rules, "light");
        let from_text = crate::css_map::collect_document_css_custom_properties(css, "light");
        assert_eq!(
            from_rules.get("--bg"),
            from_text.get("--bg").map(String::as_str)
        );
        assert_eq!(
            from_rules.get("--fg"),
            from_text.get("--fg").map(String::as_str)
        );
        assert_eq!(
            from_rules.get("--shared"),
            from_text.get("--shared").map(String::as_str)
        );
        assert!(
            from_rules.get("--row-h").is_none(),
            "element-scoped .surface --* must stay out of document base"
        );

        let dark = collect_document_custom_properties_from_rules(&rules, "dark");
        assert_eq!(dark.get("--bg"), Some("#000"));
    }

    #[test]
    fn import_merges_into_same_cascade() {
        use crate::css_at_rule::{MemoryStylesheetLoader, ParseStylesheetOptions};
        let mut files = std::collections::HashMap::new();
        files.insert("theme.css".into(), ".imported { width: 40px; }".into());
        let loader = MemoryStylesheetLoader { files };
        let mut options = ParseStylesheetOptions {
            loader: Some(&loader),
            base_href: Some("main.css"),
            ..ParseStylesheetOptions::default()
        };
        let (sheet, report) = parse_stylesheet_full_with_options(
            r#"
            @import url("theme.css");
            .local { height: 20px; }
            "#,
            0,
            &mut options,
        );
        assert_eq!(report.imported_sheets, 1);
        assert_eq!(report.skipped_at_rules, 0);
        let env = crate::css_at_rule::MediaEnvironment::default();
        let flat = sheet.flatten(&env);
        assert_eq!(flat.static_rules.len(), 2);
        let names: Vec<String> = flat
            .static_rules
            .iter()
            .filter_map(|r| r.selectors.first()?.subject.classes.first().cloned())
            .collect();
        assert_eq!(names, vec!["imported".to_string(), "local".to_string()]);
        assert!(flat.static_rules[0].source_order < flat.static_rules[1].source_order);
    }

    #[test]
    fn import_cycle_is_skipped_once() {
        use crate::css_at_rule::{MemoryStylesheetLoader, ParseStylesheetOptions};
        let mut files = std::collections::HashMap::new();
        files.insert(
            "a.css".into(),
            "@import \"b.css\"; .a { width: 1px; }".into(),
        );
        files.insert(
            "b.css".into(),
            "@import \"a.css\"; .b { width: 2px; }".into(),
        );
        let loader = MemoryStylesheetLoader { files };
        let mut options = ParseStylesheetOptions {
            loader: Some(&loader),
            base_href: Some("root.css"),
            ..ParseStylesheetOptions::default()
        };
        let (sheet, report) = parse_stylesheet_full_with_options(
            "@import \"a.css\"; .root { width: 3px; }",
            0,
            &mut options,
        );
        assert!(report.skipped_at_rules >= 1, "cycle must increment skip");
        assert!(report.imported_sheets >= 1);
        let flat = sheet.flatten(&crate::css_at_rule::MediaEnvironment::default());
        let names: Vec<String> = flat
            .static_rules
            .iter()
            .filter_map(|r| r.selectors.first()?.subject.classes.first().cloned())
            .collect();
        assert!(names.contains(&"a".to_string()));
        assert!(names.contains(&"b".to_string()));
        assert!(names.contains(&"root".to_string()));
        assert_eq!(names.len(), 3, "each sheet once: {names:?}");
    }

    #[test]
    fn media_min_width_apply_and_skip() {
        let (sheet, report) = parse_stylesheet_full(
            "@media (min-width: 800px) { .wide { width: 100px; } } .always { height: 8px; }",
            0,
        );
        assert_eq!(report.skipped_at_rules, 0);
        assert_eq!(sheet.media_rules.len(), 1);
        assert_eq!(sheet.static_rules.len(), 1);

        let wide = crate::css_at_rule::MediaEnvironment {
            width: 900.0,
            height: 500.0,
            color_scheme_dark: false,
        };
        let narrow = crate::css_at_rule::MediaEnvironment {
            width: 400.0,
            height: 500.0,
            color_scheme_dark: false,
        };
        let applied = sheet.flatten(&wide);
        assert_eq!(applied.static_rules.len(), 2);
        let skipped = sheet.flatten(&narrow);
        assert_eq!(skipped.static_rules.len(), 1);
        assert_eq!(
            skipped.static_rules[0].selectors[0].subject.classes[0],
            "always"
        );
    }

    #[test]
    fn font_face_parses_family_src_weight() {
        let (sheet, report) = parse_stylesheet_full(
            r#"
            @font-face {
                font-family: "Display";
                src: url("./Display.woff2") format("woff2");
                font-weight: bold;
            }
            .use { font-family: Display; }
            "#,
            0,
        );
        assert_eq!(report.skipped_at_rules, 0);
        assert_eq!(sheet.font_faces.len(), 1);
        assert_eq!(sheet.font_faces[0].family, "Display");
        assert_eq!(sheet.font_faces[0].weight, Some(700));
        assert_eq!(
            sheet.font_faces[0].src[0],
            crate::css_at_rule::FontFaceSrc::Url("./Display.woff2".into())
        );
        assert_eq!(sheet.static_rules.len(), 1);
    }

    #[test]
    fn supports_display_flex_applies_lab_skips() {
        let (sheet, report) = parse_stylesheet_full(
            concat!(
                "@supports (display: flex) { .ok { display: flex; width: 40px; } }",
                "@supports (color: lab(0% 0 0)) { .lab { width: 99px; } }",
            ),
            0,
        );
        assert_eq!(report.skipped_at_rules, 1);
        assert_eq!(report.rules, 1);
        assert_eq!(sheet.static_rules.len(), 1);
        assert_eq!(sheet.static_rules[0].selectors[0].subject.classes[0], "ok");
    }

    #[test]
    fn layer_inner_rules_apply_as_author_order_names_recorded() {
        // Full cascade-layer priority (unlayered beats layered) is not
        // implemented: inner rules join author source order. Names are
        // recorded first-seen. Anonymous `@layer { }` is the same flattening.
        let (sheet, report) = parse_stylesheet_full(
            r#"
            @layer base, utilities;
            @layer base { .x { width: 10px; } }
            @layer { .anon { height: 8px; } }
            .after { width: 20px; }
            "#,
            0,
        );
        assert_eq!(report.skipped_at_rules, 0);
        assert_eq!(
            sheet.layer_names,
            vec!["base".to_string(), "utilities".to_string()]
        );
        assert_eq!(sheet.static_rules.len(), 3);
        let names: Vec<String> = sheet
            .static_rules
            .iter()
            .filter_map(|r| r.selectors.first()?.subject.classes.first().cloned())
            .collect();
        assert_eq!(
            names,
            vec!["x".to_string(), "anon".to_string(), "after".to_string()]
        );
        assert!(sheet.static_rules[0].source_order < sheet.static_rules[1].source_order);
        assert!(sheet.static_rules[1].source_order < sheet.static_rules[2].source_order);
    }

    #[test]
    fn layer_invalid_prelude_skips_whole_block() {
        let (sheet, report) = parse_stylesheet_full(
            "@layer foo bar { .x { width: 1px; } } .kept { height: 2px; }",
            0,
        );
        assert_eq!(report.skipped_at_rules, 1);
        assert_eq!(sheet.static_rules.len(), 1);
        assert_eq!(
            sheet.static_rules[0].selectors[0].subject.classes[0],
            "kept"
        );
    }

    #[test]
    fn import_supports_and_layer_do_not_load() {
        use crate::css_at_rule::{MemoryStylesheetLoader, ParseStylesheetOptions};
        let mut files = std::collections::HashMap::new();
        files.insert("theme.css".into(), ".imported { width: 40px; }".into());
        let loader = MemoryStylesheetLoader { files };
        let mut options = ParseStylesheetOptions {
            loader: Some(&loader),
            base_href: Some("main.css"),
            ..ParseStylesheetOptions::default()
        };
        let (sheet, report) = parse_stylesheet_full_with_options(
            r#"
            @import url("theme.css") supports(display: grid);
            @import url("theme.css") layer(utilities);
            @import "theme.css" layer;
            .local { height: 20px; }
            "#,
            0,
            &mut options,
        );
        assert_eq!(report.imported_sheets, 0);
        assert!(report.skipped_at_rules >= 3);
        let flat = sheet.flatten(&crate::css_at_rule::MediaEnvironment::default());
        let names: Vec<String> = flat
            .static_rules
            .iter()
            .filter_map(|r| r.selectors.first()?.subject.classes.first().cloned())
            .collect();
        assert_eq!(names, vec!["local".to_string()]);
    }

    #[test]
    fn protocol_relative_import_is_skipped() {
        use crate::css_at_rule::{MemoryStylesheetLoader, ParseStylesheetOptions};
        let mut files = std::collections::HashMap::new();
        files.insert(
            "evil.example/remote.css".into(),
            ".remote { width: 9px; }".into(),
        );
        files.insert("host/share/unc.css".into(), ".unc { width: 8px; }".into());
        let loader = MemoryStylesheetLoader { files };
        let mut options = ParseStylesheetOptions {
            loader: Some(&loader),
            ..ParseStylesheetOptions::default()
        };
        let (sheet, report) = parse_stylesheet_full_with_options(
            r#"
            @import url("//evil.example/remote.css");
            @import url("\\host\share\unc.css");
            .local { height: 2px; }
            "#,
            0,
            &mut options,
        );
        assert_eq!(report.imported_sheets, 0);
        assert!(report.skipped_at_rules >= 2);
        let flat = sheet.flatten(&crate::css_at_rule::MediaEnvironment::default());
        assert_eq!(flat.static_rules.len(), 1);
        assert_eq!(
            flat.static_rules[0].selectors[0].subject.classes[0],
            "local"
        );
    }

    #[test]
    fn escaped_supports_import_is_skipped() {
        use crate::css_at_rule::{MemoryStylesheetLoader, ParseStylesheetOptions};
        let mut files = std::collections::HashMap::new();
        files.insert("theme.css".into(), ".imported { width: 40px; }".into());
        let loader = MemoryStylesheetLoader { files };
        let mut options = ParseStylesheetOptions {
            loader: Some(&loader),
            base_href: Some("main.css"),
            ..ParseStylesheetOptions::default()
        };
        let (sheet, report) = parse_stylesheet_full_with_options(
            r#"
            @import url("theme.css") supp\6frts(display: grid);
            @import url("theme.css") l\61yer;
            .local { height: 20px; }
            "#,
            0,
            &mut options,
        );
        assert_eq!(report.imported_sheets, 0);
        assert!(report.skipped_at_rules >= 2);
        let flat = sheet.flatten(&crate::css_at_rule::MediaEnvironment::default());
        let names: Vec<String> = flat
            .static_rules
            .iter()
            .filter_map(|r| r.selectors.first()?.subject.classes.first().cloned())
            .collect();
        assert_eq!(names, vec!["local".to_string()]);
    }

    #[test]
    fn http_and_data_import_are_skipped() {
        use crate::css_at_rule::{MemoryStylesheetLoader, ParseStylesheetOptions};
        let mut files = std::collections::HashMap::new();
        files.insert(
            "https://example.com/remote.css".into(),
            ".remote { width: 9px; }".into(),
        );
        files.insert("data:text/css,.x{}".into(), ".data { width: 8px; }".into());
        let loader = MemoryStylesheetLoader { files };
        let mut options = ParseStylesheetOptions {
            loader: Some(&loader),
            ..ParseStylesheetOptions::default()
        };
        let (sheet, report) = parse_stylesheet_full_with_options(
            r#"
            @import url("https://example.com/remote.css");
            @import url("http://example.com/remote.css");
            @import url("data:text/css,.x{}");
            .local { height: 2px; }
            "#,
            0,
            &mut options,
        );
        assert_eq!(report.imported_sheets, 0);
        assert!(report.skipped_at_rules >= 3);
        let flat = sheet.flatten(&crate::css_at_rule::MediaEnvironment::default());
        assert_eq!(flat.static_rules.len(), 1);
        assert_eq!(
            flat.static_rules[0].selectors[0].subject.classes[0],
            "local"
        );
    }

    #[test]
    fn print_media_rules_and_font_faces_are_not_applied_on_screen() {
        let (sheet, report) = parse_stylesheet_full(
            r#"
            @media print {
                .print { width: 10px; }
                @font-face {
                    font-family: "PrintOnly";
                    src: url("./print.ttf");
                }
            }
            .screen { height: 8px; }
            "#,
            0,
        );
        assert_eq!(report.skipped_at_rules, 0);
        let flat = sheet.flatten(&crate::css_at_rule::MediaEnvironment::default());
        assert_eq!(flat.static_rules.len(), 1);
        assert_eq!(
            flat.static_rules[0].selectors[0].subject.classes[0],
            "screen"
        );
        assert!(
            flat.font_faces.is_empty(),
            "print @font-face must not flatten on screen"
        );
        assert_eq!(sheet.all_font_faces().len(), 1);
    }

    #[test]
    fn filesystem_jail_rejects_dotdot_and_absolute_escape() {
        use crate::css_at_rule::{FsStylesheetLoader, ParseStylesheetOptions};
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "nanaui-css-jail-{}-{seq}-escape",
            std::process::id()
        ));
        let jail = root.join("jail");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&jail).expect("jail");
        std::fs::write(root.join("secret.css"), ".escaped { width: 99px; }").expect("secret");
        let abs = root
            .join("secret.css")
            .canonicalize()
            .expect("abs")
            .to_string_lossy()
            .replace('\\', "/");
        let loader = FsStylesheetLoader { base: &jail };
        let mut options = ParseStylesheetOptions {
            loader: Some(&loader),
            ..ParseStylesheetOptions::default()
        };
        let (sheet, report) = parse_stylesheet_full_with_options(
            &format!(
                r#"
                @import url("../secret.css");
                @import url("{abs}");
                .local {{ height: 3px; }}
                "#
            ),
            0,
            &mut options,
        );
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(report.imported_sheets, 0);
        assert!(report.skipped_at_rules >= 2);
        let flat = sheet.flatten(&crate::css_at_rule::MediaEnvironment::default());
        assert_eq!(flat.static_rules.len(), 1);
        assert_eq!(
            flat.static_rules[0].selectors[0].subject.classes[0],
            "local"
        );
    }

    #[test]
    fn stylesheet_read_size_cap_skips_oversize_import() {
        use crate::css_at_rule::{
            FsStylesheetLoader, ParseStylesheetOptions, with_stylesheet_byte_cap,
        };
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let jail =
            std::env::temp_dir().join(format!("nanaui-css-jail-{}-{seq}-cap", std::process::id()));
        let _ = std::fs::remove_dir_all(&jail);
        std::fs::create_dir_all(&jail).expect("jail");
        std::fs::write(jail.join("big.css"), "x".repeat(64)).expect("big");
        let loader = FsStylesheetLoader { base: &jail };
        let (sheet, report) = with_stylesheet_byte_cap(32, || {
            let mut options = ParseStylesheetOptions {
                loader: Some(&loader),
                ..ParseStylesheetOptions::default()
            };
            parse_stylesheet_full_with_options(
                "@import url(\"big.css\"); .local { height: 4px; }",
                0,
                &mut options,
            )
        });
        let _ = std::fs::remove_dir_all(&jail);
        assert_eq!(report.imported_sheets, 0);
        assert!(report.skipped_at_rules >= 1);
        let flat = sheet.flatten(&crate::css_at_rule::MediaEnvironment::default());
        assert_eq!(flat.static_rules.len(), 1);
    }

    #[test]
    fn font_face_url_resolves_relative_to_declaring_sheet() {
        use crate::css_at_rule::{
            FsStylesheetLoader, ParseStylesheetOptions, font_face_url_src, load_font_face_bytes,
        };
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let jail = std::env::temp_dir().join(format!(
            "nanaui-css-jail-{}-{seq}-font-rel",
            std::process::id()
        ));
        let sheets = jail.join("sheets");
        let fonts = sheets.join("fonts");
        let _ = std::fs::remove_dir_all(&jail);
        std::fs::create_dir_all(&fonts).expect("fonts");
        std::fs::write(fonts.join("n.ttf"), b"dummy-font").expect("ttf");
        std::fs::write(
            sheets.join("theme.css"),
            r#"
            @font-face {
                font-family: "Rel";
                src: url("./fonts/n.ttf");
            }
            "#,
        )
        .expect("theme");
        let loader = FsStylesheetLoader { base: &jail };
        let mut options = ParseStylesheetOptions {
            loader: Some(&loader),
            ..ParseStylesheetOptions::default()
        };
        let (sheet, report) = parse_stylesheet_full_with_options(
            "@import url(\"sheets/theme.css\");",
            0,
            &mut options,
        );
        assert_eq!(report.imported_sheets, 1);
        assert_eq!(report.skipped_at_rules, 0);
        assert_eq!(sheet.font_faces.len(), 1);
        let face = &sheet.font_faces[0];
        assert_eq!(face.family, "Rel");
        let base = face.base_href.as_deref().expect("declaring sheet href");
        let base_norm = base.replace('\\', "/");
        assert!(
            base_norm.ends_with("theme.css"),
            "base_href should be the declaring sheet, got {base}"
        );
        let url = font_face_url_src(face).expect("url");
        let loaded = load_font_face_bytes(url, face.base_href.as_deref(), &jail);
        assert!(
            loaded.is_some(),
            "font url must resolve relative to the declaring sheet"
        );
        assert!(
            load_font_face_bytes(url, None, &jail).is_none(),
            "resolving against the root jail must miss sheets/fonts/n.ttf"
        );
        let _ = std::fs::remove_dir_all(&jail);
    }

    #[test]
    fn late_import_after_style_rule_is_skipped() {
        use crate::css_at_rule::{MemoryStylesheetLoader, ParseStylesheetOptions};
        let mut files = std::collections::HashMap::new();
        files.insert("late.css".into(), ".imported { width: 40px; }".into());
        let loader = MemoryStylesheetLoader { files };
        let mut options = ParseStylesheetOptions {
            loader: Some(&loader),
            base_href: Some("main.css"),
            ..ParseStylesheetOptions::default()
        };
        let (sheet, report) = parse_stylesheet_full_with_options(
            r#"
            .local { height: 20px; }
            @import url("late.css");
            "#,
            0,
            &mut options,
        );
        assert!(
            report.skipped_at_rules >= 1,
            "CSS ignores @import after a style rule"
        );
        assert_eq!(report.imported_sheets, 0);
        let flat = sheet.flatten(&crate::css_at_rule::MediaEnvironment::default());
        assert_eq!(flat.static_rules.len(), 1);
        assert_eq!(
            flat.static_rules[0].selectors[0].subject.classes[0],
            "local"
        );
    }

    #[test]
    fn nested_import_inside_media_is_skipped() {
        use crate::css_at_rule::{MemoryStylesheetLoader, ParseStylesheetOptions};
        let mut files = std::collections::HashMap::new();
        files.insert("x.css".into(), ".imported { width: 40px; }".into());
        let loader = MemoryStylesheetLoader { files };
        let mut options = ParseStylesheetOptions {
            loader: Some(&loader),
            base_href: Some("main.css"),
            ..ParseStylesheetOptions::default()
        };
        let (sheet, report) = parse_stylesheet_full_with_options(
            r#"@media screen { @import url(x.css); .a{color:red} }"#,
            0,
            &mut options,
        );
        assert!(
            report.skipped_at_rules >= 1,
            "CSS ignores nested @import inside @media"
        );
        assert_eq!(report.imported_sheets, 0);
        let env = crate::css_at_rule::MediaEnvironment::default();
        let flat = sheet.flatten(&env);
        assert_eq!(flat.static_rules.len(), 1);
        assert_eq!(flat.static_rules[0].selectors[0].subject.classes[0], "a");
    }

    #[test]
    fn subject_disabled_compiles_to_attr_presence() {
        let rules = parse_stylesheet("button:disabled { width: 10px; }", 0);
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selectors[0].subject.attrs[0].name, "disabled",
            "subject :disabled compiles to [disabled] presence"
        );
    }

    #[test]
    fn checked_adjacent_sibling_label_parses() {
        let rules = parse_stylesheet("input:checked + label { width: 16px; }", 0);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].selectors[0].ancestors[0].1.checked);
    }

    #[test]
    fn empty_and_not_empty_parse() {
        let rules = parse_stylesheet(
            r#"
            .box:empty { width: 8px; }
            .box:not(:empty) { height: 12px; }
            "#,
            0,
        );
        assert_eq!(rules.len(), 2);
        assert!(rules[0].selectors[0].subject.empty);
        assert!(rules[1].selectors[0].subject.not_alts[0].empty);
    }

    #[test]
    fn only_child_and_of_type_and_nth_last_child_parse() {
        let rules = parse_stylesheet(
            r#"
            .row > :only-child { width: 10px; }
            .row > p:first-of-type { height: 20px; }
            .row > p:last-of-type { padding: 8px; }
            .row > :nth-last-child(1) { gap: 4px; }
            .row > :nth-last-child(odd) { margin: 2px; }
            "#,
            0,
        );
        assert_eq!(rules.len(), 5);
        assert!(rules[0].selectors[0].subject.only_child);
        assert!(rules[1].selectors[0].subject.first_of_type);
        assert!(rules[2].selectors[0].subject.last_of_type);
        assert_eq!(
            rules[3].selectors[0].subject.nth_last_child,
            Some(AnPlusB { a: 0, b: 1 })
        );
        assert_eq!(
            rules[4].selectors[0].subject.nth_last_child,
            Some(AnPlusB::odd())
        );
    }
}
