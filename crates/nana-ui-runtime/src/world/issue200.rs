//! Issue #200: placement shares one coordinate authority.
//!
//! An absolute box hangs from its containing block as CSS defines it: the
//! padding box of its nearest positioned ancestor, static ones in between
//! skipped, or the initial containing block -- the viewport at the document
//! origin -- when it has none. A retained pass follows that block when it
//! moves or resizes, and leaves the box where it is when only a static box
//! between them moves, matching a cold layout at every step.
//!
//! A pointer menu and an anchored action menu state an anchor
//! ([`nana_ui_core::AnchorSpec`]). Layout places them, at the size they
//! measure and against the real viewport, through the same
//! [`crate::resolve_anchored_surface`] that triggered menus, hover cards,
//! popovers and the select drop-down use. They flip to the side that fits,
//! stay in the window (and in the room an owner gives), follow a viewport
//! change with no reprojection, and hit testing and accessibility read the
//! box layout gave them.
//!
//! A fixed box hangs from its own document's viewport, past positioned and
//! scrolled ancestors, and so does an absolute box with no positioned
//! ancestor. Two documents of one context keep their own viewports: a
//! resize of one runs no layout in the other, and hit testing and
//! accessibility read the boxes each viewport gave.

#![cfg(test)]

use nana_ui_core::{
    AnchoredMenuPlacement, FlexDirection, LayoutStyle, LengthSpec, PositionSpec, WorkCounters,
};

use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, product_frame, resize_frame, styled,
};
use super::{DocumentId, StableNodeId};
use crate::{
    AnchoredActionMenu, AppContext, ComponentView, ContextMenu, ContextMenuItem, Entity, LayoutBox,
    LayoutViewport, MutationQueue, NanaTextEngineShaper, OverlayHost, resolve_anchored_origin,
};

/// A window whose overlay layer hosts one open menu, as an application
/// mounts it. Motion is reduced so the surface shows at its laid-out size.
struct Window {
    context: AppContext,
    document: DocumentId,
    host: Entity<OverlayHost>,
    shaper: NanaTextEngineShaper,
    viewport: LayoutViewport,
}

impl Window {
    fn new(width: f32, height: f32) -> Self {
        let mut context = AppContext::new();
        context.set_reduced_motion(true);
        let document = DocumentId::new(1).unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        Self {
            context,
            document,
            host,
            shaper: bundled_face_shaper(),
            viewport: LayoutViewport::new(width, height),
        }
    }

    fn open<C: ComponentView>(&mut self, surface: C) -> StableNodeId {
        let surface = self
            .context
            .create_component(self.document, surface)
            .unwrap();
        self.context.append_child(self.host, surface).unwrap();
        self.context.activate_overlay(self.host, surface).unwrap();
        surface.stable_id()
    }

    fn frame(&mut self) {
        product_frame(
            &mut self.context,
            self.document,
            self.viewport,
            &mut self.shaper,
        );
    }

    fn resize(&mut self, width: f32, height: f32) {
        self.viewport = LayoutViewport::new(width, height);
        resize_frame(
            &mut self.context,
            self.document,
            self.viewport,
            &mut self.shaper,
        );
    }

    fn surface(&self, id: StableNodeId) -> LayoutBox {
        self.context.world().layout_box(id).unwrap()
    }

    fn window_box(&self) -> LayoutBox {
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: self.viewport.width,
            height: self.viewport.height,
        }
    }
}

fn items(count: usize) -> Vec<ContextMenuItem> {
    (0..count)
        .map(|index| ContextMenuItem::new(format!("item{index}"), format!("Item {index}")))
        .collect()
}

fn inside(surface: LayoutBox, room: LayoutBox) -> bool {
    surface.x >= room.x - 0.01
        && surface.y >= room.y - 0.01
        && surface.x + surface.width <= room.x + room.width + 0.01
        && surface.y + surface.height <= room.y + room.height + 0.01
}

#[test]
fn issue200_a_pointer_menu_is_placed_by_layout_and_follows_the_viewport() {
    let mut window = Window::new(800.0, 600.0);
    let (x, y) = (760.0, 560.0);
    let menu = window.open(ContextMenu::new(x, y).items(items(6)).open(true));
    window.frame();
    let surface = window.surface(menu);
    // No room below or after the pointer: it opens above it and moves left
    // into the window, where the shared resolver puts a box of its size.
    assert!(surface.height > 0.0);
    assert!((surface.y + surface.height - y).abs() < 0.01, "{surface:?}");
    assert!(inside(surface, window.window_box()), "{surface:?}");
    assert_eq!(
        (surface.x, surface.y),
        resolve_anchored_origin(
            x,
            y,
            surface.width,
            surface.height,
            window.window_box(),
            AnchoredMenuPlacement::BottomStart,
        )
    );
    // Hit testing and accessibility read the box layout gave it.
    window.context.rebuild_hit_test(window.document);
    let world = window.context.world();
    let center = (
        surface.x + surface.width / 2.0,
        surface.y + surface.height / 2.0,
    );
    let mut hit = world.hit_test(window.document, center.0, center.1);
    while hit.is_some_and(|node| node != menu) {
        hit = hit.and_then(|node| world.parent_id(node));
    }
    assert_eq!(hit, Some(menu), "the pointer over the menu lands in it");
    let spoken = world
        .project_accessibility(window.document)
        .into_iter()
        .find(|node| node.id == menu)
        .unwrap()
        .bounds;
    assert_eq!(
        (spoken.x, spoken.y, spoken.width, spoken.height),
        (surface.x, surface.y, surface.width, surface.height)
    );

    // A taller window has room below the pointer: layout moves the menu
    // there on the resize alone, with no reprojection of the component.
    let authored = |window: &Window| {
        window
            .context
            .world()
            .node_style(menu)
            .unwrap()
            .layout
            .clone()
    };
    let before = authored(&window);
    window.resize(1200.0, 900.0);
    let moved = window.surface(menu);
    assert_eq!((moved.x, moved.y), (x, y), "{moved:?}");
    assert!(
        std::sync::Arc::ptr_eq(&before, &authored(&window)),
        "the menu's authored style is the one it had before the resize"
    );
}

#[test]
fn issue200_a_menu_stays_in_the_room_its_owner_gives() {
    let mut window = Window::new(800.0, 600.0);
    let room = LayoutBox {
        x: 100.0,
        y: 100.0,
        width: 300.0,
        height: 240.0,
    };
    let mut menu = ContextMenu::new(380.0, 300.0).items(items(5)).open(true);
    menu.place_in(room);
    let menu = window.open(menu);
    window.frame();
    let surface = window.surface(menu);
    assert!(inside(surface, room), "{surface:?} outside {room:?}");
    assert_eq!(
        (surface.x, surface.y),
        resolve_anchored_origin(
            380.0,
            300.0,
            surface.width,
            surface.height,
            room,
            AnchoredMenuPlacement::BottomStart,
        )
    );
}

#[test]
fn issue200_an_anchored_action_menu_hangs_on_its_side_and_flips() {
    let hang = |x: f32, y: f32| {
        let mut window = Window::new(800.0, 600.0);
        let menu = window.open(
            AnchoredActionMenu::new(x, y)
                .placement(AnchoredMenuPlacement::TopEnd)
                .menu_size(160.0, 120.0)
                .open(true),
        );
        window.frame();
        window.surface(menu)
    };
    // Room above and before the point: the menu ends on it.
    let surface = hang(300.0, 300.0);
    assert_eq!(
        (
            surface.x + surface.width,
            surface.y + surface.height,
            surface.width,
            surface.height
        ),
        (300.0, 300.0, 160.0, 120.0)
    );
    // Top-end from a point near the top-left corner: no room above, so it
    // flips below, and its end alignment would leave the window, so it moves
    // right into it.
    let surface = hang(40.0, 20.0);
    assert_eq!((surface.x, surface.y), (0.0, 20.0), "{surface:?}");
}

/// A tree given as `(parent, style)` per node -- `None` for the page, an
/// index into the list otherwise -- laid out through the product frame. Each
/// step restyles the retained tree and compares it with one built cold.
struct Tree {
    spec: Vec<(Option<usize>, LayoutStyle)>,
    context: AppContext,
    ids: Vec<StableNodeId>,
    shaper: NanaTextEngineShaper,
    viewport: LayoutViewport,
}

impl Tree {
    fn new(spec: Vec<(Option<usize>, LayoutStyle)>) -> Self {
        let viewport = LayoutViewport::new(800.0, 600.0);
        let (context, ids) = Self::build(&spec, viewport);
        Self {
            spec,
            context,
            ids,
            shaper: bundled_face_shaper(),
            viewport,
        }
    }

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn build(
        spec: &[(Option<usize>, LayoutStyle)],
        viewport: LayoutViewport,
    ) -> (AppContext, Vec<StableNodeId>) {
        let (mut b, page) = Builder::new(Self::document(), 1);
        let page = b.element(
            page,
            LayoutStyle {
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            },
        );
        let mut ids: Vec<StableNodeId> = Vec::with_capacity(spec.len());
        for (parent, style) in spec {
            let parent = parent.map_or(page, |index| ids[index]);
            ids.push(b.element(parent, style.clone()));
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        product_frame(
            &mut context,
            Self::document(),
            viewport,
            &mut bundled_face_shaper(),
        );
        (context, ids)
    }

    fn rect(&self, index: usize) -> (f32, f32, f32, f32) {
        let b = self.context.world().layout_box(self.ids[index]).unwrap();
        (b.x, b.y, b.width, b.height)
    }

    /// Restyles node `index` in the retained tree, lays it out, and checks
    /// it against the same tree laid out cold.
    fn restyle(&mut self, index: usize, style: LayoutStyle) {
        self.spec[index].1 = style.clone();
        let mut queue = MutationQueue::new();
        queue.set_style(self.ids[index], styled(style));
        self.context.commit_mutations(queue).unwrap();
        product_frame(
            &mut self.context,
            Self::document(),
            self.viewport,
            &mut self.shaper,
        );
        self.matches_cold();
    }

    fn resize(&mut self, width: f32, height: f32) {
        self.viewport = LayoutViewport::new(width, height);
        resize_frame(
            &mut self.context,
            Self::document(),
            self.viewport,
            &mut self.shaper,
        );
        self.matches_cold();
    }

    fn matches_cold(&mut self) {
        let (mut cold, _) = Self::build(&self.spec, self.viewport);
        assert_matches_cold(&mut self.context, &mut cold, Self::document());
    }
}

fn sized(width: f32, height: f32) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        flex_shrink: Some(0.0),
        ..LayoutStyle::default()
    }
}

/// A card that is the containing block, with padding and a border.
fn card(position: PositionSpec, width: f32) -> LayoutStyle {
    LayoutStyle {
        position,
        padding_top: Some(LengthSpec::Px(12.0)),
        padding_right: Some(LengthSpec::Px(12.0)),
        padding_bottom: Some(LengthSpec::Px(12.0)),
        padding_left: Some(LengthSpec::Px(12.0)),
        border_width: Some(2.0),
        direction: Some(FlexDirection::Column),
        ..sized(width, 200.0)
    }
}

/// A static box with padding of its own between the card and the badge.
fn wrapper(position: PositionSpec) -> LayoutStyle {
    LayoutStyle {
        position,
        padding_top: Some(LengthSpec::Px(10.0)),
        padding_left: Some(LengthSpec::Px(10.0)),
        ..sized(200.0, 100.0)
    }
}

/// Half as wide as its containing block, in its top-right corner.
fn badge() -> LayoutStyle {
    LayoutStyle {
        position: PositionSpec::Absolute,
        offset_top: Some(LengthSpec::Px(0.0)),
        offset_right: Some(LengthSpec::Px(0.0)),
        width: Some(LengthSpec::Percent(50.0)),
        height: Some(LengthSpec::Px(10.0)),
        ..LayoutStyle::default()
    }
}

const CARD: usize = 0;
const SPACER: usize = 1;
const WRAPPER: usize = 2;
const BADGE: usize = 3;

/// The card holds a spacer and then the static box; the badge and a row in
/// flow are in the static box, so a pass where only the static box moves
/// replays its plan rather than placing it again.
fn card_tree() -> Tree {
    Tree::new(vec![
        (None, card(PositionSpec::Relative, 300.0)),
        (Some(CARD), sized(200.0, 30.0)),
        (Some(CARD), wrapper(PositionSpec::Static)),
        (Some(WRAPPER), badge()),
        (Some(WRAPPER), sized(80.0, 20.0)),
    ])
}

/// The badge sits in the top-right corner of the card's padding box, half
/// its width, though a static box with padding of its own stands between.
fn assert_hangs_from_card(tree: &Tree) {
    let (cx, cy, cw, _) = tree.rect(CARD);
    let (bx, by, bw, bh) = tree.rect(BADGE);
    let inner = cw - 4.0;
    assert_eq!(
        (bx, by, bw, bh),
        (cx + 2.0 + inner / 2.0, cy + 2.0, inner / 2.0, 10.0),
        "the badge hangs from the card's padding box"
    );
}

#[test]
fn issue200_an_absolute_box_hangs_from_its_nearest_positioned_ancestor() {
    let mut tree = card_tree();
    tree.matches_cold();
    assert_hangs_from_card(&tree);
    // The card grows; the static box between them does not move or resize,
    // so no replay of its plan reaches the badge -- the badge follows anyway.
    tree.restyle(CARD, card(PositionSpec::Relative, 360.0));
    assert_eq!(tree.rect(WRAPPER).2, 200.0);
    assert_hangs_from_card(&tree);
    // The static box moves down; the card does not, and neither does the
    // badge.
    let before = tree.rect(BADGE);
    tree.restyle(SPACER, sized(200.0, 50.0));
    assert_eq!(
        tree.rect(BADGE),
        before,
        "only its containing block moves it"
    );
    // The static box becomes positioned: it is the nearest one now.
    tree.restyle(WRAPPER, wrapper(PositionSpec::Relative));
    let (wx, wy, ww, _) = tree.rect(WRAPPER);
    assert_eq!(tree.rect(BADGE), (wx + ww / 2.0, wy, ww / 2.0, 10.0));
    // Neither is positioned: the initial containing block, the viewport.
    tree.restyle(WRAPPER, wrapper(PositionSpec::Static));
    tree.restyle(CARD, card(PositionSpec::Static, 360.0));
    assert_eq!(tree.rect(BADGE), (400.0, 0.0, 400.0, 10.0));
    // And back: the card is its containing block again.
    tree.restyle(CARD, card(PositionSpec::Relative, 360.0));
    assert_hangs_from_card(&tree);
}

#[test]
fn issue200_a_sticky_box_is_a_containing_block() {
    let mut tree = Tree::new(vec![
        (
            None,
            LayoutStyle {
                offset_top: Some(LengthSpec::Px(0.0)),
                ..card(PositionSpec::Sticky, 300.0)
            },
        ),
        (Some(CARD), sized(200.0, 30.0)),
        (Some(CARD), wrapper(PositionSpec::Static)),
        (Some(WRAPPER), badge()),
    ]);
    tree.matches_cold();
    assert_hangs_from_card(&tree);
}

#[test]
fn issue200_with_no_positioned_ancestor_the_viewport_is_the_containing_block() {
    let corner = LayoutStyle {
        position: PositionSpec::Absolute,
        offset_right: Some(LengthSpec::Px(0.0)),
        offset_bottom: Some(LengthSpec::Px(0.0)),
        ..sized(30.0, 20.0)
    };
    let mut tree = Tree::new(vec![
        (None, sized(300.0, 200.0)),
        (Some(0), wrapper(PositionSpec::Static)),
        (Some(1), corner),
    ]);
    tree.matches_cold();
    assert_eq!(tree.rect(2), (770.0, 580.0, 30.0, 20.0));
    // The viewport resizes; nothing between the box and the document reads
    // it, and the box still follows.
    tree.resize(1000.0, 700.0);
    assert_eq!(tree.rect(2), (970.0, 680.0, 30.0, 20.0));
}

#[test]
fn issue200_an_axis_with_auto_insets_keeps_its_static_position() {
    // Right-aligned against the card, with no vertical inset: it stays at
    // the top of the static box's content, where it would start in flow.
    let pinned = LayoutStyle {
        position: PositionSpec::Absolute,
        offset_right: Some(LengthSpec::Px(0.0)),
        ..sized(20.0, 10.0)
    };
    let mut tree = Tree::new(vec![
        (None, card(PositionSpec::Relative, 300.0)),
        (Some(CARD), sized(200.0, 30.0)),
        (Some(CARD), wrapper(PositionSpec::Static)),
        (Some(WRAPPER), pinned),
        (Some(WRAPPER), sized(80.0, 20.0)),
    ]);
    tree.matches_cold();
    let at = |tree: &Tree| {
        let (cx, _, cw, _) = tree.rect(CARD);
        let (_, wy, _, _) = tree.rect(WRAPPER);
        (cx + cw - 2.0 - 20.0, wy + 10.0)
    };
    let (x, y, ..) = tree.rect(BADGE);
    assert_eq!((x, y), at(&tree));
    // The static box moves: the box keeps its static position in it, and
    // its containing block's right edge.
    tree.restyle(SPACER, sized(200.0, 50.0));
    let (x, y, ..) = tree.rect(BADGE);
    assert_eq!((x, y), at(&tree));
}

/// A page with a 500 px spacer and then an open Select of eight options, laid
/// out in a window of `height`, its hit index rebuilt.
struct SelectPage {
    context: AppContext,
    document: DocumentId,
    select: Entity<crate::Select>,
    shaper: NanaTextEngineShaper,
}

impl SelectPage {
    fn new(height: f32) -> Self {
        use crate::{Select, SelectOption, Stack};
        let document = DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        context.set_reduced_motion(true);
        let root = context
            .create_component(
                document,
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(400.0));
                }),
            )
            .unwrap();
        let spacer = context
            .create_detached_component(
                document,
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Fill);
                    layout.height = Some(LengthSpec::Px(500.0));
                }),
            )
            .unwrap();
        context.append_child(root, spacer).unwrap();
        let select = context
            .create_detached_component(
                document,
                Select::new(Some("v0")).options(
                    (0..8).map(|n| SelectOption::new(format!("v{n}"), format!("Option {n}"))),
                ),
            )
            .unwrap();
        context.append_child(root, select).unwrap();
        let mut page = Self {
            context,
            document,
            select,
            shaper: bundled_face_shaper(),
        };
        page.frame(height, false);
        assert!(page.context.toggle_select(select).unwrap());
        page.frame(height, false);
        page
    }

    fn frame(&mut self, height: f32, resized: bool) {
        let viewport = LayoutViewport::new(400.0, height);
        if resized {
            resize_frame(&mut self.context, self.document, viewport, &mut self.shaper);
        } else {
            product_frame(&mut self.context, self.document, viewport, &mut self.shaper);
        }
        self.context.rebuild_hit_test(self.document);
    }

    fn id(&self) -> StableNodeId {
        self.select.stable_id()
    }

    fn field(&self) -> LayoutBox {
        self.context.world().layout_box(self.id()).unwrap()
    }

    fn menu(&self) -> crate::SelectMenuGeometry {
        self.context.open_select_menu(self.id()).unwrap()
    }

    /// The row the menu paints at `(x, y)`, the option the pointer picks
    /// there, and whether hit testing gives the point to the field.
    fn at(&self, x: f32, y: f32) -> (Option<usize>, Option<usize>, bool) {
        // The field reads as its laid-out box, wherever its menu hangs.
        let spoken = self
            .context
            .world()
            .project_accessibility(self.document)
            .into_iter()
            .find(|node| node.id == self.id())
            .unwrap()
            .bounds;
        assert_eq!(spoken, self.field());
        let menu = self.menu();
        let painted = menu.viewport.contains(x, y).then(|| {
            menu.options
                .iter()
                .position(|option| option.bounds.contains(x, y))
        });
        let picked = crate::select::select_option_at(&menu, x, y);
        let hit = self.context.world().hit_test(self.document, x, y) == Some(self.id());
        (painted.flatten(), picked, hit)
    }
}

fn row_centre(menu: &crate::SelectMenuGeometry, row: usize) -> (f32, f32) {
    let bounds = menu.options[row].bounds;
    (
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    )
}

#[test]
fn issue200_a_select_menu_follows_the_viewport_and_hits_where_it_paints() {
    // A tall window: the menu hangs below the field.
    let mut page = SelectPage::new(1000.0);
    let field = page.field();
    let below = page.menu();
    assert_eq!(below.surface.y, field.y + field.height, "{below:?}");
    let low_row = row_centre(&below, 3);
    assert_eq!(page.at(low_row.0, low_row.1), (Some(3), Some(3), true));

    // The window shrinks under the open menu. Nothing about the field
    // changes; the menu flips above it against the window it is in now.
    page.frame(600.0, true);
    assert_eq!(page.field(), field);
    let above = page.menu();
    assert_eq!(above.surface.y + above.surface.height, field.y, "{above:?}");
    assert!(above.surface.y >= 0.0);
    // The row painted under a point is the one the pointer picks there and
    // the one hit testing gives to the field; where the menu was, it is not.
    let high_row = row_centre(&above, 3);
    assert_eq!(page.at(high_row.0, high_row.1), (Some(3), Some(3), true));
    assert_eq!(page.at(low_row.0, low_row.1), (None, None, false));
    // The same window laid out cold places the same menu.
    assert_eq!(SelectPage::new(600.0).menu(), above);

    // Back: it hangs below again, and the pointer follows it.
    page.frame(1000.0, true);
    assert_eq!(page.menu(), below);
    assert_eq!(page.at(low_row.0, low_row.1), (Some(3), Some(3), true));
    assert_eq!(page.at(high_row.0, high_row.1), (None, None, false));
    // A press there chooses the row painted under it.
    assert!(
        page.context
            .activate_select_at(page.select, low_row.0, low_row.1)
            .unwrap()
    );
    assert_eq!(
        page.context
            .read(page.select, |select| select.value.clone())
            .unwrap()
            .as_deref(),
        Some("v3")
    );
}

/// Two windows of one application, each a document with its own viewport.
/// Each holds a `position: relative` card with a fixed box deep inside it
/// (a quarter of the viewport wide, in its bottom-right corner), an absolute
/// box with no positioned ancestor (the initial containing block's
/// bottom-left corner), and a scrolled list with a fixed box inside it (the
/// window's top-left corner).
struct Windows {
    context: AppContext,
    shaper: NanaTextEngineShaper,
    viewports: [LayoutViewport; 2],
}

const WINDOWS: [DocumentId; 2] = [DocumentId::new(1).unwrap(), DocumentId::new(2).unwrap()];
/// Each window's node ids start here.
const WINDOW_BASE: [u64; 2] = [1, 1_000];
/// Node ids within a window, after its document and page.
const PINNED: u64 = 4;
const CORNER: u64 = 5;
const LIST: u64 = 6;
const LIST_PIN: u64 = 8;

fn window_node(window: usize, offset: u64) -> StableNodeId {
    super::reflow_oracle::node(WINDOW_BASE[window] + offset)
}

fn windows_queues() -> Vec<MutationQueue> {
    use crate::InteractionState;
    use nana_ui_core::OverflowSpec;
    let mut queues = Vec::new();
    for (window, document) in WINDOWS.into_iter().enumerate() {
        let (mut b, page) = Builder::page(document, WINDOW_BASE[window], 300.0);
        let card = b.element(page, card(PositionSpec::Relative, 300.0));
        let wrapper = b.element(card, wrapper(PositionSpec::Static));
        let pinned = b.element(
            wrapper,
            LayoutStyle {
                position: PositionSpec::Fixed,
                offset_right: Some(LengthSpec::Px(0.0)),
                offset_bottom: Some(LengthSpec::Px(0.0)),
                width: Some(LengthSpec::Percent(25.0)),
                height: Some(LengthSpec::Px(20.0)),
                ..LayoutStyle::default()
            },
        );
        let corner = b.element(
            page,
            LayoutStyle {
                position: PositionSpec::Absolute,
                offset_left: Some(LengthSpec::Px(0.0)),
                offset_bottom: Some(LengthSpec::Px(0.0)),
                ..sized(30.0, 20.0)
            },
        );
        let list = b.element(
            page,
            LayoutStyle {
                overflow_y: OverflowSpec::Scroll,
                direction: Some(FlexDirection::Column),
                ..sized(200.0, 100.0)
            },
        );
        b.element(list, sized(200.0, 400.0));
        let list_pin = b.element(
            list,
            LayoutStyle {
                position: PositionSpec::Fixed,
                offset_left: Some(LengthSpec::Px(0.0)),
                offset_top: Some(LengthSpec::Px(0.0)),
                ..sized(40.0, 10.0)
            },
        );
        assert_eq!(
            [pinned, corner, list, list_pin],
            [PINNED, CORNER, LIST, LIST_PIN].map(|offset| window_node(window, offset))
        );
        for id in [pinned, corner, list_pin] {
            b.queue.set_interaction(
                id,
                InteractionState {
                    pointer_events: true,
                    focusable: false,
                },
            );
        }
        queues.push(b.queue);
    }
    queues
}

impl Windows {
    fn new(viewports: [LayoutViewport; 2]) -> Self {
        let mut context = AppContext::new();
        for queue in windows_queues() {
            context.commit_mutations(queue).unwrap();
        }
        let mut shaper = bundled_face_shaper();
        // A window's first frame lays out against the viewport it opens at.
        for (document, viewport) in WINDOWS.into_iter().zip(viewports) {
            resize_frame(&mut context, document, viewport, &mut shaper);
            context.rebuild_hit_test(document);
        }
        Self {
            context,
            shaper,
            viewports,
        }
    }

    fn rect(&self, window: usize, offset: u64) -> (f32, f32, f32, f32) {
        let b = self
            .context
            .world()
            .layout_box(window_node(window, offset))
            .unwrap();
        (b.x, b.y, b.width, b.height)
    }

    fn boxes(&self, window: usize) -> Vec<(StableNodeId, Option<LayoutBox>)> {
        let world = self.context.world();
        world
            .document_order(WINDOWS[window])
            .into_iter()
            .map(|id| (id, world.layout_box(id)))
            .collect()
    }

    /// Every published result of `window`, by its geometry.
    fn results(&self, window: usize) -> Vec<(StableNodeId, Option<crate::LayoutResult>)> {
        let world = self.context.world();
        world
            .document_order(WINDOWS[window])
            .into_iter()
            .map(|id| (id, world.layout_result(id).cloned()))
            .collect()
    }

    /// The node hit testing gives `(x, y)` of `window`, and the bounds
    /// accessibility reads for node `offset` of it.
    fn hit_and_spoken(&self, window: usize, offset: u64, (x, y): (f32, f32)) -> (u64, LayoutBox) {
        let world = self.context.world();
        let hit = world.hit_test(WINDOWS[window], x, y).unwrap();
        let id = window_node(window, offset);
        let spoken = world
            .project_accessibility_nodes(&[id])
            .into_iter()
            .find(|node| node.id == id)
            .unwrap()
            .bounds;
        (hit.get() - WINDOW_BASE[window], spoken)
    }

    /// Each fixed or initial-containing-block box of `window` sits where its
    /// viewport puts it, and hit testing and accessibility read that box.
    fn assert_placed(&self, window: usize) {
        let LayoutViewport { width, height } = self.viewports[window];
        let pinned = (width * 0.75, height - 20.0, width * 0.25, 20.0);
        let corner = (0.0, height - 20.0, 30.0, 20.0);
        let list_pin = (0.0, 0.0, 40.0, 10.0);
        assert_eq!(self.rect(window, PINNED), pinned, "window {window}");
        assert_eq!(self.rect(window, CORNER), corner, "window {window}");
        assert_eq!(self.rect(window, LIST_PIN), list_pin, "window {window}");
        for (offset, (x, y, w, h)) in [(PINNED, pinned), (CORNER, corner), (LIST_PIN, list_pin)] {
            let (hit, spoken) = self.hit_and_spoken(window, offset, (x + w / 2.0, y + h / 2.0));
            assert_eq!(hit, offset, "window {window}: hit at node {offset}");
            assert_eq!(
                (spoken.x, spoken.y, spoken.width, spoken.height),
                (x, y, w, h),
                "window {window}: accessibility bounds of node {offset}"
            );
        }
    }

    fn resize(&mut self, window: usize, width: f32, height: f32) -> WorkCounters {
        self.viewports[window] = LayoutViewport::new(width, height);
        let counters = resize_frame(
            &mut self.context,
            WINDOWS[window],
            self.viewports[window],
            &mut self.shaper,
        );
        self.context.rebuild_hit_test(WINDOWS[window]);
        counters
    }
}

#[test]
fn issue200_fixed_boxes_hang_from_their_own_windows_viewport() {
    let mut windows = Windows::new([
        LayoutViewport::new(800.0, 600.0),
        LayoutViewport::new(500.0, 700.0),
    ]);
    windows.assert_placed(0);
    windows.assert_placed(1);

    // The list scrolls: the fixed box inside it stays on the window, for the
    // pointer and for accessibility, as it is painted.
    let mut queue = MutationQueue::new();
    queue.set_scroll_offset(
        window_node(0, LIST),
        crate::ScrollOffset { x: 0.0, y: 50.0 },
    );
    windows.context.commit_mutations(queue).unwrap();
    windows.context.rebuild_hit_test(WINDOWS[0]);
    assert_eq!(
        windows.context.world().scroll_offset(window_node(0, LIST)),
        Some(crate::ScrollOffset { x: 0.0, y: 50.0 })
    );
    windows.assert_placed(0);

    // Each window resizes on its own: its boxes follow its viewport, and the
    // other window runs no layout pass, keeps every box and result, and
    // keeps the viewport it was laid out at.
    for (window, (width, height)) in [(0, (1000.0, 700.0)), (1, (360.0, 480.0))] {
        let other = 1 - window;
        let boxes = windows.boxes(other);
        let results = windows.results(other);
        let stats = windows.context.layout_frontier_stats(WINDOWS[other]);
        windows.resize(window, width, height);
        windows.assert_placed(window);
        assert_eq!(windows.boxes(other), boxes, "window {other}'s boxes");
        assert_eq!(windows.results(other), results, "window {other}'s results");
        assert_eq!(
            windows.context.layout_frontier_stats(WINDOWS[other]),
            stats,
            "no layout pass ran for window {other}"
        );
        assert_eq!(
            windows.context.world().document_viewport(WINDOWS[other]),
            Some(windows.viewports[other])
        );
        assert!(windows.context.take_system_work().is_empty());
        assert!(
            windows
                .context
                .take_layout_frontier_seeds(WINDOWS[other])
                .is_empty()
        );
        windows.assert_placed(other);
    }

    // The same windows opened at these sizes place everything the same.
    let mut cold = Windows::new(windows.viewports);
    let mut queue = MutationQueue::new();
    queue.set_scroll_offset(
        window_node(0, LIST),
        crate::ScrollOffset { x: 0.0, y: 50.0 },
    );
    cold.context.commit_mutations(queue).unwrap();
    for document in WINDOWS {
        assert_matches_cold(&mut windows.context, &mut cold.context, document);
    }
}
