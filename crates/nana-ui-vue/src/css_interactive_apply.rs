//! Apply parsed interactive CSS buckets onto [`LayoutStyle`] and motion contracts.

use std::collections::BTreeMap;
use std::time::Duration;

use nana_ui_runtime::{AnimationId, AnimationSpec, Easing, StableNodeId};

use crate::css_cascade::{DeclarationEntry, MatchContext};
use crate::css_interactive::{
    InteractivePseudo, InteractiveStyleRule, KeyframeBlock, KeyframeSelector, KeyframesRule,
    MotionDeclarations, MotionStyleRule, matched_interactive_rules, matched_motion_rules,
    partition_motion_entries,
};
use crate::css_map::{LayoutStyle, LayoutStyleCss};

/// Resolved transition / animation longhands exposed to `getComputedStyle`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CssComputedMotion {
    pub transition_property: String,
    pub transition_duration: String,
    pub transition_delay: String,
    pub transition_timing_function: String,
    pub animation_name: String,
    pub animation_duration: String,
    pub animation_delay: String,
}

impl CssComputedMotion {
    pub fn has_transition(&self) -> bool {
        parse_css_time_ms(&self.transition_duration).unwrap_or(0.0) > 0.0
            && self.transition_property != "none"
            && !self.transition_property.is_empty()
    }
}

/// Snapshot of Runtime pointer / focus activation for cascade matching.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractiveRuntimeSnapshot {
    pub hovered: BTreeMap<u64, ()>,
    pub pressed: BTreeMap<u64, ()>,
    pub focused: Option<u64>,
}

impl InteractiveRuntimeSnapshot {
    pub fn subject_flags(&self, id: u64) -> crate::css_interactive::InteractivePseudoFlags {
        use crate::css_interactive::InteractivePseudoFlags;
        InteractivePseudoFlags {
            hover: self.hovered.contains_key(&id),
            focus: self.focused == Some(id),
            active: self.pressed.contains_key(&id),
        }
    }

    pub fn ancestor_flags(
        &self,
        bridge: &crate::MessageBridge,
        id: u64,
    ) -> Vec<crate::css_interactive::InteractivePseudoFlags> {
        bridge.interactive_ancestor_flags(id)
    }
}

/// Paint fields interpolated by CSS transitions / keyframes.
#[derive(Debug, Clone, PartialEq)]
pub struct CssPaintSnapshot {
    pub opacity: Option<f32>,
    pub color: Option<[f32; 4]>,
    pub background: Option<[f32; 4]>,
    pub transform: Option<nana_ui_core::box_layout::PaintTransform>,
    pub filter: Option<nana_ui_core::box_layout::ColorFilter>,
}

impl CssPaintSnapshot {
    pub fn from_layout(layout: &LayoutStyle) -> Self {
        Self {
            opacity: layout.opacity,
            color: layout.color,
            background: layout.background,
            transform: layout.transform,
            filter: layout.paint.filter,
        }
    }

    pub fn apply_to_layout(&self, layout: &mut LayoutStyle) {
        if let Some(opacity) = self.opacity {
            layout.opacity = Some(opacity);
        }
        if let Some(color) = self.color {
            layout.color = Some(color);
        }
        if let Some(background) = self.background {
            layout.background = Some(background);
        }
        if let Some(transform) = self.transform {
            layout.transform = Some(transform);
        }
        layout.paint.filter = self.filter;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveCssTransition {
    pub from: CssPaintSnapshot,
    pub to: CssPaintSnapshot,
    pub spec: AnimationSpec,
}

const CSS_TRANSITION_ANIMATION_BASE: u64 = 0xC000_0000_0000_0000;

pub fn css_transition_animation_id(widget_id: u64) -> AnimationId {
    AnimationId::new(CSS_TRANSITION_ANIMATION_BASE | (widget_id & 0x3FFF_FFFF_FFFF_FFFF))
        .expect("css transition animation id is nonzero")
}

pub fn css_keyframes_animation_id(widget_id: u64) -> AnimationId {
    AnimationId::new((CSS_TRANSITION_ANIMATION_BASE >> 1) | (widget_id & 0x3FFF_FFFF_FFFF_FFFF))
        .expect("css keyframes animation id is nonzero")
}

pub fn apply_interactive_declarations(
    layout: &mut LayoutStyle,
    rules: &[(crate::css_cascade::Specificity, u32, &InteractiveStyleRule)],
    percent_w: Option<f32>,
    percent_h: Option<f32>,
) {
    let mut entries: Vec<(
        bool,
        crate::css_cascade::Specificity,
        u32,
        u32,
        DeclarationEntry,
    )> = Vec::new();
    for (_spec, _order, rule) in rules {
        for entry in &rule.declaration_entries {
            entries.push((
                entry.important,
                rule.selector.specificity,
                rule.source_order,
                entry.index,
                entry.clone(),
            ));
        }
    }
    entries.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });
    for (_, _, _, _, entry) in entries {
        layout.apply_css_property(&entry.property, &entry.value, percent_w, percent_h);
    }
}

pub fn apply_generated_pseudo_entries(
    layout: &mut LayoutStyle,
    blocks: &[Vec<DeclarationEntry>],
    percent_w: Option<f32>,
    percent_h: Option<f32>,
) {
    for block in blocks {
        for entry in block {
            if entry.property.eq_ignore_ascii_case("content") {
                continue;
            }
            layout.apply_css_property(&entry.property, &entry.value, percent_w, percent_h);
        }
    }
}

pub fn parse_content_text(entries: &[DeclarationEntry]) -> Option<String> {
    let value = entries
        .iter()
        .find(|e| e.property.eq_ignore_ascii_case("content"))?
        .value
        .trim();
    if value.is_empty() || value.eq_ignore_ascii_case("none") {
        return Some(String::new());
    }
    if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        return Some(value[1..value.len() - 1].to_string());
    }
    None
}

pub fn generated_pseudo_has_content(entries: &[DeclarationEntry]) -> bool {
    entries
        .iter()
        .any(|e| e.property.eq_ignore_ascii_case("content"))
}

pub fn resolve_computed_motion(
    static_motion: &[MotionStyleRule],
    interactive_motion: Option<&MotionDeclarations>,
    generated_motion: Option<&MotionDeclarations>,
    ctx: &MatchContext<'_>,
) -> CssComputedMotion {
    let mut motion = MotionDeclarations::default();
    for (_, _, rule) in matched_motion_rules(static_motion, ctx) {
        merge_motion(&mut motion, &rule.motion);
    }
    if let Some(extra) = interactive_motion {
        merge_motion(&mut motion, extra);
    }
    if let Some(extra) = generated_motion {
        merge_motion(&mut motion, extra);
    }
    motion_to_computed(&motion)
}

fn merge_motion(target: &mut MotionDeclarations, source: &MotionDeclarations) {
    macro_rules! take {
        ($field:ident) => {
            if let Some(v) = &source.$field {
                target.$field = Some(v.clone());
            }
        };
    }
    take!(transition);
    take!(transition_property);
    take!(transition_duration);
    take!(transition_timing_function);
    take!(transition_delay);
    take!(animation);
    take!(animation_name);
    take!(animation_duration);
    take!(animation_timing_function);
    take!(animation_delay);
    take!(animation_iteration_count);
    take!(animation_direction);
    take!(animation_fill_mode);
    take!(animation_play_state);
}

fn motion_to_computed(motion: &MotionDeclarations) -> CssComputedMotion {
    let mut out = CssComputedMotion::default();
    if let Some(shorthand) = motion.transition.as_deref() {
        if let Some(parsed) = parse_transition_shorthand(shorthand) {
            out.transition_property = parsed.property;
            out.transition_duration = parsed.duration;
            out.transition_timing_function = parsed.timing_function;
            out.transition_delay = parsed.delay;
        }
    }
    if let Some(v) = &motion.transition_property {
        out.transition_property = v.clone();
    }
    if let Some(v) = &motion.transition_duration {
        out.transition_duration = v.clone();
    }
    if let Some(v) = &motion.transition_timing_function {
        out.transition_timing_function = v.clone();
    }
    if let Some(v) = &motion.transition_delay {
        out.transition_delay = v.clone();
    }
    if out.transition_property.is_empty() {
        out.transition_property = "all".into();
    }
    if out.transition_duration.is_empty() {
        out.transition_duration = "0s".into();
    }
    if out.transition_delay.is_empty() {
        out.transition_delay = "0s".into();
    }
    if out.transition_timing_function.is_empty() {
        out.transition_timing_function = "ease".into();
    }

    if let Some(shorthand) = motion.animation.as_deref() {
        if let Some(parsed) = parse_animation_shorthand_name_duration(shorthand) {
            out.animation_name = parsed.0;
            out.animation_duration = parsed.1;
        }
    }
    if let Some(v) = &motion.animation_name {
        out.animation_name = v.clone();
    }
    if let Some(v) = &motion.animation_duration {
        out.animation_duration = v.clone();
    }
    if let Some(v) = &motion.animation_delay {
        out.animation_delay = v.clone();
    }
    if out.animation_name.is_empty() {
        out.animation_name = "none".into();
    }
    if out.animation_duration.is_empty() {
        out.animation_duration = "0s".into();
    }
    if out.animation_delay.is_empty() {
        out.animation_delay = "0s".into();
    }
    out
}

struct TransitionShorthand {
    property: String,
    duration: String,
    timing_function: String,
    delay: String,
}

fn parse_transition_shorthand(raw: &str) -> Option<TransitionShorthand> {
    let mut property = String::new();
    let mut duration = String::new();
    let mut timing_function = String::new();
    let mut delay = String::new();
    for token in split_css_tokens(raw) {
        if token.ends_with("ms") || token.ends_with('s') {
            if duration.is_empty() {
                duration = token;
            } else {
                delay = token;
            }
            continue;
        }
        if matches!(
            token.as_str(),
            "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out" | "step-start" | "step-end"
        ) || token.starts_with("cubic-bezier(")
        {
            timing_function = token;
            continue;
        }
        if property.is_empty() {
            property = token;
        }
    }
    if duration.is_empty() {
        return None;
    }
    Some(TransitionShorthand {
        property: if property.is_empty() {
            "all".into()
        } else {
            property
        },
        duration,
        timing_function: if timing_function.is_empty() {
            "ease".into()
        } else {
            timing_function
        },
        delay: if delay.is_empty() { "0s".into() } else { delay },
    })
}

fn parse_animation_shorthand_name_duration(raw: &str) -> Option<(String, String)> {
    let tokens = split_css_tokens(raw);
    let mut name = String::new();
    let mut duration = String::new();
    for token in tokens {
        if token.ends_with("ms") || token.ends_with('s') {
            if duration.is_empty() {
                duration = token;
            }
            continue;
        }
        if name.is_empty() && token != "infinite" {
            name = token;
        }
    }
    if name.is_empty() {
        None
    } else {
        Some((
            name,
            if duration.is_empty() {
                "0s".into()
            } else {
                duration
            },
        ))
    }
}

fn split_css_tokens(raw: &str) -> Vec<String> {
    raw.split_whitespace()
        .map(|t| t.trim().trim_end_matches(',').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

pub fn parse_css_time_ms(raw: &str) -> Option<f32> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.ends_with("ms") {
        return trimmed[..trimmed.len() - 2]
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| v.max(0.0));
    }
    if trimmed.ends_with('s') {
        return trimmed[..trimmed.len() - 1]
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| (v * 1000.0).max(0.0));
    }
    None
}

pub fn easing_from_css(name: &str) -> Easing {
    match name.trim().to_ascii_lowercase().as_str() {
        "linear" => Easing::Linear,
        "ease-in-out" | "ease-in-out-cubic" => Easing::EaseInOutCubic,
        _ => Easing::EaseOutCubic,
    }
}

pub fn parse_transition_properties(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty() && !token.eq_ignore_ascii_case("none"))
        .map(str::to_string)
        .collect()
}

pub fn lerp_paint(from: &CssPaintSnapshot, to: &CssPaintSnapshot, t: f32) -> CssPaintSnapshot {
    lerp_paint_for_properties(from, to, t, &["all".into()])
}

pub fn lerp_paint_for_properties(
    from: &CssPaintSnapshot,
    to: &CssPaintSnapshot,
    t: f32,
    properties: &[String],
) -> CssPaintSnapshot {
    let t = t.clamp(0.0, 1.0);
    let all = properties.is_empty()
        || properties
            .iter()
            .any(|property| property.eq_ignore_ascii_case("all"));
    let applies = |name: &str| {
        all || properties
            .iter()
            .any(|property| property.eq_ignore_ascii_case(name))
    };
    CssPaintSnapshot {
        opacity: if applies("opacity") {
            Some(lerp_opt(from.opacity, to.opacity, 1.0, t))
        } else {
            to.opacity.or(from.opacity)
        },
        color: if applies("color") {
            lerp_color(from.color, to.color, t)
        } else {
            to.color.or(from.color)
        },
        background: if applies("background") || applies("background-color") {
            lerp_color(from.background, to.background, t)
        } else {
            to.background.or(from.background)
        },
        transform: if applies("transform") {
            lerp_transform(from.transform, to.transform, t)
        } else {
            to.transform.or(from.transform)
        },
        filter: if applies("filter") {
            lerp_filter(from.filter, to.filter, t)
        } else {
            to.filter.or(from.filter)
        },
    }
}

fn lerp_opt(from: Option<f32>, to: Option<f32>, default: f32, t: f32) -> f32 {
    let a = from.unwrap_or(default);
    let b = to.unwrap_or(a);
    a + (b - a) * t
}

fn lerp_color(from: Option<[f32; 4]>, to: Option<[f32; 4]>, t: f32) -> Option<[f32; 4]> {
    match (from, to) {
        (Some(a), Some(b)) => Some(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)),
        (None, None) => None,
        (None, Some(b)) => Some(b),
        (Some(a), None) => Some(a),
    }
}

fn lerp_transform(
    from: Option<nana_ui_core::box_layout::PaintTransform>,
    to: Option<nana_ui_core::box_layout::PaintTransform>,
    t: f32,
) -> Option<nana_ui_core::box_layout::PaintTransform> {
    use nana_ui_core::box_layout::PaintTransform;
    match (from, to) {
        (Some(a), Some(b)) => Some(PaintTransform {
            a: a.a + (b.a - a.a) * t,
            b: a.b + (b.b - a.b) * t,
            c: a.c + (b.c - a.c) * t,
            d: a.d + (b.d - a.d) * t,
            e: a.e + (b.e - a.e) * t,
            f: a.f + (b.f - a.f) * t,
        }),
        (None, None) => None,
        (None, Some(b)) => Some(b),
        (Some(a), None) => Some(a),
    }
}

fn lerp_filter(
    from: Option<nana_ui_core::box_layout::ColorFilter>,
    to: Option<nana_ui_core::box_layout::ColorFilter>,
    t: f32,
) -> Option<nana_ui_core::box_layout::ColorFilter> {
    use nana_ui_core::box_layout::ColorFilter;
    match (from, to) {
        (Some(a), Some(b)) => Some(ColorFilter {
            brightness: a.brightness + (b.brightness - a.brightness) * t,
            contrast: a.contrast + (b.contrast - a.contrast) * t,
            saturate: a.saturate + (b.saturate - a.saturate) * t,
        }),
        (None, None) => None,
        (None, Some(b)) => Some(b),
        (Some(a), None) => Some(a),
    }
}

pub fn keyframe_paint_at(rule: &KeyframesRule, progress: f32) -> Option<CssPaintSnapshot> {
    let pct = progress.clamp(0.0, 1.0) * 100.0;
    let mut stops: Vec<(f32, &KeyframeBlock)> = rule
        .blocks
        .iter()
        .filter_map(|block| {
            let key = block
                .selectors
                .iter()
                .map(|sel| match sel {
                    KeyframeSelector::From => 0.0,
                    KeyframeSelector::To => 100.0,
                    KeyframeSelector::Percent(p) => *p,
                })
                .fold(f32::INFINITY, f32::min);
            (key < f32::INFINITY).then_some((key, block))
        })
        .collect();
    if stops.is_empty() {
        return None;
    }
    stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let (before_key, before) = stops.iter().rev().find(|(k, _)| *k <= pct).copied()?;
    let (after_key, after) = stops.iter().find(|(k, _)| *k >= pct).copied()?;
    if (before_key - after_key).abs() < f32::EPSILON {
        return Some(paint_from_entries(&before.declaration_entries));
    }
    let local = ((pct - before_key) / (after_key - before_key)).clamp(0.0, 1.0);
    let from = paint_from_entries(&before.declaration_entries);
    let to = paint_from_entries(&after.declaration_entries);
    Some(lerp_paint(&from, &to, local))
}

fn paint_from_entries(entries: &[DeclarationEntry]) -> CssPaintSnapshot {
    let mut layout = LayoutStyle::default();
    for entry in entries {
        let (layout_entries, _) = partition_motion_entries(std::slice::from_ref(entry));
        for e in layout_entries {
            layout.apply_css_property(&e.property, &e.value, None, None);
        }
    }
    CssPaintSnapshot::from_layout(&layout)
}

pub fn build_transition_spec(
    widget_id: u64,
    motion: &CssComputedMotion,
    now: Duration,
) -> Option<AnimationSpec> {
    let duration_ms = parse_css_time_ms(&motion.transition_duration)?;
    if duration_ms <= 0.0 {
        return None;
    }
    let delay_ms = parse_css_time_ms(&motion.transition_delay).unwrap_or(0.0);
    let duration = Duration::from_secs_f32(duration_ms / 1000.0);
    let delay = Duration::from_secs_f32(delay_ms / 1000.0);
    let start = now.checked_add(delay)?;
    Some(AnimationSpec {
        id: css_transition_animation_id(widget_id),
        target: StableNodeId::new(widget_id)?,
        start,
        duration,
        frame_interval: Duration::from_millis(16),
        easing: easing_from_css(&motion.transition_timing_function),
    })
}

pub fn build_keyframes_spec(
    widget_id: u64,
    motion: &CssComputedMotion,
    now: Duration,
) -> Option<AnimationSpec> {
    if motion.animation_name.eq_ignore_ascii_case("none") {
        return None;
    }
    let duration_ms = parse_css_time_ms(&motion.animation_duration).unwrap_or(0.0);
    if duration_ms <= 0.0 {
        return None;
    }
    let delay_ms = parse_css_time_ms(&motion.animation_delay).unwrap_or(0.0);
    let duration = Duration::from_secs_f32(duration_ms / 1000.0);
    let delay = Duration::from_secs_f32(delay_ms / 1000.0);
    let start = now.checked_add(delay)?;
    Some(AnimationSpec {
        id: css_keyframes_animation_id(widget_id),
        target: StableNodeId::new(widget_id)?,
        start,
        duration,
        frame_interval: Duration::from_millis(16),
        easing: Easing::Linear,
    })
}

pub fn apply_interactive_layers(
    layout: &mut LayoutStyle,
    ctx: &MatchContext<'_>,
    interactive_rules: &[InteractiveStyleRule],
    state: &crate::css_interactive::InteractiveMatchState<'_>,
    percent_w: Option<f32>,
    percent_h: Option<f32>,
) {
    for pseudo in [
        InteractivePseudo::Hover,
        InteractivePseudo::Focus,
        InteractivePseudo::Active,
    ] {
        let matched = matched_interactive_rules(interactive_rules, ctx, state, pseudo);
        apply_interactive_declarations(layout, &matched, percent_w, percent_h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css_cascade::{MatchContext, MatchNode, parse_stylesheet_full};
    use crate::css_interactive::{InteractiveMatchState, InteractivePseudoFlags};

    #[test]
    fn transition_shorthand_parses_duration() {
        let parsed = parse_transition_shorthand("opacity 0.2s ease").expect("transition");
        assert_eq!(parsed.property, "opacity");
        assert_eq!(parsed.duration, "0.2s");
        assert_eq!(parse_css_time_ms(&parsed.duration), Some(200.0));
    }

    #[test]
    fn computed_motion_reports_nonzero_transition() {
        let (sheet, _) = parse_stylesheet_full(".btn { transition: opacity 0.2s; }", 0);
        let ctx = MatchContext {
            tag: "button",
            id: "",
            classes: &["btn".to_string()],
            attrs: &Default::default(),
            ancestors: &[],
            preceding_siblings: &[],
            sibling_index: 0,
            sibling_count: 1,
            of_type_index: 0,
            of_type_count: 1,
        };
        let motion = resolve_computed_motion(&sheet.motion_rules, None, None, &ctx);
        assert!(motion.has_transition());
        assert_eq!(motion.transition_duration, "0.2s");
    }

    #[test]
    fn hover_background_applies_only_with_hover_state() {
        let (sheet, _) = parse_stylesheet_full(
            ".ok { background: blue; } .ok:hover { background: red; }",
            0,
        );
        let classes = vec!["ok".to_string()];
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
        };
        let mut base = LayoutStyle::default();
        crate::css_cascade::apply_stylesheet_to_layout(
            &mut base,
            &sheet.static_rules,
            &ctx,
            None,
            None,
        );
        let idle = base.background;
        let mut hover = base.clone();
        apply_interactive_layers(
            &mut hover,
            &ctx,
            &sheet.interactive_rules,
            &InteractiveMatchState {
                subject: InteractivePseudoFlags {
                    hover: true,
                    ..Default::default()
                },
                ancestors: &[],
            },
            None,
            None,
        );
        assert_ne!(idle, hover.background);
        assert_eq!(hover.background, Some([1.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn card_hover_applies_to_descendant_icon() {
        let (sheet, _) = parse_stylesheet_full(".card:hover .icon { color: red; }", 0);
        let card = vec!["card".to_string()];
        let icon = vec!["icon".to_string()];
        let empty = std::collections::BTreeMap::new();
        let ancestors = [MatchNode {
            tag: "div",
            id: "",
            classes: &card,
            attrs: &empty,
        }];
        let ctx = MatchContext {
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
        };
        let mut layout = LayoutStyle::default();
        apply_interactive_layers(
            &mut layout,
            &ctx,
            &sheet.interactive_rules,
            &InteractiveMatchState {
                subject: Default::default(),
                ancestors: &[InteractivePseudoFlags {
                    hover: true,
                    ..Default::default()
                }],
            },
            None,
            None,
        );
        assert_eq!(layout.color, Some([1.0, 0.0, 0.0, 1.0]));
    }
}
