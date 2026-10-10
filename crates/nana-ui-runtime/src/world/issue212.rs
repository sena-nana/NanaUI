//! Issue #212: a line that solves its overflow takes what its items declare,
//! cheapest first, and each item resolves its share inside itself.
//!
//! One toolbar: a row of six items, each a padded row around a fixed 40x20
//! box. The toolbar's gaps close to 2 px at the gap price; each item's
//! padding closes to 4 px a side at the padding price. Six items at 72 px and
//! five 8 px gaps prefer 472 px.

#![cfg(test)]

use nana_ui_core::dynamic_layout::{
    AdaptationProfile, AxisElasticity, ElasticFloor, ElasticLength, costs,
};
use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec, WorkCounters};

use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, fixed, product_frame, styled,
};
use super::{DocumentId, StableNodeId};
use crate::{AppContext, LayoutBox, LayoutViewport, MutationQueue, NanaTextEngineShaper};

pub(super) fn gap_profile() -> AdaptationProfile {
    AdaptationProfile {
        inline: AxisElasticity {
            gap: Some(ElasticLength::new(
                ElasticFloor::Px(2.0),
                costs::PLACEMENT_GAP,
            )),
            ..AxisElasticity::NONE
        },
        solve_overflow: true,
        ..AdaptationProfile::RIGID
    }
}

pub(super) fn padding_profile() -> AdaptationProfile {
    AdaptationProfile {
        inline: AxisElasticity {
            padding: Some(ElasticLength::new(ElasticFloor::Px(4.0), costs::PADDING)),
            ..AxisElasticity::NONE
        },
        ..AdaptationProfile::RIGID
    }
}

/// How the toolbar is built.
#[derive(Debug, Clone, Copy)]
pub(super) struct Bar {
    pub(super) width: f32,
    /// The toolbar solves its overflow.
    pub(super) solves: bool,
    /// The items declare their padding.
    pub(super) declares: bool,
    pub(super) shrink: Option<f32>,
}

impl Bar {
    pub(super) fn new(width: f32) -> Self {
        Self {
            width,
            solves: true,
            declares: true,
            shrink: None,
        }
    }

    fn toolbar_style(self) -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Px(self.width)),
            height: Some(LengthSpec::Px(40.0)),
            direction: Some(FlexDirection::Row),
            gap: Some(LengthSpec::Px(8.0)),
            adaptation: self.solves.then(gap_profile),
            ..LayoutStyle::default()
        }
    }

    fn item_style(self) -> LayoutStyle {
        LayoutStyle {
            direction: Some(FlexDirection::Row),
            padding_left: Some(LengthSpec::Px(16.0)),
            padding_right: Some(LengthSpec::Px(16.0)),
            flex_shrink: self.shrink,
            adaptation: self.declares.then(padding_profile),
            ..LayoutStyle::default()
        }
    }
}

pub(super) struct Toolbar {
    pub(super) context: AppContext,
    pub(super) document: DocumentId,
    shaper: NanaTextEngineShaper,
    pub(super) bar: StableNodeId,
    pub(super) items: Vec<StableNodeId>,
    pub(super) boxes: Vec<StableNodeId>,
    pub(super) shape: Bar,
}

impl Toolbar {
    pub(super) fn new(shape: Bar) -> Self {
        let document = DocumentId::new(1).unwrap();
        let (mut b, root) = Builder::new(document, 1);
        let bar = b.element(root, shape.toolbar_style());
        let mut items = Vec::new();
        let mut boxes = Vec::new();
        for _ in 0..6 {
            let item = b.element(bar, shape.item_style());
            boxes.push(b.element(item, fixed(40.0, 20.0)));
            items.push(item);
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut toolbar = Self {
            context,
            document,
            shaper: bundled_face_shaper(),
            bar,
            items,
            boxes,
            shape,
        };
        toolbar.frame();
        toolbar
    }

    pub(super) fn frame(&mut self) -> WorkCounters {
        product_frame(
            &mut self.context,
            self.document,
            LayoutViewport::new(1200.0, 400.0),
            &mut self.shaper,
        )
    }

    pub(super) fn resize(&mut self, width: f32) -> WorkCounters {
        self.shape.width = width;
        let mut queue = MutationQueue::new();
        queue.set_style(self.bar, styled(self.shape.toolbar_style()));
        self.context.commit_mutations(queue).unwrap();
        self.frame()
    }

    pub(super) fn layout(&self, id: StableNodeId) -> LayoutBox {
        self.context.world().layout_box(id).unwrap()
    }

    pub(super) fn item_boxes(&self) -> Vec<LayoutBox> {
        self.items.iter().map(|id| self.layout(*id)).collect()
    }
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.05
}

/// A line that fits lays out exactly as one nobody declared anything on,
/// and asks no item for its envelope.
#[test]
fn issue212_a_line_that_fits_is_unchanged() {
    let elastic = Toolbar::new(Bar::new(600.0));
    let rigid = Toolbar::new(Bar {
        solves: false,
        declares: false,
        ..Bar::new(600.0)
    });
    assert_eq!(elastic.item_boxes(), rigid.item_boxes());
    let counters = elastic.context.last_work_counters().dynamic;
    assert_eq!(counters.envelope_queries, 0);
    assert_eq!(counters.solver_runs, 0);
}

/// 32 px short: the gaps close first (30 px at the gap price), then the
/// padding gives the last 2 px. The line fills the toolbar exactly and every
/// fixed box keeps its size, inside its item's narrower padding.
#[test]
fn issue212_an_overflowing_line_closes_gaps_before_padding() {
    let toolbar = Toolbar::new(Bar::new(440.0));
    let items = toolbar.item_boxes();
    let bar = toolbar.layout(toolbar.bar);
    let last = items.last().unwrap();
    assert!(near(last.x + last.width, bar.x + bar.width), "{items:?}");
    for pair in items.windows(2) {
        assert!(
            near(pair[1].x - (pair[0].x + pair[0].width), 2.0),
            "{items:?}"
        );
    }
    let padding_given: f32 = items.iter().map(|item| 72.0 - item.width).sum();
    assert!(near(padding_given, 2.0), "{padding_given}");
    for (item, fixed) in toolbar.items.iter().zip(&toolbar.boxes) {
        let (item, fixed) = (toolbar.layout(*item), toolbar.layout(*fixed));
        assert_eq!((fixed.width, fixed.height), (40.0, 20.0));
        let left = fixed.x - item.x;
        let right = item.x + item.width - (fixed.x + fixed.width);
        assert!(
            near(left, right) && (4.0..16.0).contains(&left),
            "{left} {right}"
        );
    }
}

/// Past every declared capacity the solve takes it all; an explicit shrink
/// factor then clips only what is left, and without one the rest overflows.
#[test]
fn issue212_a_shrink_factor_takes_only_what_the_solve_left() {
    let clipped = Toolbar::new(Bar {
        shrink: Some(1.0),
        ..Bar::new(250.0)
    });
    let items = clipped.item_boxes();
    let bar = clipped.layout(clipped.bar);
    let end = items.last().map(|item| item.x + item.width).unwrap();
    assert!(near(end, bar.x + bar.width), "{items:?}");
    let overflowing = Toolbar::new(Bar::new(250.0));
    let items = overflowing.item_boxes();
    for item in &items {
        assert!(near(item.width, 48.0), "{items:?}");
    }
}

/// A toolbar that does not solve never reads what its items declare: the
/// same boxes as a toolbar of items that declare nothing, and no envelope
/// asked for.
#[test]
fn issue212_a_line_that_does_not_solve_ignores_declarations() {
    let declared = Toolbar::new(Bar {
        solves: false,
        ..Bar::new(300.0)
    });
    let plain = Toolbar::new(Bar {
        solves: false,
        declares: false,
        ..Bar::new(300.0)
    });
    assert_eq!(declared.item_boxes(), plain.item_boxes());
    let counters = declared.context.last_work_counters().dynamic;
    assert_eq!(
        (counters.contexts_considered, counters.envelope_queries),
        (0, 0)
    );
}

/// Resizing through fitting, gap-only, padding and infeasible widths: every
/// pass matches a full layout, and the end matches a cold one.
#[test]
fn issue212_every_resize_matches_a_full_layout() {
    let mut toolbar = Toolbar::new(Bar::new(600.0));
    for width in [
        480.0, 460.0, 445.0, 400.0, 330.0, 250.0, 300.0, 500.0, 600.0, 320.0,
    ] {
        toolbar.resize(width);
    }
    let mut cold = Toolbar::new(Bar::new(320.0));
    assert_matches_cold(&mut toolbar.context, &mut cold.context, toolbar.document);
}

/// Devtools sees what an item declared and what its line gave it: its
/// envelope's segments and generation, and the part its padding closed up.
#[test]
fn issue212_inspect_shows_the_envelope_and_the_assignment() {
    let toolbar = Toolbar::new(Bar::new(330.0));
    let item = toolbar.items[0];
    let dynamic = toolbar.context.inspect(item).unwrap().dynamic.unwrap();
    assert!(dynamic.generation > 0);
    assert_eq!(dynamic.segments.len(), 1);
    assert_eq!(
        dynamic.segments[0].kind,
        nana_ui_core::dynamic_layout::AdjustmentKind::Padding
    );
    let applied = dynamic.applied.unwrap();
    assert!(applied.inline && applied.amount.is_positive());
    assert_eq!(applied.padding, applied.amount);
    let fits = Toolbar::new(Bar::new(600.0));
    assert!(
        fits.context
            .inspect(fits.items[0])
            .unwrap()
            .dynamic
            .is_none()
    );
}

/// The built-in strip: a `Toolbar` of `Button`s too wide for its page closes
/// its gaps and then the buttons' padding, and fits; the same toolbar with
/// its solving turned off overflows. Buttons in a plain row never compress.
#[test]
fn issue212_a_toolbar_of_buttons_closes_up_before_it_overflows() {
    use crate::{Button, Stack, Toolbar};
    use nana_ui_core::dynamic_layout::AdaptationProfile;
    let document = DocumentId::new(1).unwrap();
    let build = |solving: bool| {
        let mut context = AppContext::new();
        let page = context
            .create_component(
                document,
                Stack::from_layout(LayoutStyle {
                    width: Some(LengthSpec::Px(440.0)),
                    direction: Some(FlexDirection::Column),
                    ..LayoutStyle::default()
                }),
            )
            .unwrap();
        let mut toolbar = Toolbar::new();
        if !solving {
            std::sync::Arc::make_mut(&mut toolbar.style.layout).adaptation =
                Some(AdaptationProfile::RIGID);
        }
        let toolbar = context
            .create_detached_component(document, toolbar)
            .unwrap();
        context.append_child(page, toolbar).unwrap();
        let buttons: Vec<_> = (0..6)
            .map(|at| {
                let button = context
                    .create_detached_component(document, Button::new(format!("Action {at}")))
                    .unwrap();
                context.append_child(toolbar, button).unwrap();
                button.stable_id()
            })
            .collect();
        let mut shaper = bundled_face_shaper();
        product_frame(
            &mut context,
            document,
            LayoutViewport::new(800.0, 600.0),
            &mut shaper,
        );
        let bar = context.world().layout_box(toolbar.stable_id()).unwrap();
        let boxes: Vec<LayoutBox> = buttons
            .iter()
            .map(|id| context.world().layout_box(*id).unwrap())
            .collect();
        (context, bar, boxes)
    };
    let (context, bar, solved) = build(true);
    let (_, _, rigid) = build(false);
    let end = |boxes: &[LayoutBox]| boxes.last().map(|last| last.x + last.width).unwrap();
    assert!(
        end(&rigid) > bar.x + bar.width,
        "the rigid toolbar fits: {rigid:?}"
    );
    assert!(
        end(&solved) <= bar.x + bar.width + 0.05,
        "{solved:?} in {bar:?}"
    );
    for (solved, rigid) in solved.iter().zip(&rigid) {
        assert!(solved.width < rigid.width);
    }
    assert!(context.last_work_counters().dynamic.child_resolves >= 6);
}
