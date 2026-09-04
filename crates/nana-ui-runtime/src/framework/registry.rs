//! AppContext registry operations.

use super::*;

impl AppContext {
    pub fn resolve_component_tag(&self, tag: &str) -> Option<&ComponentTypeId> {
        self.components.resolve_tag(tag)
    }

    /// Resolve an already-normalized tag (see [`normalize_tag`]).
    pub fn resolve_component_tag_normalized(
        &self,
        normalized_tag: &str,
    ) -> Option<&ComponentTypeId> {
        self.components.resolve_normalized(normalized_tag)
    }

    pub fn bind_semantic(
        &self,
        id: StableNodeId,
        spec: &SemanticSpec<'_>,
        mutations: &mut MutationQueue,
    ) -> Result<ComponentBindKind, FrameworkError> {
        self.prepare_semantic_binding(id, spec, mutations)
            .map(|binding| binding.kind())
    }

    /// Stage a registry component once, preserving opted-in interaction state.
    pub fn prepare_semantic_binding(
        &self,
        id: StableNodeId,
        spec: &SemanticSpec<'_>,
        mutations: &mut MutationQueue,
    ) -> Result<crate::PreparedSemanticBinding, FrameworkError> {
        let mut request = ComponentBindRequest {
            id,
            world: &self.world,
            mutations,
            spec,
            previous: self.views.get(&id).map(|view| view.as_ref()),
            retained: None,
            finish: None,
        };
        let kind = self.components.bind(&mut request)?;
        request
            .mutations
            .set_component_type(id, Some(spec.type_id.clone()));
        Ok(crate::PreparedSemanticBinding {
            id,
            type_id: spec.type_id.clone(),
            kind,
            retained: request.retained,
            finish: request.finish,
        })
    }

    /// Install typed state after its UiWorld projection was committed. Does not
    /// project or parse a second time; optional assembly belongs to the type.
    pub fn finish_semantic_binding(
        &mut self,
        binding: crate::PreparedSemanticBinding,
    ) -> Result<(), FrameworkError> {
        if !self.world.contains(binding.id) {
            return Err(FrameworkError::MissingView(binding.id));
        }
        if self.world.component_type(binding.id) != Some(&binding.type_id) {
            return Err(FrameworkError::ViewType(binding.id));
        }
        if let Some(component) = binding.retained {
            self.views.insert(binding.id, component);
            self.sync_component_lifecycle(binding.id)?;
            if let Some(finish) = binding.finish {
                finish(self, binding.id)?;
            }
        }
        Ok(())
    }

    pub fn install(&mut self, extension: &impl UiExtension) -> Result<(), FrameworkError> {
        let name = extension.name().trim().to_owned();
        if name.is_empty() {
            return Err(FrameworkError::InvalidExtension);
        }
        if self.extensions.contains(&name) {
            return Err(FrameworkError::DuplicateExtension(name));
        }
        let mut registrar = ExtensionRegistrar::default();
        extension.install(&mut registrar)?;
        if let Some(id) = registrar
            .actions
            .keys()
            .find(|id| self.actions.contains_key(*id))
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
            .any(|type_id| self.activations.contains_key(type_id))
        {
            return Err(FrameworkError::DuplicateActivation);
        }
        self.components.extend(registrar.components)?;
        self.actions.extend(registrar.actions);
        self.activations.extend(registrar.activations);
        for presenter in registrar.presenters {
            self.world.register_presenter(presenter)?;
        }
        self.extensions.insert(name);
        Ok(())
    }

    pub(super) fn register_builtin_activations(&mut self) {
        self.bind_activation::<Button>(Self::activate_button);
        self.bind_activation::<IconButton>(Self::activate_icon_button);
        self.bind_activation::<ListItem>(Self::activate_list_item);
        self.bind_activation::<SidebarRow>(Self::activate_sidebar_row);
        self.bind_activation::<FileTab>(Self::activate_file_tab);
        self.bind_activation::<BreadcrumbSegment>(Self::activate_breadcrumb_segment);
        self.bind_activation::<SidebarFooterButton>(Self::activate_sidebar_footer_button);
        self.bind_activation::<SidebarSection>(Self::activate_sidebar_section);
        self.bind_activation::<SettingsCollapsibleCard>(Self::activate_settings_collapsible_card);
        self.bind_activation::<Tabs>(Self::activate_tabs);
        self.bind_activation::<ActionMenuItem>(Self::activate_action_menu_item);
        self.bind_activation::<Select>(Self::toggle_select);
        self.bind_activation::<Dropdown>(Self::toggle_dropdown);
        self.bind_activation::<SearchDropdown>(Self::toggle_search_dropdown);
        self.bind_activation::<Popover>(Self::toggle_popover);
        self.bind_activation::<ActionMenu>(Self::toggle_action_menu);
        self.bind_activation::<ContextMenu>(Self::dismiss_context_menu);
        self.bind_activation::<Checkbox>(Self::toggle_checkbox);
        self.bind_activation::<Switch>(Self::toggle_switch);
        self.bind_activation::<Progress>(Self::cancel_progress);
        self.bind_activation::<SegmentedOption>(Self::activate_segmented_option);
    }

    pub(super) fn bind_activation<C: View>(
        &mut self,
        handler: fn(&mut Self, Entity<C>) -> Result<bool, FrameworkError>,
    ) {
        self.activations.insert(
            TypeId::of::<C>(),
            Arc::new(move |context, id| handler(context, Entity::from_stable_id(id))),
        );
    }

    pub fn register_presenter(
        &mut self,
        presenter: Box<dyn TextPresenter>,
    ) -> Result<(), FrameworkError> {
        self.world
            .register_presenter(presenter)
            .map_err(FrameworkError::from)
    }

    pub(super) fn stamp_component_type<C: ComponentView>(
        &mut self,
        id: StableNodeId,
        queue: &mut MutationQueue,
    ) {
        if let Some(entry) = self.components.get_by_rust(TypeId::of::<C>()) {
            queue.set_component_type(id, Some(entry.id.clone()));
        }
        if C::wants_child_reproject() {
            self.child_reproject_views.insert(id, |context, id| {
                context.update_component(Entity::<C>::from_stable_id(id), |_, _| {})
            });
        }
        self.secondary_presses
            .entry(TypeId::of::<C>())
            .or_insert_with(|| {
                Arc::new(|context: &mut AppContext, id, press| {
                    context.update_component(Entity::<C>::from_stable_id(id), |_, cx| {
                        cx.emit(press);
                    })
                })
            });
    }

    pub(super) fn allocate_id(&mut self) -> StableNodeId {
        loop {
            let id = StableNodeId::new(self.next_id).expect("allocator never emits zero");
            self.next_id = self
                .next_id
                .checked_add(1)
                .expect("stable ID space exhausted");
            if !self.world.contains(id) && !self.world.is_retired(id) {
                return id;
            }
        }
    }
}
