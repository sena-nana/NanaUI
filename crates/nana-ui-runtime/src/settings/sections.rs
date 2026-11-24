use std::sync::Arc;

use nana_ui_core::{
    AppearanceEvent, AppearanceSettings, BackdropTarget, ButtonKind, ControlSize, Icon,
    SemanticColorRole, ThemeAppearance, ThemeChoice, ThemeId, WindowMaterialMode,
};

use super::group::SettingsCard;
use super::row::{mount_settings_row, styled_text, sync_text};
use super::{document_of, reconcile_children};
use crate::view_components::{Button, RangeField, RangeInput, Switch, ToggleChanged};
use crate::{
    AccessibilityRole, AccessibilityState, Activate, AppContext, ComponentView, Entity,
    FrameworkError, MutationQueue, NodeKind, SegmentedControl, SegmentedOption,
    SegmentedSelectionRequested, StableNodeId, UiWorld,
};

/// Host-owned appearance snapshot. Events stay [`AppearanceEvent`]; values stay outside NanaUI.
#[derive(Debug, Clone, PartialEq)]
pub struct AppearanceSection {
    pub theme_id: ThemeId,
    pub theme_options: Vec<ThemeChoice>,
    pub appearance: AppearanceSettings,
    pub material_status: Option<Arc<str>>,
    pub platform_hint: Option<Arc<str>>,
    /// Native materials the host is willing to offer besides 实色 / 透明.
    pub available_materials: Vec<WindowMaterialMode>,
    pub assembly: Option<AppearanceSectionAssembly>,
}

/// Retained children created by [`AppContext::assemble_appearance_section`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppearanceSectionAssembly {
    pub theme_row: Option<StableNodeId>,
    pub theme_control: Option<StableNodeId>,
    pub theme_dark: Option<StableNodeId>,
    pub theme_light: Option<StableNodeId>,
    pub theme_options: Vec<StableNodeId>,
    pub material_row: Option<StableNodeId>,
    pub material_control: Option<StableNodeId>,
    pub material_solid: Option<StableNodeId>,
    pub material_translucent: Option<StableNodeId>,
    pub material_vibrancy: Option<StableNodeId>,
    pub material_mica: Option<StableNodeId>,
    pub material_acrylic: Option<StableNodeId>,
    pub material_status_row: Option<StableNodeId>,
    pub material_status_value: Option<StableNodeId>,
    pub target_row: Option<StableNodeId>,
    pub target_control: Option<StableNodeId>,
    pub target_sidebar: Option<StableNodeId>,
    pub target_main: Option<StableNodeId>,
    pub titlebar_row: Option<StableNodeId>,
    pub titlebar_switch: Option<StableNodeId>,
    pub opacity_row: Option<StableNodeId>,
    pub opacity_range: Option<StableNodeId>,
    pub opacity_text: Option<StableNodeId>,
    pub workspace_row: Option<StableNodeId>,
    pub workspace_switch: Option<StableNodeId>,
    pub radius_row: Option<StableNodeId>,
    pub radius_range: Option<StableNodeId>,
    pub reset_row: Option<StableNodeId>,
    pub reset_button: Option<StableNodeId>,
}

impl AppearanceSection {
    pub fn new(theme: ThemeAppearance, appearance: AppearanceSettings) -> Self {
        let theme_id = match theme {
            ThemeAppearance::Light => ThemeId::new("nana.light"),
            ThemeAppearance::Dark => ThemeId::new("nana.dark"),
            ThemeAppearance::Custom => ThemeId::new("nana.light"),
        };
        Self {
            theme_id,
            theme_options: Vec::new(),
            appearance,
            material_status: None,
            platform_hint: None,
            available_materials: Vec::new(),
            assembly: None,
        }
    }

    pub fn from_theme_id(theme_id: ThemeId, appearance: AppearanceSettings) -> Self {
        Self {
            theme_id,
            theme_options: Vec::new(),
            appearance,
            material_status: None,
            platform_hint: None,
            available_materials: Vec::new(),
            assembly: None,
        }
    }

    pub fn with_theme_options(mut self, selected: ThemeId, options: Vec<ThemeChoice>) -> Self {
        self.theme_id = selected;
        self.theme_options = options;
        self
    }

    pub fn material_status(mut self, status: impl Into<Arc<str>>) -> Self {
        self.material_status = Some(status.into());
        self
    }

    pub fn platform_hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        self.platform_hint = Some(hint.into());
        self
    }

    pub fn available_materials(mut self, modes: impl Into<Vec<WindowMaterialMode>>) -> Self {
        self.available_materials = modes.into();
        self
    }
}

impl ComponentView for AppearanceSection {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(AppContext::assemble_appearance_section),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "appearance-section".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        SettingsCard::new("").project(id, world, mutations);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutMetadata {
    pub product_title: Arc<str>,
    pub version: Arc<str>,
    pub description: Option<Arc<str>>,
}

impl AboutMetadata {
    pub fn new(product_title: impl Into<Arc<str>>, version: impl Into<Arc<str>>) -> Self {
        Self {
            product_title: product_title.into(),
            version: version.into(),
            description: None,
        }
    }

    pub fn description(mut self, description: impl Into<Arc<str>>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Injected metadata only. No application constants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutSection {
    pub metadata: AboutMetadata,
    pub assembly: Option<AboutSectionAssembly>,
}

/// Retained children created by [`AppContext::assemble_about_section`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AboutSectionAssembly {
    pub name_row: Option<StableNodeId>,
    pub name_value: Option<StableNodeId>,
    pub version_row: Option<StableNodeId>,
    pub version_value: Option<StableNodeId>,
    pub description: Option<StableNodeId>,
}

impl AboutSection {
    pub fn new(metadata: AboutMetadata) -> Self {
        Self {
            metadata,
            assembly: None,
        }
    }
}

impl ComponentView for AboutSection {
    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(AppContext::assemble_about_section),
        ..crate::TypeBehavior::NONE
    };

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "about-section".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        SettingsCard::new("").project(id, world, mutations);
        let accessibility = AccessibilityState {
            role: AccessibilityRole::Generic,
            label: Some(Arc::clone(&self.metadata.product_title)),
            value: Some(Arc::clone(&self.metadata.version)),
            description: self.metadata.description.clone(),
            ..AccessibilityState::default()
        };
        if !world.accessibility_eq(id, &accessibility) {
            mutations.set_accessibility(id, accessibility);
        }
    }
}

/// Apply a host-owned appearance event. Theme is not stored on [`AppearanceSettings`].
pub fn apply_appearance_event(
    theme_id: &mut nana_ui_core::ThemeId,
    appearance: &mut AppearanceSettings,
    event: AppearanceEvent,
) -> bool {
    match event {
        AppearanceEvent::Theme(next) => {
            if *theme_id == next {
                return false;
            }
            *theme_id = next;
            true
        }
        AppearanceEvent::StandardRadius(radius) => {
            appearance.set_standard_radius(f32::from(radius))
        }
        AppearanceEvent::WorkspaceCorners(enabled) => {
            appearance.set_workspace_corners_enabled(enabled)
        }
        AppearanceEvent::WindowMaterial(mode) => appearance.set_window_material(mode),
        AppearanceEvent::BackdropTarget(target) => appearance.set_backdrop_target(target),
        AppearanceEvent::BackdropOpacity(opacity) => appearance.set_backdrop_opacity(opacity),
        AppearanceEvent::TitlebarFollowsSidebar(enabled) => {
            appearance.set_titlebar_follows_sidebar(enabled)
        }
        AppearanceEvent::Reset => {
            let reset = nana_ui_core::ThemeId::new("nana.light");
            let theme_changed = *theme_id != reset;
            *theme_id = reset;
            appearance.reset() || theme_changed
        }
    }
}

fn appearance_mode(appearance: &AppearanceSettings) -> (bool, bool) {
    let solid_mode = !appearance.window_material().wants_transparent_surface();
    let titlebar_follow_disabled =
        solid_mode || !matches!(appearance.backdrop_target(), BackdropTarget::Sidebar);
    (solid_mode, titlebar_follow_disabled)
}

fn opacity_percent(appearance: &AppearanceSettings) -> f64 {
    f64::from((appearance.backdrop_opacity() * 100.0).round())
}

fn ensure_segmented_pair(
    context: &mut AppContext,
    document: crate::DocumentId,
    control_slot: &mut Option<StableNodeId>,
    first_slot: &mut Option<StableNodeId>,
    second_slot: &mut Option<StableNodeId>,
    first: SegmentedOption,
    second: SegmentedOption,
    selected_first: bool,
    first_disabled: bool,
    second_disabled: bool,
) -> Result<
    (
        Entity<SegmentedControl>,
        Entity<SegmentedOption>,
        Entity<SegmentedOption>,
    ),
    FrameworkError,
> {
    let created = control_slot.is_none();
    let control = if let Some(id) = *control_slot {
        Entity::from_stable_id(id)
    } else {
        let entity = context.create_detached_component(document, SegmentedControl::new())?;
        *control_slot = Some(entity.stable_id());
        entity
    };
    let first_entity = if let Some(id) = *first_slot {
        Entity::from_stable_id(id)
    } else {
        let entity = context.create_detached_component(document, first)?;
        *first_slot = Some(entity.stable_id());
        entity
    };
    let second_entity = if let Some(id) = *second_slot {
        Entity::from_stable_id(id)
    } else {
        let entity = context.create_detached_component(document, second)?;
        *second_slot = Some(entity.stable_id());
        entity
    };
    if created {
        context.set_segmented_options(
            control,
            vec![first_entity, second_entity],
            Some(if selected_first {
                first_entity
            } else {
                second_entity
            }),
        )?;
    } else {
        context.set_segmented_selection(
            control,
            Some(if selected_first {
                first_entity
            } else {
                second_entity
            }),
        )?;
    }
    context.set_segmented_option_disabled(control, first_entity, first_disabled)?;
    context.set_segmented_option_disabled(control, second_entity, second_disabled)?;
    Ok((control, first_entity, second_entity))
}

pub(super) fn offered_window_materials(
    available: &[WindowMaterialMode],
    current: WindowMaterialMode,
) -> Vec<WindowMaterialMode> {
    let mut modes = vec![WindowMaterialMode::Solid, WindowMaterialMode::Translucent];
    for mode in available
        .iter()
        .copied()
        .chain(std::iter::once(current))
        .filter(|mode| mode.wants_native())
    {
        if !modes.contains(&mode) {
            modes.push(mode);
        }
    }
    modes
}

fn material_option_slot(
    assembly: &mut AppearanceSectionAssembly,
    mode: WindowMaterialMode,
) -> &mut Option<StableNodeId> {
    match mode {
        WindowMaterialMode::Solid => &mut assembly.material_solid,
        WindowMaterialMode::Translucent => &mut assembly.material_translucent,
        WindowMaterialMode::Vibrancy => &mut assembly.material_vibrancy,
        WindowMaterialMode::Mica => &mut assembly.material_mica,
        WindowMaterialMode::Acrylic => &mut assembly.material_acrylic,
    }
}

fn ensure_material_options(
    context: &mut AppContext,
    document: crate::DocumentId,
    assembly: &mut AppearanceSectionAssembly,
    offered: &[WindowMaterialMode],
    selected: WindowMaterialMode,
) -> Result<Entity<SegmentedControl>, FrameworkError> {
    let control = if let Some(id) = assembly.material_control {
        Entity::from_stable_id(id)
    } else {
        let entity = context.create_detached_component(document, SegmentedControl::new())?;
        assembly.material_control = Some(entity.stable_id());
        entity
    };
    let mut options = Vec::with_capacity(offered.len());
    for mode in offered {
        let slot = material_option_slot(assembly, *mode);
        let entity = if let Some(id) = *slot {
            Entity::from_stable_id(id)
        } else {
            let entity = {
                let label = mode.label(context.world().framework_strings()).to_owned();
                context.create_detached_component(document, SegmentedOption::new(label))?
            };
            *slot = Some(entity.stable_id());
            entity
        };
        options.push(entity);
    }
    let selected_id = *material_option_slot(assembly, selected);
    let selected = options
        .iter()
        .copied()
        .find(|option| selected_id == Some(option.stable_id()))
        .or_else(|| options.first().copied());
    context.set_segmented_options(control, options, selected)?;
    Ok(control)
}

fn ensure_theme_options(
    context: &mut AppContext,
    document: crate::DocumentId,
    assembly: &mut AppearanceSectionAssembly,
    offered: &[ThemeChoice],
    selected: Option<&str>,
) -> Result<
    (
        Vec<Entity<SegmentedOption>>,
        Option<Entity<SegmentedOption>>,
    ),
    FrameworkError,
> {
    let mut options = Vec::with_capacity(offered.len());
    let previous = std::mem::take(&mut assembly.theme_options);
    for choice in offered {
        let entity = if let Some(id) = assembly
            .theme_options
            .get(options.len())
            .or_else(|| previous.get(options.len()))
            .copied()
        {
            Entity::from_stable_id(id)
        } else {
            // The built-in themes are named in the framework's words.
            let strings = context.world().framework_strings();
            let label = match choice.id.as_str() {
                "nana.light" => Arc::clone(&strings.theme_light),
                "nana.dark" => Arc::clone(&strings.theme_dark),
                _ => Arc::clone(&choice.label),
            };
            context.create_detached_component(document, SegmentedOption::new(label))?
        };
        assembly.theme_options.push(entity.stable_id());
        options.push(entity);
    }
    let selected = selected.and_then(|id| {
        offered
            .iter()
            .position(|choice| choice.id.as_str() == id)
            .and_then(|index| options.get(index).copied())
    });
    let selected = selected.or_else(|| options.first().copied());
    Ok((options, selected))
}

fn ensure_switch(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    label: &str,
    checked: bool,
    disabled: bool,
) -> Result<Entity<Switch>, FrameworkError> {
    if let Some(id) = *slot {
        let entity = Entity::<Switch>::from_stable_id(id);
        context.update_component(entity, |switch, _| {
            switch.checked = checked;
            switch.disabled = disabled;
        })?;
        Ok(entity)
    } else {
        let entity = context
            .create_detached_component(document, Switch::new(label, checked).disabled(disabled))?;
        *slot = Some(entity.stable_id());
        Ok(entity)
    }
}

fn ensure_range(
    context: &mut AppContext,
    document: crate::DocumentId,
    slot: &mut Option<StableNodeId>,
    value: f64,
    minimum: f64,
    maximum: f64,
    unit: &str,
    disabled: bool,
) -> Result<Entity<RangeField>, FrameworkError> {
    if let Some(id) = *slot {
        let entity = Entity::<RangeField>::from_stable_id(id);
        context.update_component(entity, |range, _| {
            range.value = range.quantize(value);
            range.disabled = disabled;
        })?;
        Ok(entity)
    } else {
        let mut field = RangeField::new(value, minimum, maximum, 1.0).unit(unit);
        field.disabled = disabled;
        let entity = context.create_detached_component(document, field)?;
        *slot = Some(entity.stable_id());
        Ok(entity)
    }
}

impl AppContext {
    pub fn assemble_appearance_section(
        &mut self,
        section: Entity<AppearanceSection>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, section.stable_id())?;
        let strings = self.world().framework_strings().clone();
        let snapshot = self.read(section, |section| {
            (
                section.theme_id.clone(),
                section.theme_options.clone(),
                section.appearance,
                section.material_status.clone(),
                section.platform_hint.clone(),
                section.available_materials.clone(),
                section.assembly.clone().unwrap_or_default(),
            )
        })?;
        let (
            theme_id,
            theme_options,
            appearance,
            material_status,
            platform_hint,
            available_materials,
            mut assembly,
        ) = snapshot;
        let (solid_mode, titlebar_follow_disabled) = appearance_mode(&appearance);
        let created_theme = assembly.theme_control.is_none();
        let created_material = assembly.material_control.is_none();
        let created_target = assembly.target_control.is_none();
        let created_titlebar = assembly.titlebar_switch.is_none();
        let created_workspace = assembly.workspace_switch.is_none();
        let created_radius = assembly.radius_range.is_none();
        let created_reset = assembly.reset_button.is_none();
        let mut created_opacity_range = false;
        let (theme_control, theme_dark, theme_light): (
            Entity<SegmentedControl>,
            Option<Entity<SegmentedOption>>,
            Option<Entity<SegmentedOption>>,
        ) = if theme_options.is_empty() {
            let (control, dark, light) = ensure_segmented_pair(
                self,
                document,
                &mut assembly.theme_control,
                &mut assembly.theme_dark,
                &mut assembly.theme_light,
                SegmentedOption::new(strings.settings_theme_dark.as_ref()).icon(Icon::Moon),
                SegmentedOption::new(strings.settings_theme_light.as_ref()).icon(Icon::Appearance),
                theme_id.as_str() == "nana.dark",
                false,
                false,
            )?;
            (control, Some(dark), Some(light))
        } else {
            let control = if let Some(id) = assembly.theme_control {
                Entity::from_stable_id(id)
            } else {
                let entity = self.create_detached_component(document, SegmentedControl::new())?;
                assembly.theme_control = Some(entity.stable_id());
                entity
            };
            let (options, selected) = ensure_theme_options(
                self,
                document,
                &mut assembly,
                &theme_options,
                Some(theme_id.as_str()),
            )?;
            self.set_segmented_options(control, options, selected)?;
            (control, None, None)
        };
        assembly.theme_control = Some(theme_control.stable_id());
        assembly.theme_dark = theme_dark.map(|entity| entity.stable_id());
        assembly.theme_light = theme_light.map(|entity| entity.stable_id());
        let theme_row = mount_settings_row(
            self,
            document,
            &mut assembly.theme_row,
            strings.settings_theme.as_ref(),
            Some(strings.settings_theme_hint.as_ref()),
            true,
            true,
            false,
            theme_control.stable_id(),
        )?;

        let offered_materials =
            offered_window_materials(&available_materials, appearance.window_material());
        let material_control = ensure_material_options(
            self,
            document,
            &mut assembly,
            &offered_materials,
            appearance.window_material(),
        )?;
        let material_hint = platform_hint
            .as_deref()
            .unwrap_or(strings.settings_material_hint.as_ref());
        let material_row = mount_settings_row(
            self,
            document,
            &mut assembly.material_row,
            strings.settings_material.as_ref(),
            Some(material_hint),
            true,
            false,
            false,
            material_control.stable_id(),
        )?;

        let status_row = if let Some(status) = material_status.as_deref() {
            let status_value = sync_text(
                self,
                document,
                &mut assembly.material_status_value,
                styled_text(
                    status,
                    SemanticColorRole::Muted,
                    nana_ui_core::TypeRole::Meta,
                    nana_ui_core::TextWeight::Regular,
                ),
            )?;
            Some(mount_settings_row(
                self,
                document,
                &mut assembly.material_status_row,
                strings.settings_material_state.as_ref(),
                Some(strings.settings_material_state_hint.as_ref()),
                true,
                false,
                false,
                status_value.stable_id(),
            )?)
        } else {
            None
        };

        let (target_control, target_sidebar, target_main) = ensure_segmented_pair(
            self,
            document,
            &mut assembly.target_control,
            &mut assembly.target_sidebar,
            &mut assembly.target_main,
            SegmentedOption::new(strings.material_region_sidebar.as_ref()).disabled(solid_mode),
            SegmentedOption::new(strings.material_region_main.as_ref()).disabled(solid_mode),
            matches!(appearance.backdrop_target(), BackdropTarget::Sidebar),
            solid_mode,
            solid_mode,
        )?;
        assembly.target_control = Some(target_control.stable_id());
        assembly.target_sidebar = Some(target_sidebar.stable_id());
        assembly.target_main = Some(target_main.stable_id());
        let target_hint = if solid_mode {
            strings.settings_region_solid_hint.as_ref()
        } else {
            strings.settings_region_hint.as_ref()
        };
        let target_row = mount_settings_row(
            self,
            document,
            &mut assembly.target_row,
            strings.settings_region.as_ref(),
            Some(target_hint),
            true,
            false,
            false,
            target_control.stable_id(),
        )?;

        let titlebar_switch = ensure_switch(
            self,
            document,
            &mut assembly.titlebar_switch,
            "",
            appearance.titlebar_follows_sidebar(),
            titlebar_follow_disabled,
        )?;
        let titlebar_hint = if titlebar_follow_disabled {
            strings.settings_title_bar_follows_main_hint.as_ref()
        } else {
            strings.settings_title_bar_follows_hint.as_ref()
        };
        let titlebar_row = mount_settings_row(
            self,
            document,
            &mut assembly.titlebar_row,
            strings.settings_title_bar_follows.as_ref(),
            Some(titlebar_hint),
            true,
            false,
            false,
            titlebar_switch.stable_id(),
        )?;

        let opacity = opacity_percent(&appearance);
        // Solid mode keeps the slider in place, disabled, so the row has the
        // same control as every other row and the kept value stays visible.
        created_opacity_range |= assembly.opacity_range.is_none();
        let opacity_control = ensure_range(
            self,
            document,
            &mut assembly.opacity_range,
            opacity,
            f64::from(AppearanceSettings::MIN_BACKDROP_OPACITY) * 100.0,
            f64::from(AppearanceSettings::MAX_BACKDROP_OPACITY) * 100.0,
            "%",
            solid_mode,
        )?
        .stable_id();
        assembly.opacity_text = None;
        let opacity_hint = if solid_mode {
            strings.settings_opacity_solid_hint.as_ref()
        } else {
            strings.settings_opacity_hint.as_ref()
        };
        let opacity_row = mount_settings_row(
            self,
            document,
            &mut assembly.opacity_row,
            strings.settings_opacity.as_ref(),
            Some(opacity_hint),
            true,
            false,
            false,
            opacity_control,
        )?;

        let workspace_switch = ensure_switch(
            self,
            document,
            &mut assembly.workspace_switch,
            "",
            appearance.workspace_corners_enabled(),
            false,
        )?;
        // The row names the setting; a second label inside the switch would
        // sit mid-row and push the track off the column of other controls.
        let workspace_row = mount_settings_row(
            self,
            document,
            &mut assembly.workspace_row,
            strings.settings_main_radius.as_ref(),
            None,
            true,
            false,
            false,
            workspace_switch.stable_id(),
        )?;

        let radius_range = ensure_range(
            self,
            document,
            &mut assembly.radius_range,
            f64::from(appearance.standard_radius()),
            f64::from(AppearanceSettings::MIN_STANDARD_RADIUS),
            f64::from(AppearanceSettings::MAX_STANDARD_RADIUS),
            " px",
            false,
        )?;
        let radius_row = mount_settings_row(
            self,
            document,
            &mut assembly.radius_row,
            strings.settings_radius.as_ref(),
            None,
            true,
            false,
            false,
            radius_range.stable_id(),
        )?;

        let reset_button = if let Some(id) = assembly.reset_button {
            Entity::<Button>::from_stable_id(id)
        } else {
            let entity = self.create_detached_component(
                document,
                Button::new(strings.settings_restore.as_ref())
                    .kind(ButtonKind::Subtle)
                    .size(ControlSize::Small),
            )?;
            assembly.reset_button = Some(entity.stable_id());
            entity
        };
        let reset_row = mount_settings_row(
            self,
            document,
            &mut assembly.reset_row,
            strings.settings_defaults.as_ref(),
            Some(strings.settings_defaults_hint.as_ref()),
            false,
            false,
            true,
            reset_button.stable_id(),
        )?;

        if created_theme && !theme_options.is_empty() {
            let option_ids = assembly.theme_options.clone();
            let theme_ids: Vec<_> = theme_options
                .iter()
                .map(|choice| choice.id.clone())
                .collect();
            self.observe(
                theme_control,
                section,
                move |_, event: &SegmentedSelectionRequested, cx| {
                    let Some(index) = option_ids.iter().position(|id| *id == event.option) else {
                        return;
                    };
                    if let Some(theme_id) = theme_ids.get(index) {
                        cx.emit(AppearanceEvent::Theme(theme_id.clone()));
                    }
                },
            )?;
        } else if created_theme {
            let (Some(dark), Some(light)) = (theme_dark, theme_light) else {
                return Err(FrameworkError::InvalidComponentValue(
                    theme_control.stable_id(),
                ));
            };
            let dark = dark.stable_id();
            let light = light.stable_id();
            self.observe(
                theme_control,
                section,
                move |_, event: &SegmentedSelectionRequested, cx| {
                    let next = if event.option == dark {
                        nana_ui_core::ThemeId::new("nana.dark")
                    } else if event.option == light {
                        nana_ui_core::ThemeId::new("nana.light")
                    } else {
                        return;
                    };
                    cx.emit(AppearanceEvent::Theme(next));
                },
            )?;
        }
        if created_material {
            let solid = assembly.material_solid;
            let translucent = assembly.material_translucent;
            let vibrancy = assembly.material_vibrancy;
            let mica = assembly.material_mica;
            let acrylic = assembly.material_acrylic;
            self.observe(
                material_control,
                section,
                move |_, event: &SegmentedSelectionRequested, cx| {
                    let next = if Some(event.option) == solid {
                        WindowMaterialMode::Solid
                    } else if Some(event.option) == translucent {
                        WindowMaterialMode::Translucent
                    } else if Some(event.option) == vibrancy {
                        WindowMaterialMode::Vibrancy
                    } else if Some(event.option) == mica {
                        WindowMaterialMode::Mica
                    } else if Some(event.option) == acrylic {
                        WindowMaterialMode::Acrylic
                    } else {
                        return;
                    };
                    cx.emit(AppearanceEvent::WindowMaterial(next));
                },
            )?;
        }
        if created_target {
            let sidebar = target_sidebar.stable_id();
            let main = target_main.stable_id();
            self.observe(
                target_control,
                section,
                move |_, event: &SegmentedSelectionRequested, cx| {
                    let next = if event.option == sidebar {
                        BackdropTarget::Sidebar
                    } else if event.option == main {
                        BackdropTarget::Main
                    } else {
                        return;
                    };
                    cx.emit(AppearanceEvent::BackdropTarget(next));
                },
            )?;
        }
        if created_titlebar {
            self.observe(titlebar_switch, section, |_, event: &ToggleChanged, cx| {
                cx.emit(AppearanceEvent::TitlebarFollowsSidebar(event.checked));
            })?;
        }
        if created_opacity_range && let Some(id) = assembly.opacity_range {
            self.observe(
                Entity::<RangeField>::from_stable_id(id),
                section,
                |_, event: &RangeInput, cx| {
                    cx.emit(AppearanceEvent::BackdropOpacity(event.value as f32 / 100.0));
                },
            )?;
        }
        if created_workspace {
            self.observe(workspace_switch, section, |_, event: &ToggleChanged, cx| {
                cx.emit(AppearanceEvent::WorkspaceCorners(event.checked));
            })?;
        }
        if created_radius {
            self.observe(radius_range, section, |_, event: &RangeInput, cx| {
                cx.emit(AppearanceEvent::StandardRadius(event.value.round() as u8));
            })?;
        }
        if created_reset {
            self.observe(reset_button, section, |_, _: &Activate, cx| {
                cx.emit(AppearanceEvent::Reset);
            })?;
        }

        let mut ordered = vec![theme_row.stable_id(), material_row.stable_id()];
        if let Some(status_row) = status_row {
            ordered.push(status_row.stable_id());
        }
        ordered.extend([
            target_row.stable_id(),
            titlebar_row.stable_id(),
            opacity_row.stable_id(),
            workspace_row.stable_id(),
            radius_row.stable_id(),
            reset_row.stable_id(),
        ]);
        self.update_component(section, |section, _| {
            section.assembly = Some(assembly);
        })?;
        reconcile_children(self, section, &ordered)
    }

    /// Mount or refresh name/version/description rows from injected metadata.
    pub fn assemble_about_section(
        &mut self,
        section: Entity<AboutSection>,
    ) -> Result<bool, FrameworkError> {
        let document = document_of(self, section.stable_id())?;
        let strings = self.world().framework_strings().clone();
        let (metadata, mut assembly) = self.read(section, |section| {
            (
                section.metadata.clone(),
                section.assembly.clone().unwrap_or_default(),
            )
        })?;
        let name_value = sync_text(
            self,
            document,
            &mut assembly.name_value,
            styled_text(
                metadata.product_title.as_ref(),
                SemanticColorRole::Text,
                nana_ui_core::TypeRole::Meta,
                nana_ui_core::TextWeight::Medium,
            ),
        )?;
        // The description describes the product, so it is the name row's
        // hint rather than a loose line under the version.
        let name_row = mount_settings_row(
            self,
            document,
            &mut assembly.name_row,
            strings.settings_name.as_ref(),
            metadata.description.as_deref(),
            true,
            true,
            false,
            name_value.stable_id(),
        )?;
        let version_value = sync_text(
            self,
            document,
            &mut assembly.version_value,
            styled_text(
                metadata.version.as_ref(),
                SemanticColorRole::Muted,
                nana_ui_core::TypeRole::Meta,
                nana_ui_core::TextWeight::Regular,
            ),
        )?;
        let version_row = mount_settings_row(
            self,
            document,
            &mut assembly.version_row,
            strings.settings_version.as_ref(),
            None,
            false,
            false,
            true,
            version_value.stable_id(),
        )?;
        let ordered = vec![name_row.stable_id(), version_row.stable_id()];
        assembly.description = None;
        self.update_component(section, |section, _| {
            section.assembly = Some(assembly);
        })?;
        reconcile_children(self, section, &ordered)
    }
}
