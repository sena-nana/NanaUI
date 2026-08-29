//! Parsed-but-not-yet-applied CSS: interactive pseudos, generated boxes, keyframes, motion.
//!
//! Rules land in [`ParsedStylesheet`] buckets at parse time. Static cascade in
//! [`crate::css_cascade`] ignores them until a later bridge / Runtime agent wires
//! hover restyle, `::before`/`::after` boxes, `::placeholder` input paint, and
//! animation timelines.

use std::collections::BTreeMap;

use crate::css_at_rule::{
    FontFaceRule, MediaEnvironment, MediaQueryList, evaluate_media_query_list,
};
use crate::css_cascade::{
    Combinator, CompoundSelector, DeclarationEntry, MatchContext, Selector, Specificity, StyleRule,
    compound_matches, parse_declaration_entries,
};
#[cfg(test)]
use crate::css_cascade::MatchNode;

/// Interactive pseudo-class supported at parse time.
///
/// `:focus-visible` maps to [`Self::Focus`]: this engine has no keyboard-vs-pointer
/// signal, so it matches only while focused (same as `:focus`) and never paints
/// a focus-visible layer without focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InteractivePseudo {
    Hover,
    Focus,
    Active,
}

impl InteractivePseudo {
    pub fn from_ident(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "hover" => Some(Self::Hover),
            "focus" | "focus-visible" => Some(Self::Focus),
            "active" => Some(Self::Active),
            _ => None,
        }
    }
}

/// Selector whose declarations apply only while `pseudo` is active on one compound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractiveSelector {
    pub subject: CompoundSelector,
    pub ancestors: Vec<(Combinator, CompoundSelector)>,
    /// Index of the compound carrying `pseudo`: `0..ancestors.len()` for an
    /// ancestor, `ancestors.len()` for the subject.
    pub interactive_at: usize,
    pub pseudo: InteractivePseudo,
    pub specificity: Specificity,
}

/// One interactive rule (not applied during static cascade).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractiveStyleRule {
    pub selector: InteractiveSelector,
    pub declarations: String,
    pub declaration_entries: Vec<DeclarationEntry>,
    pub motion: MotionDeclarations,
    pub source_order: u32,
}

/// Generated pseudo-element kind.
///
/// `::before` / `::after` (including legacy single-colon) materialize boxes.
/// `::placeholder` is paint-only on Runtime TextInput — never a generated child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GeneratedPseudo {
    Before,
    After,
    Placeholder,
}

impl GeneratedPseudo {
    pub fn from_ident(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "before" => Some(Self::Before),
            "after" => Some(Self::After),
            "placeholder" => Some(Self::Placeholder),
            _ => None,
        }
    }
}

/// Rule for a generated `::before` / `::after` box or `::placeholder` paint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedPseudoRule {
    /// Originating element selector (pseudo stripped from subject).
    pub originating_selector: Selector,
    pub pseudo: GeneratedPseudo,
    pub declarations: String,
    pub declaration_entries: Vec<DeclarationEntry>,
    pub motion: MotionDeclarations,
    pub source_order: u32,
}

/// Keyframe stop selector (`from` / `to` / percentage).
#[derive(Debug, Clone, PartialEq)]
pub enum KeyframeSelector {
    From,
    To,
    Percent(f32),
}

/// One `@keyframes` block stop.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframeBlock {
    pub selectors: Vec<KeyframeSelector>,
    pub declaration_entries: Vec<DeclarationEntry>,
}

/// Parsed `@keyframes` rule (timeline not executed yet).
#[derive(Debug, Clone, PartialEq)]
pub struct KeyframesRule {
    pub name: String,
    pub blocks: Vec<KeyframeBlock>,
    pub source_order: u32,
}

/// Shorthand and longhand transition / animation properties (parse-only storage).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MotionDeclarations {
    pub transition: Option<String>,
    pub transition_property: Option<String>,
    pub transition_duration: Option<String>,
    pub transition_timing_function: Option<String>,
    pub transition_delay: Option<String>,
    pub animation: Option<String>,
    pub animation_name: Option<String>,
    pub animation_duration: Option<String>,
    pub animation_timing_function: Option<String>,
    pub animation_delay: Option<String>,
    pub animation_iteration_count: Option<String>,
    pub animation_direction: Option<String>,
    pub animation_fill_mode: Option<String>,
    pub animation_play_state: Option<String>,
}

impl MotionDeclarations {
    pub fn is_empty(&self) -> bool {
        self.transition.is_none()
            && self.transition_property.is_none()
            && self.transition_duration.is_none()
            && self.transition_timing_function.is_none()
            && self.transition_delay.is_none()
            && self.animation.is_none()
            && self.animation_name.is_none()
            && self.animation_duration.is_none()
            && self.animation_timing_function.is_none()
            && self.animation_delay.is_none()
            && self.animation_iteration_count.is_none()
            && self.animation_direction.is_none()
            && self.animation_fill_mode.is_none()
            && self.animation_play_state.is_none()
    }
}

/// Motion properties keyed by matching static selector(s).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MotionStyleRule {
    pub selectors: Vec<Selector>,
    pub motion: MotionDeclarations,
    pub source_order: u32,
}

/// Parsed `@media` block. Inner rules join the cascade only while the query matches.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaRule {
    pub query: MediaQueryList,
    pub sheet: ParsedStylesheet,
}

/// Full stylesheet parse output: static cascade rules plus deferred buckets.
///
/// `static_rules` / interactive / … are **unconditional**. Matching `@media`
/// inner rules are applied through [`ParsedStylesheet::flatten`] so viewport
/// / theme changes do not re-parse CSS text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedStylesheet {
    pub static_rules: Vec<StyleRule>,
    pub interactive_rules: Vec<InteractiveStyleRule>,
    pub generated_pseudo_rules: Vec<GeneratedPseudoRule>,
    pub keyframes: BTreeMap<String, KeyframesRule>,
    pub motion_rules: Vec<MotionStyleRule>,
    pub media_rules: Vec<MediaRule>,
    pub font_faces: Vec<FontFaceRule>,
    /// First-seen `@layer` names (order recorded; cascade-layer *priority* is not applied).
    pub layer_names: Vec<String>,
}

impl ParsedStylesheet {
    pub fn is_cascade_empty(&self) -> bool {
        self.static_rules.is_empty()
            && self.interactive_rules.is_empty()
            && self.generated_pseudo_rules.is_empty()
            && self.motion_rules.is_empty()
            && self.keyframes.is_empty()
            && self.media_rules.is_empty()
            && self.font_faces.is_empty()
    }

    /// Copy unconditional buckets plus inner sheets whose `@media` matches `env`.
    pub fn flatten(&self, env: &MediaEnvironment) -> ParsedStylesheet {
        let mut out = ParsedStylesheet {
            static_rules: self.static_rules.clone(),
            interactive_rules: self.interactive_rules.clone(),
            generated_pseudo_rules: self.generated_pseudo_rules.clone(),
            keyframes: self.keyframes.clone(),
            motion_rules: self.motion_rules.clone(),
            media_rules: Vec::new(),
            font_faces: self.font_faces.clone(),
            layer_names: self.layer_names.clone(),
        };
        for media in &self.media_rules {
            if evaluate_media_query_list(&media.query, env) {
                merge_parsed_stylesheet(&mut out, media.sheet.flatten(env));
            }
        }
        out
    }

    pub fn max_source_order(&self) -> Option<u32> {
        let nested = self
            .media_rules
            .iter()
            .filter_map(|m| m.sheet.max_source_order());
        [
            self.static_rules.last().map(|r| r.source_order),
            self.interactive_rules.last().map(|r| r.source_order),
            self.generated_pseudo_rules.last().map(|r| r.source_order),
            self.motion_rules.last().map(|r| r.source_order),
            self.keyframes.values().map(|r| r.source_order).max(),
        ]
        .into_iter()
        .flatten()
        .chain(nested)
        .max()
    }

    pub fn all_font_faces(&self) -> Vec<&FontFaceRule> {
        let mut out = Vec::new();
        self.collect_font_faces(&mut out);
        out
    }

    fn collect_font_faces<'a>(&'a self, out: &mut Vec<&'a FontFaceRule>) {
        out.extend(self.font_faces.iter());
        for media in &self.media_rules {
            media.sheet.collect_font_faces(out);
        }
    }
}

/// Append `src` buckets onto `dest` (same cascade, not a second sheet store).
pub fn merge_parsed_stylesheet(dest: &mut ParsedStylesheet, src: ParsedStylesheet) {
    dest.static_rules.extend(src.static_rules);
    dest.interactive_rules.extend(src.interactive_rules);
    dest.generated_pseudo_rules
        .extend(src.generated_pseudo_rules);
    dest.motion_rules.extend(src.motion_rules);
    dest.font_faces.extend(src.font_faces);
    dest.media_rules.extend(src.media_rules);
    for name in src.layer_names {
        if !name.is_empty() && !dest.layer_names.iter().any(|existing| existing == &name) {
            dest.layer_names.push(name);
        }
    }
    for (name, rule) in src.keyframes {
        dest.keyframes.insert(name, rule);
    }
}

pub(crate) fn offset_source_order(sheet: &mut ParsedStylesheet, delta: u32) {
    if delta == 0 {
        return;
    }
    for rule in &mut sheet.static_rules {
        rule.source_order = rule.source_order.saturating_add(delta);
    }
    for rule in &mut sheet.interactive_rules {
        rule.source_order = rule.source_order.saturating_add(delta);
    }
    for rule in &mut sheet.generated_pseudo_rules {
        rule.source_order = rule.source_order.saturating_add(delta);
    }
    for rule in &mut sheet.motion_rules {
        rule.source_order = rule.source_order.saturating_add(delta);
    }
    for rule in sheet.keyframes.values_mut() {
        rule.source_order = rule.source_order.saturating_add(delta);
    }
    for media in &mut sheet.media_rules {
        offset_source_order(&mut media.sheet, delta);
    }
}

/// Declaration blocks for generated pseudos matching an originating element.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GeneratedPseudoMatch {
    pub before: Vec<Vec<DeclarationEntry>>,
    pub after: Vec<Vec<DeclarationEntry>>,
    pub placeholder: Vec<Vec<DeclarationEntry>>,
}

const MOTION_PROPERTIES: &[&str] = &[
    "transition",
    "transition-property",
    "transition-duration",
    "transition-timing-function",
    "transition-delay",
    "animation",
    "animation-name",
    "animation-duration",
    "animation-timing-function",
    "animation-delay",
    "animation-iteration-count",
    "animation-direction",
    "animation-fill-mode",
    "animation-play-state",
];

fn is_motion_property(name: &str) -> bool {
    MOTION_PROPERTIES
        .iter()
        .any(|p| name.eq_ignore_ascii_case(p))
}

/// Split layout/paint declarations from motion longhands/shorthands.
pub fn partition_motion_entries(
    entries: &[DeclarationEntry],
) -> (Vec<DeclarationEntry>, MotionDeclarations) {
    let mut layout = Vec::new();
    let mut motion = MotionDeclarations::default();
    for entry in entries {
        if is_motion_property(&entry.property) {
            apply_motion_entry(&mut motion, entry);
        } else {
            layout.push(entry.clone());
        }
    }
    (layout, motion)
}

fn apply_motion_entry(motion: &mut MotionDeclarations, entry: &DeclarationEntry) {
    let slot = match entry.property.to_ascii_lowercase().as_str() {
        "transition" => &mut motion.transition,
        "transition-property" => &mut motion.transition_property,
        "transition-duration" => &mut motion.transition_duration,
        "transition-timing-function" => &mut motion.transition_timing_function,
        "transition-delay" => &mut motion.transition_delay,
        "animation" => &mut motion.animation,
        "animation-name" => &mut motion.animation_name,
        "animation-duration" => &mut motion.animation_duration,
        "animation-timing-function" => &mut motion.animation_timing_function,
        "animation-delay" => &mut motion.animation_delay,
        "animation-iteration-count" => &mut motion.animation_iteration_count,
        "animation-direction" => &mut motion.animation_direction,
        "animation-fill-mode" => &mut motion.animation_fill_mode,
        "animation-play-state" => &mut motion.animation_play_state,
        _ => return,
    };
    *slot = Some(entry.value.clone());
}

/// Lookup parsed `@keyframes` by name (last source-order wins on duplicate names).
pub fn keyframes_by_name<'a>(
    keyframes: &'a BTreeMap<String, KeyframesRule>,
    name: &str,
) -> Option<&'a KeyframesRule> {
    keyframes.get(name)
}

/// Which `:hover` / `:focus` / `:active` pseudos are active on one node.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InteractivePseudoFlags {
    pub hover: bool,
    pub focus: bool,
    pub active: bool,
}

impl InteractivePseudoFlags {
    pub fn has(self, pseudo: InteractivePseudo) -> bool {
        match pseudo {
            InteractivePseudo::Hover => self.hover,
            InteractivePseudo::Focus => self.focus,
            InteractivePseudo::Active => self.active,
        }
    }

    pub fn with(self, pseudo: InteractivePseudo) -> Self {
        let mut next = self;
        match pseudo {
            InteractivePseudo::Hover => next.hover = true,
            InteractivePseudo::Focus => next.focus = true,
            InteractivePseudo::Active => next.active = true,
        }
        next
    }
}

/// Per-node interactive activation aligned with [`MatchContext::ancestors`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InteractiveMatchState<'a> {
    /// Pseudos active on the subject element.
    pub subject: InteractivePseudoFlags,
    /// Parallel to [`MatchContext::ancestors`] (immediate parent at index 0).
    pub ancestors: &'a [InteractivePseudoFlags],
}

/// Interactive rules matching `ctx` while `pseudo` is active on the selector's
/// interactive compound (specificity + source order).
pub fn matched_interactive_rules<'a>(
    rules: &'a [InteractiveStyleRule],
    ctx: &MatchContext<'_>,
    state: &InteractiveMatchState<'_>,
    pseudo: InteractivePseudo,
) -> Vec<(Specificity, u32, &'a InteractiveStyleRule)> {
    let mut matched = Vec::new();
    for rule in rules {
        if rule.selector.pseudo == pseudo
            && interactive_selector_matches(&rule.selector, ctx, state)
        {
            matched.push((rule.selector.specificity, rule.source_order, rule));
        }
    }
    matched.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    matched
}

/// Generated `::before` / `::after` / `::placeholder` blocks for an originating element.
pub fn matched_generated_pseudo(
    rules: &[GeneratedPseudoRule],
    ctx: &MatchContext<'_>,
) -> GeneratedPseudoMatch {
    let mut out = GeneratedPseudoMatch::default();
    let mut matched: Vec<(Specificity, u32, GeneratedPseudo, Vec<DeclarationEntry>)> = Vec::new();
    for rule in rules {
        if crate::css_cascade::selector_matches(&rule.originating_selector, ctx) {
            matched.push((
                rule.originating_selector.specificity,
                rule.source_order,
                rule.pseudo,
                rule.declaration_entries.clone(),
            ));
        }
    }
    matched.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    for (_, _, pseudo, entries) in matched {
        match pseudo {
            GeneratedPseudo::Before => out.before.push(entries),
            GeneratedPseudo::After => out.after.push(entries),
            GeneratedPseudo::Placeholder => out.placeholder.push(entries),
        }
    }
    out
}

/// Motion rules whose selector list matches `ctx` (specificity + source order).
pub fn matched_motion_rules<'a>(
    rules: &'a [MotionStyleRule],
    ctx: &MatchContext<'_>,
) -> Vec<(Specificity, u32, &'a MotionStyleRule)> {
    let mut matched = Vec::new();
    for rule in rules {
        for sel in &rule.selectors {
            if crate::css_cascade::selector_matches(sel, ctx) {
                matched.push((sel.specificity, rule.source_order, rule));
                break;
            }
        }
    }
    matched.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    matched
}

/// True when selector structure matches and the interactive pseudo is active on
/// the compound named by [`InteractiveSelector::interactive_at`].
pub fn interactive_selector_matches(
    sel: &InteractiveSelector,
    ctx: &MatchContext<'_>,
    state: &InteractiveMatchState<'_>,
) -> bool {
    if !interactive_selector_structural_matches(sel, ctx) {
        return false;
    }
    interactive_pseudo_active(sel, ctx, state)
}

fn interactive_selector_structural_matches(
    sel: &InteractiveSelector,
    ctx: &MatchContext<'_>,
) -> bool {
    let static_sel = interactive_selector_as_static(sel);
    crate::css_cascade::selector_matches(&static_sel, ctx)
}

fn interactive_pseudo_active(
    sel: &InteractiveSelector,
    ctx: &MatchContext<'_>,
    state: &InteractiveMatchState<'_>,
) -> bool {
    let required = sel.pseudo;
    if sel.interactive_at == sel.ancestors.len() {
        return state.subject.has(required);
    }
    let (_, compound) = &sel.ancestors[sel.interactive_at];
    let stripped = compound_without_interactive(compound);
    for (idx, node) in ctx.ancestors.iter().enumerate() {
        if compound_matches(&stripped, node) {
            if state
                .ancestors
                .get(idx)
                .map(|flags| flags.has(required))
                .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

fn compound_without_interactive(compound: &CompoundSelector) -> CompoundSelector {
    let mut out = compound.clone();
    out.interactive = None;
    out
}

fn interactive_selector_as_static(sel: &InteractiveSelector) -> Selector {
    let mut subject = sel.subject.clone();
    subject.interactive = None;
    let ancestors = sel
        .ancestors
        .iter()
        .map(|(comb, compound)| {
            let mut c = compound.clone();
            c.interactive = None;
            (*comb, c)
        })
        .collect();
    Selector {
        subject,
        ancestors,
        specificity: sel.specificity,
    }
}

/// Parse `@keyframes name { … }` at-rule; returns parsed rule and remaining CSS.
pub fn parse_keyframes_at_rule(css: &str, source_order: u32) -> Option<(KeyframesRule, &str)> {
    let rest = css.trim_start();
    let lower = rest.to_ascii_lowercase();
    let prefix = if lower.starts_with("@keyframes") {
        "@keyframes"
    } else if lower.starts_with("@-webkit-keyframes") {
        "@-webkit-keyframes"
    } else {
        return None;
    };
    let mut cursor = &rest[prefix.len()..];
    cursor = cursor.trim_start();
    let name_end = cursor
        .find(|c: char| c.is_whitespace() || c == '{')
        .unwrap_or(cursor.len());
    if name_end == 0 {
        return None;
    }
    let name = cursor[..name_end].trim().to_string();
    cursor = &cursor[name_end..].trim_start();
    if !cursor.starts_with('{') {
        return None;
    }
    cursor = &cursor[1..];
    let (body, next) = extract_balanced_block(cursor)?;
    let blocks = parse_keyframe_blocks(body)?;
    if blocks.is_empty() {
        return None;
    }
    Some((
        KeyframesRule {
            name,
            blocks,
            source_order,
        },
        next,
    ))
}

fn extract_balanced_block(s: &str) -> Option<(&str, &str)> {
    let mut depth = 1i32;
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        match *b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[..i], &s[i + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_keyframe_blocks(body: &str) -> Option<Vec<KeyframeBlock>> {
    let mut blocks = Vec::new();
    let mut rest = body.trim();
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if rest.starts_with('}') {
            break;
        }
        let brace = rest.find('{')?;
        let selector_text = rest[..brace].trim();
        rest = &rest[brace + 1..];
        let (decl_body, next) = extract_balanced_block(rest)?;
        rest = next;
        let selectors = match parse_keyframe_selector_list(selector_text) {
            Some(s) => s,
            None => continue,
        };
        let entries = parse_declaration_entries(decl_body.trim());
        if selectors.is_empty() || entries.is_empty() {
            continue;
        }
        blocks.push(KeyframeBlock {
            selectors,
            declaration_entries: entries,
        });
    }
    if blocks.is_empty() {
        None
    } else {
        Some(blocks)
    }
}

fn parse_keyframe_selector_list(raw: &str) -> Option<Vec<KeyframeSelector>> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let lower = part.to_ascii_lowercase();
        if lower == "from" {
            out.push(KeyframeSelector::From);
            continue;
        }
        if lower == "to" {
            out.push(KeyframeSelector::To);
            continue;
        }
        let pct = if part.ends_with('%') {
            part[..part.len() - 1].trim().parse::<f32>().ok()?
        } else {
            return None;
        };
        if !(0.0..=100.0).contains(&pct) {
            return None;
        }
        out.push(KeyframeSelector::Percent(pct));
    }
    if out.is_empty() { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css_cascade::parse_stylesheet_full;

    fn hover_state<'a>(ancestors: &'a [InteractivePseudoFlags]) -> InteractiveMatchState<'a> {
        InteractiveMatchState {
            subject: InteractivePseudoFlags::default(),
            ancestors,
        }
    }

    #[test]
    fn partition_motion_pulls_animation_and_transition() {
        let entries = parse_declaration_entries(
            "width: 10px; transition: opacity 0.2s; animation-name: spin; color: red",
        );
        let (layout, motion) = partition_motion_entries(&entries);
        assert_eq!(layout.len(), 2);
        assert_eq!(motion.transition.as_deref(), Some("opacity 0.2s"));
        assert_eq!(motion.animation_name.as_deref(), Some("spin"));
    }

    #[test]
    fn parse_keyframes_at_rule_from_to_and_percent() {
        let css =
            "@keyframes spin { from { opacity: 0 } 50% { opacity: 0.5 } to { opacity: 1 } } .x{}";
        let (rule, rest) = parse_keyframes_at_rule(css, 0).expect("keyframes");
        assert_eq!(rule.name, "spin");
        assert_eq!(rule.blocks.len(), 3);
        assert_eq!(rule.blocks[0].selectors, vec![KeyframeSelector::From]);
        assert_eq!(rule.blocks[2].selectors, vec![KeyframeSelector::To]);
        assert!(rest.trim_start().starts_with(".x"));
    }

    #[test]
    fn webkit_keyframes_prefix_parses() {
        let css = "@-webkit-keyframes spin { to { opacity: 1 } } .x{}";
        let (rule, rest) = parse_keyframes_at_rule(css, 0).expect("keyframes");
        assert_eq!(rule.name, "spin");
        assert_eq!(rule.blocks.len(), 1);
        assert!(rest.trim_start().starts_with(".x"));
    }

    #[test]
    fn matched_interactive_requires_ancestor_hover_state() {
        let (sheet, _) = parse_stylesheet_full(".card:hover .icon { color: red; }", 0);
        assert_eq!(sheet.interactive_rules.len(), 1);

        let card = vec!["card".into()];
        let icon = vec!["icon".into()];
        let empty = std::collections::BTreeMap::new();
        let ancestors = [MatchNode {
            tag: "div",
            id: "",
            classes: &card,
            attrs: &empty,
            is_empty: true,
            checked: false,
        }];
        let icon_ctx = MatchContext {
            tag: "span",
            id: "",
            classes: &icon,
            attrs: &empty,
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

        let card_hovered = [InteractivePseudoFlags {
            hover: true,
            ..Default::default()
        }];
        let matched = matched_interactive_rules(
            &sheet.interactive_rules,
            &icon_ctx,
            &hover_state(&card_hovered),
            InteractivePseudo::Hover,
        );
        assert_eq!(matched.len(), 1);

        let icon_only = matched_interactive_rules(
            &sheet.interactive_rules,
            &icon_ctx,
            &InteractiveMatchState {
                subject: InteractivePseudoFlags {
                    hover: true,
                    ..Default::default()
                },
                ancestors: &[Default::default()],
            },
            InteractivePseudo::Hover,
        );
        assert!(icon_only.is_empty());
    }

    #[test]
    fn nested_card_outer_hover_matches_descendant_icon() {
        let (sheet, _) = parse_stylesheet_full(".card:hover .icon { color: red; }", 0);
        assert_eq!(sheet.interactive_rules.len(), 1);

        let card = vec!["card".into()];
        let icon = vec!["icon".into()];
        let empty = std::collections::BTreeMap::new();
        let ancestors = [
            MatchNode {
                tag: "div",
                id: "",
                classes: &card,
                attrs: &empty,
                is_empty: true,
                checked: false,
            },
            MatchNode {
                tag: "div",
                id: "",
                classes: &card,
                attrs: &empty,
                is_empty: true,
                checked: false,
            },
        ];
        let icon_ctx = MatchContext {
            tag: "span",
            id: "",
            classes: &icon,
            attrs: &empty,
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

        // Inner card not hovered; outer card hovered — must still match.
        let ancestor_flags = [
            InteractivePseudoFlags::default(),
            InteractivePseudoFlags {
                hover: true,
                ..Default::default()
            },
        ];
        let matched = matched_interactive_rules(
            &sheet.interactive_rules,
            &icon_ctx,
            &hover_state(&ancestor_flags),
            InteractivePseudo::Hover,
        );
        assert_eq!(matched.len(), 1);
    }

    #[test]
    fn motion_only_hover_rule_stored() {
        let (sheet, _) = parse_stylesheet_full(".btn:hover { transition: opacity 0.2s; }", 0);
        assert!(sheet.static_rules.is_empty());
        assert_eq!(sheet.interactive_rules.len(), 1);
        assert_eq!(
            sheet.interactive_rules[0].motion.transition.as_deref(),
            Some("opacity 0.2s")
        );
        assert!(sheet.motion_rules.is_empty());
    }

    #[test]
    fn full_parse_buckets_hover_pseudo_and_keyframes() {
        let css = r#"
            .ok { height: 100%; transition: opacity 0.2s; }
            .ok:hover { height: 50%; }
            @keyframes spin { to { transform: rotate(1turn); } }
            .ok::before { content: ""; width: 4px; }
            .card:hover .icon { color: red; }
        "#;
        let (sheet, report) = parse_stylesheet_full(css, 0);
        assert_eq!(sheet.static_rules.len(), 1);
        assert_eq!(sheet.interactive_rules.len(), 2);
        assert_eq!(sheet.generated_pseudo_rules.len(), 1);
        assert_eq!(sheet.keyframes.get("spin").map(|k| k.blocks.len()), Some(1));
        assert_eq!(sheet.motion_rules.len(), 1);
        assert_eq!(report.skipped_at_rules, 0);

        let classes = vec!["ok".into()];
        let attrs = std::collections::BTreeMap::new();
        let ctx = MatchContext {
            tag: "div",
            id: "",
            classes: &classes,
            attrs: &attrs,
            ancestors: &[],
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
        let hover = matched_interactive_rules(
            &sheet.interactive_rules,
            &ctx,
            &InteractiveMatchState {
                subject: InteractivePseudoFlags {
                    hover: true,
                    ..Default::default()
                },
                ancestors: &[],
            },
            InteractivePseudo::Hover,
        );
        assert_eq!(hover.len(), 1);
        let pseudo = matched_generated_pseudo(&sheet.generated_pseudo_rules, &ctx);
        assert_eq!(pseudo.before.len(), 1);
        assert!(pseudo.before[0].iter().any(|e| e.property == "content"));
    }

    #[test]
    fn hover_not_disabled_skips_disabled_button() {
        let (sheet, report) =
            parse_stylesheet_full("button:hover:not(:disabled) { background: red; }", 0);
        assert_eq!(report.skipped_selectors, 0);
        assert_eq!(sheet.interactive_rules.len(), 1);
        let empty = BTreeMap::new();
        let mut disabled = BTreeMap::new();
        disabled.insert("disabled".into(), String::new());
        let enabled_ctx = MatchContext {
            tag: "button",
            id: "",
            classes: &[],
            attrs: &empty,
            ancestors: &[],
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
        let hovered = InteractiveMatchState {
            subject: InteractivePseudoFlags {
                hover: true,
                ..Default::default()
            },
            ancestors: &[],
        };
        assert_eq!(
            matched_interactive_rules(
                &sheet.interactive_rules,
                &enabled_ctx,
                &hovered,
                InteractivePseudo::Hover,
            )
            .len(),
            1
        );
        let disabled_ctx = MatchContext {
            tag: "button",
            id: "",
            classes: &[],
            attrs: &disabled,
            ancestors: &[],
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
        assert!(
            matched_interactive_rules(
                &sheet.interactive_rules,
                &disabled_ctx,
                &hovered,
                InteractivePseudo::Hover,
            )
            .is_empty()
        );
    }

    #[test]
    fn focus_visible_maps_to_focus_and_requires_focus() {
        let (sheet, report) = parse_stylesheet_full(".field:focus-visible { color: red; }", 0);
        assert_eq!(report.skipped_selectors, 0);
        assert_eq!(sheet.interactive_rules.len(), 1);
        assert_eq!(
            sheet.interactive_rules[0].selector.pseudo,
            InteractivePseudo::Focus
        );
        let classes = vec!["field".into()];
        let empty = BTreeMap::new();
        let ctx = MatchContext {
            tag: "input",
            id: "",
            classes: &classes,
            attrs: &empty,
            ancestors: &[],
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
        assert!(
            matched_interactive_rules(
                &sheet.interactive_rules,
                &ctx,
                &InteractiveMatchState::default(),
                InteractivePseudo::Focus,
            )
            .is_empty(),
            "focus-visible must not match without focus"
        );
        let focused = matched_interactive_rules(
            &sheet.interactive_rules,
            &ctx,
            &InteractiveMatchState {
                subject: InteractivePseudoFlags {
                    focus: true,
                    ..Default::default()
                },
                ancestors: &[],
            },
            InteractivePseudo::Focus,
        );
        assert_eq!(focused.len(), 1);
    }
}
