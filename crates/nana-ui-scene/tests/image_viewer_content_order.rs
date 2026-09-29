#![cfg(feature = "image-viewer")]

use nana_ui_runtime::{
    DocumentId, ImageViewer, ImageViewerContent, ImageViewerOffset, LayoutViewport,
    MeasureTextShaper,
};
use nana_ui_scene::{RuntimeDocument, ScenePrimitiveKind};

#[test]
fn image_viewer_texture_is_above_backdrop_and_below_controls_with_stage_clipping() {
    let document = DocumentId::new(770).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let viewer = runtime
        .context_mut()
        .create_component(
            document,
            ImageViewer::new(ImageViewerContent::host_texture("preview"))
                .name("Image")
                .metadata("PNG"),
        )
        .unwrap();
    runtime.context_mut().assemble_image_viewer(viewer).unwrap();
    let viewport = LayoutViewport::new(800.0, 600.0);
    runtime.flush(viewport, &mut MeasureTextShaper).unwrap();
    for zoom in [1.0, 2.0] {
        runtime
            .context_mut()
            .update_component(viewer, |viewer, _| {
                viewer.zoom = zoom;
                viewer.offset = ImageViewerOffset::new(10.0, 12.0);
            })
            .unwrap();
        runtime.flush(viewport, &mut MeasureTextShaper).unwrap();
        let bounds = runtime
            .context()
            .world()
            .layout_box(viewer.stable_id())
            .unwrap();
        let geometry = runtime
            .context()
            .read(viewer, |viewer| {
                viewer.geometry(bounds, nana_ui_core::UI_METRICS)
            })
            .unwrap();
        let primitives = runtime
            .scene()
            .primitives()
            .filter(|primitive| primitive.node == viewer.stable_id())
            .collect::<Vec<_>>();
        let custom = primitives
            .iter()
            .enumerate()
            .filter(|(_, primitive)| matches!(primitive.kind, ScenePrimitiveKind::Custom { .. }))
            .collect::<Vec<_>>();
        assert_eq!(custom.len(), 1);
        let (index, image) = custom[0];
        assert_eq!(
            (
                image.bounds.x,
                image.bounds.y,
                image.bounds.width,
                image.bounds.height
            ),
            (
                geometry.content.x,
                geometry.content.y,
                geometry.content.width,
                geometry.content.height
            )
        );
        assert!(image.clips.iter().any(|clip| (
            clip.bounds.x,
            clip.bounds.y,
            clip.bounds.width,
            clip.bounds.height
        ) == (
            geometry.stage.x,
            geometry.stage.y,
            geometry.stage.width,
            geometry.stage.height
        )));
        let backgrounds = primitives
            .iter()
            .enumerate()
            .filter(|(_, primitive)| {
                matches!(&primitive.kind,
            ScenePrimitiveKind::Quad { background: Some(color), .. } if color[3] > 0.0)
            })
            .collect::<Vec<_>>();
        assert!(backgrounds.len() >= 3);
        assert!(
            backgrounds
                .iter()
                .all(|(background, _)| *background < index)
        );
        let captions = primitives
            .iter()
            .enumerate()
            .filter(|(_, primitive)| matches!(primitive.kind, ScenePrimitiveKind::Text { .. }))
            .collect::<Vec<_>>();
        assert_eq!(captions.len(), 2);
        assert!(captions.iter().all(|(caption, _)| *caption > index));
        // The close control is a child the viewer keeps after its content:
        // everything it paints comes after the image.
        let close = runtime
            .context()
            .world()
            .node(viewer.stable_id())
            .unwrap()
            .children
            .last()
            .copied()
            .expect("the viewer assembles its close control");
        let scene = runtime.scene().primitives().collect::<Vec<_>>();
        let image_at = scene
            .iter()
            .position(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom { .. }))
            .unwrap();
        let close_at = scene
            .iter()
            .enumerate()
            .filter(|(_, primitive)| primitive.node == close)
            .map(|(at, _)| at)
            .collect::<Vec<_>>();
        assert!(!close_at.is_empty());
        assert!(close_at.iter().all(|at| *at > image_at));
    }
    runtime
        .context_mut()
        .update_component(viewer, |viewer, _| {
            viewer.content = ImageViewerContent::None
        })
        .unwrap();
    runtime.flush(viewport, &mut MeasureTextShaper).unwrap();
    assert!(
        !runtime
            .scene()
            .primitives()
            .any(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom { .. }))
    );
}

#[test]
fn image_viewer_scene_receives_intrinsic_dimensions_from_retained_projection() {
    let document = DocumentId::new(771).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let viewer = runtime
        .context_mut()
        .create_component(
            document,
            ImageViewer::new(ImageViewerContent::host_texture("preview")).intrinsic_size(72, 40),
        )
        .unwrap();
    runtime
        .flush(LayoutViewport::new(800.0, 600.0), &mut MeasureTextShaper)
        .unwrap();
    let image = runtime
        .scene()
        .primitives()
        .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom { .. }))
        .unwrap();
    assert_eq!((image.bounds.width, image.bounds.height), (72.0, 40.0));
    runtime
        .context_mut()
        .update_component(viewer, |viewer, _| {
            viewer.intrinsic_size = Some((4000, 1000))
        })
        .unwrap();
    runtime
        .flush(LayoutViewport::new(800.0, 600.0), &mut MeasureTextShaper)
        .unwrap();
    let image = runtime
        .scene()
        .primitives()
        .find(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Custom { .. }))
        .unwrap();
    assert!((image.bounds.width / image.bounds.height - 4.0).abs() < 0.001);
    assert!(image.bounds.width <= 800.0 && image.bounds.height <= 600.0);
}

/// sRGB to linear light, the space the painter blends in.
fn linear(u: f32) -> f32 {
    if u < 0.04045 {
        u / 12.92
    } else {
        ((u + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light back to sRGB, what the window shows.
fn encode(u: f32) -> f32 {
    if u <= 0.003_130_8 {
        u * 12.92
    } else {
        1.055 * u.powf(1.0 / 2.4) - 0.055
    }
}

/// The scrim behind an open viewer shows near-black over the page in both
/// themes, as the painter composites it: in linear light, over the theme's
/// page background.
#[test]
fn the_viewer_scrim_shows_near_black_over_the_page_in_both_themes() {
    use nana_ui_core::ThemeMode;
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        let document = DocumentId::new(771).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        runtime.context_mut().set_theme(mode).unwrap();
        let viewer = runtime
            .context_mut()
            .create_component(document, ImageViewer::new(ImageViewerContent::None))
            .unwrap();
        runtime.context_mut().assemble_image_viewer(viewer).unwrap();
        runtime
            .flush(LayoutViewport::new(800.0, 600.0), &mut MeasureTextShaper)
            .unwrap();
        let page = runtime
            .context()
            .world()
            .theme()
            .palette()
            .background
            .as_rgba_array();
        let scrim = runtime
            .scene()
            .primitives()
            .find(|primitive| primitive.node == viewer.stable_id() && primitive.id.slot == 10)
            .expect("the scrim paints");
        let ScenePrimitiveKind::Quad {
            background: Some([r, g, b, alpha]),
            ..
        } = scrim.kind
        else {
            panic!("the scrim is a filled quad: {:?}", scrim.kind);
        };
        let alpha = alpha * scrim.opacity;
        let shown = [r, g, b]
            .iter()
            .zip(&page[..3])
            .map(|(scrim, page)| encode(linear(*scrim) * alpha + linear(*page) * (1.0 - alpha)))
            .fold(0.0f32, f32::max);
        assert!(
            shown < 0.15,
            "{mode:?}: the scrim shows as {shown:.3} over the page, not near-black"
        );
    }
}
