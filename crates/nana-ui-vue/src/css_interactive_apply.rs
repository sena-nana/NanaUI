//! Apply parsed interactive CSS buckets onto [`LayoutStyle`] and motion contracts.

use std::collections::BTreeMap;
use std::time::Duration;

use nana_ui_runtime::{
    AnimationDirection, AnimationFillMode, AnimationId, AnimationIteration, AnimationPlayState,
    AnimationPlayback, AnimationSpec, Easing, StableNodeId,
};

use crate::css_cascade::{DeclarationEntry, MatchContext};
use crate::css_interactive::{
    InteractivePseudo, InteractiveStyleRule, KeyframeBlock, KeyframeSelector, KeyframesRule,
    MotionDeclarations, MotionStyleRule, ScrollbarPseudo, matched_interactive_rules,
    matched_motion_rules, matched_scrollbar_pseudo, partition_motion_entries,
};
use crate::css_map::{LayoutStyle, LayoutStyleCss, css_key_is_direction_or_writing_mode};

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
    pub animation_timing_function: String,
    pub animation_iteration_count: String,
    pub animation_direction: String,
    pub animation_fill_mode: String,
    pub animation_play_state: String,
}

/// Host → JS `__nanaMotionComplete` payload. Not WAAPI / `Element.animate`.
#[derive(Debug, Clone, PartialEq)]
pub struct CssMotionComplete {
    pub widget_id: u64,
    pub event_type: &'static str,
    pub property_name: String,
    pub animation_name: String,
    pub transition_property: String,
    pub elapsed_time: f32,
}

impl CssMotionComplete {
    pub fn transition_end(widget_id: u64, motion: &CssComputedMotion, elapsed_time: f32) -> Self {
        Self {
            widget_id,
            event_type: "transitionend",
            property_name: motion.transition_property.clone(),
            animation_name: String::new(),
            transition_property: motion.transition_property.clone(),
            elapsed_time,
        }
    }

    pub fn animation_end(widget_id: u64, motion: &CssComputedMotion, elapsed_time: f32) -> Self {
        Self {
            widget_id,
            event_type: "animationend",
            property_name: motion.animation_name.clone(),
            animation_name: motion.animation_name.clone(),
            transition_property: String::new(),
            elapsed_time,
        }
    }
}

impl CssComputedMotion {
    pub fn has_transition(&self) -> bool {
        parse_css_time_ms(&self.transition_duration).unwrap_or(0.0) > 0.0
            && self.transition_property != "none"
            && !self.transition_property.is_empty()
    }
}

/// Serialized used/computed longhands for `getComputedStyle` (not a CSSOM object).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CssComputedStyle {
    pub width: String,
    pub height: String,
    pub color: String,
    pub background_color: String,
    pub font_size: String,
    pub font_family: String,
    pub font_weight: String,
}

pub fn serialize_computed_style(
    layout: &LayoutStyle,
    used_width: Option<f32>,
    used_height: Option<f32>,
) -> CssComputedStyle {
    CssComputedStyle {
        width: used_width
            .map(format_px)
            .or_else(|| layout.width.map(serialize_length_spec))
            .unwrap_or_else(|| "auto".into()),
        height: used_height
            .map(format_px)
            .or_else(|| layout.height.map(serialize_length_spec))
            .unwrap_or_else(|| "auto".into()),
        color: layout.color.map(serialize_rgba).unwrap_or_default(),
        background_color: layout
            .background
            .map(serialize_rgba)
            .unwrap_or_else(|| "rgba(0, 0, 0, 0)".into()),
        font_size: layout.font_size.map(format_px).unwrap_or_default(),
        font_family: layout.font_family.clone().unwrap_or_default(),
        font_weight: layout
            .font_weight
            .map(|w| w.to_string())
            .unwrap_or_default(),
    }
}

fn format_px(v: f32) -> String {
    if v == v.trunc() {
        format!("{v:.0}px")
    } else {
        format!("{v}px")
    }
}

fn serialize_rgba(color: [f32; 4]) -> String {
    let r = (color[0].clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (color[1].clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (color[2].clamp(0.0, 1.0) * 255.0).round() as u8;
    let a = color[3].clamp(0.0, 1.0);
    if (a - 1.0).abs() < 0.001 {
        format!("rgb({r}, {g}, {b})")
    } else {
        format!("rgba({r}, {g}, {b}, {a})")
    }
}

fn serialize_length_spec(spec: crate::css_map::LengthSpec) -> String {
    use crate::css_map::LengthSpec;
    match spec {
        LengthSpec::Px(v) => format_px(v),
        LengthSpec::Percent(p) => format!("{p}%"),
        LengthSpec::Fill => "100%".into(),
        LengthSpec::Auto => "auto".into(),
        LengthSpec::Em(v) => format!("{v}em"),
        LengthSpec::Rem(v) => format!("{v}rem"),
        _ => "auto".into(),
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

/// Paint and interpolable layout-size fields for CSS transitions / keyframes.
///
/// `transform_origin` is the Copy [`nana_ui_core::TransformOrigin`] on
/// [`LayoutStyle`]: paint-only, same TRANSFORM dirty as `transform`, not a
/// layout length. `width` / `height` share this snapshot and the existing
/// `LengthSpec` lerp; applying them dirties LAYOUT. Non-interpolable sizes
/// (`min()`/`max()`/`clamp()`, other calc than percent±px) fail closed — they
/// take the target, they do not snap-fake a mid. Padding is not in this
/// snapshot. `MotionDeclarations` stays animation/transition longhands.
#[derive(Debug, Clone, PartialEq)]
pub struct CssPaintSnapshot {
    pub opacity: Option<f32>,
    pub color: Option<[f32; 4]>,
    pub background: Option<[f32; 4]>,
    pub transform: Option<nana_ui_core::box_layout::PaintTransform>,
    pub transform_3d: Option<nana_ui_core::box_layout::PaintMat4>,
    pub transform_origin: Option<nana_ui_core::box_layout::TransformOrigin>,
    pub filter: Option<nana_ui_core::box_layout::ColorFilter>,
    pub width: Option<nana_ui_core::box_layout::LengthSpec>,
    pub height: Option<nana_ui_core::box_layout::LengthSpec>,
}

impl CssPaintSnapshot {
    pub fn from_layout(layout: &LayoutStyle) -> Self {
        Self::from_layout_resolved(layout, None, None, None)
    }

    pub fn from_layout_resolved(
        layout: &LayoutStyle,
        _percent_w: Option<f32>,
        _percent_h: Option<f32>,
        _viewport: Option<(f32, f32)>,
    ) -> Self {
        Self {
            opacity: layout.opacity,
            color: layout.color,
            background: layout.background,
            transform: layout.transform,
            transform_3d: layout.transform_3d,
            transform_origin: layout.transform_origin,
            filter: layout.paint.filter,
            width: layout.width,
            height: layout.height,
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
            layout.transform_3d = None;
        }
        if let Some(transform_3d) = self.transform_3d {
            layout.transform_3d = Some(transform_3d);
            layout.transform = None;
        }
        if let Some(origin) = self.transform_origin {
            layout.transform_origin = Some(origin);
        }
        layout.paint.filter = self.filter;
        if let Some(width) = self.width {
            layout.width = Some(width);
        }
        if let Some(height) = self.height {
            layout.height = Some(height);
        }
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
        u32,
        crate::css_cascade::Specificity,
        u32,
        u32,
        DeclarationEntry,
    )> = Vec::new();
    for (_spec, _order, rule) in rules {
        for entry in &rule.declaration_entries {
            entries.push((
                entry.important,
                crate::css_cascade::cascade_layer_key(entry.important, rule.layer),
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
            .then(a.4.cmp(&b.4))
    });
    let (dir_entries, rest): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .partition(|(_, _, _, _, _, entry)| css_key_is_direction_or_writing_mode(&entry.property));
    for (_, _, _, _, _, entry) in dir_entries.into_iter().chain(rest) {
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
        let (dir_entries, rest): (Vec<_>, Vec<_>) = block
            .iter()
            .filter(|e| !e.property.eq_ignore_ascii_case("content"))
            .partition(|e| css_key_is_direction_or_writing_mode(&e.property));
        for entry in dir_entries.into_iter().chain(rest) {
            layout.apply_css_property(&entry.property, &entry.value, percent_w, percent_h);
        }
    }
}

/// Map `::placeholder` color/opacity onto the originating TextInput layout.
/// Other declarations are ignored — this is not a generated box.
pub fn apply_placeholder_paint(
    layout: &mut LayoutStyle,
    blocks: &[Vec<DeclarationEntry>],
    percent_w: Option<f32>,
    percent_h: Option<f32>,
) {
    if blocks.is_empty() {
        return;
    }
    let mut paint = LayoutStyle::default();
    apply_generated_pseudo_entries(&mut paint, blocks, percent_w, percent_h);
    if paint.color.is_some() {
        layout.placeholder_color = paint.color;
    }
    if paint.opacity.is_some() {
        layout.placeholder_opacity = paint.opacity;
    }
}

/// Overlay `::-webkit-scrollbar` / thumb color and thickness onto the originating layout.
pub fn apply_scrollbar_pseudo_skin(
    layout: &mut LayoutStyle,
    rules: &[crate::css_interactive::ScrollbarPseudoRule],
    ctx: &MatchContext<'_>,
    percent_w: Option<f32>,
    percent_h: Option<f32>,
) {
    let matched = matched_scrollbar_pseudo(rules, ctx);
    if matched.is_empty() {
        return;
    }
    let mut skin = layout.paint.scrollbar.unwrap_or_default();
    for (pseudo, entries) in matched {
        for entry in entries {
            apply_scrollbar_decl(
                &mut skin,
                pseudo,
                &entry.property,
                &entry.value,
                percent_w,
                percent_h,
            );
        }
    }
    if !skin.is_empty() {
        layout.paint.scrollbar = Some(skin);
    }
}

fn apply_scrollbar_decl(
    skin: &mut nana_ui_core::ScrollbarSkin,
    pseudo: ScrollbarPseudo,
    property: &str,
    value: &str,
    percent_w: Option<f32>,
    percent_h: Option<f32>,
) {
    let key = property.trim().to_ascii_lowercase();
    match key.as_str() {
        "width" | "height" => {
            let Some(px) = crate::css_map::parse_css_length_px(value, percent_w.or(percent_h))
            else {
                return;
            };
            let px = px.max(0.0);
            match pseudo {
                ScrollbarPseudo::Scrollbar => skin.thickness = Some(px),
                ScrollbarPseudo::Thumb => skin.thumb_thickness = Some(px),
            }
        }
        "background" | "background-color" => {
            let color = if value.trim().eq_ignore_ascii_case("transparent")
                || value.trim().eq_ignore_ascii_case("none")
            {
                Some([0.0, 0.0, 0.0, 0.0])
            } else {
                crate::style::parse_css_color(value)
            };
            let Some(color) = color else {
                return;
            };
            match pseudo {
                ScrollbarPseudo::Scrollbar => skin.track_color = Some(color),
                ScrollbarPseudo::Thumb => skin.thumb_color = Some(color),
            }
        }
        _ => {}
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
        if let Some(parsed) = parse_animation_shorthand(shorthand) {
            out.animation_name = parsed.name;
            out.animation_duration = parsed.duration;
            out.animation_delay = parsed.delay;
            out.animation_timing_function = parsed.timing_function;
            out.animation_iteration_count = parsed.iteration_count;
            out.animation_direction = parsed.direction;
            out.animation_fill_mode = parsed.fill_mode;
            out.animation_play_state = parsed.play_state;
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
    if let Some(v) = &motion.animation_timing_function {
        out.animation_timing_function = v.clone();
    }
    if let Some(v) = &motion.animation_iteration_count {
        out.animation_iteration_count = v.clone();
    }
    if let Some(v) = &motion.animation_direction {
        out.animation_direction = v.clone();
    }
    if let Some(v) = &motion.animation_fill_mode {
        out.animation_fill_mode = v.clone();
    }
    if let Some(v) = &motion.animation_play_state {
        out.animation_play_state = v.clone();
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
    if out.animation_timing_function.is_empty() {
        out.animation_timing_function = "ease".into();
    }
    if out.animation_iteration_count.is_empty() {
        out.animation_iteration_count = "1".into();
    }
    if out.animation_direction.is_empty() {
        out.animation_direction = "normal".into();
    }
    if out.animation_fill_mode.is_empty() {
        out.animation_fill_mode = "none".into();
    }
    if out.animation_play_state.is_empty() {
        out.animation_play_state = "running".into();
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

struct AnimationShorthand {
    name: String,
    duration: String,
    delay: String,
    timing_function: String,
    iteration_count: String,
    direction: String,
    fill_mode: String,
    play_state: String,
}

fn parse_animation_shorthand(raw: &str) -> Option<AnimationShorthand> {
    let tokens = split_css_tokens(raw);
    let mut name = String::new();
    let mut duration = String::new();
    let mut delay = String::new();
    let mut timing_function = String::new();
    let mut iteration_count = String::new();
    let mut direction = String::new();
    let mut fill_mode = String::new();
    let mut play_state = String::new();
    for token in tokens {
        let lower = token.to_ascii_lowercase();
        if lower.ends_with("ms") || lower.ends_with('s') {
            if duration.is_empty() {
                duration = token;
            } else if delay.is_empty() {
                delay = token;
            }
            continue;
        }
        if is_css_timing_function(&lower) {
            timing_function = token;
            continue;
        }
        if lower == "infinite" || lower.parse::<f32>().is_ok() {
            iteration_count = token;
            continue;
        }
        if matches!(
            lower.as_str(),
            "normal" | "reverse" | "alternate" | "alternate-reverse"
        ) {
            direction = token;
            continue;
        }
        if matches!(lower.as_str(), "forwards" | "backwards" | "both")
            || (lower == "none" && !name.is_empty())
        {
            fill_mode = token;
            continue;
        }
        if matches!(lower.as_str(), "running" | "paused") {
            play_state = token;
            continue;
        }
        if name.is_empty() {
            name = token;
        }
    }
    if name.is_empty() {
        return None;
    }
    Some(AnimationShorthand {
        name,
        duration: if duration.is_empty() {
            "0s".into()
        } else {
            duration
        },
        delay,
        timing_function,
        iteration_count,
        direction,
        fill_mode,
        play_state,
    })
}

fn is_css_timing_function(token: &str) -> bool {
    matches!(
        token,
        "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out" | "step-start" | "step-end"
    ) || token.starts_with("cubic-bezier(")
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
    let applies_edge = |prefix: &str, side: &str| applies(prefix) || applies(side);
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
        transform_3d: if applies("transform") {
            lerp_transform_3d(from.transform_3d, to.transform_3d, t)
        } else {
            to.transform_3d.or(from.transform_3d)
        },
        transform_origin: if applies("transform-origin") {
            lerp_transform_origin(from.transform_origin, to.transform_origin, t)
        } else {
            to.transform_origin.or(from.transform_origin)
        },
        filter: if applies("filter") {
            lerp_filter(from.filter, to.filter, t)
        } else {
            to.filter.or(from.filter)
        },
        width: if applies("width") {
            lerp_layout_length(from.width, to.width, t)
        } else {
            to.width.or(from.width)
        },
        height: if applies("height") {
            lerp_layout_length(from.height, to.height, t)
        } else {
            to.height.or(from.height)
        },
    }
}

fn lerp_opt(from: Option<f32>, to: Option<f32>, default: f32, t: f32) -> f32 {
    let a = from.unwrap_or(default);
    let b = to.unwrap_or(a);
    a + (b - a) * t
}

fn lerp_resolved(from: Option<f32>, to: Option<f32>, t: f32) -> Option<f32> {
    match (from, to) {
        (Some(a), Some(b)) => Some(a + (b - a) * t),
        _ => None,
    }
}

fn lerp_color(from: Option<[f32; 4]>, to: Option<[f32; 4]>, t: f32) -> Option<[f32; 4]> {
    match (from, to) {
        (Some(a), Some(b)) => Some(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)),
        (None, None) => None,
        (None, Some(b)) => Some(b),
        (Some(a), None) => Some(a),
    }
}

fn lerp_transform_3d(
    from: Option<nana_ui_core::box_layout::PaintMat4>,
    to: Option<nana_ui_core::box_layout::PaintMat4>,
    t: f32,
) -> Option<nana_ui_core::box_layout::PaintMat4> {
    use nana_ui_core::box_layout::PaintMat4;
    match (from, to) {
        (None, None) => None,
        (a, b) => {
            let a = a.unwrap_or(PaintMat4::IDENTITY);
            let b = b.unwrap_or(PaintMat4::IDENTITY);
            let mut m = [0.0f32; 16];
            for i in 0..16 {
                m[i] = a.m[i] + (b.m[i] - a.m[i]) * t;
            }
            PaintMat4::from_matrix3d(m)
        }
    }
}

fn lerp_transform(
    from: Option<nana_ui_core::box_layout::PaintTransform>,
    to: Option<nana_ui_core::box_layout::PaintTransform>,
    t: f32,
) -> Option<nana_ui_core::box_layout::PaintTransform> {
    use nana_ui_core::box_layout::PaintTransform;
    match (from, to) {
        (None, None) => None,
        (a, b) => {
            let a = a.unwrap_or_default();
            let b = b.unwrap_or_default();
            Some(PaintTransform {
                a: a.a + (b.a - a.a) * t,
                b: a.b + (b.b - a.b) * t,
                c: a.c + (b.c - a.c) * t,
                d: a.d + (b.d - a.d) * t,
                e: a.e + (b.e - a.e) * t,
                f: a.f + (b.f - a.f) * t,
            })
        }
    }
}

fn lerp_transform_origin(
    from: Option<nana_ui_core::box_layout::TransformOrigin>,
    to: Option<nana_ui_core::box_layout::TransformOrigin>,
    t: f32,
) -> Option<nana_ui_core::box_layout::TransformOrigin> {
    use nana_ui_core::box_layout::TransformOrigin;
    match (from, to) {
        (None, None) => None,
        (a, b) => {
            let a = a.unwrap_or_default();
            let b = b.unwrap_or_default();
            Some(TransformOrigin {
                x: lerp_length_spec(a.x, b.x, t),
                y: lerp_length_spec(a.y, b.y, t),
            })
        }
    }
}

fn lerp_layout_length(
    from: Option<nana_ui_core::box_layout::LengthSpec>,
    to: Option<nana_ui_core::box_layout::LengthSpec>,
    t: f32,
) -> Option<nana_ui_core::box_layout::LengthSpec> {
    match (from, to) {
        (Some(a), Some(b)) if length_specs_honestly_interpolable(a, b) => {
            Some(lerp_length_spec(a, b, t))
        }
        (_, b) => b,
    }
}

fn length_specs_honestly_interpolable(
    from: nana_ui_core::box_layout::LengthSpec,
    to: nana_ui_core::box_layout::LengthSpec,
) -> bool {
    use nana_ui_core::box_layout::LengthSpec;
    match (from, to) {
        (LengthSpec::Em(_), LengthSpec::Em(_)) => true,
        (LengthSpec::Rem(_), LengthSpec::Rem(_)) => true,
        (
            LengthSpec::Viewport {
                axis: from_axis, ..
            },
            LengthSpec::Viewport { axis: to_axis, .. },
        ) if from_axis == to_axis => true,
        _ => length_as_percent_px(from).is_some() && length_as_percent_px(to).is_some(),
    }
}

fn lerp_length_spec(
    from: nana_ui_core::box_layout::LengthSpec,
    to: nana_ui_core::box_layout::LengthSpec,
    t: f32,
) -> nana_ui_core::box_layout::LengthSpec {
    use nana_ui_core::box_layout::LengthSpec;
    const EPS: f32 = 1e-6;
    match (from, to) {
        (LengthSpec::Em(a), LengthSpec::Em(b)) => LengthSpec::Em(a + (b - a) * t),
        (LengthSpec::Rem(a), LengthSpec::Rem(b)) => LengthSpec::Rem(a + (b - a) * t),
        (
            LengthSpec::Viewport {
                axis: from_axis,
                value: a,
            },
            LengthSpec::Viewport {
                axis: to_axis,
                value: b,
            },
        ) if from_axis == to_axis => LengthSpec::Viewport {
            axis: from_axis,
            value: a + (b - a) * t,
        },
        _ => match (length_as_percent_px(from), length_as_percent_px(to)) {
            (Some((ap, ax)), Some((bp, bx))) => {
                let percent = ap + (bp - ap) * t;
                let offset_px = ax + (bx - ax) * t;
                if percent.abs() < EPS {
                    LengthSpec::Px(offset_px)
                } else if offset_px.abs() < EPS {
                    LengthSpec::Percent(percent)
                } else {
                    LengthSpec::CalcPercentOffset { percent, offset_px }
                }
            }
            _ if t < 0.5 => from,
            _ => to,
        },
    }
}

fn length_as_percent_px(spec: nana_ui_core::box_layout::LengthSpec) -> Option<(f32, f32)> {
    use nana_ui_core::box_layout::LengthSpec;
    match spec {
        LengthSpec::Px(px) => Some((0.0, px)),
        LengthSpec::Percent(pct) => Some((pct, 0.0)),
        LengthSpec::CalcPercentOffset { percent, offset_px } => Some((percent, offset_px)),
        _ => None,
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
            hue_rotate_deg: a.hue_rotate_deg + (b.hue_rotate_deg - a.hue_rotate_deg) * t,
            invert: a.invert + (b.invert - a.invert) * t,
            opacity: a.opacity + (b.opacity - a.opacity) * t,
            blur_radius: a.blur_radius + (b.blur_radius - a.blur_radius) * t,
            drop_shadow: lerp_drop_shadow(a.drop_shadow, b.drop_shadow, t),
        }),
        (None, None) => None,
        (None, Some(b)) => Some(b),
        (Some(a), None) => Some(a),
    }
}

fn lerp_drop_shadow(
    from: Option<nana_ui_core::FilterDropShadow>,
    to: Option<nana_ui_core::FilterDropShadow>,
    t: f32,
) -> Option<nana_ui_core::FilterDropShadow> {
    use nana_ui_core::FilterDropShadow;
    let zero = FilterDropShadow {
        offset_x: 0.0,
        offset_y: 0.0,
        blur_radius: 0.0,
        color: [0.0, 0.0, 0.0, 0.0],
    };
    if from.is_none() && to.is_none() {
        return None;
    }
    let a = from.unwrap_or(zero);
    let b = to.unwrap_or(zero);
    let out = FilterDropShadow {
        offset_x: a.offset_x + (b.offset_x - a.offset_x) * t,
        offset_y: a.offset_y + (b.offset_y - a.offset_y) * t,
        blur_radius: a.blur_radius + (b.blur_radius - a.blur_radius) * t,
        color: [
            a.color[0] + (b.color[0] - a.color[0]) * t,
            a.color[1] + (b.color[1] - a.color[1]) * t,
            a.color[2] + (b.color[2] - a.color[2]) * t,
            a.color[3] + (b.color[3] - a.color[3]) * t,
        ],
    };
    if out.color[3].abs() < 1e-5
        && out.offset_x.abs() < 1e-5
        && out.offset_y.abs() < 1e-5
        && out.blur_radius <= 0.0
    {
        None
    } else {
        Some(out)
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
    Some(AnimationSpec::new(
        css_transition_animation_id(widget_id),
        StableNodeId::new(widget_id)?,
        start,
        duration,
        Duration::from_millis(16),
        easing_from_css(&motion.transition_timing_function),
    ))
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
    Some(
        AnimationSpec::new(
            css_keyframes_animation_id(widget_id),
            StableNodeId::new(widget_id)?,
            start,
            duration,
            Duration::from_millis(16),
            easing_from_css(&motion.animation_timing_function),
        )
        .with_playback(playback_from_computed(motion)),
    )
}

pub fn playback_from_computed(motion: &CssComputedMotion) -> AnimationPlayback {
    AnimationPlayback {
        iteration_count: parse_animation_iteration(&motion.animation_iteration_count),
        direction: parse_animation_direction(&motion.animation_direction),
        fill_mode: parse_animation_fill_mode(&motion.animation_fill_mode),
        play_state: parse_animation_play_state(&motion.animation_play_state),
    }
}

pub fn parse_animation_iteration(raw: &str) -> AnimationIteration {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed == "infinite" {
        return AnimationIteration::INFINITE;
    }
    if let Ok(count) = trimmed.parse::<u32>() {
        return AnimationIteration::Count(count);
    }
    if let Ok(count) = trimmed.parse::<f32>() {
        return AnimationIteration::Count(count.max(0.0) as u32);
    }
    AnimationIteration::ONCE
}

pub fn parse_animation_direction(raw: &str) -> AnimationDirection {
    match raw.trim().to_ascii_lowercase().as_str() {
        "reverse" => AnimationDirection::Reverse,
        "alternate" => AnimationDirection::Alternate,
        "alternate-reverse" => AnimationDirection::AlternateReverse,
        _ => AnimationDirection::Normal,
    }
}

pub fn parse_animation_fill_mode(raw: &str) -> AnimationFillMode {
    match raw.trim().to_ascii_lowercase().as_str() {
        "forwards" => AnimationFillMode::Forwards,
        "backwards" => AnimationFillMode::Backwards,
        "both" => AnimationFillMode::Both,
        _ => AnimationFillMode::None,
    }
}

pub fn parse_animation_play_state(raw: &str) -> AnimationPlayState {
    match raw.trim().to_ascii_lowercase().as_str() {
        "paused" => AnimationPlayState::Paused,
        _ => AnimationPlayState::Running,
    }
}

pub fn animation_elapsed_secs(motion: &CssComputedMotion) -> f32 {
    let duration = parse_css_time_ms(&motion.animation_duration).unwrap_or(0.0) / 1000.0;
    match parse_animation_iteration(&motion.animation_iteration_count) {
        AnimationIteration::Count(count) => duration * count as f32,
        AnimationIteration::Infinite => duration,
    }
}

pub fn transition_elapsed_secs(motion: &CssComputedMotion) -> f32 {
    parse_css_time_ms(&motion.transition_duration).unwrap_or(0.0) / 1000.0
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
            has_bits: 0,
            has_args: &[],
            focus_within: false,
            is_empty: true,
            checked: false,
            media: crate::css_cascade::MediaEnv::default(),
            children: &[],
            following_siblings: &[],
            all_siblings: &[],
            ancestor_subtrees: &[],
            owned_children: &[],
            owned_following: &[],
            owned_ancestor_trees: &[],
            relative: None,
            relative_id: 0,
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
            has_bits: 0,
            has_args: &[],
            focus_within: false,
            is_empty: true,
            checked: false,
            media: crate::css_cascade::MediaEnv::default(),
            children: &[],
            following_siblings: &[],
            all_siblings: &[],
            ancestor_subtrees: &[],
            owned_children: &[],
            owned_following: &[],
            owned_ancestor_trees: &[],
            relative: None,
            relative_id: 0,
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
            is_empty: true,
            checked: false,
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
            has_bits: 0,
            has_args: &[],
            focus_within: false,
            is_empty: true,
            checked: false,
            media: crate::css_cascade::MediaEnv::default(),
            children: &[],
            following_siblings: &[],
            all_siblings: &[],
            ancestor_subtrees: &[],
            owned_children: &[],
            owned_following: &[],
            owned_ancestor_trees: &[],
            relative: None,
            relative_id: 0,
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

    #[test]
    fn lerp_interpolates_copy_transform_origin() {
        use nana_ui_core::box_layout::{LengthSpec, TransformOrigin};

        let from = CssPaintSnapshot {
            transform_origin: Some(TransformOrigin {
                x: LengthSpec::Percent(0.0),
                y: LengthSpec::Percent(0.0),
            }),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let to = CssPaintSnapshot {
            transform_origin: Some(TransformOrigin {
                x: LengthSpec::Percent(100.0),
                y: LengthSpec::Px(20.0),
            }),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let mid = lerp_paint(&from, &to, 0.5);
        assert_eq!(
            mid.transform_origin,
            Some(TransformOrigin {
                x: LengthSpec::Percent(50.0),
                y: LengthSpec::Px(10.0),
            })
        );

        let mixed_from = CssPaintSnapshot {
            transform_origin: Some(TransformOrigin {
                x: LengthSpec::Percent(50.0),
                y: LengthSpec::Px(0.0),
            }),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let mixed_to = CssPaintSnapshot {
            transform_origin: Some(TransformOrigin {
                x: LengthSpec::Px(10.0),
                y: LengthSpec::Px(0.0),
            }),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        assert_eq!(
            lerp_paint(&mixed_from, &mixed_to, 0.5)
                .transform_origin
                .expect("origin")
                .x,
            LengthSpec::CalcPercentOffset {
                percent: 25.0,
                offset_px: 5.0,
            }
        );
    }

    #[test]
    fn transition_property_transform_does_not_lerp_origin() {
        use nana_ui_core::box_layout::{LengthSpec, TransformOrigin};

        let from = CssPaintSnapshot {
            transform_origin: Some(TransformOrigin {
                x: LengthSpec::Percent(0.0),
                y: LengthSpec::Percent(0.0),
            }),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let to = CssPaintSnapshot {
            transform_origin: Some(TransformOrigin {
                x: LengthSpec::Percent(100.0),
                y: LengthSpec::Percent(100.0),
            }),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let mid = lerp_paint_for_properties(&from, &to, 0.5, &["transform".into()]);
        assert_eq!(mid.transform_origin, to.transform_origin);
        let origin_only = lerp_paint_for_properties(&from, &to, 0.5, &["transform-origin".into()]);
        assert_eq!(
            origin_only.transform_origin,
            Some(TransformOrigin {
                x: LengthSpec::Percent(50.0),
                y: LengthSpec::Percent(50.0),
            })
        );
    }

    #[test]
    fn keyframes_lerp_transform_origin() {
        use crate::css_interactive::parse_keyframes_at_rule;
        use nana_ui_core::box_layout::{LengthSpec, TransformOrigin};

        let (rule, _) = parse_keyframes_at_rule(
            "@keyframes pivot { from { transform-origin: 0 0; } to { transform-origin: 100% 100%; } }",
            0,
        )
        .expect("keyframes");
        let mid = keyframe_paint_at(&rule, 0.5).expect("sample");
        assert_eq!(
            mid.transform_origin,
            Some(TransformOrigin {
                x: LengthSpec::Percent(50.0),
                y: LengthSpec::Percent(50.0),
            })
        );
    }

    #[test]
    fn lerp_interpolates_px_width_and_height() {
        use nana_ui_core::box_layout::LengthSpec;

        let from = CssPaintSnapshot {
            width: Some(LengthSpec::Px(10.0)),
            height: Some(LengthSpec::Px(20.0)),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let to = CssPaintSnapshot {
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(80.0)),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let mid = lerp_paint(&from, &to, 0.5);
        assert_eq!(mid.width, Some(LengthSpec::Px(25.0)));
        assert_eq!(mid.height, Some(LengthSpec::Px(50.0)));
        let origin_only = lerp_paint_for_properties(&from, &to, 0.5, &["transform-origin".into()]);
        assert_eq!(origin_only.width, to.width);
        assert_eq!(origin_only.height, to.height);
    }

    #[test]
    fn lerp_width_fail_closes_min2_without_snap_fake() {
        use nana_ui_core::box_layout::{LengthAtom, LengthSpec};

        let from = CssPaintSnapshot {
            width: Some(LengthSpec::Min2(LengthAtom::Px(10.0), LengthAtom::Px(80.0))),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let to = CssPaintSnapshot {
            width: Some(LengthSpec::Px(40.0)),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let early = lerp_paint(&from, &to, 0.25);
        assert_eq!(
            early.width, to.width,
            "Min2 cannot interpolate; fail-closed to target, not t<0.5 snap-fake"
        );
        let calc_from = CssPaintSnapshot {
            width: Some(LengthSpec::CalcEmOffset {
                em: 2.0,
                offset_px: 8.0,
            }),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let calc_to = CssPaintSnapshot {
            width: Some(LengthSpec::Px(40.0)),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        assert_eq!(
            lerp_paint(&calc_from, &calc_to, 0.25).width,
            calc_to.width,
            "non percent±px calc cannot interpolate; fail-closed to target"
        );
    }

    #[test]
    fn keyframes_lerp_px_width() {
        use crate::css_interactive::parse_keyframes_at_rule;
        use nana_ui_core::box_layout::LengthSpec;

        let (rule, _) = parse_keyframes_at_rule(
            "@keyframes grow { from { width: 10px; } to { width: 40px; } }",
            0,
        )
        .expect("keyframes");
        let mid = keyframe_paint_at(&rule, 0.5).expect("sample");
        assert_eq!(mid.width, Some(LengthSpec::Px(25.0)));
    }
}
