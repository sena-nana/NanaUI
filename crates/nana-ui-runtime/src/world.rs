use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bevy_ecs::component::{Component, Mutable};
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use nana_ui_core::{StyleModelRef, SwitchControlPosition, ThemeMode};

use crate::animation::ActiveAnimation;
use crate::schedule::{DirtyMask, SystemWork, push_work};
use crate::{
    AccessibilityDelta, AccessibilityNode, AccessibilityRole, AccessibilityState, AnimationFrame,
    AnimationId, AnimationSpec, ComputedStyle, CustomRenderNode, EventRoute, ExtractedNode,
    ImeComposition, InteractionState, LayoutBox, LayoutInput, MutationQueue, NodeStyle,
    OverlayHostState, PointerCaptureChange, ScrollMetrics, ScrollOffset, StandardVisual,
    TextContent, TextInputPresentation, TextInputState, TextMetrics, TextShaper, UiMutation,
};

/// Stable external node identity. Zero is reserved so missing/default IDs
/// cannot accidentally address a live node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StableNodeId(u64);

impl StableNodeId {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Stable document/window ownership boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentId(u64);

impl DocumentId {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Element { tag: String },
    Text,
    Comment,
}

#[derive(Component)]
struct Identity {
    stable: StableNodeId,
    document: DocumentId,
}

#[derive(Component)]
struct Kind(NodeKind);

#[derive(Component, Default)]
struct Hierarchy {
    parent: Option<StableNodeId>,
    children: Vec<StableNodeId>,
}

#[derive(Debug, Clone)]
struct HitEntry {
    id: StableNodeId,
    layout: LayoutBox,
    transform: [f32; 6],
    clips: Vec<(LayoutBox, [f32; 6])>,
    z_index: i32,
    order: usize,
}

impl HitEntry {
    fn contains(&self, x: f32, y: f32) -> bool {
        transformed_contains(self.layout, self.transform, x, y)
            && self
                .clips
                .iter()
                .all(|(bounds, transform)| transformed_contains(*bounds, *transform, x, y))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSnapshot {
    pub id: StableNodeId,
    pub document: DocumentId,
    pub kind: NodeKind,
    pub parent: Option<StableNodeId>,
    pub children: Vec<StableNodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitReport {
    pub generation: u64,
    pub mutations: usize,
    pub created: usize,
    pub inserted: usize,
    pub detached: usize,
    pub reparented: usize,
    pub despawned: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiWorldError {
    DuplicateNode(StableNodeId),
    RetiredNode(StableNodeId),
    MissingNode(StableNodeId),
    CrossDocument {
        parent: StableNodeId,
        child: StableNodeId,
    },
    FocusDocument {
        document: DocumentId,
        target: StableNodeId,
    },
    PointerDocument {
        document: DocumentId,
        target: StableNodeId,
    },
    Cycle {
        parent: StableNodeId,
        child: StableNodeId,
    },
    InvalidBefore {
        parent: StableNodeId,
        before: StableNodeId,
    },
    InvalidStyle(StableNodeId),
    InvalidText(StableNodeId),
    InvalidLayout(StableNodeId),
    InvalidScrollOffset(StableNodeId),
    InvalidScrollMetrics(StableNodeId),
    InvalidIme(StableNodeId),
    InvalidCustomRender(StableNodeId),
    InvalidStandardVisual(StableNodeId),
    InvalidOverlayHost(StableNodeId),
    NotFocusable(StableNodeId),
    NotPointerInteractive(StableNodeId),
    NotFocused(StableNodeId),
    PointerCaptureMismatch {
        pointer_id: u64,
        target: StableNodeId,
    },
    InvalidAnimation(AnimationId),
    MissingAnimation(AnimationId),
    InvalidTextInput(StableNodeId),
    MissingTextInput(StableNodeId),
}

impl fmt::Display for UiWorldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNode(id) => write!(formatter, "node {} already exists", id.get()),
            Self::RetiredNode(id) => write!(formatter, "node {} was retired", id.get()),
            Self::MissingNode(id) => write!(formatter, "node {} does not exist", id.get()),
            Self::CrossDocument { parent, child } => write!(
                formatter,
                "cannot parent node {} under node {} from another document",
                child.get(),
                parent.get()
            ),
            Self::FocusDocument { document, target } => write!(
                formatter,
                "node {} does not belong to document {}",
                target.get(),
                document.get()
            ),
            Self::PointerDocument { document, target } => write!(
                formatter,
                "pointer target {} does not belong to document {}",
                target.get(),
                document.get()
            ),
            Self::Cycle { parent, child } => write!(
                formatter,
                "parenting node {} under node {} would create a cycle",
                child.get(),
                parent.get()
            ),
            Self::InvalidBefore { parent, before } => write!(
                formatter,
                "node {} is not a child of parent {}",
                before.get(),
                parent.get()
            ),
            Self::InvalidStyle(id) => write!(formatter, "node {} has an invalid style", id.get()),
            Self::InvalidText(id) => {
                write!(formatter, "node {} has invalid text metrics", id.get())
            }
            Self::InvalidLayout(id) => {
                write!(formatter, "node {} has an invalid layout box", id.get())
            }
            Self::InvalidScrollOffset(id) => {
                write!(formatter, "node {} has an invalid scroll offset", id.get())
            }
            Self::InvalidScrollMetrics(id) => {
                write!(formatter, "node {} has invalid scroll metrics", id.get())
            }
            Self::InvalidIme(id) => write!(formatter, "node {} has an invalid IME range", id.get()),
            Self::InvalidCustomRender(id) => {
                write!(
                    formatter,
                    "node {} has invalid custom render content",
                    id.get()
                )
            }
            Self::InvalidStandardVisual(id) => {
                write!(
                    formatter,
                    "node {} has invalid standard visual state",
                    id.get()
                )
            }
            Self::InvalidOverlayHost(id) => {
                write!(formatter, "node {} has an invalid active overlay", id.get())
            }
            Self::NotFocusable(id) => write!(formatter, "node {} cannot receive focus", id.get()),
            Self::NotPointerInteractive(id) => {
                write!(formatter, "node {} cannot receive pointer input", id.get())
            }
            Self::NotFocused(id) => write!(formatter, "node {} is not focused", id.get()),
            Self::PointerCaptureMismatch { pointer_id, target } => write!(
                formatter,
                "pointer {pointer_id} is not captured by node {}",
                target.get()
            ),
            Self::InvalidAnimation(id) => write!(formatter, "animation {} is invalid", id.get()),
            Self::MissingAnimation(id) => {
                write!(formatter, "animation {} is not active", id.get())
            }
            Self::InvalidTextInput(id) => {
                write!(formatter, "node {} has invalid text input state", id.get())
            }
            Self::MissingTextInput(id) => {
                write!(formatter, "node {} has no text input state", id.get())
            }
        }
    }
}

impl std::error::Error for UiWorldError {}

#[derive(Debug, Clone)]
struct PlannedNode {
    document: DocumentId,
    parent: Option<StableNodeId>,
    children: Vec<StableNodeId>,
}

/// The sole authoritative retained identity and hierarchy store.
pub struct UiWorld {
    world: World,
    entities: HashMap<StableNodeId, Entity>,
    retired: HashSet<StableNodeId>,
    dirty_entities: HashSet<StableNodeId>,
    focused: HashMap<DocumentId, StableNodeId>,
    hit_test_index: HashMap<DocumentId, Vec<HitEntry>>,
    pointer_captures: HashMap<(DocumentId, u64), StableNodeId>,
    pointer_hover: HashMap<(DocumentId, u64), StableNodeId>,
    pointer_press: HashMap<(DocumentId, u64), StableNodeId>,
    pending_pointer_capture_changes: Vec<PointerCaptureChange>,
    pending_render_removals: Vec<StableNodeId>,
    pending_accessibility_removals: Vec<StableNodeId>,
    animations: HashMap<AnimationId, ActiveAnimation>,
    animation_deadlines: BTreeSet<(Duration, AnimationId)>,
    style_model: StyleModelRef,
    generation: u64,
}

impl Default for UiWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl UiWorld {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            entities: HashMap::new(),
            retired: HashSet::new(),
            dirty_entities: HashSet::new(),
            focused: HashMap::new(),
            hit_test_index: HashMap::new(),
            pointer_captures: HashMap::new(),
            pointer_hover: HashMap::new(),
            pointer_press: HashMap::new(),
            pending_pointer_capture_changes: Vec::new(),
            pending_render_removals: Vec::new(),
            pending_accessibility_removals: Vec::new(),
            animations: HashMap::new(),
            animation_deadlines: BTreeSet::new(),
            style_model: StyleModelRef::default(),
            generation: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn contains(&self, id: StableNodeId) -> bool {
        self.entities.contains_key(&id)
    }

    pub fn is_retired(&self, id: StableNodeId) -> bool {
        self.retired.contains(&id)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn theme_mode(&self) -> ThemeMode {
        self.style_model.theme_mode
    }

    pub fn event_route(&self, target: StableNodeId) -> Option<EventRoute> {
        let mut bubble = Vec::new();
        let mut current = self.node(target)?.parent;
        while let Some(id) = current {
            bubble.push(id);
            current = self.node(id)?.parent;
        }
        let mut capture = bubble.clone();
        capture.reverse();
        Some(EventRoute {
            capture,
            target,
            bubble,
        })
    }

    pub fn pointer_capture(&self, document: DocumentId, pointer_id: u64) -> Option<StableNodeId> {
        self.pointer_captures.get(&(document, pointer_id)).copied()
    }

    pub fn pointer_captures(&self, document: DocumentId) -> Vec<(u64, StableNodeId)> {
        let mut captures = self
            .pointer_captures
            .iter()
            .filter_map(|(&(owner, pointer_id), &target)| {
                (owner == document).then_some((pointer_id, target))
            })
            .collect::<Vec<_>>();
        captures.sort_unstable_by_key(|(pointer_id, _)| *pointer_id);
        captures
    }

    pub fn take_pointer_capture_changes(&mut self) -> Vec<PointerCaptureChange> {
        std::mem::take(&mut self.pending_pointer_capture_changes)
    }

    pub fn pointer_hover(&self, document: DocumentId, pointer_id: u64) -> Option<StableNodeId> {
        self.pointer_hover.get(&(document, pointer_id)).copied()
    }

    pub fn pointer_press(&self, document: DocumentId, pointer_id: u64) -> Option<StableNodeId> {
        self.pointer_press.get(&(document, pointer_id)).copied()
    }

    /// Update per-pointer hover authority and invalidate only the old/new
    /// interaction paint. DOM adapters may derive enter/leave paths from the
    /// returned previous target and the retained hierarchy.
    pub fn set_pointer_hover(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: Option<StableNodeId>,
    ) -> Result<Option<StableNodeId>, UiWorldError> {
        if let Some(target) = target {
            self.validate_pointer_target(document, target)?;
        }
        let key = (document, pointer_id);
        let previous = match target {
            Some(target) => self.pointer_hover.insert(key, target),
            None => self.pointer_hover.remove(&key),
        };
        if previous != target {
            self.generation = self.generation.wrapping_add(1);
            if let Some(previous) = previous {
                self.mark_interaction_style(previous);
            }
            if let Some(target) = target {
                self.mark_interaction_style(target);
            }
        }
        Ok(previous)
    }

    pub fn press_pointer(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
        target: StableNodeId,
    ) -> Result<Option<StableNodeId>, UiWorldError> {
        self.validate_pointer_target(document, target)?;
        let previous = self.pointer_press.insert((document, pointer_id), target);
        if previous != Some(target) {
            self.generation = self.generation.wrapping_add(1);
            if let Some(previous) = previous {
                self.mark_interaction_style(previous);
            }
            self.mark_interaction_style(target);
        }
        Ok(previous)
    }

    pub fn release_pointer_press(
        &mut self,
        document: DocumentId,
        pointer_id: u64,
    ) -> Option<StableNodeId> {
        let previous = self.pointer_press.remove(&(document, pointer_id));
        if let Some(previous) = previous {
            self.generation = self.generation.wrapping_add(1);
            self.mark_interaction_style(previous);
        }
        previous
    }

    pub fn clear_pointer_interactions(&mut self, document: DocumentId) {
        let affected = self
            .pointer_hover
            .iter()
            .chain(&self.pointer_press)
            .filter_map(|(&(owner, _), &target)| (owner == document).then_some(target))
            .collect::<HashSet<_>>();
        self.pointer_hover
            .retain(|(owner, _), _| *owner != document);
        self.pointer_press
            .retain(|(owner, _), _| *owner != document);
        if !affected.is_empty() {
            self.generation = self.generation.wrapping_add(1);
            for target in affected {
                self.mark_interaction_style(target);
            }
        }
    }

    pub fn next_animation_deadline(&self) -> Option<Duration> {
        self.animation_deadlines
            .first()
            .map(|(deadline, _)| *deadline)
    }

    /// Sample only active animations that are due at `now`. This method does
    /// not mark render state dirty: consumers apply sampled values through the
    /// normal atomic mutation boundary.
    pub fn advance_animations(&mut self, now: Duration) -> AnimationFrame {
        let due = self
            .animation_deadlines
            .range(..=(now, AnimationId::new(u64::MAX).expect("max ID is nonzero")))
            .copied()
            .collect::<Vec<_>>();
        let mut samples = Vec::with_capacity(due.len());
        for (deadline, id) in due {
            self.animation_deadlines.remove(&(deadline, id));
            let (sample, next_deadline) = {
                let animation = self
                    .animations
                    .get_mut(&id)
                    .expect("due animation must remain active");
                let sample = animation
                    .sample(now)
                    .expect("due animation must produce a sample");
                let next_deadline = (!sample.finished).then_some(animation.next_deadline);
                (sample, next_deadline)
            };
            if sample.finished {
                self.animations.remove(&id);
            } else if let Some(next_deadline) = next_deadline {
                self.animation_deadlines.insert((next_deadline, id));
            }
            samples.push(sample);
        }
        AnimationFrame {
            samples,
            component_updates: Vec::new(),
            next_deadline: self.next_animation_deadline(),
        }
    }

    /// Drain dirty components into deterministic system work. Calling this on
    /// an unchanged world returns an empty work set and performs no scheduling.
    pub fn take_system_work(&mut self) -> SystemWork {
        let mut ids = std::mem::take(&mut self.dirty_entities)
            .into_iter()
            .collect::<Vec<_>>();
        ids.sort_unstable();
        let mut work = SystemWork {
            generation: self.generation,
            style: Vec::new(),
            text: Vec::new(),
            layout: Vec::new(),
            input_hit_test: Vec::new(),
            focus_ime: Vec::new(),
            accessibility: Vec::new(),
            accessibility_removals: std::mem::take(&mut self.pending_accessibility_removals),
            render_extraction: Vec::new(),
            render_removals: std::mem::take(&mut self.pending_render_removals),
        };
        work.render_removals.sort_unstable();
        work.accessibility_removals.sort_unstable();
        for id in ids {
            let entity = self.entities[&id];
            let bits = self
                .world
                .get_mut::<DirtyMask>(entity)
                .expect("entity must have dirty component")
                .take();
            let has_text = matches!(self.component::<Kind>(id).0, NodeKind::Text)
                || !self.component::<TextContent>(id).value.is_empty();
            let bits = if has_text {
                bits
            } else {
                bits & !DirtyMask::TEXT
            };
            push_work(&mut work, id, bits);
        }
        work
    }

    /// Restore drained work after a frame-system failure. Derived writes are
    /// idempotent, so retrying the complete transaction is safer than losing
    /// accessibility or render invalidations from an earlier pass.
    pub fn restore_system_work(&mut self, work: SystemWork) {
        for (ids, bit) in [
            (work.style, DirtyMask::STYLE),
            (work.text, DirtyMask::TEXT),
            (work.layout, DirtyMask::LAYOUT),
            (work.input_hit_test, DirtyMask::INPUT),
            (work.focus_ime, DirtyMask::FOCUS_IME),
            (work.accessibility, DirtyMask::ACCESSIBILITY),
            (work.render_extraction, DirtyMask::RENDER),
        ] {
            for id in ids {
                let Some(&entity) = self.entities.get(&id) else {
                    continue;
                };
                self.world
                    .get_mut::<DirtyMask>(entity)
                    .expect("retained node must have dirty state")
                    .insert(bit);
                self.dirty_entities.insert(id);
            }
        }
        self.pending_accessibility_removals
            .extend(work.accessibility_removals);
        self.pending_accessibility_removals.sort_unstable();
        self.pending_accessibility_removals.dedup();
        self.pending_render_removals.extend(work.render_removals);
        self.pending_render_removals.sort_unstable();
        self.pending_render_removals.dedup();
    }

    pub fn node(&self, id: StableNodeId) -> Option<NodeSnapshot> {
        let entity = *self.entities.get(&id)?;
        let identity = self.world.get::<Identity>(entity)?;
        let kind = self.world.get::<Kind>(entity)?;
        let hierarchy = self.world.get::<Hierarchy>(entity)?;
        Some(NodeSnapshot {
            id: identity.stable,
            document: identity.document,
            kind: kind.0.clone(),
            parent: hierarchy.parent,
            children: hierarchy.children.clone(),
        })
    }

    pub fn focused(&self, document: DocumentId) -> Option<StableNodeId> {
        self.focused.get(&document).copied()
    }

    pub fn focused_text_input(
        &self,
        document: DocumentId,
    ) -> Option<(StableNodeId, &TextInputState)> {
        let id = self.focused(document)?;
        Some((id, self.text_input(id)?))
    }

    pub fn text(&self, id: StableNodeId) -> Option<&str> {
        let entity = *self.entities.get(&id)?;
        self.world
            .get::<TextContent>(entity)
            .map(|text| text.value.as_str())
    }

    pub fn layout_box(&self, id: StableNodeId) -> Option<LayoutBox> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<LayoutBox>(entity).copied()
    }

    pub fn scroll_offset(&self, id: StableNodeId) -> Option<ScrollOffset> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<ScrollOffset>(entity).copied()
    }

    pub fn scroll_metrics(&self, id: StableNodeId) -> Option<ScrollMetrics> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<ScrollMetrics>(entity).copied()
    }

    pub fn clamp_scroll_offset(&self, id: StableNodeId, offset: ScrollOffset) -> ScrollOffset {
        self.scroll_metrics(id)
            .map_or(offset, |metrics| metrics.clamp(offset))
    }

    pub fn node_style(&self, id: StableNodeId) -> Option<&NodeStyle> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<NodeStyle>(entity)
    }

    pub fn interaction(&self, id: StableNodeId) -> Option<InteractionState> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<InteractionState>(entity).copied()
    }

    pub fn text_input(&self, id: StableNodeId) -> Option<&TextInputState> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<TextInputState>(entity)
    }

    pub fn text_metrics(&self, id: StableNodeId) -> Option<TextMetrics> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<TextMetrics>(entity).copied()
    }

    pub fn ime(&self, id: StableNodeId) -> Option<&ImeComposition> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<ImeComposition>(entity)
    }

    pub fn custom_render(&self, id: StableNodeId) -> Option<&CustomRenderNode> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<CustomRenderNode>(entity)
    }

    pub fn standard_visual(&self, id: StableNodeId) -> Option<StandardVisual> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<StandardVisual>(entity).cloned()
    }

    pub fn component_geometry(&self, id: StableNodeId) -> Option<crate::ComponentGeometry> {
        let entity = *self.entities.get(&id)?;
        let visual = self.world.get::<StandardVisual>(entity)?;
        let style = self.world.get::<ComputedStyle>(entity)?;
        self.derive_component_geometry(id, visual, style)
    }

    pub fn accessibility(&self, id: StableNodeId) -> Option<&AccessibilityState> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<AccessibilityState>(entity)
    }

    pub fn overlay_host(&self, id: StableNodeId) -> Option<OverlayHostState> {
        let entity = *self.entities.get(&id)?;
        self.world.get::<OverlayHostState>(entity).copied()
    }

    /// Project the visible accessibility tree from the same retained authority.
    pub fn project_accessibility(&self, document: DocumentId) -> Vec<AccessibilityNode> {
        self.document_order(document)
            .into_iter()
            .filter_map(|id| self.project_accessibility_node(id))
            .collect()
    }

    /// Project only accessibility nodes named by scheduled dirty work.
    pub fn project_accessibility_nodes(&self, ids: &[StableNodeId]) -> Vec<AccessibilityNode> {
        ids.iter()
            .filter_map(|&id| self.project_accessibility_node(id))
            .collect()
    }

    /// Project one complete incremental accessibility transaction, including
    /// tombstones for nodes removed from the retained world.
    pub fn project_accessibility_delta(&self, work: &SystemWork) -> AccessibilityDelta {
        let mut removed = work
            .accessibility_removals
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        removed.extend(work.accessibility.iter().copied().filter(|id| {
            self.entities.contains_key(id) && self.project_accessibility_node(*id).is_none()
        }));
        AccessibilityDelta {
            generation: work.generation,
            updated: self.project_accessibility_nodes(&work.accessibility),
            removed: removed.into_iter().collect(),
        }
    }

    /// Resolve inherited visual state for dirty nodes. Parent state is always
    /// resolved before its descendants, independent of stable ID order.
    pub fn resolve_styles(&mut self, ids: &[StableNodeId]) -> Result<(), UiWorldError> {
        let mut resolved = HashSet::new();
        for &id in ids {
            self.resolve_style(id, &mut resolved)?;
        }
        self.reconcile_focus(ids);
        Ok(())
    }

    /// Drop focus and composition when dirty visual or interaction state makes
    /// the focused node ineligible.
    pub fn reconcile_focus(&mut self, ids: &[StableNodeId]) {
        let dirty = ids.iter().copied().collect::<HashSet<_>>();
        let invalid_focus = self
            .focused
            .iter()
            .filter_map(|(&document, &id)| {
                let invalid = dirty.contains(&id)
                    && (!self.component::<ComputedStyle>(id).visible
                        || !self.component::<InteractionState>(id).focusable);
                invalid.then_some((document, id))
            })
            .collect::<Vec<_>>();
        for (document, id) in invalid_focus {
            self.focused.remove(&document);
            self.remove_ime(id);
            self.mark(
                id,
                DirtyMask::FOCUS_IME | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
            );
        }
    }

    /// Shape only explicitly scheduled text. The runtime owns invalidation and
    /// storage while the renderer adapter supplies its real shaping backend.
    pub fn shape_text(
        &mut self,
        ids: &[StableNodeId],
        shaper: &mut impl TextShaper,
    ) -> Result<(), UiWorldError> {
        let mut shaped = Vec::with_capacity(ids.len());
        for &id in ids {
            if !self.contains(id) {
                return Err(UiWorldError::MissingNode(id));
            }
            let presentation = self.text_input_presentation_source(id);
            let text = presentation.as_ref().map_or_else(
                || self.component::<TextContent>(id).clone(),
                |source| source.text.clone(),
            );
            let style = self.component::<ComputedStyle>(id).clone();
            let metrics = shaper.shape(
                id,
                &text,
                &style,
                crate::TextShapeConstraints {
                    shaping: self.text_shaping(id),
                    ..crate::TextShapeConstraints::default()
                },
            );
            validate_text_metrics(id, metrics)?;
            let presentation = presentation
                .map(|source| shape_text_input_presentation(id, source, &style, shaper));
            shaped.push((id, metrics, presentation));
        }
        for (id, metrics, presentation) in shaped {
            *self.component_mut::<TextMetrics>(id) = metrics;
            if let Some(presentation) = presentation {
                self.world
                    .entity_mut(self.entities[&id])
                    .insert(presentation);
            }
        }
        Ok(())
    }

    /// Re-shape visible text against its resolved content box after the first
    /// layout pass. This closes wrapping/ellipsis height measurement without
    /// moving layout ownership into the renderer adapter.
    pub fn shape_text_for_layout(
        &mut self,
        document: DocumentId,
        shaper: &mut impl TextShaper,
    ) -> Result<bool, UiWorldError> {
        let mut shaped = Vec::new();
        for id in self.document_order(document) {
            let presentation = self.text_input_presentation_source(id);
            let text = presentation.as_ref().map_or_else(
                || self.component::<TextContent>(id).clone(),
                |source| source.text.clone(),
            );
            let computed = self.component::<ComputedStyle>(id);
            if text.value.is_empty() || !computed.visible {
                continue;
            }
            let source = self.component::<NodeStyle>(id);
            let layout = *self.component::<LayoutBox>(id);
            let padding = source.layout.resolved_padding_against(Some(layout.width));
            let border = source.layout.resolved_border_width();
            let leading_visual = match self.world.get::<StandardVisual>(self.entities[&id]) {
                Some(StandardVisual::Checkbox { .. }) => 24.0,
                Some(StandardVisual::Switch { .. }) => 38.0,
                _ => 0.0,
            };
            let constraints = crate::TextShapeConstraints {
                max_width: Some(
                    (layout.width - padding.left - padding.right - border * 2.0 - leading_visual)
                        .max(0.0),
                ),
                // Auto-height text must be allowed to grow after wrapping. Feeding the
                // provisional intrinsic height back as a hard bound would make the first
                // one-line measurement self-fulfilling. Only authored definite heights
                // constrain the text backend; max-height is enforced by layout itself.
                max_height: (source
                    .layout
                    .height
                    .is_some_and(nana_ui_core::LengthSpec::is_definite_declared)
                    || source
                        .layout
                        .max_height
                        .is_some_and(nana_ui_core::LengthSpec::is_definite_declared))
                .then(|| (layout.height - padding.top - padding.bottom - border * 2.0).max(0.0)),
                wrap: !source.layout.white_space_nowrap,
                ellipsis: source.layout.text_overflow_ellipsis,
                shaping: self.text_shaping(id),
            };
            let metrics = shaper.shape(id, &text, computed, constraints);
            validate_text_metrics(id, metrics)?;
            let presentation = presentation
                .map(|source| shape_text_input_presentation(id, source, computed, shaper));
            if *self.component::<TextMetrics>(id) != metrics
                || presentation.as_ref().is_some_and(|value| {
                    self.world.get::<TextInputPresentation>(self.entities[&id]) != Some(value)
                })
            {
                shaped.push((id, metrics, presentation));
            }
        }
        let changed = !shaped.is_empty();
        for (id, metrics, presentation) in shaped {
            *self.component_mut::<TextMetrics>(id) = metrics;
            if let Some(presentation) = presentation {
                self.world
                    .entity_mut(self.entities[&id])
                    .insert(presentation);
            }
        }
        Ok(changed)
    }

    fn text_input_presentation_source(
        &self,
        id: StableNodeId,
    ) -> Option<TextInputPresentationSource> {
        let StandardVisual::TextInput {
            placeholder,
            secure,
            ..
        } = self.world.get::<StandardVisual>(self.entities[&id])?
        else {
            return None;
        };
        let state = self.world.get::<TextInputState>(self.entities[&id])?;
        let ime = self.world.get::<ImeComposition>(self.entities[&id]);
        Some(build_text_input_presentation_source(
            state,
            ime,
            placeholder,
            *secure,
        ))
    }

    fn text_shaping(&self, id: StableNodeId) -> crate::TextShaping {
        if self
            .world
            .get::<TextInputState>(self.entities[&id])
            .is_some()
        {
            crate::TextShaping::Advanced
        } else {
            crate::TextShaping::Auto
        }
    }

    pub fn layout_inputs(&self, ids: &[StableNodeId]) -> Result<Vec<LayoutInput>, UiWorldError> {
        ids.iter()
            .copied()
            .map(|id| {
                if !self.contains(id) {
                    return Err(UiWorldError::MissingNode(id));
                }
                let hierarchy = self.component::<Hierarchy>(id);
                let has_text = matches!(self.component::<Kind>(id).0, NodeKind::Text)
                    || !self.component::<TextContent>(id).value.is_empty();
                let mut style = Arc::clone(&self.component::<NodeStyle>(id).layout);
                if !self.overlay_branch_active(id) {
                    Arc::make_mut(&mut style).hidden = true;
                }
                Ok(LayoutInput {
                    id,
                    parent: hierarchy.parent,
                    children: hierarchy.children.clone(),
                    style,
                    text_metrics: has_text.then(|| *self.component::<TextMetrics>(id)),
                })
            })
            .collect()
    }

    /// Rebuild one document's event-time hit-test index after scheduled input
    /// or layout work. Pointer dispatch then scans only compact hit entries.
    pub fn rebuild_hit_test(&mut self, document: DocumentId) {
        let mut roots = self
            .entities
            .keys()
            .copied()
            .filter(|id| {
                self.component::<Identity>(*id).document == document
                    && self.component::<Hierarchy>(*id).parent.is_none()
            })
            .collect::<Vec<_>>();
        roots.sort_unstable();
        let mut stack = roots
            .into_iter()
            .rev()
            .map(|id| (id, IDENTITY_AFFINE, Vec::new()))
            .collect::<Vec<_>>();
        let mut entries = Vec::new();
        let mut order = 0;
        while let Some((id, parent_transform, parent_clips)) = stack.pop() {
            let layout = *self.component::<LayoutBox>(id);
            let node_style = self.component::<NodeStyle>(id).layout.as_ref();
            let local = node_style
                .transform
                .map(|transform| {
                    transform.around_center(layout.x, layout.y, layout.width, layout.height)
                })
                .unwrap_or(IDENTITY_AFFINE);
            let transform = then_affine(parent_transform, local);
            let mut child_clips = parent_clips.clone();
            if node_style.clips_overflow() {
                child_clips.push((layout, transform));
            }
            let scroll = *self.component::<ScrollOffset>(id);
            let child_transform =
                then_affine(transform, [1.0, 0.0, 0.0, 1.0, -scroll.x, -scroll.y]);
            stack.extend(
                self.component::<Hierarchy>(id)
                    .children
                    .iter()
                    .rev()
                    .map(|child| (*child, child_transform, child_clips.clone())),
            );

            let style = self.component::<ComputedStyle>(id);
            let interaction = self.component::<InteractionState>(id);
            if style.visible && interaction.pointer_events {
                entries.push(HitEntry {
                    id,
                    layout,
                    transform,
                    clips: parent_clips,
                    z_index: node_style.z_index.unwrap_or_default(),
                    order,
                });
            }
            order += 1;
        }
        self.hit_test_index.insert(document, entries);
    }

    pub fn hit_test(&self, document: DocumentId, x: f32, y: f32) -> Option<StableNodeId> {
        self.hit_test_candidates(document, x, y).into_iter().next()
    }

    pub fn hit_test_candidates(&self, document: DocumentId, x: f32, y: f32) -> Vec<StableNodeId> {
        let mut candidates = self
            .hit_test_index
            .get(&document)
            .into_iter()
            .flatten()
            .filter(|entry| entry.contains(x, y))
            .collect::<Vec<_>>();
        candidates.sort_by_key(|entry| {
            (
                std::cmp::Reverse(entry.z_index),
                std::cmp::Reverse(entry.order),
            )
        });
        candidates.into_iter().map(|entry| entry.id).collect()
    }

    /// Produce a renderer-neutral snapshot in retained document order.
    pub fn extract_document(&self, document: DocumentId) -> Vec<ExtractedNode> {
        self.document_order(document)
            .into_iter()
            .filter_map(|id| self.extract_node(id).filter(|node| node.style.visible))
            .collect()
    }

    /// Extract only dirty nodes. Hidden nodes stay present with `visible=false`
    /// so an incremental renderer can remove their previous primitives.
    pub fn extract_nodes(&self, ids: &[StableNodeId]) -> Vec<ExtractedNode> {
        ids.iter().filter_map(|&id| self.extract_node(id)).collect()
    }

    pub fn commit(&mut self, queue: MutationQueue) -> Result<CommitReport, UiWorldError> {
        let mut report = CommitReport {
            generation: self.generation,
            mutations: queue.len(),
            created: 0,
            inserted: 0,
            detached: 0,
            reparented: 0,
            despawned: 0,
        };
        if queue.is_empty() {
            return Ok(report);
        }
        if !self.validate_simple_mutation(queue.as_slice())? {
            ValidationPlan::new(self).validate(queue.as_slice())?;
        }
        self.generation = self.generation.wrapping_add(1);
        report.generation = self.generation;
        for mutation in queue.as_slice() {
            self.apply(mutation, &mut report);
        }
        Ok(report)
    }

    /// Single-node creation and detached append have no staged cross-mutation
    /// state to simulate. Validate them directly so retained DOM/component
    /// construction does not scan the world or clone a growing child list.
    fn validate_simple_mutation(&self, mutations: &[UiMutation]) -> Result<bool, UiWorldError> {
        if let [UiMutation::Create { id, .. }] = mutations {
            if self.contains(*id) {
                return Err(UiWorldError::DuplicateNode(*id));
            }
            if self.is_retired(*id) {
                return Err(UiWorldError::RetiredNode(*id));
            }
            return Ok(true);
        }
        let [
            UiMutation::Insert {
                parent,
                child,
                before: None,
            },
        ] = mutations
        else {
            return Ok(false);
        };
        let (parent_document, _) = self.identity_and_parent(*parent)?;
        let (child_document, child_parent) = self.identity_and_parent(*child)?;
        if child_parent.is_some() {
            return Ok(false);
        }
        if parent_document != child_document {
            return Err(UiWorldError::CrossDocument {
                parent: *parent,
                child: *child,
            });
        }
        let mut ancestor = Some(*parent);
        while let Some(id) = ancestor {
            if id == *child {
                return Err(UiWorldError::Cycle {
                    parent: *parent,
                    child: *child,
                });
            }
            ancestor = self.identity_and_parent(id)?.1;
        }
        Ok(true)
    }

    fn identity_and_parent(
        &self,
        id: StableNodeId,
    ) -> Result<(DocumentId, Option<StableNodeId>), UiWorldError> {
        let entity = *self
            .entities
            .get(&id)
            .ok_or(UiWorldError::MissingNode(id))?;
        let identity = self
            .world
            .get::<Identity>(entity)
            .ok_or(UiWorldError::MissingNode(id))?;
        let hierarchy = self
            .world
            .get::<Hierarchy>(entity)
            .ok_or(UiWorldError::MissingNode(id))?;
        Ok((identity.document, hierarchy.parent))
    }

    fn apply(&mut self, mutation: &UiMutation, report: &mut CommitReport) {
        match mutation {
            UiMutation::Create { id, document, kind } => {
                let entity = self
                    .world
                    .spawn((
                        Identity {
                            stable: *id,
                            document: *document,
                        },
                        Kind(kind.clone()),
                        Hierarchy::default(),
                        NodeStyle::default(),
                        ComputedStyle::default(),
                        TextContent::default(),
                        TextMetrics::default(),
                        LayoutBox::default(),
                        ScrollOffset::default(),
                        InteractionState::default(),
                        AccessibilityState::default(),
                        DirtyMask::all(),
                    ))
                    .id();
                self.entities.insert(*id, entity);
                self.dirty_entities.insert(*id);
                report.created += 1;
            }
            UiMutation::Insert {
                parent,
                child,
                before,
            } => {
                let old_parent = self
                    .identity_and_parent(*child)
                    .expect("validated child must exist")
                    .1;
                if old_parent == Some(*parent) && *before == Some(*child) {
                    return;
                }
                if let Some(old_parent) = old_parent {
                    self.hierarchy_mut(old_parent)
                        .children
                        .retain(|id| id != child);
                }
                let siblings = &mut self.hierarchy_mut(*parent).children;
                let index = before
                    .and_then(|before| siblings.iter().position(|id| *id == before))
                    .unwrap_or(siblings.len());
                siblings.insert(index, *child);
                self.hierarchy_mut(*child).parent = Some(*parent);
                if old_parent == Some(*parent) {
                    // Retained-order moves carry the entire subtree; descendants
                    // keep their inherited state and local geometry until layout
                    // writeback identifies actual changes.
                    self.mark(
                        *child,
                        DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                } else {
                    self.mark_subtree(
                        *child,
                        DirtyMask::STYLE
                            | DirtyMask::TEXT
                            | DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                }
                self.mark_ancestors(
                    *parent,
                    DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                );
                if let Some(old_parent) = old_parent {
                    self.mark_ancestors(
                        old_parent,
                        DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                }
                if old_parent.is_some() {
                    report.reparented += 1;
                } else {
                    report.inserted += 1;
                }
            }
            UiMutation::Remove { id } => {
                let parent = self.node(*id).expect("validated node must exist").parent;
                if let Some(parent) = parent {
                    self.hierarchy_mut(parent)
                        .children
                        .retain(|child| child != id);
                    self.hierarchy_mut(*id).parent = None;
                    self.mark_subtree(
                        *id,
                        DirtyMask::STYLE
                            | DirtyMask::TEXT
                            | DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                    self.mark_ancestors(
                        parent,
                        DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                    report.detached += 1;
                }
            }
            UiMutation::DespawnSubtree { root } => {
                let root_snapshot = self.node(*root).expect("validated root must exist");
                if let Some(parent) = root_snapshot.parent {
                    self.hierarchy_mut(parent)
                        .children
                        .retain(|child| child != root);
                    self.mark_ancestors(
                        parent,
                        DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                }
                let mut stack = vec![*root];
                while let Some(id) = stack.pop() {
                    let snapshot = self.node(id).expect("validated subtree must exist");
                    stack.extend(snapshot.children.iter().rev().copied());
                    let entity = self
                        .entities
                        .remove(&id)
                        .expect("entity index must contain node");
                    self.dirty_entities.remove(&id);
                    if self.focused.get(&snapshot.document) == Some(&id) {
                        self.focused.remove(&snapshot.document);
                    }
                    if let Some(index) = self.hit_test_index.get_mut(&snapshot.document) {
                        index.retain(|entry| entry.id != id);
                    }
                    let released = self
                        .pointer_captures
                        .iter()
                        .filter_map(|(&(document, pointer_id), &target)| {
                            (target == id).then_some((document, pointer_id))
                        })
                        .collect::<Vec<_>>();
                    for key @ (_, pointer_id) in released {
                        self.pointer_captures.remove(&key);
                        self.pending_pointer_capture_changes
                            .push(PointerCaptureChange {
                                pointer_id,
                                target: id,
                                captured: false,
                            });
                    }
                    self.pointer_hover.retain(|_, target| *target != id);
                    self.pointer_press.retain(|_, target| *target != id);
                    let cancelled = self
                        .animations
                        .iter()
                        .filter_map(|(&animation_id, animation)| {
                            (animation.spec.target == id)
                                .then_some((animation_id, animation.next_deadline))
                        })
                        .collect::<Vec<_>>();
                    for (animation_id, deadline) in cancelled {
                        self.animations.remove(&animation_id);
                        self.animation_deadlines.remove(&(deadline, animation_id));
                    }
                    self.clear_overlay_references(id);
                    let _ = self.world.despawn(entity);
                    self.retired.insert(id);
                    self.pending_render_removals.push(id);
                    self.pending_accessibility_removals.push(id);
                    report.despawned += 1;
                }
            }
            UiMutation::SetStyle { id, style } => {
                let previous = self.component::<NodeStyle>(*id).clone();
                let inherited_text_changed = previous.layout.font_size != style.layout.font_size
                    || previous.layout.font_weight != style.layout.font_weight
                    || previous.layout.font_family != style.layout.font_family
                    || previous.layout.line_height != style.layout.line_height
                    || previous.layout.letter_spacing != style.layout.letter_spacing;
                let inherited_paint_changed = previous.foreground != style.foreground
                    || previous.layout.color != style.layout.color
                    || previous.layout.opacity != style.layout.opacity;
                let visibility_changed = previous.layout.hidden != style.layout.hidden;
                let transform_changed = previous.layout.transform != style.layout.transform
                    || previous.layout.unsupported_transform != style.layout.unsupported_transform;
                let stacking_changed = previous.layout.z_index != style.layout.z_index;
                let layout_changed =
                    layout_semantics_changed(previous.layout.as_ref(), style.layout.as_ref());
                *self.component_mut::<NodeStyle>(*id) = style.clone();

                self.mark(*id, DirtyMask::STYLE | DirtyMask::RENDER);
                if inherited_paint_changed {
                    self.mark_subtree(*id, DirtyMask::STYLE | DirtyMask::RENDER);
                }
                if inherited_text_changed {
                    self.mark_subtree(
                        *id,
                        DirtyMask::STYLE
                            | DirtyMask::TEXT
                            | DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::RENDER,
                    );
                }
                if visibility_changed {
                    self.mark_subtree(
                        *id,
                        DirtyMask::STYLE
                            | DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                    if let Some(parent) = self.node(*id).and_then(|node| node.parent) {
                        self.mark(parent, DirtyMask::ACCESSIBILITY);
                    }
                }
                if transform_changed || stacking_changed {
                    self.mark_subtree(*id, DirtyMask::INPUT | DirtyMask::RENDER);
                }
                if layout_changed {
                    self.mark_subtree(
                        *id,
                        DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                }
                if (layout_changed || inherited_text_changed || visibility_changed)
                    && let Some(parent) = self.node(*id).and_then(|node| node.parent)
                {
                    self.mark_ancestors(parent, DirtyMask::LAYOUT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetTheme { mode } => {
                if self.style_model.theme_mode != *mode {
                    self.style_model = StyleModelRef::new(*mode);
                    let ids = self.entities.keys().copied().collect::<Vec<_>>();
                    for id in ids {
                        self.mark(id, DirtyMask::STYLE | DirtyMask::RENDER);
                    }
                }
            }
            UiMutation::SetText { id, text } => {
                *self.component_mut::<TextContent>(*id) = text.clone();
                self.mark(
                    *id,
                    DirtyMask::TEXT
                        | DirtyMask::LAYOUT
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
                if let Some(parent) = self.node(*id).and_then(|node| node.parent) {
                    self.mark_ancestors(parent, DirtyMask::LAYOUT | DirtyMask::RENDER);
                }
            }
            UiMutation::WriteLayout { id, layout } => {
                *self.component_mut::<LayoutBox>(*id) = *layout;
                self.mark_subtree(
                    *id,
                    DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetScrollOffset { id, offset } => {
                let offset = self.clamp_scroll_offset(*id, *offset);
                if *self.component::<ScrollOffset>(*id) != offset {
                    *self.component_mut::<ScrollOffset>(*id) = offset;
                    self.mark_subtree(*id, DirtyMask::INPUT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetScrollMetrics { id, metrics } => {
                let entity = self.entities[id];
                if let Some(metrics) = metrics {
                    self.world.entity_mut(entity).insert(*metrics);
                } else {
                    self.world.entity_mut(entity).remove::<ScrollMetrics>();
                }
                let current = *self.component::<ScrollOffset>(*id);
                let clamped = self.clamp_scroll_offset(*id, current);
                if current != clamped {
                    *self.component_mut::<ScrollOffset>(*id) = clamped;
                    self.mark_subtree(*id, DirtyMask::INPUT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetInteraction { id, interaction } => {
                *self.component_mut::<InteractionState>(*id) = *interaction;
                if !interaction.pointer_events {
                    self.pointer_hover.retain(|_, target| target != id);
                    self.pointer_press.retain(|_, target| target != id);
                }
                self.mark(
                    *id,
                    DirtyMask::INPUT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetCustomRender { id, content } => {
                let entity = self.entities[id];
                if let Some(content) = content {
                    self.world.entity_mut(entity).insert(content.clone());
                } else {
                    self.world.entity_mut(entity).remove::<CustomRenderNode>();
                }
                self.mark(*id, DirtyMask::RENDER);
            }
            UiMutation::SetStandardVisual { id, visual } => {
                let entity = self.entities[id];
                let text_input_presentation_changed =
                    matches!(
                        self.world.get::<StandardVisual>(entity),
                        Some(StandardVisual::TextInput { .. })
                    ) || matches!(visual, Some(StandardVisual::TextInput { .. }));
                if let Some(visual) = visual {
                    self.world.entity_mut(entity).insert(visual.clone());
                } else {
                    self.world.entity_mut(entity).remove::<StandardVisual>();
                }
                if !matches!(visual, Some(StandardVisual::TextInput { .. })) {
                    self.world
                        .entity_mut(entity)
                        .remove::<TextInputPresentation>();
                }
                self.mark(
                    *id,
                    DirtyMask::RENDER
                        | if text_input_presentation_changed {
                            DirtyMask::TEXT
                        } else {
                            0
                        },
                );
            }
            UiMutation::SetAccessibility { id, accessibility } => {
                let previous = self.component::<AccessibilityState>(*id);
                let interaction_style_changed = previous.disabled != accessibility.disabled
                    || previous.checked != accessibility.checked
                    || previous.selected != accessibility.selected;
                *self.component_mut::<AccessibilityState>(*id) = accessibility.clone();
                self.mark(*id, DirtyMask::ACCESSIBILITY);
                if interaction_style_changed
                    && !self.component::<NodeStyle>(*id).interaction.is_empty()
                {
                    self.mark(*id, DirtyMask::STYLE | DirtyMask::RENDER);
                }
            }
            UiMutation::SetOverlayHost { host, state } => {
                let entity = self.entities[host];
                let previous = self.world.get::<OverlayHostState>(entity).copied();
                if previous == Some(*state) {
                    return;
                }
                self.world.entity_mut(entity).insert(*state);
                self.mark(*host, DirtyMask::ACCESSIBILITY);
                if let Some(inactive) = previous
                    .and_then(|previous| previous.active)
                    .filter(|active| Some(*active) != state.active)
                {
                    let mut inactive_nodes = HashSet::new();
                    let mut stack = vec![inactive];
                    while let Some(id) = stack.pop() {
                        stack.extend(self.component::<Hierarchy>(id).children.iter().copied());
                        inactive_nodes.insert(id);
                    }
                    let released = self
                        .pointer_captures
                        .iter()
                        .filter_map(|(&(document, pointer_id), &target)| {
                            inactive_nodes
                                .contains(&target)
                                .then_some((document, pointer_id, target))
                        })
                        .collect::<Vec<_>>();
                    for (document, pointer_id, target) in released {
                        self.pointer_captures.remove(&(document, pointer_id));
                        self.pending_pointer_capture_changes
                            .push(PointerCaptureChange {
                                pointer_id,
                                target,
                                captured: false,
                            });
                    }
                    self.pointer_hover
                        .retain(|_, target| !inactive_nodes.contains(target));
                    self.pointer_press
                        .retain(|_, target| !inactive_nodes.contains(target));
                }
                let changed_roots = previous
                    .and_then(|previous| previous.active)
                    .into_iter()
                    .chain(state.active)
                    .collect::<HashSet<_>>();
                for root in changed_roots {
                    self.mark_subtree(
                        root,
                        DirtyMask::STYLE
                            | DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                }
            }
            UiMutation::CapturePointer { pointer_id, target } => {
                let document = self.component::<Identity>(*target).document;
                let previous = self
                    .pointer_captures
                    .insert((document, *pointer_id), *target);
                if previous == Some(*target) {
                    return;
                }
                if let Some(previous) = previous {
                    self.pending_pointer_capture_changes
                        .push(PointerCaptureChange {
                            pointer_id: *pointer_id,
                            target: previous,
                            captured: false,
                        });
                }
                self.pending_pointer_capture_changes
                    .push(PointerCaptureChange {
                        pointer_id: *pointer_id,
                        target: *target,
                        captured: true,
                    });
            }
            UiMutation::ReleasePointer { pointer_id, target } => {
                let document = self.component::<Identity>(*target).document;
                self.pointer_captures.remove(&(document, *pointer_id));
                self.pending_pointer_capture_changes
                    .push(PointerCaptureChange {
                        pointer_id: *pointer_id,
                        target: *target,
                        captured: false,
                    });
            }
            UiMutation::StartAnimation { animation } => {
                let active = ActiveAnimation::new(*animation);
                if let Some(previous) = self.animations.insert(animation.id, active) {
                    self.animation_deadlines
                        .remove(&(previous.next_deadline, animation.id));
                }
                self.animation_deadlines
                    .insert((animation.start, animation.id));
            }
            UiMutation::StopAnimation { id } => {
                if let Some(animation) = self.animations.remove(id) {
                    self.animation_deadlines
                        .remove(&(animation.next_deadline, *id));
                }
            }
            UiMutation::RequestFocus { document, target } => {
                let old = match target {
                    Some(target) => self.focused.insert(*document, *target),
                    None => self.focused.remove(document),
                };
                if let Some(old) = old.filter(|old| Some(*old) != *target) {
                    self.remove_ime(old);
                    if !self
                        .component::<NodeStyle>(old)
                        .interaction
                        .focused
                        .is_empty()
                    {
                        self.mark(old, DirtyMask::STYLE | DirtyMask::RENDER);
                    }
                    self.mark(
                        old,
                        DirtyMask::FOCUS_IME | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                }
                if let Some(target) = target {
                    if !self
                        .component::<NodeStyle>(*target)
                        .interaction
                        .focused
                        .is_empty()
                    {
                        self.mark(*target, DirtyMask::STYLE | DirtyMask::RENDER);
                    }
                    self.mark(
                        *target,
                        DirtyMask::FOCUS_IME | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                }
            }
            UiMutation::SetIme { id, composition } => {
                let entity = self.entities[id];
                if let Some(composition) = composition {
                    self.world.entity_mut(entity).insert(composition.clone());
                } else {
                    self.world.entity_mut(entity).remove::<ImeComposition>();
                }
                self.mark(
                    *id,
                    DirtyMask::TEXT | DirtyMask::FOCUS_IME | DirtyMask::RENDER,
                );
            }
            UiMutation::SetTextInput { id, state } => {
                let entity = self.entities[id];
                if let Some(state) = state {
                    self.world.entity_mut(entity).insert(state.clone());
                    *self.component_mut::<TextContent>(*id) = TextContent {
                        value: state.value.clone(),
                    };
                } else {
                    self.world.entity_mut(entity).remove::<TextInputState>();
                    *self.component_mut::<TextContent>(*id) = TextContent::default();
                    self.remove_ime(*id);
                }
                self.mark(
                    *id,
                    DirtyMask::TEXT
                        | DirtyMask::LAYOUT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetTextSelection { id, selection } => {
                self.component_mut::<TextInputState>(*id).selection = *selection;
                self.mark(
                    *id,
                    DirtyMask::TEXT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::ReplaceTextSelection { id, text } => {
                let (replaced, value) = {
                    let mut state = self.component_mut::<TextInputState>(*id);
                    let replaced = state.replace_selection(text);
                    (replaced, state.value.clone())
                };
                debug_assert!(replaced, "validated selection must remain valid");
                *self.component_mut::<TextContent>(*id) = TextContent { value };
                self.mark(
                    *id,
                    DirtyMask::TEXT
                        | DirtyMask::LAYOUT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
        }
    }

    fn component<T: Component>(&self, id: StableNodeId) -> &T {
        self.world
            .get::<T>(self.entities[&id])
            .expect("entity must have runtime component")
    }

    fn extract_node(&self, id: StableNodeId) -> Option<ExtractedNode> {
        let entity = *self.entities.get(&id)?;
        let identity = self.world.get::<Identity>(entity)?;
        let style = self.world.get::<ComputedStyle>(entity)?.clone();
        let kind = self.world.get::<Kind>(entity)?.0.clone();
        let has_text = matches!(kind, NodeKind::Text)
            || self
                .world
                .get::<TextContent>(entity)
                .is_some_and(|text| !text.value.is_empty());
        let source_style = self.world.get::<NodeStyle>(entity)?.clone();
        let hierarchy = self.world.get::<Hierarchy>(entity)?;
        let standard_visual = self.world.get::<StandardVisual>(entity).cloned();
        let component_geometry = standard_visual
            .as_ref()
            .and_then(|visual| self.derive_component_geometry(id, visual, &style));
        let standard_visual_foreground = standard_visual.as_ref().map(|visual| match visual {
            StandardVisual::Icon { .. } => style
                .color
                .unwrap_or_else(|| self.style_model.palette.muted.as_rgba_array()),
            StandardVisual::Button { .. } => style
                .color
                .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array()),
            StandardVisual::TextInput { .. } => style
                .color
                .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array()),
            StandardVisual::Checkbox { checked: true }
            | StandardVisual::Switch { checked: true, .. } => {
                self.style_model.palette.accent_text.as_rgba_array()
            }
            StandardVisual::Checkbox { checked: false }
            | StandardVisual::Switch { checked: false, .. } => {
                self.style_model.palette.muted.as_rgba_array()
            }
            StandardVisual::Slider { .. }
            | StandardVisual::Range { .. }
            | StandardVisual::Card { .. }
            | StandardVisual::ListItem { .. } => self.style_model.palette.accent.as_rgba_array(),
        });
        Some(ExtractedNode {
            id,
            kind,
            parent: hierarchy.parent,
            children: hierarchy.children.clone(),
            layout: *self.world.get::<LayoutBox>(entity)?,
            scroll_offset: *self.world.get::<ScrollOffset>(entity)?,
            z_index: source_style.layout.z_index.unwrap_or_default(),
            source_style,
            style,
            text: if has_text {
                self.world.get::<TextContent>(entity).cloned()
            } else {
                None
            },
            text_metrics: if has_text {
                self.world.get::<TextMetrics>(entity).copied()
            } else {
                None
            },
            focused: self.focused.get(&identity.document) == Some(&id),
            ime: self.world.get::<ImeComposition>(entity).cloned(),
            text_input: self.world.get::<TextInputState>(entity).cloned(),
            standard_visual,
            component_geometry,
            standard_visual_foreground,
            custom_render: self.world.get::<CustomRenderNode>(entity).cloned(),
        })
    }

    fn derive_component_geometry(
        &self,
        id: StableNodeId,
        visual: &StandardVisual,
        style: &ComputedStyle,
    ) -> Option<crate::ComponentGeometry> {
        let bounds = *self.world.get::<LayoutBox>(self.entities[&id])?;
        let source = self.world.get::<NodeStyle>(self.entities[&id])?;
        let padding = source.layout.resolved_padding_against(Some(bounds.width));
        let border = source.layout.resolved_border_width();
        let content = LayoutBox {
            x: bounds.x + border + padding.left,
            y: bounds.y + border + padding.top,
            width: (bounds.width - border * 2.0 - padding.left - padding.right).max(0.0),
            height: (bounds.height - border * 2.0 - padding.top - padding.bottom).max(0.0),
        };
        let text_region = |bounds, content: Arc<str>, muted: bool, size: f32, weight| {
            crate::ComponentTextRegion {
                bounds,
                content,
                color: Some(if muted {
                    self.style_model.palette.muted.as_rgba_array()
                } else {
                    style
                        .color
                        .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array())
                }),
                font_size: size,
                font_weight: weight,
            }
        };
        match visual {
            StandardVisual::Button {
                label,
                size,
                loading,
                invalid,
                ..
            } => {
                // Loading reserves 20px through symmetric intrinsic padding in
                // the layout pass. That reservation grows the outer button; it
                // is not additional visual padding, so return it to the inline
                // content box before centering spinner + label.
                let button_content = if *loading {
                    LayoutBox {
                        x: content.x - 10.0,
                        width: content.width + 20.0,
                        ..content
                    }
                } else {
                    content
                };
                let label_width = self
                    .text_metrics(id)
                    .map_or(0.0, |metrics| metrics.width.min(button_content.width));
                let spinner_extent = size.icon_size().min(button_content.height);
                let gap = if *loading { 6.0 } else { 0.0 };
                let group_width = (label_width + if *loading { spinner_extent + gap } else { 0.0 })
                    .min(button_content.width);
                let group_x = button_content.x + (button_content.width - group_width) / 2.0;
                let spinner = (*loading).then_some(LayoutBox {
                    x: group_x,
                    y: button_content.y + (button_content.height - spinner_extent) / 2.0,
                    width: spinner_extent,
                    height: spinner_extent,
                });
                let label_x = group_x + if *loading { spinner_extent + gap } else { 0.0 };
                Some(crate::ComponentGeometry::Button {
                    label: text_region(
                        LayoutBox {
                            x: label_x,
                            y: button_content.y,
                            width: (group_x + group_width - label_x).max(0.0),
                            height: button_content.height,
                        },
                        Arc::clone(label),
                        false,
                        size.text_size(),
                        Some(500),
                    ),
                    spinner,
                    background: style.background,
                    border: style.border_color,
                    border_width: if style.border_color.is_some() {
                        source.layout.resolved_border_width()
                    } else {
                        0.0
                    },
                    focus_ring: (self.focused.get(&self.component::<Identity>(id).document)
                        == Some(&id))
                    .then(|| {
                        if *invalid {
                            self.style_model.palette.danger.as_rgba_array()
                        } else {
                            self.style_model.palette.accent.as_rgba_array()
                        }
                    }),
                })
            }
            StandardVisual::TextInput { size, invalid, .. } => {
                let presentation = self
                    .world
                    .get::<TextInputPresentation>(self.entities[&id])?;
                let focused =
                    self.focused.get(&self.component::<Identity>(id).document) == Some(&id);
                let text_width = self.text_metrics(id).map_or(0.0, |metrics| metrics.width);
                let scroll = (presentation.caret_x - content.width + 1.0)
                    .max(0.0)
                    .min((text_width - content.width).max(0.0));
                let line_height = size.line_height().min(content.height);
                let line_y = content.y + (content.height - line_height) / 2.0;
                let field_x = |offset: f32| content.x + offset - scroll;
                let selection = presentation.selection.map(|(start, end)| LayoutBox {
                    x: field_x(start),
                    y: line_y,
                    width: (end - start).max(0.0),
                    height: line_height,
                });
                let caret = focused.then_some(LayoutBox {
                    x: field_x(presentation.caret_x),
                    y: line_y,
                    width: 1.0,
                    height: line_height,
                });
                let preedit = presentation.preedit.map(|(start, end)| LayoutBox {
                    x: field_x(start),
                    y: line_y + line_height - 2.0,
                    width: (end - start).max(1.0),
                    height: 2.0,
                });
                Some(crate::ComponentGeometry::TextInput {
                    text: crate::ComponentTextRegion {
                        bounds: LayoutBox {
                            x: content.x - scroll,
                            y: content.y,
                            width: text_width.max(content.width),
                            height: content.height,
                        },
                        content: Arc::from(presentation.display_value.as_str()),
                        color: Some(if presentation.placeholder {
                            self.style_model.palette.faint.as_rgba_array()
                        } else {
                            style
                                .color
                                .unwrap_or_else(|| self.style_model.palette.text.as_rgba_array())
                        }),
                        font_size: size.text_size(),
                        font_weight: style.font_weight,
                    },
                    selection,
                    caret,
                    preedit,
                    background: style.background,
                    border: style.border_color,
                    border_width: if style.border_color.is_some() {
                        source.layout.resolved_border_width()
                    } else {
                        0.0
                    },
                    focus_ring: focused.then(|| {
                        if *invalid {
                            self.style_model.palette.danger.as_rgba_array()
                        } else {
                            self.style_model.palette.accent.as_rgba_array()
                        }
                    }),
                    selection_color: self.style_model.palette.accent_soft.as_rgba_array(),
                    caret_color: self.style_model.palette.accent.as_rgba_array(),
                    preedit_color: self.style_model.palette.accent.as_rgba_array(),
                })
            }
            StandardVisual::Switch {
                label,
                hint,
                checked,
                control_position,
                size,
                invalid,
                ..
            } => {
                let control = LayoutBox {
                    x: match control_position {
                        SwitchControlPosition::Start => content.x,
                        SwitchControlPosition::End => content.x + (content.width - 30.0).max(0.0),
                    },
                    y: content.y + (content.height - 16.0) / 2.0,
                    width: 30.0_f32.min(content.width),
                    height: 16.0_f32.min(content.height),
                };
                let text_x = if *control_position == SwitchControlPosition::Start {
                    control.x + control.width + 8.0
                } else {
                    content.x
                };
                let text_right = if *control_position == SwitchControlPosition::End {
                    control.x - 8.0
                } else {
                    content.x + content.width
                };
                let text_width = (text_right - text_x).max(0.0);
                let (label_bounds, hint_bounds) = if hint.is_some() {
                    (
                        LayoutBox {
                            x: text_x,
                            y: content.y,
                            width: text_width,
                            height: 18.0_f32.min(content.height),
                        },
                        Some(LayoutBox {
                            x: text_x,
                            y: content.y + 20.0,
                            width: text_width,
                            height: (content.height - 20.0).max(0.0),
                        }),
                    )
                } else {
                    (
                        LayoutBox {
                            x: text_x,
                            y: content.y,
                            width: text_width,
                            height: content.height,
                        },
                        None,
                    )
                };
                let palette = self.style_model.palette;
                let hovered = self.pointer_hover.values().any(|target| *target == id);
                let pressed = self.pointer_press.values().any(|target| *target == id);
                let disabled = self.component::<AccessibilityState>(id).disabled;
                let mix = |foreground: [f32; 4], background: [f32; 4], amount: f32| {
                    let amount = amount.clamp(0.0, 1.0);
                    std::array::from_fn(|channel| {
                        foreground[channel] * amount + background[channel] * (1.0 - amount)
                    })
                };
                let fade = |mut color: [f32; 4]| {
                    if disabled {
                        color[3] *= 0.55;
                    }
                    color
                };
                let track_background = if *checked {
                    if pressed {
                        palette.accent_strong.as_rgba_array()
                    } else {
                        palette.accent.as_rgba_array()
                    }
                } else {
                    mix(
                        palette.hover.as_rgba_array(),
                        palette.background.as_rgba_array(),
                        0.78,
                    )
                };
                let track_border = if *invalid {
                    palette.danger.as_rgba_array()
                } else if *checked {
                    if hovered || pressed {
                        palette.accent_strong.as_rgba_array()
                    } else {
                        palette.accent.as_rgba_array()
                    }
                } else if hovered || pressed {
                    mix(
                        palette.accent.as_rgba_array(),
                        palette.border_strong.as_rgba_array(),
                        if pressed { 0.70 } else { 0.42 },
                    )
                } else {
                    palette.border_strong.as_rgba_array()
                };
                let thumb_background = if *checked {
                    palette.accent_text.as_rgba_array()
                } else {
                    mix(
                        palette.faint.as_rgba_array(),
                        palette.background.as_rgba_array(),
                        0.70,
                    )
                };
                Some(crate::ComponentGeometry::Switch {
                    label: text_region(
                        label_bounds,
                        Arc::clone(label),
                        false,
                        size.text_size(),
                        Some(500),
                    ),
                    hint: hint.as_ref().zip(hint_bounds).map(|(hint, bounds)| {
                        text_region(
                            bounds,
                            Arc::clone(hint),
                            true,
                            (size.text_size() - 1.0).max(10.0),
                            None,
                        )
                    }),
                    control,
                    track_background: fade(track_background),
                    track_border: fade(track_border),
                    thumb_background: fade(thumb_background),
                })
            }
            StandardVisual::Range {
                label,
                value,
                unit,
                size,
                ..
            } => {
                let gap = match size {
                    nana_ui_core::ControlSize::Small => 6.0,
                    nana_ui_core::ControlSize::Medium => 8.0,
                    nana_ui_core::ControlSize::Large => 10.0,
                };
                let label_width = label
                    .as_ref()
                    .map_or(0.0, |_| 84.0_f32.min(content.width * 0.28));
                let unit_width = unit.as_ref().map_or(0.0, |_| 32.0_f32.min(content.width));
                let value_width = 60.0_f32.min((content.width - unit_width).max(0.0));
                let trailing_width = value_width + unit_width;
                let track_x = content.x + label_width + if label.is_some() { gap } else { 0.0 };
                let track_right = content.x + content.width
                    - trailing_width
                    - if trailing_width > 0.0 { gap } else { 0.0 };
                let thumb = size.icon_size();
                let track = LayoutBox {
                    x: track_x + thumb / 2.0,
                    y: content.y + (content.height - thumb) / 2.0,
                    width: (track_right - track_x - thumb).max(0.0),
                    height: thumb.min(content.height),
                };
                Some(crate::ComponentGeometry::Range {
                    label: label.as_ref().map(|label| {
                        text_region(
                            LayoutBox {
                                x: content.x,
                                y: content.y,
                                width: label_width,
                                height: content.height,
                            },
                            Arc::clone(label),
                            false,
                            size.text_size(),
                            Some(500),
                        )
                    }),
                    value: text_region(
                        LayoutBox {
                            x: content.x + content.width - value_width - unit_width,
                            y: content.y,
                            width: value_width,
                            height: content.height,
                        },
                        Arc::clone(value),
                        false,
                        size.text_size(),
                        Some(500),
                    ),
                    unit: unit.as_ref().map(|unit| {
                        text_region(
                            LayoutBox {
                                x: content.x + content.width - unit_width,
                                y: content.y,
                                width: unit_width,
                                height: content.height,
                            },
                            Arc::clone(unit),
                            true,
                            (size.text_size() - 1.0).max(10.0),
                            None,
                        )
                    }),
                    track,
                })
            }
            StandardVisual::Card {
                title,
                kind,
                loading,
                ..
            } => {
                let shaped_title_width = self
                    .text_metrics(id)
                    .map_or(0.0, |metrics| metrics.width.min(content.width));
                let title_width = (content.width - if *loading { 22.0 } else { 0.0 }).max(0.0);
                let title_y = bounds.y + border + (padding.top - 24.0).max(0.0);
                Some(crate::ComponentGeometry::Card {
                    title: title.as_ref().map(|title| {
                        text_region(
                            LayoutBox {
                                x: bounds.x + border + padding.left,
                                y: title_y,
                                width: title_width,
                                height: 18.0,
                            },
                            Arc::clone(title),
                            false,
                            13.0,
                            Some(600),
                        )
                    }),
                    content,
                    elevation: (*kind == nana_ui_core::CardKind::Raised).then_some(
                        crate::ComponentElevation {
                            color: self.style_model.palette.background.as_rgba_array(),
                            offset_y: 4.0,
                            blur_radius: 12.0,
                        },
                    ),
                    spinner: (*loading).then_some(LayoutBox {
                        x: (bounds.x + border + padding.left + shaped_title_width + 8.0)
                            .min(content.x + content.width - 14.0),
                        y: title_y + 2.0,
                        width: 14.0,
                        height: 14.0,
                    }),
                })
            }
            StandardVisual::ListItem {
                leading,
                content: content_slot,
                trailing,
            } => {
                let leading = leading.and_then(|id| self.layout_box(id));
                let trailing = trailing.and_then(|id| self.layout_box(id));
                let fallback_x = leading.map_or(content.x, |leading| {
                    leading.x
                        + leading.width
                        + source.layout.main_gap_against(
                            nana_ui_core::FlexDirection::Row,
                            nana_ui_core::ParentBox::from_viewport(content.width, content.height),
                        )
                });
                let fallback_right = trailing.map_or(content.x + content.width, |trailing| {
                    trailing.x
                        - source.layout.main_gap_against(
                            nana_ui_core::FlexDirection::Row,
                            nana_ui_core::ParentBox::from_viewport(content.width, content.height),
                        )
                });
                Some(crate::ComponentGeometry::ListItem {
                    leading,
                    content: content_slot.and_then(|id| self.layout_box(id)).or_else(|| {
                        Some(LayoutBox {
                            x: fallback_x,
                            y: content.y,
                            width: (fallback_right - fallback_x).max(0.0),
                            height: content.height,
                        })
                    }),
                    trailing,
                })
            }
            _ => None,
        }
    }

    fn project_accessibility_node(&self, id: StableNodeId) -> Option<AccessibilityNode> {
        let entity = *self.entities.get(&id)?;
        let identity = self.world.get::<Identity>(entity)?;
        let style = self.world.get::<ComputedStyle>(entity)?;
        if !style.visible {
            return None;
        }
        let hierarchy = self.world.get::<Hierarchy>(entity)?;
        let state = self.world.get::<AccessibilityState>(entity)?;
        let kind = &self.world.get::<Kind>(entity)?.0;
        if matches!(kind, NodeKind::Comment) {
            return None;
        }
        let role = match (state.role, kind) {
            (AccessibilityRole::Generic, NodeKind::Document) => AccessibilityRole::Document,
            (AccessibilityRole::Generic, NodeKind::Text) => AccessibilityRole::Text,
            (role, _) => role,
        };
        let label = state.label.clone().or_else(|| {
            self.world
                .get::<TextContent>(entity)
                .filter(|text| !text.value.is_empty())
                .map(|text| Arc::<str>::from(text.value.as_str()))
        });
        Some(AccessibilityNode {
            id,
            parent: hierarchy.parent,
            children: hierarchy
                .children
                .iter()
                .copied()
                .filter(|child| {
                    let child = self.entities[child];
                    self.world
                        .get::<ComputedStyle>(child)
                        .is_some_and(|style| style.visible)
                        && self
                            .world
                            .get::<Kind>(child)
                            .is_some_and(|kind| !matches!(kind.0, NodeKind::Comment))
                })
                .collect(),
            role,
            label,
            value: if matches!(
                self.world.get::<StandardVisual>(entity),
                Some(StandardVisual::TextInput { secure: true, .. })
            ) {
                None
            } else {
                self.world
                    .get::<TextInputState>(entity)
                    .map(|input| Arc::<str>::from(input.value.as_str()))
                    .or_else(|| state.value.clone())
            },
            disabled: state.disabled,
            checked: state.checked,
            selected: state.selected,
            multiline: state.multiline,
            editable: state.editable,
            selection: self
                .world
                .get::<TextInputState>(entity)
                .map(|input| input.selection),
            modal: state.modal,
            busy: state.busy,
            invalid: state.invalid,
            numeric_minimum: state.numeric_minimum,
            numeric_maximum: state.numeric_maximum,
            numeric_step: state.numeric_step,
            numeric_value: state.numeric_value,
            focused: self.focused.get(&identity.document) == Some(&id),
            bounds: *self.world.get::<LayoutBox>(entity)?,
        })
    }

    fn component_mut<T: Component<Mutability = Mutable>>(
        &mut self,
        id: StableNodeId,
    ) -> bevy_ecs::world::Mut<'_, T> {
        self.world
            .get_mut::<T>(self.entities[&id])
            .expect("entity must have runtime component")
    }

    fn validate_pointer_target(
        &self,
        document: DocumentId,
        target: StableNodeId,
    ) -> Result<(), UiWorldError> {
        let entity = *self
            .entities
            .get(&target)
            .ok_or(UiWorldError::MissingNode(target))?;
        let identity = self
            .world
            .get::<Identity>(entity)
            .expect("runtime entity must have identity");
        if identity.document != document {
            return Err(UiWorldError::PointerDocument { document, target });
        }
        let interaction = self
            .world
            .get::<InteractionState>(entity)
            .expect("runtime entity must have interaction state");
        if !interaction.pointer_events {
            return Err(UiWorldError::NotPointerInteractive(target));
        }
        Ok(())
    }

    fn mark_interaction_style(&mut self, id: StableNodeId) {
        if !self.component::<NodeStyle>(id).interaction.is_empty() {
            self.mark(id, DirtyMask::STYLE | DirtyMask::RENDER);
        }
    }

    fn remove_ime(&mut self, id: StableNodeId) {
        let entity = self.entities[&id];
        self.world.entity_mut(entity).remove::<ImeComposition>();
    }

    fn clear_overlay_references(&mut self, removed: StableNodeId) {
        let updates = self
            .entities
            .iter()
            .filter_map(|(&host, &entity)| {
                (host != removed)
                    .then(|| self.world.get::<OverlayHostState>(entity).copied())
                    .flatten()
                    .and_then(|mut state| {
                        let previous = state;
                        let restore_focus = (state.active == Some(removed))
                            .then_some(state.restore_focus)
                            .flatten();
                        if state.active == Some(removed) {
                            state.active = None;
                            state.restore_focus = None;
                        }
                        if state.restore_focus == Some(removed) {
                            state.restore_focus = None;
                        }
                        (state != previous).then_some((host, entity, state, restore_focus))
                    })
            })
            .collect::<Vec<_>>();
        for (host, entity, state, restore_focus) in updates {
            self.world.entity_mut(entity).insert(state);
            self.mark(host, DirtyMask::ACCESSIBILITY);
            let document = self.component::<Identity>(host).document;
            if let Some(restore_focus) = restore_focus.filter(|id| {
                self.contains(*id)
                    && self.component::<Identity>(*id).document == document
                    && self.component::<InteractionState>(*id).focusable
                    && self.component::<ComputedStyle>(*id).visible
            }) {
                self.focused.insert(document, restore_focus);
                self.mark(
                    restore_focus,
                    DirtyMask::FOCUS_IME | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                );
            }
        }
    }

    fn resolve_style(
        &mut self,
        id: StableNodeId,
        resolved: &mut HashSet<StableNodeId>,
    ) -> Result<(), UiWorldError> {
        if !self.contains(id) {
            return Err(UiWorldError::MissingNode(id));
        }
        if !resolved.insert(id) {
            return Ok(());
        }
        let parent = self.component::<Hierarchy>(id).parent;
        if let Some(parent) = parent {
            self.resolve_style(parent, resolved)?;
        }
        let local = self.component::<NodeStyle>(id).clone();
        let inherited = parent
            .map(|parent| self.component::<ComputedStyle>(parent).clone())
            .unwrap_or_default();
        let layout = local.layout.as_ref();
        let mut paint = crate::SemanticPaint {
            foreground: local.foreground,
            background: local.background,
            border: local.border,
        };
        let accessibility = self.component::<AccessibilityState>(id);
        let selected = accessibility.checked == Some(true) || accessibility.selected == Some(true);
        if selected {
            paint = paint.overlay(local.interaction.selected);
        }
        if self.pointer_hover.values().any(|target| *target == id) {
            paint = paint.overlay(
                if selected && !local.interaction.selected_hovered.is_empty() {
                    local.interaction.selected_hovered
                } else {
                    local.interaction.hovered
                },
            );
        }
        if self.pointer_press.values().any(|target| *target == id) {
            paint = paint.overlay(
                if selected && !local.interaction.selected_pressed.is_empty() {
                    local.interaction.selected_pressed
                } else {
                    local.interaction.pressed
                },
            );
        }
        let identity = self.component::<Identity>(id);
        if self.focused.get(&identity.document) == Some(&id) {
            paint = paint.overlay(local.interaction.focused);
        }
        if accessibility.disabled {
            paint = paint.overlay(local.interaction.disabled);
        }
        let foreground = paint.foreground.unwrap_or(inherited.foreground);
        let palette = self.style_model.palette;
        *self.component_mut::<ComputedStyle>(id) = ComputedStyle {
            foreground,
            color: layout.color.or_else(|| {
                paint
                    .foreground
                    .map(|role| palette.get(role).as_rgba_array())
                    .or(inherited.color)
                    .or_else(|| Some(palette.get(foreground).as_rgba_array()))
            }),
            background: layout.background.or_else(|| {
                paint
                    .background
                    .map(|role| palette.get(role).as_rgba_array())
            }),
            border_color: layout
                .border_color
                .or_else(|| paint.border.map(|role| palette.get(role).as_rgba_array())),
            opacity: layout.opacity.unwrap_or(1.0) * inherited.opacity,
            visible: !layout.hidden && inherited.visible && self.overlay_branch_active(id),
            font_size: layout.font_size.unwrap_or(inherited.font_size),
            font_weight: layout.font_weight.or(inherited.font_weight),
            font_family: layout
                .font_family
                .as_deref()
                .map(Arc::<str>::from)
                .or(inherited.font_family),
            line_height: layout.line_height.or(inherited.line_height),
            letter_spacing: layout.letter_spacing.unwrap_or(inherited.letter_spacing),
        };
        Ok(())
    }

    fn overlay_branch_active(&self, id: StableNodeId) -> bool {
        let Some(parent) = self.component::<Hierarchy>(id).parent else {
            return true;
        };
        self.overlay_host(parent)
            .is_none_or(|state| state.active == Some(id))
    }

    pub fn document_order(&self, document: DocumentId) -> Vec<StableNodeId> {
        let mut roots = self
            .entities
            .keys()
            .copied()
            .filter(|id| {
                let identity = self.component::<Identity>(*id);
                identity.document == document && self.component::<Hierarchy>(*id).parent.is_none()
            })
            .collect::<Vec<_>>();
        roots.sort_unstable();
        let mut order = Vec::new();
        let mut stack = roots.into_iter().rev().collect::<Vec<_>>();
        while let Some(id) = stack.pop() {
            order.push(id);
            stack.extend(
                self.component::<Hierarchy>(id)
                    .children
                    .iter()
                    .rev()
                    .copied(),
            );
        }
        order
    }

    fn hierarchy_mut(&mut self, id: StableNodeId) -> bevy_ecs::world::Mut<'_, Hierarchy> {
        let entity = *self.entities.get(&id).expect("validated node must exist");
        self.world
            .get_mut::<Hierarchy>(entity)
            .expect("entity must have hierarchy")
    }

    fn mark(&mut self, id: StableNodeId, bits: u8) -> bool {
        let entity = self.entities[&id];
        let changed = self
            .world
            .get_mut::<DirtyMask>(entity)
            .expect("entity must have dirty component")
            .insert(bits);
        if changed {
            self.dirty_entities.insert(id);
        }
        changed
    }

    fn mark_subtree(&mut self, root: StableNodeId, bits: u8) {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let children = self.node(id).expect("hierarchy node must exist").children;
            stack.extend(children.iter().rev().copied());
            let _ = self.mark(id, bits);
        }
    }

    fn mark_ancestors(&mut self, start: StableNodeId, bits: u8) {
        let mut current = Some(start);
        while let Some(id) = current {
            current = self
                .identity_and_parent(id)
                .expect("hierarchy node must exist")
                .1;
            if !self.mark(id, bits) {
                break;
            }
        }
    }
}

const IDENTITY_AFFINE: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Debug, Clone)]
struct TextInputPresentationSource {
    text: TextContent,
    placeholder: bool,
    selection: Option<(usize, usize)>,
    caret: usize,
    preedit: Option<(usize, usize)>,
}

fn build_text_input_presentation_source(
    state: &TextInputState,
    ime: Option<&ImeComposition>,
    placeholder: &str,
    secure: bool,
) -> TextInputPresentationSource {
    use unicode_segmentation::UnicodeSegmentation;

    let mask = |value: &str| {
        if secure {
            "•".repeat(value.graphemes(true).count())
        } else {
            value.to_owned()
        }
    };
    let display_offset = |value: &str, offset: usize| {
        if secure {
            value[..offset].graphemes(true).count() * "•".len()
        } else {
            offset
        }
    };
    if state.value.is_empty() && ime.is_none() && !placeholder.is_empty() {
        return TextInputPresentationSource {
            text: TextContent {
                value: placeholder.to_owned(),
            },
            placeholder: true,
            selection: None,
            caret: 0,
            preedit: None,
        };
    }

    let selection = if state.selection.is_valid_for(&state.value) {
        state.selection
    } else {
        crate::TextSelection::caret(state.value.len())
    };
    if let Some(ime) = ime {
        let replaced = selection.ordered();
        let prefix = mask(&state.value[..replaced.start]);
        let suffix = mask(&state.value[replaced.end..]);
        let preedit_start = prefix.len();
        let preedit_end = preedit_start + ime.text.len();
        let ime_focus = ime
            .selection
            .map(|(_, focus)| focus)
            .filter(|focus| *focus <= ime.text.len() && ime.text.is_char_boundary(*focus))
            .unwrap_or(ime.text.len());
        return TextInputPresentationSource {
            text: TextContent {
                value: format!("{prefix}{}{suffix}", ime.text),
            },
            placeholder: false,
            selection: None,
            caret: preedit_start + ime_focus,
            preedit: Some((preedit_start, preedit_end)),
        };
    }

    let anchor = display_offset(&state.value, selection.anchor);
    let focus = display_offset(&state.value, selection.focus);
    TextInputPresentationSource {
        text: TextContent {
            value: mask(&state.value),
        },
        placeholder: false,
        selection: (anchor != focus).then_some((anchor.min(focus), anchor.max(focus))),
        caret: focus,
        preedit: None,
    }
}

fn shape_text_input_presentation(
    id: StableNodeId,
    source: TextInputPresentationSource,
    style: &ComputedStyle,
    shaper: &mut impl TextShaper,
) -> TextInputPresentation {
    TextInputPresentation {
        display_value: source.text.value.clone(),
        placeholder: source.placeholder,
        selection: source.selection.map(|(start, end)| {
            (
                shaper.horizontal_offset(id, &source.text, start, style),
                shaper.horizontal_offset(id, &source.text, end, style),
            )
        }),
        caret_x: shaper.horizontal_offset(id, &source.text, source.caret, style),
        preedit: source.preedit.map(|(start, end)| {
            (
                shaper.horizontal_offset(id, &source.text, start, style),
                shaper.horizontal_offset(id, &source.text, end, style),
            )
        }),
    }
}

fn validate_text_metrics(id: StableNodeId, metrics: TextMetrics) -> Result<(), UiWorldError> {
    if !metrics.width.is_finite()
        || !metrics.height.is_finite()
        || metrics.width < 0.0
        || metrics.height < 0.0
    {
        return Err(UiWorldError::InvalidText(id));
    }
    Ok(())
}

fn then_affine([a, b, c, d, e, f]: [f32; 6], rhs: [f32; 6]) -> [f32; 6] {
    let [ra, rb, rc, rd, re, rf] = rhs;
    [
        a * ra + c * rb,
        b * ra + d * rb,
        a * rc + c * rd,
        b * rc + d * rd,
        a * re + c * rf + e,
        b * re + d * rf + f,
    ]
}

fn transformed_contains(bounds: LayoutBox, [a, b, c, d, e, f]: [f32; 6], x: f32, y: f32) -> bool {
    let determinant = a * d - b * c;
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return false;
    }
    let translated_x = x - e;
    let translated_y = y - f;
    let local_x = (d * translated_x - c * translated_y) / determinant;
    let local_y = (-b * translated_x + a * translated_y) / determinant;
    bounds.contains(local_x, local_y)
}

fn layout_semantics_changed(
    previous: &nana_ui_core::LayoutStyle,
    next: &nana_ui_core::LayoutStyle,
) -> bool {
    previous.direction != next.direction
        || previous.flex_reverse != next.flex_reverse
        || previous.order != next.order
        || previous.flex_wrap != next.flex_wrap
        || previous.display != next.display
        || previous.box_sizing != next.box_sizing
        || previous.position != next.position
        || previous.gap != next.gap
        || previous.row_gap != next.row_gap
        || previous.column_gap != next.column_gap
        || previous.padding != next.padding
        || previous.padding_top != next.padding_top
        || previous.padding_right != next.padding_right
        || previous.padding_bottom != next.padding_bottom
        || previous.padding_left != next.padding_left
        || previous.margin != next.margin
        || previous.margin_top != next.margin_top
        || previous.margin_right != next.margin_right
        || previous.margin_bottom != next.margin_bottom
        || previous.margin_left != next.margin_left
        || previous.offset_top != next.offset_top
        || previous.offset_right != next.offset_right
        || previous.offset_bottom != next.offset_bottom
        || previous.offset_left != next.offset_left
        || previous.width != next.width
        || previous.height != next.height
        || previous.min_width != next.min_width
        || previous.max_width != next.max_width
        || previous.min_height != next.min_height
        || previous.max_height != next.max_height
        || previous.allow_shrink != next.allow_shrink
        || previous.align_items != next.align_items
        || previous.align_self != next.align_self
        || previous.align_content != next.align_content
        || previous.justify_content != next.justify_content
        || previous.justify_items != next.justify_items
        || previous.justify_self != next.justify_self
        || previous.flex_grow != next.flex_grow
        || previous.flex_shrink != next.flex_shrink
        || previous.flex_basis != next.flex_basis
        || previous.overflow_x != next.overflow_x
        || previous.overflow_y != next.overflow_y
        || previous.text_overflow_ellipsis != next.text_overflow_ellipsis
        || previous.white_space_nowrap != next.white_space_nowrap
        || previous.grid_columns != next.grid_columns
        || previous.grid_rows != next.grid_rows
        || previous.grid_columns_unsupported != next.grid_columns_unsupported
        || previous.grid_rows_unsupported != next.grid_rows_unsupported
        || previous.grid_auto_columns != next.grid_auto_columns
        || previous.grid_auto_rows != next.grid_auto_rows
        || previous.grid_auto_flow != next.grid_auto_flow
        || previous.border_width != next.border_width
}

struct ValidationPlan<'a> {
    source: &'a UiWorld,
    nodes: HashMap<StableNodeId, PlannedNode>,
    removed: HashSet<StableNodeId>,
    newly_retired: HashSet<StableNodeId>,
    interactions: HashMap<StableNodeId, InteractionState>,
    styles: HashMap<StableNodeId, NodeStyle>,
    focus: HashMap<DocumentId, Option<StableNodeId>>,
    pointer_captures: HashMap<(DocumentId, u64), StableNodeId>,
    animations: HashMap<AnimationId, AnimationSpec>,
    text_inputs: HashMap<StableNodeId, Option<TextInputState>>,
    overlay_hosts: HashMap<StableNodeId, OverlayHostState>,
    accessibility: HashMap<StableNodeId, AccessibilityState>,
}

impl<'a> ValidationPlan<'a> {
    fn new(source: &'a UiWorld) -> Self {
        Self {
            source,
            nodes: HashMap::new(),
            removed: HashSet::new(),
            newly_retired: HashSet::new(),
            interactions: HashMap::new(),
            styles: HashMap::new(),
            focus: HashMap::new(),
            pointer_captures: source.pointer_captures.clone(),
            animations: source
                .animations
                .iter()
                .map(|(&id, animation)| (id, animation.spec))
                .collect(),
            text_inputs: HashMap::new(),
            overlay_hosts: HashMap::new(),
            accessibility: HashMap::new(),
        }
    }

    fn validate(mut self, mutations: &[UiMutation]) -> Result<(), UiWorldError> {
        for mutation in mutations {
            match mutation {
                UiMutation::Create { id, document, .. } => self.create(*id, *document)?,
                UiMutation::Insert {
                    parent,
                    child,
                    before,
                } => self.insert(*parent, *child, *before)?,
                UiMutation::Remove { id } => self.detach(*id)?,
                UiMutation::DespawnSubtree { root } => self.despawn_subtree(*root)?,
                UiMutation::SetStyle { id, style } => {
                    self.node(*id)?;
                    let layout = style.layout.as_ref();
                    if layout.opacity.is_some_and(|opacity| {
                        !opacity.is_finite() || !(0.0..=1.0).contains(&opacity)
                    }) || layout
                        .font_size
                        .is_some_and(|size| !size.is_finite() || size <= 0.0)
                        || layout
                            .letter_spacing
                            .is_some_and(|spacing| !spacing.is_finite())
                        || layout
                            .font_weight
                            .is_some_and(|weight| !(1..=1000).contains(&weight))
                        || layout.color.is_some_and(|color| {
                            color.into_iter().any(|channel| {
                                !channel.is_finite() || !(0.0..=1.0).contains(&channel)
                            })
                        })
                        || layout.background.is_some_and(|color| {
                            color.into_iter().any(|channel| {
                                !channel.is_finite() || !(0.0..=1.0).contains(&channel)
                            })
                        })
                        || layout.border_color.is_some_and(|color| {
                            color.into_iter().any(|channel| {
                                !channel.is_finite() || !(0.0..=1.0).contains(&channel)
                            })
                        })
                    {
                        return Err(UiWorldError::InvalidStyle(*id));
                    }
                    self.styles.insert(*id, style.clone());
                }
                UiMutation::SetTheme { .. } => {}
                UiMutation::SetText { id, .. } => {
                    self.node(*id)?;
                }
                UiMutation::WriteLayout { id, layout } => {
                    self.node(*id)?;
                    if !layout.x.is_finite()
                        || !layout.y.is_finite()
                        || !layout.width.is_finite()
                        || !layout.height.is_finite()
                        || layout.width < 0.0
                        || layout.height < 0.0
                    {
                        return Err(UiWorldError::InvalidLayout(*id));
                    }
                }
                UiMutation::SetScrollOffset { id, offset } => {
                    self.node(*id)?;
                    if !offset.x.is_finite()
                        || !offset.y.is_finite()
                        || offset.x < 0.0
                        || offset.y < 0.0
                    {
                        return Err(UiWorldError::InvalidScrollOffset(*id));
                    }
                }
                UiMutation::SetScrollMetrics { id, metrics } => {
                    self.node(*id)?;
                    if metrics.is_some_and(|metrics| {
                        [
                            metrics.viewport_width,
                            metrics.viewport_height,
                            metrics.content_width,
                            metrics.content_height,
                        ]
                        .into_iter()
                        .any(|extent| !extent.is_finite() || extent < 0.0)
                    }) {
                        return Err(UiWorldError::InvalidScrollMetrics(*id));
                    }
                }
                UiMutation::SetInteraction { id, interaction } => {
                    self.node(*id)?;
                    self.interactions.insert(*id, *interaction);
                }
                UiMutation::SetCustomRender { id, content } => {
                    self.node(*id)?;
                    if content.as_ref().is_some_and(|content| {
                        content.renderer.trim().is_empty() || content.resource.trim().is_empty()
                    }) {
                        return Err(UiWorldError::InvalidCustomRender(*id));
                    }
                }
                UiMutation::SetStandardVisual { id, visual } => {
                    self.node(*id)?;
                    if matches!(visual, Some(StandardVisual::Slider { ratio }) if !ratio.is_finite() || !(0.0..=1.0).contains(ratio))
                    {
                        return Err(UiWorldError::InvalidStandardVisual(*id));
                    }
                }
                UiMutation::SetAccessibility { id, accessibility } => {
                    self.node(*id)?;
                    self.accessibility.insert(*id, accessibility.clone());
                }
                UiMutation::SetOverlayHost { host, state } => {
                    let host_document = self.node(*host)?.document;
                    if let Some(active) = state.active {
                        let active_node = self.node(active)?;
                        if active_node.parent != Some(*host) {
                            return Err(UiWorldError::InvalidOverlayHost(*host));
                        }
                    }
                    if let Some(restore_focus) = state.restore_focus
                        && self.node(restore_focus)?.document != host_document
                    {
                        return Err(UiWorldError::FocusDocument {
                            document: host_document,
                            target: restore_focus,
                        });
                    }
                    self.overlay_hosts.insert(*host, *state);
                }
                UiMutation::CapturePointer { pointer_id, target } => {
                    let document = self.node(*target)?.document;
                    self.pointer_captures
                        .insert((document, *pointer_id), *target);
                }
                UiMutation::ReleasePointer { pointer_id, target } => {
                    let document = self.node(*target)?.document;
                    if self.pointer_captures.get(&(document, *pointer_id)) != Some(target) {
                        return Err(UiWorldError::PointerCaptureMismatch {
                            pointer_id: *pointer_id,
                            target: *target,
                        });
                    }
                    self.pointer_captures.remove(&(document, *pointer_id));
                }
                UiMutation::StartAnimation { animation } => {
                    self.node(animation.target)?;
                    if !animation.is_valid() {
                        return Err(UiWorldError::InvalidAnimation(animation.id));
                    }
                    self.animations.insert(animation.id, *animation);
                }
                UiMutation::StopAnimation { id } => {
                    if self.animations.remove(id).is_none() {
                        return Err(UiWorldError::MissingAnimation(*id));
                    }
                }
                UiMutation::RequestFocus { document, target } => {
                    if let Some(target) = target {
                        let node = self.node(*target)?;
                        if node.document != *document {
                            return Err(UiWorldError::FocusDocument {
                                document: *document,
                                target: *target,
                            });
                        }
                        let interaction =
                            self.interactions.get(target).copied().unwrap_or_else(|| {
                                self.source
                                    .entities
                                    .get(target)
                                    .map(|_| *self.source.component::<InteractionState>(*target))
                                    .unwrap_or_default()
                            });
                        let visible = self.focus_target_visible(*target)?;
                        if !interaction.focusable
                            || !visible
                            || !self.active_modal_allows_focus(*document, *target)?
                        {
                            return Err(UiWorldError::NotFocusable(*target));
                        }
                    }
                    self.focus.insert(*document, *target);
                }
                UiMutation::SetIme { id, composition } => {
                    let document = self.node(*id)?.document;
                    let focused = self
                        .focus
                        .get(&document)
                        .copied()
                        .unwrap_or_else(|| self.source.focused(document));
                    if focused != Some(*id) {
                        return Err(UiWorldError::NotFocused(*id));
                    }
                    if composition.is_some() {
                        self.text_input(*id)?;
                    }
                    if let Some(ImeComposition {
                        text,
                        selection: Some((start, end)),
                    }) = composition
                        && (start > end
                            || *end > text.len()
                            || !text.is_char_boundary(*start)
                            || !text.is_char_boundary(*end))
                    {
                        return Err(UiWorldError::InvalidIme(*id));
                    }
                }
                UiMutation::SetTextInput { id, state } => {
                    self.node(*id)?;
                    if state
                        .as_ref()
                        .is_some_and(|state| !state.selection.is_valid_for(&state.value))
                    {
                        return Err(UiWorldError::InvalidTextInput(*id));
                    }
                    self.text_inputs.insert(*id, state.clone());
                }
                UiMutation::SetTextSelection { id, selection } => {
                    let mut state = self.text_input(*id)?;
                    if !selection.is_valid_for(&state.value) {
                        return Err(UiWorldError::InvalidTextInput(*id));
                    }
                    state.selection = *selection;
                    self.text_inputs.insert(*id, Some(state));
                }
                UiMutation::ReplaceTextSelection { id, text } => {
                    let mut state = self.text_input(*id)?;
                    if !state.replace_selection(text) {
                        return Err(UiWorldError::InvalidTextInput(*id));
                    }
                    self.text_inputs.insert(*id, Some(state));
                }
            }
        }
        self.validate_overlay_hosts()?;
        Ok(())
    }

    fn create(&mut self, id: StableNodeId, document: DocumentId) -> Result<(), UiWorldError> {
        if self.exists(id) {
            return Err(UiWorldError::DuplicateNode(id));
        }
        if self.source.retired.contains(&id) || self.newly_retired.contains(&id) {
            return Err(UiWorldError::RetiredNode(id));
        }
        self.removed.remove(&id);
        self.nodes.insert(
            id,
            PlannedNode {
                document,
                parent: None,
                children: Vec::new(),
            },
        );
        Ok(())
    }

    fn insert(
        &mut self,
        parent: StableNodeId,
        child: StableNodeId,
        before: Option<StableNodeId>,
    ) -> Result<(), UiWorldError> {
        let parent_document = self.node(parent)?.document;
        let child_node = self.node(child)?.clone();
        if child_node.document != parent_document {
            return Err(UiWorldError::CrossDocument { parent, child });
        }
        if parent == child || self.has_ancestor(parent, child)? {
            return Err(UiWorldError::Cycle { parent, child });
        }
        if before == Some(child) && child_node.parent == Some(parent) {
            return Ok(());
        }
        if let Some(before) = before
            && !self.node(parent)?.children.contains(&before)
        {
            return Err(UiWorldError::InvalidBefore { parent, before });
        }
        self.detach(child)?;
        let siblings = &mut self.node_mut(parent)?.children;
        let index = before
            .and_then(|before| siblings.iter().position(|id| *id == before))
            .unwrap_or(siblings.len());
        siblings.insert(index, child);
        self.node_mut(child)?.parent = Some(parent);
        Ok(())
    }

    fn detach(&mut self, id: StableNodeId) -> Result<(), UiWorldError> {
        let parent = self.node(id)?.parent;
        if let Some(parent) = parent {
            self.node_mut(parent)?.children.retain(|child| *child != id);
            self.node_mut(id)?.parent = None;
        }
        Ok(())
    }

    fn despawn_subtree(&mut self, root: StableNodeId) -> Result<(), UiWorldError> {
        self.detach(root)?;
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let children = self.node(id)?.children.clone();
            stack.extend(children);
            self.nodes.remove(&id);
            self.removed.insert(id);
            self.newly_retired.insert(id);
            self.pointer_captures.retain(|_, target| *target != id);
            self.animations
                .retain(|_, animation| animation.target != id);
            self.text_inputs.remove(&id);
            self.clear_overlay_references(id);
        }
        Ok(())
    }

    fn clear_overlay_references(&mut self, removed: StableNodeId) {
        let hosts = self
            .source
            .entities
            .keys()
            .copied()
            .chain(self.overlay_hosts.keys().copied())
            .collect::<HashSet<_>>();
        for host in hosts {
            if host == removed || !self.exists(host) {
                continue;
            }
            let Some(mut state) = self
                .overlay_hosts
                .get(&host)
                .copied()
                .or_else(|| self.source.overlay_host(host))
            else {
                continue;
            };
            if state.active == Some(removed) {
                state.active = None;
                state.restore_focus = None;
            }
            if state.restore_focus == Some(removed) {
                state.restore_focus = None;
            }
            self.overlay_hosts.insert(host, state);
        }
    }

    fn validate_overlay_hosts(&mut self) -> Result<(), UiWorldError> {
        let hosts = self
            .source
            .entities
            .keys()
            .copied()
            .chain(self.overlay_hosts.keys().copied())
            .collect::<HashSet<_>>();
        for host in hosts {
            if !self.exists(host) {
                continue;
            }
            let Some(state) = self
                .overlay_hosts
                .get(&host)
                .copied()
                .or_else(|| self.source.overlay_host(host))
            else {
                continue;
            };
            let host_document = self.node(host)?.document;
            if let Some(active) = state.active
                && (!self.exists(active) || self.node(active)?.parent != Some(host))
            {
                return Err(UiWorldError::InvalidOverlayHost(host));
            }
            if let Some(restore_focus) = state.restore_focus
                && (!self.exists(restore_focus)
                    || self.node(restore_focus)?.document != host_document)
            {
                return Err(UiWorldError::InvalidOverlayHost(host));
            }
        }
        Ok(())
    }

    fn has_ancestor(
        &mut self,
        mut id: StableNodeId,
        candidate: StableNodeId,
    ) -> Result<bool, UiWorldError> {
        let mut visited = HashSet::new();
        loop {
            if id == candidate {
                return Ok(true);
            }
            if !visited.insert(id) {
                return Ok(false);
            }
            let Some(parent) = self.node(id)?.parent else {
                return Ok(false);
            };
            id = parent;
        }
    }

    fn exists(&self, id: StableNodeId) -> bool {
        !self.removed.contains(&id) && (self.nodes.contains_key(&id) || self.source.contains(id))
    }

    fn text_input(&mut self, id: StableNodeId) -> Result<TextInputState, UiWorldError> {
        self.node(id)?;
        if let Some(state) = self.text_inputs.get(&id) {
            return state.clone().ok_or(UiWorldError::MissingTextInput(id));
        }
        self.source
            .text_input(id)
            .cloned()
            .ok_or(UiWorldError::MissingTextInput(id))
    }

    fn overlay_branch_active(&mut self, id: StableNodeId) -> Result<bool, UiWorldError> {
        let Some(parent) = self.node(id)?.parent else {
            return Ok(true);
        };
        let state = self
            .overlay_hosts
            .get(&parent)
            .copied()
            .or_else(|| self.source.overlay_host(parent));
        Ok(state.is_none_or(|state| state.active == Some(id)))
    }

    fn focus_target_visible(&mut self, mut id: StableNodeId) -> Result<bool, UiWorldError> {
        loop {
            let hidden = self
                .styles
                .get(&id)
                .map(|style| style.layout.hidden)
                .unwrap_or_else(|| {
                    self.source
                        .node_style(id)
                        .is_some_and(|style| style.layout.hidden)
                });
            if hidden || !self.overlay_branch_active(id)? {
                return Ok(false);
            }
            let Some(parent) = self.node(id)?.parent else {
                return Ok(true);
            };
            id = parent;
        }
    }

    fn active_modal_allows_focus(
        &mut self,
        document: DocumentId,
        target: StableNodeId,
    ) -> Result<bool, UiWorldError> {
        let hosts = self
            .source
            .entities
            .keys()
            .copied()
            .chain(self.overlay_hosts.keys().copied())
            .collect::<HashSet<_>>();
        for host in hosts {
            let Some(state) = self
                .overlay_hosts
                .get(&host)
                .copied()
                .or_else(|| self.source.overlay_host(host))
            else {
                continue;
            };
            let Some(active) = state.active else {
                continue;
            };
            if self.node(host)?.document != document {
                continue;
            }
            let modal = self
                .accessibility
                .get(&active)
                .or_else(|| self.source.accessibility(active))
                .is_some_and(|state| state.modal);
            if modal && !self.has_ancestor(target, active)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn node(&mut self, id: StableNodeId) -> Result<&PlannedNode, UiWorldError> {
        self.ensure(id)?;
        Ok(self.nodes.get(&id).expect("ensured node must exist"))
    }

    fn node_mut(&mut self, id: StableNodeId) -> Result<&mut PlannedNode, UiWorldError> {
        self.ensure(id)?;
        Ok(self.nodes.get_mut(&id).expect("ensured node must exist"))
    }

    fn ensure(&mut self, id: StableNodeId) -> Result<(), UiWorldError> {
        if self.removed.contains(&id) {
            return Err(UiWorldError::MissingNode(id));
        }
        if self.nodes.contains_key(&id) {
            return Ok(());
        }
        let snapshot = self.source.node(id).ok_or(UiWorldError::MissingNode(id))?;
        self.nodes.insert(
            id,
            PlannedNode {
                document: snapshot.document,
                parent: snapshot.parent,
                children: snapshot.children,
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Easing;
    use nana_ui_core::{LayoutStyle, LengthSpec, OverflowSpec, PaintTransform, SemanticColorRole};

    fn node(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }

    fn document(value: u64) -> DocumentId {
        DocumentId::new(value).unwrap()
    }

    #[test]
    fn batch_builds_reparents_and_detaches_hierarchy() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=4 {
            queue.create(
                node(id),
                document(1),
                NodeKind::Element { tag: "div".into() },
            );
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        queue.insert(node(1), node(4), Some(node(3)));
        let report = world.commit(queue).unwrap();
        assert_eq!(report.created, 4);
        assert_eq!(report.inserted, 3);
        assert_eq!(report.reparented, 0);
        assert_eq!(
            world.node(node(1)).unwrap().children,
            vec![node(2), node(4), node(3)]
        );

        let mut queue = MutationQueue::new();
        queue.insert(node(2), node(3), None);
        queue.remove(node(4));
        let report = world.commit(queue).unwrap();
        assert_eq!(report.reparented, 1);
        assert_eq!(report.detached, 1);
        assert_eq!(world.node(node(1)).unwrap().children, vec![node(2)]);
        assert_eq!(world.node(node(2)).unwrap().children, vec![node(3)]);
        assert_eq!(world.node(node(4)).unwrap().parent, None);
    }

    #[test]
    fn invalid_batch_is_atomic() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        world.commit(queue).unwrap();

        let mut queue = MutationQueue::new();
        queue.create(node(2), document(1), NodeKind::Text);
        queue.insert(node(99), node(2), None);
        assert_eq!(
            world.commit(queue),
            Err(UiWorldError::MissingNode(node(99)))
        );
        assert!(!world.contains(node(2)));
        assert_eq!(world.len(), 1);
    }

    #[test]
    fn committed_text_selection_is_unicode_safe_and_batch_atomic() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element {
                tag: "input".into(),
            },
        );
        queue.set_text_input(node(1), Some(TextInputState::new("你好ab")));
        queue.set_text_selection(
            node(1),
            crate::TextSelection {
                anchor: 0,
                focus: "你".len(),
            },
        );
        queue.replace_text_selection(node(1), "娜");
        world.commit(queue).unwrap();
        let state = world.text_input(node(1)).unwrap();
        assert_eq!(state.value, "娜好ab");
        assert_eq!(state.selection, crate::TextSelection::caret("娜".len()));

        let generation = world.generation();
        let mut invalid = MutationQueue::new();
        invalid.set_text_selection(
            node(1),
            crate::TextSelection {
                anchor: 1,
                focus: 1,
            },
        );
        assert_eq!(
            world.commit(invalid),
            Err(UiWorldError::InvalidTextInput(node(1)))
        );
        assert_eq!(world.generation(), generation);
        assert_eq!(world.text_input(node(1)).unwrap().value, "娜好ab");
        assert_eq!(world.text(node(1)), Some("娜好ab"));

        let accessibility = world.project_accessibility(document(1));
        assert_eq!(accessibility[0].value.as_deref(), Some("娜好ab"));
        assert_eq!(
            world.extract_document(document(1))[0]
                .text_input
                .as_ref()
                .unwrap()
                .selection,
            crate::TextSelection::caret("娜".len())
        );
    }

    #[test]
    fn text_input_presentation_masks_graphemes_and_replaces_selection_with_preedit() {
        let value = "A👩‍💻界";
        let state = TextInputState {
            value: value.into(),
            selection: crate::TextSelection {
                anchor: "A".len(),
                focus: "A👩‍💻".len(),
            },
        };
        let masked = build_text_input_presentation_source(&state, None, "", true);
        assert_eq!(masked.text.value, "•••");
        assert_eq!(masked.selection, Some(("•".len(), "••".len())));

        let preedit = build_text_input_presentation_source(
            &state,
            Some(&ImeComposition {
                text: "输入".into(),
                selection: Some((0, "输".len())),
            }),
            "",
            true,
        );
        assert_eq!(preedit.text.value, "•输入•");
        assert_eq!(preedit.preedit, Some(("•".len(), "•输入".len())));
        assert_eq!(preedit.caret, "•输".len());
    }

    #[test]
    fn animations_are_atomic_deadline_driven_and_replaceable() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(node(2), document(1), NodeKind::Text);
        world.commit(queue).unwrap();

        let animation_id = AnimationId::new(1).unwrap();
        let missing_id = AnimationId::new(2).unwrap();
        let animation = AnimationSpec {
            id: animation_id,
            target: node(1),
            start: Duration::from_millis(100),
            duration: Duration::from_millis(100),
            frame_interval: Duration::from_millis(16),
            easing: Easing::EaseOutCubic,
        };
        let generation = world.generation();
        let mut invalid = MutationQueue::new();
        invalid.start_animation(animation);
        invalid.stop_animation(missing_id);
        assert_eq!(
            world.commit(invalid),
            Err(UiWorldError::MissingAnimation(missing_id))
        );
        assert_eq!(world.generation(), generation);
        assert_eq!(world.next_animation_deadline(), None);

        let mut invalid_timing = MutationQueue::new();
        invalid_timing.start_animation(AnimationSpec {
            duration: Duration::ZERO,
            ..animation
        });
        assert_eq!(
            world.commit(invalid_timing),
            Err(UiWorldError::InvalidAnimation(animation_id))
        );
        assert_eq!(world.generation(), generation);

        let mut start = MutationQueue::new();
        start.start_animation(animation);
        world.commit(start).unwrap();
        assert_eq!(
            world.next_animation_deadline(),
            Some(Duration::from_millis(100))
        );
        assert!(
            world
                .advance_animations(Duration::from_millis(99))
                .samples
                .is_empty()
        );
        let first = world.advance_animations(Duration::from_millis(100));
        assert_eq!(first.samples.len(), 1);
        assert_eq!(first.samples[0].progress, 0.0);
        assert_eq!(first.next_deadline, Some(Duration::from_millis(116)));

        let replacement = AnimationSpec {
            target: node(2),
            start: Duration::from_millis(150),
            easing: Easing::Linear,
            ..animation
        };
        let mut replace = MutationQueue::new();
        replace.start_animation(replacement);
        world.commit(replace).unwrap();
        let middle = world.advance_animations(Duration::from_millis(200));
        assert_eq!(middle.samples.len(), 1);
        assert_eq!(middle.samples[0].target, node(2));
        assert_eq!(middle.samples[0].progress, 0.5);
        assert!(!middle.samples[0].finished);

        let end = world.advance_animations(Duration::from_millis(250));
        assert_eq!(end.samples.len(), 1);
        assert_eq!(end.samples[0].progress, 1.0);
        assert!(end.samples[0].finished);
        assert_eq!(end.next_deadline, None);

        let mut start_then_stop = MutationQueue::new();
        start_then_stop.start_animation(animation);
        start_then_stop.stop_animation(animation_id);
        world.commit(start_then_stop).unwrap();
        assert_eq!(world.next_animation_deadline(), None);
    }

    #[test]
    fn despawning_animation_target_cancels_its_wakeup() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(node(2), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.start_animation(AnimationSpec {
            id: AnimationId::new(1).unwrap(),
            target: node(2),
            start: Duration::from_millis(10),
            duration: Duration::from_secs(1),
            frame_interval: Duration::from_millis(16),
            easing: Easing::Linear,
        });
        world.commit(queue).unwrap();
        assert_eq!(
            world.next_animation_deadline(),
            Some(Duration::from_millis(10))
        );

        let mut remove = MutationQueue::new();
        remove.despawn_subtree(node(1));
        world.commit(remove).unwrap();
        assert_eq!(world.next_animation_deadline(), None);
        assert!(world.advance_animations(Duration::MAX).samples.is_empty());
    }

    #[test]
    fn subtree_despawn_retires_ids_and_stale_handles_do_not_alias() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=3 {
            queue.create(node(id), document(1), NodeKind::Text);
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut queue = MutationQueue::new();
        queue.despawn_subtree(node(2));
        let report = world.commit(queue).unwrap();
        assert_eq!(report.despawned, 2);
        assert_eq!(
            world.node(node(1)).unwrap().children,
            Vec::<StableNodeId>::new()
        );
        assert!(!world.contains(node(2)));
        assert!(world.is_retired(node(2)));
        let work = world.take_system_work();
        assert_eq!(work.render_removals, vec![node(2), node(3)]);
        assert_eq!(work.accessibility_removals, vec![node(2), node(3)]);
        let accessibility = world.project_accessibility_delta(&work);
        assert_eq!(accessibility.generation, work.generation);
        assert_eq!(accessibility.removed, vec![node(2), node(3)]);
        assert_eq!(
            accessibility
                .updated
                .iter()
                .map(|node| node.id)
                .collect::<Vec<_>>(),
            vec![node(1)]
        );
        assert!(accessibility.updated[0].children.is_empty());

        let mut queue = MutationQueue::new();
        queue.create(node(2), document(1), NodeKind::Text);
        assert_eq!(world.commit(queue), Err(UiWorldError::RetiredNode(node(2))));
    }

    #[test]
    fn batch_cannot_recreate_an_id_after_despawn() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        world.commit(queue).unwrap();

        let mut queue = MutationQueue::new();
        queue.despawn_subtree(node(1));
        queue.create(node(1), document(1), NodeKind::Document);
        assert_eq!(world.commit(queue), Err(UiWorldError::RetiredNode(node(1))));
        assert!(world.contains(node(1)));
        assert!(!world.is_retired(node(1)));
    }

    #[test]
    fn dirty_work_is_incremental_and_static_world_stays_idle() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=4 {
            queue.create(node(id), document(1), NodeKind::Text);
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        queue.insert(node(3), node(4), None);
        let report = world.commit(queue).unwrap();
        assert_eq!(report.generation, 1);

        let initial = world.take_system_work();
        assert_eq!(initial.generation, 1);
        assert_eq!(initial.style, vec![node(1), node(2), node(3), node(4)]);
        assert_eq!(initial.render_extraction, initial.style);
        assert!(world.take_system_work().is_empty());

        let empty = world.commit(MutationQueue::new()).unwrap();
        assert_eq!(empty.generation, 1);
        assert!(world.take_system_work().is_empty());

        let mut queue = MutationQueue::new();
        queue.insert(node(2), node(3), None);
        let report = world.commit(queue).unwrap();
        assert_eq!(report.generation, 2);
        let work = world.take_system_work();
        assert_eq!(work.style, vec![node(3), node(4)]);
        assert_eq!(work.text, vec![node(3), node(4)]);
        assert_eq!(work.focus_ime, vec![node(3), node(4)]);
        assert_eq!(work.layout, vec![node(1), node(2), node(3), node(4)]);
        assert_eq!(work.render_extraction, work.layout);
        assert!(world.take_system_work().is_empty());
    }

    #[test]
    fn sibling_reorder_does_not_recompute_unchanged_descendant_styles() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=4 {
            queue.create(node(id), document(1), NodeKind::Text);
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        queue.insert(node(2), node(4), None);
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut queue = MutationQueue::new();
        queue.insert(node(1), node(3), Some(node(2)));
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        assert!(work.style.is_empty());
        assert!(work.text.is_empty());
        assert!(work.focus_ime.is_empty());
        assert_eq!(work.input_hit_test, vec![node(3)]);
        assert_eq!(work.layout, vec![node(1)]);
        assert_eq!(work.render_extraction, vec![node(1), node(3)]);
    }

    #[test]
    fn paint_only_style_change_does_not_schedule_subtree_layout() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=3 {
            queue.create(node(id), document(1), NodeKind::Text);
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut paint = MutationQueue::new();
        paint.set_style(
            node(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    color: Some([0.2, 0.4, 0.8, 1.0]),
                    opacity: Some(0.8),
                    ..LayoutStyle::default()
                }),
                foreground: Some(SemanticColorRole::Accent),
                background: None,
                border: None,
                interaction: crate::InteractionStyle::default(),
                ..NodeStyle::default()
            },
        );
        world.commit(paint).unwrap();
        let work = world.take_system_work();
        assert_eq!(work.style, vec![node(1), node(2), node(3)]);
        assert!(work.text.is_empty());
        assert!(work.layout.is_empty());
        assert!(work.input_hit_test.is_empty());
        assert_eq!(work.render_extraction, vec![node(1), node(2), node(3)]);

        let mut layout = MutationQueue::new();
        layout.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Px(240.0)),
                    ..LayoutStyle::default()
                }),
                foreground: None,
                background: None,
                border: None,
                interaction: crate::InteractionStyle::default(),
                ..NodeStyle::default()
            },
        );
        world.commit(layout).unwrap();
        let work = world.take_system_work();
        assert_eq!(work.layout, vec![node(1), node(2), node(3)]);
        assert_eq!(work.input_hit_test, vec![node(2), node(3)]);
    }

    #[test]
    fn hit_test_respects_cumulative_transform_and_ancestor_clip() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        queue.write_layout(
            node(1),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 50.0,
                height: 50.0,
            },
        );
        queue.write_layout(
            node(2),
            LayoutBox {
                x: 40.0,
                y: 0.0,
                width: 30.0,
                height: 30.0,
            },
        );
        queue.set_style(
            node(1),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    overflow_x: OverflowSpec::Hidden,
                    transform: Some(PaintTransform {
                        e: 10.0,
                        ..PaintTransform::default()
                    }),
                    ..LayoutStyle::default()
                }),
                foreground: None,
                background: None,
                border: None,
                interaction: crate::InteractionStyle::default(),
                ..NodeStyle::default()
            },
        );
        queue.set_interaction(
            node(1),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
        );
        world.commit(queue).unwrap();
        world.rebuild_hit_test(document(1));

        assert_eq!(world.hit_test(document(1), 55.0, 10.0), Some(node(2)));
        assert_eq!(world.hit_test(document(1), 45.0, 10.0), None);
        assert_eq!(world.hit_test(document(1), 65.0, 10.0), None);
    }

    #[test]
    fn rejects_cycles_cross_document_parenting_and_invalid_sibling_anchor() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element { tag: "main".into() },
        );
        queue.create(node(3), document(2), NodeKind::Document);
        queue.insert(node(1), node(2), None);
        world.commit(queue).unwrap();

        let mut cycle = MutationQueue::new();
        cycle.insert(node(2), node(1), None);
        assert_eq!(
            world.commit(cycle),
            Err(UiWorldError::Cycle {
                parent: node(2),
                child: node(1)
            })
        );

        let mut cross_document = MutationQueue::new();
        cross_document.insert(node(1), node(3), None);
        assert_eq!(
            world.commit(cross_document),
            Err(UiWorldError::CrossDocument {
                parent: node(1),
                child: node(3)
            })
        );

        let mut invalid_before = MutationQueue::new();
        invalid_before.insert(node(1), node(2), Some(node(3)));
        assert_eq!(
            world.commit(invalid_before),
            Err(UiWorldError::InvalidBefore {
                parent: node(1),
                before: node(3)
            })
        );
    }

    #[derive(Default)]
    struct FunctionalShaper {
        calls: Vec<StableNodeId>,
    }

    impl TextShaper for FunctionalShaper {
        fn shape(
            &mut self,
            id: StableNodeId,
            text: &TextContent,
            style: &ComputedStyle,
            _constraints: crate::TextShapeConstraints,
        ) -> TextMetrics {
            self.calls.push(id);
            TextMetrics {
                width: text.value.chars().count() as f32 * style.font_size,
                height: style.font_size,
            }
        }
    }

    #[test]
    fn style_text_layout_input_focus_ime_hit_test_and_extraction_form_one_pipeline() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "input".into(),
            },
        );
        queue.create(node(3), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    opacity: Some(0.5),
                    z_index: Some(2),
                    ..LayoutStyle::default()
                }),
                foreground: Some(SemanticColorRole::Accent),
                background: None,
                border: None,
                interaction: crate::InteractionStyle::default(),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            node(3),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    opacity: Some(0.5),
                    font_size: Some(20.0),
                    ..LayoutStyle::default()
                }),
                foreground: None,
                background: None,
                border: None,
                interaction: crate::InteractionStyle::default(),
                ..NodeStyle::default()
            },
        );
        queue.set_text(
            node(3),
            TextContent {
                value: "输入".into(),
            },
        );
        queue.set_interaction(
            node(2),
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        world.commit(queue).unwrap();

        let work = world.take_system_work();
        assert_eq!(work.text, vec![node(3)]);
        world.resolve_styles(&work.style).unwrap();
        let mut shaper = FunctionalShaper::default();
        world.shape_text(&work.text, &mut shaper).unwrap();
        assert_eq!(shaper.calls, vec![node(3)]);
        let layout = world.layout_inputs(&work.layout).unwrap();
        let text = layout.iter().find(|input| input.id == node(3)).unwrap();
        assert_eq!(text.parent, Some(node(2)));
        assert_eq!(text.text_metrics.unwrap().width, 40.0);

        let mut queue = MutationQueue::new();
        queue.write_layout(
            node(1),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        );
        queue.write_layout(
            node(2),
            LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 40.0,
            },
        );
        queue.write_layout(
            node(3),
            LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 40.0,
                height: 20.0,
            },
        );
        queue.request_focus(document(1), Some(node(2)));
        queue.set_text_input(node(2), Some(TextInputState::new("")));
        queue.set_ime(
            node(2),
            Some(ImeComposition {
                text: "拼音".into(),
                selection: Some((0, 6)),
            }),
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.reconcile_focus(&work.focus_ime);
        world.rebuild_hit_test(document(1));
        assert_eq!(world.hit_test(document(1), 20.0, 20.0), Some(node(2)));

        let extracted = world.extract_document(document(1));
        let input = extracted.iter().find(|entry| entry.id == node(2)).unwrap();
        let text = extracted.iter().find(|entry| entry.id == node(3)).unwrap();
        assert!(input.focused);
        assert_eq!(input.ime.as_ref().unwrap().text, "拼音");
        assert_eq!(text.style.foreground, SemanticColorRole::Accent);
        assert_eq!(text.style.opacity, 0.25);

        let mut queue = MutationQueue::new();
        queue.set_interaction(
            node(2),
            InteractionState {
                focusable: false,
                ..InteractionState::default()
            },
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.reconcile_focus(&work.focus_ime);
        assert_eq!(world.focused(document(1)), None);
        assert!(
            world
                .extract_document(document(1))
                .iter()
                .find(|entry| entry.id == node(2))
                .unwrap()
                .ime
                .is_none()
        );
    }

    #[test]
    fn invalid_visual_input_is_rejected_atomically() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Text);
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut queue = MutationQueue::new();
        queue.set_text(
            node(1),
            TextContent {
                value: "valid".into(),
            },
        );
        queue.write_layout(
            node(1),
            LayoutBox {
                width: -1.0,
                ..LayoutBox::default()
            },
        );
        assert_eq!(
            world.commit(queue),
            Err(UiWorldError::InvalidLayout(node(1)))
        );
        assert_eq!(world.generation(), 1);
        assert!(world.take_system_work().is_empty());
    }

    #[test]
    fn custom_render_extension_requires_nonempty_backend_neutral_keys() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element { tag: "gpu".into() },
        );
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut queue = MutationQueue::new();
        queue.set_custom_render(
            node(1),
            Some(CustomRenderNode {
                renderer: Arc::from(""),
                resource: Arc::from("program"),
                revision: 0,
            }),
        );
        assert_eq!(
            world.commit(queue),
            Err(UiWorldError::InvalidCustomRender(node(1)))
        );
        assert!(world.custom_render(node(1)).is_none());
        assert!(world.take_system_work().is_empty());
    }

    #[test]
    fn accessibility_projection_uses_runtime_hierarchy_focus_and_geometry() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(
            node(1),
            document(1),
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        queue.create(node(2), document(1), NodeKind::Text);
        queue.create(node(3), document(1), NodeKind::Comment);
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        queue.set_text(
            node(2),
            TextContent {
                value: "Build".into(),
            },
        );
        queue.set_interaction(
            node(1),
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        queue.set_accessibility(
            node(1),
            AccessibilityState {
                role: AccessibilityRole::Button,
                label: Some(Arc::from("Build project")),
                value: None,
                disabled: false,
                checked: None,
                selected: None,
                multiline: false,
                editable: false,
                modal: false,
                ..AccessibilityState::default()
            },
        );
        queue.write_layout(
            node(1),
            LayoutBox {
                x: 10.0,
                y: 20.0,
                width: 80.0,
                height: 28.0,
            },
        );
        queue.request_focus(document(1), Some(node(1)));
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();

        let projected = world.project_accessibility(document(1));
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].role, AccessibilityRole::Button);
        assert_eq!(projected[0].label.as_deref(), Some("Build project"));
        assert!(projected[0].focused);
        assert_eq!(projected[0].children, vec![node(2)]);
        assert_eq!(projected[0].bounds.width, 80.0);
        assert_eq!(projected[1].role, AccessibilityRole::Text);
        assert_eq!(projected[1].label.as_deref(), Some("Build"));

        let mut queue = MutationQueue::new();
        queue.set_accessibility(
            node(1),
            AccessibilityState {
                disabled: true,
                ..world.accessibility(node(1)).unwrap().clone()
            },
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        assert_eq!(work.accessibility, vec![node(1)]);
        assert!(work.style.is_empty());
        assert!(work.layout.is_empty());
    }

    #[test]
    fn accessibility_delta_removes_and_restores_hidden_subtrees_atomically() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element { tag: "div".into() },
        );
        queue.create(node(3), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        assert_eq!(world.project_accessibility(document(1)).len(), 3);

        let mut hide = MutationQueue::new();
        hide.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    hidden: true,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(hide).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let hidden = world.project_accessibility_delta(&work);
        assert_eq!(hidden.removed, vec![node(2), node(3)]);
        assert_eq!(
            hidden
                .updated
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![node(1)]
        );
        assert!(hidden.updated[0].children.is_empty());

        let mut show = MutationQueue::new();
        show.set_style(node(2), NodeStyle::default());
        world.commit(show).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let visible = world.project_accessibility_delta(&work);
        assert!(visible.removed.is_empty());
        assert_eq!(
            visible
                .updated
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![node(1), node(2), node(3)]
        );
        assert_eq!(visible.updated[0].children, vec![node(2)]);
        assert_eq!(visible.updated[1].children, vec![node(3)]);
    }

    #[test]
    fn pointer_capture_and_event_routes_share_runtime_hierarchy_and_lifetime() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        for id in 1..=3 {
            queue.create(
                node(id),
                document(1),
                NodeKind::Element { tag: "div".into() },
            );
        }
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        world.commit(queue).unwrap();

        assert_eq!(
            world.event_route(node(3)).unwrap(),
            EventRoute {
                capture: vec![node(1), node(2)],
                target: node(3),
                bubble: vec![node(2), node(1)],
            }
        );

        let mut queue = MutationQueue::new();
        queue.capture_pointer(7, node(2));
        queue.capture_pointer(7, node(3));
        world.commit(queue).unwrap();
        assert_eq!(world.pointer_capture(document(1), 7), Some(node(3)));
        assert_eq!(
            world.take_pointer_capture_changes(),
            vec![
                PointerCaptureChange {
                    pointer_id: 7,
                    target: node(2),
                    captured: true,
                },
                PointerCaptureChange {
                    pointer_id: 7,
                    target: node(2),
                    captured: false,
                },
                PointerCaptureChange {
                    pointer_id: 7,
                    target: node(3),
                    captured: true,
                },
            ]
        );

        let generation = world.generation();
        let mut invalid = MutationQueue::new();
        invalid.release_pointer(7, node(2));
        assert_eq!(
            world.commit(invalid),
            Err(UiWorldError::PointerCaptureMismatch {
                pointer_id: 7,
                target: node(2),
            })
        );
        assert_eq!(world.generation(), generation);
        assert_eq!(world.pointer_capture(document(1), 7), Some(node(3)));

        let mut remove = MutationQueue::new();
        remove.despawn_subtree(node(2));
        world.commit(remove).unwrap();
        assert_eq!(world.pointer_capture(document(1), 7), None);
        assert_eq!(
            world.take_pointer_capture_changes(),
            vec![PointerCaptureChange {
                pointer_id: 7,
                target: node(3),
                captured: false,
            }]
        );
        assert!(world.event_route(node(3)).is_none());
    }

    #[test]
    fn pointer_hover_and_press_are_runtime_owned_and_targeted() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(node(2), document(1), NodeKind::Element { tag: "a".into() });
        queue.create(node(3), document(1), NodeKind::Element { tag: "b".into() });
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        for id in [node(2), node(3)] {
            queue.set_style(
                id,
                NodeStyle {
                    interaction: crate::InteractionStyle {
                        hovered: crate::SemanticPaint {
                            background: Some(SemanticColorRole::Hover),
                            ..crate::SemanticPaint::default()
                        },
                        pressed: crate::SemanticPaint {
                            background: Some(SemanticColorRole::Active),
                            ..crate::SemanticPaint::default()
                        },
                        ..crate::InteractionStyle::default()
                    },
                    ..NodeStyle::default()
                },
            );
        }
        world.commit(queue).unwrap();
        world.take_system_work();

        assert_eq!(
            world.set_pointer_hover(document(1), 7, Some(node(2))),
            Ok(None)
        );
        assert_eq!(world.pointer_hover(document(1), 7), Some(node(2)));
        let work = world.take_system_work();
        assert_eq!(work.style, vec![node(2)]);
        world.resolve_styles(&work.style).unwrap();
        assert_eq!(
            world.extract_nodes(&[node(2)])[0].style.background,
            Some(nana_ui_core::SemanticPalette::dark().hover.as_rgba_array())
        );
        let generation = world.generation();
        assert_eq!(
            world.set_pointer_hover(document(1), 7, Some(node(2))),
            Ok(Some(node(2)))
        );
        assert_eq!(world.generation(), generation);
        assert!(world.take_system_work().is_empty());

        assert_eq!(world.press_pointer(document(1), 7, node(2)), Ok(None));
        assert_eq!(world.pointer_press(document(1), 7), Some(node(2)));
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        assert_eq!(
            world.extract_nodes(&[node(2)])[0].style.background,
            Some(nana_ui_core::SemanticPalette::dark().active.as_rgba_array())
        );
        assert_eq!(
            world.set_pointer_hover(document(1), 7, Some(node(3))),
            Ok(Some(node(2)))
        );
        let work = world.take_system_work();
        assert_eq!(work.style, vec![node(2), node(3)]);
        assert_eq!(world.release_pointer_press(document(1), 7), Some(node(2)));

        let mut disable = MutationQueue::new();
        disable.set_interaction(
            node(3),
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
        );
        world.commit(disable).unwrap();
        assert_eq!(world.pointer_hover(document(1), 7), None);
        assert_eq!(
            world.set_pointer_hover(document(1), 7, Some(node(3))),
            Err(UiWorldError::NotPointerInteractive(node(3)))
        );
    }

    #[test]
    fn scroll_offset_moves_descendant_hit_testing_without_rewriting_layout() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "scroll".into(),
            },
        );
        queue.create(
            node(3),
            document(1),
            NodeKind::Element { tag: "item".into() },
        );
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.write_layout(
            node(2),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 50.0,
            },
        );
        queue.write_layout(
            node(3),
            LayoutBox {
                x: 0.0,
                y: 80.0,
                width: 100.0,
                height: 20.0,
            },
        );
        queue.set_style(
            node(2),
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    overflow_y: OverflowSpec::Scroll,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        world.commit(queue).unwrap();
        world.take_system_work();

        let mut scroll = MutationQueue::new();
        scroll.set_scroll_offset(node(2), ScrollOffset { x: 0.0, y: 60.0 });
        world.commit(scroll).unwrap();
        assert_eq!(world.layout_box(node(3)).unwrap().y, 80.0);
        assert_eq!(world.scroll_offset(node(2)).unwrap().y, 60.0);
        let work = world.take_system_work();
        assert_eq!(work.input_hit_test, vec![node(2), node(3)]);
        assert_eq!(work.render_extraction, vec![node(2), node(3)]);
        assert!(work.layout.is_empty());
        world.rebuild_hit_test(document(1));
        assert_eq!(world.hit_test(document(1), 10.0, 25.0), Some(node(3)));
        assert_ne!(world.hit_test(document(1), 10.0, 85.0), Some(node(3)));

        let mut metrics = MutationQueue::new();
        metrics.set_scroll_metrics(
            node(2),
            Some(ScrollMetrics {
                viewport_width: 100.0,
                viewport_height: 50.0,
                content_width: 100.0,
                content_height: 100.0,
            }),
        );
        world.commit(metrics).unwrap();
        assert_eq!(world.scroll_offset(node(2)).unwrap().y, 50.0);
        let work = world.take_system_work();
        assert_eq!(work.input_hit_test, vec![node(2), node(3)]);
        assert!(work.layout.is_empty());

        let generation = world.generation();
        let mut invalid = MutationQueue::new();
        invalid.set_scroll_offset(node(2), ScrollOffset { x: 0.0, y: -1.0 });
        assert_eq!(
            world.commit(invalid),
            Err(UiWorldError::InvalidScrollOffset(node(2)))
        );
        assert_eq!(world.generation(), generation);

        let mut invalid_metrics = MutationQueue::new();
        invalid_metrics.set_scroll_metrics(
            node(2),
            Some(ScrollMetrics {
                viewport_width: f32::NAN,
                viewport_height: 50.0,
                content_width: 100.0,
                content_height: 100.0,
            }),
        );
        assert_eq!(
            world.commit(invalid_metrics),
            Err(UiWorldError::InvalidScrollMetrics(node(2)))
        );
        assert_eq!(world.generation(), generation);
    }
}
