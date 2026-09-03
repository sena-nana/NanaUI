//! Atomic mutation validation and application.

use super::*;

/// Validate-then-apply staging for one mutation batch.
///
/// Every field is an overlay over `source`, never a copy of it: validation cost
/// tracks the batch, not the retained world. `parked`, `pointer_captures`, and
/// `animations` fall back to `source` on a miss instead of being pre-filled, and
/// overlay-host walks use `UiWorld::overlay_host_ids`.
pub(super) struct ValidationPlan<'a> {
    pub(super) source: &'a UiWorld,
    pub(super) nodes: HashMap<StableNodeId, PlannedNode>,
    pub(super) removed: HashSet<StableNodeId>,
    pub(super) newly_retired: HashSet<StableNodeId>,
    /// Mount overrides staged by this batch. Absent means "ask `source`".
    pub(super) parked: HashMap<StableNodeId, bool>,
    pub(super) interactions: HashMap<StableNodeId, InteractionState>,
    pub(super) styles: HashMap<StableNodeId, NodeStyle>,
    pub(super) focus: HashMap<DocumentId, Option<StableNodeId>>,
    /// Cloned from `source` on first pointer-capture mutation. Batches that do
    /// not touch capture never pay for it.
    pub(super) pointer_captures: Option<HashMap<(DocumentId, u64), StableNodeId>>,
    /// Cloned from `source` on first animation mutation, same rationale.
    pub(super) animations: Option<HashMap<AnimationId, AnimationSpec>>,
    pub(super) text_inputs: HashMap<StableNodeId, Option<TextInputState>>,
    pub(super) surface_open: HashMap<StableNodeId, bool>,
    pub(super) overlay_hosts: HashMap<StableNodeId, OverlayHostState>,
    pub(super) accessibility: HashMap<StableNodeId, AccessibilityState>,
    /// Nodes visited by whole-set walks during this validation. Reported through
    /// `UiWorld::validation_nodes_scanned` so a reintroduced world scan fails a
    /// test instead of silently costing a frame.
    pub(super) scanned: usize,
}

impl<'a> ValidationPlan<'a> {
    pub(super) fn new(source: &'a UiWorld) -> Self {
        Self {
            source,
            nodes: HashMap::new(),
            removed: HashSet::new(),
            newly_retired: HashSet::new(),
            parked: HashMap::new(),
            interactions: HashMap::new(),
            styles: HashMap::new(),
            focus: HashMap::new(),
            pointer_captures: None,
            animations: None,
            text_inputs: HashMap::new(),
            surface_open: HashMap::new(),
            overlay_hosts: HashMap::new(),
            accessibility: HashMap::new(),
            scanned: 0,
        }
    }

    /// Staged mount state. A node this batch created has no `MountState` in
    /// `source`, and `mount_state` returning `None` correctly reads as unparked.
    pub(super) fn is_parked(&self, id: StableNodeId) -> bool {
        if let Some(&parked) = self.parked.get(&id) {
            return parked;
        }
        self.source.mount_state(id) == Some(MountState::Parked)
    }

    pub(super) fn set_parked(&mut self, id: StableNodeId, parked: bool) {
        self.parked.insert(id, parked);
    }

    pub(super) fn pointer_captures_mut(&mut self) -> &mut HashMap<(DocumentId, u64), StableNodeId> {
        if self.pointer_captures.is_none() {
            self.pointer_captures = Some(self.source.input.pointer_captures.clone());
        }
        self.pointer_captures
            .as_mut()
            .expect("initialized directly above")
    }

    pub(super) fn pointer_capture(
        &self,
        document: DocumentId,
        pointer_id: u64,
    ) -> Option<StableNodeId> {
        match &self.pointer_captures {
            Some(captures) => captures.get(&(document, pointer_id)).copied(),
            None => self.source.pointer_capture(document, pointer_id),
        }
    }

    pub(super) fn animations_mut(&mut self) -> &mut HashMap<AnimationId, AnimationSpec> {
        if self.animations.is_none() {
            self.animations = Some(
                self.source
                    .animations
                    .iter()
                    .map(|(&id, animation)| (id, animation.spec))
                    .collect(),
            );
        }
        self.animations
            .as_mut()
            .expect("initialized directly above")
    }

    /// Overlay hosts staged by this batch plus those already in `source`.
    pub(super) fn overlay_host_candidates(&mut self) -> Vec<StableNodeId> {
        let mut hosts = self.overlay_hosts.keys().copied().collect::<HashSet<_>>();
        hosts.extend(self.source.overlay_host_ids());
        self.scanned = self.scanned.saturating_add(hosts.len());
        hosts.into_iter().collect()
    }

    pub(super) fn validate(&mut self, mutations: &[UiMutation]) -> Result<(), UiWorldError> {
        for mutation in mutations {
            match mutation {
                UiMutation::Create { id, document, .. } => self.create(*id, *document)?,
                UiMutation::Insert {
                    parent,
                    child,
                    before,
                } => self.insert(*parent, *child, *before)?,
                UiMutation::Detach { id } => {
                    self.detach(*id)?;
                }
                UiMutation::ParkSubtree { root } => self.park(*root)?,
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
                UiMutation::SetTheme { .. } | UiMutation::SetStyleTokens { .. } => {}
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
                UiMutation::SetEventListener { id, event, .. } => {
                    self.node(*id)?;
                    if event.trim().is_empty() {
                        return Err(UiWorldError::InvalidEventListener(*id));
                    }
                }
                UiMutation::SetComponentType { id, .. } => {
                    self.node(*id)?;
                }
                UiMutation::SetStandardVisual { id, visual } => {
                    self.node(*id)?;
                    let invalid_ratio = match visual {
                        Some(StandardVisual::Range { ratio, .. })
                        | Some(StandardVisual::Progress {
                            value_ratio: ratio, ..
                        })
                        | Some(StandardVisual::LevelMeter {
                            value_ratio: ratio, ..
                        }) => !ratio.is_finite() || !(0.0..=1.0).contains(ratio),
                        _ => false,
                    };
                    if invalid_ratio {
                        return Err(UiWorldError::InvalidStandardVisual(*id));
                    }
                }
                UiMutation::SetAccessibility { id, accessibility } => {
                    self.node(*id)?;
                    self.accessibility.insert(*id, accessibility.clone());
                }
                UiMutation::SetSurfaceOpen { id, open, .. } => {
                    self.node(*id)?;
                    self.surface_open.insert(*id, *open);
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
                    if !animation.is_valid() || self.is_parked(animation.target) {
                        return Err(UiWorldError::InvalidAnimation(animation.id));
                    }
                    self.animations_mut().insert(animation.id, *animation);
                }
                UiMutation::StopAnimation { id } => {
                    if self.animations_mut().remove(id).is_none() {
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
                    if let Some(ImeComposition {
                        text,
                        selection: Some((start, end)),
                    }) = composition
                        && (start > end
                            || *end > text.len()
                            || !text.is_char_boundary(*start)
                            || !text.is_char_boundary(*end)
                            || !crate::TextSelection {
                                anchor: *start,
                                focus: *end,
                            }
                            .is_valid_for(text))
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
                UiMutation::SetHighlightRequest { id, request } => {
                    self.node(*id)?;
                    if request
                        .as_ref()
                        .is_some_and(|request| request.presenter.trim().is_empty())
                    {
                        return Err(UiWorldError::InvalidHighlightRequest(*id));
                    }
                }
                UiMutation::SetTextInputFoldCollapsed { id, .. }
                | UiMutation::SetTextInputSnippet { id, .. }
                | UiMutation::SetTextInputCompletions { id, .. }
                | UiMutation::SetTextInputCompletionView { id, .. }
                | UiMutation::SetTextInputCompletionDismissed { id }
                | UiMutation::SetTextInputCompletionReopened { id }
                | UiMutation::SetTextInputHover { id, .. }
                | UiMutation::SetTextInputHoverScroll { id, .. } => {
                    self.node(*id)?;
                }
            }
        }
        self.validate_overlay_hosts()?;
        Ok(())
    }

    pub(super) fn create(
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

    pub(super) fn insert(
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
        let depth = self
            .ancestor_depth(parent)?
            .saturating_add(self.subtree_height(child)?);
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
        let parked = self.is_parked(parent);
        self.set_parked_subtree(child, parked)?;
        Ok(())
    }

    /// Levels from `id` up to its root, counting `id` itself.
    pub(super) fn ancestor_depth(&mut self, id: StableNodeId) -> Result<usize, UiWorldError> {
        let mut depth = 1;
        let mut cursor = self.node(id)?.parent;
        while let Some(ancestor) = cursor {
            depth += 1;
            if depth > MAX_TREE_DEPTH {
                // `has_ancestor` already rejected cycles, so this is genuine
                // depth rather than a loop.
                return Ok(depth);
            }
            cursor = self.node(ancestor)?.parent;
        }
        Ok(depth)
    }

    /// Levels from `root` down to its deepest descendant, counting `root`.
    /// Stops climbing once past the limit; the caller only needs to know that.
    pub(super) fn subtree_height(&mut self, root: StableNodeId) -> Result<usize, UiWorldError> {
        let mut frontier = vec![root];
        let mut height = 0;
        while !frontier.is_empty() {
            height += 1;
            if height > MAX_TREE_DEPTH {
                return Ok(height);
            }
            let mut next = Vec::new();
            for id in frontier {
                next.extend(self.node(id)?.children.iter().copied());
            }
            frontier = next;
        }
        Ok(height)
    }

    pub(super) fn park(&mut self, root: StableNodeId) -> Result<(), UiWorldError> {
        self.detach(root)?;
        let subtree = self.subtree(root)?;
        self.set_parked_subtree(root, true)?;
        let parked = subtree.iter().copied().collect::<HashSet<_>>();
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
            if focused.is_some_and(|target| parked.contains(&target)) {
                self.focus.insert(document, None);
            }
        }
        if self.pointer_captures.is_some() || !self.source.input.pointer_captures.is_empty() {
            self.pointer_captures_mut()
                .retain(|_, target| !parked.contains(target));
        }
        if self.animations.is_some() || !self.source.animations.is_empty() {
            self.animations_mut()
                .retain(|_, animation| !parked.contains(&animation.target));
        }
        for id in subtree {
            self.clear_overlay_references(id);
        }
        Ok(())
    }

    pub(super) fn subtree(
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

    pub(super) fn set_parked_subtree(
        &mut self,
        root: StableNodeId,
        parked: bool,
    ) -> Result<(), UiWorldError> {
        for id in self.subtree(root)? {
            self.set_parked(id, parked);
        }
        Ok(())
    }

    pub(super) fn detach(&mut self, id: StableNodeId) -> Result<(), UiWorldError> {
        let parent = self.node(id)?.parent;
        if let Some(parent) = parent {
            self.node_mut(parent)?.children.retain(|child| *child != id);
            self.node_mut(id)?.parent = None;
        }
        Ok(())
    }

    pub(super) fn despawn_subtree(&mut self, root: StableNodeId) -> Result<(), UiWorldError> {
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
            if self.animations.is_some() || !self.source.animations.is_empty() {
                self.animations_mut()
                    .retain(|_, animation| animation.target != id);
            }
            self.text_inputs.remove(&id);
            self.clear_overlay_references(id);
        }
        Ok(())
    }

    pub(super) fn clear_overlay_references(&mut self, removed: StableNodeId) {
        for host in self.overlay_host_candidates() {
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

    pub(super) fn validate_overlay_hosts(&mut self) -> Result<(), UiWorldError> {
        for host in self.overlay_host_candidates() {
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
            if let Some(active) = state.active {
                let accessibility = self
                    .accessibility
                    .get(&active)
                    .or_else(|| self.source.accessibility(active));
                if !accessibility.is_some_and(|accessibility| match accessibility.role {
                    AccessibilityRole::Dialog | AccessibilityRole::AlertDialog => {
                        accessibility.modal
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

    pub(super) fn has_ancestor(
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

    pub(super) fn exists(&self, id: StableNodeId) -> bool {
        !self.removed.contains(&id) && (self.nodes.contains_key(&id) || self.source.contains(id))
    }

    pub(super) fn text_input(&mut self, id: StableNodeId) -> Result<TextInputState, UiWorldError> {
        self.node(id)?;
        if let Some(state) = self.text_inputs.get(&id) {
            return state.clone().ok_or(UiWorldError::MissingTextInput(id));
        }
        self.source
            .text_input(id)
            .cloned()
            .ok_or(UiWorldError::MissingTextInput(id))
    }

    pub(super) fn overlay_branch_active(&mut self, id: StableNodeId) -> Result<bool, UiWorldError> {
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

    pub(super) fn focus_target_visible(
        &mut self,
        mut id: StableNodeId,
    ) -> Result<bool, UiWorldError> {
        loop {
            if self.is_parked(id) {
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
            if layout.is_some_and(|layout| layout.omits_box())
                || !self.overlay_branch_active(id)?
                || self
                    .node(id)?
                    .parent
                    .and_then(|parent| self.source.standard_visual(parent))
                    .is_some_and(|visual| {
                        matches!(visual, StandardVisual::MenuSurface { open: false, .. })
                    })
            {
                return Ok(false);
            }
            let Some(parent) = self.node(id)?.parent else {
                return Ok(true);
            };
            id = parent;
        }
    }

    pub(super) fn active_modal_allows_focus(
        &mut self,
        document: DocumentId,
        target: StableNodeId,
    ) -> Result<bool, UiWorldError> {
        let hosts = self.overlay_host_candidates();
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

    pub(super) fn planned_document_order(
        &mut self,
        document: DocumentId,
    ) -> Result<Vec<StableNodeId>, UiWorldError> {
        let ids = self
            .source
            .nodes
            .keys()
            .chain(self.nodes.keys().copied())
            .collect::<HashSet<_>>();
        self.scanned = self.scanned.saturating_add(ids.len());
        let mut roots = Vec::new();
        for id in ids {
            if self.exists(id)
                && !self.is_parked(id)
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
            if !self.exists(id) || self.is_parked(id) {
                continue;
            }
            order.push(id);
            let children = self.node(id)?.children.clone();
            stack.extend(children.into_iter().rev());
        }
        Ok(order)
    }

    pub(super) fn node(&mut self, id: StableNodeId) -> Result<&PlannedNode, UiWorldError> {
        self.ensure(id)?;
        Ok(self.nodes.get(&id).expect("ensured node must exist"))
    }

    pub(super) fn node_mut(&mut self, id: StableNodeId) -> Result<&mut PlannedNode, UiWorldError> {
        self.ensure(id)?;
        Ok(self.nodes.get_mut(&id).expect("ensured node must exist"))
    }

    pub(super) fn ensure(&mut self, id: StableNodeId) -> Result<(), UiWorldError> {
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
    pub(super) fn apply(&mut self, mutation: &UiMutation, report: &mut CommitReport) {
        match mutation {
            UiMutation::Create { id, document, kind } => {
                self.nodes.insert(
                    *id,
                    NodeRecord::new(*document, kind, initial_interaction(kind)),
                );
                self.dirty_entities.insert(*id);
                self.spawned_since_drain += 1;
                report.created += 1;
                self.refresh_root_membership(*id);
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
                    let hierarchy = self.hierarchy_mut(old_parent);
                    Arc::make_mut(&mut hierarchy.children).retain(|id| id != child);
                    intern_empty_children(&mut hierarchy.children);
                }
                let parent_hierarchy = self.hierarchy_mut(*parent);
                let siblings = Arc::make_mut(&mut parent_hierarchy.children);
                let index = before
                    .and_then(|before| siblings.iter().position(|id| *id == before))
                    .unwrap_or(siblings.len());
                siblings.insert(index, *child);
                let _parent_hierarchy = parent_hierarchy;
                self.hierarchy_mut(*child).parent = Some(*parent);
                let parent_mount = self.record(*parent).mount;
                if self.record(*child).mount != parent_mount {
                    self.set_subtree_mount_state(*child, parent_mount);
                }
                if old_parent == Some(*parent) {
                    // Retained-order moves carry the entire subtree; descendants
                    // keep their inherited state and local geometry until layout
                    // writeback identifies actual changes.
                    self.mark(
                        *child,
                        DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                } else {
                    self.mark_subtree(*child, DirtyMask::ALL);
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
                self.detached.remove(child);
                self.sync_subtree_presence(*child);
                self.refresh_root_membership(*child);
                self.note_structural_change(*parent);
                if let Some(old_parent) = old_parent {
                    self.note_structural_change(old_parent);
                }
            }
            UiMutation::Detach { id } => {
                if self.unlink_from_parent(*id) {
                    report.detached += 1;
                }
                self.leave_live_document(*id);
                self.refresh_root_membership(*id);
            }
            UiMutation::ParkSubtree { root } => {
                self.unlink_from_parent(*root);
                self.set_subtree_mount_state(*root, MountState::Parked);
                self.leave_live_document(*root);
                self.refresh_root_membership(*root);
            }
            UiMutation::DespawnSubtree { root } => {
                let root_snapshot = self.node(*root).expect("validated root must exist");
                if let Some(parent) = root_snapshot.parent {
                    let hierarchy = self.hierarchy_mut(parent);
                    Arc::make_mut(&mut hierarchy.children).retain(|child| child != root);
                    intern_empty_children(&mut hierarchy.children);
                    let _hierarchy = hierarchy;
                    self.mark_ancestors(
                        parent,
                        DirtyMask::LAYOUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                    self.note_structural_change(parent);
                }
                let mut stack = vec![*root];
                while let Some(id) = stack.pop() {
                    let snapshot = self.node(id).expect("validated subtree must exist");
                    stack.extend(snapshot.children.iter().rev().copied());
                    self.forget_visual_presence(id);
                    let _removed = self.nodes.remove(id);
                    self.dirty_entities.remove(&id);
                    self.refresh_root_membership(id);
                    if self.input.focused.get(&snapshot.document) == Some(&id) {
                        self.input.focused.remove(&snapshot.document);
                    }
                    if let Some(index) = self.hit_test_index.get_mut(&snapshot.document) {
                        retain_hit_tree(index, id);
                    }
                    let released = self
                        .input
                        .pointer_captures
                        .iter()
                        .filter_map(|(&(document, pointer_id), &target)| {
                            (target == id).then_some((document, pointer_id))
                        })
                        .collect::<Vec<_>>();
                    for key @ (_, pointer_id) in released {
                        self.input.pointer_captures.remove(&key);
                        self.input
                            .pending_pointer_capture_changes
                            .push(PointerCaptureChange {
                                pointer_id,
                                target: id,
                                captured: false,
                            });
                    }
                    self.input.pointer_hover.retain(|_, target| *target != id);
                    self.input.pointer_press.retain(|_, target| *target != id);
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
                    self.surface_motion.remove(&id);
                    self.closing_surfaces.remove(&id);
                    self.switch_transitions.remove(&id);
                    self.hover_transitions.remove(&id);
                    self.clear_overlay_references(id);
                    self.overlay_host_nodes.remove(&id);
                    self.detached.remove(&id);
                    self.retired.insert(id);
                    self.pending_render_removals.push(id);
                    self.pending_accessibility_removals.push(id);
                    self.despawned_since_drain += 1;
                    report.despawned += 1;
                }
                self.refresh_root_membership(*root);
            }
            UiMutation::SetStyle { id, style } => {
                let previous = self.record(*id).style.clone();
                let inherited_text_changed = previous.layout.font_size != style.layout.font_size
                    || previous.layout.font_weight != style.layout.font_weight
                    || previous.layout.font_italic != style.layout.font_italic
                    || previous.layout.font_family != style.layout.font_family
                    || previous.layout.line_height != style.layout.line_height
                    || previous.layout.letter_spacing != style.layout.letter_spacing
                    || previous.layout.font_features != style.layout.font_features
                    || previous.layout.font_variation_settings
                        != style.layout.font_variation_settings
                    || previous.layout.font_kerning != style.layout.font_kerning
                    || previous.layout.word_break != style.layout.word_break
                    || previous.layout.line_break != style.layout.line_break;
                let inherited_paint_changed = previous.foreground != style.foreground
                    || previous.layout.color != style.layout.color
                    || previous.layout.opacity != style.layout.opacity;
                let paint_visibility_changed =
                    previous.layout.paint.visibility != style.layout.paint.visibility;
                let pointer_events_changed =
                    previous.layout.pointer_events != style.layout.pointer_events;
                let omits_box_changed = previous.layout.omits_box() != style.layout.omits_box();
                let transform_changed = previous.layout.transform != style.layout.transform
                    || previous.layout.transform_3d != style.layout.transform_3d
                    || previous.layout.transform_origin != style.layout.transform_origin
                    || previous.layout.transform_box != style.layout.transform_box
                    || previous.layout.css_perspective != style.layout.css_perspective
                    || previous.layout.preserve_3d != style.layout.preserve_3d
                    || previous.layout.unsupported_transform != style.layout.unsupported_transform;
                let stacking_changed = previous.layout.z_index != style.layout.z_index
                    || previous.layout.isolation != style.layout.isolation;
                let layout_changed =
                    layout_semantics_changed(previous.layout.as_ref(), style.layout.as_ref());
                self.record_mut(*id).style = style.clone();
                self.sync_node_presence(*id);

                if !style_excluding_transform_eq(&previous, style) {
                    self.mark(*id, DirtyMask::STYLE | DirtyMask::RENDER);
                }
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
                if omits_box_changed {
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
                } else if paint_visibility_changed {
                    self.mark_subtree(
                        *id,
                        DirtyMask::STYLE
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                    if let Some(parent) = self.node(*id).and_then(|node| node.parent) {
                        self.mark(parent, DirtyMask::ACCESSIBILITY);
                    }
                }
                if pointer_events_changed {
                    // Inherited: unspecified descendants pick up the new used
                    // value. Not a layout dirty.
                    self.mark_subtree(*id, DirtyMask::STYLE | DirtyMask::INPUT);
                    self.clear_hover_for_pointer_events_none(*id);
                }
                if transform_changed {
                    // Scene extract and hit-test read `layout.transform`; LAYOUT
                    // does not, so paint-transform is not a layout dirty.
                    self.mark_subtree(
                        *id,
                        DirtyMask::TRANSFORM | DirtyMask::INPUT | DirtyMask::RENDER,
                    );
                } else if stacking_changed {
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
                if (layout_changed || inherited_text_changed || omits_box_changed)
                    && let Some(parent) = self.node(*id).and_then(|node| node.parent)
                {
                    self.mark_ancestors(parent, DirtyMask::LAYOUT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetTheme { mode } => {
                self.apply_style_model(StyleModelRef::new(*mode));
            }
            UiMutation::SetStyleTokens {
                mode,
                metrics,
                palette,
                titlebar,
            } => {
                self.apply_style_model(StyleModelRef::with_tokens(
                    *mode, *metrics, **palette, *titlebar,
                ));
            }
            UiMutation::SetText { id, text } => {
                self.record_mut(*id).text = text.clone();
                self.mark(
                    *id,
                    DirtyMask::TEXT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::WriteLayout { id, layout } => {
                self.record_mut(*id).layout = *layout;
                // Scoped layout already emits every recomputed box, including
                // shifted descendants. Mark only this node so a bit-identical
                // child is not extracted solely because an ancestor was written.
                self.mark(
                    *id,
                    DirtyMask::INPUT | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetScrollOffset { id, offset } => {
                let offset = self.clamp_scroll_offset(*id, *offset);
                let previous = self.record(*id).scroll_offset;
                if previous != offset {
                    self.record_mut(*id).scroll_offset = offset;
                    // Hit-index patch + Scene extract of this scroller only.
                    // Descendants keep LayoutBox; paint uses scroll_offset.
                    self.scroll_hit_updates
                        .push((*id, [previous.x - offset.x, previous.y - offset.y]));
                    self.mark(*id, DirtyMask::INPUT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetScrollMetrics { id, metrics } => {
                self.nodes.set_scroll_metrics(*id, *metrics);
                let current = self.record(*id).scroll_offset;
                let clamped = self.clamp_scroll_offset(*id, current);
                if current != clamped {
                    self.record_mut(*id).scroll_offset = clamped;
                    self.scroll_hit_updates
                        .push((*id, [current.x - clamped.x, current.y - clamped.y]));
                    self.mark(*id, DirtyMask::INPUT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetInteraction { id, interaction } => {
                self.record_mut(*id).interaction = *interaction;
                if !interaction.pointer_events {
                    self.input.pointer_hover.retain(|_, target| target != id);
                    self.input.pointer_press.retain(|_, target| target != id);
                }
                self.mark(
                    *id,
                    DirtyMask::STATE
                        | DirtyMask::INPUT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetCustomRender { id, content } => {
                self.nodes.set_custom_render(*id, content.clone());
                self.mark(*id, DirtyMask::RENDER);
            }
            UiMutation::SetEventListener { id, event, enabled } => {
                let mut listeners = self.nodes.event_listeners(*id).cloned().unwrap_or_default();
                listeners.set(event.clone(), *enabled);
                if listeners.is_empty() {
                    self.nodes.set_event_listeners(*id, None);
                } else {
                    self.nodes.set_event_listeners(*id, Some(listeners));
                }
            }
            UiMutation::SetComponentType { id, type_id } => {
                let current = self.nodes.component_type(*id);
                if current != type_id.as_ref() {
                    self.nodes.set_component_type(*id, type_id.clone());
                }
            }
            UiMutation::SetStandardVisual { id, visual } => {
                if let (
                    Some(StandardVisual::Switch {
                        checked: old,
                        thumb_progress,
                        ..
                    }),
                    Some(StandardVisual::Switch { checked: next, .. }),
                ) = (self.nodes.visual(*id), visual.as_ref())
                    && old != next
                {
                    self.switch_transitions.insert(*id, *thumb_progress);
                    self.start_component_animation(
                        *id,
                        crate::component_animation_kinds::SWITCH,
                        nana_ui_core::motion::OVERLAY_FADE,
                        crate::Easing::EaseOutCubic,
                    );
                }

                let (
                    text_input_presentation_changed,
                    empty_state_presentation_changed,
                    modal_presentation_changed,
                    modal_state_changed,
                    menu_state_changed,
                    text_folds_changed,
                ) = {
                    let previous_visual = self.nodes.visual(*id);
                    let text_folds_changed = match (previous_visual, visual.as_ref()) {
                        (
                            Some(StandardVisual::TextInput {
                                folds: previous, ..
                            }),
                            Some(StandardVisual::TextInput { folds: next, .. }),
                        ) => previous != next,
                        (Some(StandardVisual::TextInput { .. }), _) => true,
                        (_, Some(StandardVisual::TextInput { folds, .. })) => !folds.is_empty(),
                        _ => false,
                    };
                    (
                        matches!(previous_visual, Some(StandardVisual::TextInput { .. }))
                            || matches!(visual, Some(StandardVisual::TextInput { .. })),
                        matches!(previous_visual, Some(StandardVisual::EmptyState { .. }))
                            || matches!(visual, Some(StandardVisual::EmptyState { .. })),
                        matches!(previous_visual, Some(StandardVisual::ModalFrame { .. }))
                            || matches!(visual, Some(StandardVisual::ModalFrame { .. })),
                        match (previous_visual, visual) {
                            (
                                Some(StandardVisual::ModalFrame {
                                    busy: old_busy,
                                    danger: old_danger,
                                    ..
                                }),
                                Some(StandardVisual::ModalFrame { busy, danger, .. }),
                            ) => old_busy != busy || old_danger != danger,
                            _ => false,
                        },
                        // Whether a menu's items take part in layout follows the
                        // surface's own open state, so opening it has to reach
                        // them the way an overlay host reaches its branch.
                        menu_surface_open(previous_visual) != menu_surface_open(visual.as_ref()),
                        text_folds_changed,
                    )
                };
                self.nodes.set_visual(*id, visual.clone());
                self.sync_node_presence(*id);
                if !matches!(visual, Some(StandardVisual::TextInput { .. })) {
                    self.nodes.set_text_input_presentation(*id, None);
                    self.nodes.set_text_viewport_pin(*id, None);
                }
                if !matches!(visual, Some(StandardVisual::EmptyState { .. })) {
                    self.nodes.set_empty_state_text(*id, None);
                }
                if !matches!(visual, Some(StandardVisual::ModalFrame { .. })) {
                    self.nodes.set_modal_text(*id, None);
                }
                if text_folds_changed {
                    let offered = match visual.as_ref() {
                        Some(StandardVisual::TextInput { folds, .. }) if !folds.is_empty() => {
                            Some(Arc::clone(folds))
                        }
                        _ => None,
                    };
                    self.reconcile_text_fold_offered(*id, offered);
                }
                self.mark(
                    *id,
                    DirtyMask::RENDER
                        | if text_input_presentation_changed
                            || empty_state_presentation_changed
                            || modal_presentation_changed
                        {
                            DirtyMask::TEXT | DirtyMask::LAYOUT
                        } else {
                            0
                        },
                );
                if menu_state_changed {
                    self.mark_subtree(
                        *id,
                        DirtyMask::STYLE
                            | DirtyMask::LAYOUT
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                    self.mark_ancestors(*id, DirtyMask::LAYOUT | DirtyMask::RENDER);
                }
                if modal_state_changed {
                    self.mark_subtree(
                        *id,
                        DirtyMask::STYLE
                            | DirtyMask::INPUT
                            | DirtyMask::FOCUS_IME
                            | DirtyMask::ACCESSIBILITY
                            | DirtyMask::RENDER,
                    );
                }
            }
            UiMutation::SetAccessibility { id, accessibility } => {
                let previous = &self.record(*id).accessibility;
                let interaction_style_changed = previous.disabled != accessibility.disabled
                    || previous.checked != accessibility.checked
                    || previous.selected != accessibility.selected;
                self.record_mut(*id).accessibility = accessibility.clone();
                self.mark(*id, DirtyMask::ACCESSIBILITY);
                if interaction_style_changed && !self.record(*id).style.interaction.is_empty() {
                    self.mark(*id, DirtyMask::STYLE | DirtyMask::RENDER);
                }
            }
            UiMutation::SetSurfaceOpen { id, open, menu } => {
                self.set_surface_open(*id, *open, *menu);
            }
            UiMutation::SetOverlayHost { host, state } => {
                let previous = self.nodes.overlay_host(*host).copied();
                if previous == Some(*state) {
                    return;
                }
                self.nodes.set_overlay_host(*host, Some(*state));
                self.overlay_host_nodes.insert(*host);
                self.mark(*host, DirtyMask::ACCESSIBILITY);
                if let Some(inactive) = previous
                    .and_then(|previous| previous.active)
                    .filter(|active| Some(*active) != state.active)
                {
                    self.clear_surface_pointer_interactions(inactive);
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
                let document = self.record(*target).document;
                let previous = self
                    .input
                    .pointer_captures
                    .insert((document, *pointer_id), *target);
                if previous == Some(*target) {
                    return;
                }
                if let Some(previous) = previous {
                    self.input
                        .pending_pointer_capture_changes
                        .push(PointerCaptureChange {
                            pointer_id: *pointer_id,
                            target: previous,
                            captured: false,
                        });
                }
                self.input
                    .pending_pointer_capture_changes
                    .push(PointerCaptureChange {
                        pointer_id: *pointer_id,
                        target: *target,
                        captured: true,
                    });
            }
            UiMutation::ReleasePointer { pointer_id, target } => {
                let document = self.record(*target).document;
                self.input.pointer_captures.remove(&(document, *pointer_id));
                self.input
                    .pending_pointer_capture_changes
                    .push(PointerCaptureChange {
                        pointer_id: *pointer_id,
                        target: *target,
                        captured: false,
                    });
            }
            UiMutation::StartAnimation { animation } => {
                let active = ActiveAnimation::new(*animation);
                let next_deadline = active.next_deadline;
                if let Some(previous) = self.animations.insert(animation.id, active) {
                    self.animation_deadlines
                        .remove(&(previous.next_deadline, animation.id));
                }
                self.animation_deadlines
                    .insert((next_deadline, animation.id));
            }
            UiMutation::StopAnimation { id } => {
                if let Some(animation) = self.animations.remove(id) {
                    self.animation_deadlines
                        .remove(&(animation.next_deadline, *id));
                }
            }
            UiMutation::RequestFocus { document, target } => {
                let old = match target {
                    Some(target) => self.input.focused.insert(*document, *target),
                    None => self.input.focused.remove(document),
                };
                if let Some(old) = old.filter(|old| Some(*old) != *target) {
                    self.remove_ime(old);
                    self.mark(old, DirtyMask::STATE);
                    if !self.record(old).style.interaction.focused.is_empty() {
                        self.mark(old, DirtyMask::STYLE | DirtyMask::RENDER);
                    }
                    self.mark(
                        old,
                        DirtyMask::FOCUS_IME | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                }
                if let Some(target) = target {
                    self.mark(*target, DirtyMask::STATE);
                    if !self.record(*target).style.interaction.focused.is_empty() {
                        self.mark(*target, DirtyMask::STYLE | DirtyMask::RENDER);
                    }
                    self.mark(
                        *target,
                        DirtyMask::FOCUS_IME | DirtyMask::RENDER | DirtyMask::ACCESSIBILITY,
                    );
                }
            }
            UiMutation::SetIme { id, composition } => {
                self.nodes.set_ime(*id, composition.clone());
                self.mark(
                    *id,
                    DirtyMask::TEXT | DirtyMask::FOCUS_IME | DirtyMask::RENDER,
                );
            }
            UiMutation::SetTextInput { id, state } => {
                // 旧值只用于折叠态与 snippet 会话的编辑重映射；不存在这两类
                // 视图状态时跳过克隆，普通文本输入的值变更不再复制整个旧值。
                let previous_value = match state {
                    Some(_)
                        if self.nodes.text_fold_view(*id).is_some()
                            || self.nodes.text_snippet_session(*id).is_some() =>
                    {
                        self.nodes.text_input(*id).map(|input| input.value.clone())
                    }
                    _ => None,
                };
                if let Some(state) = state {
                    self.nodes.set_text_input(*id, Some(state.clone()));
                    self.record_mut(*id).text = TextContent {
                        value: state.value.clone(),
                    };
                } else {
                    self.nodes.set_text_input(*id, None);
                    self.record_mut(*id).text = TextContent::default();
                    self.remove_ime(*id);
                }
                // 值变化后重映射折叠态与 snippet 会话：受影响的折叠自动
                // 展开，跳位失效即结束会话。
                if let (Some(previous), Some(next)) = (&previous_value, &state)
                    && previous != &next.value
                {
                    self.reconcile_text_view_state(*id, previous, &next.value);
                }
                if state.is_none() {
                    self.nodes.set_text_fold_view(*id, None);
                    self.nodes.set_text_snippet_session(*id, None);
                    self.nodes.set_text_completion_view(*id, None);
                    self.nodes.set_text_hover_view(*id, None);
                }
                self.mark(
                    *id,
                    DirtyMask::TEXT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetTextSelection { id, selection } => {
                self.nodes
                    .text_input_mut(*id)
                    .expect("entity must have runtime component")
                    .selection = *selection;
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
                    let state = self
                        .nodes
                        .text_input_mut(*id)
                        .expect("entity must have runtime component");
                    let replaced = state.replace_selection(text);
                    (replaced, state.value.clone())
                };
                debug_assert!(replaced, "validated selection must remain valid");
                self.record_mut(*id).text = TextContent { value };
                self.mark(
                    *id,
                    DirtyMask::TEXT
                        | DirtyMask::FOCUS_IME
                        | DirtyMask::RENDER
                        | DirtyMask::ACCESSIBILITY,
                );
            }
            UiMutation::SetHighlightRequest { id, request } => {
                self.nodes.set_highlight(*id, request.clone());
                if request.is_none() {
                    self.nodes.set_text_presentation(*id, None);
                }
                self.mark(*id, DirtyMask::TEXT | DirtyMask::RENDER);
            }
            UiMutation::SetTextInputFoldCollapsed { id, folds } => {
                // 规范化：仅保留仍在宿主喂入区间内、且确实可折叠的条目。
                let offered = match self.nodes.visual(*id) {
                    Some(StandardVisual::TextInput { folds: offered, .. }) => Arc::clone(offered),
                    _ => Arc::from([]),
                };
                let mut collapsed: Vec<crate::TextCodeFold> = folds
                    .iter()
                    .copied()
                    .filter(|fold| offered.contains(fold))
                    .collect();
                collapsed.sort_by_key(|fold| (fold.start, fold.end));
                collapsed.dedup();
                let changed = self
                    .nodes
                    .text_fold_view(*id)
                    .map(|entry| entry.collapsed.as_slice() != collapsed.as_slice())
                    .unwrap_or(!collapsed.is_empty());
                if collapsed.is_empty() && offered.is_empty() {
                    self.nodes.set_text_fold_view(*id, None);
                } else {
                    self.nodes.set_text_fold_view(
                        *id,
                        Some(crate::store::TextFoldViewState { offered, collapsed }),
                    );
                }
                if changed {
                    self.mark(*id, DirtyMask::TEXT | DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputSnippet { id, session } => {
                if self.nodes.text_snippet_session(*id) != session.as_ref() {
                    self.nodes.set_text_snippet_session(*id, session.clone());
                }
            }
            UiMutation::SetTextInputCompletions { id, items } => {
                if items.is_empty() {
                    // 空列表关闭弹层：条目移除，零分配待机。
                    if self.nodes.text_completion_view(*id).is_some() {
                        self.nodes.set_text_completion_view(*id, None);
                        self.mark(*id, DirtyMask::RENDER);
                    }
                    return;
                }
                let next = match self.nodes.text_completion_view(*id) {
                    // 相同列表：无操作，键盘选中、滚动与 Esc 关闭态保持。
                    // 组件投影已过滤未变的喂入，这里的内容比较服务于直接
                    // 下发变更的调用方：内容相等即同一会话，不能降为指针
                    // 比较（换 Arc 重喂相同列表会被误判成新会话）。
                    Some(state) if state.items == *items => None,
                    // 不同列表：视为新会话（选中归零、重新打开）。
                    Some(_) | None => Some(crate::store::TextCompletionViewState {
                        items: Arc::clone(items),
                        selected: 0,
                        scroll: 0,
                        dismissed: false,
                    }),
                };
                if let Some(next) = next {
                    self.nodes.set_text_completion_view(*id, Some(next));
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputCompletionView {
                id,
                selected,
                scroll,
            } => {
                let changed = self
                    .nodes
                    .text_completion_view(*id)
                    .is_some_and(|state| state.selected != *selected || state.scroll != *scroll);
                if changed {
                    if let Some(state) = self.nodes.text_completion_view_mut(*id) {
                        state.selected = *selected;
                        state.scroll = *scroll;
                    }
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputCompletionDismissed { id } => {
                let changed = self
                    .nodes
                    .text_completion_view(*id)
                    .is_some_and(|state| !state.dismissed);
                if changed {
                    if let Some(state) = self.nodes.text_completion_view_mut(*id) {
                        state.dismissed = true;
                    }
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputCompletionReopened { id } => {
                let changed = self
                    .nodes
                    .text_completion_view(*id)
                    .is_some_and(|state| state.dismissed);
                if changed {
                    if let Some(state) = self.nodes.text_completion_view_mut(*id) {
                        state.dismissed = false;
                        state.selected = 0;
                        state.scroll = 0;
                    }
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
            UiMutation::SetTextInputHover { id, hover } => {
                let changed = match (hover, self.nodes.text_hover_view(*id)) {
                    (None, None) => false,
                    (None, Some(_)) => true,
                    (Some(doc), Some(state)) => &state.doc != doc || state.scroll != 0,
                    (Some(_), None) => true,
                };
                if !changed {
                    return;
                }
                match hover {
                    Some(doc) => {
                        self.nodes.set_text_hover_view(
                            *id,
                            Some(crate::store::TextHoverViewState {
                                doc: doc.clone(),
                                scroll: 0,
                            }),
                        );
                    }
                    None => self.nodes.set_text_hover_view(*id, None),
                }
                self.mark(*id, DirtyMask::RENDER);
            }
            UiMutation::SetTextInputHoverScroll { id, scroll } => {
                let changed = self
                    .nodes
                    .text_hover_view(*id)
                    .is_some_and(|state| state.scroll != *scroll);
                if changed {
                    if let Some(state) = self.nodes.text_hover_view_mut(*id) {
                        state.scroll = *scroll;
                    }
                    self.mark(*id, DirtyMask::RENDER);
                }
            }
        }
    }
}

impl UiWorld {
    pub(super) fn identity_and_parent(
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
    pub(super) fn validate_simple_mutation(
        &self,
        mutations: &[UiMutation],
    ) -> Result<bool, UiWorldError> {
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
        // Near the depth limit the child's own height decides, and measuring it
        // is exactly the walk this fast path exists to avoid. Hand those rare
        // batches to the planner, which already bounds depth.
        Ok(depth < MAX_TREE_DEPTH)
    }
}

impl UiWorld {
    /// Borrowing variant of [`commit`]: validate-then-apply against a queue
    /// the caller still owns. Validation runs fully before the apply loop, so
    /// a rejected batch never lands partially and the caller may replay it.
    pub fn commit_ref(&mut self, queue: &MutationQueue) -> Result<CommitReport, UiWorldError> {
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
        let mut scanned = 0;
        let mut validated = Ok(());
        if !self.validate_simple_mutation(queue.as_slice())? {
            let mut plan = ValidationPlan::new(self);
            validated = plan.validate(queue.as_slice());
            scanned = plan.scanned;
        }
        self.validation_nodes_scanned = self.validation_nodes_scanned.saturating_add(scanned);
        validated?;
        self.generation = self.generation.wrapping_add(1);
        report.generation = self.generation;
        for mutation in queue.as_slice() {
            self.apply(mutation, &mut report);
        }
        Ok(report)
    }
}

impl UiWorld {
    pub fn commit(&mut self, queue: MutationQueue) -> Result<CommitReport, UiWorldError> {
        self.commit_ref(&queue)
    }
}
