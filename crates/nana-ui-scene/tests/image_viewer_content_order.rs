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
            .read(viewer, |viewer| viewer.geometry(bounds))
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
        let controls = primitives
            .iter()
            .enumerate()
            .filter(|(_, primitive)| matches!(primitive.kind, ScenePrimitiveKind::Text { .. }))
            .collect::<Vec<_>>();
        assert!(controls.len() >= 3);
        assert!(controls.iter().all(|(control, _)| *control > index));
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
