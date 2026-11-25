//! AppContext registry operations.

use super::*;

impl AppContext {
    pub fn resolve_component_tag(&self, tag: &str) -> Option<&ComponentTypeId> {
        self.store.components.resolve_tag(tag)
    }

    /// Every component type this context can instantiate.
    #[cfg(test)]
    pub(crate) fn component_type_ids(&self) -> impl Iterator<Item = &ComponentTypeId> {
        self.store.components.type_ids()
    }

    /// Resolve an already-normalized tag (see [`normalize_tag`]).
    pub fn resolve_component_tag_normalized(
        &self,
        normalized_tag: &str,
    ) -> Option<&ComponentTypeId> {
        self.store.components.resolve_normalized(normalized_tag)
    }

    pub fn install(&mut self, extension: &impl UiExtension) -> Result<(), FrameworkError> {
        let name = extension.name().trim().to_owned();
        if name.is_empty() {
            return Err(FrameworkError::InvalidExtension);
        }
        if self.lifecycle.extensions.contains(&name) {
            return Err(FrameworkError::DuplicateExtension(name));
        }
        let mut registrar = ExtensionRegistrar::default();
        extension.install(&mut registrar)?;
        if let Some(id) = registrar
            .actions
            .keys()
            .find(|id| self.lifecycle.actions.contains_key(*id))
        {
            return Err(FrameworkError::DuplicateAction(id.clone()));
        }
        if let Some(presenter) = registrar
            .presenters
            .iter()
            .find(|presenter| self.world.has_presenter(presenter.name()))
        {
            return Err(FrameworkError::DuplicatePresenter(
                presenter.name().to_owned(),
            ));
        }
        if registrar
            .activations
            .keys()
            .any(|type_id| self.store.activations.contains_key(type_id))
            || registrar
                .pointers
                .keys()
                .any(|type_id| self.store.pointer_extensions.contains_key(type_id))
        {
            return Err(FrameworkError::DuplicateActivation);
        }
        self.store.components.extend(registrar.components)?;
        self.lifecycle.actions.extend(registrar.actions);
        self.store.activations.extend(registrar.activations);
        self.store.pointer_extensions.extend(registrar.pointers);
        for presenter in registrar.presenters {
            self.world.register_presenter(presenter)?;
        }
        self.lifecycle.extensions.insert(name);
        Ok(())
    }

    pub fn register_presenter(
        &mut self,
        presenter: Box<dyn TextPresenter>,
    ) -> Result<(), FrameworkError> {
        self.world
            .register_presenter(presenter)
            .map_err(FrameworkError::from)
    }

    /// Install `presenter`, replacing any existing presenter under the same
    /// name and invalidating cached highlights on matching nodes.
    pub fn replace_presenter(
        &mut self,
        presenter: Box<dyn TextPresenter>,
    ) -> Result<Option<Box<dyn TextPresenter>>, FrameworkError> {
        self.world
            .replace_presenter(presenter)
            .map_err(FrameworkError::from)
    }

    /// What a component type brings once, whichever path created its first
    /// node: reprojection and its [`ComponentView::BEHAVIOR`]. Every path
    /// that creates a component goes through this (via
    /// [`Self::stamp_component_type`] or [`Self::install_view`]), or the
    /// type's first node misses its behavior.
    pub(crate) fn register_view_type<C: ComponentView>(&mut self) {
        self.register_view_type_erased(&ViewType::of::<C>());
    }

    /// [`Self::register_view_type`] for a type described by value, so its
    /// map lookups and inserts are compiled once and not per component type.
    #[inline(never)]
    fn register_view_type_erased(&mut self, view: &ViewType) {
        let type_id = view.type_id;
        if self.store.behaviors.contains_key(&type_id) {
            return;
        }
        self.store
            .reprojectors
            .entry(type_id)
            .or_insert(view.update);
        self.store.behaviors.insert(type_id, (view.behavior)());
        if view.metrics_reproject {
            self.store
                .metrics_reproject_types
                .insert(type_id, view.reproject);
        }
        if view.recipe_reproject {
            self.store
                .recipe_reproject_types
                .insert(type_id, (view.reproject, view.recipe_dependency_mask));
        }
        if view.reduced_motion_reproject {
            self.store
                .reduced_motion_reproject_types
                .insert(type_id, view.reproject);
        }
        if view.text_backend_reproject {
            self.store
                .text_backend_reproject_types
                .insert(type_id, view.reproject);
        }
        if let Some(install) = view.capabilities {
            install(&mut self.capabilities);
        }
        if let Some(activation) = view.activation {
            self.store
                .activations
                .entry(type_id)
                .or_insert_with(activation);
        }
    }

    /// Install the view of a node a composite created itself.
    pub(super) fn install_view<C: ComponentView>(&mut self, id: StableNodeId, view: C) {
        self.register_view_type::<C>();
        if TypeId::of::<C>() == TypeId::of::<crate::Stack>() {
            // SAFETY: `TypeId` matched `Stack`.
            let stack = unsafe { &*std::ptr::from_ref(&view).cast::<crate::Stack>() };
            self.store.views.remove(&id);
            self.world.set_elided_stack_flags(id, stack.elided_flags());
        } else {
            self.world.set_elided_stack_flags(id, 0);
            self.store.views.insert(id, Box::new(view));
        }
    }

    pub(super) fn stamp_component_type<C: ComponentView>(
        &mut self,
        id: StableNodeId,
        queue: &mut MutationQueue,
    ) {
        self.stamp_component_type_erased(id, queue, &ViewType::of::<C>());
    }

    #[inline(never)]
    pub(super) fn stamp_component_type_erased(
        &mut self,
        id: StableNodeId,
        queue: &mut MutationQueue,
        view: &ViewType,
    ) {
        if let Some(component_id) = view.component_id {
            queue.set_component_type(id, Some(ComponentTypeId::from_static(component_id)));
        } else if let Some(component_id) = self.store.components.type_id_for(view.type_id) {
            queue.set_component_type(id, Some(component_id.clone()));
        }
        self.register_view_type_erased(view);

        if view.child_reproject {
            self.store.child_reproject_views.insert(id, view.reproject);
        }
        if view.hover_tracking {
            self.component_lifecycle.hover_cards.entry(id).or_default();
        }
        self.store
            .emitters
            .entry(view.type_id)
            .or_insert(view.update);
    }

    pub(super) fn allocate_id(&mut self) -> StableNodeId {
        loop {
            let id = StableNodeId::new(self.store.next_id).expect("allocator never emits zero");
            self.store.next_id = self
                .store
                .next_id
                .checked_add(1)
                .expect("stable ID space exhausted");
            if !self.world.contains(id) && !self.world.is_retired(id) {
                return id;
            }
        }
    }
}

/// What registering and stamping a component type read from it, as plain
/// values: the generic entry points only fill this in.
pub(super) struct ViewType {
    type_id: TypeId,
    component_id: Option<&'static str>,
    update: UpdateErasedFn,
    reproject: ChildReprojectFn,
    behavior: fn() -> hooks::ErasedBehavior,
    capabilities: Option<fn(&mut hooks::Capabilities)>,
    activation: Option<fn() -> ActivationFn>,
    child_reproject: bool,
    metrics_reproject: bool,
    recipe_reproject: bool,
    recipe_dependency_mask: nana_ui_core::ThemeDependencyMask,
    reduced_motion_reproject: bool,
    text_backend_reproject: bool,
    hover_tracking: bool,
}

impl ViewType {
    pub(super) fn of<C: ComponentView>() -> Self {
        Self {
            type_id: TypeId::of::<C>(),
            component_id: C::TYPE_ID,
            update: update_erased::<C>,
            reproject: reproject_typed::<C>,
            behavior: hooks::ErasedBehavior::of::<C>,
            capabilities: C::BEHAVIOR.capabilities,
            activation: C::BEHAVIOR
                .activation
                .map(|_| activation_of::<C> as fn() -> ActivationFn),
            child_reproject: C::wants_child_reproject(),
            metrics_reproject: C::wants_metrics_reproject(),
            recipe_reproject: C::wants_recipe_reproject(),
            recipe_dependency_mask: C::recipe_dependency_mask(),
            reduced_motion_reproject: C::wants_reduced_motion_reproject(),
            text_backend_reproject: C::wants_text_backend_reproject(),
            hover_tracking: C::wants_hover_tracking(),
        }
    }
}

/// The activation a type's [`ComponentView::BEHAVIOR`] names, reached by
/// node id.
fn activation_of<C: ComponentView>() -> ActivationFn {
    let activate = C::BEHAVIOR.activation;
    Arc::new(move |context, id| match activate {
        Some(activate) => activate(context, Entity::from_stable_id(id)),
        None => Ok(false),
    })
}
