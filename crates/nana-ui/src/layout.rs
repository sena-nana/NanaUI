use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable identifiers used by the standard workspace plus application-defined
/// identifiers for additional regions.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegionId {
    GlobalNavigation,
    SectionNavigation,
    Resources,
    PrimaryToolbar,
    Primary,
    Inspector,
    Diagnostics,
    Custom(String),
}

impl RegionId {
    pub fn custom(id: impl Into<String>) -> Self {
        Self::Custom(id.into())
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::GlobalNavigation => "global-navigation",
            Self::SectionNavigation => "section-navigation",
            Self::Resources => "resources",
            Self::PrimaryToolbar => "primary-toolbar",
            Self::Primary => "primary",
            Self::Inspector => "inspector",
            Self::Diagnostics => "diagnostics",
            Self::Custom(id) => id,
        }
    }
}

impl fmt::Display for RegionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Semantic role of a workspace region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegionRole {
    GlobalNavigation,
    SectionNavigation,
    Resources,
    Primary,
    Inspector,
    Timeline,
    Console,
    Utility,
}

/// A region's position in the workspace grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegionPlacement {
    Start,
    Primary,
    End,
    Top,
    Bottom,
}

/// Whether a top or bottom region spans the whole workspace or only primary
/// columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegionScope {
    Workspace,
    Primary,
}

/// Responsive behavior when the workspace becomes narrower than a region's
/// threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NarrowBehavior {
    Shrink,
    Collapse,
    Overlay,
    None,
}

/// A registered region's structural and persisted layout contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegionState {
    id: RegionId,
    role: RegionRole,
    placement: RegionPlacement,
    scope: RegionScope,
    size: Option<f32>,
    default_size: Option<f32>,
    min_size: f32,
    max_size: f32,
    fill_priority: u16,
    collapsible: bool,
    resizable: bool,
    collapsed: bool,
    hidden: bool,
    disabled: bool,
    narrow_behavior: NarrowBehavior,
    collapse_below: Option<f32>,
    responsive_priority: u16,
}

impl RegionState {
    pub fn new(id: RegionId, role: RegionRole) -> Self {
        let placement = match role {
            RegionRole::GlobalNavigation
            | RegionRole::SectionNavigation
            | RegionRole::Resources => RegionPlacement::Start,
            RegionRole::Primary => RegionPlacement::Primary,
            RegionRole::Inspector => RegionPlacement::End,
            RegionRole::Timeline | RegionRole::Console => RegionPlacement::Bottom,
            RegionRole::Utility => RegionPlacement::Start,
        };
        let size = if matches!(
            placement,
            RegionPlacement::Primary | RegionPlacement::Top | RegionPlacement::Bottom
        ) {
            None
        } else {
            Some(240.0)
        };

        Self {
            id,
            role,
            placement,
            scope: RegionScope::Workspace,
            size,
            default_size: size,
            min_size: 0.0,
            max_size: 4096.0,
            fill_priority: u16::from(role == RegionRole::Primary),
            collapsible: false,
            resizable: false,
            collapsed: false,
            hidden: false,
            disabled: false,
            narrow_behavior: NarrowBehavior::Shrink,
            collapse_below: None,
            responsive_priority: 0,
        }
    }

    pub fn placement(mut self, placement: RegionPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn scope(mut self, scope: RegionScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self.default_size = Some(size);
        self.normalize();
        self
    }

    pub fn content_sized(mut self) -> Self {
        self.size = None;
        self.default_size = None;
        self
    }

    pub fn min_size(mut self, min_size: f32) -> Self {
        self.min_size = min_size;
        self.normalize();
        self
    }

    pub fn max_size(mut self, max_size: f32) -> Self {
        self.max_size = max_size;
        self.normalize();
        self
    }

    pub fn fill_priority(mut self, fill_priority: u16) -> Self {
        self.fill_priority = fill_priority;
        if fill_priority > 0 {
            self.size = None;
            self.default_size = None;
        }
        self
    }

    pub fn collapsible(mut self, collapsible: bool) -> Self {
        self.collapsible = collapsible;
        self.normalize();
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self.normalize();
        self
    }

    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn narrow_behavior(mut self, narrow_behavior: NarrowBehavior) -> Self {
        self.narrow_behavior = narrow_behavior;
        self
    }

    pub fn collapse_below(mut self, collapse_below: f32) -> Self {
        self.collapse_below = Some(collapse_below);
        self
    }

    pub fn responsive_priority(mut self, responsive_priority: u16) -> Self {
        self.responsive_priority = responsive_priority;
        self
    }

    fn normalize(&mut self) {
        self.min_size = finite_non_negative(self.min_size);
        self.max_size = finite_non_negative(self.max_size).max(self.min_size);
        self.size = self
            .size
            .filter(|size| size.is_finite())
            .map(|size| size.clamp(self.min_size, self.max_size));
        self.default_size = self
            .default_size
            .filter(|size| size.is_finite())
            .map(|size| size.clamp(self.min_size, self.max_size));
        self.collapse_below = self
            .collapse_below
            .filter(|threshold| threshold.is_finite() && *threshold > 0.0);
        if !self.collapsible {
            self.collapsed = false;
        }
        if self.fill_priority > 0 {
            self.size = None;
        }
    }

    pub fn id(&self) -> &RegionId {
        &self.id
    }

    pub fn role(&self) -> RegionRole {
        self.role
    }

    pub fn placement_value(&self) -> RegionPlacement {
        self.placement
    }

    pub fn scope_value(&self) -> RegionScope {
        self.scope
    }

    pub fn size_value(&self) -> Option<f32> {
        self.size
    }

    pub fn min_size_value(&self) -> f32 {
        self.min_size
    }

    pub fn max_size_value(&self) -> f32 {
        self.max_size
    }

    pub fn fill_priority_value(&self) -> u16 {
        self.fill_priority
    }

    pub fn collapsible_value(&self) -> bool {
        self.collapsible
    }

    pub fn resizable_value(&self) -> bool {
        self.resizable
    }

    pub fn collapsed_value(&self) -> bool {
        self.collapsed
    }

    pub fn hidden_value(&self) -> bool {
        self.hidden
    }

    pub fn disabled_value(&self) -> bool {
        self.disabled
    }

    pub fn narrow_behavior_value(&self) -> NarrowBehavior {
        self.narrow_behavior
    }

    pub fn collapse_below_value(&self) -> Option<f32> {
        self.collapse_below
    }

    pub fn responsive_priority_value(&self) -> u16 {
        self.responsive_priority
    }

    pub fn requested_visible(&self) -> bool {
        !self.hidden && !self.collapsed
    }

    pub fn responsive_threshold(&self) -> Option<f32> {
        if let Some(threshold) = self.collapse_below {
            return Some(threshold);
        }
        if matches!(
            self.narrow_behavior,
            NarrowBehavior::None | NarrowBehavior::Shrink
        ) {
            return None;
        }
        Some((960.0 - f32::from(self.responsive_priority) * 160.0).max(480.0))
    }

    pub fn responsive_collapsed(&self, inline_size: f32) -> bool {
        self.narrow_behavior == NarrowBehavior::Collapse
            && self
                .responsive_threshold()
                .is_some_and(|threshold| inline_size > 0.0 && inline_size < threshold)
    }

    pub fn responsive_overlay(&self, inline_size: f32) -> bool {
        self.requested_visible()
            && self.narrow_behavior == NarrowBehavior::Overlay
            && self
                .responsive_threshold()
                .is_some_and(|threshold| inline_size > 0.0 && inline_size < threshold)
    }

    pub fn visible_at(&self, inline_size: f32) -> bool {
        self.requested_visible() && !self.responsive_collapsed(inline_size)
    }

    pub fn extent(&self) -> f32 {
        self.size.unwrap_or(self.min_size)
    }
}

/// Invalid workspace region registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceLayoutError {
    DuplicateRegion(RegionId),
}

impl fmt::Display for WorkspaceLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateRegion(id) => write!(formatter, "workspace region `{id}` is duplicated"),
        }
    }
}

impl std::error::Error for WorkspaceLayoutError {}

/// Serializable ordered registration model for a workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceLayout {
    regions: Vec<RegionState>,
}

impl Default for WorkspaceLayout {
    fn default() -> Self {
        Self {
            regions: vec![
                RegionState::new(RegionId::GlobalNavigation, RegionRole::GlobalNavigation)
                    .size(56.0)
                    .min_size(44.0)
                    .max_size(96.0),
                RegionState::new(RegionId::Resources, RegionRole::Resources)
                    .size(240.0)
                    .min_size(180.0)
                    .max_size(520.0)
                    .collapsible(true)
                    .resizable(true),
                RegionState::new(RegionId::PrimaryToolbar, RegionRole::Utility)
                    .placement(RegionPlacement::Top)
                    .scope(RegionScope::Primary)
                    .size(42.0),
                RegionState::new(RegionId::Primary, RegionRole::Primary)
                    .min_size(320.0)
                    .fill_priority(1),
                RegionState::new(RegionId::Inspector, RegionRole::Inspector)
                    .size(240.0)
                    .min_size(200.0)
                    .max_size(560.0)
                    .collapsible(true)
                    .resizable(true),
                RegionState::new(RegionId::Diagnostics, RegionRole::Console)
                    .placement(RegionPlacement::Bottom)
                    .scope(RegionScope::Primary)
                    .size(200.0)
                    .min_size(96.0)
                    .max_size(520.0)
                    .collapsible(true)
                    .resizable(true),
            ],
        }
    }
}

impl WorkspaceLayout {
    pub fn new(
        regions: impl IntoIterator<Item = RegionState>,
    ) -> Result<Self, WorkspaceLayoutError> {
        let mut layout = Self {
            regions: Vec::new(),
        };
        for region in regions {
            layout.register(region)?;
        }
        Ok(layout)
    }

    pub fn regions(&self) -> &[RegionState] {
        &self.regions
    }

    pub fn region(&self, id: &RegionId) -> Option<&RegionState> {
        self.regions.iter().find(|region| region.id() == id)
    }

    pub fn register(&mut self, mut region: RegionState) -> Result<(), WorkspaceLayoutError> {
        if self.region(region.id()).is_some() {
            return Err(WorkspaceLayoutError::DuplicateRegion(region.id().clone()));
        }
        region.normalize();
        self.regions.push(region);
        Ok(())
    }

    pub fn unregister(&mut self, id: &RegionId) -> Option<RegionState> {
        let index = self.regions.iter().position(|region| region.id() == id)?;
        Some(self.regions.remove(index))
    }

    fn region_mut(&mut self, id: &RegionId) -> Option<&mut RegionState> {
        self.regions.iter_mut().find(|region| region.id() == id)
    }

    pub fn resize_by(&mut self, id: &RegionId, delta: f32) -> bool {
        let Some(region) = self.region_mut(id) else {
            return false;
        };
        if !region.resizable
            || region.disabled
            || region.fill_priority > 0
            || !region.requested_visible()
            || !delta.is_finite()
        {
            return false;
        }

        let previous = region.size;
        let base = region.size.unwrap_or(region.min_size);
        region.size = Some((base + delta).clamp(region.min_size, region.max_size));
        region.size != previous
    }

    pub fn set_size(&mut self, id: &RegionId, size: f32) -> bool {
        let Some(region) = self.region_mut(id) else {
            return false;
        };
        if region.fill_priority > 0 || !size.is_finite() {
            return false;
        }
        let previous = region.size;
        region.size = Some(size.clamp(region.min_size, region.max_size));
        region.size != previous
    }

    pub fn reset_size(&mut self, id: &RegionId) -> bool {
        let Some(region) = self.region_mut(id) else {
            return false;
        };
        if region.fill_priority > 0 {
            return false;
        }
        let previous = region.size;
        region.size = region.default_size;
        region.size != previous
    }

    pub fn set_hidden(&mut self, id: &RegionId, hidden: bool) -> bool {
        let Some(region) = self.region_mut(id) else {
            return false;
        };
        let changed = region.hidden != hidden;
        region.hidden = hidden;
        changed
    }

    pub fn toggle_collapsed(&mut self, id: &RegionId) -> bool {
        let Some(region) = self.region_mut(id) else {
            return false;
        };
        if !region.collapsible || region.hidden || region.disabled {
            return false;
        }
        region.collapsed = !region.collapsed;
        true
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(value: &str) -> Result<Self, serde_json::Error> {
        let mut layout: Self = serde_json::from_str(value)?;
        for region in &mut layout.regions {
            region.normalize();
        }
        Ok(layout)
    }

    /// Restores mutable region state while preserving the currently registered
    /// roles, placement, constraints, and responsive contract.
    pub fn restore_json(&mut self, value: &str) -> Result<(), serde_json::Error> {
        let restored = Self::from_json(value)?;
        for region in &mut self.regions {
            let Some(saved) = restored.region(region.id()) else {
                continue;
            };
            region.size = saved
                .size
                .map(|size| size.clamp(region.min_size, region.max_size));
            region.collapsed = region.collapsible && saved.collapsed;
            region.hidden = saved.hidden;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), WorkspaceLayoutError> {
        let mut ids = HashSet::new();
        for region in &self.regions {
            if !ids.insert(region.id()) {
                return Err(WorkspaceLayoutError::DuplicateRegion(region.id().clone()));
            }
        }
        Ok(())
    }
}

fn finite_non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NarrowBehavior, RegionId, RegionPlacement, RegionRole, RegionScope, RegionState,
        WorkspaceLayout, WorkspaceLayoutError,
    };

    #[test]
    fn standard_layout_matches_the_workspace_region_contract() {
        let layout = WorkspaceLayout::default();

        assert_eq!(layout.regions().len(), 6);
        assert_eq!(
            layout
                .region(&RegionId::PrimaryToolbar)
                .expect("primary toolbar")
                .scope_value(),
            RegionScope::Primary
        );
        assert_eq!(
            layout
                .region(&RegionId::Diagnostics)
                .expect("diagnostics")
                .placement_value(),
            RegionPlacement::Bottom
        );
        assert_eq!(
            layout
                .region(&RegionId::Primary)
                .expect("primary")
                .fill_priority_value(),
            1
        );
    }

    #[test]
    fn dynamic_layout_accepts_additional_ordered_regions() {
        let pull_requests = RegionId::custom("pull-requests");
        let layout = WorkspaceLayout::new([
            RegionState::new(RegionId::Resources, RegionRole::Resources),
            RegionState::new(pull_requests.clone(), RegionRole::SectionNavigation)
                .size(230.0)
                .resizable(true),
            RegionState::new(RegionId::Primary, RegionRole::Primary),
        ])
        .expect("dynamic workspace");

        assert_eq!(layout.regions()[1].id(), &pull_requests);
        assert_eq!(
            layout
                .region(&pull_requests)
                .expect("pull requests")
                .size_value(),
            Some(230.0)
        );
    }

    #[test]
    fn duplicate_region_ids_are_rejected() {
        let duplicate = WorkspaceLayout::new([
            RegionState::new(RegionId::Resources, RegionRole::Resources),
            RegionState::new(RegionId::Resources, RegionRole::SectionNavigation),
        ]);

        assert_eq!(
            duplicate,
            Err(WorkspaceLayoutError::DuplicateRegion(RegionId::Resources))
        );
    }

    #[test]
    fn resize_respects_constraints_and_region_capabilities() {
        let mut layout = WorkspaceLayout::default();

        assert!(layout.resize_by(&RegionId::Resources, 400.0));
        assert_eq!(
            layout
                .region(&RegionId::Resources)
                .expect("resources")
                .size_value(),
            Some(520.0)
        );
        assert!(!layout.resize_by(&RegionId::GlobalNavigation, 8.0));
        assert!(!layout.resize_by(&RegionId::Primary, 80.0));
    }

    #[test]
    fn responsive_regions_collapse_or_overlay_at_their_threshold() {
        let collapsed = RegionState::new(RegionId::Resources, RegionRole::Resources)
            .narrow_behavior(NarrowBehavior::Collapse)
            .collapse_below(800.0);
        let overlay = RegionState::new(RegionId::Inspector, RegionRole::Inspector)
            .narrow_behavior(NarrowBehavior::Overlay)
            .responsive_priority(1);

        assert!(collapsed.responsive_collapsed(799.0));
        assert!(!collapsed.visible_at(799.0));
        assert!(overlay.responsive_overlay(799.0));
        assert!(!overlay.responsive_overlay(801.0));
    }

    #[test]
    fn restore_preserves_registered_structure_and_constraints() {
        let mut layout = WorkspaceLayout::default();
        let mut saved = layout.clone();
        saved.toggle_collapsed(&RegionId::Inspector);
        saved.set_size(&RegionId::Resources, 312.0);

        let mut value = serde_json::to_value(saved).expect("layout value");
        value["regions"][1]["role"] = serde_json::json!("Primary");
        value["regions"][1]["max_size"] = serde_json::json!(9000.0);

        layout
            .restore_json(&value.to_string())
            .expect("layout restores");
        let resources = layout.region(&RegionId::Resources).expect("resources");
        assert_eq!(resources.role(), RegionRole::Resources);
        assert_eq!(resources.max_size_value(), 520.0);
        assert_eq!(resources.size_value(), Some(312.0));
        assert!(
            layout
                .region(&RegionId::Inspector)
                .expect("inspector")
                .collapsed_value()
        );
    }
}
