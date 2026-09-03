//! image-viewer geometry from committed node data.

use super::*;

pub(in crate::world) fn image_viewer_geometry(
    bounds: LayoutBox,
    name: Option<&Arc<str>>,
    metadata: Option<&Arc<str>>,
    zoom: f32,
    offset_x: f32,
    offset_y: f32,
    palette: &SemanticPalette,
) -> crate::ComponentGeometry {
    let mut viewer = crate::ImageViewer::new(crate::ImageViewerContent::None);
    if let Some(name) = name {
        viewer = viewer.name(Arc::clone(name));
    }
    if let Some(metadata) = metadata {
        viewer = viewer.metadata(Arc::clone(metadata));
    }
    viewer.zoom = zoom;
    viewer.offset = crate::ImageViewerOffset::new(offset_x, offset_y);
    let geometry = viewer.geometry(bounds);
    let mut scrim = palette.background.as_rgba_array();
    scrim[3] = 0.94;
    let mut stage = palette.background.as_rgba_array();
    stage[3] = 0.34;
    crate::ComponentGeometry::ImageViewer {
        scrim: geometry.scrim,
        surface: geometry.surface,
        stage: geometry.stage,
        close: geometry.close,
        name: name
            .zip(geometry.name)
            .map(|(text, region)| crate::ComponentTextRegion {
                bounds: region,
                content: Arc::clone(text),
                color: Some(palette.text.as_rgba_array()),
                font_size: 12.0,
                font_weight: Some(600),
            }),
        metadata: metadata.zip(geometry.metadata).map(|(text, region)| {
            crate::ComponentTextRegion {
                bounds: region,
                content: Arc::clone(text),
                color: Some(palette.muted.as_rgba_array()),
                font_size: 11.0,
                font_weight: None,
            }
        }),
        content: geometry.content,
        scrim_color: scrim,
        surface_color: palette.surface.as_rgba_array(),
        stage_color: stage,
    }
}
