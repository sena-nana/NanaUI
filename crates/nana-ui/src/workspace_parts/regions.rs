use super::*;

/// Application content registered for one workspace region.
pub struct WorkspaceRegion<'a, Message> {
    pub(super) id: RegionId,
    pub(super) content: Element<'a, Message>,
}

impl<'a, Message> WorkspaceRegion<'a, Message> {
    pub fn new(id: RegionId, content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            id,
            content: content.into(),
        }
    }
}

/// Ordered content registrations consumed by [`workspace_view`].
pub struct WorkspaceRegions<'a, Message> {
    pub(super) regions: Vec<WorkspaceRegion<'a, Message>>,
}

impl<'a, Message> WorkspaceRegions<'a, Message> {
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
        }
    }

    pub fn with_region(mut self, id: RegionId, content: impl Into<Element<'a, Message>>) -> Self {
        self.regions.push(WorkspaceRegion::new(id, content));
        self
    }

    pub fn push(&mut self, region: WorkspaceRegion<'a, Message>) {
        self.regions.push(region);
    }
}

impl<Message> Default for WorkspaceRegions<'_, Message> {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience content bundle for the standard six-region workspace.
pub struct WorkspaceSlots<'a, Message> {
    pub global_navigation: Element<'a, Message>,
    pub resources: Element<'a, Message>,
    pub primary_toolbar: Element<'a, Message>,
    pub primary: Element<'a, Message>,
    pub inspector: Element<'a, Message>,
    pub diagnostics: Element<'a, Message>,
}

impl<'a, Message> WorkspaceSlots<'a, Message> {
    pub fn new(
        global_navigation: impl Into<Element<'a, Message>>,
        resources: impl Into<Element<'a, Message>>,
        primary_toolbar: impl Into<Element<'a, Message>>,
        primary: impl Into<Element<'a, Message>>,
        inspector: impl Into<Element<'a, Message>>,
        diagnostics: impl Into<Element<'a, Message>>,
    ) -> Self {
        Self {
            global_navigation: global_navigation.into(),
            resources: resources.into(),
            primary_toolbar: primary_toolbar.into(),
            primary: primary.into(),
            inspector: inspector.into(),
            diagnostics: diagnostics.into(),
        }
    }
}

impl<'a, Message> From<WorkspaceSlots<'a, Message>> for WorkspaceRegions<'a, Message> {
    fn from(slots: WorkspaceSlots<'a, Message>) -> Self {
        Self::new()
            .with_region(RegionId::GlobalNavigation, slots.global_navigation)
            .with_region(RegionId::Resources, slots.resources)
            .with_region(RegionId::PrimaryToolbar, slots.primary_toolbar)
            .with_region(RegionId::Primary, slots.primary)
            .with_region(RegionId::Inspector, slots.inspector)
            .with_region(RegionId::Diagnostics, slots.diagnostics)
    }
}
