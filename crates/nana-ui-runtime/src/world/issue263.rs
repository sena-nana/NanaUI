//! Issue #263: replaced content reaches layout only through the natural
//! size its resource reports, and only where a box reads it.
//!
//! One document of filler cards, then a media column: an image with no
//! size of its own, a fixed 300x200 image, an image 400 wide whose height
//! follows its aspect ratio, a fixed video and a host texture with no size;
//! then a hundred nodes showing one shared image, half fixed and half not.
//!
//! - Gate A: 1,200 video frames lay nothing out and bump no intrinsic
//!   generation.
//! - Gate B: an image's natural size arrives: the image with no size lays
//!   out again and nothing else does; the fixed image paints and lays
//!   nothing out; the 400-wide one takes its height from the ratio.
//! - Gate C: a shared image reaches its hundred nodes through the resource
//!   index, the same in 10k and 100k documents; only the fifty that read the
//!   size lay out again.
//! - Gate D: fit and sampling changes lay nothing out.
//! - Gate E: a video's resolution changes a hundred times under an explicit
//!   size: nothing above it reflows.

#![cfg(test)]

use std::sync::Arc;

use nana_ui_core::{
    BackgroundImage, BackgroundImageFit, ContentFit, ImageSampling, LayoutStyle, LengthSpec,
    WorkCounters,
};

use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, column, product_frame, styled,
};
use super::{DocumentId, StableNodeId, UiWorld};
use crate::layout_engine::verify::skip_layout_verify;
use crate::{
    AppContext, CustomRenderNode, HOST_TEXTURE_RENDERER, LayoutBox, LayoutViewport, MutationQueue,
    NanaTextEngineShaper, ReplacedMetadata, ReplacedResource,
};

const SHARED: u64 = 100;

fn viewport() -> LayoutViewport {
    LayoutViewport::new(800.0, 600.0)
}

pub(super) fn url(name: &str) -> ReplacedResource {
    ReplacedResource::Url(Arc::from(format!("asset://{name}.png")))
}

fn video_resource() -> ReplacedResource {
    ReplacedResource::Render {
        renderer: Arc::from(HOST_TEXTURE_RENDERER),
        resource: Arc::from("video:1"),
    }
}

fn texture_resource() -> ReplacedResource {
    ReplacedResource::Render {
        renderer: Arc::from(HOST_TEXTURE_RENDERER),
        resource: Arc::from("slot:7"),
    }
}

pub(super) fn image_layout(name: &str, width: Option<f32>, height: Option<f32>) -> LayoutStyle {
    let mut layout = LayoutStyle {
        width: width.map(LengthSpec::Px),
        height: height.map(LengthSpec::Px),
        ..LayoutStyle::default()
    };
    layout.paint.content_image = Some(BackgroundImage::url(format!("asset://{name}.png")));
    layout
}

/// The nodes the gates name.
struct Parts {
    hero: StableNodeId,
    banner: StableNodeId,
    wide: StableNodeId,
    video: StableNodeId,
    texture: StableNodeId,
    shared_fixed: Vec<StableNodeId>,
    shared_auto: Vec<StableNodeId>,
}

struct Media {
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
    parts: Parts,
}

impl Media {
    /// About `nodes` nodes, with `reported` natural sizes in place before the
    /// first layout.
    fn new(nodes: u64, reported: &[(ReplacedResource, ReplacedMetadata)]) -> Self {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        // No direction of their own, unlike the shared filler's row cards.
        let fixed = |width: f32, height: f32| LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        };
        // The filler first: growth at the end of the page moves nothing after it.
        let groups = (nodes.saturating_sub(10 + SHARED) / 21).max(1);
        for _ in 0..groups {
            let group = b.element(page, column(None));
            for _ in 0..10 {
                let card = b.element(group, fixed(200.0, 40.0));
                b.label(card, "one");
            }
        }
        let media = b.element(page, column(None));
        let hero = b.element(media, image_layout("hero", None, None));
        let banner = b.element(media, image_layout("banner", Some(300.0), Some(200.0)));
        let wide = b.element(media, image_layout("wide", Some(400.0), None));
        let video = b.element(media, fixed(320.0, 180.0));
        let texture = b.element(media, LayoutStyle::default());
        let shared = b.element(page, column(None));
        let mut shared_fixed = Vec::new();
        let mut shared_auto = Vec::new();
        for index in 0..SHARED {
            if index % 2 == 0 {
                shared_fixed
                    .push(b.element(shared, image_layout("shared", Some(64.0), Some(64.0))));
            } else {
                shared_auto.push(b.element(shared, image_layout("shared", None, None)));
            }
        }
        b.queue.set_custom_render(
            video,
            Some(CustomRenderNode::new(HOST_TEXTURE_RENDERER, "video:1", 0)),
        );
        b.queue.set_custom_render(
            texture,
            Some(CustomRenderNode::new(HOST_TEXTURE_RENDERER, "slot:7", 0)),
        );
        for (resource, metadata) in reported {
            b.queue
                .set_replaced_metadata(resource.clone(), Some(*metadata));
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut media = Self {
            context,
            document,
            shaper: bundled_face_shaper(),
            parts: Parts {
                hero,
                banner,
                wide,
                video,
                texture,
                shared_fixed,
                shared_auto,
            },
        };
        media.frame();
        media
    }

    fn world(&self) -> &UiWorld {
        self.context.world()
    }

    fn frame(&mut self) -> WorkCounters {
        product_frame(
            &mut self.context,
            self.document,
            viewport(),
            &mut self.shaper,
        )
    }

    fn commit(&mut self, edit: impl FnOnce(&mut MutationQueue)) {
        let mut queue = MutationQueue::new();
        edit(&mut queue);
        self.context.commit_mutations(queue).unwrap();
    }

    fn report(&mut self, resource: ReplacedResource, metadata: ReplacedMetadata) {
        self.commit(|queue| queue.set_replaced_metadata(resource, Some(metadata)));
    }
}

/// What a frame cost layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Cost {
    seeds: usize,
    frontier_measure: usize,
    frontier_placement: usize,
    measured: usize,
    placed: usize,
    full_fallbacks: usize,
}

impl From<WorkCounters> for Cost {
    fn from(counters: WorkCounters) -> Self {
        Self {
            seeds: counters.layout_frontier_seeds,
            frontier_measure: counters.layout_frontier_nodes_measure,
            frontier_placement: counters.layout_frontier_nodes_placement,
            measured: counters.layout_measure_nodes,
            placed: counters.layout_placement_nodes,
            full_fallbacks: counters.layout_full_document_fallbacks,
        }
    }
}

/// Gate A. 1,200 frames of the video and of the texture: paint, never layout.
#[test]
fn issue263_video_frames_lay_nothing_out() {
    let _unguarded = skip_layout_verify();
    let mut media = Media::new(10_000, &[]);
    let (video, texture) = (media.parts.video, media.parts.texture);
    for frame in 1..=1_200u64 {
        media.commit(|queue| {
            queue.set_custom_render(
                video,
                Some(CustomRenderNode::new(
                    HOST_TEXTURE_RENDERER,
                    "video:1",
                    frame,
                )),
            );
            queue.set_custom_render(
                texture,
                Some(CustomRenderNode::new(
                    HOST_TEXTURE_RENDERER,
                    "slot:7",
                    frame,
                )),
            );
        });
        let counters = media.frame();
        assert_eq!(Cost::from(counters), Cost::default(), "frame {frame}");
        assert_eq!(counters.replaced_content_updates, 2, "frame {frame}");
        assert_eq!(counters.replaced_content_only_layout_invalidations, 0);
        assert_eq!(counters.intrinsic_generation_bumps, 0, "frame {frame}");
    }
}

/// Gate B. Natural sizes arrive one image at a time.
#[test]
fn issue263_an_image_natural_size_lays_out_only_its_box() {
    let _unguarded = skip_layout_verify();
    let mut media = Media::new(10_000, &[]);
    let parts = (media.parts.hero, media.parts.banner, media.parts.wide);
    let (hero, banner, wide) = parts;

    // The image with no size of its own takes 1920x1080.
    media.report(url("hero"), ReplacedMetadata::new(1920.0, 1080.0));
    let counters = media.frame();
    assert_eq!(counters.replaced_intrinsic_metadata_updates, 1);
    assert_eq!(counters.resource_intrinsic_dependents_notified, 1);
    assert_eq!(counters.replaced_layout_seeds, 1);
    assert_eq!(counters.layout_frontier_seeds, 1);
    assert_eq!(counters.layout_full_document_fallbacks, 0);
    let hero_box = media.world().layout_box(hero).unwrap();
    assert_eq!((hero_box.width, hero_box.height), (1920.0, 1080.0));

    // The fixed one paints with it and lays nothing out.
    let banner_box = media.world().layout_box(banner);
    media.report(url("banner"), ReplacedMetadata::new(4000.0, 3000.0));
    let work = media.context.take_system_work();
    assert!(work.layout_frontier_seeds.is_empty());
    assert_eq!(work.render_extraction, vec![banner]);
    assert_eq!(media.world().layout_box(banner), banner_box);

    // The 400-wide one takes its height from the aspect ratio.
    media.report(url("wide"), ReplacedMetadata::new(800.0, 300.0));
    let counters = media.frame();
    assert_eq!(counters.replaced_layout_seeds, 1);
    let wide_box = media.world().layout_box(wide).unwrap();
    assert_eq!((wide_box.width, wide_box.height), (400.0, 150.0));

    let mut cold = Media::new(
        10_000,
        &[
            (url("hero"), ReplacedMetadata::new(1920.0, 1080.0)),
            (url("banner"), ReplacedMetadata::new(4000.0, 3000.0)),
            (url("wide"), ReplacedMetadata::new(800.0, 300.0)),
        ],
    );
    assert_matches_cold(&mut media.context, &mut cold.context, media.document);
}

/// Gate C. One image shown by a hundred nodes: the index reaches the
/// hundred, the same in a 10k and a 100k document, and only the fifty that
/// read the size lay out again.
#[test]
fn issue263_a_shared_image_reaches_its_dependents_through_the_index() {
    let _unguarded = skip_layout_verify();
    let mut costs = Vec::new();
    for nodes in [10_000, 100_000] {
        let mut media = Media::new(nodes, &[]);
        assert!(media.world().len() as u64 >= nodes * 98 / 100);
        media.report(url("shared"), ReplacedMetadata::new(48.0, 32.0));
        let counters = media.frame();
        assert_eq!(
            counters.resource_intrinsic_dependents_notified,
            SHARED as usize
        );
        assert_eq!(counters.replaced_layout_seeds, SHARED as usize / 2);
        assert_eq!(counters.layout_full_document_fallbacks, 0);
        for id in &media.parts.shared_fixed {
            assert_eq!(
                media
                    .world()
                    .layout_box(*id)
                    .map(|layout| (layout.width, layout.height)),
                Some((64.0, 64.0))
            );
        }
        for id in &media.parts.shared_auto {
            assert_eq!(
                media
                    .world()
                    .layout_box(*id)
                    .map(|layout| (layout.width, layout.height)),
                Some((48.0, 32.0))
            );
        }
        costs.push(Cost::from(counters));
    }
    assert_eq!(costs[0], costs[1]);
}

/// Gate D. Fit and sampling are paint.
#[test]
fn issue263_fit_and_sampling_lay_nothing_out() {
    let mut media = Media::new(
        1_000,
        &[(url("banner"), ReplacedMetadata::new(600.0, 400.0))],
    );
    let (banner, video) = (media.parts.banner, media.parts.video);
    let mut layout = image_layout("banner", Some(300.0), Some(200.0));
    layout.paint.object_fit = Some(BackgroundImageFit::Cover);
    media.commit(|queue| {
        queue.set_style(banner, styled(layout));
        queue.set_custom_render(
            video,
            Some(
                CustomRenderNode::new(HOST_TEXTURE_RENDERER, "video:1", 0)
                    .with_fit(ContentFit::Contain)
                    .with_sampling(ImageSampling::Mipmap),
            ),
        );
    });
    let counters = media.frame();
    assert_eq!(Cost::from(counters), Cost::default());
    assert_eq!(counters.replaced_content_only_layout_invalidations, 0);
}

/// Gate E. A video changes resolution a hundred times under an explicit
/// size: it repaints, and nothing reflows.
#[test]
fn issue263_a_resolution_storm_under_an_explicit_size_reflows_nothing() {
    let mut media = Media::new(1_000, &[]);
    let video = media.parts.video;
    let before = media.world().layout_box(video);
    for step in 0..100u32 {
        let width = 640.0 + 16.0 * step as f32;
        media.report(
            video_resource(),
            ReplacedMetadata::new(width, width * 9.0 / 16.0),
        );
        let work = media.context.take_system_work();
        assert!(work.layout_frontier_seeds.is_empty(), "step {step}");
        assert_eq!(work.render_extraction, vec![video], "step {step}");
    }
    assert_eq!(media.world().layout_box(video), before);
}

/// A texture with no size of its own takes its resolution, then a new one;
/// each pass is checked against a full layout and the end against the same
/// inputs laid out once.
#[test]
fn issue263_natural_sizes_match_a_full_layout_every_pass() {
    let mut media = Media::new(400, &[]);
    for (width, height) in [(256.0, 128.0), (512.0, 512.0), (64.0, 32.0)] {
        media.report(texture_resource(), ReplacedMetadata::new(width, height));
        media.report(
            url("shared"),
            ReplacedMetadata::new(width / 4.0, height / 4.0),
        );
        media.frame();
        let texture: Option<LayoutBox> = media.world().layout_box(media.parts.texture);
        assert_eq!(
            texture.map(|layout| (layout.width, layout.height)),
            Some((width, height))
        );
    }
    let mut cold = Media::new(
        400,
        &[
            (texture_resource(), ReplacedMetadata::new(64.0, 32.0)),
            (url("shared"), ReplacedMetadata::new(16.0, 8.0)),
        ],
    );
    assert_matches_cold(&mut media.context, &mut cold.context, media.document);
}

/// A host reports only what a node shows: a resource is shown while one
/// does, through the same index the reports travel.
#[test]
fn issue263_a_resource_is_shown_while_a_node_shows_it() {
    let mut media = Media::new(400, &[]);
    assert!(media.world().shows_replaced(&url("hero")));
    assert!(media.world().shows_replaced(&url("shared")));
    assert!(media.world().shows_replaced(&video_resource()));
    assert!(!media.world().shows_replaced(&url("elsewhere")));
    let hero = media.parts.hero;
    media.commit(|queue| queue.set_style(hero, styled(LayoutStyle::default())));
    assert!(!media.world().shows_replaced(&url("hero")));
}

/// A markdown image that resolves its size makes the document taller: the
/// block list it projects moved, and layout hears of it.
#[cfg(feature = "rich-text")]
#[test]
fn issue263_a_markdown_image_resolving_lays_its_document_out_again() {
    let document = DocumentId::new(1).unwrap();
    let build = |resolved: bool| {
        let mut context = AppContext::new();
        let root = context
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        let mut markdown =
            crate::NativeMarkdown::from_source("before\n\n![picture](image.png)\n\nafter");
        if resolved {
            markdown.resolve_image("image.png", "data:image/png;base64,test", 200, 80);
        }
        let entity = context
            .create_detached_component(document, markdown)
            .unwrap();
        context.append_child(root, entity).unwrap();
        (context, entity)
    };
    let mut shaper = bundled_face_shaper();
    let (mut context, entity) = build(false);
    product_frame(&mut context, document, viewport(), &mut shaper);
    let before = context.world().layout_box(entity.stable_id()).unwrap();
    context
        .update_component(entity, |markdown, _| {
            markdown.resolve_image("image.png", "data:image/png;base64,test", 200, 80)
        })
        .unwrap();
    let counters = product_frame(&mut context, document, viewport(), &mut shaper);
    assert!(counters.layout_frontier_seeds > 0);
    let after = context.world().layout_box(entity.stable_id()).unwrap();
    assert!(after.height > before.height, "{before:?} -> {after:?}");
    let (mut cold, _) = build(true);
    product_frame(&mut cold, document, viewport(), &mut shaper);
    assert_matches_cold(&mut context, &mut cold, document);
}
