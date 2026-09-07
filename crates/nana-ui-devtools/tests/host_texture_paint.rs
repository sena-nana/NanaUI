#![cfg(feature = "runtime-agent")]
//! Avatar / Thumbnail are host-texture slots. Asserting that a binding lands in
//! the component (or that its accessibility box exists) does not prove the
//! texture is ever sampled — a Scene revision that never advances paints the
//! placeholder forever while every structural assertion stays green. These
//! tests therefore assert on painted pixels.

use nana_ui::runtime::{Avatar, DocumentId, GpuTextureView, RuntimeDocument, Thumbnail};
use nana_ui::{HostTexture, HostTextureAlphaMode, HostTextureRegistry};
use nana_ui_devtools::offscreen::{self, OffscreenSnapshots, Size};

const SLOT: &str = "test.cover";
const W: u32 = 96;
const H: u32 = 96;
/// Opaque red, distinct from every semantic surface colour in both themes.
const FILL: [u8; 4] = [255, 0, 0, 255];

/// A 1×1 opaque red texture registered under [`SLOT`].
fn register_fill(gpu: &OffscreenSnapshots, registry: &HostTextureRegistry) {
    let texture = gpu
        .device
        .create_texture(&nana_ui::wgpu::TextureDescriptor {
            label: Some("host texture paint evidence"),
            size: nana_ui::wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: nana_ui::wgpu::TextureDimension::D2,
            format: nana_ui::wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: nana_ui::wgpu::TextureUsages::TEXTURE_BINDING
                | nana_ui::wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
    gpu.queue.write_texture(
        nana_ui::wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: nana_ui::wgpu::Origin3d::ZERO,
            aspect: nana_ui::wgpu::TextureAspect::All,
        },
        &FILL,
        nana_ui::wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        nana_ui::wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&nana_ui::wgpu::TextureViewDescriptor::default());
    registry.register(
        SLOT,
        HostTexture::from_wgpu(1, 1, view),
        1,
        1,
        HostTextureAlphaMode::Opaque,
    );
}

/// Fraction of pixels close to [`FILL`].
fn fill_ratio(pixels: &[u8]) -> f32 {
    let (chunks, _) = pixels.as_chunks::<4>();
    let hits = chunks
        .iter()
        .filter(|p| p[0] > 200 && p[1] < 80 && p[2] < 80)
        .count();
    hits as f32 / chunks.len().max(1) as f32
}

/// One device and one painter throughout. The Scene revision only governs the
/// painter's cross-frame host-texture cache, so a fresh session per paint would
/// re-extract cold and pass no matter what the revision does — that is exactly
/// the false green this file exists to avoid.
fn paint_with(
    gpu: &mut OffscreenSnapshots,
    scene: &nana_ui::runtime::host::UiScene,
    registry: &HostTextureRegistry,
) -> Vec<u8> {
    gpu.paint_layers_scaled(
        &[(scene, true)],
        Size::new(W, H),
        1.0,
        [1.0, 1.0, 1.0, 1.0],
        Some(registry),
        None,
    )
    .expect("paint")
}

#[test]
fn avatar_bound_after_mount_reaches_the_screen() {
    let Some(mut gpu) = offscreen::optional() else {
        return;
    };
    let registry = HostTextureRegistry::new();
    register_fill(&gpu, &registry);

    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    let avatar = document
        .context_mut()
        .create_component(id, Avatar::new("").size(80.0))
        .unwrap();
    let mut session =
        nana_ui_devtools::agent::RuntimeAgentSession::new(document, W, H).expect("session");
    session.flush().expect("flush");

    let before = paint_with(&mut gpu, &session.document().scene().clone(), &registry);
    assert!(
        fill_ratio(&before) < 0.01,
        "nothing is bound yet, so nothing may sample the texture"
    );

    // Same painter, same session: this is the frame where the revision decides
    // whether the cached (empty) sample is reused or the new texture is read.
    session
        .document_mut()
        .context_mut()
        .update_component(avatar, |view, _| {
            view.resource = SLOT.into();
            view.replace_view(1);
            view.version = 0;
        })
        .unwrap();
    session.flush().expect("flush");

    let after = paint_with(&mut gpu, &session.document().scene().clone(), &registry);
    assert!(
        fill_ratio(&after) > 0.2,
        "a texture bound after mount must reach the screen; with a fixed Scene          revision the painter keeps the empty sample while every structural          assertion stays green (fill={})",
        fill_ratio(&after)
    );
}

#[test]
fn thumbnail_bound_after_mount_reaches_the_screen() {
    let Some(mut gpu) = offscreen::optional() else {
        return;
    };
    let registry = HostTextureRegistry::new();
    register_fill(&gpu, &registry);

    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    let thumbnail = document
        .context_mut()
        .create_component(id, Thumbnail::loading())
        .unwrap();
    let mut session =
        nana_ui_devtools::agent::RuntimeAgentSession::new(document, W, H).expect("session");
    session.flush().expect("flush");

    let before = paint_with(&mut gpu, &session.document().scene().clone(), &registry);
    assert!(
        fill_ratio(&before) < 0.01,
        "a loading thumbnail must not paint the texture colour"
    );

    session
        .document_mut()
        .context_mut()
        .update_component(thumbnail, |view, _| {
            view.resource = SLOT.into();
            view.state = nana_ui::runtime::ThumbnailState::Ready;
            view.replace_view(1);
            view.version = 0;
        })
        .unwrap();
    session.flush().expect("flush");

    let after = paint_with(&mut gpu, &session.document().scene().clone(), &registry);
    assert!(
        fill_ratio(&after) > 0.05,
        "a thumbnail bound after mount must reach the screen (fill={})",
        fill_ratio(&after)
    );
}

/// The revision is the scene's conflict key: every view of one host-texture
/// slot in a frame must carry the same one, or `frame_graph` rejects the whole
/// frame with `ConflictingExternalResource` and nothing paints. A component
/// that hardcodes revision 0 therefore cannot share a slot with a
/// `GpuTextureView` the host has re-bound — which is what a product does when
/// the same cover appears both as an avatar and as a plain texture view.
#[test]
fn avatar_and_texture_view_can_share_one_slot() {
    let Some(mut gpu) = offscreen::optional() else {
        return;
    };
    let registry = HostTextureRegistry::new();
    register_fill(&gpu, &registry);

    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    let (avatar, view) = {
        let cx = document.context_mut();
        let avatar = cx.create_component(id, Avatar::new("").size(40.0)).unwrap();
        let view = cx.create_component(id, GpuTextureView::new("")).unwrap();
        (avatar, view)
    };
    let mut session =
        nana_ui_devtools::agent::RuntimeAgentSession::new(document, W, H).expect("session");
    session.flush().expect("flush");

    // Host delivery: one texture, one generation, both views re-bound.
    const GENERATION: u64 = 4;
    session
        .document_mut()
        .context_mut()
        .update_component(avatar, |node, _| {
            node.resource = SLOT.into();
            node.replace_view(GENERATION);
            node.version = 0;
        })
        .unwrap();
    session
        .document_mut()
        .context_mut()
        .update_component(view, |node, _| {
            node.resource = SLOT.into();
            node.replace_view(GENERATION);
            node.version = 0;
        })
        .unwrap();
    session.flush().expect("flush");

    let painted = paint_with(&mut gpu, &session.document().scene().clone(), &registry);
    assert!(
        fill_ratio(&painted) > 0.05,
        "both views must paint the shared texture (fill={})",
        fill_ratio(&painted)
    );
}
