//! Atomic mutation batch validation.

use super::super::*;

/// Validate-then-apply staging for one mutation batch.
///
/// Every field is an overlay over `source`, never a copy of it: validation cost
/// tracks the batch, not the retained world. `parked`, `pointer_captures`, and
/// `animations` fall back to `source` on a miss instead of being pre-filled, and
/// overlay-host walks use `UiWorld::overlay_host_ids`.
pub(in crate::world) struct ValidationPlan<'a> {
    pub(in crate::world) source: &'a UiWorld,
    pub(in crate::world) nodes: NodeMap<PlannedNode>,
    pub(in crate::world) removed: NodeSet,
    pub(in crate::world) newly_retired: NodeSet,
    /// Mount overrides staged by this batch. Absent means "ask `source`".
    pub(in crate::world) parked: NodeMap<bool>,
    detached: NodeMap<bool>,
    /// Children detached from each parent whose staged list still holds them.
    /// The list drops them in one pass when next read, so a batch removing
    /// many children of one parent filters its list once, not once per child
    /// (#389).
    leaving: NodeMap<NodeSet>,
    pub(in crate::world) interactions: NodeMap<InteractionState>,
    pub(in crate::world) styles: NodeMap<NodeStyle>,
    pub(in crate::world) focus: HashMap<DocumentId, Option<StableNodeId>>,
    pub(in crate::world) scope_focus: NodeMap<StableNodeId>,
    /// Cloned from `source` on first pointer-capture mutation. Batches that do
    /// not touch capture never pay for it.
    pub(in crate::world) pointer_captures: Option<HashMap<(DocumentId, u64), StableNodeId>>,
    /// Cloned from `source` on first animation mutation, same rationale.
    pub(in crate::world) animations: Option<HashMap<AnimationId, AnimationSpec>>,
    pub(in crate::world) text_inputs: NodeMap<Option<StagedText>>,
    pub(in crate::world) surface_open: NodeMap<bool>,
    pub(in crate::world) overlay_hosts: NodeMap<OverlayHostState>,
    pub(in crate::world) accessibility: NodeMap<AccessibilityState>,
    overlay_dependents: NodeMap<NodeSet>,
    affected_overlay_hosts: NodeSet,
    /// Nodes visited by whole-set walks during this validation. Reported through
    /// `UiWorld::validation_nodes_scanned` so a reintroduced world scan fails a
    /// test instead of silently costing a frame.
    pub(in crate::world) scanned: usize,
    /// Child-list entries `leaving` filtered, for cost tests.
    #[cfg(test)]
    pub(in crate::world) child_list_steps: u64,
    /// Whether this batch has staged a change to any node's parent or
    /// children (through [`Self::node_mut`]) or removed one. Until it has,
    /// every depth is the one `source` keeps.
    restructured: bool,
    /// Depths this batch has settled, as it leaves the tree. Placing a node
    /// with no children moves only it, so its entry is rewritten; moving one
    /// with children moves a whole subtree, so the memo is cleared.
    depths: NodeMap<usize>,
}

/// An editor's text and selections as a batch leaves them, for validating
/// what follows in the same batch. The text is a shared copy: staging a caret
/// move copies no text.
#[derive(Debug, Clone)]
pub(in crate::world) struct StagedText {
    pub value: crate::TextValue,
    pub selection: crate::TextSelection,
    pub additional: Vec<crate::TextSelection>,
}

impl<'a> ValidationPlan<'a> {
    pub(in crate::world) fn new(source: &'a UiWorld) -> Self {
        Self {
            source,
            nodes: Default::default(),
            removed: Default::default(),
            newly_retired: Default::default(),
            parked: Default::default(),
            detached: Default::default(),
            leaving: Default::default(),
            interactions: Default::default(),
            styles: Default::default(),
            focus: HashMap::new(),
            scope_focus: Default::default(),
            pointer_captures: None,
            animations: None,
            text_inputs: Default::default(),
            surface_open: Default::default(),
            overlay_hosts: Default::default(),
            accessibility: Default::default(),
            overlay_dependents: Default::default(),
            affected_overlay_hosts: Default::default(),
            scanned: 0,
            #[cfg(test)]
            child_list_steps: 0,
            restructured: false,
            depths: NodeMap::default(),
        }
    }

    /// Staged mount state. A node this batch created has no `MountState` in
    /// `source`, and `mount_state` returning `None` correctly reads as unparked.
    pub(in crate::world) fn is_parked(&self, id: StableNodeId) -> bool {
        if let Some(&parked) = self.parked.get(&id) {
            return parked;
        }
        self.source.mount_state(id) == Some(MountState::Parked)
    }

    fn is_detached(&self, id: StableNodeId) -> bool {
        self.detached
            .get(&id)
            .copied()
            .unwrap_or_else(|| self.source.tree.detached.contains(&id))
    }

    pub(in crate::world) fn set_parked(&mut self, id: StableNodeId, parked: bool) {
        self.parked.insert(id, parked);
    }

    pub(in crate::world) fn pointer_captures_mut(
        &mut self,
    ) -> &mut HashMap<(DocumentId, u64), StableNodeId> {
        if self.pointer_captures.is_none() {
            self.pointer_captures = Some(self.source.input.pointer_captures.clone());
        }
        self.pointer_captures
            .as_mut()
            .expect("initialized directly above")
    }

    pub(in crate::world) fn pointer_capture(
        &self,
        document: DocumentId,
        pointer_id: u64,
    ) -> Option<StableNodeId> {
        match &self.pointer_captures {
            Some(captures) => captures.get(&(document, pointer_id)).copied(),
            None => self.source.pointer_capture(document, pointer_id),
        }
    }

    pub(in crate::world) fn animations_mut(&mut self) -> &mut HashMap<AnimationId, AnimationSpec> {
        if self.animations.is_none() {
            self.animations = Some(
                self.source
                    .motion
                    .animations
                    .iter()
                    .map(|(&id, animation)| (id, animation.spec.clone()))
                    .collect(),
            );
        }
        self.animations
            .as_mut()
            .expect("initialized directly above")
    }

    /// Overlay hosts staged by this batch plus those already in `source`.
    pub(in crate::world) fn overlay_host_candidates(
        &mut self,
        document: DocumentId,
    ) -> Vec<StableNodeId> {
        let mut hosts = self
            .overlay_hosts
            .keys()
            .copied()
            .filter(|id| {
                self.planned_identity(*id)
                    .is_ok_and(|(doc, _)| doc == document)
            })
            .collect::<HashSet<_>>();
        hosts.extend(self.source.overlay_host_ids(document));
        self.scanned = self.scanned.saturating_add(hosts.len());
        hosts.into_iter().collect()
    }

    fn planned_identity(
        &self,
        id: StableNodeId,
    ) -> Result<(DocumentId, Option<StableNodeId>), UiWorldError> {
        self.require_exists(id)?;
        if let Some(node) = self.nodes.get(&id) {
            Ok((node.document, node.parent))
        } else {
            self.source.identity_and_parent(id)
        }
    }

    fn overlay_referencing(&mut self, target: StableNodeId) -> Vec<StableNodeId> {
        // Almost no node is an overlay's focus target: answer without
        // building the set.
        if !self
            .source
            .overlays_a11y
            .overlay_dependents
            .contains_key(&target)
            && !self.overlay_dependents.contains_key(&target)
        {
            return Vec::new();
        }
        let mut hosts = self
            .source
            .overlays_a11y
            .overlay_dependents
            .get(&target)
            .into_iter()
            .flatten()
            .copied()
            .collect::<NodeSet>();
        hosts.extend(
            self.overlay_dependents
                .get(&target)
                .into_iter()
                .flatten()
                .copied(),
        );
        self.scanned += hosts.len();
        hosts
            .into_iter()
            .filter(|host| {
                self.exists(*host)
                    && self
                        .overlay_hosts
                        .get(host)
                        .copied()
                        .or_else(|| self.source.overlay_host(*host))
                        .is_some_and(|state| {
                            state.active == Some(target) || state.restore_focus == Some(target)
                        })
            })
            .collect()
    }

    fn stage_overlay_host(&mut self, host: StableNodeId, state: OverlayHostState) {
        if let Some(previous) = self.overlay_hosts.insert(host, state) {
            for target in [previous.active, previous.restore_focus]
                .into_iter()
                .flatten()
            {
                if let Some(hosts) = self.overlay_dependents.get_mut(&target) {
                    hosts.remove(&host);
                    if hosts.is_empty() {
                        self.overlay_dependents.remove(&target);
                    }
                }
            }
        }
        for target in [state.active, state.restore_focus].into_iter().flatten() {
            self.overlay_dependents
                .entry(target)
                .or_default()
                .insert(host);
        }
        self.affected_overlay_hosts.insert(host);
    }

    #[cfg(test)]
    pub(in crate::world) fn validate(
        &mut self,
        mutations: &[UiMutation],
    ) -> Result<(), UiWorldError> {
        let caps = mutations.iter().fold(
            crate::mutation::BatchCapabilities::EMPTY,
            |acc, mutation| acc.union(crate::mutation::BatchCapabilities::for_mutation(mutation)),
        );
        self.validate_with_caps(mutations, caps)
    }

    pub(in crate::world) fn validate_with_caps(
        &mut self,
        mutations: &[UiMutation],
        caps: crate::mutation::BatchCapabilities,
    ) -> Result<(), UiWorldError> {
        // Staged accessibility is read only to check overlay hosts; with none
        // in the world or in this batch, skip copying it.
        let overlays = !self.source.overlays_a11y.overlay_host_nodes.is_empty()
            || (caps.intersects(crate::mutation::BatchCapabilities::VISUAL_OR_INTERACTION)
                && mutations
                    .iter()
                    .any(|mutation| matches!(mutation, UiMutation::SetOverlayHost { .. })));
        for mutation in mutations {
            match mutation {
                UiMutation::Insert { child: id, .. }
                | UiMutation::Detach { id }
                | UiMutation::SetAccessibility { id, .. } => {
                    let hosts = self.overlay_referencing(*id);
                    self.affected_overlay_hosts.extend(hosts);
                }
                _ => {}
            }
            match mutation {
                UiMutation::Create { id, document, .. } => self.create(*id, *document)?,
                UiMutation::Insert {
                    parent,
                    child,
                    before,
                } => self.insert(*parent, *child, *before)?,
                UiMutation::ReorderChildren { parent, order } => self.reorder(*parent, order)?,
                UiMutation::Detach { id } => {
                    self.detach(*id)?;
                }
                UiMutation::ParkSubtree { root } => self.park(*root)?,
                UiMutation::DespawnSubtree { root } => self.despawn_subtree(*root)?,
                UiMutation::SetStyle { id, style } => {
                    self.require_exists(*id)?;
                    if !style_is_valid(style.layout.as_ref()) {
                        return Err(UiWorldError::InvalidStyle(*id));
                    }
                    self.styles.insert(*id, (**style).clone());
                }
                UiMutation::SetPresetTheme { .. } | UiMutation::SetThemeTokens { .. } => {}
                UiMutation::SetText { id, .. }
                | UiMutation::SetInlineContent { id, .. }
                | UiMutation::SetRichText { id, .. }
                | UiMutation::SetRichEditorMarks { id, .. }
                | UiMutation::SetLanguage { id, .. }
                | UiMutation::SetLocale { id, .. }
                | UiMutation::SetLocalizedText { id, .. }
                | UiMutation::SetGlyphPresentation { id, .. } => {
                    self.require_exists(*id)?;
                }
                UiMutation::SetTextScale { id, scale } => {
                    self.require_exists(*id)?;
                    if scale.is_some_and(|scale| !valid_text_scale(scale)) {
                        return Err(UiWorldError::InvalidStyle(*id));
                    }
                }
                UiMutation::SetExplicitRect { id, rect } => {
                    self.require_exists(*id)?;
                    if rect.as_deref().is_some_and(|rect| !rect.is_valid()) {
                        return Err(UiWorldError::InvalidStyle(*id));
                    }
                }
                UiMutation::SetResponsive { id, rule } => {
                    self.require_exists(*id)?;
                    if let Some(rule) = rule {
                        if !rule.is_valid() {
                            return Err(UiWorldError::InvalidResponsiveRule(*id));
                        }
                        if let crate::ResponsiveContainer::Node(container) = rule.container() {
                            self.require_exists(*container)?;
                        }
                    }
                }
                UiMutation::SetReplacedMetadata { metadata, .. } => {
                    if metadata
                        .as_deref()
                        .is_some_and(|metadata| !metadata.is_valid())
                    {
                        return Err(UiWorldError::InvalidReplacedMetadata);
                    }
                }
                UiMutation::WriteLayout { id, layout }
                | UiMutation::PlaceLayout { id, layout, .. } => {
                    self.require_exists(*id)?;
                    if !layout_is_valid(layout) {
                        return Err(UiWorldError::InvalidLayout(*id));
                    }
                }
                UiMutation::PatchPlacement { id, top, height } => {
                    self.require_exists(*id)?;
                    if !top.is_finite()
                        || height.is_some_and(|height| !height.is_finite() || height < 0.0)
                    {
                        return Err(UiWorldError::InvalidStyle(*id));
                    }
                }
                UiMutation::SetScrollOffset { id, offset }
                | UiMutation::SetScrollOverscroll { id, offset } => {
                    self.require_exists(*id)?;
                    // Negative on an axis whose scroll origin is the right /
                    // bottom edge; the metrics clamp decides the range.
                    if !offset.x.is_finite() || !offset.y.is_finite() {
                        return Err(UiWorldError::InvalidScrollOffset(*id));
                    }
                }
                UiMutation::SetScrollMetrics { id, metrics } => {
                    self.require_exists(*id)?;
                    if metrics.is_some_and(|metrics| {
                        [
                            metrics.viewport_width,
                            metrics.viewport_height,
                            metrics.content_width,
                            metrics.content_height,
                        ]
                        .into_iter()
                        .any(|extent| !extent.is_finite() || extent < 0.0)
                            || [metrics.origin_x, metrics.origin_y]
                                .into_iter()
                                .any(|origin| !origin.is_finite() || origin > 0.0)
                    }) {
                        return Err(UiWorldError::InvalidScrollMetrics(*id));
                    }
                }
                UiMutation::SetInteraction { id, interaction } => {
                    self.require_exists(*id)?;
                    self.interactions.insert(*id, *interaction);
                }
                UiMutation::SetPainter { id, .. } | UiMutation::SetMediaTime { id, .. } => {
                    self.require_exists(*id)?;
                }
                UiMutation::SetCustomRender { id, content } => {
                    self.require_exists(*id)?;
                    if content.as_ref().is_some_and(|content| {
                        content.renderer.trim().is_empty() || content.resource.trim().is_empty()
                    }) {
                        return Err(UiWorldError::InvalidCustomRender(*id));
                    }
                }
                UiMutation::SetEventListener { id, event, .. } => {
                    self.require_exists(*id)?;
                    if event.trim().is_empty() {
                        return Err(UiWorldError::InvalidEventListener(*id));
                    }
                }
                UiMutation::SetComponentType { id, .. } => {
                    self.require_exists(*id)?;
                }
                UiMutation::SetAccessibility { id, accessibility } => {
                    self.require_exists(*id)?;
                    if overlays {
                        self.accessibility.insert(*id, (**accessibility).clone());
                    }
                }
                UiMutation::SetLabelledBy { id, label } => {
                    self.require_exists(*id)?;
                    if let Some(label) = label {
                        self.require_exists(*label)?;
                    }
                }
                UiMutation::SetLive { id, .. } | UiMutation::SetOverlayMount { id, .. } => {
                    self.require_exists(*id)?;
                }
                UiMutation::SetWidgetVisual { id, visual } => {
                    self.require_exists(*id)?;
                    if visual.as_ref().is_some_and(|visual| !visual.is_valid()) {
                        return Err(UiWorldError::InvalidWidgetVisual(*id));
                    }
                }
                UiMutation::SetSurfaceOpen { id, open, .. } => {
                    self.require_exists(*id)?;
                    self.surface_open.insert(*id, *open);
                }
                UiMutation::SetOverlayHost { host, state } => {
                    let host_document = self.planned_identity(*host)?.0;
                    if let Some(active) = state.active
                        && self.planned_identity(active)?.1 != Some(*host)
                    {
                        return Err(UiWorldError::InvalidOverlayHost(*host));
                    }
                    if let Some(restore_focus) = state.restore_focus
                        && self.planned_identity(restore_focus)?.0 != host_document
                    {
                        return Err(UiWorldError::FocusDocument {
                            document: host_document,
                            target: restore_focus,
                        });
                    }
                    self.stage_overlay_host(*host, **state);
                }
                UiMutation::CapturePointer { pointer_id, target } => {
                    let document = self.node(*target)?.document;
                    if self.is_parked(*target) {
                        return Err(UiWorldError::NotPointerInteractive(*target));
                    }
                    self.pointer_captures_mut()
                        .insert((document, *pointer_id), *target);
                }
                UiMutation::ReleasePointer { pointer_id, target } => {
                    let document = self.node(*target)?.document;
                    if self.pointer_capture(document, *pointer_id) != Some(*target) {
                        return Err(UiWorldError::PointerCaptureMismatch {
                            pointer_id: *pointer_id,
                            target: *target,
                        });
                    }
                    self.pointer_captures_mut().remove(&(document, *pointer_id));
                }
                UiMutation::StartAnimation { animation } => {
                    self.node(animation.target)?;
                    if !animation.is_valid() {
                        return Err(UiWorldError::InvalidAnimation(animation.id));
                    }
                    if self.is_parked(animation.target) && !animation.uses_presentation_overlay() {
                        return Err(UiWorldError::InvalidAnimation(animation.id));
                    }
                    self.animations_mut()
                        .insert(animation.id, (**animation).clone());
                }
                UiMutation::ReplaceAnimation { animation } => {
                    self.node(animation.target)?;
                    if !animation.is_valid() {
                        return Err(UiWorldError::InvalidAnimation(animation.id));
                    }
                    if self.is_parked(animation.target) && !animation.uses_presentation_overlay() {
                        return Err(UiWorldError::InvalidAnimation(animation.id));
                    }
                    if !self.animations_mut().contains_key(&animation.id) {
                        return Err(UiWorldError::MissingAnimation(animation.id));
                    }
                    self.animations_mut()
                        .insert(animation.id, (**animation).clone());
                }
                UiMutation::StopAnimation { id } => {
                    // A finished run that fills forwards still holds its
                    // property; stopping it is how its owner gives it back.
                    if self.animations_mut().remove(id).is_none()
                        && self.source.motion.presentation.get(id.track_id()).is_none()
                    {
                        return Err(UiWorldError::MissingAnimation(*id));
                    }
                }
                UiMutation::FinishAnimation { id } => {
                    if self.animations_mut().remove(id).is_none() {
                        return Err(UiWorldError::MissingAnimation(*id));
                    }
                }
                UiMutation::ReverseAnimation { id }
                | UiMutation::PauseAnimation { id }
                | UiMutation::ResumeAnimation { id } => {
                    if !self.animations_mut().contains_key(id) {
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
                                    .nodes
                                    .get(*target)
                                    .map(|node| node.interaction)
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
                    if let Some(target) = target {
                        self.remember_scope_focus(*target)?;
                    }
                }
                UiMutation::RestoreFocusWithin { root } => {
                    if let Some((document, target)) = self.restorable_scope_focus(*root)? {
                        self.focus.insert(document, Some(target));
                        self.remember_scope_focus(target)?;
                    }
                }
                UiMutation::SetIme { id, composition } => {
                    let document = self.node(*id)?.document;
                    if composition.is_some() && self.is_parked(*id) {
                        return Err(UiWorldError::NotFocused(*id));
                    }
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
                    if let Some(composition) = composition.as_deref()
                        && let ImeComposition {
                            text,
                            selection: Some((start, end)),
                        } = composition
                        && (start > end
                            || *end > text.len()
                            || !text.is_char_boundary(*start)
                            || !text.is_char_boundary(*end)
                            || !crate::TextSelection::new(*start, *end).is_valid_for(text))
                    {
                        return Err(UiWorldError::InvalidIme(*id));
                    }
                }
                UiMutation::SetTextInput { id, state } => {
                    self.require_exists(*id)?;
                    if state
                        .as_ref()
                        .is_some_and(|state| !state.selection.is_valid_for(&state.value))
                    {
                        return Err(UiWorldError::InvalidTextInput(*id));
                    }
                    // Staged as the session will hold it: further cursors
                    // snapped and fused, so what validates after this in the
                    // batch edits the text the apply will edit.
                    let staged = state.as_ref().map(|state| {
                        let mut state = state.clone();
                        state.normalize_selections();
                        StagedText {
                            value: state.value,
                            selection: state.selection,
                            additional: state.additional_selections,
                        }
                    });
                    self.text_inputs.insert(*id, staged);
                }
                UiMutation::SetTextSelection { id, selection } => {
                    let mut staged = self.text_input(*id)?;
                    if !selection.is_valid_for(&staged.value) {
                        return Err(UiWorldError::InvalidTextInput(*id));
                    }
                    staged.selection = *selection;
                    self.text_inputs.insert(*id, Some(staged));
                }
                UiMutation::ReplaceTextSelection { id, text } => {
                    let staged = self.text_input(*id)?;
                    let mut state = crate::TextInputState {
                        value: staged.value,
                        selection: staged.selection,
                        additional_selections: staged.additional,
                    };
                    if !state.replace_selection(text.as_ref()) {
                        return Err(UiWorldError::InvalidTextInput(*id));
                    }
                    self.text_inputs.insert(
                        *id,
                        Some(StagedText {
                            value: state.value,
                            selection: state.selection,
                            additional: state.additional_selections,
                        }),
                    );
                }
                UiMutation::SetHighlightRequest { id, request } => {
                    self.require_exists(*id)?;
                    if request
                        .as_ref()
                        .is_some_and(|request| request.presenter.trim().is_empty())
                    {
                        return Err(UiWorldError::InvalidHighlightRequest(*id));
                    }
                }
                UiMutation::SetTextInputFoldCollapsed { id, .. }
                | UiMutation::SetTextInputInlays { id, .. }
                | UiMutation::SetTextInputSnippet { id, .. }
                | UiMutation::SetTextInputCompletions { id, .. }
                | UiMutation::SetTextInputCompletionView { id, .. }
                | UiMutation::SetTextInputCompletionDismissed { id }
                | UiMutation::SetTextInputCompletionReopened { id }
                | UiMutation::SetTextInputHover { id, .. }
                | UiMutation::SetTextInputDiagnosticHover { id, .. }
                | UiMutation::SetTextInputHoverScroll { id, .. }
                | UiMutation::SetTextInputSignature { id, .. } => {
                    self.require_exists(*id)?;
                }
            }
        }
        if !self.affected_overlay_hosts.is_empty() {
            self.validate_overlay_hosts()?;
        }
        if caps.intersects(
            crate::mutation::BatchCapabilities::TOPOLOGY
                .union(crate::mutation::BatchCapabilities::STYLE)
                .union(crate::mutation::BatchCapabilities::TEXT)
                .union(crate::mutation::BatchCapabilities::VISUAL_OR_INTERACTION),
        ) {
            self.validate_inline_content(mutations)?;
        }
        Ok(())
    }
    pub(in crate::world) fn create(
        &mut self,
        id: StableNodeId,
        document: DocumentId,
    ) -> Result<(), UiWorldError> {
        if self.exists(id) {
            return Err(UiWorldError::DuplicateNode(id));
        }
        if self.source.is_retired(id) || self.newly_retired.contains(&id) {
            return Err(UiWorldError::RetiredNode(id));
        }
        self.removed.remove(&id);
        self.set_parked(id, false);
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

    pub(in crate::world) fn insert(
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
        // A child with no children cannot hold `parent`: only `parent == child`
        // makes a cycle then.
        let childless = child_node.children.is_empty();
        if parent == child || (!childless && self.has_ancestor(parent, child)?) {
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
        let parent_depth = self.ancestor_depth(parent)?;
        let height = if childless {
            1
        } else {
            self.subtree_height(child)?
        };
        let depth = parent_depth.saturating_add(height);
        if depth > MAX_TREE_DEPTH {
            return Err(UiWorldError::TreeTooDeep {
                parent,
                child,
                depth,
            });
        }
        self.detach(child)?;
        let siblings = &mut self.node_mut(parent)?.children;
        let index = before
            .and_then(|before| siblings.iter().position(|id| *id == before))
            .unwrap_or(siblings.len());
        siblings.insert(index, child);
        self.node_mut(child)?.parent = Some(parent);
        if childless {
            self.depths.insert(child, parent_depth + 1);
        } else {
            self.depths.clear();
        }
        self.detached.insert(child, false);
        let parked = self.is_parked(parent);
        self.set_parked_subtree(child, parked)?;
        Ok(())
    }

    /// `id`'s parent as this batch leaves it, read without staging `id`: the
    /// walks up and down that validate an insert only read, and staging
    /// clones a node's snapshot out of the world for every node they pass.
    fn parent_of(&self, id: StableNodeId) -> Result<Option<StableNodeId>, UiWorldError> {
        if self.removed.contains(&id) {
            return Err(UiWorldError::MissingNode(id));
        }
        match self.nodes.get(&id) {
            Some(node) => Ok(node.parent),
            None => self
                .source
                .nodes
                .get(id)
                .map(|node| node.hierarchy.parent)
                .ok_or(UiWorldError::MissingNode(id)),
        }
    }

    /// Levels from `id` up to its root, counting `id` itself.
    pub(in crate::world) fn ancestor_depth(
        &mut self,
        id: StableNodeId,
    ) -> Result<usize, UiWorldError> {
        if self.removed.contains(&id) {
            return Err(UiWorldError::MissingNode(id));
        }
        if let Some(&depth) = self.depths.get(&id) {
            return Ok(depth);
        }
        if !self.restructured
            && self.removed.is_empty()
            && !self.nodes.contains_key(&id)
            && let Some(depth) = self.source.node_depth(id)
        {
            return Ok(depth);
        }
        // Up to the first ancestor whose depth is settled, or the root.
        let mut steps = 0;
        let mut cursor = Some(id);
        let mut above = 0;
        while let Some(node) = cursor {
            if let Some(&depth) = self.depths.get(&node) {
                above = depth;
                break;
            }
            steps += 1;
            if steps > MAX_TREE_DEPTH {
                // `has_ancestor` already rejected cycles, so this is genuine
                // depth rather than a loop.
                return Ok(steps);
            }
            cursor = self.parent_of(node)?;
        }
        let depth = above + steps;
        self.depths.insert(id, depth);
        #[cfg(test)]
        {
            // The memo stands in for this walk; every test checks it.
            let mut walked = 1;
            let mut cursor = self.parent_of(id)?;
            while let Some(ancestor) = cursor {
                walked += 1;
                cursor = self.parent_of(ancestor)?;
            }
            assert_eq!(depth, walked, "settled depth of {id:?} is stale");
        }
        Ok(depth)
    }

    /// Levels from `root` down to its deepest descendant, counting `root`.
    /// Stops climbing once past the limit; the caller only needs to know that.
    pub(in crate::world) fn subtree_height(
        &mut self,
        root: StableNodeId,
    ) -> Result<usize, UiWorldError> {
        let mut frontier = vec![root];
        let mut next = Vec::new();
        let mut height = 0;
        while !frontier.is_empty() {
            height += 1;
            if height > MAX_TREE_DEPTH {
                return Ok(height);
            }
            for &id in &frontier {
                if self.leaving.contains_key(&id) {
                    // Its staged list still holds children leaving it.
                    next.extend(self.node(id)?.children.iter().copied());
                    continue;
                }
                if self.removed.contains(&id) {
                    return Err(UiWorldError::MissingNode(id));
                }
                match self.nodes.get(&id) {
                    Some(node) => next.extend(node.children.iter().copied()),
                    None => next.extend(
                        self.source
                            .nodes
                            .get(id)
                            .ok_or(UiWorldError::MissingNode(id))?
                            .hierarchy
                            .children
                            .iter()
                            .copied(),
                    ),
                }
            }
            std::mem::swap(&mut frontier, &mut next);
            next.clear();
        }
        Ok(height)
    }

    pub(in crate::world) fn park(&mut self, root: StableNodeId) -> Result<(), UiWorldError> {
        self.detach(root)?;
        let subtree = self.subtree(root)?;
        for &id in &subtree {
            self.set_parked(id, true);
        }
        let parked = subtree.iter().copied().collect::<NodeSet>();
        // A subtree almost always lies in one document.
        let mut documents: Vec<DocumentId> = Vec::new();
        for &id in &subtree {
            let document = self.node(id)?.document;
            if !documents.contains(&document) {
                documents.push(document);
            }
        }
        for document in documents {
            let focused = self
                .focus
                .get(&document)
                .copied()
                .unwrap_or_else(|| self.source.focused(document));
            if focused.is_some_and(|target| parked.contains(&target)) {
                self.focus.insert(document, None);
            }
        }
        if self.pointer_captures.is_some() || !self.source.input.pointer_captures.is_empty() {
            self.pointer_captures_mut()
                .retain(|_, target| !parked.contains(target));
        }
        if self.animations.is_some() || !self.source.motion.animations.is_empty() {
            self.animations_mut().retain(|_, animation| {
                !parked.contains(&animation.target) || animation.uses_presentation_overlay()
            });
        }
        for id in subtree {
            self.clear_overlay_references(id);
        }
        Ok(())
    }

    pub(in crate::world) fn subtree(
        &mut self,
        root: StableNodeId,
    ) -> Result<Vec<StableNodeId>, UiWorldError> {
        let mut subtree = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let children = self.node(id)?.children.clone();
            stack.extend(children);
            subtree.push(id);
        }
        Ok(subtree)
    }

    pub(in crate::world) fn set_parked_subtree(
        &mut self,
        root: StableNodeId,
        parked: bool,
    ) -> Result<(), UiWorldError> {
        for id in self.subtree(root)? {
            self.set_parked(id, parked);
        }
        Ok(())
    }

    pub(in crate::world) fn detach(&mut self, id: StableNodeId) -> Result<(), UiWorldError> {
        let node = self.node(id)?;
        let (parent, childless) = (node.parent, node.children.is_empty());
        if childless {
            self.depths.remove(&id);
        } else {
            self.depths.clear();
        }
        self.detached.insert(id, true);
        if let Some(parent) = parent {
            self.ensure(parent)?;
            self.leaving.entry(parent).or_default().insert(id);
            self.node_mut(id)?.parent = None;
        }
        Ok(())
    }

    pub(in crate::world) fn despawn_subtree(
        &mut self,
        root: StableNodeId,
    ) -> Result<(), UiWorldError> {
        let subtree = self.subtree(root)?;
        let removed = subtree.iter().copied().collect::<HashSet<_>>();
        let documents = subtree
            .iter()
            .map(|id| self.node(*id).map(|node| node.document))
            .collect::<Result<HashSet<_>, _>>()?;
        for document in documents {
            let focused = self
                .focus
                .get(&document)
                .copied()
                .unwrap_or_else(|| self.source.focused(document));
            if focused.is_some_and(|target| removed.contains(&target)) {
                self.focus.insert(document, None);
            }
        }
        self.detach(root)?;
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let children = self.node(id)?.children.clone();
            stack.extend(children);
            self.nodes.remove(&id);
            self.removed.insert(id);
            self.newly_retired.insert(id);
            self.parked.remove(&id);
            if self.pointer_captures.is_some() || !self.source.input.pointer_captures.is_empty() {
                self.pointer_captures_mut()
                    .retain(|_, target| *target != id);
            }
            if self.animations.is_some() || !self.source.motion.animations.is_empty() {
                self.animations_mut()
                    .retain(|_, animation| animation.target != id);
            }
            self.text_inputs.remove(&id);
            self.clear_overlay_references(id);
        }
        Ok(())
    }

    pub(in crate::world) fn clear_overlay_references(&mut self, removed: StableNodeId) {
        for host in self.overlay_referencing(removed) {
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
            self.stage_overlay_host(host, state);
        }
    }

    pub(in crate::world) fn validate_overlay_hosts(&mut self) -> Result<(), UiWorldError> {
        let hosts = self
            .affected_overlay_hosts
            .iter()
            .copied()
            .collect::<Vec<_>>();
        self.scanned += hosts.len();
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
            let host_document = self.planned_identity(host)?.0;
            if let Some(active) = state.active
                && (!self.exists(active) || self.planned_identity(active)?.1 != Some(host))
            {
                return Err(UiWorldError::InvalidOverlayHost(host));
            }
            if let Some(active) = state.active {
                let accessibility = self
                    .accessibility
                    .get(&active)
                    .map(std::borrow::Cow::Borrowed)
                    .or_else(|| self.source.accessibility(active));
                if !accessibility.is_some_and(|accessibility| match accessibility.role {
                    AccessibilityRole::Dialog | AccessibilityRole::AlertDialog => {
                        accessibility.modal
                    }
                    AccessibilityRole::Region => {
                        !accessibility.modal
                            && accessibility
                                .label
                                .as_ref()
                                .is_some_and(|label| !label.is_empty())
                    }
                    AccessibilityRole::Menu
                    | AccessibilityRole::Tooltip
                    | AccessibilityRole::Status => true,
                    _ => false,
                }) {
                    return Err(UiWorldError::InvalidOverlayHost(host));
                }
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

    pub(in crate::world) fn has_ancestor(
        &mut self,
        mut id: StableNodeId,
        candidate: StableNodeId,
    ) -> Result<bool, UiWorldError> {
        // No chain is longer than the tree is deep; a walk past that is going
        // round a loop that does not pass `candidate`.
        for _ in 0..=MAX_TREE_DEPTH + 1 {
            if id == candidate {
                return Ok(true);
            }
            let Some(parent) = self.parent_of(id)? else {
                return Ok(false);
            };
            id = parent;
        }
        Ok(false)
    }

    fn require_exists(&self, id: StableNodeId) -> Result<(), UiWorldError> {
        // Scalar writes need staged identity, not a cloned child list.
        self.exists(id)
            .then_some(())
            .ok_or(UiWorldError::MissingNode(id))
    }

    pub(in crate::world) fn exists(&self, id: StableNodeId) -> bool {
        !self.removed.contains(&id) && (self.nodes.contains_key(&id) || self.source.contains(id))
    }

    pub(in crate::world) fn text_input(
        &mut self,
        id: StableNodeId,
    ) -> Result<StagedText, UiWorldError> {
        self.node(id)?;
        if let Some(state) = self.text_inputs.get(&id) {
            return state.clone().ok_or(UiWorldError::MissingTextInput(id));
        }
        self.source
            .text_input(id)
            .map(|view| StagedText {
                value: view.value_shared(),
                selection: view.selection,
                additional: view.additional_selections.to_vec(),
            })
            .ok_or(UiWorldError::MissingTextInput(id))
    }

    pub(in crate::world) fn overlay_branch_active(
        &mut self,
        id: StableNodeId,
    ) -> Result<bool, UiWorldError> {
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

    pub(in crate::world) fn focus_target_visible(
        &mut self,
        mut id: StableNodeId,
    ) -> Result<bool, UiWorldError> {
        let mut visibility = None;
        loop {
            if self.is_parked(id) || self.is_detached(id) {
                return Ok(false);
            }
            let layout = self
                .styles
                .get(&id)
                .map(|style| style.layout.as_ref())
                .or_else(|| {
                    self.source
                        .node_style(id)
                        .map(|style| style.layout.as_ref())
                });
            visibility = visibility.or_else(|| layout.and_then(|layout| layout.paint.visibility));
            if layout.is_some_and(|layout| layout.omits_box() || layout.inert)
                || !self.overlay_branch_active(id)?
                || self.node(id)?.parent.is_some_and(|parent| {
                    self.source
                        .widget_visual_ref(parent)
                        .is_some_and(|w| w.closed_menu_hides(id))
                })
            {
                return Ok(false);
            }
            let Some(parent) = self.node(id)?.parent else {
                return Ok(visibility != Some(nana_ui_core::VisibilitySpec::Hidden));
            };
            id = parent;
        }
    }

    pub(in crate::world) fn active_modal_allows_focus(
        &mut self,
        document: DocumentId,
        target: StableNodeId,
    ) -> Result<bool, UiWorldError> {
        let hosts = self.overlay_host_candidates(document);
        if hosts.is_empty() {
            return Ok(true);
        }
        // Document order only breaks ties between competing modals, so it stays
        // unevaluated until a first modal is found. Hosts that exist with no
        // active surface are the common case and must not pay for the walk.
        let mut order: Option<Vec<StableNodeId>> = None;
        let mut top = None;
        for host in hosts {
            if !self.exists(host) || self.is_parked(host) {
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
            let Some(active) = state.active else {
                continue;
            };
            if !self
                .surface_open
                .get(&active)
                .copied()
                .unwrap_or(!self.source.surface_closed(active))
            {
                continue;
            }
            if !self.exists(active)
                || self.is_parked(active)
                || self.node(host)?.document != document
                || self.node(active)?.parent != Some(host)
                || !self.focus_target_visible(active)?
            {
                continue;
            }
            let modal = self
                .accessibility
                .get(&active)
                .map(std::borrow::Cow::Borrowed)
                .or_else(|| self.source.accessibility(active))
                .is_some_and(|state| state.modal);
            if modal {
                let z = self
                    .styles
                    .get(&active)
                    .or_else(|| self.source.node_style(active))
                    .and_then(|style| style.layout.z_index)
                    .unwrap_or_default();
                if order.is_none() {
                    order = Some(self.planned_document_order(document)?);
                }
                let document_order = order
                    .as_ref()
                    .expect("document order resolved directly above")
                    .iter()
                    .position(|candidate| *candidate == active)
                    .unwrap_or_default();
                if top.is_none_or(|(top_z, top_order, _)| (z, document_order) > (top_z, top_order))
                {
                    top = Some((z, document_order, active));
                }
            }
        }
        top.map(|(_, _, active)| self.has_ancestor(target, active))
            .transpose()
            .map(|allowed| allowed.unwrap_or(true))
    }

    pub(in crate::world) fn planned_document_order(
        &mut self,
        document: DocumentId,
    ) -> Result<Vec<StableNodeId>, UiWorldError> {
        let ids = self
            .source
            .document_roots(document)
            .into_iter()
            .chain(
                self.nodes
                    .iter()
                    .filter_map(|(id, node)| (node.document == document).then_some(*id)),
            )
            .collect::<HashSet<_>>();
        self.scanned = self.scanned.saturating_add(ids.len());
        let mut roots = Vec::new();
        for id in ids {
            if self.exists(id)
                && !self.is_parked(id)
                && !self.is_detached(id)
                && self.node(id)?.document == document
                && self.node(id)?.parent.is_none()
            {
                roots.push(id);
            }
        }
        roots.sort_unstable();
        let mut order = Vec::new();
        let mut stack = roots.into_iter().rev().collect::<Vec<_>>();
        while let Some(id) = stack.pop() {
            if !self.exists(id) || self.is_parked(id) || self.is_detached(id) {
                continue;
            }
            order.push(id);
            let children = self.node(id)?.children.clone();
            stack.extend(children.into_iter().rev());
        }
        Ok(order)
    }

    pub(in crate::world) fn node(
        &mut self,
        id: StableNodeId,
    ) -> Result<&PlannedNode, UiWorldError> {
        self.ensure(id)?;
        self.drop_leaving(id);
        Ok(self.nodes.get(&id).expect("ensured node must exist"))
    }

    pub(in crate::world) fn node_mut(
        &mut self,
        id: StableNodeId,
    ) -> Result<&mut PlannedNode, UiWorldError> {
        self.restructured = true;
        self.ensure(id)?;
        self.drop_leaving(id);
        Ok(self.nodes.get_mut(&id).expect("ensured node must exist"))
    }

    /// Take the children detached from `id` out of its staged list, before
    /// anything reads it.
    fn drop_leaving(&mut self, id: StableNodeId) {
        if self.leaving.is_empty() {
            return;
        }
        let Some(leaving) = self.leaving.remove(&id) else {
            return;
        };
        let node = self.nodes.get_mut(&id).expect("ensured node must exist");
        #[cfg(test)]
        {
            self.child_list_steps += node.children.len() as u64;
        }
        node.children.retain(|child| !leaving.contains(child));
    }

    pub(in crate::world) fn ensure(&mut self, id: StableNodeId) -> Result<(), UiWorldError> {
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

impl UiWorld {
    /// A structural mutation that leaves the tree exactly as it is: an
    /// `Insert` that puts a child back where it already sits, or parking or
    /// detaching a root nothing holds any more.
    ///
    /// Applying one would still invalidate layout up the ancestor chain and
    /// retire the subtree from its document again. Refreshing a list by
    /// re-appending and re-parking what is already in place is the common
    /// case; skipping these is what makes it free.
    pub(super) fn is_structural_noop(&self, mutation: &UiMutation) -> bool {
        match mutation {
            UiMutation::SetInlineContent { id, content } => {
                self.inline_content(*id).as_ref() == Some(content)
                    && self.localized_text(*id).is_none()
            }
            UiMutation::Insert {
                parent,
                child,
                before,
            } => {
                if self.nodes.get(*child).map(|node| node.hierarchy.parent) != Some(Some(*parent)) {
                    return false;
                }
                let Some(siblings) = self.nodes.get(*parent).map(|node| &node.hierarchy.children)
                else {
                    return false;
                };
                // `child`'s parent is `parent`, so it is in the list.
                match before {
                    None => siblings.last() == Some(child),
                    Some(before) if before == child => true,
                    Some(before) => siblings
                        .iter()
                        .position(|id| id == child)
                        .is_some_and(|index| siblings.get(index + 1) == Some(before)),
                }
            }
            UiMutation::ReorderChildren { parent, order } => self.children_in_order(*parent, order),
            UiMutation::ParkSubtree { root } => {
                self.unlinked_root(*root) && self.mount_state(*root) == Some(MountState::Parked)
            }
            UiMutation::Detach { id } => self.unlinked_root(*id),
            UiMutation::SetLanguage { id, language } => {
                self.nodes.language(*id) == language.as_deref()
            }
            UiMutation::SetTextScale { id, scale } => self.nodes.text_scale(*id).copied() == *scale,
            UiMutation::SetExplicitRect { id, rect } => {
                self.explicit_rect(*id) == rect.as_deref().copied()
            }
            UiMutation::SetResponsive { id, rule } => self.responsive_rule(*id) == rule.as_ref(),
            UiMutation::SetLocale { id, locale } => {
                self.scope_locale(LocaleScope::Node(*id)) == locale.as_deref()
            }
            UiMutation::SetLocalizedText { id, text } => {
                self.localized_text(*id) == text.as_deref()
            }
            UiMutation::SetReplacedMetadata { resource, metadata } => {
                self.replaced_metadata(resource) == metadata.as_deref().copied()
            }
            UiMutation::PatchPlacement { id, top, height } => {
                self.node_style(*id).is_some_and(|style| {
                    super::apply_topology::placement_already(style.layout.as_ref(), *top, *height)
                })
            }
            _ => false,
        }
    }

    /// A skipped no-op that sets the typography scale a scope already has
    /// counts as one: nothing is visited for it.
    pub(crate) fn note_noop_text_scale(&mut self, mutation: &UiMutation) {
        if matches!(mutation, UiMutation::SetTextScale { .. }) {
            self.note_equivalent_text_scale();
        }
    }

    /// Whether committing `queue` would change nothing: every mutation in it
    /// is a structural no-op. A no-op changes nothing, so judging each one
    /// against the state the batch starts from is exact.
    pub(crate) fn is_noop_batch(&self, queue: &MutationQueue) -> bool {
        queue
            .as_slice()
            .iter()
            .all(|mutation| self.is_structural_noop(mutation))
    }

    /// Detached or parked, with no parent: nothing left to unlink.
    fn unlinked_root(&self, id: StableNodeId) -> bool {
        self.tree.detached.contains(&id)
            && self
                .nodes
                .get(id)
                .is_some_and(|node| node.hierarchy.parent.is_none())
    }

    pub(in crate::world) fn identity_and_parent(
        &self,
        id: StableNodeId,
    ) -> Result<(DocumentId, Option<StableNodeId>), UiWorldError> {
        let node = self.nodes.get(id).ok_or(UiWorldError::MissingNode(id))?;
        Ok((node.document, node.hierarchy.parent))
    }
}

impl UiWorld {
    /// Single-node creation and detached append have no staged cross-mutation
    /// state to simulate. Validate them directly so retained DOM/component
    /// construction does not scan the world or clone a growing child list.
    pub(in crate::world) fn validate_simple_mutation(
        &self,
        mutations: &[UiMutation],
    ) -> Result<bool, UiWorldError> {
        if let [UiMutation::Create { id, .. }, rest @ ..] = mutations {
            if !self.creates_plainly(*id, rest) {
                return Ok(false);
            }
            if self.contains(*id) {
                return Err(UiWorldError::DuplicateNode(*id));
            }
            if self.is_retired(*id) {
                return Err(UiWorldError::RetiredNode(*id));
            }
            for mutation in rest {
                if let UiMutation::SetStyle { style, .. } = mutation
                    && !style_is_valid(style.layout.as_ref())
                {
                    return Err(UiWorldError::InvalidStyle(*id));
                }
            }
            return Ok(true);
        }
        // A layout pass's writeback: boxes of nodes that exist, no
        // structure, so nothing staged for the plan to follow.
        if mutations.iter().all(|mutation| {
            matches!(
                mutation,
                UiMutation::WriteLayout { .. } | UiMutation::PlaceLayout { .. }
            )
        }) {
            for mutation in mutations {
                let (UiMutation::WriteLayout { id, layout }
                | UiMutation::PlaceLayout { id, layout, .. }) = mutation
                else {
                    unreachable!("matched above");
                };
                if !self.contains(*id) {
                    return Err(UiWorldError::MissingNode(*id));
                }
                if !layout_is_valid(layout) {
                    return Err(UiWorldError::InvalidLayout(*id));
                }
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
        if parent == child {
            return Err(UiWorldError::Cycle {
                parent: *parent,
                child: *child,
            });
        }
        let childless = self
            .nodes
            .get(*child)
            .is_some_and(|node| node.hierarchy.children.is_empty());
        let depth = match self.node_depth(*parent) {
            // A child with no children cannot hold `parent`, so no cycle:
            // the depth `parent` keeps is all there is to check.
            Some(depth) if childless => depth,
            _ => {
                let mut ancestor = Some(*parent);
                let mut depth = 0usize;
                while let Some(id) = ancestor {
                    if id == *child {
                        return Err(UiWorldError::Cycle {
                            parent: *parent,
                            child: *child,
                        });
                    }
                    depth += 1;
                    ancestor = self.identity_and_parent(id)?.1;
                }
                depth
            }
        };
        // Near the depth limit the child's own height decides, and measuring it
        // is exactly the walk this fast path exists to avoid. Hand those rare
        // batches to the planner, which already bounds depth.
        Ok(depth < MAX_TREE_DEPTH)
    }
}

impl UiWorld {
    /// Whether a batch that creates `id` and then only writes `id`'s
    /// style, interaction, accessibility or component type, parking it last
    /// or not -- what creating a component is -- needs nothing of the
    /// staged validation beyond the checks `validate_simple_mutation` makes
    /// itself. A node that does not exist yet has no parent, children,
    /// focus, pointer capture, animation or overlay reference for the plan
    /// to follow, and no inline content to check (with none in the world).
    fn creates_plainly(&self, id: StableNodeId, rest: &[UiMutation]) -> bool {
        let writes = match rest {
            [writes @ .., UiMutation::ParkSubtree { root }] if *root == id => writes,
            writes => writes,
        };
        !self.nodes.has_inline_content()
            && !self.overlays_a11y.overlay_dependents.contains_key(&id)
            && writes.iter().all(|mutation| match mutation {
                UiMutation::SetStyle { id: target, .. }
                | UiMutation::SetInteraction { id: target, .. }
                | UiMutation::SetAccessibility { id: target, .. }
                | UiMutation::SetComponentType { id: target, .. } => *target == id,
                _ => false,
            })
    }
}

/// Whether a style's paint and font values are in range: what a
/// `SetStyle` is validated against.
pub(in crate::world) fn style_is_valid(layout: &nana_ui_core::LayoutStyle) -> bool {
    let channel_ok = |channel: f32| channel.is_finite() && (0.0..=1.0).contains(&channel);
    !(layout
        .opacity
        .is_some_and(|opacity| !opacity.is_finite() || !(0.0..=1.0).contains(&opacity))
        || layout
            .font_size
            .is_some_and(|size| !size.is_finite() || size <= 0.0)
        || layout
            .text_ext
            .letter_spacing
            .is_some_and(|spacing| !spacing.is_finite())
        || layout
            .font_weight
            .is_some_and(|weight| !(1..=1000).contains(&weight))
        || layout
            .color
            .is_some_and(|color| !color.into_iter().all(channel_ok))
        || layout
            .background
            .is_some_and(|color| !color.into_iter().all(channel_ok))
        || layout
            .border_color
            .is_some_and(|color| !color.into_iter().all(channel_ok)))
}

/// Whether a written box is finite and of non-negative size.
pub(in crate::world) fn layout_is_valid(layout: &LayoutBox) -> bool {
    layout.x.is_finite()
        && layout.y.is_finite()
        && layout.width.is_finite()
        && layout.height.is_finite()
        && layout.width >= 0.0
        && layout.height >= 0.0
}
