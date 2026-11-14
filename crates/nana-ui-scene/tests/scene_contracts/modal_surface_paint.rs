//! A dialog that paints its own card, as the scene hands it to a painter.
//!
//! The card surface painter's output takes the theme card's place: over the
//! scrim, under the card's title and text, on the frame's own layer, so it
//! fades and moves with the card.

use std::time::Duration;

use nana_ui_core::SemanticColorRole;
use nana_ui_runtime::{
    ComputedStyle, ConfirmDialog, DocumentId, LayoutViewport, OverlayHost, PaintContext, PaintPath,
    Painter, StableNodeId, TextContent, TextMetrics, TextShapeConstraints, TextShaper,
};
use nana_ui_scene::{PrimitiveId, RuntimeDocument};

struct NoText;

impl TextShaper for NoText {
    fn shape(
        &mut self,
        _id: StableNodeId,
        _text: &TextContent,
        _style: &ComputedStyle,
        _constraints: TextShapeConstraints,
    ) -> TextMetrics {
        TextMetrics::default()
    }
}

/// A card with a band across its header: two fills, card coordinates.
struct BandedCard {
    themed: bool,
}

impl Painter for BandedCard {
    fn paint(&self, cx: &mut PaintContext<'_>) {
        if self.themed {
            cx.draw_default();
        }
        let [width, height] = cx.size();
        let header = cx
            .modal_sections()
            .map_or(0.0, |sections| sections.header.height);
        let mut card = PaintPath::new();
        card.rounded_rect(
            nana_ui_runtime::LayoutBox {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
            [12.0; 4],
        );
        cx.fill_path(&card, SemanticColorRole::Surface);
        let mut band = PaintPath::new();
        band.rounded_rect(
            nana_ui_runtime::LayoutBox {
                x: 0.0,
                y: 0.0,
                width,
                height: header,
            },
            [12.0, 12.0, 0.0, 0.0],
        );
        cx.fill_path(&band, SemanticColorRole::Accent);
    }

    fn paint_key(&self) -> u64 {
        u64::from(self.themed)
    }
}

/// The slot namespace the scene gives a card surface painter's primitives.
const MODAL_SURFACE_PAINT: u64 = 0xFFFF_FFD0;

fn open(themed: bool) -> (RuntimeDocument, StableNodeId) {
    open_with_motion(themed, true)
}

fn open_with_motion(themed: bool, reduced: bool) -> (RuntimeDocument, StableNodeId) {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let cx = runtime.context_mut();
    let host = cx.create_component(document, OverlayHost::new()).unwrap();
    let dialog = cx
        .create_detached_component(
            document,
            ConfirmDialog::new("删除场景", "无法恢复。").surface_painter(BandedCard { themed }),
        )
        .unwrap();
    cx.assemble_confirm_dialog(dialog).unwrap();
    cx.append_child(host, dialog).unwrap();
    cx.set_reduced_motion(reduced);
    runtime
        .flush(LayoutViewport::new(800.0, 600.0), &mut NoText)
        .unwrap();
    assert!(
        runtime
            .context_mut()
            .activate_overlay(host, dialog)
            .unwrap()
    );
    runtime.context_mut().advance_animations(Duration::ZERO);
    runtime
        .flush(LayoutViewport::new(800.0, 600.0), &mut NoText)
        .unwrap();
    (runtime, dialog.stable_id())
}

#[test]
fn a_custom_card_is_transparent_before_its_layer_is_promoted() {
    let (runtime, id) = open_with_motion(false, false);
    assert!(runtime.scene().compositor_layer(id).is_none());
    let mut painted = 0;
    for primitive in runtime.scene().primitives().filter(|p| p.node == id) {
        if primitive.id.slot >> 32 == MODAL_SURFACE_PAINT {
            painted += 1;
            let draw = runtime.scene().draw_primitive(primitive.id).unwrap();
            let groups = runtime.scene().primitive_opacity_groups(primitive);
            assert_eq!(groups.len(), 1, "the complete custom card fades together");
            assert_eq!(groups[0].node, id);
            assert_eq!(
                groups[0].opacity, 0.0,
                "custom card flashed at animation start"
            );
            assert_eq!(draw.paint_opacity, 1.0, "the group owns the fade");
            let encoded = runtime.scene().compositor_primitive_encode(
                primitive,
                draw.transform,
                draw.paint_opacity,
            );
            assert_eq!(encoded.opacity, 1.0, "no second fade inside the group");
            assert_eq!(encoded.motion_ids, (0, 0));
        }
    }
    assert!(painted > 0);
}

/// The frame's own primitives, in paint order.
fn frame_slots(runtime: &RuntimeDocument, id: StableNodeId) -> Vec<u64> {
    runtime
        .scene()
        .primitives()
        .filter(|primitive| primitive.node == id)
        .map(|primitive| primitive.id.slot)
        .collect()
}

#[test]
fn a_painted_card_takes_the_themed_cards_place_between_scrim_and_title() {
    let (runtime, id) = open(false);
    let slots = frame_slots(&runtime, id);
    let painted = |index: u64| (MODAL_SURFACE_PAINT << 32) | index;
    assert!(!slots.contains(&11), "no theme card under it: {slots:x?}");
    let position = |slot: u64| {
        slots
            .iter()
            .position(|held| *held == slot)
            .unwrap_or_else(|| panic!("slot {slot:x} in {slots:x?}"))
    };
    let (scrim, card, band, title) = (
        position(10),
        position(painted(0)),
        position(painted(1)),
        position(12),
    );
    assert!(
        scrim < card && card < band && band < title,
        "scrim, card, band, then the title: {slots:x?}"
    );

    let scene = runtime.scene();
    let card = scene
        .draw_primitive(PrimitiveId {
            node: id,
            slot: painted(0),
        })
        .expect("the painted card draws");
    let scrim = scene
        .draw_primitive(PrimitiveId { node: id, slot: 10 })
        .expect("the scrim draws");
    assert_eq!(
        card.primitive.transform, scrim.primitive.transform,
        "the card rides the frame's own layer"
    );
}

#[test]
fn draw_default_keeps_the_themed_card_under_the_painted_one() {
    let (runtime, id) = open(true);
    let slots = frame_slots(&runtime, id);
    let card = slots
        .iter()
        .position(|slot| *slot == 11)
        .expect("theme card");
    // `draw_default()` is the recording's first op, so the painted ones
    // start after it.
    let painted = slots
        .iter()
        .position(|slot| *slot >> 32 == MODAL_SURFACE_PAINT)
        .expect("painted card");
    assert!(card < painted, "the theme's card is under: {slots:x?}");
    assert!(
        slots[painted..].iter().all(|slot| *slot != 11),
        "and only once: {slots:x?}"
    );
}

/// A card shaped by a real path: a tab rising from the card's left part
/// and sloping down to its top edge, the way a tabbed card is drawn.
struct SlopedTab;

impl Painter for SlopedTab {
    fn paint(&self, cx: &mut PaintContext<'_>) {
        let [width, height] = cx.size();
        let rise = cx
            .modal_sections()
            .map_or(24.0, |sections| sections.header.height);
        let mut card = PaintPath::new();
        card.move_to(0.0, 0.0)
            .line_to(width * 0.55, 0.0)
            .line_to(width * 0.55 + 30.0, rise)
            .line_to(width, rise)
            .line_to(width, height)
            .line_to(0.0, height)
            .close();
        cx.fill_path(&card, SemanticColorRole::Surface);
    }

    fn paint_key(&self) -> u64 {
        2
    }
}

/// An edit after a path-shaped card is painted is a delta, not a rebuild,
/// and the scene's own audit of every delta (debug builds) holds: each
/// painted primitive keeps the recording it was cut from, whether a node
/// painter or a dialog's card painter recorded it.
#[test]
fn a_path_shaped_card_survives_a_delta() {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let cx = runtime.context_mut();
    let host = cx.create_component(document, OverlayHost::new()).unwrap();
    let dialog = cx
        .create_detached_component(
            document,
            ConfirmDialog::new("删除场景", "无法恢复。").surface_painter(SlopedTab),
        )
        .unwrap();
    cx.assemble_confirm_dialog(dialog).unwrap();
    cx.append_child(host, dialog).unwrap();
    cx.set_reduced_motion(true);
    let viewport = LayoutViewport::new(800.0, 600.0);
    runtime.flush(viewport, &mut NoText).unwrap();
    assert!(
        runtime
            .context_mut()
            .activate_overlay(host, dialog)
            .unwrap()
    );
    for step in 1..=3 {
        runtime
            .context_mut()
            .advance_animations(Duration::from_millis(500 * step));
        runtime.flush(viewport, &mut NoText).unwrap();
    }
    runtime
        .context_mut()
        .update_component(dialog, |dialog, _| {
            dialog.message = "场景内的模型会一并删除。".into();
        })
        .unwrap();
    runtime.flush(viewport, &mut NoText).unwrap();
    assert!(
        frame_slots(&runtime, dialog.stable_id())
            .iter()
            .any(|slot| *slot >> 32 == MODAL_SURFACE_PAINT),
        "still painted after the delta"
    );
}
