//! Apply parsed interactive CSS buckets onto [`LayoutStyle`] and motion contracts.

use std::{collections::BTreeMap, time::Duration};

use nana_ui_runtime::{
    AnimatableProperty, AnimationClass, AnimationDirection, AnimationFillMode, AnimationId,
    AnimationIteration, AnimationPlayState, AnimationPlayback, AnimationSpec, Easing, Keyframe,
    MotionCurve, MotionTo, MotionValue, StableNodeId, StepJump, classify_animatable_property,
};

use crate::{
    css_cascade::{DeclarationEntry, MatchContext},
    css_interactive::{
        InteractivePseudo, InteractiveStyleRule, KeyframeBlock, KeyframeSelector, KeyframesRule,
        MotionDeclarations, MotionStyleRule, ScrollbarPseudo, matched_interactive_rules,
        matched_motion_rules, matched_scrollbar_pseudo, partition_motion_entries,
    },
    css_map::{LayoutStyle, LayoutStyleCss, css_key_is_direction_or_writing_mode},
};

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

    /// Writes compositor longhands too. Recascade / tick must use
    /// [`Self::apply_cpu_to_layout`] so opacity/transform stay on the overlay.
    #[allow(dead_code)]
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

    /// Paint / Layout longhands only. Opacity / transform stay on the
    /// compositor overlay; writing them here would make UiWorld the visual clock.
    pub fn apply_cpu_to_layout(&self, layout: &mut LayoutStyle) {
        if let Some(color) = self.color {
            layout.color = Some(color);
        }
        if let Some(background) = self.background {
            layout.background = Some(background);
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
        if let Some(transform_3d) = self.transform_3d {
            layout.transform_3d = Some(transform_3d);
            layout.transform = None;
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveCssTransition {
    pub from: CssPaintSnapshot,
    pub to: CssPaintSnapshot,
    pub spec: AnimationSpec,
    pub overlay_ids: Vec<AnimationId>,
    pub cpu_id: Option<AnimationId>,
    pub cpu_properties: Vec<String>,
}

impl ActiveCssTransition {
    pub fn tracks_sample(&self, id: AnimationId) -> bool {
        self.cpu_id == Some(id) || self.overlay_ids.contains(&id)
    }

    pub fn is_cpu_sample(&self, id: AnimationId) -> bool {
        self.cpu_id == Some(id)
    }

    pub fn note_finished(&mut self, id: AnimationId) {
        if self.cpu_id == Some(id) {
            self.cpu_id = None;
        }
        self.overlay_ids.retain(|overlay| *overlay != id);
    }

    pub fn all_tracks_finished(&self) -> bool {
        self.cpu_id.is_none() && self.overlay_ids.is_empty()
    }
}

/// CSS transition / `@keyframes` compiled onto Motion IR tracks.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledCssMotion {
    pub cpu: Option<AnimationSpec>,
    pub overlays: Vec<AnimationSpec>,
}

impl CompiledCssMotion {
    pub fn is_empty(&self) -> bool {
        self.cpu.is_none() && self.overlays.is_empty()
    }

    pub fn overlay_ids(&self) -> Vec<AnimationId> {
        self.overlays.iter().map(|spec| spec.id).collect()
    }

    pub fn primary_spec(&self) -> Option<AnimationSpec> {
        self.cpu.clone().or_else(|| self.overlays.first().cloned())
    }
}

const CSS_TRANSITION_ANIMATION_BASE: u64 = 0xC000_0000_0000_0000;
const CSS_KEYFRAMES_ANIMATION_BASE: u64 = CSS_TRANSITION_ANIMATION_BASE >> 1;
const CSS_OVERLAY_TAG_SHIFT: u64 = 48;
const CSS_WIDGET_ID_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

pub fn css_transition_animation_id(widget_id: u64) -> AnimationId {
    AnimationId::new(CSS_TRANSITION_ANIMATION_BASE | (widget_id & 0x3FFF_FFFF_FFFF_FFFF))
        .expect("css transition animation id is nonzero")
}

pub fn css_keyframes_animation_id(widget_id: u64) -> AnimationId {
    AnimationId::new(CSS_KEYFRAMES_ANIMATION_BASE | (widget_id & 0x3FFF_FFFF_FFFF_FFFF))
        .expect("css keyframes animation id is nonzero")
}

fn css_overlay_tag(property: AnimatableProperty) -> u64 {
    match property {
        AnimatableProperty::Opacity => 1,
        AnimatableProperty::Transform => 2,
        AnimatableProperty::Clip => 3,
        AnimatableProperty::ShaderParameter => 4,
        AnimatableProperty::Width => 5,
        AnimatableProperty::Height => 6,
        _ => 0,
    }
}

pub fn css_transition_overlay_id(widget_id: u64, property: AnimatableProperty) -> AnimationId {
    let tag = css_overlay_tag(property);
    AnimationId::new(
        CSS_TRANSITION_ANIMATION_BASE
            | (tag << CSS_OVERLAY_TAG_SHIFT)
            | (widget_id & CSS_WIDGET_ID_MASK),
    )
    .expect("css transition overlay id is nonzero")
}

pub fn css_keyframes_overlay_id(widget_id: u64, property: AnimatableProperty) -> AnimationId {
    let tag = css_overlay_tag(property);
    AnimationId::new(
        CSS_KEYFRAMES_ANIMATION_BASE
            | (tag << CSS_OVERLAY_TAG_SHIFT)
            | (widget_id & CSS_WIDGET_ID_MASK),
    )
    .expect("css keyframes overlay id is nonzero")
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

/// Map `::selection` / `::-moz-selection` background/color onto the originating layout.
/// Other highlight properties stay fail-closed — this is not a CSS Highlight API.
pub fn apply_selection_paint(
    layout: &mut LayoutStyle,
    blocks: &[Vec<DeclarationEntry>],
    _percent_w: Option<f32>,
    _percent_h: Option<f32>,
) {
    if blocks.is_empty() {
        return;
    }
    for entries in blocks {
        for entry in entries {
            let key = entry.property.trim().to_ascii_lowercase();
            match key.as_str() {
                "color" => {
                    if let Some(color) = crate::css_map::resolve_paint_color(&entry.value) {
                        layout.selection_color = Some(color);
                    }
                }
                "background" | "background-color" => {
                    if let Some(color) = crate::css_map::resolve_paint_color(&entry.value) {
                        layout.selection_background = Some(color);
                    }
                }
                _ => {}
            }
        }
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
    if let Some(shorthand) = motion.transition.as_deref()
        && let Some(parsed) = parse_transition_shorthand(shorthand)
    {
        out.transition_property = parsed.property;
        out.transition_duration = parsed.duration;
        out.transition_timing_function = parsed.timing_function;
        out.transition_delay = parsed.delay;
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

    if let Some(shorthand) = motion.animation.as_deref()
        && let Some(parsed) = parse_animation_shorthand(shorthand)
    {
        out.animation_name = parsed.name;
        out.animation_duration = parsed.duration;
        out.animation_delay = parsed.delay;
        out.animation_timing_function = parsed.timing_function;
        out.animation_iteration_count = parsed.iteration_count;
        out.animation_direction = parsed.direction;
        out.animation_fill_mode = parsed.fill_mode;
        out.animation_play_state = parsed.play_state;
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
    let mut items = split_css_comma_list(raw);
    if items.is_empty() {
        items = vec![raw.to_string()];
    }
    let mut properties = Vec::new();
    let mut durations = Vec::new();
    let mut timings = Vec::new();
    let mut delays = Vec::new();
    for item in items {
        // CSS defaults a missing `transition-duration` to `0s` **per item**.
        // Failing the item — and with `?`, the whole list — meant
        // `transition: opacity, transform 200ms` lost the transform transition
        // as well as the opacity one.
        let parsed = parse_transition_item(&item);
        properties.push(parsed.property);
        durations.push(parsed.duration);
        timings.push(parsed.timing_function);
        delays.push(parsed.delay);
    }
    if durations.iter().all(|duration| duration == "0s") {
        return None;
    }
    Some(TransitionShorthand {
        property: properties.join(", "),
        duration: durations.join(", "),
        timing_function: timings.join(", "),
        delay: delays.join(", "),
    })
}

fn parse_transition_item(raw: &str) -> TransitionShorthand {
    let mut property = String::new();
    let mut duration = String::new();
    let mut timing_function = String::new();
    let mut delay = String::new();
    for token in split_css_tokens(raw) {
        let lower = token.to_ascii_lowercase();
        if is_css_time_token(&lower) {
            if duration.is_empty() {
                duration = token;
            } else {
                delay = token;
            }
            continue;
        }
        if is_css_timing_function(&lower) {
            timing_function = token;
            continue;
        }
        if property.is_empty() {
            property = token;
        }
    }
    TransitionShorthand {
        property: if property.is_empty() {
            "all".into()
        } else {
            property
        },
        duration: if duration.is_empty() {
            "0s".into()
        } else {
            duration
        },
        timing_function: if timing_function.is_empty() {
            "ease".into()
        } else {
            timing_function
        },
        delay: if delay.is_empty() { "0s".into() } else { delay },
    }
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
        "linear"
            | "ease"
            | "ease-in"
            | "ease-out"
            | "ease-in-out"
            | "ease-in-out-cubic"
            | "step-start"
            | "step-end"
    ) || token.starts_with("cubic-bezier(")
        || token.starts_with("steps(")
}

fn is_css_time_token(token: &str) -> bool {
    token.ends_with("ms") || token.ends_with('s')
}

fn split_css_tokens(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for ch in raw.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            _ if ch.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

pub fn split_css_comma_list(raw: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for ch in raw.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                let item = current.trim().to_string();
                if !item.is_empty() {
                    items.push(item);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let item = current.trim().to_string();
    if !item.is_empty() {
        items.push(item);
    }
    items
}

fn css_list_at(list: &[String], index: usize) -> &str {
    if list.is_empty() {
        return "";
    }
    list.get(index)
        .map(String::as_str)
        .unwrap_or(list[list.len() - 1].as_str())
}

fn parse_css_time_token(trimmed: &str) -> Option<f32> {
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(value) = trimmed.strip_suffix("ms") {
        return value.trim().parse::<f32>().ok().map(|v| v.max(0.0));
    }
    if let Some(value) = trimmed.strip_suffix('s') {
        return value
            .trim()
            .parse::<f32>()
            .ok()
            .map(|v| (v * 1000.0).max(0.0));
    }
    None
}

pub fn parse_css_time_ms(raw: &str) -> Option<f32> {
    let parts = split_css_comma_list(raw);
    if parts.is_empty() {
        return parse_css_time_token(raw);
    }
    parts
        .iter()
        .filter_map(|part| parse_css_time_token(part))
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
}

/// CSS `transition-timing-function` / `animation-timing-function`.
///
/// Named keywords and `cubic-bezier()` become [`Easing`]. `steps()` lives on
/// [`MotionCurve`]; use [`curve_from_css`] when compiling a track.
pub fn easing_from_css(name: &str) -> Easing {
    match curve_from_css(name) {
        MotionCurve::Easing(easing) => easing,
        MotionCurve::Steps { .. } | MotionCurve::Spring(_) | MotionCurve::Decay(_) => {
            Easing::Linear
        }
    }
}

/// Full CSS timing function, including `steps()` / `step-start` / `step-end`.
pub fn curve_from_css(name: &str) -> MotionCurve {
    let token = first_timing_token(name);
    let lower = token.to_ascii_lowercase();
    if let Some(steps) = parse_css_steps(&lower) {
        return MotionCurve::Steps {
            count: steps.0,
            jump: steps.1,
        };
    }
    MotionCurve::Easing(easing_from_css_keyword(&lower))
}

fn first_timing_token(raw: &str) -> String {
    split_css_comma_list(raw)
        .into_iter()
        .next()
        .unwrap_or_else(|| raw.trim().to_string())
}

fn easing_from_css_keyword(name: &str) -> Easing {
    match name {
        "linear" => Easing::Linear,
        "ease" => Easing::CubicBezier([0.25, 0.1, 0.25, 1.0]),
        "ease-in" => Easing::CubicBezier([0.42, 0.0, 1.0, 1.0]),
        "ease-out" => Easing::CubicBezier([0.0, 0.0, 0.58, 1.0]),
        "ease-in-out-cubic" => Easing::EaseInOutCubic,
        "ease-in-out" => Easing::CubicBezier([0.42, 0.0, 0.58, 1.0]),
        "ease-out-cubic" => Easing::EaseOutCubic,
        other => parse_cubic_bezier(other)
            .map(Easing::CubicBezier)
            .unwrap_or(Easing::EaseOutCubic),
    }
}

fn parse_cubic_bezier(name: &str) -> Option<[f32; 4]> {
    let inner = name
        .strip_prefix("cubic-bezier(")?
        .trim_end_matches(')')
        .trim();
    let mut values = [0.0f32; 4];
    let mut count = 0usize;
    for part in inner.split(',') {
        let value: f32 = part.trim().parse().ok()?;
        if count >= 4 {
            return None;
        }
        if matches!(count, 0 | 2) && !(0.0..=1.0).contains(&value) {
            return None;
        }
        values[count] = value;
        count += 1;
    }
    (count == 4).then_some(values)
}

fn parse_css_steps(name: &str) -> Option<(u32, StepJump)> {
    match name {
        "step-start" => return Some((1, StepJump::Start)),
        "step-end" => return Some((1, StepJump::End)),
        _ => {}
    }
    let inner = name.strip_prefix("steps(")?.trim_end_matches(')').trim();
    let mut parts = inner.split(',').map(str::trim);
    let count = parts
        .next()?
        .parse::<u32>()
        .ok()
        .filter(|count| *count > 0)?;
    let jump = match parts.next().unwrap_or("end") {
        "start" | "jump-start" => StepJump::Start,
        "end" | "jump-end" => StepJump::End,
        "jump-none" => StepJump::None,
        "jump-both" => StepJump::Both,
        _ => return None,
    };
    Some((count, jump))
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
    let _applies_edge = |prefix: &str, side: &str| applies(prefix) || applies(side);
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
            for (i, value) in m.iter_mut().enumerate() {
                *value = a.m[i] + (b.m[i] - a.m[i]) * t;
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

/// Compile `transition` longhands onto Motion IR. Compositor-safe properties
/// become overlay tracks; px width/height become Layout-class CPU tracks;
/// remaining paint longhands keep a Progress spec.
pub fn compile_css_transition(
    widget_id: u64,
    motion: &CssComputedMotion,
    from: &CssPaintSnapshot,
    to: &CssPaintSnapshot,
    now: Duration,
) -> Option<CompiledCssMotion> {
    if !motion.has_transition() {
        return None;
    }
    let listed = parse_transition_properties(&motion.transition_property);
    let durations = split_css_comma_list(&motion.transition_duration);
    let delays = split_css_comma_list(&motion.transition_delay);
    let timings = split_css_comma_list(&motion.transition_timing_function);
    let all = listed.is_empty()
        || listed
            .iter()
            .any(|property| property.eq_ignore_ascii_case("all"));
    let mut overlays = Vec::new();
    let mut cpu_properties = Vec::new();
    let mut cpu_index = 0usize;
    let properties: Vec<String> = if all {
        compositor_css_names()
            .into_iter()
            .chain(cpu_css_names())
            .map(str::to_string)
            .collect()
    } else {
        listed
    };
    for (index, property) in properties.iter().enumerate() {
        let duration = css_list_at(&durations, index);
        let delay = css_list_at(&delays, index);
        let timing = css_list_at(&timings, index);
        if let Some(animatable) = compositor_css_property(property) {
            if !snapshot_property_changed(from, to, animatable) {
                continue;
            }
            let Some(spec) = overlay_transition_spec(
                widget_id, animatable, from, to, duration, delay, timing, now,
            ) else {
                continue;
            };
            overlays.push(spec);
            continue;
        }
        if let Some(animatable) = layout_css_property(property) {
            // Deliberately not an early `continue` on "unchanged": that test
            // reads the value through `length_px_value`, which is `None` for
            // every non-px length, so `0%` and `60%` compare equal. Skipping
            // here would keep the property out of `cpu_properties` as well, and
            // a percentage width would snap instead of animating.
            if snapshot_property_changed(from, to, animatable)
                && let Some(spec) = overlay_transition_spec(
                    widget_id, animatable, from, to, duration, delay, timing, now,
                )
            {
                overlays.push(spec);
                continue;
            }
        }
        if snapshot_cpu_property_changed(from, to, property) {
            cpu_properties.push(property.clone());
            cpu_index = index;
        }
    }
    let cpu = if cpu_properties.is_empty() {
        None
    } else {
        timing_spec(
            css_transition_animation_id(widget_id),
            widget_id,
            css_list_at(&durations, cpu_index),
            css_list_at(&delays, cpu_index),
            css_list_at(&timings, cpu_index),
            now,
            None,
        )
    };
    let compiled = CompiledCssMotion { cpu, overlays };
    (!compiled.is_empty()).then_some(compiled)
}

/// Compile `@keyframes` onto Motion IR. Opacity/transform become compositor
/// overlay keyframe tracks; remaining paint/layout longhands keep a Progress spec.
pub fn compile_css_keyframes(
    widget_id: u64,
    motion: &CssComputedMotion,
    rule: &KeyframesRule,
    now: Duration,
) -> Option<CompiledCssMotion> {
    if motion.animation_name.eq_ignore_ascii_case("none") {
        return None;
    }
    let playback = playback_from_computed(motion);
    let mut overlays = Vec::new();
    let mut has_cpu = false;
    for name in keyframe_declared_names(rule) {
        if let Some(property) = compositor_css_property(&name) {
            if let Some(spec) = overlay_keyframe_spec(widget_id, property, rule, motion, now) {
                overlays.push(spec);
            }
            continue;
        }
        if cpu_css_names()
            .iter()
            .any(|cpu| cpu.eq_ignore_ascii_case(&name))
            || name.eq_ignore_ascii_case("transform-origin")
            || name.eq_ignore_ascii_case("background-color")
        {
            has_cpu = true;
        }
    }
    let cpu = has_cpu
        .then(|| {
            timing_spec(
                css_keyframes_animation_id(widget_id),
                widget_id,
                &motion.animation_duration,
                &motion.animation_delay,
                &motion.animation_timing_function,
                now,
                Some(playback),
            )
        })
        .flatten();
    let compiled = CompiledCssMotion { cpu, overlays };
    if compiled.is_empty() {
        timing_spec(
            css_keyframes_animation_id(widget_id),
            widget_id,
            &motion.animation_duration,
            &motion.animation_delay,
            &motion.animation_timing_function,
            now,
            Some(playback),
        )
        .map(|cpu| CompiledCssMotion {
            cpu: Some(cpu),
            overlays: Vec::new(),
        })
    } else {
        Some(compiled)
    }
}

fn timing_spec(
    id: AnimationId,
    widget_id: u64,
    duration_raw: &str,
    delay_raw: &str,
    timing_raw: &str,
    now: Duration,
    playback: Option<AnimationPlayback>,
) -> Option<AnimationSpec> {
    let duration_ms = parse_css_time_ms(duration_raw).unwrap_or(0.0);
    if duration_ms <= 0.0 {
        return None;
    }
    let delay_ms = parse_css_time_ms(delay_raw).unwrap_or(0.0);
    let duration = Duration::from_secs_f32(duration_ms / 1000.0);
    let delay = Duration::from_secs_f32(delay_ms / 1000.0);
    let start = now.checked_add(delay)?;
    let mut spec = AnimationSpec::new(
        id,
        StableNodeId::new(widget_id)?,
        start,
        duration,
        Duration::from_millis(16),
        easing_from_css(timing_raw),
    )
    .with_curve(curve_from_css(timing_raw));
    if let Some(playback) = playback {
        spec = spec.with_playback(playback);
    }
    Some(spec)
}

fn overlay_transition_spec(
    widget_id: u64,
    property: AnimatableProperty,
    from: &CssPaintSnapshot,
    to: &CssPaintSnapshot,
    duration_raw: &str,
    delay_raw: &str,
    timing_raw: &str,
    now: Duration,
) -> Option<AnimationSpec> {
    let from_value = snapshot_motion_value(from, property)?;
    let to_value = snapshot_motion_value(to, property)?;
    Some(
        timing_spec(
            css_transition_overlay_id(widget_id, property),
            widget_id,
            duration_raw,
            delay_raw,
            timing_raw,
            now,
            None,
        )?
        .with_property(property)
        .with_range(from_value, MotionTo::Value(to_value)),
    )
}

fn overlay_keyframe_spec(
    widget_id: u64,
    property: AnimatableProperty,
    rule: &KeyframesRule,
    motion: &CssComputedMotion,
    now: Duration,
) -> Option<AnimationSpec> {
    let (from, to) = motion_keyframes_for_property(rule, property)?;
    Some(
        timing_spec(
            css_keyframes_overlay_id(widget_id, property),
            widget_id,
            &motion.animation_duration,
            &motion.animation_delay,
            &motion.animation_timing_function,
            now,
            Some(playback_from_computed(motion)),
        )?
        .with_property(property)
        .with_range(from, to),
    )
}

fn compositor_css_property(name: &str) -> Option<AnimatableProperty> {
    let (property, class) = classify_animatable_property(name)?;
    (class == AnimationClass::Compositor
        && matches!(
            property,
            AnimatableProperty::Opacity | AnimatableProperty::Transform | AnimatableProperty::Clip
        ))
    .then_some(property)
}

fn layout_css_property(name: &str) -> Option<AnimatableProperty> {
    let (property, class) = classify_animatable_property(name)?;
    (class == AnimationClass::Layout
        && matches!(
            property,
            AnimatableProperty::Width | AnimatableProperty::Height
        ))
    .then_some(property)
}

fn compositor_css_names() -> [&'static str; 2] {
    ["opacity", "transform"]
}

fn cpu_css_names() -> [&'static str; 6] {
    [
        "color",
        "background",
        "filter",
        "width",
        "height",
        "transform-origin",
    ]
}

pub fn cpu_transition_properties(listed: &[String]) -> Vec<String> {
    let all = listed.is_empty()
        || listed
            .iter()
            .any(|property| property.eq_ignore_ascii_case("all"));
    if all {
        return cpu_css_names()
            .iter()
            .map(|name| (*name).to_string())
            .collect();
    }
    listed
        .iter()
        .filter(|property| compositor_css_property(property).is_none())
        .cloned()
        .collect()
}

fn snapshot_motion_value(
    snapshot: &CssPaintSnapshot,
    property: AnimatableProperty,
) -> Option<MotionValue> {
    match property {
        AnimatableProperty::Opacity => Some(MotionValue::Scalar(snapshot.opacity.unwrap_or(1.0))),
        AnimatableProperty::Transform => Some(MotionValue::Transform(
            snapshot.transform.unwrap_or_default(),
        )),
        AnimatableProperty::Width => length_px_value(snapshot.width),
        AnimatableProperty::Height => length_px_value(snapshot.height),
        _ => None,
    }
}

fn length_px_value(spec: Option<nana_ui_core::LengthSpec>) -> Option<MotionValue> {
    match spec {
        Some(nana_ui_core::LengthSpec::Px(px)) if px.is_finite() => Some(MotionValue::Scalar(px)),
        _ => None,
    }
}

fn snapshot_property_changed(
    from: &CssPaintSnapshot,
    to: &CssPaintSnapshot,
    property: AnimatableProperty,
) -> bool {
    snapshot_motion_value(from, property) != snapshot_motion_value(to, property)
}

fn snapshot_cpu_property_changed(
    from: &CssPaintSnapshot,
    to: &CssPaintSnapshot,
    property: &str,
) -> bool {
    match property.to_ascii_lowercase().as_str() {
        "color" => from.color != to.color,
        "background" | "background-color" => from.background != to.background,
        "filter" => from.filter != to.filter,
        "width" => from.width != to.width,
        "height" => from.height != to.height,
        "transform-origin" => from.transform_origin != to.transform_origin,
        "transform" => from.transform_3d != to.transform_3d,
        _ => false,
    }
}

fn keyframe_declared_names(rule: &KeyframesRule) -> Vec<String> {
    let mut names = Vec::new();
    for block in &rule.blocks {
        for entry in &block.declaration_entries {
            if !names
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&entry.property))
            {
                names.push(entry.property.clone());
            }
        }
    }
    names
}

fn motion_keyframes_for_property(
    rule: &KeyframesRule,
    property: AnimatableProperty,
) -> Option<(MotionValue, MotionTo)> {
    let mut stops: Vec<(f32, CssPaintSnapshot)> = rule
        .blocks
        .iter()
        .filter_map(|block| {
            let offset = block
                .selectors
                .iter()
                .map(|sel| match sel {
                    KeyframeSelector::From => 0.0,
                    KeyframeSelector::To => 100.0,
                    KeyframeSelector::Percent(p) => *p,
                })
                .fold(f32::INFINITY, f32::min);
            (offset < f32::INFINITY)
                .then_some((offset, paint_from_entries(&block.declaration_entries)))
        })
        .collect();
    stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut values = Vec::new();
    for (percent, paint) in &stops {
        let Some(value) = snapshot_motion_value(paint, property) else {
            continue;
        };
        if property == AnimatableProperty::Opacity && paint.opacity.is_none() {
            continue;
        }
        if property == AnimatableProperty::Transform && paint.transform.is_none() {
            continue;
        }
        values.push(Keyframe {
            offset: (percent / 100.0).clamp(0.0, 1.0),
            value,
            easing: None,
        });
    }
    if values.is_empty() {
        return None;
    }
    let from = values[0].value;
    Some((from, MotionTo::Keyframes(values)))
}

pub fn playback_from_computed(motion: &CssComputedMotion) -> AnimationPlayback {
    AnimationPlayback {
        iteration_count: parse_animation_iteration(&motion.animation_iteration_count),
        direction: parse_animation_direction(&motion.animation_direction),
        fill_mode: parse_animation_fill_mode(&motion.animation_fill_mode),
        play_state: parse_animation_play_state(&motion.animation_play_state),
        paused_at: None,
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
    use crate::{
        css_cascade::{MatchContext, MatchNode, parse_stylesheet_full},
        css_interactive::{InteractiveMatchState, InteractivePseudoFlags},
    };

    #[test]
    fn transition_shorthand_parses_duration() {
        let parsed = parse_transition_shorthand("opacity 0.2s ease").expect("transition");
        assert_eq!(parsed.property, "opacity");
        assert_eq!(parsed.duration, "0.2s");
        assert_eq!(parse_css_time_ms(&parsed.duration), Some(200.0));
    }

    #[test]
    fn transform_opacity_shorthand_keeps_both_properties() {
        let parsed = parse_transition_shorthand("transform 150ms ease, opacity 150ms ease")
            .expect("transition");
        assert_eq!(parsed.property, "transform, opacity");
        assert_eq!(parsed.duration, "150ms, 150ms");
        assert_eq!(parse_css_time_ms(&parsed.duration), Some(150.0));
    }

    #[test]
    fn easing_from_css_parses_cubic_bezier_and_keywords() {
        assert_eq!(easing_from_css("linear"), Easing::Linear);
        assert_eq!(
            easing_from_css("cubic-bezier(0.2, 0.8, 0.2, 1)"),
            Easing::CubicBezier([0.2, 0.8, 0.2, 1.0])
        );
        assert_eq!(
            easing_from_css("ease"),
            Easing::CubicBezier([0.25, 0.1, 0.25, 1.0])
        );
    }

    #[test]
    fn curve_from_css_parses_steps() {
        assert_eq!(
            curve_from_css("steps(4, end)"),
            MotionCurve::Steps {
                count: 4,
                jump: StepJump::End
            }
        );
        assert_eq!(
            curve_from_css("step-start"),
            MotionCurve::Steps {
                count: 1,
                jump: StepJump::Start
            }
        );
        assert_eq!(
            curve_from_css("steps(2, jump-both)"),
            MotionCurve::Steps {
                count: 2,
                jump: StepJump::Both
            }
        );
    }

    #[test]
    fn apply_cpu_to_layout_skips_compositor_opacity() {
        let paint = CssPaintSnapshot {
            opacity: Some(1.0),
            background: Some([0.0, 0.0, 1.0, 1.0]),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let mut cpu = LayoutStyle {
            opacity: Some(0.0),
            ..LayoutStyle::default()
        };
        paint.apply_cpu_to_layout(&mut cpu);
        assert_eq!(cpu.opacity, Some(0.0));
        assert_eq!(cpu.background, Some([0.0, 0.0, 1.0, 1.0]));

        let mut all = LayoutStyle {
            opacity: Some(0.0),
            ..LayoutStyle::default()
        };
        paint.apply_to_layout(&mut all);
        assert_eq!(all.opacity, Some(1.0));
    }

    #[test]
    fn compile_transition_emits_compositor_overlay_not_progress() {
        let motion = CssComputedMotion {
            transition_property: "transform, opacity".into(),
            transition_duration: "150ms".into(),
            transition_delay: "0s".into(),
            transition_timing_function: "ease".into(),
            ..CssComputedMotion::default()
        };
        let from = CssPaintSnapshot {
            opacity: Some(0.0),
            transform: Some(nana_ui_core::PaintTransform {
                e: 12.0,
                ..nana_ui_core::PaintTransform::default()
            }),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let to = CssPaintSnapshot {
            opacity: Some(1.0),
            transform: Some(nana_ui_core::PaintTransform::default()),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let compiled =
            compile_css_transition(7, &motion, &from, &to, Duration::ZERO).expect("compiled");
        assert!(compiled.cpu.is_none());
        assert_eq!(compiled.overlays.len(), 2);
        assert!(
            compiled
                .overlays
                .iter()
                .all(|spec| spec.uses_presentation_overlay())
        );
        assert!(
            compiled
                .overlays
                .iter()
                .any(|spec| spec.property == AnimatableProperty::Opacity)
        );
        assert!(
            compiled
                .overlays
                .iter()
                .any(|spec| spec.property == AnimatableProperty::Transform)
        );
    }

    #[test]
    fn compile_width_transition_stays_layout_class() {
        use nana_ui_core::LengthSpec;
        let motion = CssComputedMotion {
            transition_property: "width".into(),
            transition_duration: "200ms".into(),
            transition_delay: "0s".into(),
            transition_timing_function: "linear".into(),
            ..CssComputedMotion::default()
        };
        let from = CssPaintSnapshot {
            width: Some(LengthSpec::Px(40.0)),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let to = CssPaintSnapshot {
            width: Some(LengthSpec::Px(80.0)),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let compiled =
            compile_css_transition(3, &motion, &from, &to, Duration::ZERO).expect("compiled");
        assert!(compiled.cpu.is_none());
        assert_eq!(compiled.overlays.len(), 1);
        let layout = &compiled.overlays[0];
        assert!(!layout.uses_presentation_overlay());
        assert_eq!(layout.property, AnimatableProperty::Width);
        assert_eq!(layout.from, MotionValue::Scalar(40.0));
        assert_eq!(layout.to, MotionTo::Value(MotionValue::Scalar(80.0)));
    }

    /// A percentage width has no px value, so the overlay path cannot carry it
    /// and `snapshot_property_changed` cannot even see the change. It has to
    /// fall through to the CPU spec rather than being dropped, or the box snaps.
    #[test]
    fn a_percentage_width_transition_falls_through_to_the_cpu_spec() {
        use nana_ui_core::LengthSpec;
        let motion = CssComputedMotion {
            transition_property: "width".into(),
            transition_duration: "200ms".into(),
            transition_delay: "0s".into(),
            transition_timing_function: "linear".into(),
            ..CssComputedMotion::default()
        };
        let from = CssPaintSnapshot {
            width: Some(LengthSpec::Percent(0.0)),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let to = CssPaintSnapshot {
            width: Some(LengthSpec::Percent(60.0)),
            ..CssPaintSnapshot::from_layout(&LayoutStyle::default())
        };
        let compiled =
            compile_css_transition(11, &motion, &from, &to, Duration::ZERO).expect("compiled");
        assert!(compiled.overlays.is_empty());
        assert!(
            compiled.cpu.is_some(),
            "percentage width must still animate"
        );
    }

    /// `transition-duration` defaults to `0s` per item. An item that omits it
    /// is legal CSS and must not take the rest of the list down with it.
    #[test]
    fn an_item_without_a_duration_keeps_the_rest_of_the_list() {
        let parsed = parse_transition_shorthand("opacity, transform 200ms").expect("transition");
        assert_eq!(parsed.property, "opacity, transform");
        assert_eq!(parsed.duration, "0s, 200ms");
        assert_eq!(parsed.timing_function, "ease, ease");
        assert_eq!(parsed.delay, "0s, 0s");
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
