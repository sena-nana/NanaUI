//! AppContext choice operations.

use super::*;

impl AppContext {
    pub(crate) fn set_select_opened(
        &mut self,
        entity: Entity<Select>,
        opened: bool,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, Select::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |select, _| {
            if select.opened == opened {
                false
            } else if opened {
                select.toggle_open()
            } else {
                select.close();
                true
            }
        })
    }

    pub(crate) fn set_dropdown_opened(
        &mut self,
        entity: Entity<Dropdown>,
        opened: bool,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, Dropdown::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |dropdown, cx| {
            if dropdown.opened == opened {
                return false;
            }
            if opened {
                dropdown.toggle_open().is_some_and(|event| {
                    cx.emit(event);
                    true
                })
            } else {
                dropdown.close().is_some_and(|event| {
                    cx.emit(event);
                    true
                })
            }
        })
    }

    pub(crate) fn set_search_dropdown_opened(
        &mut self,
        entity: Entity<SearchDropdown>,
        opened: bool,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, SearchDropdown::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |dropdown, cx| {
            if dropdown.opened == opened {
                return false;
            }
            let event = if opened {
                dropdown.toggle_open()
            } else {
                dropdown.close()
            };
            event.is_some_and(|event| {
                cx.emit(event);
                true
            })
        })
    }

    pub fn toggle_select(&mut self, entity: Entity<Select>) -> Result<bool, FrameworkError> {
        if self.read(entity, Select::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |select, _| select.toggle_open())
    }

    pub fn activate_select_at(
        &mut self,
        entity: Entity<Select>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, Select::inactive)? {
            return Ok(false);
        }
        let opened = self.read(entity, |select| select.opened)?;
        if opened {
            if let Some(crate::ComponentGeometry::Select {
                menu: Some(menu), ..
            }) = self.world.component_geometry(entity.id)
                && let Some(index) = crate::select::select_option_at(&menu, x, y)
            {
                return self.update_component(entity, |select, cx| {
                    if let Some(changed) = select.select_index(index) {
                        cx.emit(changed);
                        true
                    } else {
                        false
                    }
                });
            }
            let Some(field) = self.world.component_layout_box(entity.id) else {
                return Ok(false);
            };
            if field.contains(x, y) {
                return self.toggle_select(entity);
            }
            return self.update_component(entity, |select, _| {
                select.close();
                true
            });
        }
        self.toggle_select(entity)
    }

    pub fn adjust_focused_select(
        &mut self,
        document: DocumentId,
        delta: i32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<Select>())
        {
            return Ok(false);
        }
        let entity = Entity::<Select>::from_stable_id(target);
        if self.read(entity, Select::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |select, _| {
            if !select.opened {
                select.toggle_open()
            } else {
                select.highlight_delta(delta)
            }
        })
    }

    pub fn adjust_focused_dropdown(
        &mut self,
        document: DocumentId,
        delta: i32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<Dropdown>())
        {
            return Ok(false);
        }
        let entity = Entity::<Dropdown>::from_stable_id(target);
        if self.read(entity, Dropdown::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |dropdown, cx| {
            if !dropdown.opened {
                if let Some(event) = dropdown.toggle_open() {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            } else {
                dropdown.highlight_delta(delta)
            }
        })
    }

    pub fn commit_focused_dropdown(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<Dropdown>())
        {
            return Ok(false);
        }
        self.update_component(
            Entity::<Dropdown>::from_stable_id(target),
            |dropdown, cx| {
                if let Some(event) = dropdown.commit_highlighted() {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            },
        )
    }

    pub fn commit_focused_select(&mut self, document: DocumentId) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<Select>())
        {
            return Ok(false);
        }
        self.update_component(Entity::<Select>::from_stable_id(target), |select, cx| {
            if let Some(changed) = select.commit_highlighted() {
                cx.emit(changed);
                true
            } else {
                false
            }
        })
    }

    pub fn toggle_dropdown(&mut self, entity: Entity<Dropdown>) -> Result<bool, FrameworkError> {
        if self.read(entity, Dropdown::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |dropdown, cx| {
            if let Some(event) = dropdown.toggle_open() {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn toggle_search_dropdown(
        &mut self,
        entity: Entity<SearchDropdown>,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, SearchDropdown::inactive)? {
            return Ok(false);
        }
        if self.read(entity, |dropdown| dropdown.opened)? {
            return Ok(false);
        }
        self.update_component(entity, |dropdown, cx| {
            if let Some(event) = dropdown.toggle_open() {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn activate_search_dropdown_at(
        &mut self,
        entity: Entity<SearchDropdown>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, SearchDropdown::inactive)? {
            return Ok(false);
        }
        let menu = match self.world.component_geometry(entity.id) {
            Some(crate::ComponentGeometry::Select { menu, .. }) => menu,
            _ => None,
        };
        let Some(field) = self.world.component_layout_box(entity.id) else {
            return Ok(false);
        };
        self.update_component(entity, |dropdown, cx| {
            if let Some(event) = crate::search_dropdown::activate_search_dropdown_at(
                dropdown,
                menu.as_ref(),
                field,
                x,
                y,
            ) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn adjust_focused_search_dropdown(
        &mut self,
        document: DocumentId,
        delta: i32,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<SearchDropdown>())
        {
            return Ok(false);
        }
        let entity = Entity::<SearchDropdown>::from_stable_id(target);
        if self.read(entity, SearchDropdown::inactive)? {
            return Ok(false);
        }
        self.update_component(entity, |dropdown, cx| {
            if !dropdown.opened {
                if let Some(event) = dropdown.toggle_open() {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            } else {
                dropdown.highlight_delta(delta)
            }
        })
    }

    pub fn commit_focused_search_dropdown(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<SearchDropdown>())
        {
            return Ok(false);
        }
        self.update_component(
            Entity::<SearchDropdown>::from_stable_id(target),
            |dropdown, cx| {
                if let Some(event) = dropdown.commit_highlighted() {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            },
        )
    }

    pub fn activate_command_palette_at(
        &mut self,
        entity: Entity<CommandPalette>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(crate::ComponentGeometry::CommandPalette {
            surface,
            input,
            rows,
            ..
        }) = self.world.component_geometry(entity.id)
        else {
            return Ok(false);
        };
        self.update_component(entity, |palette, cx| {
            if let Some(event) = crate::command_palette::activate_command_palette_at(
                palette,
                surface,
                input.bounds,
                &rows,
                x,
                y,
            ) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn navigate_focused_command_palette(
        &mut self,
        document: DocumentId,
        navigation: ActionPickerNavigation,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<CommandPalette>())
        {
            return Ok(false);
        }
        self.update_component(
            Entity::<CommandPalette>::from_stable_id(target),
            |palette, cx| {
                if let Some(event) = palette.navigate(navigation) {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            },
        )
    }

    pub fn activate_dropdown_at(
        &mut self,
        entity: Entity<Dropdown>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if self.read(entity, Dropdown::inactive)? {
            return Ok(false);
        }
        let menu = match self.world.component_geometry(entity.id) {
            Some(crate::ComponentGeometry::Select { menu, .. }) => menu,
            _ => None,
        };
        let Some(field) = self.world.component_layout_box(entity.id) else {
            return Ok(false);
        };
        self.update_component(entity, |dropdown, cx| {
            if let Some(event) =
                crate::dropdown::activate_dropdown_at(dropdown, menu.as_ref(), field, x, y)
            {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn activate_tree_at(
        &mut self,
        entity: Entity<TreeView>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(crate::ComponentGeometry::TreeView { rows }) =
            self.world.component_geometry(entity.id)
        else {
            return Ok(false);
        };
        if let Some(index) = crate::tree_view::tree_disclosure_at(&rows, x, y) {
            let id = Arc::clone(&rows[index].id);
            return self.update_component(entity, |tree, cx| {
                let event = crate::TreeViewEvent::Toggle(id);
                if tree.apply_event(event.clone()) {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            });
        }
        if let Some(index) = crate::tree_view::tree_row_at(&rows, x, y) {
            if rows[index].disabled {
                return Ok(false);
            }
            let id = Arc::clone(&rows[index].id);
            return self.update_component(entity, |tree, cx| {
                let event = crate::TreeViewEvent::Select(id);
                if tree.apply_event(event.clone()) {
                    cx.emit(event);
                    true
                } else {
                    false
                }
            });
        }
        Ok(false)
    }

    pub fn navigate_focused_tree(
        &mut self,
        document: DocumentId,
        navigation: crate::TreeNavigation,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        if !self
            .views
            .get(&target)
            .is_some_and(|view| view.is::<TreeView>())
        {
            return Ok(false);
        }
        self.update_component(Entity::<TreeView>::from_stable_id(target), |tree, cx| {
            if let Some(event) = tree.navigate(navigation) {
                cx.emit(event);
                true
            } else {
                false
            }
        })
    }

    pub fn activate_node_at(
        &mut self,
        id: StableNodeId,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if let Some(activate) = self.behavior(id).and_then(|b| b.activate_at) {
            return activate(self, id, x, y);
        }
        self.activate_node(id)
    }

    /// Route a secondary (right) press to the nearest `SecondaryPress` handler
    /// at or above the hit node.
    ///
    /// Returns the node that handled it. The framework opens no menu and picks
    /// no default items; an application with no handler gets `None`.
    pub fn secondary_press_at(
        &mut self,
        document: DocumentId,
        x: f32,
        y: f32,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(FrameworkError::InvalidInput);
        }
        let Some(target) = self.world.hit_test(document, x, y) else {
            return Ok(None);
        };
        self.route_secondary_press(SecondaryPress {
            target,
            x,
            y,
            keyboard: false,
            focus: None,
        })
    }

    /// The keyboard's context-menu request: [`SecondaryPress`] with
    /// `keyboard` set, raised on the focused node and routed like a pointer
    /// press on it, with the point at the centre of its box.
    ///
    /// The input path calls it for the `ContextMenu` key, Shift+F10 and the
    /// arrows on a popup trigger once the focused control has passed on the
    /// key; a host with its own gesture for the same request calls it too.
    /// The press names no end of the menu to focus (`focus: None`), as for
    /// the `ContextMenu` key. Returns the node that handled it, `None` when
    /// nothing is focused or no handler is registered.
    pub fn secondary_press_focused(
        &mut self,
        document: DocumentId,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        self.secondary_press_focused_toward(document, None)
    }

    /// [`Self::secondary_press_focused`] with the end of the menu the key
    /// asked to focus.
    pub(super) fn secondary_press_focused_toward(
        &mut self,
        document: DocumentId,
        focus: Option<crate::RovingEdge>,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        let Some(target) = self.world.focused(document) else {
            return Ok(None);
        };
        let Some(bounds) = self
            .world
            .viewport_layout_box(target)
            .or_else(|| self.world.component_layout_box(target))
        else {
            return Ok(None);
        };
        self.route_secondary_press(SecondaryPress {
            target,
            x: bounds.x + bounds.width / 2.0,
            y: bounds.y + bounds.height / 2.0,
            keyboard: true,
            focus,
        })
    }

    /// Deliver `press` to the nearest `SecondaryPress` handler at or above
    /// its target.
    fn route_secondary_press(
        &mut self,
        press: SecondaryPress,
    ) -> Result<Option<StableNodeId>, FrameworkError> {
        let mut current = Some(press.target);
        while let Some(id) = current {
            if self
                .event_handlers
                .contains_key(&(id, TypeId::of::<SecondaryPress>()))
                && let Some(emit) = self
                    .views
                    .get(&id)
                    .and_then(|view| self.secondary_presses.get(&view.as_ref().type_id()))
                    .cloned()
            {
                emit(self, id, press)?;
                return Ok(Some(id));
            }
            // Reorder-list row bodies are hit-tested at the list shell (row
            // surfaces are pointer-transparent), so a secondary press there
            // never reaches a row handler; resolve the row on the list itself.
            #[cfg(feature = "controls")]
            if self.emit_reorder_row_secondary(id, press.x, press.y)? {
                return Ok(Some(id));
            }
            current = self.world.parent_id(id);
        }
        Ok(None)
    }

    /// Close only the focused field's detached options, leaving its value and
    /// search draft intact. Returns false once the options are already closed.
    pub fn dismiss_focused_field_options(
        &mut self,
        document: DocumentId,
    ) -> Result<bool, FrameworkError> {
        let Some(target) = self.world().focused(document) else {
            return Ok(false);
        };
        match self.behavior(target).and_then(|b| b.close_options) {
            Some(close) => close(self, target),
            None => Ok(false),
        }
    }

    pub fn dismiss_detached_menus(
        &mut self,
        keep: Option<StableNodeId>,
    ) -> Result<(), FrameworkError> {
        let ids = self.views.keys().copied().collect::<Vec<_>>();
        for id in ids {
            if Some(id) == keep {
                continue;
            }
            if let Some(close) = self.behavior(id).and_then(|b| b.close_options) {
                close(self, id)?;
            }
        }
        Ok(())
    }

    /// Closes every open popover whose policy `allows` dismissal. A popover
    /// keeps its state when `inside` sits in its own subtree, so pressing one
    /// of its items still activates that item.
    pub(super) fn close_open_popovers(
        &mut self,
        inside: Option<StableNodeId>,
        allows: fn(&Popover) -> bool,
    ) -> Result<bool, FrameworkError> {
        let ids = self.views.keys().copied().collect::<Vec<_>>();
        let mut dismissed = false;
        for id in ids {
            if inside.is_some_and(|node| self.world.is_descendant_or_self(node, id)) {
                continue;
            }
            if let Some(entity) = self.view_entity::<Popover>(id) {
                if self.read(entity, |popover| popover.open && allows(popover))? {
                    self.toggle_popover(entity)?;
                    dismissed = true;
                }
            } else if let Some(entity) = self.view_entity::<ActionMenu>(id)
                && self.read(entity, |menu| menu.popover.open && allows(&menu.popover))?
            {
                self.toggle_action_menu(entity)?;
                dismissed = true;
            }
        }
        Ok(dismissed)
    }

    /// Light dismiss for toggle-driven popovers, mirroring the outside-press
    /// rule the overlay host applies to dialogs and menus. `inside` is the node
    /// under the pointer. Returns whether anything closed; the caller consumes
    /// the press in that case so it cannot also drive the control underneath,
    /// nor re-open the popover through its own trigger.
    pub fn dismiss_popovers_outside(
        &mut self,
        inside: Option<StableNodeId>,
    ) -> Result<bool, FrameworkError> {
        self.close_open_popovers(inside, |popover| popover.close_on_outside)
    }

    /// Escape closes every open popover and hover card that allows it.
    pub fn dismiss_popovers_on_escape(&mut self) -> Result<bool, FrameworkError> {
        let mut dismissed = self.close_open_popovers(None, |popover| popover.close_on_escape)?;
        let targets = self
            .component_lifecycle
            .hover_cards
            .iter()
            .filter(|(_, lifecycle)| lifecycle.open)
            .map(|(&target, _)| target)
            .collect::<Vec<_>>();
        for target in targets {
            let allows_escape = self
                .view_entity::<crate::HoverCard>(target)
                .and_then(|entity| self.read(entity, |card| card.close_on_escape).ok())
                .unwrap_or(false);
            if allows_escape && self.close_hover_card(target)? {
                dismissed = true;
            }
        }
        Ok(dismissed)
    }

    pub fn toggle_popover(&mut self, entity: Entity<Popover>) -> Result<bool, FrameworkError> {
        self.update_component(entity, |popover, cx| {
            popover.open = !popover.open;
            cx.emit(PopoverToggled { open: popover.open });
            if !popover.open {
                cx.emit(PopoverClosed);
            }
            true
        })
    }

    /// A press released on a popover: its trigger toggles it; the open
    /// surface around its items, which takes the pointer above the page so
    /// the press does not fall through, does nothing.
    pub(crate) fn activate_popover_at(
        &mut self,
        entity: Entity<Popover>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if self.world.hanging_surface_contains(entity.id, x, y) {
            return Ok(false);
        }
        self.toggle_popover(entity)
    }

    /// [`Self::activate_popover_at`] for an [`ActionMenu`].
    pub(crate) fn activate_action_menu_at(
        &mut self,
        entity: Entity<ActionMenu>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if self.world.hanging_surface_contains(entity.id, x, y) {
            return Ok(false);
        }
        self.toggle_action_menu(entity)
    }

    /// A press released on a hover card activates its trigger, not the open
    /// card's surface around its content.
    pub(crate) fn activate_hover_card_at(
        &mut self,
        entity: Entity<HoverCard>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        if self.world.hanging_surface_contains(entity.id, x, y) {
            return Ok(false);
        }
        self.activate_hover_card(entity)
    }

    /// A popover that closed around the focus (on one of its items) gives it
    /// back to its trigger, which stays on screen, instead of leaving it on a
    /// hidden item. Runs after each write, so it holds however the popover
    /// closed: its trigger, Escape, a press elsewhere, or the application.
    pub(crate) fn return_focus_to_popover_trigger(
        &mut self,
        id: StableNodeId,
        open: bool,
    ) -> Result<(), FrameworkError> {
        if open {
            return Ok(());
        }
        let Some(document) = self.world.document_of(id) else {
            return Ok(());
        };
        let Some(focused) = self.world.focused(document) else {
            return Ok(());
        };
        if focused != id
            && self.world.is_descendant_or_self(focused, id)
            && self
                .world
                .interaction(id)
                .is_some_and(|interaction| interaction.focusable)
        {
            self.focus_node(document, id)?;
        }
        Ok(())
    }

    pub(crate) fn settle_popover(&mut self, entity: Entity<Popover>) -> Result<(), FrameworkError> {
        let open = self.read(entity, |popover| popover.open)?;
        self.return_focus_to_popover_trigger(entity.stable_id(), open)
    }

    pub(crate) fn settle_action_menu(
        &mut self,
        entity: Entity<ActionMenu>,
    ) -> Result<(), FrameworkError> {
        let open = self.read(entity, |menu| menu.popover.open)?;
        self.return_focus_to_popover_trigger(entity.stable_id(), open)
    }

    pub fn toggle_action_menu(
        &mut self,
        entity: Entity<ActionMenu>,
    ) -> Result<bool, FrameworkError> {
        self.update_component(entity, |menu, cx| {
            menu.popover.open = !menu.popover.open;
            cx.emit(PopoverToggled {
                open: menu.popover.open,
            });
            if !menu.popover.open {
                cx.emit(PopoverClosed);
            }
            true
        })
    }

    pub fn activate_context_menu_at(
        &mut self,
        entity: Entity<ContextMenu>,
        x: f32,
        y: f32,
    ) -> Result<bool, FrameworkError> {
        let Some(geometry) = self.world.component_geometry(entity.id) else {
            return Ok(false);
        };
        let Some(index) = crate::menus::context_menu_option_at(&geometry, x, y) else {
            return Ok(false);
        };
        self.update_component(entity, |menu, cx| {
            let before = (menu.open, menu.active_path.clone(), menu.highlighted);
            if let Some(event) = menu.select_index(index) {
                cx.emit(event);
            }
            before != (menu.open, menu.active_path.clone(), menu.highlighted)
        })
    }

    /// Activate a painted context-menu row by its visible index. Accessibility
    /// exposes rows as virtual MenuItem nodes because the surface keeps one
    /// retained entity for pointer hit-testing and nested navigation.
    pub fn activate_context_menu_index(
        &mut self,
        entity: Entity<ContextMenu>,
        index: usize,
    ) -> Result<bool, FrameworkError> {
        self.update_component(entity, |menu, cx| {
            let before = (menu.open, menu.active_path.clone(), menu.highlighted);
            if let Some(event) = menu.select_index(index) {
                cx.emit(event);
            }
            before != (menu.open, menu.active_path.clone(), menu.highlighted)
        })
    }

    /// Move the virtual accessibility focus/highlight to a visible menu row.
    pub fn focus_context_menu_index(
        &mut self,
        entity: Entity<ContextMenu>,
        index: usize,
    ) -> Result<bool, FrameworkError> {
        self.update_component(entity, |menu, _cx| {
            if !menu.open
                || menu
                    .visible_items()
                    .get(index)
                    .is_none_or(|item| item.disabled)
            {
                return false;
            }
            if menu.highlighted == Some(index) {
                return false;
            }
            menu.highlighted = Some(index);
            true
        })
    }

    pub fn cancel_progress(&mut self, entity: Entity<Progress>) -> Result<bool, FrameworkError> {
        self.update_component(entity, |progress, cx| {
            if !progress.cancellable {
                return false;
            }
            cx.emit(ProgressCancelled);
            true
        })
    }

    pub fn dismiss_context_menu(
        &mut self,
        entity: Entity<ContextMenu>,
    ) -> Result<bool, FrameworkError> {
        self.update_component(entity, |menu, cx| {
            if !menu.open {
                return false;
            }
            menu.dismiss();
            cx.emit(ContextMenuEvent::Dismiss);
            true
        })
    }

    /// Keys for an open context menu. Its rows are virtual, so they are not
    /// focus stops and arrow keys would otherwise move the control underneath.
    /// The key is consumed even when the highlight does not change.
    pub(crate) fn navigate_open_context_menu(
        &mut self,
        document: DocumentId,
        key: &str,
        repeat: bool,
    ) -> Result<bool, FrameworkError> {
        let Some(overlay) = self.active_runtime_overlay(document) else {
            return Ok(false);
        };
        if overlay.kind != RuntimeOverlayKind::Menu {
            return Ok(false);
        }
        let Some(entity) = self.view_entity::<ContextMenu>(overlay.root) else {
            return Ok(false);
        };
        let (open, enabled, highlighted, searchable) = self.read(entity, |menu| {
            (
                menu.open,
                menu.visible_items()
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| !item.disabled)
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>(),
                menu.highlighted,
                menu.searchable,
            )
        })?;
        if !open {
            return Ok(false);
        }
        if let Some(movement) = ContextMenuMove::from_key(key) {
            if let Some(index) = context_menu_enabled_index(&enabled, highlighted, movement) {
                let _ = self.focus_context_menu_index(entity, index)?;
            }
            return Ok(true);
        }
        let space = matches!(key, " " | "Space");
        if key == "Enter" || space {
            // A searchable menu that already holds focus types spaces into its
            // query. Every other Space or Enter chooses a row.
            if space && searchable && self.world.focused(document) == Some(overlay.root) {
                return Ok(false);
            }
            if !repeat {
                let index = highlighted
                    .filter(|index| enabled.contains(index))
                    .or_else(|| enabled.first().copied());
                if let Some(index) = index {
                    let _ = self.activate_context_menu_index(entity, index)?;
                }
            }
            return Ok(true);
        }
        Ok(false)
    }
}

enum ContextMenuMove {
    Next,
    Previous,
    First,
    Last,
}

impl ContextMenuMove {
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "ArrowDown" => Some(Self::Next),
            "ArrowUp" => Some(Self::Previous),
            "Home" => Some(Self::First),
            "End" => Some(Self::Last),
            _ => None,
        }
    }
}

/// `None` and a highlight on a disabled row start at the first enabled row
/// for Down and the last for Up. Movement wraps inside the enabled rows.
fn context_menu_enabled_index(
    enabled: &[usize],
    highlighted: Option<usize>,
    movement: ContextMenuMove,
) -> Option<usize> {
    let len = enabled.len();
    if len == 0 {
        return None;
    }
    let position = highlighted.and_then(|index| enabled.iter().position(|item| *item == index));
    let next = match movement {
        ContextMenuMove::First => 0,
        ContextMenuMove::Last => len - 1,
        ContextMenuMove::Next => position.map(|index| (index + 1) % len).unwrap_or(0),
        ContextMenuMove::Previous => position
            .map(|index| (index + len - 1) % len)
            .unwrap_or(len - 1),
    };
    Some(enabled[next])
}

/// The commands of an [`ActionMenu`]: its items wherever its children put
/// them, fixed ones and the rows of keyed lists or conditional blocks alike.
impl AppContext {
    /// The nearest action menu at or above `node`, when it is open.
    pub(crate) fn open_action_menu_of(&self, node: StableNodeId) -> Option<Entity<ActionMenu>> {
        let mut current = Some(node);
        while let Some(id) = current {
            if let Some(menu) = self.view_entity::<ActionMenu>(id) {
                return self
                    .views
                    .get(&id)
                    .and_then(|view| view.downcast_ref::<ActionMenu>())
                    .is_some_and(|menu| menu.popover.open)
                    .then_some(menu);
            }
            current = self.world.parent_id(id);
        }
        None
    }

    /// `menu`'s items in order, looking through the containers its children
    /// build (the column of an `each`, the branch of a `when`) but not into
    /// another menu, popover, or the content that draws its trigger.
    pub fn action_menu_items(&self, menu: Entity<ActionMenu>) -> Vec<StableNodeId> {
        let trigger = self
            .views
            .get(&menu.stable_id())
            .and_then(|view| view.downcast_ref::<ActionMenu>())
            .and_then(|menu| menu.popover.trigger_content);
        let mut items = Vec::new();
        let mut stack: Vec<StableNodeId> = self
            .world
            .node(menu.stable_id())
            .map(|node| node.children.iter().rev().copied().collect())
            .unwrap_or_default();
        while let Some(id) = stack.pop() {
            if Some(id) == trigger {
                continue;
            }
            if self.view_is::<ActionMenuItem>(id) {
                items.push(id);
            } else if !self.view_is::<ActionMenu>(id)
                && !self.view_is::<Popover>(id)
                && let Some(node) = self.world.node(id)
            {
                stack.extend(node.children.iter().rev().copied());
            }
        }
        items
    }

    /// The items of `menu` a keyboard can land on: enabled, and shown
    /// (neither they nor a container between them and the menu hidden).
    pub(crate) fn reachable_action_menu_items(
        &self,
        menu: Entity<ActionMenu>,
    ) -> Vec<StableNodeId> {
        self.action_menu_items(menu)
            .into_iter()
            .filter(|&item| {
                let enabled = self
                    .views
                    .get(&item)
                    .and_then(|view| view.downcast_ref::<ActionMenuItem>())
                    .is_some_and(|item| !item.disabled);
                let mut shown = true;
                let mut current = Some(item);
                while let Some(id) = current
                    && id != menu.stable_id()
                {
                    if self
                        .world
                        .node_style(id)
                        .is_some_and(|style| style.layout.hidden)
                    {
                        shown = false;
                        break;
                    }
                    current = self.world.parent_id(id);
                }
                enabled && shown
            })
            .collect()
    }

    /// Arrow keys, Home and End inside an open action menu (or on its
    /// trigger while it is open) move focus between its reachable items,
    /// wrapping at the ends. Returns whether the key was the menu's.
    pub(crate) fn navigate_open_action_menu(
        &mut self,
        document: DocumentId,
        key: &str,
    ) -> Result<bool, FrameworkError> {
        if !matches!(key, "ArrowDown" | "ArrowUp" | "Home" | "End") {
            return Ok(false);
        }
        let Some(focused) = self.world.focused(document) else {
            return Ok(false);
        };
        let Some(menu) = self.open_action_menu_of(focused) else {
            return Ok(false);
        };
        let items = self.reachable_action_menu_items(menu);
        if items.is_empty() {
            return Ok(true);
        }
        let current = items.iter().position(|item| *item == focused);
        let last = items.len() - 1;
        let next = match (key, current) {
            ("Home", _) | ("ArrowDown", None) => 0,
            ("End", _) | ("ArrowUp", None) => last,
            ("ArrowDown", Some(index)) => (index + 1) % items.len(),
            ("ArrowUp", Some(index)) => index.checked_sub(1).unwrap_or(last),
            _ => return Ok(false),
        };
        self.focus_node(document, items[next])?;
        Ok(true)
    }

    /// Before a commit that removes the focused node: where focus is in an
    /// open action menu, which survives the commit, so the item that takes
    /// its place can take focus ([`Self::hand_over_menu_focus`]).
    pub(crate) fn menu_focus_before_removal(
        &self,
        focused: &HashMap<DocumentId, StableNodeId>,
        despawned: &HashSet<StableNodeId>,
    ) -> Vec<(DocumentId, Entity<ActionMenu>, usize)> {
        focused
            .iter()
            .filter(|(_, node)| despawned.contains(node))
            .filter_map(|(&document, &node)| {
                let menu = self.open_action_menu_of(node)?;
                if despawned.contains(&menu.stable_id()) {
                    return None;
                }
                let index = self
                    .reachable_action_menu_items(menu)
                    .iter()
                    .position(|item| *item == node)?;
                Some((document, menu, index))
            })
            .collect()
    }

    /// After that commit: the focused item went with it, so focus the item
    /// now at its place (or the last one; the trigger when none is left),
    /// rather than leaving the open menu with no focus at all.
    pub(crate) fn hand_over_menu_focus(
        &mut self,
        handoffs: Vec<(DocumentId, Entity<ActionMenu>, usize)>,
    ) -> Result<(), FrameworkError> {
        for (document, menu, index) in handoffs {
            if self.world.focused(document).is_some()
                || !self.world.contains(menu.stable_id())
                || self.open_action_menu_of(menu.stable_id()) != Some(menu)
            {
                continue;
            }
            let items = self.reachable_action_menu_items(menu);
            let target = items
                .get(index)
                .or(items.last())
                .copied()
                .unwrap_or(menu.stable_id());
            if self.focus_node(document, target).is_err() {
                self.clear_focus(document)?;
            }
        }
        Ok(())
    }
}
