//! Issue #202: replaced content is one kind of layout item in every context.
//!
//! - Gate A: an image with no size of its own takes the natural size its
//!   resource reports, and one with only a width takes its height from the
//!   aspect ratio, as a flex item, a grid item, an atomic inline box and an
//!   absolutely positioned box alike. Each matches a cold layout.
//! - Gate B: replaced content aligns by its bottom edge: as an atom of a
//!   native paragraph (its margin box's, Gate G) and under `align-items:
//!   baseline` (its border box's), an image's and a custom renderer's bottom
//!   edge sits on the text's baseline.
//! - Gate C: an image, a video, a host texture, a GPU canvas and a bare
//!   custom-render anchor are one kind of layout item: built the way an
//!   application builds each, they are replaced content with the same
//!   participation in every context, and the same natural size gives each
//!   the same box. Each matches a cold layout.
//! - Gate D: min/max bound a natural size without bending its aspect ratio
//!   (CSS 2.1 §10.4), with no size, or one side, of the box's own.
//! - Gate E: invalidation follows the natural size and nothing else. The
//!   same size reported again, and a switch to another resource of the same
//!   size, lay nothing out; a switch to one of another size lays out the box
//!   that reads it.
//! - Gate F: markup gives a video and a host texture their fit the way it
//!   gives an image its fit: the cascaded `object-fit`, every keyword of it,
//!   ahead of the `fit` attribute. The painter then sizes all three with
//!   `ContentFit::fitted_size` (nana-ui `scene_paint/replaced_context_tests.rs`
//!   checks its `url()` path against the host-texture one).
//! - Gate G: atoms sharing a line of an ordinary block align by baseline
//!   (CSS 2.1 §10.8.1, `vertical-align: baseline`). An inline-block with no
//!   line box, one whose overflow is not visible and replaced content sit on
//!   the baseline by their bottom margin edge; an inline-block with text by
//!   its last line. The line box spans the highest top to the lowest bottom.
//!   A taller atom, text that wraps to another line in a box of the same
//!   size, and a line box arriving in an empty atom each place the line
//!   again, and each matches a cold layout.

#![cfg(test)]

use nana_ui_core::{
    AlignSpec, DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec, PositionSpec,
};

use super::issue263::{image_layout, url};
use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, product_frame, styled,
};
use std::sync::Arc;

use super::{DocumentId, StableNodeId, UiWorld};
use crate::{
    AppContext, CustomRenderNode, GPU_VIEW_RENDERER, GpuTextureView, GpuView,
    HOST_TEXTURE_RENDERER, InlineContent, InlineFlow, LayoutContentKind, LayoutViewport,
    MutationQueue, NanaTextEngineShaper, ParticipationKind as Part, ReplacedMetadata,
    ReplacedResource, Video,
};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(800.0, 600.0)
}

/// How a context's container and its replaced children are styled.
#[derive(Debug, Clone, Copy)]
enum Context {
    Flex,
    /// A flex row that stretches its items down the line.
    FlexStretch,
    Grid,
    /// A grid that stretches its items down their areas.
    GridStretch,
    Inline,
    Absolute,
}

impl Context {
    fn container(self) -> LayoutStyle {
        let mut layout = LayoutStyle {
            width: Some(LengthSpec::Px(600.0)),
            ..LayoutStyle::default()
        };
        match self {
            Self::Flex => {
                layout.display = Some(DisplaySpec::Flex);
                layout.direction = Some(FlexDirection::Row);
            }
            Self::FlexStretch => {
                layout.display = Some(DisplaySpec::Flex);
                layout.direction = Some(FlexDirection::Row);
                layout.align_items = AlignSpec::Stretch;
            }
            Self::Grid | Self::GridStretch => {
                layout.display = Some(DisplaySpec::Grid);
                layout.grid_columns = Some(vec![GridTrack::Auto, GridTrack::Auto]);
                if matches!(self, Self::GridStretch) {
                    layout.align_items = AlignSpec::Stretch;
                }
            }
            Self::Inline => layout.display = Some(DisplaySpec::Block),
            Self::Absolute => {
                layout.display = Some(DisplaySpec::Block);
                layout.position = PositionSpec::Relative;
                layout.height = Some(LengthSpec::Px(300.0));
            }
        }
        layout
    }

    fn item(self, mut layout: LayoutStyle, offset: f32) -> LayoutStyle {
        match self {
            Self::Flex | Self::FlexStretch | Self::Grid | Self::GridStretch => {}
            Self::Inline => layout.display = Some(DisplaySpec::InlineBlock),
            Self::Absolute => {
                layout.position = PositionSpec::Absolute;
                layout.offset_top = Some(LengthSpec::Px(offset));
                layout.offset_left = Some(LengthSpec::Px(offset));
            }
        }
        layout
    }

    fn participation(self) -> Part {
        match self {
            Self::Flex | Self::FlexStretch => Part::FlexItem,
            Self::Grid | Self::GridStretch => Part::GridItem,
            Self::Inline => Part::AtomicInline,
            Self::Absolute => Part::Absolute,
        }
    }
}

struct Page {
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
    /// Shows `pic` with no size of its own.
    auto: StableNodeId,
    /// Shows `pic` 60 wide; its height follows the aspect ratio.
    wide: StableNodeId,
}

impl Page {
    fn new(context: Context, reported: bool) -> Self {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        let container = b.element(page, context.container());
        let auto = b.element(
            container,
            context.item(image_layout("pic", None, None), 0.0),
        );
        let wide = b.element(
            container,
            context.item(image_layout("pic", Some(60.0), None), 100.0),
        );
        if reported {
            b.queue
                .set_replaced_metadata(url("pic"), Some(ReplacedMetadata::new(120.0, 80.0)));
        }
        let mut app = AppContext::new();
        app.commit_mutations(b.queue).unwrap();
        let mut page = Self {
            context: app,
            document,
            shaper: bundled_face_shaper(),
            auto,
            wide,
        };
        page.frame();
        page
    }

    fn world(&self) -> &UiWorld {
        self.context.world()
    }

    fn frame(&mut self) -> nana_ui_core::WorkCounters {
        product_frame(
            &mut self.context,
            self.document,
            viewport(),
            &mut self.shaper,
        )
    }

    fn size(&self, id: StableNodeId) -> (f32, f32) {
        let bounds = self.world().layout_box(id).unwrap();
        (bounds.width, bounds.height)
    }
}

/// Gate A.
#[test]
fn issue202_a_natural_size_reaches_every_context() {
    for context in [
        Context::Flex,
        Context::FlexStretch,
        Context::Grid,
        Context::GridStretch,
        Context::Inline,
        Context::Absolute,
    ] {
        let mut page = Page::new(context, false);
        for id in [page.auto, page.wide] {
            assert_eq!(
                page.world().layout_node(id).unwrap().participation,
                Some(context.participation()),
                "{context:?}"
            );
        }
        let mut queue = MutationQueue::new();
        queue.set_replaced_metadata(url("pic"), Some(ReplacedMetadata::new(120.0, 80.0)));
        page.context.commit_mutations(queue).unwrap();
        let work = page.frame();
        assert_eq!(work.replaced_layout_seeds, 2, "{context:?}");
        assert_eq!(work.layout_full_document_fallbacks, 0, "{context:?}");
        assert_eq!(page.size(page.auto), (120.0, 80.0), "{context:?}");
        assert_eq!(page.size(page.wide), (60.0, 40.0), "{context:?}");
        let mut cold = Page::new(context, true);
        assert_matches_cold(&mut page.context, &mut cold.context, page.document);
    }
}

/// Gate B.
#[test]
fn issue202_replaced_content_sits_on_the_text_baseline_by_its_bottom_edge() {
    let document = DocumentId::new(1).unwrap();
    let (mut b, page) = Builder::page(document, 1, 800.0);
    let fixed = |width: f32, height: f32| LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        ..LayoutStyle::default()
    };
    let render_anchor = |b: &mut Builder, id| {
        b.queue.set_custom_render(
            id,
            Some(CustomRenderNode::new("fixture-renderer", "anchor", 0)),
        );
    };
    // A flex row aligning by baseline.
    let row = b.element(
        page,
        LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            align_items: AlignSpec::Baseline,
            width: Some(LengthSpec::Px(600.0)),
            ..LayoutStyle::default()
        },
    );
    let row_text = b.label(row, "Baseline");
    let row_image = b.element(row, image_layout("tall", Some(20.0), Some(48.0)));
    let row_render = b.element(row, fixed(30.0, 36.0));
    render_anchor(&mut b, row_render);
    // The same two as atoms of a native paragraph.
    let line_image = b.detached(image_layout("tall", Some(20.0), Some(48.0)));
    let line_render = b.detached(fixed(30.0, 36.0));
    render_anchor(&mut b, line_render);
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    let paragraph = context
        .create_component(
            document,
            InlineFlow::new(
                InlineContent::builder()
                    .text("Baseline ")
                    .child(line_image)
                    .text(" and ")
                    .child(line_render)
                    .build(),
            )
            .width(LengthSpec::Px(600.0)),
        )
        .unwrap()
        .stable_id();
    let mut shaper = bundled_face_shaper();
    product_frame(&mut context, document, viewport(), &mut shaper);
    let world = context.world();
    // Text baselines are in document space.
    let baseline = |id| world.layout_result(id).unwrap().first_baseline.unwrap();
    for (text, replaced) in [
        (row_text, [row_image, row_render]),
        (paragraph, [line_image, line_render]),
    ] {
        let text_baseline = baseline(text);
        for id in replaced {
            assert_eq!(
                world.layout_node(id).unwrap().content,
                crate::LayoutContentKind::Replaced
            );
            let bounds = world.layout_box(id).unwrap();
            let bottom = bounds.y + bounds.height;
            assert!(
                (bottom - text_baseline).abs() < 0.01,
                "{id:?} bottom {bottom} vs text baseline {text_baseline}"
            );
        }
    }
    assert_eq!(
        world.layout_node(line_image).unwrap().participation,
        Some(Part::AtomicInline)
    );
}

/// The replaced kinds the issue names.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Image,
    Video,
    HostTexture,
    /// A GPU canvas: a `GpuView` render target.
    Canvas,
    /// A custom renderer's presentation anchor on a plain element.
    Anchor,
}

const KINDS: [Kind; 5] = [
    Kind::Image,
    Kind::Video,
    Kind::HostTexture,
    Kind::Canvas,
    Kind::Anchor,
];

impl Kind {
    fn resource(self) -> ReplacedResource {
        let render = |renderer: &str, resource: &str| ReplacedResource::Render {
            renderer: Arc::from(renderer),
            resource: Arc::from(resource),
        };
        match self {
            Self::Image => url("pic"),
            Self::Video => render(HOST_TEXTURE_RENDERER, "video:1"),
            Self::HostTexture => render(HOST_TEXTURE_RENDERER, "slot:7"),
            Self::Canvas => render(GPU_VIEW_RENDERER, "3"),
            Self::Anchor => render("fixture.anchor", "anchor"),
        }
    }

    /// Adds the node under `parent` as an application would, with `layout`:
    /// an image is a style, an anchor a custom render node on an element,
    /// and the rest are their components.
    fn build(
        self,
        b: &mut Builder,
        components: &mut Vec<(StableNodeId, Self, LayoutStyle)>,
        parent: StableNodeId,
        layout: LayoutStyle,
    ) -> Option<StableNodeId> {
        match self {
            Self::Image => {
                let mut layout = layout;
                layout.paint.content_image =
                    Some(nana_ui_core::BackgroundImage::url("asset://pic.png"));
                Some(b.element(parent, layout))
            }
            Self::Anchor => {
                let id = b.element(parent, layout);
                b.queue.set_custom_render(
                    id,
                    Some(CustomRenderNode::new("fixture.anchor", "anchor", 0)),
                );
                Some(id)
            }
            Self::Video | Self::HostTexture | Self::Canvas => {
                components.push((parent, self, layout));
                None
            }
        }
    }

    fn create(
        self,
        context: &mut AppContext,
        document: DocumentId,
        layout: LayoutStyle,
    ) -> StableNodeId {
        // The components fill their box unless told otherwise; `auto` asks
        // for the natural size, as an image's absent size does.
        let mut layout = layout;
        layout.width.get_or_insert(LengthSpec::Auto);
        layout.height.get_or_insert(LengthSpec::Auto);
        let style = styled(layout);
        match self {
            Self::Video => context
                .create_component(document, Video::new("video:1").style(style))
                .map(|entity| entity.stable_id()),
            Self::HostTexture => context
                .create_component(document, GpuTextureView::new("slot:7").style(style))
                .map(|entity| entity.stable_id()),
            Self::Canvas => context
                .create_component(document, GpuView::new(3).style(style))
                .map(|entity| entity.stable_id()),
            Self::Image | Self::Anchor => unreachable!("built as elements"),
        }
        .unwrap()
    }
}

/// Every kind under one container of `context`.
struct Gallery {
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
    nodes: Vec<(Kind, StableNodeId)>,
}

impl Gallery {
    fn new(context: Context, reported: Option<ReplacedMetadata>) -> Self {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        let container = b.element(page, context.container());
        if let Some(metadata) = reported {
            for kind in KINDS {
                b.queue
                    .set_replaced_metadata(kind.resource(), Some(metadata));
            }
        }
        let mut nodes = Vec::new();
        let mut components = Vec::new();
        for (index, kind) in KINDS.into_iter().enumerate() {
            let layout = context.item(LayoutStyle::default(), 130.0 * index as f32);
            if let Some(id) = kind.build(&mut b, &mut components, container, layout) {
                nodes.push((kind, id));
            }
        }
        let mut app = AppContext::new();
        app.commit_mutations(b.queue).unwrap();
        for (parent, kind, layout) in components {
            let id = kind.create(&mut app, document, layout);
            let mut queue = MutationQueue::new();
            queue.insert(parent, id, None);
            app.commit_mutations(queue).unwrap();
            nodes.push((kind, id));
        }
        let mut gallery = Self {
            context: app,
            document,
            shaper: bundled_face_shaper(),
            nodes,
        };
        gallery.frame();
        gallery
    }

    fn frame(&mut self) -> nana_ui_core::WorkCounters {
        product_frame(
            &mut self.context,
            self.document,
            viewport(),
            &mut self.shaper,
        )
    }
}

/// Gate C.
#[test]
fn issue202_c_every_replaced_kind_is_one_layout_item() {
    for context in [
        Context::Flex,
        Context::FlexStretch,
        Context::Grid,
        Context::GridStretch,
        Context::Inline,
        Context::Absolute,
    ] {
        let mut gallery = Gallery::new(context, None);
        let world = gallery.context.world();
        for &(kind, id) in &gallery.nodes {
            assert!(
                world.shows_replaced(&kind.resource()),
                "{context:?} {kind:?}"
            );
            let node = world.layout_node(id).unwrap();
            assert_eq!(
                node.content,
                LayoutContentKind::Replaced,
                "{context:?} {kind:?}"
            );
            assert_eq!(
                node.participation,
                Some(context.participation()),
                "{context:?} {kind:?}"
            );
        }
        let mut queue = MutationQueue::new();
        for kind in KINDS {
            queue.set_replaced_metadata(kind.resource(), Some(ReplacedMetadata::new(120.0, 80.0)));
        }
        gallery.context.commit_mutations(queue).unwrap();
        let work = gallery.frame();
        assert_eq!(work.replaced_layout_seeds, KINDS.len(), "{context:?}");
        assert_eq!(work.layout_full_document_fallbacks, 0, "{context:?}");
        for &(kind, id) in &gallery.nodes {
            let bounds = gallery.context.world().layout_box(id).unwrap();
            assert_eq!(
                (bounds.width, bounds.height),
                (120.0, 80.0),
                "{context:?} {kind:?}"
            );
        }
        let mut cold = Gallery::new(context, Some(ReplacedMetadata::new(120.0, 80.0)));
        assert_matches_cold(&mut gallery.context, &mut cold.context, gallery.document);
    }
}

/// Gate D.
#[test]
fn issue202_d_min_max_keep_the_natural_ratio() {
    type Edit = fn(&mut LayoutStyle);
    let cases: [(&str, Edit, (f32, f32)); 8] = [
        (
            "max-width",
            |l| l.max_width = Some(LengthSpec::Px(60.0)),
            (60.0, 40.0),
        ),
        (
            "min-width",
            |l| l.min_width = Some(LengthSpec::Px(240.0)),
            (240.0, 160.0),
        ),
        (
            "max-height",
            |l| l.max_height = Some(LengthSpec::Px(40.0)),
            (60.0, 40.0),
        ),
        (
            "min-height",
            |l| l.min_height = Some(LengthSpec::Px(160.0)),
            (240.0, 160.0),
        ),
        // Too wide and too tall: the tighter limit wins.
        (
            "max-width and max-height",
            |l| {
                l.max_width = Some(LengthSpec::Px(90.0));
                l.max_height = Some(LengthSpec::Px(40.0));
            },
            (60.0, 40.0),
        ),
        // Limits that cannot both keep the ratio: each holds, the ratio gives.
        (
            "min-width and max-height",
            |l| {
                l.min_width = Some(LengthSpec::Px(240.0));
                l.max_height = Some(LengthSpec::Px(40.0));
            },
            (240.0, 40.0),
        ),
        // One side of its own: the other follows, then meets its own limits.
        (
            "width and max-height",
            |l| {
                l.width = Some(LengthSpec::Px(60.0));
                l.max_height = Some(LengthSpec::Px(30.0));
            },
            (60.0, 30.0),
        ),
        (
            "height and min-width",
            |l| {
                l.height = Some(LengthSpec::Px(40.0));
                l.min_width = Some(LengthSpec::Px(90.0));
            },
            (90.0, 40.0),
        ),
    ];
    for context in [
        Context::Flex,
        Context::Grid,
        Context::Inline,
        Context::Absolute,
    ] {
        for (label, edit, expected) in cases {
            let build = |reported: bool| {
                let document = DocumentId::new(1).unwrap();
                let (mut b, page) = Builder::page(document, 1, 800.0);
                let container = b.element(page, context.container());
                let mut layout = image_layout("pic", None, None);
                edit(&mut layout);
                let id = b.element(container, context.item(layout, 0.0));
                if reported {
                    b.queue.set_replaced_metadata(
                        url("pic"),
                        Some(ReplacedMetadata::new(120.0, 80.0)),
                    );
                }
                let mut app = AppContext::new();
                app.commit_mutations(b.queue).unwrap();
                product_frame(&mut app, document, viewport(), &mut bundled_face_shaper());
                (app, document, id)
            };
            let (mut warm, document, id) = build(false);
            let mut queue = MutationQueue::new();
            queue.set_replaced_metadata(url("pic"), Some(ReplacedMetadata::new(120.0, 80.0)));
            warm.commit_mutations(queue).unwrap();
            product_frame(&mut warm, document, viewport(), &mut bundled_face_shaper());
            let bounds = warm.world().layout_box(id).unwrap();
            assert_eq!(
                (bounds.width, bounds.height),
                expected,
                "{context:?} {label}"
            );
            let (mut cold, _, _) = build(true);
            assert_matches_cold(&mut warm, &mut cold, document);
        }
    }
}

/// Gate E.
#[test]
fn issue202_e_only_a_new_natural_size_lays_out() {
    let mut page = Page::new(Context::Flex, true);
    let (auto, wide) = (page.auto, page.wide);
    let boxes = |page: &Page| (page.size(auto), page.size(wide));
    let before = boxes(&page);
    // A frame with nothing to do reports the last frame's counters again.
    let idle = page.frame();

    // The same size again is nothing: no update, no notice, no paint.
    let mut queue = MutationQueue::new();
    queue.set_replaced_metadata(url("pic"), Some(ReplacedMetadata::new(120.0, 80.0)));
    page.context.commit_mutations(queue).unwrap();
    assert!(page.context.take_system_work().is_empty());
    assert_eq!(page.frame(), idle);

    // Another resource of the same size: both boxes show it, neither moves.
    let mut queue = MutationQueue::new();
    queue.set_replaced_metadata(url("twin"), Some(ReplacedMetadata::new(120.0, 80.0)));
    for (id, width) in [(auto, None), (wide, Some(60.0))] {
        queue.set_style(
            id,
            styled(Context::Flex.item(image_layout("twin", width, None), 0.0)),
        );
    }
    page.context.commit_mutations(queue).unwrap();
    let work = page.frame();
    assert_eq!(work.replaced_layout_seeds, 0);
    assert_eq!(work.layout_frontier_seeds, 0);
    assert_eq!(work.layout_measure_nodes, 0);
    assert_eq!(work.intrinsic_generation_bumps, 0);
    assert!(page.world().shows_replaced(&url("twin")));
    assert!(!page.world().shows_replaced(&url("pic")));
    assert_eq!(boxes(&page), before);

    // One of another size: the two boxes that read it lay out again.
    let mut queue = MutationQueue::new();
    queue.set_replaced_metadata(url("tall"), Some(ReplacedMetadata::new(80.0, 120.0)));
    for (id, width) in [(auto, None), (wide, Some(60.0))] {
        queue.set_style(
            id,
            styled(Context::Flex.item(image_layout("tall", width, None), 0.0)),
        );
    }
    page.context.commit_mutations(queue).unwrap();
    let work = page.frame();
    assert_eq!(work.replaced_layout_seeds, 2);
    assert_eq!(boxes(&page), ((80.0, 120.0), (60.0, 90.0)));
}

/// Gate F.
#[test]
fn issue202_f_markup_fits_every_replaced_kind_alike() {
    use crate::component_registry::{ComponentTypeId, RegisterableComponent, SemanticSpec};
    use nana_ui_core::{BackgroundImageFit, ContentFit};

    let fits = |object_fit: Option<BackgroundImageFit>, attr: Option<&str>| {
        let mut layout = LayoutStyle::default();
        layout.paint.object_fit = object_fit;
        let layout = Arc::new(layout);
        let attrs: Vec<(&str, &str)> = [("data-nana-video", "1"), ("data-nana-gpu", "slot:7")]
            .into_iter()
            .chain(attr.map(|value| ("fit", value)))
            .collect();
        let video_type = ComponentTypeId::new("nana.video").unwrap();
        let mut spec = SemanticSpec::from_parts(&video_type, &layout);
        spec.attrs = &attrs;
        let video = Video::from_semantic(&spec).fit;
        let texture_type = ComponentTypeId::new("nana.gpu").unwrap();
        let mut spec = SemanticSpec::from_parts(&texture_type, &layout);
        spec.attrs = &attrs;
        let texture = GpuTextureView::from_semantic(&spec).fit;
        assert_eq!(video, texture);
        video
    };
    for fit in [
        ContentFit::Fill,
        ContentFit::Contain,
        ContentFit::Cover,
        ContentFit::None,
        ContentFit::ScaleDown,
    ] {
        // An image's `object-fit` is the same value.
        let image = BackgroundImageFit::from(fit);
        assert_eq!(image.object_fit(), Some(fit));
        // The stylesheet's `object-fit`, over any attribute.
        assert_eq!(fits(Some(image), None), fit, "{fit:?}");
        assert_eq!(fits(Some(image), Some("contain")), fit, "{fit:?}");
        // The attribute alone reads the same keywords.
        let keyword = match fit {
            ContentFit::Fill => "fill",
            ContentFit::Contain => "contain",
            ContentFit::Cover => "cover",
            ContentFit::None => "none",
            ContentFit::ScaleDown => "scale-down",
        };
        assert_eq!(fits(None, Some(keyword)), fit, "{keyword}");
    }
    assert_eq!(fits(None, None), ContentFit::Fill);
    assert_eq!(fits(None, Some("bogus")), ContentFit::Fill);
}

/// One block-flow line of mixed atoms, for Gate G.
struct AtomLine {
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
    block: StableNodeId,
    /// 40x20, empty: no line box.
    short: StableNodeId,
    /// 40 wide, `tall` high, 6 below: no line box.
    tall: StableNodeId,
    /// A 20x30 image, 4 below.
    image: StableNodeId,
    /// 50x30, `overflow: hidden`, holding a label.
    clipped: StableNodeId,
    /// A 60x60 text node of its own.
    text: StableNodeId,
    /// A 30x20 text node of its own, that neither sets the line's baseline
    /// nor its depth.
    note: StableNodeId,
}

impl AtomLine {
    fn new(tall: f32, text: &str, note: &str, short_label: bool) -> Self {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        let block = b.element(
            page,
            LayoutStyle {
                display: Some(DisplaySpec::Block),
                width: Some(LengthSpec::Px(600.0)),
                ..LayoutStyle::default()
            },
        );
        let atom = |width: f32, height: f32| LayoutStyle {
            display: Some(DisplaySpec::InlineBlock),
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        };
        let short = b.element(block, atom(40.0, 20.0));
        let tall = b.element(
            block,
            LayoutStyle {
                margin_bottom: Some(LengthSpec::Px(6.0)),
                ..atom(40.0, tall)
            },
        );
        let image = b.element(
            block,
            LayoutStyle {
                display: Some(DisplaySpec::InlineBlock),
                margin_bottom: Some(LengthSpec::Px(4.0)),
                ..image_layout("pic", Some(20.0), Some(30.0))
            },
        );
        let clipped = b.element(
            block,
            LayoutStyle {
                overflow_y: nana_ui_core::OverflowSpec::Hidden,
                ..atom(50.0, 30.0)
            },
        );
        b.label(clipped, "Clipped");
        let text = b.label(block, text);
        b.queue.set_style(text, styled(atom(60.0, 60.0)));
        let note = b.label(block, note);
        b.queue.set_style(note, styled(atom(30.0, 20.0)));
        if short_label {
            short_label_into(&mut b.queue, document, short);
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut line = Self {
            context,
            document,
            shaper: bundled_face_shaper(),
            block,
            short,
            tall,
            image,
            clipped,
            text,
            note,
        };
        line.frame();
        line
    }

    fn frame(&mut self) -> nana_ui_core::WorkCounters {
        product_frame(
            &mut self.context,
            self.document,
            viewport(),
            &mut self.shaper,
        )
    }

    fn commit(&mut self, queue: MutationQueue) {
        self.context.commit_mutations(queue).unwrap();
        self.frame();
    }

    /// The line's baseline, as the text atom's last line puts it.
    fn text_baseline(&self) -> f32 {
        self.last_baseline(self.text)
    }

    /// Where text atom `id` puts its last line's baseline, kept inside its
    /// box.
    fn last_baseline(&self, id: StableNodeId) -> f32 {
        let world = self.context.world();
        let (_, layout) = world.text_layout(id).expect("shaped text");
        let last = layout.lines.last().expect("a line").metrics.baseline_y_px;
        let bounds = world.layout_box(id).unwrap();
        bounds.y + last.min(bounds.height)
    }

    /// Every atom on one baseline, and the line box around them. `short`
    /// sits by its bottom edge while it has no line box.
    fn check(&self, short_empty: bool) {
        let world = self.context.world();
        let bounds = |id| world.layout_box(id).unwrap();
        let baseline = self.text_baseline();
        let lines = world.text_layout(self.text).unwrap().1.lines.len();
        let mut on_edge = vec![(self.tall, 6.0), (self.image, 4.0), (self.clipped, 0.0)];
        if short_empty {
            on_edge.push((self.short, 0.0));
        }
        for (id, below) in on_edge {
            let edge = bounds(id).y + bounds(id).height + below;
            assert!(
                (edge - baseline).abs() < 0.01,
                "{id:?}: bottom margin edge {edge} vs the text's last baseline {baseline} \
                 ({lines} lines)"
            );
        }
        let note = self.last_baseline(self.note);
        assert!(
            (note - baseline).abs() < 0.01,
            "the note's last baseline {note} vs {baseline}"
        );
        // Not top-aligned: the atoms' tops differ.
        assert!(bounds(self.short).y > bounds(self.tall).y + 1.0);
        // The line box: highest top to lowest bottom, margins included.
        let block = bounds(self.block);
        let atoms = [
            (self.short, 0.0),
            (self.tall, 6.0),
            (self.image, 4.0),
            (self.clipped, 0.0),
            (self.text, 0.0),
            (self.note, 0.0),
        ];
        let top = atoms
            .iter()
            .map(|&(id, _)| bounds(id).y)
            .fold(f32::INFINITY, f32::min);
        let bottom = atoms
            .iter()
            .map(|&(id, below)| bounds(id).y + bounds(id).height + below)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            (top - block.y).abs() < 0.01,
            "line top {top} vs {}",
            block.y
        );
        assert!(
            (block.height - (bottom - top)).abs() < 0.01,
            "block {} vs line box {}",
            block.height,
            bottom - top
        );
    }
}

/// Gate G.
#[test]
fn issue202_g_atoms_on_a_block_line_share_one_baseline() {
    let mut line = AtomLine::new(50.0, "Word", "a", false);
    assert_eq!(
        line.context
            .world()
            .layout_node(line.text)
            .unwrap()
            .participation,
        Some(Part::AtomicInline)
    );
    line.check(true);
    let first = line.text_baseline();

    // A taller atom lowers the baseline.
    let mut queue = MutationQueue::new();
    queue.set_style(
        line.tall,
        styled(LayoutStyle {
            display: Some(DisplaySpec::InlineBlock),
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(70.0)),
            margin_bottom: Some(LengthSpec::Px(6.0)),
            ..LayoutStyle::default()
        }),
    );
    line.commit(queue);
    line.check(true);
    assert!(line.text_baseline() > first + 1.0);
    assert_matches_cold(
        &mut line.context,
        &mut AtomLine::new(70.0, "Word", "a", false).context,
        line.document,
    );

    // Text that wraps in a box of the same size moves its last baseline:
    // the box rises to keep that baseline on the line's.
    let before = line.context.world().layout_box(line.text).unwrap().y;
    let mut queue = MutationQueue::new();
    queue.set_text(
        line.text,
        crate::TextContent {
            value: "Word word word".into(),
        },
    );
    line.commit(queue);
    assert!(
        line.context
            .world()
            .text_layout(line.text)
            .unwrap()
            .1
            .lines
            .len()
            > 1
    );
    assert_eq!(
        line.context.world().layout_box(line.text).unwrap().height,
        60.0
    );
    line.check(true);
    assert!(line.context.world().layout_box(line.text).unwrap().y < before - 1.0);
    assert_matches_cold(
        &mut line.context,
        &mut AtomLine::new(70.0, "Word word word", "a", false).context,
        line.document,
    );

    // A baseline that moves in a box of the same size, under a line of the
    // same height: the line's box and size stay, the atom moves.
    let block = line.context.world().layout_box(line.block).unwrap();
    let note = line.context.world().layout_box(line.note).unwrap();
    let mut queue = MutationQueue::new();
    queue.set_text(
        line.note,
        crate::TextContent {
            value: "aa aa aa aa".into(),
        },
    );
    line.commit(queue);
    assert!(
        line.context
            .world()
            .text_layout(line.note)
            .unwrap()
            .1
            .lines
            .len()
            > 1
    );
    line.check(true);
    assert_eq!(line.context.world().layout_box(line.block).unwrap(), block);
    let moved = line.context.world().layout_box(line.note).unwrap();
    assert_eq!((moved.width, moved.height), (note.width, note.height));
    assert!(moved.y < note.y - 1.0, "{moved:?} vs {note:?}");
    assert_matches_cold(
        &mut line.context,
        &mut AtomLine::new(70.0, "Word word word", "aa aa aa aa", false).context,
        line.document,
    );

    // A line box arriving in an empty atom gives it a baseline of its own.
    let mut queue = MutationQueue::new();
    short_label_into(&mut queue, line.document, line.short);
    line.commit(queue);
    line.check(false);
    assert_matches_cold(
        &mut line.context,
        &mut AtomLine::new(70.0, "Word word word", "aa aa aa aa", true).context,
        line.document,
    );
}

/// A label for Gate G's empty atom, at an id past the page's.
fn short_label_into(queue: &mut MutationQueue, document: DocumentId, short: StableNodeId) {
    let label = StableNodeId::new(1_000).unwrap();
    queue.create(label, document, crate::NodeKind::Text);
    queue.insert(short, label, None);
    queue.set_text(
        label,
        crate::TextContent {
            value: "Short".into(),
        },
    );
}
