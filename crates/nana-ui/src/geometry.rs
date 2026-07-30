use crate::layout::{RegionId, RegionPlacement, RegionScope, RegionState, WorkspaceLayout};

/// Fixed chrome dimensions used by the NanaUI application shell.
pub const TITLE_BAR_HEIGHT: f32 = 36.0;
pub const RESIZE_HANDLE_SIZE: f32 = 8.0;

/// A logical-pixel rectangle that can be handed to a content view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicalRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl LogicalRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width: width.max(0.0),
            height: height.max(0.0),
        }
    }
}

/// A physical-pixel rectangle derived from a logical rectangle and scale factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Geometry for one registered workspace region.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionRect {
    pub id: RegionId,
    pub logical: LogicalRect,
    pub physical: PhysicalRect,
    pub visible: bool,
    pub overlay: bool,
}

/// Layout snapshot suitable for host-owned WGPU content.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceGeometry {
    pub logical_size: (f32, f32),
    pub physical_size: (u32, u32),
    pub scale_factor: f32,
    regions: Vec<RegionRect>,
}

impl WorkspaceGeometry {
    pub fn new(
        layout: &WorkspaceLayout,
        logical_width: f32,
        logical_height: f32,
        scale_factor: f32,
    ) -> Self {
        let logical_width = finite_non_negative(logical_width);
        let logical_height = finite_non_negative(logical_height);
        let scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let body_y = TITLE_BAR_HEIGHT.min(logical_height);
        let body_height = (logical_height - body_y).max(0.0);
        let inline_size = logical_width;
        let mut regions = layout
            .regions()
            .iter()
            .map(|region| RegionRect {
                id: region.id().clone(),
                logical: LogicalRect::new(0.0, 0.0, 0.0, 0.0),
                physical: PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                },
                visible: false,
                overlay: false,
            })
            .collect::<Vec<_>>();

        let mut starts = Vec::new();
        let mut primaries = Vec::new();
        let mut ends = Vec::new();
        let mut workspace_top = Vec::new();
        let mut primary_top = Vec::new();
        let mut primary_bottom = Vec::new();
        let mut workspace_bottom = Vec::new();
        let mut overlays = Vec::new();

        for region in layout.regions() {
            if !region.visible_at(inline_size) {
                continue;
            }
            if region.responsive_overlay(inline_size) {
                overlays.push(region);
                continue;
            }
            match (region.placement_value(), region.scope_value()) {
                (RegionPlacement::Start, _) => starts.push(region),
                (RegionPlacement::Primary, _) => primaries.push(region),
                (RegionPlacement::End, _) => ends.push(region),
                (RegionPlacement::Top, RegionScope::Workspace) => workspace_top.push(region),
                (RegionPlacement::Top, RegionScope::Primary) => primary_top.push(region),
                (RegionPlacement::Bottom, RegionScope::Workspace) => workspace_bottom.push(region),
                (RegionPlacement::Bottom, RegionScope::Primary) => primary_bottom.push(region),
            }
        }

        let workspace_top_total = group_extent(&workspace_top);
        let workspace_bottom_total = group_extent(&workspace_bottom);
        let middle_y = body_y + workspace_top_total;
        let middle_height = (body_height - workspace_top_total - workspace_bottom_total).max(0.0);
        let middle_bottom = middle_y + middle_height;

        let mut y = body_y;
        for region in workspace_top {
            let height = region.extent().min((middle_y - y).max(0.0));
            set_region(
                &mut regions,
                region.id(),
                LogicalRect::new(0.0, y, logical_width, height),
                false,
                scale_factor,
            );
            y += height + handle_extent(region);
        }

        y = middle_bottom;
        for region in workspace_bottom {
            let height = region.extent().min((logical_height - y).max(0.0));
            set_region(
                &mut regions,
                region.id(),
                LogicalRect::new(0.0, y + handle_extent(region), logical_width, height),
                false,
                scale_factor,
            );
            y += height + handle_extent(region);
        }

        let starts_total = group_extent(&starts);
        let ends_total = group_extent(&ends);
        let primary_x = starts_total.min(logical_width);
        let primary_width = (logical_width - starts_total - ends_total).max(0.0);
        let primary_end = primary_x + primary_width;

        let mut x = 0.0;
        for region in starts {
            let width = region.extent();
            set_region(
                &mut regions,
                region.id(),
                LogicalRect::new(x, middle_y, width, middle_height),
                false,
                scale_factor,
            );
            x += width + handle_extent(region);
        }

        x = primary_end;
        for region in ends {
            x += handle_extent(region);
            let width = region.extent();
            set_region(
                &mut regions,
                region.id(),
                LogicalRect::new(x, middle_y, width, middle_height),
                false,
                scale_factor,
            );
            x += width;
        }

        let primary_top_total = group_extent(&primary_top);
        let primary_bottom_total = group_extent(&primary_bottom);
        let primary_middle_y = middle_y + primary_top_total;
        let primary_middle_height =
            (middle_height - primary_top_total - primary_bottom_total).max(0.0);

        y = middle_y;
        for region in primary_top {
            let height = region.extent();
            set_region(
                &mut regions,
                region.id(),
                LogicalRect::new(primary_x, y, primary_width, height),
                false,
                scale_factor,
            );
            y += height + handle_extent(region);
        }

        let primary_widths = allocate_primary_widths(&primaries, primary_width);
        x = primary_x;
        for (region, width) in primaries.into_iter().zip(primary_widths) {
            set_region(
                &mut regions,
                region.id(),
                LogicalRect::new(x, primary_middle_y, width, primary_middle_height),
                false,
                scale_factor,
            );
            x += width + handle_extent(region);
        }

        y = primary_middle_y + primary_middle_height;
        for region in primary_bottom {
            y += handle_extent(region);
            let height = region.extent();
            set_region(
                &mut regions,
                region.id(),
                LogicalRect::new(primary_x, y, primary_width, height),
                false,
                scale_factor,
            );
            y += height;
        }

        for region in overlays {
            let extent = region.extent();
            let logical = match region.placement_value() {
                RegionPlacement::Start | RegionPlacement::Primary => {
                    LogicalRect::new(0.0, body_y, extent, body_height)
                }
                RegionPlacement::End => LogicalRect::new(
                    (logical_width - extent).max(0.0),
                    body_y,
                    extent,
                    body_height,
                ),
                RegionPlacement::Top => LogicalRect::new(0.0, body_y, logical_width, extent),
                RegionPlacement::Bottom => LogicalRect::new(
                    0.0,
                    (logical_height - extent).max(body_y),
                    logical_width,
                    extent,
                ),
            };
            set_region(&mut regions, region.id(), logical, true, scale_factor);
        }

        Self {
            logical_size: (logical_width, logical_height),
            physical_size: (
                physical_dimension(logical_width, scale_factor),
                physical_dimension(logical_height, scale_factor),
            ),
            scale_factor,
            regions,
        }
    }

    pub fn region(&self, id: &RegionId) -> Option<&RegionRect> {
        self.regions.iter().find(|region| &region.id == id)
    }

    pub fn regions(&self) -> &[RegionRect] {
        &self.regions
    }
}

fn group_extent(regions: &[&RegionState]) -> f32 {
    regions
        .iter()
        .map(|region| region.extent() + handle_extent(region))
        .sum()
}

fn handle_extent(region: &RegionState) -> f32 {
    if region.resizable_value() && !region.disabled_value() && region.fill_priority_value() == 0 {
        RESIZE_HANDLE_SIZE
    } else {
        0.0
    }
}

fn allocate_primary_widths(regions: &[&RegionState], total_width: f32) -> Vec<f32> {
    if regions.is_empty() {
        return Vec::new();
    }
    let handles: f32 = regions.iter().map(|region| handle_extent(region)).sum();
    let available = (total_width - handles).max(0.0);
    let fixed: f32 = regions
        .iter()
        .filter(|region| region.fill_priority_value() == 0)
        .map(|region| region.extent())
        .sum();
    let fill_minimum: f32 = regions
        .iter()
        .filter(|region| region.fill_priority_value() > 0)
        .map(|region| region.min_size_value())
        .sum();
    let fill_weight: u32 = regions
        .iter()
        .map(|region| u32::from(region.fill_priority_value()))
        .sum();
    let extra = (available - fixed - fill_minimum).max(0.0);

    regions
        .iter()
        .map(|region| {
            if region.fill_priority_value() == 0 {
                region.extent()
            } else if fill_weight == 0 {
                region.min_size_value()
            } else {
                region.min_size_value()
                    + extra * f32::from(region.fill_priority_value()) / fill_weight as f32
            }
        })
        .collect()
}

fn set_region(
    regions: &mut [RegionRect],
    id: &RegionId,
    logical: LogicalRect,
    overlay: bool,
    scale_factor: f32,
) {
    let Some(region) = regions.iter_mut().find(|region| &region.id == id) else {
        return;
    };
    region.logical = logical;
    region.physical = physical_rect(logical, scale_factor);
    region.visible = true;
    region.overlay = overlay;
}

fn finite_non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn physical_dimension(logical: f32, scale_factor: f32) -> u32 {
    (logical * scale_factor).round().clamp(0.0, u32::MAX as f32) as u32
}

fn physical_rect(logical: LogicalRect, scale_factor: f32) -> PhysicalRect {
    PhysicalRect {
        x: physical_dimension(logical.x, scale_factor),
        y: physical_dimension(logical.y, scale_factor),
        width: physical_dimension(logical.width, scale_factor),
        height: physical_dimension(logical.height, scale_factor),
    }
}

#[cfg(test)]
mod tests {
    use super::{LogicalRect, TITLE_BAR_HEIGHT, WorkspaceGeometry};
    use crate::layout::{
        NarrowBehavior, RegionId, RegionPlacement, RegionRole, RegionState, WorkspaceLayout,
    };

    #[test]
    fn geometry_maps_standard_regions_to_logical_and_physical_pixels() {
        let geometry = WorkspaceGeometry::new(&WorkspaceLayout::default(), 1440.0, 900.0, 2.0);

        assert_eq!(geometry.logical_size, (1440.0, 900.0));
        assert_eq!(geometry.physical_size, (2880, 1800));
        assert_eq!(region(&geometry, &RegionId::Resources).logical.width, 240.0);
        assert_eq!(region(&geometry, &RegionId::Resources).physical.width, 480);
        assert_eq!(
            region(&geometry, &RegionId::GlobalNavigation).logical.y,
            TITLE_BAR_HEIGHT
        );
        assert!(region(&geometry, &RegionId::Primary).logical.width > 0.0);
        assert!(region(&geometry, &RegionId::Inspector).logical.width > 0.0);
        assert_eq!(
            region(&geometry, &RegionId::PrimaryToolbar).logical.width,
            region(&geometry, &RegionId::Primary).logical.width
        );
    }

    #[test]
    fn geometry_supports_additional_start_and_workspace_bottom_regions() {
        let pull_requests = RegionId::custom("pull-requests");
        let status = RegionId::custom("status");
        let layout = WorkspaceLayout::new([
            RegionState::new(RegionId::Resources, RegionRole::Resources).size(240.0),
            RegionState::new(pull_requests.clone(), RegionRole::SectionNavigation).size(230.0),
            RegionState::new(RegionId::Primary, RegionRole::Primary),
            RegionState::new(status.clone(), RegionRole::Utility)
                .placement(RegionPlacement::Bottom)
                .size(32.0),
        ])
        .expect("dynamic layout");
        let geometry = WorkspaceGeometry::new(&layout, 1200.0, 800.0, 1.0);

        assert_eq!(region(&geometry, &pull_requests).logical.x, 240.0);
        assert_eq!(region(&geometry, &status).logical.width, 1200.0);
        assert_eq!(region(&geometry, &status).logical.height, 32.0);
    }

    #[test]
    fn geometry_marks_responsive_overlay_without_consuming_primary_space() {
        let inspector = RegionState::new(RegionId::Inspector, RegionRole::Inspector)
            .size(240.0)
            .narrow_behavior(NarrowBehavior::Overlay)
            .collapse_below(900.0);
        let layout = WorkspaceLayout::new([
            RegionState::new(RegionId::Primary, RegionRole::Primary),
            inspector,
        ])
        .expect("responsive layout");
        let geometry = WorkspaceGeometry::new(&layout, 800.0, 600.0, 1.0);

        assert!(region(&geometry, &RegionId::Inspector).overlay);
        assert_eq!(region(&geometry, &RegionId::Primary).logical.width, 800.0);
    }

    #[test]
    fn geometry_reclaims_collapsed_regions_and_sanitizes_scale() {
        let mut layout = WorkspaceLayout::default();
        layout.toggle_collapsed(&RegionId::Resources);
        layout.toggle_collapsed(&RegionId::Inspector);
        layout.toggle_collapsed(&RegionId::Diagnostics);

        let geometry = WorkspaceGeometry::new(&layout, f32::NAN, -1.0, 0.0);
        assert_eq!(geometry.logical_size, (0.0, 0.0));
        assert_eq!(geometry.scale_factor, 1.0);
        assert!(!region(&geometry, &RegionId::Resources).visible);
        assert!(!region(&geometry, &RegionId::Inspector).visible);
        assert!(!region(&geometry, &RegionId::Diagnostics).visible);
    }

    #[test]
    fn logical_rect_never_exposes_negative_dimensions() {
        let rect = LogicalRect::new(10.0, 20.0, -4.0, -8.0);
        assert_eq!(rect.width, 0.0);
        assert_eq!(rect.height, 0.0);
    }

    fn region<'a>(geometry: &'a WorkspaceGeometry, id: &RegionId) -> &'a super::RegionRect {
        geometry.region(id).expect("registered region geometry")
    }
}
