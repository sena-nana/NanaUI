use nana_ui_core::Icon;

pub use nana_ui_core::{IconGeometry, IconPathCommand, IconShape};

/// Backend-neutral geometry for Nana's 24×24 line icons.
///
/// Geometry lives on [`Icon`]; this wrapper keeps scene/painter call sites
/// stable without a central match table.
pub fn icon_geometry(icon: Icon) -> IconGeometry {
    icon.geometry()
}
