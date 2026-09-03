//! nanavue → NanaUI L1/L2 semantic projection (not a retained world).
//!
//! ## L2 边界
//! - 本模块保存 compatibility semantic props：`WidgetKind` / `WidgetProps` /
//!   [`SemanticSnapshot`]。
//! - **kind 解析**集中在 [`crate::widget_map::resolve_kind_from_hints`]（本文件
//!   `pub use` 转发）；勿在本模块再维护第二份 class/role 表。
//! - Layout 声明解析属 L1（`css_map` / cascade）；本模块只存储与触发 rebuild。
//! - Hierarchy on [`SemanticWidget`] is a cascade/projection index. Observable
//!   parent/children/roots are overwritten from `UiWorld` by
//!   [`crate::NanaTreeDocument::apply_runtime_hierarchy`] before Scene paint.
//!
//! Vue Custom Renderer hostOps project every visible node onto Nana layout
//! primitives + base controls, then draw through Runtime / UiScene. This is
//! not a second ECS tree.
//!
//! Vue "custom components" are combinations and variants of those foundations —
//! not a separate CPU paint channel. CustomContent has been removed.

mod cascade;
mod motion;
mod resources;

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nana_ui_core::{
    AppearanceSettings, BackdropTarget, ButtonKind, CardKind, ControlSize, Icon,
    SwitchControlPosition, ThemeMode, WindowMaterialMode,
};

use crate::css_at_rule::{
    FontFaceRule, FontFaceSrc, FsStylesheetLoader, MediaEnvironment, ParseStylesheetOptions,
    evaluate_media_query_list, font_registration_would_exceed_cap, load_font_face_bytes,
    parse_media_query_list,
};
use crate::css_cascade::{
    MatchContext, MatchNode, RelativeMatchForest, RelativeMatchNode, SimpleCompound, StyleRule,
    StylesheetParseReport, collect_document_custom_properties_from_rules,
    parse_stylesheet_full_with_options, rebuild_layout_style_indexed, simple_matches,
    stylesheet_matches, stylesheet_may_match_subject, stylesheet_needs_relative,
};
use crate::css_interactive::{
    GeneratedPseudo, GeneratedPseudoRule, InteractiveMatchState, InteractiveStyleRule,
    KeyframesRule, MotionStyleRule, ParsedStylesheet, ScrollbarPseudoRule, merge_parsed_stylesheet,
};
use crate::css_interactive_apply::{
    ActiveCssTransition, CssComputedMotion, CssMotionComplete, CssPaintSnapshot,
    InteractiveRuntimeSnapshot, animation_elapsed_secs, apply_generated_pseudo_entries,
    apply_interactive_layers, apply_placeholder_paint, apply_scrollbar_pseudo_skin,
    build_keyframes_spec, build_transition_spec, css_keyframes_animation_id,
    generated_pseudo_has_content, keyframe_paint_at, lerp_paint_for_properties, parse_content_text,
    parse_transition_properties, resolve_computed_motion, transition_elapsed_secs,
};
use crate::css_map::{
    FlexDirection, GridTrack, LayoutStyle, LayoutStyleCss, LengthSpec, ParentBox,
};
use crate::layout_map::default_layout_for_kind;
use crate::tree::NodeHandle;
pub use crate::widget_map::resolve_kind_from_hints;

mod semantic;
pub use semantic::{
    BridgeEvent, SelectOptionProp, SemanticRegionViews, SemanticSnapshot, SemanticWidget,
    SnapshotChanges, WidgetId, WidgetKind, WidgetProps,
};

/// Attribute marking bridge-owned `::before` / `::after` boxes.
pub const GENERATED_PSEUDO_ATTR: &str = "data-nana-generated-pseudo";
/// Originating element id for generated pseudo widgets.
pub const GENERATED_PSEUDO_ORIGIN_ATTR: &str = "data-nana-generated-origin";

const GENERATED_PSEUDO_ID_BASE: u64 = 0xA000_0000_0000_0000;

/// Pending [`BridgeEvent`]s are Runtime-bound input, not a second event tree.
#[derive(Debug)]
pub struct MessageBridge {
    cascade: cascade::State,
    resources: resources::State,
    motion: motion::State,
    widgets: HashMap<WidgetId, SemanticWidget>,
    roots: Vec<WidgetId>,
    pending: VecDeque<BridgeEvent>,
    revision: u64,
    /// Mutation footprint since the previous snapshot (incremental sync).
    changes: SnapshotChanges,
    theme: ThemeMode,
    appearance: AppearanceSettings,
    /// When true, html/body scaffold owns roots — createElement must not promote.
    scaffolded: bool,
    next_generated_pseudo_id: u64,
    /// Originating widget → generated pseudo child ids.
    generated_pseudo_children: HashMap<WidgetId, GeneratedPseudoChildren>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct GeneratedPseudoChildren {
    before: Option<WidgetId>,
    after: Option<WidgetId>,
}

fn stylesheet_base_is_set(base: &Path) -> bool {
    !base.as_os_str().is_empty() && base != Path::new(".")
}

fn host_alias_local_font_face(
    css_family: &str,
    local_name: &str,
    weight: Option<u16>,
    weight_end: Option<u16>,
) -> bool {
    #[cfg(feature = "scene-view")]
    {
        nana_ui::alias_host_font_face_local(css_family, local_name, weight, weight_end) > 0
    }
    #[cfg(not(feature = "scene-view"))]
    {
        let _ = (css_family, local_name, weight, weight_end);
        false
    }
}

fn font_face_register_key(src: String, face: &FontFaceRule) -> (String, String, u16, u16) {
    let (lo, hi) = face.weight_span().unwrap_or((400, 400));
    (src, face.family.clone(), lo, hi)
}

impl Default for MessageBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl MessageBridge {
    pub fn new() -> Self {
        Self {
            widgets: HashMap::new(),
            roots: Vec::new(),
            pending: VecDeque::new(),
            revision: 0,
            changes: SnapshotChanges::default(),
            theme: ThemeMode::Light,
            appearance: AppearanceSettings::default(),
            scaffolded: false,
            cascade: cascade::State::default(),
            resources: resources::State::default(),
            motion: motion::State::default(),
            next_generated_pseudo_id: 1,
            generated_pseudo_children: HashMap::new(),
        }
    }

    pub(crate) fn interactive_ancestor_flags(
        &self,
        id: WidgetId,
    ) -> Vec<crate::css_interactive::InteractivePseudoFlags> {
        use crate::css_interactive::InteractivePseudoFlags;
        let Some(runtime) = &self.cascade.interactive_runtime else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut cur = self.widgets.get(&id).and_then(|w| w.parent);
        while let Some(pid) = cur {
            out.push(InteractivePseudoFlags {
                hover: runtime.hovered.contains_key(&pid),
                focus: runtime.focused == Some(pid),
                active: runtime.pressed.contains_key(&pid),
            });
            cur = self.widgets.get(&pid).and_then(|w| w.parent);
        }
        out
    }

    /// Record Vue `setScopeId` attribute for scoped selector matching.
    pub fn set_scope_attr(&mut self, id: WidgetId, scope: &str) {
        let Some(widget) = self.widgets.get_mut(&id) else {
            return;
        };
        let name = if scope.starts_with("data-") {
            scope.to_string()
        } else {
            format!("data-v-{scope}")
        };
        widget.props.attrs.insert(name, String::new());
        self.reapply_layout_for(id);
        self.changed_widget(id);
    }

    fn sync_widget_layouts_for(&self, doc: &mut crate::tree::NanaTreeDocument, ids: &[WidgetId]) {
        // Incremental: only widgets whose interpolated LayoutStyle changed.
        // Never writes Runtime LayoutBox (scroll authority stays on the engine).
        doc.sync_widget_layouts(ids.iter().filter_map(|id| {
            self.widgets
                .get(id)
                .map(|widget| (*id, &widget.props.layout))
        }));
    }

    fn collect_interactive_runtime_snapshot(
        doc: &crate::tree::NanaTreeDocument,
    ) -> InteractiveRuntimeSnapshot {
        let document =
            nana_ui_runtime::DocumentId::try_from(doc.id()).expect("vue document IDs are nonzero");
        let world = doc.world();
        let mut hovered = BTreeMap::new();
        let mut pressed = BTreeMap::new();
        for pointer_id in 0u64..16 {
            if let Some(target) = world.pointer_hover(document, pointer_id) {
                hovered.insert(target.get(), ());
            }
            if let Some(target) = world.pointer_press(document, pointer_id) {
                pressed.insert(target.get(), ());
            }
        }
        InteractiveRuntimeSnapshot {
            hovered,
            pressed,
            focused: doc.focused().map(|h| h.0),
        }
    }

    fn is_generated_pseudo_widget(&self, id: WidgetId) -> bool {
        self.widgets
            .get(&id)
            .is_some_and(|w| w.props.attrs.contains_key(GENERATED_PSEUDO_ATTR))
    }

    fn widget_is_dom_text(&self, id: WidgetId) -> bool {
        self.widgets.get(&id).is_some_and(|w| {
            crate::widget_map::is_dom_text_node(w.kind, w.props.element_tag.as_str())
        })
    }

    fn is_element_sibling(&self, id: WidgetId) -> bool {
        !self.is_generated_pseudo_widget(id) && !self.widget_is_dom_text(id)
    }

    fn sync_generated_pseudo_for(
        &mut self,
        origin: WidgetId,
        doc: &mut crate::tree::NanaTreeDocument,
    ) {
        if self.cascade.generated_pseudo_rules.is_empty() {
            return;
        }
        let matched = {
            let Some(ancestry) = self.match_ancestry(origin) else {
                return;
            };
            let is_empty = self.widget_is_empty(origin);
            let Some(widget) = self.widgets.get(&origin) else {
                return;
            };
            let leaf_classes = widget.props.class_names.clone();
            let leaf_attrs = cascade_attrs_from_widget(widget);
            let leaf_tag = if widget.props.element_tag.is_empty() {
                widget.kind.element_tag().to_string()
            } else {
                widget.props.element_tag.clone()
            };
            let leaf_id = widget.props.element_id.clone();
            let (sibling_index, sibling_count) = self.sibling_position(origin);
            let (of_type_index, of_type_count) = self.of_type_position(origin);
            let prev_snaps = self.prev_sibling_snaps(origin);
            let ancestor_nodes: Vec<MatchNode<'_>> =
                ancestry.iter().skip(1).map(|n| n.as_node()).collect();
            let prev_nodes: Vec<MatchNode<'_>> = prev_snaps.iter().map(|n| n.as_node()).collect();
            let ctx = MatchContext {
                tag: leaf_tag.as_str(),
                id: leaf_id.as_str(),
                classes: leaf_classes.as_slice(),
                attrs: &leaf_attrs,
                ancestors: ancestor_nodes.as_slice(),
                preceding_siblings: prev_nodes.as_slice(),
                sibling_index,
                sibling_count,
                of_type_index,
                of_type_count,
                has_bits: self
                    .cascade
                    .has_descendant_bits
                    .get(&origin)
                    .copied()
                    .unwrap_or(0),
                has_args: self.cascade.has_args.as_slice(),
                focus_within: self.focus_within_of(origin),
                is_empty,
                checked: widget_checked_state(widget),
                media: self.media_env(),
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
            crate::css_interactive::matched_generated_pseudo(
                &self.cascade.generated_pseudo_rules,
                &ctx,
            )
        };
        let slots = *self.generated_pseudo_children.entry(origin).or_default();
        let mut next = slots;
        for (pseudo, blocks, slot) in [
            (
                GeneratedPseudo::Before,
                matched.before.clone(),
                &mut next.before,
            ),
            (
                GeneratedPseudo::After,
                matched.after.clone(),
                &mut next.after,
            ),
        ] {
            if blocks.is_empty()
                || !blocks
                    .iter()
                    .any(|entries| generated_pseudo_has_content(entries))
            {
                if let Some(child) = slot.take() {
                    self.remove_generated_pseudo_widget(child, doc);
                }
                continue;
            }
            let child = slot.get_or_insert_with(|| self.alloc_generated_pseudo_id());
            self.ensure_generated_pseudo_widget(origin, *child, pseudo, &blocks);
            doc.ensure_css_pseudo_element(
                *child,
                crate::tree::NodeHandle(origin),
                pseudo,
                pseudo == GeneratedPseudo::Before,
            );
            self.insert_generated_pseudo_child(origin, *child, pseudo);
        }
        if next.before.is_none() && next.after.is_none() {
            self.generated_pseudo_children.remove(&origin);
        } else {
            self.generated_pseudo_children.insert(origin, next);
        }
    }

    fn alloc_generated_pseudo_id(&mut self) -> WidgetId {
        let id = GENERATED_PSEUDO_ID_BASE | self.next_generated_pseudo_id;
        self.next_generated_pseudo_id = self.next_generated_pseudo_id.saturating_add(1);
        id
    }

    fn ensure_generated_pseudo_widget(
        &mut self,
        origin: WidgetId,
        child: WidgetId,
        _pseudo: GeneratedPseudo,
        blocks: &[Vec<crate::css_cascade::DeclarationEntry>],
    ) {
        let pseudo_name = match _pseudo {
            GeneratedPseudo::Before => "before",
            GeneratedPseudo::After => "after",
            // Paint-only; this helper is only called for before/after boxes.
            GeneratedPseudo::Placeholder => "placeholder",
        };
        let text = blocks
            .iter()
            .find_map(|entries| parse_content_text(entries))
            .filter(|text| !text.is_empty());
        if !self.widgets.contains_key(&child) {
            let mut attrs = BTreeMap::new();
            attrs.insert(GENERATED_PSEUDO_ATTR.into(), pseudo_name.into());
            attrs.insert(GENERATED_PSEUDO_ORIGIN_ATTR.into(), origin.to_string());
            let props = WidgetProps {
                element_tag: if text.is_some() {
                    "span".into()
                } else {
                    "div".into()
                },
                attrs,
                ..WidgetProps::default()
            };
            self.register(child, WidgetKind::Box, props);
        } else if let Some(widget) = self.widgets.get_mut(&child) {
            widget
                .props
                .attrs
                .insert(GENERATED_PSEUDO_ATTR.into(), pseudo_name.into());
            widget
                .props
                .attrs
                .insert(GENERATED_PSEUDO_ORIGIN_ATTR.into(), origin.to_string());
        }
        let (cb_w, cb_h) = self
            .widgets
            .get(&origin)
            .map(|w| {
                (
                    w.props.containing_block_width,
                    w.props.containing_block_height,
                )
            })
            .unwrap_or((None, None));
        if let Some(widget) = self.widgets.get_mut(&child) {
            let mut layout = LayoutStyle::default();
            apply_generated_pseudo_entries(&mut layout, blocks, cb_w, cb_h);
            if let Some(label) = text {
                widget.kind = WidgetKind::Text;
                widget.props.label = label;
            }
            widget.props.layout = layout;
        }
    }

    fn insert_generated_pseudo_child(
        &mut self,
        origin: WidgetId,
        child: WidgetId,
        pseudo: GeneratedPseudo,
    ) {
        let anchor = match pseudo {
            GeneratedPseudo::Before => self
                .widgets
                .get(&origin)
                .and_then(|w| w.children.first().copied())
                .filter(|id| *id != child),
            GeneratedPseudo::After | GeneratedPseudo::Placeholder => None,
        };
        self.insert_child(child, origin, anchor);
    }

    fn remove_generated_pseudo_widget(
        &mut self,
        child: WidgetId,
        doc: &mut crate::tree::NanaTreeDocument,
    ) {
        doc.remove_generated_pseudo(crate::tree::NodeHandle(child));
        self.unregister(child);
        self.motion.computed_motion.remove(&child);
        self.motion.css_transitions.remove(&child);
        self.motion.css_transition_base.remove(&child);
        self.motion.css_transition_progress.remove(&child);
    }

    fn interactive_motion_for<'a>(
        &'a self,
        ctx: &MatchContext<'_>,
        runtime: &InteractiveRuntimeSnapshot,
        id: WidgetId,
    ) -> Option<&'a crate::css_interactive::MotionDeclarations> {
        let subject = runtime.subject_flags(id);
        let ancestors = runtime.ancestor_flags(self, id);
        let istate = InteractiveMatchState {
            subject,
            ancestors: &ancestors,
        };
        self.cascade
            .interactive_rules
            .iter()
            .filter(|rule| {
                crate::css_interactive::interactive_selector_matches(&rule.selector, ctx, &istate)
            })
            .map(|rule| &rule.motion)
            .find(|motion| !motion.is_empty())
    }

    fn apply_css_animation_samples_inner(
        &mut self,
        frame: nana_ui_runtime::AnimationFrame,
    ) -> Vec<WidgetId> {
        let mut changed_ids = Vec::new();
        for sample in frame.samples {
            let id = sample.target.get();
            if let Some(transition) = self.motion.css_transitions.get(&id).cloned()
                && sample.id == transition.spec.id
            {
                let base = self
                    .motion
                    .css_transition_base
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| transition.from.clone());
                let motion = self.motion.computed_motion.get(&id).cloned();
                let properties = motion
                    .as_ref()
                    .map(|motion| parse_transition_properties(&motion.transition_property))
                    .unwrap_or_default();
                let paint =
                    lerp_paint_for_properties(&base, &transition.to, sample.progress, &properties);
                if let Some(widget) = self.widgets.get_mut(&id) {
                    paint.apply_to_layout(&mut widget.props.layout);
                    if sample.finished
                        && (properties.is_empty()
                            || properties.iter().any(|p| {
                                p.eq_ignore_ascii_case("all") || p.eq_ignore_ascii_case("transform")
                            }))
                    {
                        widget.props.layout.transform = transition.to.transform;
                        widget.props.layout.transform_3d = transition.to.transform_3d;
                    }
                    changed_ids.push(id);
                }
                self.motion
                    .css_transition_progress
                    .insert(id, sample.progress);
                if sample.finished {
                    if let Some(motion) = motion {
                        self.motion.pending_motion_completes.push(
                            CssMotionComplete::transition_end(
                                id,
                                &motion,
                                transition_elapsed_secs(&motion),
                            ),
                        );
                    }
                    self.motion.css_transitions.remove(&id);
                    self.motion.css_transition_base.remove(&id);
                    self.motion.css_transition_progress.remove(&id);
                }
                continue;
            }
            if sample.id == css_keyframes_animation_id(id) {
                if let Some(motion) = self.motion.computed_motion.get(&id).cloned()
                    && let Some(rule) = self.cascade.keyframes.get(&motion.animation_name)
                    && let Some(paint) = keyframe_paint_at(rule, sample.progress)
                {
                    if let Some(widget) = self.widgets.get_mut(&id) {
                        paint.apply_to_layout(&mut widget.props.layout);
                        changed_ids.push(id);
                    }
                    if sample.finished {
                        self.motion.pending_motion_completes.push(
                            CssMotionComplete::animation_end(
                                id,
                                &motion,
                                animation_elapsed_secs(&motion),
                            ),
                        );
                    }
                }
                if sample.finished {
                    // Runtime already dropped the spec; stale name would skip
                    // the next same-name start after class remove/add recascade.
                    self.motion.css_keyframes_name.remove(&id);
                }
            }
        }
        changed_ids
    }

    fn match_ancestry(&self, id: WidgetId) -> Option<Vec<MatchNodeSnap>> {
        let mut out = Vec::new();
        let mut cur = Some(id);
        while let Some(cid) = cur {
            let is_empty = self.widget_is_empty(cid);
            let w = self.widgets.get(&cid)?;
            let parent = w.parent;
            out.push(match_snap_from_widget(w, is_empty));
            cur = parent;
        }
        Some(out)
    }

    fn real_child_ids(&self, parent_id: WidgetId) -> Vec<WidgetId> {
        let children = self
            .widgets
            .get(&parent_id)
            .map(|p| p.children.clone())
            .unwrap_or_default();
        children
            .into_iter()
            .filter(|&cid| self.is_element_sibling(cid))
            .collect()
    }

    /// Position among parent's **element** children for `:first-child` / `:last-child`.
    /// Generated `::before` / `::after` and DOM text nodes (`#text` /
    /// `createText` / `nana-text`) are not siblings. L2 `p` / `span` / `label`
    /// / headings stay elements even when their kind is [`WidgetKind::Text`].
    fn sibling_position(&self, id: WidgetId) -> (usize, usize) {
        if !self.is_element_sibling(id) {
            return (usize::MAX, 0);
        }
        let Some(parent_id) = self.widgets.get(&id).and_then(|w| w.parent) else {
            return (0, 1);
        };
        let real = self.real_child_ids(parent_id);
        let count = real.len();
        let index = real.iter().position(|&cid| cid == id).unwrap_or(0);
        (index, count.max(1))
    }

    /// Position among same-tag siblings for `:nth-of-type` (0-based index, count).
    fn of_type_position(&self, id: WidgetId) -> (usize, usize) {
        if !self.is_element_sibling(id) {
            return (usize::MAX, 0);
        }
        let Some(widget) = self.widgets.get(&id) else {
            return (0, 1);
        };
        let tag = if widget.props.element_tag.is_empty() {
            widget.kind.element_tag().to_string()
        } else {
            widget.props.element_tag.clone()
        };
        let Some(parent_id) = widget.parent else {
            return (0, 1);
        };
        let children = self.real_child_ids(parent_id);
        let mut index = 0usize;
        let mut count = 0usize;
        for cid in children {
            let Some(w) = self.widgets.get(&cid) else {
                continue;
            };
            let t = if w.props.element_tag.is_empty() {
                w.kind.element_tag().to_string()
            } else {
                w.props.element_tag.clone()
            };
            if !t.eq_ignore_ascii_case(&tag) {
                continue;
            }
            if cid == id {
                index = count;
            }
            count += 1;
        }
        (index, count.max(1))
    }

    /// `:empty`: no element children, no host `label`/`value` with a
    /// non-whitespace UTF-8 scalar (`char::is_whitespace`), and no such child
    /// text. Text nodes are `#text` / `createText` / `nana-text` only — L2
    /// `p` / `span` / `label` / headings are elements (even as
    /// [`WidgetKind::Text`]). Generated `::before`/`::after` ignored.
    fn widget_is_empty(&self, id: WidgetId) -> bool {
        let Some(w) = self.widgets.get(&id) else {
            return true;
        };
        if text_has_non_whitespace(&w.props.label) || text_has_non_whitespace(&w.props.value) {
            return false;
        }
        let children = w.children.clone();
        for cid in children {
            if self.is_generated_pseudo_widget(cid) {
                continue;
            }
            if self.widget_is_dom_text(cid) {
                if !self.widget_is_empty(cid) {
                    return false;
                }
                continue;
            }
            return false;
        }
        true
    }

    fn reapply_parent_and_children(&mut self, parent: WidgetId) {
        self.reapply_layout_for(parent);
        let children = self
            .widgets
            .get(&parent)
            .map(|w| w.children.clone())
            .unwrap_or_default();
        for id in children {
            self.reapply_layout_for(id);
        }
    }

    fn reapply_following_siblings(&mut self, id: WidgetId) {
        let Some(parent) = self.widgets.get(&id).and_then(|w| w.parent) else {
            return;
        };
        let children = self
            .widgets
            .get(&parent)
            .map(|w| w.children.clone())
            .unwrap_or_default();
        let Some(idx) = children.iter().position(|&cid| cid == id) else {
            return;
        };
        for &cid in &children[idx + 1..] {
            self.reapply_layout_for(cid);
        }
    }

    fn prev_sibling_snaps(&self, id: WidgetId) -> Vec<MatchNodeSnap> {
        let Some(parent_id) = self.widgets.get(&id).and_then(|w| w.parent) else {
            return Vec::new();
        };
        let real = self.real_child_ids(parent_id);
        let Some(index) = real.iter().position(|&cid| cid == id) else {
            return Vec::new();
        };
        real[..index]
            .iter()
            .rev()
            .copied()
            .filter_map(|cid| {
                let is_empty = self.widget_is_empty(cid);
                let w = self.widgets.get(&cid)?;
                Some(match_snap_from_widget(w, is_empty))
            })
            .collect()
    }

    fn media_env(&self) -> crate::css_cascade::MediaEnv {
        crate::css_cascade::MediaEnv {
            viewport: self.cascade.layout_viewport,
            color_scheme_dark: matches!(self.theme, ThemeMode::Dark),
        }
    }

    fn css_needs_relative(&self) -> bool {
        use crate::css_cascade::selector_needs_relative;
        stylesheet_needs_relative(&self.cascade.stylesheet_rules)
            || self
                .cascade
                .motion_rules
                .iter()
                .any(|rule| rule.selectors.iter().any(selector_needs_relative))
            || self
                .cascade
                .generated_pseudo_rules
                .iter()
                .any(|rule| selector_needs_relative(&rule.originating_selector))
            || self.cascade.interactive_rules.iter().any(|rule| {
                selector_needs_relative(&crate::css_cascade::Selector {
                    subject: rule.selector.subject.clone(),
                    ancestors: rule.selector.ancestors.clone(),
                    specificity: rule.selector.specificity,
                })
            })
    }

    fn begin_relative_pass(&mut self) {
        if self.cascade.relative_pass.is_some() || !self.css_needs_relative() {
            return;
        }
        self.cascade.relative_pass = Some(Arc::new(self.build_relative_forest()));
    }

    fn ensure_relative_pass(&mut self) {
        self.begin_relative_pass();
    }

    fn end_relative_pass(&mut self) {
        self.cascade.relative_pass = None;
    }

    fn invalidate_relative_pass(&mut self) {
        self.cascade.relative_pass = None;
    }

    fn build_relative_forest(&self) -> RelativeMatchForest {
        let mut forest = RelativeMatchForest::default();
        for (&id, widget) in &self.widgets {
            let tag = if widget.props.element_tag.is_empty() {
                widget.kind.element_tag().to_string()
            } else {
                widget.props.element_tag.clone()
            };
            forest.insert(
                id,
                RelativeMatchNode {
                    tag,
                    css_id: widget.props.element_id.clone(),
                    classes: widget.props.class_names.clone(),
                    attrs: widget.props.attrs.clone(),
                    children: widget.children.clone(),
                    parent: widget.parent,
                },
            );
        }
        self.cascade
            .relative_forest_builds
            .set(self.cascade.relative_forest_builds.get().saturating_add(1));
        self.cascade.relative_forest_nodes.set(
            self.cascade
                .relative_forest_nodes
                .get()
                .saturating_add(forest.len()),
        );
        forest
    }

    fn all_sibling_snaps(&self, id: WidgetId) -> Vec<MatchNodeSnap> {
        let Some(widget) = self.widgets.get(&id) else {
            return Vec::new();
        };
        let Some(parent_id) = widget.parent else {
            return vec![match_snap_from_widget(widget, self.widget_is_empty(id))];
        };
        let Some(parent) = self.widgets.get(&parent_id) else {
            return Vec::new();
        };
        parent
            .children
            .iter()
            .filter_map(|&cid| {
                let w = self.widgets.get(&cid)?;
                Some(match_snap_from_widget(w, self.widget_is_empty(cid)))
            })
            .collect()
    }

    fn reapply_relative_ancestors(&mut self, id: WidgetId) {
        self.reapply_relative_neighborhood(id);
    }

    /// Recascade `id` (if present), all siblings, and the ancestor chain.
    /// Required for `:has()`, `:nth-child`, and `of <selector-list>`.
    fn reapply_relative_neighborhood(&mut self, id: WidgetId) {
        if !self.css_needs_relative() {
            return;
        }
        self.invalidate_relative_pass();
        let parent = self.widgets.get(&id).and_then(|w| w.parent);
        let mut dirty = HashSet::new();
        if self.widgets.contains_key(&id) {
            dirty.insert(id);
        }
        if let Some(pid) = parent {
            dirty.insert(pid);
            if let Some(p) = self.widgets.get(&pid) {
                dirty.extend(p.children.iter().copied());
            }
            let mut cur = Some(pid);
            while let Some(cid) = cur {
                dirty.insert(cid);
                cur = self.widgets.get(&cid).and_then(|w| w.parent);
            }
        }
        let mut ordered: Vec<WidgetId> = dirty.into_iter().collect();
        ordered.sort_by_cached_key(|wid| self.widget_depth(*wid));
        self.begin_relative_pass();
        for wid in ordered {
            self.reapply_layout_for(wid);
        }
        self.end_relative_pass();
    }

    fn reapply_relative_neighborhood_of_parent(&mut self, parent: WidgetId) {
        if !self.css_needs_relative() {
            return;
        }
        self.invalidate_relative_pass();
        let mut dirty = HashSet::new();
        dirty.insert(parent);
        if let Some(p) = self.widgets.get(&parent) {
            dirty.extend(p.children.iter().copied());
        }
        let mut cur = self.widgets.get(&parent).and_then(|w| w.parent);
        while let Some(cid) = cur {
            dirty.insert(cid);
            cur = self.widgets.get(&cid).and_then(|w| w.parent);
        }
        let mut ordered: Vec<WidgetId> = dirty.into_iter().collect();
        ordered.sort_by_cached_key(|wid| self.widget_depth(*wid));
        self.begin_relative_pass();
        for wid in ordered {
            self.reapply_layout_for(wid);
        }
        self.end_relative_pass();
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn theme(&self) -> ThemeMode {
        self.theme
    }

    pub fn set_theme(&mut self, theme: ThemeMode) {
        let changed = self.theme != theme;
        self.theme = theme;
        // Mirror JS `documentElement.dataset.theme` onto the html scaffold so
        // cascade `[data-theme=…]` / `:root[data-theme=…]` can match the node
        // (document `--*` still come from theme-aware stylesheet_vars).
        self.sync_document_theme_attr();
        if changed {
            // Theme-conditional document vars (`:root[data-theme=…]`) must
            // re-resolve; otherwise Primary paint sticks on the last inject.
            if self.authored_has_media() {
                self.rebuild_active_stylesheet();
            }
            self.rebuild_stylesheet_vars();
            self.reapply_layout_cascade_all();
        }
    }

    fn sync_document_theme_attr(&mut self) {
        let label = self.theme_label().to_string();
        for w in self.widgets.values_mut() {
            let is_html_root = w.parent.is_none()
                && (w.props.element_tag.eq_ignore_ascii_case("html")
                    || w.props.class_names.iter().any(|c| c == "nana-html-root"));
            if is_html_root {
                w.props.attrs.insert("data-theme".into(), label.clone());
            }
        }
    }

    pub fn appearance(&self) -> AppearanceSettings {
        self.appearance
    }

    pub fn set_appearance(&mut self, appearance: AppearanceSettings) {
        if self.appearance != appearance {
            self.appearance = appearance;
            self.changed_all();
        }
    }

    /// Sync theme + Appearance fields from L1 `documentElement` dataset/style.
    ///
    /// Keys match `nanavue-components` / web-api shim:
    /// `theme`, `backdrop`, `backdropTarget`, `titlebarFollowsSidebar`,
    /// `workspaceCorners`, and style `--lilia-backdrop-opacity` / `--nana-backdrop-opacity` /
    /// `--backdrop-opacity` / `--app-corner-radius`.
    ///
    /// Theme direction: JS `dataset.theme` → bridge [`ThemeMode`] (paired with
    /// [`crate::VueHost::inject_theme`] for Rust → JS).
    pub fn apply_document_appearance(
        &mut self,
        dataset: &BTreeMap<String, String>,
        style: &BTreeMap<String, String>,
    ) {
        if let Some(raw) = dataset.get("theme") {
            let mode = if raw.eq_ignore_ascii_case("dark") {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            };
            self.set_theme(mode);
        }
        let mut next = self.appearance;
        if let Some(raw) = dataset.get("backdrop") {
            let mode = match raw.as_str() {
                "vibrancy" => WindowMaterialMode::Vibrancy,
                "mica" => WindowMaterialMode::Mica,
                "acrylic" => WindowMaterialMode::Acrylic,
                "translucent" | "transparent" | "system" => WindowMaterialMode::Translucent,
                _ => WindowMaterialMode::Solid,
            };
            next.set_window_material(mode);
        }
        if let Some(raw) = dataset.get("backdropTarget") {
            let target = if raw == "main" {
                BackdropTarget::Main
            } else {
                BackdropTarget::Sidebar
            };
            next.set_backdrop_target(target);
        }
        if let Some(raw) = dataset.get("titlebarFollowsSidebar") {
            next.set_titlebar_follows_sidebar(raw != "false");
        }
        if let Some(raw) = dataset.get("workspaceCorners") {
            next.set_workspace_corners_enabled(raw != "false");
        } else if let Some(raw) = dataset.get("corners") {
            // Legacy: only "square" disables workspace corners.
            next.set_workspace_corners_enabled(raw != "square");
        }
        if let Some(raw) = style
            .get("--lilia-backdrop-opacity")
            .or_else(|| style.get("--nana-backdrop-opacity"))
            .or_else(|| style.get("--backdrop-opacity"))
            .or_else(|| style.get("backdrop-opacity"))
            .or_else(|| style.get("nana-backdrop-opacity"))
            .or_else(|| style.get("lilia-backdrop-opacity"))
            && let Ok(opacity) = raw.parse::<f32>()
        {
            next.set_backdrop_opacity(opacity);
        }
        if let Some(raw) = style
            .get("--app-corner-radius")
            .or_else(|| style.get("app-corner-radius"))
        {
            let px = raw.trim_end_matches("px").trim();
            if let Ok(radius) = px.parse::<f32>() {
                next.set_standard_radius(radius);
            }
        }
        self.set_appearance(next);
    }

    pub fn theme_label(&self) -> &'static str {
        match self.theme {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }

    pub fn get(&self, id: WidgetId) -> Option<&SemanticWidget> {
        self.widgets.get(&id)
    }

    pub fn get_mut(&mut self, id: WidgetId) -> Option<&mut SemanticWidget> {
        self.widgets.get_mut(&id)
    }

    pub fn widgets(&self) -> impl Iterator<Item = &SemanticWidget> {
        self.widgets.values()
    }

    pub(crate) fn root_ids(&self) -> &[WidgetId] {
        &self.roots
    }

    pub fn contains(&self, id: WidgetId) -> bool {
        self.widgets.contains_key(&id)
    }

    /// Register html + body so Vue mounts parent under a real semantic root.
    pub fn ensure_document_roots(&mut self, html_id: WidgetId, body_id: WidgetId) {
        let theme_label = self.theme_label().to_string();
        self.widgets.entry(html_id).or_insert_with(|| {
            let mut props = WidgetProps::default();
            props.layout.width = Some(LengthSpec::Fill);
            props.layout.height = Some(LengthSpec::Fill);
            props.layout.direction = Some(FlexDirection::Column);
            props.class_names = vec!["nana-html-root".into()];
            props.element_tag = "html".into();
            props.attrs.insert("data-theme".into(), theme_label);
            SemanticWidget {
                id: html_id,
                kind: WidgetKind::Column,
                props,
                children: vec![body_id],
                parent: None,
            }
        });
        self.sync_document_theme_attr();
        self.widgets.entry(body_id).or_insert_with(|| {
            let mut props = WidgetProps::default();
            props.layout.width = Some(LengthSpec::Fill);
            props.layout.height = Some(LengthSpec::Fill);
            props.layout.direction = Some(FlexDirection::Column);
            props.class_names = vec!["nana-mount-root".into()];
            SemanticWidget {
                id: body_id,
                kind: WidgetKind::Column,
                props,
                children: Vec::new(),
                parent: Some(html_id),
            }
        });
        if let Some(body) = self.widgets.get_mut(&body_id) {
            body.parent = Some(html_id);
        }
        let child_ok: Vec<WidgetId> = self
            .widgets
            .get(&html_id)
            .map(|html| {
                html.children
                    .iter()
                    .copied()
                    .filter(|c| *c == body_id || self.widgets.contains_key(c))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(html) = self.widgets.get_mut(&html_id) {
            html.children = child_ok;
            if !html.children.contains(&body_id) {
                html.children.push(body_id);
            }
        }
        self.roots.clear();
        // Paint from body — html is scaffolding only.
        self.roots.push(body_id);
        self.scaffolded = true;
        self.changed_structure();
    }

    /// Drop mounted app widgets under body; keep html/body scaffold.
    pub fn clear_mounted(&mut self) {
        if !self.scaffolded {
            self.widgets.clear();
            self.roots.clear();
            self.changed_structure();
            return;
        }
        let body_id = self
            .widgets
            .iter()
            .find(|(_, w)| w.props.class_names.iter().any(|c| c == "nana-mount-root"))
            .map(|(id, _)| *id);
        let Some(body_id) = body_id else {
            return;
        };
        let children: Vec<WidgetId> = self
            .widgets
            .get(&body_id)
            .map(|w| w.children.clone())
            .unwrap_or_default();
        for child in children {
            self.unregister(child);
        }
        // Sweep orphans that lost their parent during partial unmounts.
        let orphans: Vec<WidgetId> = self
            .widgets
            .iter()
            .filter_map(|(&id, w)| {
                if self.roots.contains(&id) || id == body_id {
                    return None;
                }
                match w.parent {
                    Some(p) if self.widgets.contains_key(&p) => None,
                    _ => Some(id),
                }
            })
            .collect();
        for id in orphans {
            self.unregister(id);
        }
        self.changed_structure();
    }

    pub fn register(&mut self, id: WidgetId, kind: WidgetKind, mut props: WidgetProps) {
        if props.element_tag.is_empty() {
            props.element_tag = kind.element_tag().to_string();
        }
        // Seed layout defaults for layout kinds; stylesheet / class / inline win later.
        let defaults = default_layout_for_kind(kind);
        if props.layout.direction.is_none() {
            props.layout.direction = defaults.direction;
        }
        if props.layout.gap.is_none() {
            props.layout.gap = defaults.gap;
        }
        if props.layout.padding.is_none() {
            props.layout.padding = defaults.padding;
        }
        if kind.is_overlay() {
            apply_overlay_presence_open(&mut props);
            // Product floats use Nana Overlay — strip companion CSS fixed/sticky.
            if matches!(
                props.layout.position,
                crate::css_map::PositionSpec::Fixed | crate::css_map::PositionSpec::Sticky
            ) {
                props.layout.position = crate::css_map::PositionSpec::Static;
            }
        }
        self.widgets.insert(
            id,
            SemanticWidget {
                id,
                kind,
                props,
                children: Vec::new(),
                parent: None,
            },
        );
        // With document scaffold, only html is a root — insert parents under body.
        // Without scaffold (unit tests), keep legacy "register ⇒ root" behavior.
        if !self.scaffolded && !self.roots.contains(&id) {
            self.roots.push(id);
        }
        self.reapply_layout_for(id);
        self.bump();
    }

    /// Copy kind+props from `src` onto `dst` (no parenting). Returns false if `src` missing.
    pub fn clone_register(&mut self, src: WidgetId, dst: WidgetId) -> bool {
        let Some(widget) = self.widgets.get(&src).cloned() else {
            return false;
        };
        self.register(dst, widget.kind, widget.props);
        true
    }

    /// Ensure a node is registered; used when downleveling bare HTML.
    pub fn ensure(&mut self, id: WidgetId, kind: WidgetKind, props: WidgetProps) {
        if self.widgets.contains_key(&id) {
            if let Some(w) = self.widgets.get_mut(&id) {
                w.kind = kind;
                // Merge non-empty props lightly.
                if !props.label.is_empty() {
                    w.props.label = props.label;
                }
                if !props.value.is_empty() {
                    w.props.value = props.value;
                }
                if !props.class_names.is_empty() {
                    w.props.class_names = props.class_names;
                }
                if !props.role.is_empty() {
                    w.props.role = props.role;
                }
            }
            self.changed_subtree(id);
        } else {
            self.register(id, kind, props);
        }
    }

    pub fn set_kind(&mut self, id: WidgetId, kind: WidgetKind) {
        if let Some(w) = self.widgets.get_mut(&id)
            && w.kind != kind
        {
            w.kind = kind;
            self.changed_subtree(id);
        }
    }

    pub fn unregister(&mut self, id: WidgetId) {
        let old_parent = self.widgets.get(&id).and_then(|w| w.parent);
        if let Some(slots) = self.generated_pseudo_children.remove(&id) {
            for child in [slots.before, slots.after].into_iter().flatten() {
                self.teardown_generated_pseudo_sidecar(child);
            }
        }
        if let Some(origin) = self
            .widgets
            .get(&id)
            .and_then(|w| w.props.attrs.get(GENERATED_PSEUDO_ORIGIN_ATTR))
            .and_then(|raw| raw.parse::<u64>().ok())
            && let Some(slots) = self.generated_pseudo_children.get_mut(&origin)
        {
            if slots.before == Some(id) {
                slots.before = None;
            }
            if slots.after == Some(id) {
                slots.after = None;
            }
            if slots.before.is_none() && slots.after.is_none() {
                self.generated_pseudo_children.remove(&origin);
            }
        }
        let svg_parent = self.widgets.get(&id).and_then(|w| w.parent);
        if let Some(widget) = self.widgets.remove(&id) {
            if let Some(parent) = widget.parent
                && let Some(p) = self.widgets.get_mut(&parent)
            {
                p.children.retain(|&c| c != id);
            }
            for child in widget.children {
                self.unregister(child);
            }
        }
        self.roots.retain(|&r| r != id);
        self.motion.computed_motion.remove(&id);
        self.motion.css_transitions.remove(&id);
        self.motion.css_transition_base.remove(&id);
        self.motion.css_transition_progress.remove(&id);
        self.motion
            .pending_motion_completes
            .retain(|event| event.widget_id != id);
        self.motion
            .pending_motion_cancels
            .retain(|queued| *queued != id);
        self.motion.css_keyframes_name.remove(&id);
        self.motion.paint_transform_overlays.remove(&id);
        self.motion.paint_transform_releases.remove(&id);
        if let Some(parent) = old_parent
            && self.widgets.contains_key(&parent)
        {
            self.reapply_relative_neighborhood_of_parent(parent);
        }
        if let Some(parent) = svg_parent
            && self.widgets.contains_key(&parent)
        {
            self.recascade_inline_svg(parent);
        }
        // Subject `:has()` / `:empty` / sibling nth on remaining parent.
        if let Some(parent) = svg_parent.filter(|pid| self.widgets.contains_key(pid)) {
            self.cascade.has_index_ready = false;
            self.reapply_parent_and_children(parent);
            let mut walk = Some(parent);
            while let Some(pid) = walk {
                if !self.widgets.contains_key(&pid) {
                    break;
                }
                self.reapply_layout_for(pid);
                walk = self.widgets.get(&pid).and_then(|w| w.parent);
            }
        }
        self.changed_structure();
    }

    pub fn purge_generated_pseudo_runtime(
        &mut self,
        origin: WidgetId,
        doc: &mut crate::tree::NanaTreeDocument,
    ) {
        let Some(slots) = self.generated_pseudo_children.get(&origin) else {
            return;
        };
        for child in [slots.before, slots.after].into_iter().flatten() {
            doc.remove_generated_pseudo(crate::tree::NodeHandle(child));
        }
    }

    fn teardown_generated_pseudo_sidecar(&mut self, child: WidgetId) {
        self.motion.computed_motion.remove(&child);
        self.motion.css_transitions.remove(&child);
        self.motion.css_transition_base.remove(&child);
        self.motion.css_transition_progress.remove(&child);
        if let Some(widget) = self.widgets.remove(&child)
            && let Some(parent) = widget.parent
            && let Some(p) = self.widgets.get_mut(&parent)
        {
            p.children.retain(|&c| c != child);
        }
    }

    pub fn insert_child(&mut self, child: WidgetId, parent: WidgetId, anchor: Option<WidgetId>) {
        let old_parent = self.widgets.get(&child).and_then(|w| w.parent);
        if let Some(prev) = old_parent
            && let Some(p) = self.widgets.get_mut(&prev)
        {
            p.children.retain(|&c| c != child);
        }
        self.roots.retain(|&r| r != child);

        if !self.widgets.contains_key(&parent) {
            // With document scaffold, never promote random orphans to roots —
            // attach under the mount body instead so the forest stays single-rooted.
            if self.scaffolded
                && let Some(body_id) = self.mount_body_id()
            {
                return self.insert_child(child, body_id, None);
            }
            if self.widgets.contains_key(&child) {
                if let Some(w) = self.widgets.get_mut(&child) {
                    w.parent = None;
                }
                if !self.roots.contains(&child) {
                    self.roots.push(child);
                }
                self.changed_structure();
            }
            return;
        }

        if let Some(w) = self.widgets.get_mut(&child) {
            w.parent = Some(parent);
        }
        if let Some(p) = self.widgets.get_mut(&parent) {
            let idx = anchor
                .and_then(|a| p.children.iter().position(|&c| c == a))
                .unwrap_or(p.children.len());
            if !p.children.contains(&child) {
                p.children.insert(idx, child);
            }
        }
        self.sync_containing_block_from_parent(child);
        // Parent combinators / `:empty` / sibling nth match on insert.
        self.cascade.has_index_ready = false;
        self.reapply_parent_and_children(parent);
        if !self.cascade.has_args.is_empty() {
            let mut walk = Some(parent);
            while let Some(pid) = walk {
                self.reapply_layout_for(pid);
                walk = self.widgets.get(&pid).and_then(|w| w.parent);
            }
        }
        self.reapply_layout_for(child);
        if let Some(prev) = old_parent
            && prev != parent
            && self.widgets.contains_key(&prev)
        {
            self.reapply_relative_neighborhood_of_parent(prev);
        }
        self.reapply_relative_neighborhood(child);
        self.recascade_inline_svg(parent);
        self.changed_structure();
    }

    fn recascade_inline_svg(&mut self, id: WidgetId) {
        if let Some(root) = crate::svg_inline::nearest_svg_root(self, id) {
            self.reapply_layout_for(root);
        }
    }

    /// Host / Scene 回写最近布局得到的包含块尺寸（供后续 `style` `%` 解析）。
    pub fn set_containing_block(&mut self, id: WidgetId, width: Option<f32>, height: Option<f32>) {
        if !self.write_containing_block(id, width, height) {
            return;
        }
        let children = self
            .widgets
            .get(&id)
            .map(|w| w.children.clone())
            .unwrap_or_default();
        self.bump();
        for child in children {
            self.sync_containing_block_from_parent(child);
        }
    }

    /// Scene / viewport 布局回写：按 Fill 父链把 viewport → root CB → 子 content box。
    ///
    /// 与 [`LayoutStyle::resolve_content_box`] 一致；稳定时不 bump。
    pub fn sync_layout_containing_blocks(&mut self, viewport: ParentBox) {
        let mut viewport_changed = false;
        if let (Some(w), Some(h)) = (viewport.width, viewport.height) {
            let next = Some((w, h));
            if self.cascade.layout_viewport != next {
                self.cascade.layout_viewport = next;
                viewport_changed = true;
            }
        }
        let roots = self.roots.clone();
        if roots.is_empty() {
            return;
        }
        let mut changed = false;
        let vp = self.cascade.layout_viewport;
        for root in roots {
            if self.write_containing_block(root, viewport.width, viewport.height) {
                changed = true;
            }
            self.propagate_layout_containing_blocks(root, vp, &mut changed);
        }
        // Re-cascade after CB writeback so % / vh resolve against fresh bases.
        if viewport_changed {
            if self.authored_has_media() {
                self.rebuild_active_stylesheet();
                self.rebuild_stylesheet_vars();
            }
            self.reapply_layout_cascade_all();
        } else if changed {
            self.changed_all();
        }
    }

    /// Flush RuntimeLayoutEngine. CSS measure is not written over engine boxes.
    pub(crate) fn resolve_document_layout(&mut self, doc: &mut crate::tree::NanaTreeDocument) {
        let (logical_w, logical_h) = doc.logical_size();
        self.reparent_orphans();
        self.sync_sidebar_footer_into_document(doc);
        self.sync_layout_containing_blocks(ParentBox::from_viewport(logical_w, logical_h));
        if self.has_interactive_css() {
            self.reapply_interactive_cascade(doc);
        } else {
            self.discard_interactive_runtime_if_unused();
        }
        self.release_pending_flip_transforms(doc);
        self.sync_cascaded_layout_into_runtime(doc);
        doc.flush_host_frame();
    }

    pub(crate) fn sync_cascaded_layout_into_runtime(
        &self,
        doc: &mut crate::tree::NanaTreeDocument,
    ) {
        // Compare in place; clone LayoutStyle only for nodes whose cascade
        // actually changed. Never writes Runtime LayoutBox.
        doc.sync_widget_layouts(
            self.widgets
                .iter()
                .map(|(id, widget)| (*id, &widget.props.layout)),
        );
    }

    /// Fill only nodes that still have no engine box after flush.
    pub(crate) fn resolve_missing_document_layout(
        &mut self,
        doc: &mut crate::tree::NanaTreeDocument,
    ) {
        let (logical_w, logical_h) = doc.logical_size();
        self.reparent_orphans();
        self.sync_sidebar_footer_into_document(doc);
        self.sync_layout_containing_blocks(ParentBox::from_viewport(logical_w, logical_h));
        self.release_pending_flip_transforms(doc);
        self.sync_cascaded_layout_into_runtime(doc);
        doc.flush_host_frame();
        // CSS measure only fills boxes the engine has not produced yet. When
        // every reachable node already has one, skip the shadow-tree rebuild.
        if self.all_reachable_nodes_have_engine_boxes(doc) {
            return;
        }
        let boxes = crate::measure_bridge_layout_boxes(self, logical_w, logical_h);
        let missing: Vec<_> = boxes
            .into_iter()
            .filter(|(handle, _)| !doc.has_engine_layout_box(*handle))
            .collect();
        if !missing.is_empty() {
            doc.apply_layout_boxes(&missing);
        }
    }

    /// True when every widget reachable from the document roots already has an
    /// engine box. Unreachable widgets (measure never emits them) are ignored.
    fn all_reachable_nodes_have_engine_boxes(&self, doc: &crate::tree::NanaTreeDocument) -> bool {
        let mut stack: Vec<WidgetId> = self.root_ids().to_vec();
        while let Some(id) = stack.pop() {
            if !doc.has_engine_layout_box(crate::tree::NodeHandle(id)) {
                return false;
            }
            if let Some(widget) = self.widgets.get(&id) {
                stack.extend(widget.children.iter().copied());
            }
        }
        true
    }

    fn write_containing_block(
        &mut self,
        id: WidgetId,
        width: Option<f32>,
        height: Option<f32>,
    ) -> bool {
        let Some(w) = self.widgets.get_mut(&id) else {
            return false;
        };
        let next_w = width.filter(|v| *v > 0.0);
        let next_h = height.filter(|v| *v > 0.0);
        if w.props.containing_block_width == next_w && w.props.containing_block_height == next_h {
            return false;
        }
        w.props.containing_block_width = next_w;
        w.props.containing_block_height = next_h;
        self.changes.dirty.insert(id);
        true
    }

    fn propagate_layout_containing_blocks(
        &mut self,
        id: WidgetId,
        viewport: Option<(f32, f32)>,
        changed: &mut bool,
    ) {
        let (content, children) = {
            let Some(widget) = self.widgets.get(&id) else {
                return;
            };
            let parent = ParentBox {
                width: widget.props.containing_block_width,
                height: widget.props.containing_block_height,
            };
            let content = widget
                .props
                .layout
                .resolve_content_box_with_viewport(parent, viewport);
            (content, widget.children.clone())
        };
        for child in children {
            if self.write_containing_block(child, content.width, content.height) {
                *changed = true;
            }
            self.propagate_layout_containing_blocks(child, viewport, changed);
        }
    }

    /// 用父节点 content box（Fill 链友好）写入子节点的包含块基。
    fn sync_containing_block_from_parent(&mut self, id: WidgetId) {
        let parent_id = match self.widgets.get(&id).and_then(|w| w.parent) {
            Some(p) => p,
            None => return,
        };
        let (cw, ch) = self.estimate_content_box(parent_id);
        let _ = self.write_containing_block(id, cw, ch);
    }

    fn estimate_content_box(&self, id: WidgetId) -> (Option<f32>, Option<f32>) {
        let Some(widget) = self.widgets.get(&id) else {
            return (None, None);
        };
        // Match `resolve_content_box` so Fill/grow parents pass viewport size.
        let parent = ParentBox {
            width: widget.props.containing_block_width,
            height: widget.props.containing_block_height,
        };
        let content = widget
            .props
            .layout
            .resolve_content_box_with_viewport(parent, self.cascade.layout_viewport);
        (content.width, content.height)
    }

    fn mount_body_id(&self) -> Option<WidgetId> {
        self.widgets.iter().find_map(|(&id, w)| {
            w.props
                .class_names
                .iter()
                .any(|c| c == "nana-mount-root")
                .then_some(id)
        })
    }

    /// Attach unreachable sidebar shells under a stable workspace shell so Scene
    /// paints them.
    ///
    /// At most **one** orphan sidebar is reparented. Remount leftovers used to pile
    /// multiple `SidebarFrame`s into the row and starve the primary column width.
    ///
    /// Parent preference (first match wins):
    /// 1. `nana-workspace-shell__body` (nanavue DesktopShell contract)
    /// 2. Documented region content (`nana-workspace-region__content` under a
    ///    resources region via `data-region-role` / `agent_id`)
    /// 3. Resources region host (`data-region-role` / `agent_id`)
    ///
    /// Never steal onto a bare `flex-row` without workspace identity.
    pub fn reparent_orphans(&mut self) {
        if !self.scaffolded {
            return;
        }
        let Some(workspace_row) = self.find_sidebar_reparent_host() else {
            self.reparent_sidebar_footer_slots();
            return;
        };
        let already_has_sidebar = self.widgets.get(&workspace_row).is_some_and(|row| {
            row.children.iter().any(|cid| {
                self.widgets
                    .get(cid)
                    .is_some_and(|w| matches!(w.kind, WidgetKind::SidebarFrame))
            })
        });
        if already_has_sidebar {
            self.reparent_sidebar_footer_slots();
            return;
        }
        let mut reachable = std::collections::HashSet::new();
        for &root in &self.roots {
            self.collect_reachable(root, &mut reachable);
        }
        let mut orphans: Vec<(WidgetId, usize)> = self
            .widgets
            .iter()
            .filter_map(|(&id, w)| {
                if id <= 2 || reachable.contains(&id) || self.roots.contains(&id) {
                    return None;
                }
                matches!(w.kind, WidgetKind::SidebarFrame).then_some((id, w.children.len()))
            })
            .collect();
        // Prefer the densest sidebar shell (real nav over empty remount leftovers).
        orphans.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
        let Some((id, _)) = orphans.into_iter().next() else {
            self.reparent_sidebar_footer_slots();
            return;
        };
        if id == workspace_row {
            self.reparent_sidebar_footer_slots();
            return;
        }
        let mut seen = std::collections::HashSet::new();
        self.collect_reachable(id, &mut seen);
        if seen.contains(&workspace_row) {
            self.reparent_sidebar_footer_slots();
            return;
        }
        // Insert before the first existing child so the sidebar stays left of Primary.
        let anchor = self
            .widgets
            .get(&workspace_row)
            .and_then(|r| r.children.first().copied());
        self.insert_child(id, workspace_row, anchor);
        // Workspace-row fallback skips ResourcePanel's height:100% content host.
        // Re-seed CB from the layout viewport so Fill / overflow-y scrollports
        // resolve to a finite height instead of Fill→0 under auto parents.
        if let Some((vw, vh)) = self.cascade.layout_viewport {
            self.sync_layout_containing_blocks(ParentBox::from_viewport(vw, vh));
        }
        self.reparent_sidebar_footer_slots();
        self.changed_structure();
    }

    /// Reattach orphaned `nana-sidebar-frame__footer` slots (and their content)
    /// under the live reachable [`WidgetKind::SidebarFrame`].
    ///
    /// Remount + stale wrapNode insert targets used to detach footer columns
    /// from the frame while leaving top/body intact. Heal the slot contract so
    /// Scene paints the fixed footer again.
    pub fn reparent_sidebar_footer_slots(&mut self) {
        if !self.scaffolded {
            return;
        }
        let reachable = self.roots_reachable();
        let Some(frame_id) = self.reachable_sidebar_frame(&reachable) else {
            return;
        };
        let has_footer_slot = self.widgets.get(&frame_id).is_some_and(|frame| {
            frame.children.iter().any(|cid| {
                self.widgets
                    .get(cid)
                    .is_some_and(|c| is_sidebar_footer_slot(&c.props))
            })
        });
        if !has_footer_slot {
            let footer_orphans: Vec<(WidgetId, usize, u64)> = self
                .widgets
                .iter()
                .filter_map(|(&id, w)| {
                    if reachable.contains(&id) || id <= 2 || !is_sidebar_footer_slot(&w.props) {
                        return None;
                    }
                    // Prefer densest / newest leftover from the latest remount.
                    Some((id, w.children.len(), id))
                })
                .collect();
            if let Some(footer_id) = prefer_dense_newest(footer_orphans) {
                self.insert_child(footer_id, frame_id, None);
            }
        }
        let Some(footer_id) = self.widgets.get(&frame_id).and_then(|frame| {
            frame.children.iter().copied().find(|cid| {
                self.widgets
                    .get(cid)
                    .is_some_and(|c| is_sidebar_footer_slot(&c.props))
            })
        }) else {
            return;
        };
        if self
            .widgets
            .get(&footer_id)
            .is_some_and(|f| !f.children.is_empty())
        {
            return;
        }
        // Content often sits on an orphan div that still hosts sidebar.footer.*
        // actions after the slot column was emptied by a failed re-insert.
        let content_orphans: Vec<(WidgetId, usize, u64)> = self
            .widgets
            .iter()
            .filter_map(|(&id, w)| {
                if reachable.contains(&id) || id == footer_id || id <= 2 {
                    return None;
                }
                if w.parent.is_some_and(|p| self.widgets.contains_key(&p)) {
                    return None;
                }
                hosts_sidebar_footer_content(w, &self.widgets).then_some((id, w.children.len(), id))
            })
            .collect();
        if let Some(content_id) = prefer_dense_newest(content_orphans) {
            self.insert_child(content_id, footer_id, None);
        }
    }

    /// Mirror bridge footer parenting into the document tree (shared id space).
    pub fn sync_sidebar_footer_into_document(&self, doc: &mut crate::tree::NanaTreeDocument) {
        let reachable = self.roots_reachable();
        let Some(frame_id) = self.reachable_sidebar_frame(&reachable) else {
            return;
        };
        let Some(frame) = self.widgets.get(&frame_id) else {
            return;
        };
        for &cid in &frame.children {
            let Some(child) = self.widgets.get(&cid) else {
                continue;
            };
            if !is_sidebar_footer_slot(&child.props) {
                continue;
            }
            doc.insert(
                crate::tree::NodeHandle(cid),
                crate::tree::NodeHandle(frame_id),
                None,
            );
            for &gcid in &child.children {
                doc.insert(
                    crate::tree::NodeHandle(gcid),
                    crate::tree::NodeHandle(cid),
                    None,
                );
            }
        }
    }

    fn roots_reachable(&self) -> std::collections::HashSet<WidgetId> {
        let mut reachable = std::collections::HashSet::new();
        for &root in &self.roots {
            self.collect_reachable(root, &mut reachable);
        }
        reachable
    }

    fn reachable_sidebar_frame(
        &self,
        reachable: &std::collections::HashSet<WidgetId>,
    ) -> Option<WidgetId> {
        self.widgets.iter().find_map(|(&id, w)| {
            (reachable.contains(&id) && matches!(w.kind, WidgetKind::SidebarFrame)).then_some(id)
        })
    }

    fn find_sidebar_reparent_host(&self) -> Option<WidgetId> {
        let mut reachable = std::collections::HashSet::new();
        for &root in &self.roots {
            self.collect_reachable(root, &mut reachable);
        }
        let reachable = reachable;

        // 1) nanavue NanaWorkspaceShell body
        if let Some(id) = self.widgets.iter().find_map(|(&id, w)| {
            (reachable.contains(&id)
                && w.props
                    .class_names
                    .iter()
                    .any(|c| c == "nana-workspace-shell__body"))
            .then_some(id)
        }) {
            return Some(id);
        }
        let is_resources_shell = |w: &SemanticWidget| {
            w.props.region.eq_ignore_ascii_case("resources")
                || w.props.agent_id == "workspace.region.sidebar"
                || w.props.agent_id == "workspace.region.resources"
                || w.props
                    .attrs
                    .get("data-region-role")
                    .is_some_and(|r| r.eq_ignore_ascii_case("resources"))
        };
        // 2) reachable resources region content wrapper (height chain)
        if let Some(id) = self.widgets.iter().find_map(|(&id, w)| {
            if !reachable.contains(&id) {
                return None;
            }
            let is_content = w
                .props
                .class_names
                .iter()
                .any(|c| c == "nana-workspace-region__content");
            if !is_content {
                return None;
            }
            let parent_ok = w
                .parent
                .and_then(|p| self.widgets.get(&p))
                .is_some_and(is_resources_shell);
            parent_ok.then_some(id)
        }) {
            return Some(id);
        }
        // 3) reachable resources aside
        self.widgets
            .iter()
            .find_map(|(&id, w)| (reachable.contains(&id) && is_resources_shell(w)).then_some(id))
    }

    fn collect_reachable(&self, id: WidgetId, out: &mut std::collections::HashSet<WidgetId>) {
        if !out.insert(id) {
            return;
        }
        if let Some(w) = self.widgets.get(&id) {
            for &child in &w.children {
                self.collect_reachable(child, out);
            }
        }
    }

    pub fn patch_prop(&mut self, id: WidgetId, key: &str, value: &nana_js_engine::HostValue) {
        if key.starts_with("on") || key.starts_with("On") {
            return;
        }
        if !self.widgets.contains_key(&id) {
            return;
        }
        let key_n = normalize_prop_key(key);
        // Refresh CB from parent before style/gap `%` parse.
        if matches!(key_n.as_str(), "style" | "gap" | "padding") {
            self.sync_containing_block_from_parent(id);
        }
        let prev_kind = self
            .widgets
            .get(&id)
            .map(|w| w.kind)
            .unwrap_or(WidgetKind::Column);
        {
            let Some(widget) = self.widgets.get_mut(&id) else {
                return;
            };
            widget.props.apply_prop(key, value);
            // Overlays use `active || toggled` as open. Vue may patch only one side
            // (`active` / `open` / `selected` / `toggled` / `model-value` / aria-*);
            // keep both in sync so host dismiss and v-model close actually collapse
            // (mirrors note_toggle). `selected` alone must not leave toggled stuck.
            if widget.kind.is_overlay() {
                let sync_open = match key_n.as_str() {
                    "active" | "open" | "selected" | "aria-selected" | "aria-pressed"
                    | "aria-expanded" => Some(widget.props.active),
                    "toggled" | "model-value" if host_is_open_flag(value) => {
                        Some(widget.props.toggled)
                    }
                    "aria-modal" => {
                        apply_overlay_presence_open(&mut widget.props);
                        None
                    }
                    _ => None,
                };
                if let Some(open) = sync_open {
                    widget.props.active = open;
                    widget.props.toggled = open;
                }
            }
            if matches!(
                key_n.as_str(),
                "checked" | "toggled" | "model-value" | "modelvalue"
            ) {
                if widget_checked_state(widget) {
                    widget.props.attrs.insert("checked".into(), String::new());
                } else {
                    widget.props.attrs.remove("checked");
                }
            }
            // Re-resolve kind from class / role / type after attribute patches.
            // Layout props (`style` / flex / gap / size) write `LayoutStyle` only.
            if matches!(
                key_n.as_str(),
                "class"
                    | "classname"
                    | "role"
                    | "type"
                    | "aria-pressed"
                    | "aria-selected"
                    | "aria-modal"
                    | "aria-expanded"
                    | "id"
                    | "data-region-id"
            ) {
                let class = widget.props.class_names.join(" ");
                let role = widget.props.role.clone();
                let input_type = if key_n == "type" {
                    host_string(value)
                } else {
                    String::new()
                };
                if let Some(next) =
                    resolve_kind_from_hints("div", Some(&class), Some(&role), Some(&input_type))
                {
                    // Layout identity is LayoutStyle. Do not flip Column/Row/Box
                    // from a synthetic `div` re-resolve; class may still promote
                    // a layout box to a control (chip / dialog).
                    let allow = next != prev_kind && !next.is_layout();
                    if allow {
                        widget.kind = next;
                        if next.is_overlay() && !prev_kind.is_overlay() {
                            apply_overlay_presence_open(&mut widget.props);
                        }
                    }
                }
            }
            if widget.kind.is_overlay()
                && matches!(
                    widget.props.layout.position,
                    crate::css_map::PositionSpec::Fixed | crate::css_map::PositionSpec::Sticky
                )
            {
                widget.props.layout.position = crate::css_map::PositionSpec::Static;
            }
        }
        // Rebuild LayoutStyle from stylesheet + class hints + inline/prop style.
        // Layout props (width/height/…) use the same rebuild as class/style so
        // stylesheet / prop / inline `!important` is not dropped by a one-property
        // overlay. One write of LayoutStyle — never LayoutBox.
        let full_rebuild = matches!(
            key_n.as_str(),
            "class"
                | "classname"
                | "style"
                | "id"
                | "data-region-id"
                | "hidden"
                | "disabled"
                | "checked"
                | "toggled"
                | "model-value"
                | "modelvalue"
                | "label"
                | "text"
                | "title"
                | "value"
                | "dir"
                | "src"
                | "data-src"
                | "poster"
                | "gap"
                | "padding"
                | "width"
                | "height"
                | "flex"
                | "flex-direction"
                | "flexdirection"
                | "flex-grow"
                | "flexgrow"
                | "min-width"
                | "minwidth"
                | "justify-content"
                | "justifycontent"
                | "overflow"
                | "overflow-y"
                | "overflowy"
                | "grid-template-columns"
                | "gridtemplatecolumns"
        ) || key_n.starts_with("data-")
            || is_common_svg_attr(key_n.as_str());
        let prev_dir = self.widgets.get(&id).map(|w| w.props.layout.dir);
        let restyle_has_ancestors = matches!(key_n.as_str(), "class" | "classname" | "id")
            && !self.cascade.has_args.is_empty();
        if restyle_has_ancestors {
            self.cascade.has_index_ready = false;
        }
        if full_rebuild {
            self.reapply_layout_for(id);
            if matches!(key_n.as_str(), "class" | "classname" | "id" | "style")
                || key_n.starts_with("data-")
            {
                self.reapply_relative_ancestors(id);
            }
        } else if let Some(widget) = self.widgets.get_mut(&id) {
            pin_svg_chart_min_height(&mut widget.props);
        }
        if let Some(root) = crate::svg_inline::nearest_svg_root(self, id)
            && !(full_rebuild && root == id)
        {
            self.reapply_layout_for(root);
        }
        // Parent size/padding change updates children's containing-block base.
        if matches!(key_n.as_str(), "style" | "width" | "height" | "padding") {
            let children = self
                .widgets
                .get(&id)
                .map(|w| w.children.clone())
                .unwrap_or_default();
            for child in children {
                self.sync_containing_block_from_parent(child);
            }
        }
        // Inherited `direction` is seeded onto children before cascade. A later
        // parent `direction: rtl` must recascade so logical edges remap.
        if full_rebuild {
            let next_dir = self.widgets.get(&id).map(|w| w.props.layout.dir);
            if prev_dir != next_dir {
                let mut dirty = HashSet::new();
                self.collect_subtree_ids(id, &mut dirty);
                dirty.remove(&id);
                let mut ordered: Vec<WidgetId> = dirty.into_iter().collect();
                ordered.sort_unstable();
                for child in ordered {
                    self.reapply_layout_for(child);
                }
            }
        }
        // Subject `:has()` on ancestors depends on this node's class / id.
        if restyle_has_ancestors {
            let mut walk = self.widgets.get(&id).and_then(|w| w.parent);
            while let Some(pid) = walk {
                self.reapply_layout_for(pid);
                walk = self.widgets.get(&pid).and_then(|w| w.parent);
            }
        }
        if matches!(
            key_n.as_str(),
            "checked" | "toggled" | "model-value" | "modelvalue" | "disabled"
        ) {
            self.reapply_following_siblings(id);
        }
        if matches!(key_n.as_str(), "label" | "text" | "title" | "value")
            && let Some(pid) = self.widgets.get(&id).and_then(|w| w.parent)
        {
            self.reapply_layout_for(pid);
        }
        self.strip_deferred_position_on_overlay(id);
        self.changed_widget(id);
    }

    pub fn set_label(&mut self, id: WidgetId, label: impl Into<String>) {
        let parent = self.widgets.get(&id).and_then(|w| w.parent);
        if let Some(w) = self.widgets.get_mut(&id) {
            w.props.label = label.into();
        } else {
            return;
        }
        self.reapply_layout_for(id);
        if let Some(pid) = parent {
            self.reapply_layout_for(pid);
        }
        self.changed_widget(id);
    }

    pub fn push_event(&mut self, event: BridgeEvent) {
        self.pending.push_back(event);
    }

    pub fn drain_events(&mut self) -> Vec<BridgeEvent> {
        self.pending.drain(..).collect()
    }

    pub fn peek_events(&self) -> impl Iterator<Item = &BridgeEvent> {
        self.pending.iter()
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn note_press(&mut self, id: WidgetId) -> Vec<&'static str> {
        let kind = match self.widgets.get(&id).map(|w| w.kind) {
            Some(k) => k,
            None => return Vec::new(),
        };
        match kind {
            WidgetKind::Button | WidgetKind::IconButton | WidgetKind::Chip => {
                self.push_event(BridgeEvent::Press { id });
                vec!["press", "click"]
            }
            WidgetKind::SidebarRow
            | WidgetKind::ListItem
            | WidgetKind::InteractiveCard
            | WidgetKind::TableRow => {
                self.push_event(BridgeEvent::Select { id });
                vec!["select", "click"]
            }
            _ => {
                self.push_event(BridgeEvent::Press { id });
                vec!["click"]
            }
        }
    }

    pub fn note_toggle(&mut self, id: WidgetId, value: bool) -> Vec<&'static str> {
        if let Some(w) = self.widgets.get_mut(&id) {
            w.props.toggled = value;
            // Overlays treat `active || toggled` as open; keep both in sync so
            // dismiss (close / outside / cancel) actually collapses when opened
            // via `active` / `open`.
            if w.kind.is_overlay() {
                w.props.active = value;
            }
            if value
                && crate::widget_map::is_checked_match_host(
                    w.kind,
                    w.props.element_tag.as_str(),
                    &w.props.attrs,
                )
            {
                w.props.attrs.insert("checked".into(), String::new());
            } else {
                w.props.attrs.remove("checked");
            }
        } else {
            return Vec::new();
        }
        self.reapply_layout_for(id);
        self.reapply_following_siblings(id);
        self.bump();
        self.push_event(BridgeEvent::Toggle { id, value });
        vec!["change", "update:modelValue"]
    }

    pub fn note_select(&mut self, id: WidgetId) -> Vec<&'static str> {
        if !self.widgets.contains_key(&id) {
            return Vec::new();
        }
        self.push_event(BridgeEvent::Select { id });
        vec!["select", "click"]
    }

    pub fn note_select_value(
        &mut self,
        id: WidgetId,
        value: impl Into<String>,
    ) -> Vec<&'static str> {
        let value = value.into();
        if let Some(w) = self.widgets.get_mut(&id) {
            w.props.value = value.clone();
            if w.kind.is_overlay() {
                // Confirm / Drawer footer / menu item selection closes the overlay.
                w.props.active = false;
                w.props.toggled = false;
            } else {
                w.props.active = true;
            }
            self.changed_widget(id);
        } else {
            return Vec::new();
        }
        self.push_event(BridgeEvent::SelectValue { id, value });
        vec!["select", "update:modelValue", "change"]
    }

    pub fn note_input(&mut self, id: WidgetId, value: impl Into<String>) -> Vec<&'static str> {
        let value = value.into();
        if let Some(w) = self.widgets.get_mut(&id) {
            w.props.value = value.clone();
            w.props.label = value.clone();
            self.changed_widget(id);
        } else {
            return Vec::new();
        }
        self.push_event(BridgeEvent::Input { id, value });
        vec!["input", "update:modelValue"]
    }

    pub fn note_change(&mut self, id: WidgetId, value: f64) -> Vec<&'static str> {
        if let Some(w) = self.widgets.get_mut(&id) {
            w.props.number = value as f32;
            w.props.progress = value as f32;
            w.props.value = value.to_string();
            self.changed_widget(id);
        } else {
            return Vec::new();
        }
        self.push_event(BridgeEvent::Change { id, value });
        vec!["change", "update:modelValue"]
    }

    pub fn snapshot(&mut self) -> SemanticSnapshot {
        let changes = std::mem::take(&mut self.changes);
        self.peek_snapshot_with(changes)
    }

    /// Read-only snapshot for scans that must not consume the mutation
    /// footprint (only the sync path may consume it).
    pub fn peek_snapshot(&self) -> SemanticSnapshot {
        self.peek_snapshot_with(self.changes.clone())
    }

    fn peek_snapshot_with(&self, changes: SnapshotChanges) -> SemanticSnapshot {
        let mut widgets = Vec::with_capacity(self.widgets.len());
        let mut seen = std::collections::HashSet::new();
        for &root in &self.roots {
            self.collect_preorder(root, &mut widgets, &mut seen);
        }
        for (&id, widget) in &self.widgets {
            if seen.insert(id) {
                widgets.push(widget.clone());
            }
        }
        SemanticSnapshot {
            revision: self.revision,
            theme: self.theme,
            appearance: self.appearance,
            roots: self.roots.clone(),
            widgets,
            changes,
        }
    }

    /// Map a tree tag to a widget kind (`nana-*` or HTML downlevel).
    pub fn kind_from_tag(tag: &str) -> Option<WidgetKind> {
        resolve_kind_from_hints(tag, None, None, None)
    }

    fn collect_preorder(
        &self,
        id: WidgetId,
        out: &mut Vec<SemanticWidget>,
        seen: &mut std::collections::HashSet<WidgetId>,
    ) {
        if !seen.insert(id) {
            return;
        }
        let Some(widget) = self.widgets.get(&id).cloned() else {
            return;
        };
        let children = widget.children.clone();
        out.push(widget);
        for child in children {
            self.collect_preorder(child, out, seen);
        }
    }

    /// Every revision bump must go through one of the `changed_*` helpers so
    /// the semantic sync can project only what mutated. Calling [`Self::bump`]
    /// directly silently loses the footprint and degrades to a full pass.
    fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.cascade.has_index_ready = false;
    }

    fn changed_widget(&mut self, id: WidgetId) {
        self.changes.dirty.insert(id);
        self.bump();
    }

    fn changed_widgets(&mut self, ids: impl IntoIterator<Item = WidgetId>) {
        self.changes.dirty.extend(ids);
        self.bump();
    }

    /// A widget changed in a way that can cascade onto its descendants
    /// (layout, kind, cascaded props): dirty the whole subtree.
    fn changed_subtree(&mut self, id: WidgetId) {
        let mut walk = vec![id];
        while let Some(current) = walk.pop() {
            self.changes.dirty.insert(current);
            if let Some(widget) = self.widgets.get(&current) {
                walk.extend(widget.children.iter().copied());
            }
        }
        self.bump();
    }

    /// Tree shape changed; the sync must reproject from the roots.
    fn changed_structure(&mut self) {
        self.changes.structure_changed = true;
        self.bump();
    }

    /// Whole-document invalidation (theme, global cascade, viewport).
    fn changed_all(&mut self) {
        self.changes.all = true;
        self.changes.dirty.clear();
        self.bump();
    }
}

fn push_has_args(compound: &crate::css_cascade::CompoundSelector, out: &mut Vec<SimpleCompound>) {
    for query in &compound.has_queries {
        for alt in query {
            if !out.contains(alt) && out.len() < 64 {
                out.push(alt.clone());
            }
        }
    }
}

/// Structural `<svg viewBox>` with author `height: Npx`: raise `min-height` so
/// column flex-shrink cannot crush chart geometry (heatmap weekday rows).
/// Horizontal crop stays with overflow:hidden + EndCrop — do not pin min-width.
/// `overflow-y: hidden` keeps CSS min-size:auto → 0 (may shrink).
fn pin_svg_chart_min_height(props: &mut WidgetProps) {
    if !props.element_tag.eq_ignore_ascii_case("svg") {
        return;
    }
    if props.layout.overflow_y.clips() {
        return;
    }
    let has_view_box = props.attrs.keys().any(|k| {
        let n: String = k
            .chars()
            .filter(|c| *c != '-' && *c != '_')
            .flat_map(|c| c.to_lowercase())
            .collect();
        n == "viewbox"
    });
    if !has_view_box {
        return;
    }
    let Some(LengthSpec::Px(h)) = props.layout.height else {
        return;
    };
    if !h.is_finite() || h <= 0.0 {
        return;
    }
    let raise = match props.layout.min_height {
        None => true,
        Some(LengthSpec::Px(mh)) => mh + 0.5 < h,
        _ => false,
    };
    if raise {
        props.layout.min_height = Some(LengthSpec::Px(h));
    }
}

fn is_sidebar_footer_slot(props: &WidgetProps) -> bool {
    props
        .class_names
        .iter()
        .any(|c| c == "nana-sidebar-frame__footer")
        || props
            .attrs
            .get("data-slot")
            .is_some_and(|s| s == "sidebar-footer")
}

fn hosts_sidebar_footer_content(
    w: &SemanticWidget,
    widgets: &std::collections::HashMap<WidgetId, SemanticWidget>,
) -> bool {
    w.props.class_names.iter().any(|c| c == "sb-footer")
        || w.props.agent_id.starts_with("sidebar.footer.")
        || w.children.iter().any(|cid| {
            widgets.get(cid).is_some_and(|c| {
                c.props.agent_id.starts_with("sidebar.footer.")
                    || c.props
                        .class_names
                        .iter()
                        .any(|cls| cls == "sb-footer" || cls.starts_with("sb-footer__"))
            })
        })
}

fn prefer_dense_newest(mut items: Vec<(WidgetId, usize, u64)>) -> Option<WidgetId> {
    items.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
    items.into_iter().next().map(|(id, _, _)| id)
}

/// Convenience: handle as widget id.
pub fn widget_id(handle: NodeHandle) -> WidgetId {
    handle.0
}

fn normalize_prop_key(key: &str) -> String {
    // Vue `.prop` / `^attr` modifiers arrive on the host key; strip first.
    let key = key.trim();
    let key = key
        .strip_prefix('.')
        .or_else(|| key.strip_prefix('^'))
        .unwrap_or(key);
    let key = key.replace('_', "-");
    let kebab = if key.chars().any(|c| c.is_ascii_uppercase()) {
        camel_to_kebab_simple(&key)
    } else {
        key.to_string()
    };
    kebab.to_ascii_lowercase()
}

fn persist_svg_length_attr(props: &mut WidgetProps, name: &str, value: &nana_js_engine::HostValue) {
    if !is_svg_length_tag(&props.element_tag) {
        return;
    }
    let s = match value {
        nana_js_engine::HostValue::Number(n) if n.is_finite() => {
            if n.fract() == 0.0 {
                format!("{}", *n as i64)
            } else {
                n.to_string()
            }
        }
        _ => host_string(value),
    };
    if s.is_empty() {
        props.attrs.remove(name);
    } else {
        props.attrs.insert(name.to_string(), s);
    }
}

fn is_svg_length_tag(tag: &str) -> bool {
    matches!(
        tag,
        "svg"
            | "g"
            | "path"
            | "rect"
            | "circle"
            | "ellipse"
            | "line"
            | "polyline"
            | "polygon"
            | "text"
            | "image"
            | "use"
            | "symbol"
            | "foreignobject"
            | "clippath"
            | "mask"
            | "lineargradient"
            | "radialgradient"
            | "stop"
            | "defs"
    )
}

/// Common SVG attrs after [`normalize_prop_key`] (kebab / lowercase).
fn is_common_svg_attr(key: &str) -> bool {
    matches!(
        key,
        "viewbox"
            | "view-box"
            | "preserveaspectratio"
            | "preserve-aspect-ratio"
            | "pathlength"
            | "path-length"
            | "cx"
            | "cy"
            | "r"
            | "rx"
            | "ry"
            | "x"
            | "y"
            | "x1"
            | "x2"
            | "y1"
            | "y2"
            | "points"
            | "transform"
            | "opacity"
            | "stroke-width"
            | "stroke-linecap"
            | "stroke-linejoin"
            | "stroke-dasharray"
            | "stroke-dashoffset"
            | "fill-opacity"
            | "stroke-opacity"
            | "fill-rule"
            | "clip-path"
            | "href"
            | "xmlns"
            | "d"
            | "fill"
            | "stroke"
            | "width"
            | "height"
            | "offset"
            | "stop-color"
            | "stopcolor"
            | "stop-opacity"
            | "stopopacity"
            | "gradienttransform"
            | "gradient-transform"
            | "gradientunits"
            | "gradient-units"
            | "spreadmethod"
            | "spread-method"
            | "font-size"
            | "font-family"
            | "text-anchor"
            | "dominant-baseline"
            | "overflow"
    )
}

fn camel_to_kebab_simple(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    for (i, ch) in input.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn host_string(value: &nana_js_engine::HostValue) -> String {
    match value {
        nana_js_engine::HostValue::Null | nana_js_engine::HostValue::Undefined => String::new(),
        nana_js_engine::HostValue::Bool(v) => v.to_string(),
        nana_js_engine::HostValue::Number(v) => {
            if v.fract() == 0.0 && v.is_finite() {
                format!("{}", *v as i64)
            } else {
                v.to_string()
            }
        }
        nana_js_engine::HostValue::String(v) => v.clone(),
        nana_js_engine::HostValue::Object(map) => {
            // Vue sometimes passes option/chip props as objects; prefer human labels.
            for key in ["label", "name", "title", "text", "value", "id"] {
                if let Some(v) = map.get(key) {
                    let s = host_string(v);
                    if !s.is_empty() && s != "[object Object]" {
                        return s;
                    }
                }
            }
            String::new()
        }
        nana_js_engine::HostValue::Array(items) => items
            .iter()
            .map(host_string)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        other => {
            let s = other.to_json_string();
            if s == "[object Object]" || s.starts_with('{') {
                String::new()
            } else {
                s
            }
        }
    }
}

#[derive(Debug, Clone)]
struct MatchNodeSnap {
    tag: String,
    id: String,
    classes: Vec<String>,
    attrs: BTreeMap<String, String>,
    is_empty: bool,
    checked: bool,
}

impl MatchNodeSnap {
    fn as_node(&self) -> MatchNode<'_> {
        MatchNode {
            tag: self.tag.as_str(),
            id: self.id.as_str(),
            classes: self.classes.as_slice(),
            attrs: &self.attrs,
            is_empty: self.is_empty,
            checked: self.checked,
        }
    }
}

fn widget_checked_state(w: &SemanticWidget) -> bool {
    crate::widget_map::is_checked_match_host(w.kind, w.props.element_tag.as_str(), &w.props.attrs)
        && w.props.toggled
}

fn cascade_attrs_from_widget(w: &SemanticWidget) -> BTreeMap<String, String> {
    let mut attrs = w.props.attrs.clone();
    if w.props.disabled {
        attrs.entry("disabled".into()).or_default();
    }
    if widget_checked_state(w) {
        attrs.entry("checked".into()).or_default();
    } else {
        attrs.remove("checked");
    }
    attrs
}

fn text_has_non_whitespace(s: &str) -> bool {
    s.chars().any(|c| !c.is_whitespace())
}

fn match_snap_from_widget(w: &SemanticWidget, is_empty: bool) -> MatchNodeSnap {
    MatchNodeSnap {
        tag: if w.props.element_tag.is_empty() {
            w.kind.element_tag().to_string()
        } else {
            w.props.element_tag.clone()
        },
        id: w.props.element_id.clone(),
        classes: w.props.class_names.clone(),
        attrs: cascade_attrs_from_widget(w),
        is_empty,
        checked: widget_checked_state(w),
    }
}

fn stylesheet_uses_focus_within(
    static_rules: &[StyleRule],
    interactive: &[InteractiveStyleRule],
    generated: &[GeneratedPseudoRule],
) -> bool {
    static_rules
        .iter()
        .any(|rule| rule.selectors.iter().any(|sel| sel.subject.focus_within))
        || interactive
            .iter()
            .any(|rule| rule.selector.subject.focus_within)
        || generated
            .iter()
            .any(|rule| rule.originating_selector.subject.focus_within)
}

fn host_style_to_css_text(value: &nana_js_engine::HostValue) -> String {
    match value {
        nana_js_engine::HostValue::Object(map) => {
            let mut out = String::with_capacity(map.len().saturating_mul(24));
            for (key, v) in map {
                let prop = if key.starts_with("--") || key.contains('-') {
                    key.clone()
                } else {
                    camel_to_kebab_simple(key)
                };
                let val = host_string(v);
                if prop.is_empty() || val.is_empty() {
                    continue;
                }
                out.push_str(&prop);
                out.push(':');
                out.push_str(&val);
                out.push(';');
            }
            out
        }
        _ => host_string(value),
    }
}

/// True when a CSS declaration list mentions `property` (case-insensitive).
fn css_decl_mentions(style: &str, property: &str) -> bool {
    let want = property.trim();
    if want.is_empty() || style.trim().is_empty() {
        return false;
    }
    for decl in style.split(';') {
        let decl = decl.trim();
        let Some((name, _)) = decl.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(want) {
            return true;
        }
    }
    false
}

fn host_truthy(value: &nana_js_engine::HostValue) -> bool {
    match value {
        nana_js_engine::HostValue::Null | nana_js_engine::HostValue::Undefined => false,
        nana_js_engine::HostValue::Bool(v) => *v,
        nana_js_engine::HostValue::Number(n) => *n != 0.0,
        nana_js_engine::HostValue::String(s) => {
            let s = s.trim();
            !(s.is_empty() || s.eq_ignore_ascii_case("false") || s == "0")
        }
        _ => true,
    }
}

/// Open/close flag for overlay props — not a select / confirm string `model-value`.
fn host_is_open_flag(value: &nana_js_engine::HostValue) -> bool {
    match value {
        nana_js_engine::HostValue::Bool(_) | nana_js_engine::HostValue::Number(_) => true,
        nana_js_engine::HostValue::Null | nana_js_engine::HostValue::Undefined => true,
        nana_js_engine::HostValue::String(s) => {
            let s = s.trim();
            s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false")
        }
        _ => false,
    }
}

/// Host Teleport / `v-if` surfaces mount only while open and often omit `open`/`active`.
/// Presence cues (`aria-modal`, `is-open` / `is-active`, `data-nana-open`) imply Nana
/// Overlay open — without inventing CSS `fixed`/`sticky`. Explicit `open`/`active`/
/// `toggled` still win via later patches. Do **not** key off bare `nana-*` / `role`
/// alone — closed Nana* wrappers stay mounted with those hints. Do **not** key off
/// product kit BEM (`ui-dialog`, `ctx-menu`, …).
fn apply_overlay_presence_open(props: &mut WidgetProps) {
    if props.active || props.toggled {
        return;
    }
    if overlay_presence_implies_open(props) {
        props.active = true;
        props.toggled = true;
    }
}

fn overlay_presence_implies_open(props: &WidgetProps) -> bool {
    props.class_names.iter().any(|class| {
        let class = class.to_ascii_lowercase();
        class == "is-open" || class == "is-active"
    }) || props
        .attrs
        .get("aria-expanded")
        .is_some_and(|expanded| expanded.eq_ignore_ascii_case("true"))
        || props.attrs.get("aria-modal").is_some_and(|modal| {
            // Empty string = boolean true attribute; "true" likewise.
            modal.is_empty() || modal.eq_ignore_ascii_case("true")
        })
        || props
            .attrs
            .get("data-nana-open")
            .is_some_and(|open| open.is_empty() || open.eq_ignore_ascii_case("true"))
        || props.attrs.get("open").is_some_and(|open| {
            open.is_empty()
                || open.eq_ignore_ascii_case("true")
                || open.eq_ignore_ascii_case("open")
        })
}

fn encode_qr_modules_attr(value: &nana_js_engine::HostValue) -> String {
    match value {
        nana_js_engine::HostValue::Array(items) => items
            .iter()
            .map(|item| {
                if matches!(item, nana_js_engine::HostValue::Number(n) if *n != 0.0)
                    || host_truthy(item)
                    || host_string(item) == "1"
                {
                    "1"
                } else {
                    "0"
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        _ => host_string(value),
    }
}

fn host_f32(value: &nana_js_engine::HostValue, default: f32) -> f32 {
    match value {
        nana_js_engine::HostValue::Number(n) if n.is_finite() => *n as f32,
        nana_js_engine::HostValue::String(s) => s.trim().parse().unwrap_or(default),
        _ => default,
    }
}

fn parse_option_item(item: &nana_js_engine::HostValue) -> Option<SelectOptionProp> {
    match item {
        nana_js_engine::HostValue::Object(map) => {
            let value = map
                .get("value")
                .or_else(|| map.get("key"))
                .map(host_string)
                .unwrap_or_default();
            let label = map
                .get("label")
                .map(host_string)
                .filter(|s| !s.is_empty() && s != "[object Object]")
                .unwrap_or_else(|| value.clone());
            let disabled = map.get("disabled").map(host_truthy).unwrap_or(false);
            if (value.is_empty() && label.is_empty())
                || value == "[object Object]"
                || label == "[object Object]"
            {
                None
            } else {
                Some(SelectOptionProp {
                    value,
                    label,
                    disabled,
                })
            }
        }
        nana_js_engine::HostValue::String(s) => {
            if s.is_empty() || s == "[object Object]" {
                None
            } else {
                Some(SelectOptionProp {
                    value: s.clone(),
                    label: s.clone(),
                    disabled: false,
                })
            }
        }
        _ => None,
    }
}

fn parse_options(value: &nana_js_engine::HostValue) -> Vec<SelectOptionProp> {
    match value {
        nana_js_engine::HostValue::Array(items) => {
            items.iter().filter_map(parse_option_item).collect()
        }
        nana_js_engine::HostValue::Object(map) => {
            // Numeric-key object (reactive array shape) → options list.
            let mut indexed = map
                .iter()
                .filter_map(|(k, v)| k.parse::<usize>().ok().map(|i| (i, v)))
                .collect::<Vec<_>>();
            if !indexed.is_empty() {
                indexed.sort_by_key(|(i, _)| *i);
                return indexed
                    .into_iter()
                    .filter_map(|(_, v)| parse_option_item(v))
                    .collect();
            }
            parse_option_item(value).into_iter().collect()
        }
        nana_js_engine::HostValue::String(s) => {
            // Reject Array.prototype.toString of object options.
            if s.contains("[object Object]") {
                return Vec::new();
            }
            // Comma-separated "value:label" or bare values.
            s.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty() && *p != "[object Object]")
                .map(|part| {
                    if let Some((v, l)) = part.split_once(':') {
                        SelectOptionProp {
                            value: v.trim().to_string(),
                            label: l.trim().to_string(),
                            disabled: false,
                        }
                    } else {
                        SelectOptionProp {
                            value: part.to_string(),
                            label: part.to_string(),
                            disabled: false,
                        }
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

pub fn parse_button_kind(raw: &str) -> Option<ButtonKind> {
    Some(match raw.trim().to_ascii_lowercase().as_str() {
        "ghost" => ButtonKind::Ghost,
        "subtle" => ButtonKind::Subtle,
        "selected" => ButtonKind::Selected,
        "primary" => ButtonKind::Primary,
        "warning" => ButtonKind::Warning,
        "danger" => ButtonKind::Danger,
        "text" => ButtonKind::Text,
        "menu" => ButtonKind::Menu,
        _ => return None,
    })
}

pub fn parse_card_kind(raw: &str) -> Option<CardKind> {
    Some(match raw.trim().to_ascii_lowercase().as_str() {
        "surface" => CardKind::Surface,
        "outlined" | "outline" => CardKind::Outlined,
        "raised" | "elevated" => CardKind::Raised,
        "flat" => CardKind::Flat,
        "selected" => CardKind::Selected,
        _ => return None,
    })
}

pub fn parse_control_size(raw: &str) -> Option<ControlSize> {
    Some(match raw.trim().to_ascii_lowercase().as_str() {
        "small" | "sm" => ControlSize::Small,
        "medium" | "md" => ControlSize::Medium,
        "large" | "lg" => ControlSize::Large,
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
