use super::*;

impl GalleryState {
    pub(super) fn set_workspace_showcase_visible(&mut self, visible: bool) {
        for id in [
            RegionId::PrimaryToolbar,
            RegionId::Inspector,
            RegionId::Diagnostics,
        ] {
            self.workspace
                .update(WorkspaceAction::SetRegionVisible(id, visible));
        }
    }
}
