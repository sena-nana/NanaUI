//! Stylesheet → matched nodes → [`LayoutStyle`] cascade (L1 subset).
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
//! simple `:not()` / `:is()` / `:where()`;
//! author-layer `!important` (normal then important; specificity + source order
//! within each); then prop style and inline style; then stylesheet `!important`
//! (beats prop/inline normals); then prop / inline `!important` so author-important
//! inline beats stylesheet important (see [`rebuild_layout_style`]).
//!
//! After cascade, documented shell / utility classes are applied via
//! [`crate::shell_contract`]（非中立；不在此模块扩展 class 特判）.
//!
//! Parsed into separate buckets (not static cascade): `:hover`/`:focus`/`:active`,
//! `::before`/`::after`, `@keyframes`, and transition/animation longhands — see
//! [`crate::css_interactive::ParsedStylesheet`].
//!
//! Cheap `:disabled` (subject or `:not(:disabled)`) is the same present-check
//! as `[disabled]` (HTML `disabled` attr / form-control disabled). Cheap
//! `:checked` matches only checkable hosts (checkbox / radio / switch)
//! when `WidgetProps.toggled` is true — leftover `checked` attrs on
//! dialogs / other nodes do not match. Sibling combinators `+` / `~`
//! already walk preceding siblings. `:empty` / `:not(:empty)`: no element
//! children, no host `label`/`value` with a non-whitespace Unicode scalar
//! (`char::is_whitespace`), and no child text of that kind. Text nodes are
//! `#text` / `createText` / `nana-text` only — L2 `p` / `span` / `label` /
//! headings are elements even when their kind is `WidgetKind::Text`.
//! Generated `::before`/`::after` boxes and whitespace-only text do not count.
//! `:first-child` / `:last-child` / `:only-child` / `:nth-*` sibling
//! counts exclude generated boxes and those text nodes. Other `:not()` args stay simple
//! compounds. Cheap subject `:focus-within` matches when the subject or a
//! descendant is focused; ancestor `:focus-within` is skipped.
//! `:focus-visible` is stored as `:focus` (no keyboard-vs-pointer signal);
//! it never matches without focus.
//! Still deferred (skipped at parse): combinators inside `:has()`,
//! `:nth-child(… of …)`, nested / complex `:not()` args, unknown at-rules,
//! unknown `@supports` predicates, `@import … layer()` / `supports()`,
//! and cascade-layer *priority* (unlayered vs layered).
//! `::placeholder` is parsed with generated pseudos and applied as TextInput
//! placeholder paint (not a generated box).
//! Cheap subject `:has(.class|#id|type)` descendant-present is matched with an
//! O(n·k) precomputed bitset (k = unique simple `:has` args, cap 64).
//! `@import` / `@media` (width/height/orientation/prefers-color-scheme) /
//! `@font-face` / matching `@supports` / `@layer { }` (author source order,
//! names recorded) merge into the same [`ParsedStylesheet`] (not a second cascade).

use std::collections::{BTreeMap, HashMap};

use crate::css_at_rule::{
    ImportPrelude, MAX_IMPORT_DEPTH, ParseStylesheetOptions, evaluate_supports_condition,
    font_face_from_pairs, is_blocked_href, parse_import_prelude, parse_layer_prelude,
    parse_media_query_list,
};
use crate::css_interactive::{
    GeneratedPseudo, GeneratedPseudoRule, InteractivePseudo, InteractiveSelector,
    InteractiveStyleRule, MediaRule, MotionDeclarations, MotionStyleRule, ParsedStylesheet,
    merge_parsed_stylesheet, offset_source_order, parse_keyframes_at_rule,
    partition_motion_entries,
};
use crate::css_map::{
    LayoutStyle, LayoutStyleCss, css_key_is_direction_or_writing_mode, split_important_flag,
};

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

/// One ancestor/subject/sibling node's identity facts (flat; no nested refs).
#[derive(Debug, Clone, Copy)]
pub struct MatchNode<'a> {
    pub tag: &'a str,
    pub id: &'a str,
    pub classes: &'a [String],
    pub attrs: &'a BTreeMap<String, String>,
    /// True when the node has no element children and no non-whitespace text.
    pub is_empty: bool,
    /// Checkable host with `toggled` (`:checked`).
    pub checked: bool,
}

/// Element facts used for selector matching (bridge / tree).
#[derive(Debug, Clone, Copy)]
pub struct MatchContext<'a> {
    pub tag: &'a str,
    pub id: &'a str,
    pub classes: &'a [String],
    pub attrs: &'a BTreeMap<String, String>,
    /// Full parent chain toward root (immediate parent first). Arbitrary depth.
    pub ancestors: &'a [MatchNode<'a>],
    /// Preceding siblings among the parent's children, **immediate previous first**.
    /// Required for `+` / `~`; empty ⇒ sibling combinators fail closed.
    pub preceding_siblings: &'a [MatchNode<'a>],
    /// Among parent's children: index 0 = first child.
    pub sibling_index: usize,
    /// Parent's child count (1 ⇒ both first and last).
    pub sibling_count: usize,
    /// Among parent's children with the **same tag** (ASCII case-insensitive):
    /// index 0 = first of that type. Used by `:nth-of-type`.
    pub of_type_index: usize,
    /// Count of siblings (including self) sharing this element's tag.
    pub of_type_count: usize,
    /// Bit i is set when a descendant matches [`Self::has_args`]`[i]`.
    /// Precomputed once per cascade pass (O(n·k), not per-subject subtree walk).
    pub has_bits: u64,
    /// Unique simple compounds used by subject `:has()` in the current sheet.
    /// Empty ⇒ every `:has()` fails closed.
    pub has_args: &'a [SimpleCompound],
    /// Subject or a descendant is the focused element (`:focus-within`).
    pub focus_within: bool,
    /// Subject has no element children and no non-whitespace text (`:empty`).
    pub is_empty: bool,
    /// Checkable host with `toggled` (`:checked`).
    pub checked: bool,
}

impl<'a> MatchContext<'a> {
    fn as_node(&self) -> MatchNode<'a> {
        MatchNode {
            tag: self.tag,
            id: self.id,
            classes: self.classes,
            attrs: self.attrs,
            is_empty: self.is_empty,
            checked: self.checked,
        }
    }

    fn is_root(&self) -> bool {
        self.ancestors.is_empty()
    }
}

/// Skipped-content counters for one stylesheet parse. Lets L1 hosts surface
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
    /// `@import`, unknown at-rules). Applied `@supports` / `@layer` do not
    /// increment this.
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
        },
        true,
    );
    (sheet, report)
}

struct ImportParseCtx<'a> {
    loader: Option<&'a dyn crate::css_at_rule::StylesheetLoader>,
    stack: &'a mut Vec<String>,
    cache: &'a mut HashMap<String, ParsedStylesheet>,
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
        parse_stylesheet_into(inner_css, order, &mut inner, report, ctx, false);
        sheet.media_rules.push(MediaRule {
            query,
            sheet: inner,
        });
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
                parse_stylesheet_into(inner_css, order, sheet, report, ctx, false);
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
            parse_stylesheet_into(inner_css, order, sheet, report, ctx, false);
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
    let Some((css, canonical)) = loader.load(&href, from) else {
        report.skipped_at_rules += 1;
        return;
    };
    if ctx.stack.iter().any(|h| h.eq_ignore_ascii_case(&canonical)) {
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
        let declarations = body.trim().to_string();
        if declarations.is_empty() {
            report.skipped_declarations += 1;
            continue;
        }
        let declaration_entries = parse_declaration_entries(&declarations);
        if declaration_entries.is_empty() {
            report.skipped_declarations += 1;
            continue;
        }
        let (layout_entries, motion) = partition_motion_entries(&declaration_entries);
        if layout_entries.is_empty() && motion.is_empty() {
            report.skipped_declarations += 1;
            continue;
        }
        let layout_declarations = entries_to_declaration_text(&layout_entries);

        let mut static_selectors = Vec::new();
        let mut interactive_selectors = Vec::new();
        let mut generated = Vec::new();
        for part in split_selector_list(selector_text) {
            if let Some((originating, pseudo)) = parse_generated_pseudo_selector(part) {
                generated.push((originating, pseudo));
            } else if let Some(sel) = parse_interactive_selector(part) {
                interactive_selectors.push(sel);
            } else if let Some(sel) = parse_selector(part) {
                static_selectors.push(sel);
            } else {
                report.skipped_selectors += 1;
            }
        }

        if static_selectors.is_empty() && interactive_selectors.is_empty() && generated.is_empty() {
            continue;
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
                });
            }
        }
        if !static_selectors.is_empty() && has_motion {
            consumed = true;
            sheet.motion_rules.push(MotionStyleRule {
                selectors: static_selectors,
                motion,
                source_order: *order,
            });
        }
        if consumed {
            *order = order.saturating_add(1);
        }
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

/// Apply matched stylesheet declarations onto a fresh layout (author layer).
///
/// Uses cached [`StyleRule::declaration_entries`] and
/// [`LayoutStyleCss::apply_css_property`] — no per-match string re-split.
pub fn apply_stylesheet_to_layout(
    layout: &mut LayoutStyle,
    rules: &[StyleRule],
    ctx: &MatchContext<'_>,
    percent_w: Option<f32>,
    percent_h: Option<f32>,
) {
    apply_matched_stylesheet(layout, rules, ctx, percent_w, percent_h, false);
}

/// Matched declarations in cascade application order (later wins).
///
/// Author stylesheet layer (MDN Importance / Cascade): all **normal**
/// declarations first (specificity, then source order, then decl index), then
/// all **`!important`** declarations with the same tie-breakers. Important
/// therefore beats any normal declaration regardless of specificity.
/// Each entry is a single `property: value` with the `!important` flag stripped.
pub fn matched_declarations(
    rules: &[StyleRule],
    ctx: &MatchContext<'_>,
) -> Vec<(Specificity, u32, String)> {
    matched_declaration_entries(rules, ctx)
        .into_iter()
        .map(|(spec, order, entry)| (spec, order, entry.text()))
        .collect()
}

/// Like [`matched_declarations`], but returns cached structured entries.
pub fn matched_declaration_entries(
    rules: &[StyleRule],
    ctx: &MatchContext<'_>,
) -> Vec<(Specificity, u32, DeclarationEntry)> {
    let refs: Vec<&StyleRule> = rules.iter().collect();
    matched_declaration_entries_from(&refs, ctx)
}

fn matched_declaration_entries_from(
    rules: &[&StyleRule],
    ctx: &MatchContext<'_>,
) -> Vec<(Specificity, u32, DeclarationEntry)> {
    // (important, specificity, source_order, decl_index, entry)
    let mut matched: Vec<(bool, Specificity, u32, u32, DeclarationEntry)> = Vec::new();
    for rule in rules {
        for sel in &rule.selectors {
            if selector_matches(sel, ctx) {
                for entry in &rule.declaration_entries {
                    matched.push((
                        entry.important,
                        sel.specificity,
                        rule.source_order,
                        entry.index,
                        entry.clone(),
                    ));
                }
                break;
            }
        }
    }
    matched.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });
    matched
        .into_iter()
        .map(|(_, spec, order, _, entry)| (spec, order, entry))
        .collect()
}

/// Bucket index over rules by the subject compound's key facts (type / id /
/// class). Candidates are a strict superset of real matches —
/// [`selector_matches`] still validates every candidate — so cascade results
/// equal a linear scan at O(candidates) instead of O(all rules).
///
/// Keys follow [`compound_matches`] comparison semantics: types fold to ASCII
/// lowercase (`*` is no key), ids and classes compare exactly. A rule falls
/// into the always-check bucket when any of its selectors could match without
/// sharing a key (attr / structural-pseudo / universal subjects, or key-less
/// `:is()` / `:where()` arms).
#[derive(Debug, Default, Clone)]
pub struct RuleIndex {
    by_type: HashMap<String, Vec<u32>>,
    by_id: HashMap<String, Vec<u32>>,
    by_class: HashMap<String, Vec<u32>>,
    keyless: Vec<u32>,
}

impl RuleIndex {
    pub fn build(rules: &[StyleRule]) -> Self {
        let mut index = Self::default();
        for (order, rule) in rules.iter().enumerate() {
            let order = order as u32;
            let mut types: Vec<&str> = Vec::new();
            let mut ids: Vec<&str> = Vec::new();
            let mut classes: Vec<&str> = Vec::new();
            let mut keyless = false;
            for sel in &rule.selectors {
                if !subject_keys(&sel.subject, &mut types, &mut ids, &mut classes) {
                    keyless = true;
                    break;
                }
            }
            if keyless || (types.is_empty() && ids.is_empty() && classes.is_empty()) {
                index.keyless.push(order);
                continue;
            }
            for type_name in types {
                index
                    .by_type
                    .entry(type_name.to_ascii_lowercase())
                    .or_default()
                    .push(order);
            }
            for id in ids {
                index.by_id.entry(id.to_string()).or_default().push(order);
            }
            for class in classes {
                index
                    .by_class
                    .entry(class.to_string())
                    .or_default()
                    .push(order);
            }
        }
        index
    }

    /// Candidate rule indices for `ctx`, deduplicated in rule order.
    pub fn candidate_ids(&self, ctx: &MatchContext<'_>) -> Vec<u32> {
        let mut ids = self.keyless.clone();
        if !ctx.tag.is_empty()
            && let Some(bucket) = self.by_type.get(&ctx.tag.to_ascii_lowercase())
        {
            ids.extend_from_slice(bucket);
        }
        if !ctx.id.is_empty()
            && let Some(bucket) = self.by_id.get(ctx.id)
        {
            ids.extend_from_slice(bucket);
        }
        for class in ctx.classes {
            if let Some(bucket) = self.by_class.get(class) {
                ids.extend_from_slice(bucket);
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Candidate rules for `ctx`, in rule order.
    pub fn candidates<'a>(
        &self,
        rules: &'a [StyleRule],
        ctx: &MatchContext<'_>,
    ) -> Vec<&'a StyleRule> {
        self.candidate_ids(ctx)
            .into_iter()
            .map(|order| &rules[order as usize])
            .collect()
    }
}

/// Collect one subject compound's type / id / class keys. Returns `false` when
/// the compound can match without sharing any key (`:is()` / `:where()` arm
/// with only attr / structural pseudos), which forces the rule always-check.
/// `:not()` arms only restrict matches, so they contribute no keys.
fn subject_keys<'a>(
    compound: &'a CompoundSelector,
    types: &mut Vec<&'a str>,
    ids: &mut Vec<&'a str>,
    classes: &mut Vec<&'a str>,
) -> bool {
    if let Some(type_name) = &compound.type_name
        && type_name != "*"
    {
        types.push(type_name);
    }
    if let Some(id) = &compound.id {
        ids.push(id);
    }
    classes.extend(compound.classes.iter().map(String::as_str));
    for alt in compound.is_alts.iter().chain(compound.where_alts.iter()) {
        let mut alt_keys = 0usize;
        if let Some(type_name) = &alt.type_name
            && type_name != "*"
        {
            types.push(type_name);
            alt_keys += 1;
        }
        if let Some(id) = &alt.id {
            ids.push(id);
            alt_keys += 1;
        }
        alt_keys += alt.classes.len();
        classes.extend(alt.classes.iter().map(String::as_str));
        if alt_keys == 0 {
            return false;
        }
    }
    true
}

/// Split a declaration block into structured entries (once per rule at parse).
pub(crate) fn parse_declaration_entries(block: &str) -> Vec<DeclarationEntry> {
    let mut out = Vec::new();
    for (i, decl) in block.split(';').enumerate() {
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
) -> BTreeMap<String, String> {
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
    crate::css_map::merge_css_custom_properties(&map, &BTreeMap::new())
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

/// Custom properties from **element-scoped** rules matching `ctx` (cascade
/// order; later wins), restricted to [`RuleIndex`] candidates.
///
/// Document-level selectors (`:root`, `html`, `body`, `*`, `[data-theme=…]`, …)
/// are skipped here. Those `--*` are collected theme-aware into the bridge
/// `stylesheet_vars` base via
/// [`collect_document_custom_properties_from_rules`]. Rematching bare
/// `:root { --bg }` on parentless nodes would clobber
/// `:root[data-theme=light]` overlays (orphans report empty ancestors and
/// thus match `:root`).
pub fn matched_custom_properties_indexed(
    rules: &[StyleRule],
    index: &RuleIndex,
    ctx: &MatchContext<'_>,
) -> BTreeMap<String, String> {
    matched_custom_properties_from(&index.candidates(rules, ctx), ctx)
}

fn matched_custom_properties_from(
    rules: &[&StyleRule],
    ctx: &MatchContext<'_>,
) -> BTreeMap<String, String> {
    let mut matched: Vec<(Specificity, u32, u32, DeclarationEntry)> = Vec::new();
    for rule in rules {
        let mut best_spec: Option<Specificity> = None;
        for sel in &rule.selectors {
            if !selector_matches(sel, ctx) || selector_is_document_var_scope(sel) {
                continue;
            }
            best_spec = Some(match best_spec {
                Some(prev) if prev >= sel.specificity => prev,
                _ => sel.specificity,
            });
        }
        let Some(spec) = best_spec else {
            continue;
        };
        for entry in &rule.declaration_entries {
            if !entry.property.starts_with("--") {
                continue;
            }
            matched.push((spec, rule.source_order, entry.index, entry.clone()));
        }
    }
    matched.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut map = BTreeMap::new();
    for (_, _, _, entry) in matched {
        map.insert(entry.property, entry.value);
    }
    map
}

/// Subject is a bare document selector that receives theme-aware `--*` via
/// [`collect_document_custom_properties_from_rules`], not per-element rematch.
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

/// Rebuild layout from `base` (typically kind defaults).
///
/// Author layer order:
/// stylesheet (normal+important) → class hints → prop style → class hints →
/// inline style → class hints → stylesheet **important** only → prop
/// **important** → inline **important**.
/// Re-applying class hints after prop/inline keeps documented `nana-*` shell
/// contracts from being wiped by Vue layout props or SFC `style` declarations.
/// Inline (style attribute) wins over stylesheet **normal**. Stylesheet
/// `!important` beats prop/inline normals. Prop / inline `!important` is
/// author-important on this same path: the flag is stripped so the value
/// parses, then those declarations are written again after stylesheet
/// important (inline important beats stylesheet important).
pub fn rebuild_layout_style(
    mut layout: LayoutStyle,
    rules: &[StyleRule],
    ctx: &MatchContext<'_>,
    prop_style: &str,
    inline_style: &str,
    percent_w: Option<f32>,
    percent_h: Option<f32>,
) -> LayoutStyle {
    apply_stylesheet_to_layout(&mut layout, rules, ctx, percent_w, percent_h);
    layout.apply_class_layout_hints(ctx.classes);
    if !prop_style.trim().is_empty() {
        layout.apply_css_text(prop_style, percent_w, percent_h);
        layout.apply_class_layout_hints(ctx.classes);
    }
    if !inline_style.trim().is_empty() {
        layout.apply_css_text(inline_style, percent_w, percent_h);
        layout.apply_class_layout_hints(ctx.classes);
    }
    apply_matched_stylesheet(&mut layout, rules, ctx, percent_w, percent_h, true);
    if !prop_style.trim().is_empty() {
        apply_css_text_important_only(&mut layout, prop_style, percent_w, percent_h);
    }
    if !inline_style.trim().is_empty() {
        apply_css_text_important_only(&mut layout, inline_style, percent_w, percent_h);
    }
    layout.resolve_logical_box_edges();
    layout
}

/// [`rebuild_layout_style`] restricted to [`RuleIndex`] candidates. Cascade
/// order inside the candidate set is unchanged, so results are identical.
pub fn rebuild_layout_style_indexed(
    mut layout: LayoutStyle,
    rules: &[StyleRule],
    index: &RuleIndex,
    ctx: &MatchContext<'_>,
    prop_style: &str,
    inline_style: &str,
    percent_w: Option<f32>,
    percent_h: Option<f32>,
) -> LayoutStyle {
    let candidates = index.candidates(rules, ctx);
    apply_matched_stylesheet_from(&mut layout, &candidates, ctx, percent_w, percent_h, false);
    layout.apply_class_layout_hints(ctx.classes);
    if !prop_style.trim().is_empty() {
        layout.apply_css_text(prop_style, percent_w, percent_h);
        layout.apply_class_layout_hints(ctx.classes);
    }
    if !inline_style.trim().is_empty() {
        layout.apply_css_text(inline_style, percent_w, percent_h);
        layout.apply_class_layout_hints(ctx.classes);
    }
    apply_matched_stylesheet_from(&mut layout, &candidates, ctx, percent_w, percent_h, true);
    if !prop_style.trim().is_empty() {
        apply_css_text_important_only(&mut layout, prop_style, percent_w, percent_h);
    }
    if !inline_style.trim().is_empty() {
        apply_css_text_important_only(&mut layout, inline_style, percent_w, percent_h);
    }
    layout.resolve_logical_box_edges();
    layout
}

fn apply_matched_stylesheet(
    layout: &mut LayoutStyle,
    rules: &[StyleRule],
    ctx: &MatchContext<'_>,
    percent_w: Option<f32>,
    percent_h: Option<f32>,
    important_only: bool,
) {
    let refs: Vec<&StyleRule> = rules.iter().collect();
    apply_matched_stylesheet_from(layout, &refs, ctx, percent_w, percent_h, important_only);
}

fn apply_matched_stylesheet_from(
    layout: &mut LayoutStyle,
    rules: &[&StyleRule],
    ctx: &MatchContext<'_>,
    percent_w: Option<f32>,
    percent_h: Option<f32>,
    important_only: bool,
) {
    let mut dir_entries = Vec::new();
    let mut rest = Vec::new();
    for (_, _, entry) in matched_declaration_entries_from(rules, ctx) {
        if !important_only || entry.important {
            if css_key_is_direction_or_writing_mode(&entry.property) {
                dir_entries.push(entry);
            } else {
                rest.push(entry);
            }
        }
    }
    for entry in dir_entries.into_iter().chain(rest) {
        layout.apply_css_property(&entry.property, &entry.value, percent_w, percent_h);
    }
}

/// Apply only declarations whose value carries `!important` (flag already stripped
/// before [`LayoutStyleCss::apply_css_property`]).
fn apply_css_text_important_only(
    layout: &mut LayoutStyle,
    style: &str,
    percent_w: Option<f32>,
    percent_h: Option<f32>,
) {
    let entries: Vec<_> = parse_declaration_entries(style)
        .into_iter()
        .filter(|entry| entry.important)
        .collect();
    for entry in entries
        .iter()
        .filter(|e| css_key_is_direction_or_writing_mode(&e.property))
        .chain(
            entries
                .iter()
                .filter(|e| !css_key_is_direction_or_writing_mode(&e.property)),
        )
    {
        layout.apply_css_property(&entry.property, &entry.value, percent_w, percent_h);
    }
}

pub(crate) fn simple_matches(simple: &SimpleCompound, node: &MatchNode<'_>) -> bool {
    if let Some(tag) = &simple.type_name
        && !node.tag.eq_ignore_ascii_case(tag)
        && tag != "*"
    {
        return false;
    }
    if let Some(id) = &simple.id
        && node.id != id
    {
        return false;
    }
    for class in &simple.classes {
        if !node.classes.iter().any(|c| c == class) {
            return false;
        }
    }
    for attr in &simple.attrs {
        if !attr_matches(attr, node.attrs) {
            return false;
        }
    }
    if simple.empty && !node.is_empty {
        return false;
    }
    if simple.checked && !node.checked {
        return false;
    }
    // Empty simple (shouldn't parse) matches nothing useful — treat as true only
    // when at least one condition exists; callers always push non-empty alts.
    true
}

pub(crate) fn compound_matches(compound: &CompoundSelector, node: &MatchNode<'_>) -> bool {
    if let Some(tag) = &compound.type_name
        && !node.tag.eq_ignore_ascii_case(tag)
        && tag != "*"
    {
        return false;
    }
    if let Some(id) = &compound.id
        && node.id != id
    {
        return false;
    }
    for class in &compound.classes {
        if !node.classes.iter().any(|c| c == class) {
            return false;
        }
    }
    for attr in &compound.attrs {
        if !attr_matches(attr, node.attrs) {
            return false;
        }
    }
    if compound.empty && !node.is_empty {
        return false;
    }
    if compound.checked && !node.checked {
        return false;
    }
    for alt in &compound.not_alts {
        if simple_matches(alt, node) {
            return false;
        }
    }
    if !compound.is_alts.is_empty() && !compound.is_alts.iter().any(|a| simple_matches(a, node)) {
        return false;
    }
    if !compound.where_alts.is_empty()
        && !compound.where_alts.iter().any(|a| simple_matches(a, node))
    {
        return false;
    }
    true
}

fn compound_matches_ctx(compound: &CompoundSelector, ctx: &MatchContext<'_>) -> bool {
    if compound.root && !ctx.is_root() {
        return false;
    }
    if !compound_matches(compound, &ctx.as_node()) {
        return false;
    }
    if compound.first_child && ctx.sibling_index != 0 {
        return false;
    }
    if compound.last_child && (ctx.sibling_count == 0 || ctx.sibling_index + 1 != ctx.sibling_count)
    {
        return false;
    }
    if compound.only_child && ctx.sibling_count != 1 {
        return false;
    }
    if compound.first_of_type && ctx.of_type_index != 0 {
        return false;
    }
    if compound.last_of_type
        && (ctx.of_type_count == 0 || ctx.of_type_index + 1 != ctx.of_type_count)
    {
        return false;
    }
    if let Some(anb) = compound.nth_child {
        // CSS indices are 1-based among all siblings.
        if !anb.matches_index(ctx.sibling_index.saturating_add(1)) {
            return false;
        }
    }
    if let Some(anb) = compound.nth_of_type
        && !anb.matches_index(ctx.of_type_index.saturating_add(1))
    {
        return false;
    }
    if let Some(anb) = compound.nth_last_child {
        let from_end = ctx.sibling_count.saturating_sub(ctx.sibling_index);
        if !anb.matches_index(from_end) {
            return false;
        }
    }
    if !compound.has_queries.is_empty() {
        if ctx.has_args.is_empty() {
            return false;
        }
        for query in &compound.has_queries {
            let any = query.iter().any(|alt| {
                ctx.has_args
                    .iter()
                    .position(|have| have == alt)
                    .is_some_and(|i| i < 64 && (ctx.has_bits & (1u64 << i)) != 0)
            });
            if !any {
                return false;
            }
        }
    }
    if compound.focus_within && !ctx.focus_within {
        return false;
    }
    true
}

/// True if any rule's selector matches `ctx` (subject + combinators).
pub fn stylesheet_matches(rules: &[StyleRule], ctx: &MatchContext<'_>) -> bool {
    rules
        .iter()
        .any(|rule| rule.selectors.iter().any(|sel| selector_matches(sel, ctx)))
}

/// Cheap reject: if no rule subject *could* match this element's tag/id/classes,
/// skip building a full [`MatchContext`]. Combinators / attrs / pseudos still
/// require [`stylesheet_matches`].
pub fn stylesheet_may_match_subject(
    rules: &[StyleRule],
    tag: &str,
    id: &str,
    classes: &[String],
) -> bool {
    rules.iter().any(|rule| {
        rule.selectors
            .iter()
            .any(|sel| compound_subject_may_match(&sel.subject, tag, id, classes))
    })
}

fn compound_subject_may_match(
    compound: &CompoundSelector,
    tag: &str,
    id: &str,
    classes: &[String],
) -> bool {
    if let Some(want) = compound.type_name.as_deref()
        && want != "*"
        && !want.eq_ignore_ascii_case(tag)
    {
        return false;
    }
    if let Some(want) = compound.id.as_deref()
        && !want.is_empty()
        && want != id
    {
        return false;
    }
    for class in &compound.classes {
        if !classes.iter().any(|have| have == class) {
            return false;
        }
    }
    true
}

pub fn selector_matches(sel: &Selector, ctx: &MatchContext<'_>) -> bool {
    if !compound_matches_ctx(&sel.subject, ctx) {
        return false;
    }
    // Walk leftward from the subject. Child/descendant climb `ancestors`;
    // sibling combinators walk `preceding_siblings` (immediate previous first).
    let mut ancestors = ctx.ancestors;
    let mut preceding = ctx.preceding_siblings;
    for (comb, compound) in sel.ancestors.iter().rev() {
        match comb {
            Combinator::Child => {
                let Some((parent, rest)) = ancestors.split_first() else {
                    return false;
                };
                if !compound_matches(compound, parent) {
                    return false;
                }
                ancestors = rest;
                // Parent's preceding siblings are not in context — fail closed
                // if a further sibling combinator appears after ascent.
                preceding = &[];
            }
            Combinator::Descendant => {
                let mut found = false;
                let mut walk = ancestors;
                while let Some((parent, rest)) = walk.split_first() {
                    if compound_matches(compound, parent) {
                        ancestors = rest;
                        found = true;
                        break;
                    }
                    walk = rest;
                }
                if !found {
                    return false;
                }
                preceding = &[];
            }
            Combinator::AdjacentSibling => {
                let Some((sib, rest)) = preceding.split_first() else {
                    return false;
                };
                if !compound_matches(compound, sib) {
                    return false;
                }
                preceding = rest;
            }
            Combinator::SubsequentSibling => {
                let mut found = false;
                let mut walk = preceding;
                while let Some((sib, rest)) = walk.split_first() {
                    if compound_matches(compound, sib) {
                        preceding = rest;
                        found = true;
                        break;
                    }
                    walk = rest;
                }
                if !found {
                    return false;
                }
            }
        }
    }
    true
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
            if *comb != Combinator::Descendant {
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
    let originating_selector = parse_selector_chain(&base)?;
    Some((originating_selector, pseudo))
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
        ("::before", GeneratedPseudo::Before),
        ("::after", GeneratedPseudo::After),
        ("::placeholder", GeneratedPseudo::Placeholder),
        (":before", GeneratedPseudo::Before),
        (":after", GeneratedPseudo::After),
    ] {
        if lower.ends_with(suffix) {
            let base = s[..s.len().wrapping_sub(suffix.len())].trim();
            if !base.is_empty() {
                return Some((base.to_string(), pseudo));
            }
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
        match name {
            "where" | "is" | "not" | "nth-child" | "nth-of-type" | "nth-last-child" | "has" => {
                false
            }
            "root" | "first-child" | "last-child" | "only-child" | "first-of-type"
            | "last-of-type" | "empty" | "checked" | "disabled" | "focus-within" => false,
            _ => true,
        }
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
        if after.starts_with(':') {
            rest = &after[1..];
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

fn attr_matches(attr: &AttrSelector, attrs: &std::collections::BTreeMap<String, String>) -> bool {
    let Some(raw) = attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(&attr.name))
        .map(|(_, v)| v.as_str())
    else {
        return false;
    };
    let Some(expected) = attr.value.as_deref() else {
        return matches!(attr.op, AttrOperator::Present);
    };
    let (hay, needle) = match attr.case {
        AttrCase::Insensitive => (raw.to_ascii_lowercase(), expected.to_ascii_lowercase()),
        AttrCase::Default | AttrCase::Sensitive => (raw.to_string(), expected.to_string()),
    };
    match attr.op {
        AttrOperator::Present => true,
        AttrOperator::Equal => hay == needle,
        AttrOperator::Includes => hay.split_whitespace().any(|w| w == needle),
        AttrOperator::DashMatch => hay == needle || hay.starts_with(&(needle.clone() + "-")),
        AttrOperator::Prefix => hay.starts_with(&needle),
        AttrOperator::Suffix => hay.ends_with(&needle),
        AttrOperator::Substring => hay.contains(&needle),
    }
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

fn strip_css_comments(css: &str) -> String {
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
mod tests {
    use super::*;
    use crate::css_map::{DirSpec, FlexDirection, FlexWrap, LengthSpec};

    fn ctx<'a>(
        tag: &'a str,
        id: &'a str,
        classes: &'a [String],
        attrs: &'a BTreeMap<String, String>,
        ancestors: &'a [MatchNode<'a>],
    ) -> MatchContext<'a> {
        MatchContext {
            tag,
            id,
            classes,
            attrs,
            ancestors,
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
        }
    }

    fn ctx_full<'a>(
        tag: &'a str,
        id: &'a str,
        classes: &'a [String],
        attrs: &'a BTreeMap<String, String>,
        ancestors: &'a [MatchNode<'a>],
        preceding: &'a [MatchNode<'a>],
        sibling_index: usize,
        sibling_count: usize,
    ) -> MatchContext<'a> {
        MatchContext {
            tag,
            id,
            classes,
            attrs,
            ancestors,
            preceding_siblings: preceding,
            sibling_index,
            sibling_count,
            of_type_index: sibling_index,
            of_type_count: sibling_count,
            has_bits: 0,
            has_args: &[],
            focus_within: false,
            is_empty: true,
            checked: false,
        }
    }

    fn ctx_nth<'a>(
        tag: &'a str,
        id: &'a str,
        classes: &'a [String],
        attrs: &'a BTreeMap<String, String>,
        ancestors: &'a [MatchNode<'a>],
        sibling_index: usize,
        sibling_count: usize,
        of_type_index: usize,
        of_type_count: usize,
    ) -> MatchContext<'a> {
        MatchContext {
            tag,
            id,
            classes,
            attrs,
            ancestors,
            preceding_siblings: &[],
            sibling_index,
            sibling_count,
            of_type_index,
            of_type_count,
            has_bits: 0,
            has_args: &[],
            focus_within: false,
            is_empty: true,
            checked: false,
        }
    }

    fn node<'a>(
        tag: &'a str,
        id: &'a str,
        classes: &'a [String],
        attrs: &'a BTreeMap<String, String>,
    ) -> MatchNode<'a> {
        MatchNode {
            tag,
            id,
            classes,
            attrs,
            is_empty: true,
            checked: false,
        }
    }

    fn node_checked<'a>(
        tag: &'a str,
        id: &'a str,
        classes: &'a [String],
        attrs: &'a BTreeMap<String, String>,
    ) -> MatchNode<'a> {
        let mut node = node(tag, id, classes, attrs);
        node.checked = true;
        node
    }

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
    fn cheap_has_descendant_present_matches_subject() {
        let rules = parse_stylesheet(".card:has(.badge) { width: 80px }", 0);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selectors[0].subject.has_queries.len(), 1);
        let empty = BTreeMap::new();
        let card = vec!["card".into()];
        let badge = SimpleCompound {
            classes: vec!["badge".into()],
            ..Default::default()
        };
        let args = [badge];
        let mut layout = LayoutStyle::default();
        let hit = ctx("div", "", &card, &empty, &[]);
        let mut hit = hit;
        hit.has_bits = 1;
        hit.has_args = &args;
        apply_stylesheet_to_layout(&mut layout, &rules, &hit, None, None);
        assert_eq!(layout.width, Some(LengthSpec::Px(80.0)));

        let mut miss = LayoutStyle::default();
        let mut nohit = ctx("div", "", &card, &empty, &[]);
        nohit.has_args = &args;
        apply_stylesheet_to_layout(&mut miss, &rules, &nohit, None, None);
        assert!(miss.width.is_none());
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
    fn anonymous_class_stylesheet_drives_layout() {
        let rules = parse_stylesheet(
            r#"
            .anon-page {
              display: grid;
              grid-template-rows: auto auto minmax(0, 1fr);
              gap: 12px;
              height: 100%;
              overflow: hidden;
              width: 100%;
            }
            .anon-tray {
              display: inline-flex;
              flex-direction: row;
              align-items: center;
              gap: 2px;
              height: 40px;
              padding: 4px;
              border-radius: 10px;
            }
            "#,
            0,
        );
        assert_eq!(rules.len(), 2);

        let classes = vec!["anon-page".into()];
        let attrs = BTreeMap::new();
        let page_ctx = ctx("section", "", &classes, &attrs, &[]);
        let mut page = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut page, &rules, &page_ctx, None, None);
        assert_eq!(page.direction, Some(FlexDirection::Column));
        assert_eq!(page.height, Some(LengthSpec::Fill));
        assert_eq!(page.gap, Some(LengthSpec::Px(12.0)));
        assert!(page.grid_rows.as_ref().is_some_and(|r| r.len() == 3));

        let tray_classes = vec!["anon-tray".into()];
        let tray_ctx = ctx("div", "", &tray_classes, &attrs, &[]);
        let layout = rebuild_layout_style(
            LayoutStyle::default(),
            &rules,
            &tray_ctx,
            "",
            "",
            None,
            None,
        );
        assert_eq!(layout.direction, Some(FlexDirection::Row));
        assert_eq!(layout.height, Some(LengthSpec::Px(40.0)));
        assert_eq!(layout.padding, Some(LengthSpec::Px(4.0)));
        let radii = layout.paint.border_radii.expect("corners");
        assert_eq!(radii[0], LengthSpec::Px(10.0));
        assert!(layout.border_radius.is_none());
    }

    #[test]
    fn attribute_and_child_combinator_match() {
        let rules = parse_stylesheet(
            r#"
            .shell[data-host="true"] { height: 100%; display: grid; grid-template-rows: minmax(0,1fr); }
            .parent > .child { width: 100%; padding: 12px; }
            "#,
            0,
        );
        let mut attrs = BTreeMap::new();
        attrs.insert("data-host".into(), "true".into());
        let shell_classes = vec!["shell".into()];
        let shell_ctx = ctx("div", "", &shell_classes, &attrs, &[]);
        let mut shell = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut shell, &rules, &shell_ctx, None, None);
        assert_eq!(shell.height, Some(LengthSpec::Fill));

        let parent_classes = vec!["parent".into()];
        let parent_attrs = BTreeMap::new();
        let child_classes = vec!["child".into()];
        let ancestors = [node("div", "", &parent_classes, &parent_attrs)];
        let child_ctx = ctx("div", "", &child_classes, &parent_attrs, &ancestors);
        let mut child = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut child, &rules, &child_ctx, None, None);
        assert_eq!(child.width, Some(LengthSpec::Fill));
        assert_eq!(child.padding, Some(LengthSpec::Px(12.0)));
    }

    #[test]
    fn deep_descendant_and_child_combinators_match_full_chain() {
        let rules = parse_stylesheet(
            r#"
            .a > .b > .c .leaf { gap: 24px; width: 100%; }
            .a .leaf { padding: 8px; }
            "#,
            0,
        );
        let empty = BTreeMap::new();
        let a = vec!["a".into()];
        let b = vec!["b".into()];
        let c = vec!["c".into()];
        let leaf = vec!["leaf".into()];
        let ancestors = [
            node("div", "", &c, &empty),
            node("div", "", &b, &empty),
            node("div", "", &a, &empty),
        ];
        let leaf_ctx = ctx("div", "", &leaf, &empty, &ancestors);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &rules, &leaf_ctx, None, None);
        assert_eq!(layout.gap, Some(LengthSpec::Px(24.0)));
        assert_eq!(layout.width, Some(LengthSpec::Fill));
        assert_eq!(layout.padding, Some(LengthSpec::Px(8.0)));

        let shallow = [node("div", "", &c, &empty), node("div", "", &b, &empty)];
        let shallow_ctx = ctx("div", "", &leaf, &empty, &shallow);
        let mut miss = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut miss, &rules, &shallow_ctx, None, None);
        assert!(miss.gap.is_none());
        assert!(miss.padding.is_none());
    }

    #[test]
    fn prop_style_does_not_wipe_public_class_contract() {
        let classes = vec!["nana-settings-row".into()];
        let attrs = BTreeMap::new();
        let m = ctx("div", "", &classes, &attrs, &[]);
        let layout = rebuild_layout_style(
            LayoutStyle::default(),
            &[],
            &m,
            "flex-direction:column; gap:4px",
            "",
            None,
            None,
        );
        assert_eq!(layout.direction, Some(FlexDirection::Row));
        assert_eq!(layout.gap, Some(LengthSpec::Px(14.0)));
        assert_eq!(
            layout.justify_content,
            crate::css_map::JustifySpec::SpaceBetween
        );
    }

    #[test]
    fn inline_style_wins_over_stylesheet() {
        let rules = parse_stylesheet(".box { display:flex; flex-direction:column; gap:4px; }", 0);
        let classes = vec!["box".into()];
        let attrs = BTreeMap::new();
        let m = ctx("div", "", &classes, &attrs, &[]);
        let layout = rebuild_layout_style(
            LayoutStyle::default(),
            &rules,
            &m,
            "",
            "flex-direction:row; gap:20px",
            None,
            None,
        );
        assert_eq!(layout.direction, Some(FlexDirection::Row));
        assert_eq!(layout.gap, Some(LengthSpec::Px(20.0)));
    }

    #[test]
    fn direction_rtl_cross_layer_stylesheet_logical_then_inline_dir() {
        let rules = parse_stylesheet(".box { padding-inline-start: 12px; }", 0);
        let classes = vec!["box".into()];
        let attrs = BTreeMap::new();
        let m = ctx("div", "", &classes, &attrs, &[]);
        let layout = rebuild_layout_style(
            LayoutStyle::default(),
            &rules,
            &m,
            "",
            "direction: rtl",
            None,
            None,
        );
        assert_eq!(layout.dir, Some(DirSpec::Rtl));
        assert_eq!(layout.padding_right, Some(LengthSpec::Px(12.0)));
        assert!(layout.padding_left.is_none());
    }

    #[test]
    fn direction_rtl_inherited_dir_seed_remaps_stylesheet_logical() {
        let rules = parse_stylesheet(".box { padding-inline-start: 12px; }", 0);
        let classes = vec!["box".into()];
        let attrs = BTreeMap::new();
        let m = ctx("div", "", &classes, &attrs, &[]);
        let mut base = LayoutStyle::default();
        base.dir = Some(DirSpec::Rtl);
        let layout = rebuild_layout_style(base, &rules, &m, "", "", None, None);
        assert_eq!(layout.dir, Some(DirSpec::Rtl));
        assert_eq!(layout.padding_right, Some(LengthSpec::Px(12.0)));
        assert!(layout.padding_left.is_none());
    }

    #[test]
    fn stylesheet_important_beats_inline_and_prop() {
        let rules = parse_stylesheet(".box { width: 80px !important; height: 20px; }", 0);
        let classes = vec!["box".into()];
        let attrs = BTreeMap::new();
        let m = ctx("div", "", &classes, &attrs, &[]);
        let layout = rebuild_layout_style(
            LayoutStyle::default(),
            &rules,
            &m,
            "width:160px",
            "width:200px;height:40px",
            None,
            None,
        );
        assert_eq!(
            layout.width,
            Some(LengthSpec::Px(80.0)),
            "stylesheet !important must beat prop/inline normals"
        );
        assert_eq!(
            layout.height,
            Some(LengthSpec::Px(40.0)),
            "inline still wins over stylesheet normal"
        );
    }

    #[test]
    fn inline_important_beats_stylesheet_important() {
        let rules = parse_stylesheet(".box { width: 80px !important; height: 20px; }", 0);
        let classes = vec!["box".into()];
        let attrs = BTreeMap::new();
        let m = ctx("div", "", &classes, &attrs, &[]);
        let layout = rebuild_layout_style(
            LayoutStyle::default(),
            &rules,
            &m,
            "width:160px !important",
            "width:200px !important;height:40px",
            None,
            None,
        );
        assert_eq!(
            layout.width,
            Some(LengthSpec::Px(200.0)),
            "inline !important must beat stylesheet !important and prop !important"
        );
        assert_eq!(
            layout.height,
            Some(LengthSpec::Px(40.0)),
            "inline still wins over stylesheet normal"
        );
    }

    #[test]
    fn inline_only_important_applies_as_width() {
        let empty = BTreeMap::new();
        let classes: Vec<String> = Vec::new();
        let m = ctx("div", "", &classes, &empty, &[]);
        let layout = rebuild_layout_style(
            LayoutStyle::default(),
            &[],
            &m,
            "",
            "width:100px !important",
            None,
            None,
        );
        assert_eq!(
            layout.width,
            Some(LengthSpec::Px(100.0)),
            "inline-only width:100px !important must write 100, not drop the declaration"
        );
    }

    #[test]
    fn simple_not_class_and_attr_match() {
        let rules = parse_stylesheet(
            r#"
            .tab:not(.is-active) { width: 40px; }
            .tab.is-active { width: 80px; }
            button:not([disabled]) { height: 24px; }
            "#,
            0,
        );
        assert_eq!(rules.len(), 3);
        let attrs = BTreeMap::new();
        let inactive = vec!["tab".into()];
        let active = vec!["tab".into(), "is-active".into()];
        let mut a = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut a,
            &rules,
            &ctx("div", "", &inactive, &attrs, &[]),
            None,
            None,
        );
        assert_eq!(a.width, Some(LengthSpec::Px(40.0)));
        let mut b = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut b,
            &rules,
            &ctx("div", "", &active, &attrs, &[]),
            None,
            None,
        );
        assert_eq!(b.width, Some(LengthSpec::Px(80.0)));
        let mut enabled = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut enabled,
            &rules,
            &ctx("button", "", &[], &attrs, &[]),
            None,
            None,
        );
        assert_eq!(enabled.height, Some(LengthSpec::Px(24.0)));
        let mut disabled_attrs = BTreeMap::new();
        disabled_attrs.insert("disabled".into(), String::new());
        let mut disabled = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut disabled,
            &rules,
            &ctx("button", "", &[], &disabled_attrs, &[]),
            None,
            None,
        );
        assert!(disabled.height.is_none());
        let complex = parse_stylesheet("button:not(:disabled) { width: 10px; }", 0);
        assert_eq!(complex.len(), 1);
        let mut enabled_not = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut enabled_not,
            &complex,
            &ctx("button", "", &[], &attrs, &[]),
            None,
            None,
        );
        assert_eq!(enabled_not.width, Some(LengthSpec::Px(10.0)));
        let mut disabled_not = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut disabled_not,
            &complex,
            &ctx("button", "", &[], &disabled_attrs, &[]),
            None,
            None,
        );
        assert!(disabled_not.width.is_none());
    }

    #[test]
    fn focus_within_subject_matches_when_flag_set() {
        let rules = parse_stylesheet(".field:focus-within { width: 12px; }", 0);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].selectors[0].subject.focus_within);
        let classes = vec!["field".into()];
        let attrs = BTreeMap::new();
        let mut hit = ctx("div", "", &classes, &attrs, &[]);
        hit.focus_within = true;
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &rules, &hit, None, None);
        assert_eq!(layout.width, Some(LengthSpec::Px(12.0)));
        let mut miss = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut miss,
            &rules,
            &ctx("div", "", &classes, &attrs, &[]),
            None,
            None,
        );
        assert!(miss.width.is_none());
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

        let classes = vec!["ok".into()];
        let attrs = BTreeMap::new();
        let m = ctx("div", "", &classes, &attrs, &[]);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &sheet.static_rules, &m, None, None);
        assert_eq!(layout.height, Some(LengthSpec::Fill));

        let hover = crate::css_interactive::matched_interactive_rules(
            &sheet.interactive_rules,
            &m,
            &crate::css_interactive::InteractiveMatchState {
                subject: crate::css_interactive::InteractivePseudoFlags {
                    hover: true,
                    ..Default::default()
                },
                ancestors: &[],
            },
            crate::css_interactive::InteractivePseudo::Hover,
        );
        assert_eq!(hover.len(), 1);
        assert!(
            hover[0]
                .2
                .declaration_entries
                .iter()
                .any(|e| e.property == "height" && e.value == "50%")
        );

        let pseudo =
            crate::css_interactive::matched_generated_pseudo(&sheet.generated_pseudo_rules, &m);
        assert_eq!(pseudo.before.len(), 1);
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

        let card = vec!["card".into()];
        let icon = vec!["icon".into()];
        let empty = BTreeMap::new();
        let ancestors = [node("div", "", &card, &empty)];
        let icon_ctx = ctx("span", "", &icon, &empty, &ancestors);

        let card_hover = [crate::css_interactive::InteractivePseudoFlags {
            hover: true,
            ..Default::default()
        }];
        let hover = crate::css_interactive::matched_interactive_rules(
            &sheet.interactive_rules,
            &icon_ctx,
            &crate::css_interactive::InteractiveMatchState {
                subject: Default::default(),
                ancestors: &card_hover,
            },
            crate::css_interactive::InteractivePseudo::Hover,
        );
        assert_eq!(hover.len(), 1);
        assert!(
            hover[0]
                .2
                .declaration_entries
                .iter()
                .any(|e| e.property == "width" && e.value == "24px")
        );

        // Icon hovered without card hovered must not match `.card:hover .icon`.
        let icon_only_hover = crate::css_interactive::matched_interactive_rules(
            &sheet.interactive_rules,
            &icon_ctx,
            &crate::css_interactive::InteractiveMatchState {
                subject: crate::css_interactive::InteractivePseudoFlags {
                    hover: true,
                    ..Default::default()
                },
                ancestors: &[Default::default()],
            },
            crate::css_interactive::InteractivePseudo::Hover,
        );
        assert!(icon_only_hover.is_empty());

        let card_ctx = ctx("div", "", &card, &empty, &[]);
        let motion = crate::css_interactive::matched_motion_rules(&sheet.motion_rules, &card_ctx);
        assert_eq!(motion.len(), 1);
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
    fn interactive_subject_rejects_child_and_sibling_combinators() {
        let (sheet, report) = parse_stylesheet_full(
            r#"
            .card > .btn:hover { color: red; }
            .card + .btn:hover { color: red; }
            .card ~ .btn:hover { color: red; }
            .btn:hover { color: green; }
            "#,
            0,
        );
        assert_eq!(sheet.interactive_rules.len(), 1);
        assert_eq!(report.skipped_selectors, 3);
        assert_eq!(sheet.interactive_rules[0].selector.subject.classes, ["btn"]);
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
    fn wrap_from_stylesheet() {
        let rules = parse_stylesheet(
            ".g { display:flex; flex-direction:row; flex-wrap:wrap; gap:12px; height:100%; }",
            0,
        );
        let classes = vec!["g".into()];
        let attrs = BTreeMap::new();
        let m = ctx("div", "", &classes, &attrs, &[]);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &rules, &m, None, None);
        assert_eq!(layout.flex_wrap, FlexWrap::Wrap);
        assert_eq!(layout.height, Some(LengthSpec::Fill));
    }

    #[test]
    fn first_child_structural_pseudo_matches() {
        let rules = parse_stylesheet(
            ".stack > :first-child { height: 100%; } .stack > :last-child { width: 50%; }",
            0,
        );
        let parent_classes = vec!["stack".into()];
        let empty = BTreeMap::new();
        let parent = [node("div", "", &parent_classes, &empty)];
        let leaf = Vec::<String>::new();

        let mut first = MatchContext {
            tag: "div",
            id: "",
            classes: &leaf,
            attrs: &empty,
            ancestors: &parent,
            preceding_siblings: &[],
            sibling_index: 0,
            sibling_count: 3,
            of_type_index: 0,
            of_type_count: 3,
            has_bits: 0,
            has_args: &[],
            focus_within: false,
            is_empty: true,
            checked: false,
        };
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &rules, &first, None, None);
        assert_eq!(layout.height, Some(LengthSpec::Fill));
        assert!(layout.width.is_none());

        first.sibling_index = 2;
        let mut last = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut last, &rules, &first, None, None);
        assert_eq!(last.width, Some(LengthSpec::Percent(50.0)));
        assert!(last.height.is_none());

        first.sibling_index = 1;
        let mut mid = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut mid, &rules, &first, None, None);
        assert!(mid.height.is_none());
        assert!(mid.width.is_none());
    }

    #[test]
    fn root_and_data_theme_attribute_match() {
        // Anonymous DOM: document root with data-theme; child must not match :root.
        let rules = parse_stylesheet(
            r#"
            :root { gap: 4px; }
            :root[data-theme="dark"] { padding: 16px; }
            :root[data-theme="light"] { padding: 8px; }
            "#,
            0,
        );
        assert_eq!(rules.len(), 3);
        let mut dark_attrs = BTreeMap::new();
        dark_attrs.insert("data-theme".into(), "dark".into());
        let root_classes = Vec::<String>::new();
        let root_ctx = ctx("html", "", &root_classes, &dark_attrs, &[]);
        let mut root = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut root, &rules, &root_ctx, None, None);
        assert_eq!(root.gap, Some(LengthSpec::Px(4.0)));
        assert_eq!(root.padding, Some(LengthSpec::Px(16.0)));

        let empty = BTreeMap::new();
        let ancestors = [node("html", "", &root_classes, &dark_attrs)];
        let child = vec!["panel".into()];
        let child_ctx = ctx("div", "", &child, &empty, &ancestors);
        let mut miss = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut miss, &rules, &child_ctx, None, None);
        assert!(miss.gap.is_none());
        assert!(miss.padding.is_none());

        // Specificity: :root[data-theme] (0,2,0) > :root (0,1,0); source order
        // between equal theme rules is covered by light-vs-dark exclusivity.
        let sel_root = &rules[0].selectors[0];
        let sel_dark = &rules[1].selectors[0];
        assert!(sel_dark.specificity > sel_root.specificity);
    }

    #[test]
    fn rule_index_candidates_superset_of_linear_scan() {
        // Exercises every bucket: tag (case-insensitive), id, class, attr-only,
        // structural pseudos, universal, `:is()` arms, `:not()`, descendant and
        // sibling combinators.
        let (sheet, _) = parse_stylesheet_full(
            "div { width: 1px; } \
             DIV { height: 2px; } \
             * { order: 0; } \
             #main { padding: 1px; } \
             .card { background: red; } \
             .card:hover { background: blue; } \
             .card > .icon { color: red; } \
             nav a { color: green; } \
             h1 + p { margin: 2px; } \
             [hidden] { display: none; } \
             li:first-child { padding: 3px; } \
             :is(.a, [data-x]) { opacity: 0.5; } \
             .btn:not(.ghost) { border: 1px; } \
             p:is(.lead, .hero) { font-weight: bold; }",
            0,
        );
        let rules = &sheet.static_rules;
        let index = RuleIndex::build(rules);
        let attrs: BTreeMap<String, String> = BTreeMap::new();
        let card_classes = vec!["card".to_string()];
        let card_node = MatchNode {
            tag: "div",
            id: "",
            classes: &card_classes,
            attrs: &attrs,
            is_empty: true,
            checked: false,
        };
        let ancestors = [card_node];
        let cases: Vec<(&str, &str, Vec<&str>)> = vec![
            ("div", "", vec![]),
            ("div", "main", vec!["card"]),
            ("span", "", vec!["icon", "a"]),
            ("p", "", vec!["lead"]),
            ("button", "", vec!["btn", "ghost"]),
            ("li", "", vec![]),
            ("h1", "", vec![]),
            ("article", "", vec!["data-x"]),
        ];
        for (tag, id, class_names) in &cases {
            let classes: Vec<String> = class_names.iter().map(|c| c.to_string()).collect();
            let ctx = MatchContext {
                tag,
                id,
                classes: &classes,
                attrs: &attrs,
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
            };
            let matched: Vec<usize> = rules
                .iter()
                .enumerate()
                .filter(|(_, rule)| rule.selectors.iter().any(|sel| selector_matches(sel, &ctx)))
                .map(|(i, _)| i)
                .collect();
            let candidates = index.candidate_ids(&ctx);
            for order in &matched {
                assert!(
                    candidates.contains(&(*order as u32)),
                    "linear match {order} missing from index candidates {candidates:?}"
                );
            }
            // Candidate validation still yields identical matched declarations.
            let indexed = index.candidates(rules, &ctx);
            let indexed_entries = matched_declaration_entries_from(&indexed, &ctx);
            let linear_entries = matched_declaration_entries(rules, &ctx);
            assert_eq!(indexed_entries, linear_entries);
        }
    }

    #[test]
    fn matched_custom_properties_skip_document_root_scope() {
        // Orphan / parentless nodes satisfy :root matching, but document `--*`
        // must not rematch here — bridge stylesheet_vars owns theme overlays.
        let rules = parse_stylesheet(
            r#"
            :root { --bg: #181818; }
            :root[data-theme="light"] { --bg: #ffffff; }
            .surface { --row-h: 28px; }
            :root, .surface { --shared: 1px; }
            "#,
            0,
        );
        let empty = BTreeMap::new();
        let classes = vec!["surface".into()];
        // Parentless → would match :root for layout, but custom props skip it.
        let orphan = ctx("div", "", &classes, &empty, &[]);
        let props = matched_custom_properties_indexed(&rules, &RuleIndex::build(&rules), &orphan);
        assert!(
            !props.contains_key("--bg"),
            "document :root --bg must not overlay theme-aware stylesheet_vars"
        );
        assert_eq!(props.get("--row-h").map(String::as_str), Some("28px"));
        assert_eq!(
            props.get("--shared").map(String::as_str),
            Some("1px"),
            "mixed :root, .surface list still applies via element-scoped alt"
        );
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

        let empty = BTreeMap::new();
        let classes = vec!["anon-a".into()];
        // Element matches class but not #anon-id — :is still matches via .anon-a.
        let m = ctx("div", "", &classes, &empty, &[]);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &rules, &m, None, None);
        // Cascade: where (0) < class (0,1,0) < is (1,0,0) ⇒ gap 2px from :is.
        assert_eq!(layout.gap, Some(LengthSpec::Px(2.0)));

        // Low-spec type can override :where but not :is.
        let override_rules = parse_stylesheet(
            r#"
            :where(.anon-x) { gap: 10px; }
            div { gap: 20px; }
            :is(.anon-x) { gap: 30px; }
            "#,
            0,
        );
        let x = vec!["anon-x".into()];
        let mx = ctx("div", "", &x, &empty, &[]);
        let mut lx = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut lx, &override_rules, &mx, None, None);
        assert_eq!(lx.gap, Some(LengthSpec::Px(30.0)));
    }

    #[test]
    fn adjacent_and_subsequent_sibling_combinators() {
        // Anonymous sibling list: [first, mid, last] — matching from last.
        let rules = parse_stylesheet(
            r#"
            .anon-first + .anon-last { width: 100%; }
            .anon-first ~ .anon-last { padding: 8px; }
            .anon-mid + .anon-last { height: 40px; }
            "#,
            0,
        );
        let empty = BTreeMap::new();
        let first_c = vec!["anon-first".into()];
        let mid_c = vec!["anon-mid".into()];
        let last_c = vec!["anon-last".into()];
        let parent_c = vec!["anon-row".into()];
        let ancestors = [node("div", "", &parent_c, &empty)];
        // preceding: immediate previous first → mid, then first
        let preceding = [
            node("div", "", &mid_c, &empty),
            node("div", "", &first_c, &empty),
        ];
        let last_ctx = ctx_full("div", "", &last_c, &empty, &ancestors, &preceding, 2, 3);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &rules, &last_ctx, None, None);
        // `.anon-first + .anon-last` must NOT match (mid is adjacent).
        assert!(layout.width.is_none());
        // `.anon-first ~ .anon-last` matches (first precedes last).
        assert_eq!(layout.padding, Some(LengthSpec::Px(8.0)));
        // `.anon-mid + .anon-last` matches.
        assert_eq!(layout.height, Some(LengthSpec::Px(40.0)));

        // Child + adjacent: `.anon-row > .anon-mid + .anon-last`
        let chain = parse_stylesheet(".anon-row > .anon-mid + .anon-last { gap: 12px; }", 0);
        let mut chained = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut chained, &chain, &last_ctx, None, None);
        assert_eq!(chained.gap, Some(LengthSpec::Px(12.0)));
    }

    #[test]
    fn specificity_and_source_order_cascade() {
        let rules = parse_stylesheet(
            r#"
            .anon { gap: 1px; }
            .anon.anon-x { gap: 2px; }
            #anon-box { gap: 3px; }
            .anon { gap: 4px; }
            "#,
            0,
        );
        let empty = BTreeMap::new();
        let classes = vec!["anon".into(), "anon-x".into()];
        let m = ctx("div", "anon-box", &classes, &empty, &[]);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &rules, &m, None, None);
        // #id (1,0,0) wins over class rules regardless of source order.
        assert_eq!(layout.gap, Some(LengthSpec::Px(3.0)));

        let no_id = ctx("div", "", &classes, &empty, &[]);
        let mut layout2 = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout2, &rules, &no_id, None, None);
        // .anon.anon-x (0,2,0) > .anon (0,1,0); later .anon does not beat it.
        assert_eq!(layout2.gap, Some(LengthSpec::Px(2.0)));
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
        let empty = BTreeMap::new();
        let a = vec!["a".into()];
        let ab = vec!["a".into(), "b".into()];
        let mut hit = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut hit,
            &rules,
            &ctx("div", "", &a, &empty, &[]),
            None,
            None,
        );
        assert_eq!(hit.width, Some(LengthSpec::Px(10.0)));
        let mut miss = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut miss,
            &rules,
            &ctx("div", "", &ab, &empty, &[]),
            None,
            None,
        );
        assert!(miss.width.is_none());
        let mut miss_id = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut miss_id,
            &rules,
            &ctx("div", "c", &a, &empty, &[]),
            None,
            None,
        );
        assert!(miss_id.width.is_none());
    }

    #[test]
    fn author_important_beats_higher_specificity_normal() {
        // MDN: author !important outranks author normal before specificity.
        let rules = parse_stylesheet(
            r#"
            #anon-box { gap: 99px; }
            .anon { gap: 4px !important; }
            "#,
            0,
        );
        let empty = BTreeMap::new();
        let classes = vec!["anon".into()];
        let m = ctx("div", "anon-box", &classes, &empty, &[]);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &rules, &m, None, None);
        assert_eq!(layout.gap, Some(LengthSpec::Px(4.0)));
    }

    #[test]
    fn important_keeps_specificity_and_source_order() {
        let rules = parse_stylesheet(
            r#"
            .anon { gap: 1px !important; }
            .anon.anon-x { gap: 2px !important; }
            .anon { gap: 3px !important; }
            "#,
            0,
        );
        let empty = BTreeMap::new();
        let classes = vec!["anon".into(), "anon-x".into()];
        let m = ctx("div", "", &classes, &empty, &[]);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &rules, &m, None, None);
        // Among important: .anon.anon-x (0,2,0) beats both .anon rules.
        assert_eq!(layout.gap, Some(LengthSpec::Px(2.0)));

        let only_anon = vec!["anon".into()];
        let m2 = ctx("div", "", &only_anon, &empty, &[]);
        let mut layout2 = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout2, &rules, &m2, None, None);
        // Same specificity important: later source order wins (3px).
        assert_eq!(layout2.gap, Some(LengthSpec::Px(3.0)));
    }

    #[test]
    fn mixed_block_important_and_normal_independent() {
        let rules = parse_stylesheet(
            r#"
            .anon { width: 10px; height: 20px !important; }
            #anon-box { width: 30px; height: 40px; }
            "#,
            0,
        );
        let empty = BTreeMap::new();
        let classes = vec!["anon".into()];
        let m = ctx("div", "anon-box", &classes, &empty, &[]);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &rules, &m, None, None);
        // Normal: #id width wins; important height from .anon beats #id height.
        assert_eq!(layout.width, Some(LengthSpec::Px(30.0)));
        assert_eq!(layout.height, Some(LengthSpec::Px(20.0)));
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
        let empty = BTreeMap::new();
        let classes = vec!["anon".into()];
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut layout,
            &rules,
            &ctx("div", "", &classes, &empty, &[]),
            None,
            None,
        );
        assert_eq!(layout.gap, Some(LengthSpec::Px(8.0)));
    }

    #[test]
    fn matched_declarations_two_pass_order() {
        let rules = parse_stylesheet(
            r#"
            #anon-box { gap: 1px; width: 2px; }
            .anon { gap: 3px !important; padding: 4px; }
            "#,
            0,
        );
        let empty = BTreeMap::new();
        let classes = vec!["anon".into()];
        let m = ctx("div", "anon-box", &classes, &empty, &[]);
        let decls = matched_declarations(&rules, &m);
        let texts: Vec<&str> = decls.iter().map(|(_, _, t)| t.as_str()).collect();
        // Normal first (specificity / source order), then important.
        assert_eq!(
            texts,
            vec!["padding: 4px", "gap: 1px", "width: 2px", "gap: 3px",]
        );
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
    fn nth_child_and_nth_of_type_match() {
        // Anonymous siblings: [div, p, div, p] — 0-based child indices 0..3.
        // Same-tag among `p`: indices 0 (child1), 1 (child3).
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
        let parent_c = vec!["row".into()];
        let empty = BTreeMap::new();
        let ancestors = [node("div", "", &parent_c, &empty)];
        let none = Vec::<String>::new();

        // child 0 = first div: odd nth-child, of-type 0 among div
        let c0 = ctx_nth("div", "", &none, &empty, &ancestors, 0, 4, 0, 2);
        let mut l0 = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut l0, &rules, &c0, None, None);
        assert_eq!(l0.width, Some(LengthSpec::Px(10.0)));
        assert!(l0.height.is_none());
        assert_eq!(l0.gap, Some(LengthSpec::Px(4.0)));
        assert!(l0.padding.is_none());

        // child 1 = first p: even nth-child, of-type 0 among p — not :nth-of-type(2)
        let c1 = ctx_nth("p", "", &none, &empty, &ancestors, 1, 4, 0, 2);
        let mut l1 = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut l1, &rules, &c1, None, None);
        assert!(l1.width.is_none());
        assert_eq!(l1.height, Some(LengthSpec::Px(20.0)));
        assert_eq!(l1.gap, Some(LengthSpec::Px(4.0)));
        assert!(l1.padding.is_none());

        // child 3 = second p (1-based 4, even); of-type index 1 → :nth-of-type(2)
        let c3 = ctx_nth("p", "", &none, &empty, &ancestors, 3, 4, 1, 2);
        let mut l3 = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut l3, &rules, &c3, None, None);
        assert!(l3.width.is_none()); // not odd
        assert_eq!(l3.height, Some(LengthSpec::Px(20.0))); // 2n
        assert_eq!(l3.padding, Some(LengthSpec::Px(8.0)));
        assert!(l3.gap.is_none()); // not in first two

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
    fn attribute_operators_and_case_flags_match() {
        // MDN Attribute selectors: ~= |= ^= $= *= + optional i/s.
        // Avoid `:` inside attr values — deferred-pseudo scan is bracket-naive.
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

        let mut hit = BTreeMap::new();
        hit.insert("title".into(), "x".into());
        hit.insert("data-role".into(), "admin".into());
        hit.insert("class".into(), "btn logo primary".into());
        hit.insert("lang".into(), "zh-Hans".into());
        hit.insert("href".into(), "#example.ORG".into());
        hit.insert("data-id".into(), "AbC".into());
        let classes = Vec::<String>::new();
        let m = ctx("a", "", &classes, &hit, &[]);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &rules, &m, None, None);
        assert_eq!(layout.gap, Some(LengthSpec::Px(1.0)));
        assert_eq!(layout.width, Some(LengthSpec::Px(10.0)));
        assert_eq!(layout.height, Some(LengthSpec::Px(20.0)));
        assert_eq!(layout.padding, Some(LengthSpec::Px(8.0)));
        assert_eq!(layout.margin, Some(LengthSpec::Px(4.0)));
        // `$=".org"` default is case-sensitive — `.ORG` must not match.
        assert!(layout.paint.border_radii.is_none());
        assert!(layout.border_radius.is_none());
        assert_eq!(layout.min_width, Some(LengthSpec::Px(40.0)));
        // `i` makes lowercase `org` match `.ORG`.
        assert_eq!(layout.max_width, Some(LengthSpec::Px(50.0)));
        assert_eq!(layout.min_height, Some(LengthSpec::Px(12.0)));

        let mut miss_s = hit.clone();
        miss_s.insert("data-id".into(), "abc".into());
        let mut layout_s = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut layout_s,
            &rules,
            &ctx("a", "", &classes, &miss_s, &[]),
            None,
            None,
        );
        assert!(layout_s.min_height.is_none());

        let mut miss_lang = BTreeMap::new();
        miss_lang.insert("lang".into(), "en-zh".into());
        let mut layout_lang = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut layout_lang,
            &rules,
            &ctx("div", "", &classes, &miss_lang, &[]),
            None,
            None,
        );
        assert!(layout_lang.padding.is_none());
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
        assert_eq!(from_rules.get("--bg"), from_text.get("--bg"));
        assert_eq!(from_rules.get("--fg"), from_text.get("--fg"));
        assert_eq!(from_rules.get("--shared"), from_text.get("--shared"));
        assert!(
            !from_rules.contains_key("--row-h"),
            "element-scoped .surface --* must stay out of document base"
        );

        let dark = collect_document_custom_properties_from_rules(&rules, "dark");
        assert_eq!(dark.get("--bg").map(String::as_str), Some("#000"));
    }

    #[test]
    fn matched_entries_drive_layout_without_text_resplit() {
        let rules = parse_stylesheet(
            r#"
            #anon-box { gap: 1px; width: 2px; }
            .anon { gap: 3px !important; padding: 4px; }
            "#,
            0,
        );
        let empty = BTreeMap::new();
        let classes = vec!["anon".into()];
        let m = ctx("div", "anon-box", &classes, &empty, &[]);
        let entries = matched_declaration_entries(&rules, &m);
        let owned: Vec<String> = entries.iter().map(|(_, _, e)| e.text()).collect();
        let texts: Vec<&str> = owned.iter().map(String::as_str).collect();
        assert_eq!(
            texts,
            vec!["padding: 4px", "gap: 1px", "width: 2px", "gap: 3px",]
        );
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &rules, &m, None, None);
        assert_eq!(layout.gap, Some(LengthSpec::Px(3.0)));
        assert_eq!(layout.width, Some(LengthSpec::Px(2.0)));
        assert_eq!(layout.padding, Some(LengthSpec::Px(4.0)));
    }

    #[test]
    fn stylesheet_matches_and_subject_index_reject_unrelated() {
        let rules = parse_stylesheet(".foo { gap: 1px; } span.bar { width: 2px; }", 0);
        let empty = BTreeMap::new();
        let foo = vec!["foo".into()];
        let bar = vec!["bar".into()];
        let none = Vec::<String>::new();
        assert!(stylesheet_matches(
            &rules,
            &ctx("div", "", &foo, &empty, &[])
        ));
        assert!(!stylesheet_matches(
            &rules,
            &ctx("div", "", &none, &empty, &[])
        ));
        assert!(stylesheet_matches(
            &rules,
            &ctx("span", "", &bar, &empty, &[])
        ));
        assert!(!stylesheet_matches(
            &rules,
            &ctx("div", "", &bar, &empty, &[])
        ));

        assert!(stylesheet_may_match_subject(&rules, "div", "", &foo));
        assert!(!stylesheet_may_match_subject(&rules, "div", "", &none));
        assert!(!stylesheet_may_match_subject(&rules, "div", "", &bar));
        assert!(stylesheet_may_match_subject(&rules, "span", "", &bar));

        let star = parse_stylesheet("* { gap: 1px; }", 0);
        assert!(stylesheet_may_match_subject(&star, "section", "", &none));
        assert!(stylesheet_matches(
            &star,
            &ctx("section", "", &none, &empty, &[])
        ));

        let root = parse_stylesheet(":root { gap: 2px; }", 0);
        assert!(stylesheet_may_match_subject(&root, "html", "", &none));
        assert!(stylesheet_matches(
            &root,
            &ctx("html", "", &none, &empty, &[])
        ));
        let parent = [node("html", "", &none, &empty)];
        assert!(!stylesheet_matches(
            &root,
            &ctx("div", "", &none, &empty, &parent)
        ));
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

        let classes = vec!["wide".into()];
        let attrs = BTreeMap::new();
        let m = ctx("div", "", &classes, &attrs, &[]);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &applied.static_rules, &m, None, None);
        assert_eq!(layout.width, Some(LengthSpec::Px(100.0)));
        let mut skipped_layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut skipped_layout, &skipped.static_rules, &m, None, None);
        assert!(skipped_layout.width.is_none());
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

        let classes = vec!["ok".into()];
        let attrs = BTreeMap::new();
        let m = ctx("div", "", &classes, &attrs, &[]);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &sheet.static_rules, &m, None, None);
        assert_eq!(layout.display, Some(crate::css_map::DisplaySpec::Flex));
        assert_eq!(layout.width, Some(LengthSpec::Px(40.0)));

        let lab_classes = vec!["lab".into()];
        let lab_ctx = ctx("div", "", &lab_classes, &attrs, &[]);
        let mut lab_layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut lab_layout, &sheet.static_rules, &lab_ctx, None, None);
        assert!(lab_layout.width.is_none());
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

        let classes = vec!["x".into()];
        let attrs = BTreeMap::new();
        let m = ctx("div", "", &classes, &attrs, &[]);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &sheet.static_rules, &m, None, None);
        assert_eq!(layout.width, Some(LengthSpec::Px(10.0)));
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
        let root =
            std::env::temp_dir().join(format!("nanaui-css-jail-{}-escape", std::process::id()));
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
        let jail = std::env::temp_dir().join(format!("nanaui-css-jail-{}-cap", std::process::id()));
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
        let jail =
            std::env::temp_dir().join(format!("nanaui-css-jail-{}-font-rel", std::process::id()));
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
        let classes = vec!["a".into()];
        let attrs = BTreeMap::new();
        let m = ctx("div", "", &classes, &attrs, &[]);
        let mut layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut layout, &flat.static_rules, &m, None, None);
        assert_eq!(layout.color, Some([1.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn subject_disabled_matches_disabled_presence() {
        let rules = parse_stylesheet("button:disabled { width: 10px; }", 0);
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selectors[0].subject.attrs[0].name, "disabled",
            "subject :disabled compiles to [disabled] presence"
        );
        let empty = BTreeMap::new();
        let mut disabled = BTreeMap::new();
        disabled.insert("disabled".into(), String::new());
        let mut hit = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut hit,
            &rules,
            &ctx("button", "", &[], &disabled, &[]),
            None,
            None,
        );
        assert_eq!(hit.width, Some(LengthSpec::Px(10.0)));
        let mut miss = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut miss,
            &rules,
            &ctx("button", "", &[], &empty, &[]),
            None,
            None,
        );
        assert!(miss.width.is_none());
    }

    #[test]
    fn checked_adjacent_sibling_label_matches() {
        let rules = parse_stylesheet("input:checked + label { width: 16px; }", 0);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].selectors[0].ancestors[0].1.checked);
        let empty = BTreeMap::new();
        let none = Vec::<String>::new();
        let input = node_checked("input", "", &none, &empty);
        let parent = [node("div", "", &none, &empty)];
        let mut hit = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut hit,
            &rules,
            &ctx_full("label", "", &none, &empty, &parent, &[input], 1, 2),
            None,
            None,
        );
        assert_eq!(hit.width, Some(LengthSpec::Px(16.0)));
        let unchecked = node("input", "", &none, &empty);
        let mut miss = LayoutStyle::default();
        apply_stylesheet_to_layout(
            &mut miss,
            &rules,
            &ctx_full("label", "", &none, &empty, &parent, &[unchecked], 1, 2),
            None,
            None,
        );
        assert!(miss.width.is_none());
    }

    #[test]
    fn empty_and_not_empty_use_whitespace_definition() {
        // :empty = no element children and no non-whitespace UTF-8 text.
        // Whitespace-only (space / tab / LF / NBSP) still matches :empty.
        let rules = parse_stylesheet(
            r#"
            .box:empty { width: 8px; }
            .box:not(:empty) { height: 12px; }
            "#,
            0,
        );
        assert_eq!(rules.len(), 2);
        assert!(rules[0].selectors[0].subject.empty);
        let classes = vec!["box".into()];
        let attrs = BTreeMap::new();
        let mut empty_ctx = ctx("div", "", &classes, &attrs, &[]);
        empty_ctx.is_empty = true;
        let mut empty_layout = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut empty_layout, &rules, &empty_ctx, None, None);
        assert_eq!(empty_layout.width, Some(LengthSpec::Px(8.0)));
        assert!(empty_layout.height.is_none());

        empty_ctx.is_empty = false;
        let mut filled = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut filled, &rules, &empty_ctx, None, None);
        assert!(filled.width.is_none());
        assert_eq!(filled.height, Some(LengthSpec::Px(12.0)));
    }

    #[test]
    fn only_child_and_of_type_and_nth_last_child() {
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
        let parent_c = vec!["row".into()];
        let empty = BTreeMap::new();
        let ancestors = [node("div", "", &parent_c, &empty)];
        let none = Vec::<String>::new();

        let only = ctx_nth("p", "", &none, &empty, &ancestors, 0, 1, 0, 1);
        let mut lo = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut lo, &rules, &only, None, None);
        assert_eq!(lo.width, Some(LengthSpec::Px(10.0)));
        assert_eq!(lo.height, Some(LengthSpec::Px(20.0)));
        assert_eq!(lo.padding, Some(LengthSpec::Px(8.0)));
        assert_eq!(lo.gap, Some(LengthSpec::Px(4.0)));
        assert_eq!(lo.margin, Some(LengthSpec::Px(2.0)));

        // [div, p, p]: last p is last-of-type, nth-last-child(1), even from end (1-based 1).
        // first p: of-type 0, nth-last-child 2 (even) — not odd from end.
        let first_p = ctx_nth("p", "", &none, &empty, &ancestors, 1, 3, 0, 2);
        let mut l1 = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut l1, &rules, &first_p, None, None);
        assert!(l1.width.is_none());
        assert_eq!(l1.height, Some(LengthSpec::Px(20.0)));
        assert!(l1.padding.is_none());
        assert!(l1.gap.is_none());
        assert!(l1.margin.is_none());

        let last_p = ctx_nth("p", "", &none, &empty, &ancestors, 2, 3, 1, 2);
        let mut l2 = LayoutStyle::default();
        apply_stylesheet_to_layout(&mut l2, &rules, &last_p, None, None);
        assert!(l2.width.is_none());
        assert!(l2.height.is_none());
        assert_eq!(l2.padding, Some(LengthSpec::Px(8.0)));
        assert_eq!(l2.gap, Some(LengthSpec::Px(4.0)));
        assert_eq!(l2.margin, Some(LengthSpec::Px(2.0)));
    }
}
