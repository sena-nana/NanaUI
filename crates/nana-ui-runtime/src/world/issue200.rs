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
//!
//! An open Select, Dropdown or SearchDropdown menu reads as a combobox's
//! list box: one option per row the menu shows, at the bounds its menu
//! geometry paints and hits, following the menu when the window flips it.

#![cfg(test)]

use nana_ui_core::{
    AnchoredMenuPlacement, FlexDirection, LayoutStyle, LengthSpec, PositionSpec, WorkCounters,
};

use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, product_frame, resize_frame, styled,
};
use super::{DocumentId, StableNodeId};
use crate::{
    AccessibilityRole, AnchoredActionMenu, AppContext, ComponentView, ContextMenu, ContextMenuItem,
    Entity, LayoutBox, LayoutViewport, MutationQueue, NanaTextEngineShaper, OverlayHost,
    resolve_anchored_origin,
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
        Self::with_options(
            height,
            (0..8).map(|n| crate::SelectOption::new(format!("v{n}"), format!("Option {n}"))),
        )
    }

    /// The page with an open Select of `options`, holding `v0`.
    fn with_options(height: f32, options: impl IntoIterator<Item = crate::SelectOption>) -> Self {
        use crate::{Select, Stack};
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
            .create_detached_component(document, Select::new(Some("v0")).options(options))
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

// An open Select, Dropdown or SearchDropdown menu reads as a WAI-ARIA
// combobox's list box: a child of the field holding one option per row the
// menu shows, each known by its value, at the bounds the menu paints and hits
// it. The field says it is expanded and which option the keyboard is on;
// closing the menu takes the list box away, and choosing an option through
// accessibility is a press on its row.

/// Every accessibility node of `document`, by id.
fn spoken(
    context: &AppContext,
    document: DocumentId,
) -> std::collections::HashMap<StableNodeId, crate::AccessibilityNode> {
    context
        .world()
        .project_accessibility(document)
        .into_iter()
        .map(|node| (node.id, node))
        .collect()
}

/// `field`'s node, the list box its open menu shows and the options in it,
/// in row order. The field is an expanded combobox that owns the list box.
fn listbox(
    nodes: &std::collections::HashMap<StableNodeId, crate::AccessibilityNode>,
    field: StableNodeId,
) -> (
    crate::AccessibilityNode,
    crate::AccessibilityNode,
    Vec<crate::AccessibilityNode>,
) {
    let combo = nodes[&field].clone();
    assert_eq!(combo.role, AccessibilityRole::ComboBox);
    assert_eq!(combo.expanded, Some(true), "{combo:?}");
    assert_eq!(combo.selected, None, "a combobox is not itself chosen");
    let id = crate::virtual_listbox_id(field).unwrap();
    assert_eq!(combo.children, [id]);
    let listbox = nodes[&id].clone();
    assert_eq!(listbox.role, AccessibilityRole::ListBox);
    assert_eq!(listbox.parent, Some(field));
    let options = listbox
        .children
        .iter()
        .map(|option| nodes[option].clone())
        .collect::<Vec<_>>();
    for option in &options {
        assert_eq!(option.role, AccessibilityRole::ListBoxOption);
        assert_eq!(option.parent, Some(id));
        assert_eq!(crate::decode_virtual_menu_item(option.id).unwrap().0, field);
    }
    (combo, listbox, options)
}

fn centre(bounds: LayoutBox) -> (f32, f32) {
    (
        bounds.x + bounds.width / 2.0,
        bounds.y + bounds.height / 2.0,
    )
}

/// The menu row an option node stands for now.
fn row_of(context: &AppContext, field: StableNodeId, option: StableNodeId) -> usize {
    let (owner, slot) = crate::decode_virtual_menu_item(option).unwrap();
    assert_eq!(owner, field);
    context.world().virtual_menu_row_index(field, slot).unwrap()
}

/// Each option reads at the row the menu paints under it, and the pointer
/// there picks that row, unless it is disabled, and hits the field.
fn assert_reads_where_it_paints(page: &SelectPage, options: &[crate::AccessibilityNode]) {
    let menu = page.menu();
    for option in options {
        let row = row_of(&page.context, page.id(), option.id);
        let painted = super::intersect_layout_boxes(menu.options[row].bounds, menu.viewport);
        assert_eq!(Some(option.bounds), painted, "row {row}");
        let (x, y) = centre(option.bounds);
        assert_eq!(
            page.at(x, y),
            (Some(row), (!option.disabled).then_some(row), true),
            "row {row}"
        );
    }
}

#[test]
fn issue200_an_open_select_menu_reads_as_a_listbox_of_its_rows() {
    let page = SelectPage::with_options(
        1000.0,
        (0..8).map(|n| {
            let option = crate::SelectOption::new(format!("v{n}"), format!("Option {n}"));
            match n {
                1 => option.hint("first"),
                2 => option.disabled(true),
                _ => option,
            }
        }),
    );
    let nodes = spoken(&page.context, page.document);
    let (field, listbox, options) = listbox(&nodes, page.id());
    let menu = page.menu();
    assert_eq!(listbox.bounds, menu.surface);
    assert_eq!(listbox.set_size, Some(8));
    assert!(!listbox.multiselectable);
    assert_eq!(options.len(), 8);
    for (n, option) in options.iter().enumerate() {
        assert_eq!(row_of(&page.context, page.id(), option.id), n);
        assert_eq!(
            option.label.as_deref(),
            Some(format!("Option {n}").as_str())
        );
        assert_eq!(option.description.as_deref(), (n == 1).then_some("first"));
        assert_eq!(option.selected, Some(n == 0), "v0 is chosen");
        assert_eq!(option.disabled, n == 2);
        assert_eq!(option.position_in_set, Some(n as u32 + 1));
        assert_eq!(option.set_size, Some(8));
        assert!(!option.focused, "the focus stays on the field");
    }
    // The highlight opens on the chosen option: the field's active
    // descendant.
    assert_eq!(field.active_descendant, Some(options[0].id));
    assert_reads_where_it_paints(&page, &options);
}

#[test]
fn issue200_a_select_listbox_follows_its_menu_and_its_highlight() {
    let mut page = SelectPage::new(1000.0);
    // What a host holds: the full tree once, then the deltas each frame
    // drains.
    let mut held = spoken(&page.context, page.document);
    let (_, _, below) = listbox(&held, page.id());
    let mut follow = |page: &mut SelectPage, height: f32, resized: bool| {
        let work = super::reflow_oracle::accessible_frame(
            &mut page.context,
            page.document,
            LayoutViewport::new(400.0, height),
            &mut page.shaper,
            resized,
        );
        page.context.rebuild_hit_test(page.document);
        let delta = page.context.world().project_accessibility_delta(&work);
        for id in delta.removed {
            held.remove(&id);
        }
        held.extend(delta.updated.into_iter().map(|node| (node.id, node)));
        let fresh = spoken(&page.context, page.document);
        let (field, listbox, options) = listbox(&fresh, page.id());
        for node in std::iter::once(&field)
            .chain(std::iter::once(&listbox))
            .chain(&options)
        {
            assert_eq!(held.get(&node.id), Some(node), "the delta carries it");
        }
        (field, options)
    };

    // The window shrinks: the menu flips above the field, and its options
    // read there.
    let (_, above) = follow(&mut page, 600.0, true);
    assert_eq!(
        above.iter().map(|option| option.id).collect::<Vec<_>>(),
        below.iter().map(|option| option.id).collect::<Vec<_>>()
    );
    let field = page.field();
    assert!(above[7].bounds.y + above[7].bounds.height <= field.y);
    assert!(below[0].bounds.y >= field.y + field.height);
    assert_reads_where_it_paints(&page, &above);

    // The keyboard moves the highlight: only paint changes, and the field's
    // active descendant follows it.
    page.context
        .update_component(page.select, |select, _| select.highlight_delta(1))
        .unwrap();
    let (field, options) = follow(&mut page, 600.0, false);
    assert_eq!(field.active_descendant, Some(options[1].id));

    // Back: below again.
    let (_, options) = follow(&mut page, 1000.0, true);
    assert_eq!(
        options
            .iter()
            .map(|option| option.bounds)
            .collect::<Vec<_>>(),
        below.iter().map(|option| option.bounds).collect::<Vec<_>>()
    );
    assert_reads_where_it_paints(&page, &options);
}

#[test]
fn issue200_a_select_listbox_reads_only_the_rows_its_menu_shows() {
    // Forty options under a 500 px spacer in a 640 px window: the menu opens
    // above the field, capped to the room there, and scrolls its rows.
    let mut page = SelectPage::with_options(
        640.0,
        (0..40).map(|n| crate::SelectOption::new(format!("v{n}"), format!("Option {n}"))),
    );
    let check = |page: &SelectPage| {
        let menu = page.menu();
        assert!(menu.max_scroll > 0.0, "{menu:?}");
        let (_, listbox, options) = listbox(&spoken(&page.context, page.document), page.id());
        assert_eq!(listbox.set_size, Some(40));
        let shown = menu
            .options
            .iter()
            .enumerate()
            .filter(|(_, row)| super::intersect_layout_boxes(row.bounds, menu.viewport).is_some())
            .map(|(row, _)| row)
            .collect::<Vec<_>>();
        assert!(shown.len() < 40, "{shown:?}");
        assert_eq!(
            options
                .iter()
                .map(|option| row_of(&page.context, page.id(), option.id))
                .collect::<Vec<_>>(),
            shown
        );
        for option in &options {
            let row = row_of(&page.context, page.id(), option.id);
            assert_eq!(option.position_in_set, Some(row as u32 + 1));
            assert_eq!(option.set_size, Some(40));
        }
        // A row the menu cuts reads as the part it shows.
        assert_reads_where_it_paints(page, &options);
        options
    };
    let top = check(&page);
    assert_eq!(row_of(&page.context, page.id(), top[0].id), 0);

    // Scrolled on: rows that left the menu leave the list box, and a row
    // still shown keeps its node.
    page.context
        .update_component(page.select, |select, _| {
            select.menu_scroll.offset = Some(300.0)
        })
        .unwrap();
    page.frame(640.0, false);
    let scrolled = check(&page);
    assert_ne!(row_of(&page.context, page.id(), scrolled[0].id), 0);
    let kept = scrolled
        .iter()
        .filter(|option| top.iter().any(|before| before.id == option.id))
        .count();
    assert!(kept > 0 && kept < scrolled.len(), "{kept}");
}

#[test]
fn issue200_a_select_listbox_reads_only_what_the_page_leaves_of_its_menu() {
    use crate::{NodeStyle, ScrollAxes, ScrollOffset, ScrollView, Select, SelectOption, Stack};
    // A 400x300 page that scrolls, 100 px of space, a Select, and a tall
    // block under it.
    let document = DocumentId::new(1).unwrap();
    let mut context = AppContext::new();
    context.set_reduced_motion(true);
    let mut style = NodeStyle::default();
    {
        let layout = std::sync::Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Px(400.0));
        layout.height = Some(LengthSpec::Px(300.0));
    }
    let page = context
        .create_component(document, ScrollView::new(ScrollAxes::Vertical).style(style))
        .unwrap();
    let block = |height: f32| {
        Stack::column(0.0).with_layout(move |layout| {
            layout.width = Some(LengthSpec::Fill);
            layout.height = Some(LengthSpec::Px(height));
        })
    };
    let spacer = context
        .create_detached_component(document, block(100.0))
        .unwrap();
    context.append_child(page, spacer).unwrap();
    let select = context
        .create_detached_component(
            document,
            Select::new(Some("v0"))
                .options((0..8).map(|n| SelectOption::new(format!("v{n}"), format!("Option {n}")))),
        )
        .unwrap();
    context.append_child(page, select).unwrap();
    let tail = context
        .create_detached_component(document, block(900.0))
        .unwrap();
    context.append_child(page, tail).unwrap();
    let mut shaper = bundled_face_shaper();
    let viewport = LayoutViewport::new(400.0, 300.0);
    product_frame(&mut context, document, viewport, &mut shaper);
    assert!(context.toggle_select(select).unwrap());
    product_frame(&mut context, document, viewport, &mut shaper);
    // The page scrolls under the open menu with no layout: the menu goes
    // with its field, and the page's edge cuts its top rows away.
    assert!(
        context
            .scroll_to(page, ScrollOffset { x: 0.0, y: 180.0 })
            .unwrap()
    );
    context.rebuild_hit_test(document);
    let id = select.stable_id();
    let menu = context.open_select_menu(id).unwrap();
    let window = LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 300.0,
    };
    // Where the window shows each row: its part inside the menu, scrolled
    // with the page, inside the page.
    let shown = menu
        .options
        .iter()
        .map(|row| {
            super::intersect_layout_boxes(row.bounds, menu.viewport)
                .map(|part| LayoutBox {
                    y: part.y - 180.0,
                    ..part
                })
                .and_then(|part| super::intersect_layout_boxes(part, window))
        })
        .collect::<Vec<_>>();
    assert!(shown[0].is_none(), "the page hides the first row: {menu:?}");
    assert!(
        shown.iter().flatten().any(|part| part.y == 0.0),
        "and cuts one: {shown:?}"
    );
    assert!(shown.iter().any(Option::is_some));
    let (_, _, options) = listbox(&spoken(&context, document), id);
    assert_eq!(
        options
            .iter()
            .map(|option| (row_of(&context, id, option.id), option.bounds))
            .collect::<Vec<_>>(),
        shown
            .iter()
            .enumerate()
            .filter_map(|(row, bounds)| Some((row, (*bounds)?)))
            .collect::<Vec<_>>()
    );
    // Each reads where the pointer picks it and hits the field.
    for option in &options {
        let row = row_of(&context, id, option.id);
        let (x, y) = centre(option.bounds);
        assert_eq!(context.world().hit_test(document, x, y), Some(id));
        let (x, y) = context.menu_point(id, x, y);
        assert_eq!(crate::select::select_option_at(&menu, x, y), Some(row));
    }
}

#[test]
fn issue200_a_closed_select_has_no_listbox() {
    let mut page = SelectPage::new(1000.0);
    let listbox_id = crate::virtual_listbox_id(page.id()).unwrap();
    let options = listbox(&spoken(&page.context, page.document), page.id()).2;
    assert!(page.context.toggle_select(page.select).unwrap());
    page.frame(1000.0, false);
    let nodes = spoken(&page.context, page.document);
    let field = &nodes[&page.id()];
    assert_eq!(field.expanded, Some(false));
    assert_eq!(field.active_descendant, None);
    assert!(field.children.is_empty(), "{field:?}");
    assert!(!nodes.contains_key(&listbox_id));
    assert!(options.iter().all(|option| !nodes.contains_key(&option.id)));
    // A row id from the open menu chooses nothing now.
    let (_, slot) = crate::decode_virtual_menu_item(options[3].id).unwrap();
    assert!(
        !page
            .context
            .apply_accessibility_action(
                page.document,
                crate::AccessibilityActionRequest {
                    target: options[3].id,
                    action: crate::AccessibilityAction::ActivateMenuItem {
                        menu: page.id(),
                        slot,
                    },
                },
            )
            .unwrap()
    );
    assert_eq!(
        page.context
            .read(page.select, |select| select.value.clone())
            .unwrap()
            .as_deref(),
        Some("v0")
    );
}

#[test]
fn issue200_an_option_chosen_through_accessibility_is_a_press_on_its_row() {
    for action in ["activate", "click"] {
        let mut page = SelectPage::new(1000.0);
        let options = listbox(&spoken(&page.context, page.document), page.id()).2;
        let (_, slot) = crate::decode_virtual_menu_item(options[3].id).unwrap();
        assert!(
            page.context
                .apply_accessibility_action(
                    page.document,
                    crate::AccessibilityActionRequest {
                        target: options[3].id,
                        action: if action == "click" {
                            crate::AccessibilityAction::Click
                        } else {
                            crate::AccessibilityAction::ActivateMenuItem {
                                menu: page.id(),
                                slot,
                            }
                        },
                    },
                )
                .unwrap(),
            "{action}"
        );
        assert_eq!(
            page.context
                .read(page.select, |select| (select.value.clone(), select.opened))
                .unwrap(),
            (Some("v3".into()), false),
            "{action}"
        );
        page.frame(1000.0, false);
        let field = &spoken(&page.context, page.document)[&page.id()];
        assert_eq!(field.expanded, Some(false));
        assert_eq!(field.value.as_deref(), Some("Option 3"));
    }
}

#[test]
fn issue200_a_search_dropdown_listbox_keeps_its_options_while_the_query_filters() {
    use crate::{SearchDropdown, SearchDropdownOption, Stack};
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
    let field = context
        .create_detached_component(
            document,
            SearchDropdown::new(Some("cherry")).options(
                ["apple", "apricot", "banana", "blueberry", "cherry"]
                    .map(|fruit| SearchDropdownOption::new(fruit, fruit.to_uppercase())),
            ),
        )
        .unwrap();
    context.append_child(root, field).unwrap();
    let mut shaper = bundled_face_shaper();
    let viewport = LayoutViewport::new(400.0, 600.0);
    let mut frame = |context: &mut AppContext| {
        product_frame(context, document, viewport, &mut shaper);
        context.rebuild_hit_test(document);
    };
    frame(&mut context);
    assert!(context.toggle_search_dropdown(field).unwrap());
    frame(&mut context);
    let id = field.stable_id();
    let labels = |options: &[crate::AccessibilityNode]| {
        options
            .iter()
            .map(|option| option.label.as_deref().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let (_, _, all) = listbox(&spoken(&context, document), id);
    assert_eq!(
        labels(&all),
        ["APPLE", "APRICOT", "BANANA", "BLUEBERRY", "CHERRY"]
    );
    assert_eq!(
        all.iter().map(|option| option.selected).collect::<Vec<_>>(),
        [false, false, false, false, true].map(Some)
    );

    // The query keeps the berries and the banana: the rows it hides leave the
    // list box, and the ones it keeps keep their nodes, numbered among the
    // matches.
    context
        .update_component(field, |dropdown, _| {
            dropdown.set_query("b");
        })
        .unwrap();
    frame(&mut context);
    let (combo, listbox_node, matches) = listbox(&spoken(&context, document), id);
    assert_eq!(labels(&matches), ["BANANA", "BLUEBERRY"]);
    assert_eq!(
        matches.iter().map(|option| option.id).collect::<Vec<_>>(),
        [all[2].id, all[3].id]
    );
    assert_eq!(listbox_node.set_size, Some(2));
    assert_eq!(
        matches
            .iter()
            .map(|option| (option.position_in_set, option.set_size))
            .collect::<Vec<_>>(),
        [(Some(1), Some(2)), (Some(2), Some(2))]
    );
    // The first match takes the highlight.
    assert_eq!(combo.active_descendant, Some(all[2].id));
    let menu = context.open_select_menu(id).unwrap();
    for (row, option) in matches.iter().enumerate() {
        assert_eq!(option.bounds, menu.options[row].bounds);
        let (x, y) = centre(option.bounds);
        assert_eq!(crate::select::select_option_at(&menu, x, y), Some(row));
    }

    // Chosen through accessibility, the second match is the blueberry, not
    // the second option.
    assert!(
        context
            .apply_accessibility_action(
                document,
                crate::AccessibilityActionRequest {
                    target: matches[1].id,
                    action: crate::AccessibilityAction::Click,
                },
            )
            .unwrap()
    );
    assert_eq!(
        context
            .read(field, |dropdown| dropdown.value.clone())
            .unwrap()
            .as_deref(),
        Some("blueberry")
    );
    frame(&mut context);
    let nodes = spoken(&context, document);
    assert_eq!(nodes[&id].expanded, Some(false));
    assert!(!nodes.contains_key(&crate::virtual_listbox_id(id).unwrap()));
}

#[test]
fn issue200_a_multiple_dropdown_listbox_reads_its_checked_options_as_chosen() {
    use crate::{Dropdown, DropdownOption, Stack};
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
    let field = context
        .create_detached_component(
            document,
            Dropdown::multiple(["b"])
                .options(["a", "b", "c"].map(|value| DropdownOption::new(value, value))),
        )
        .unwrap();
    context.append_child(root, field).unwrap();
    let mut shaper = bundled_face_shaper();
    let viewport = LayoutViewport::new(400.0, 600.0);
    product_frame(&mut context, document, viewport, &mut shaper);
    assert!(context.toggle_dropdown(field).unwrap());
    product_frame(&mut context, document, viewport, &mut shaper);
    let (_, listbox_node, options) = listbox(&spoken(&context, document), field.stable_id());
    assert!(listbox_node.multiselectable);
    assert_eq!(
        options
            .iter()
            .map(|option| option.selected)
            .collect::<Vec<_>>(),
        [false, true, false].map(Some)
    );
    // Choosing one checks it and keeps the menu open, as a press does.
    assert!(
        context
            .apply_accessibility_action(
                document,
                crate::AccessibilityActionRequest {
                    target: options[2].id,
                    action: crate::AccessibilityAction::Click,
                },
            )
            .unwrap()
    );
    product_frame(&mut context, document, viewport, &mut shaper);
    let (_, _, options) = listbox(&spoken(&context, document), field.stable_id());
    assert_eq!(
        options
            .iter()
            .map(|option| option.selected)
            .collect::<Vec<_>>(),
        [false, true, true].map(Some)
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

#[test]
fn issue200_an_edit_pending_in_one_window_waits_for_that_windows_frame() {
    let mut windows = Windows::new([
        LayoutViewport::new(800.0, 600.0),
        LayoutViewport::new(500.0, 700.0),
    ]);
    let grow = |windows: &mut Windows, width: f32| {
        let mut queue = MutationQueue::new();
        queue.set_style(
            window_node(1, CORNER),
            styled(LayoutStyle {
                position: PositionSpec::Absolute,
                offset_left: Some(LengthSpec::Px(0.0)),
                offset_bottom: Some(LengthSpec::Px(0.0)),
                ..sized(width, 20.0)
            }),
        );
        windows.context.commit_mutations(queue).unwrap();
    };
    let draw = |windows: &mut Windows, window: usize| {
        product_frame(
            &mut windows.context,
            WINDOWS[window],
            windows.viewports[window],
            &mut windows.shaper,
        );
    };
    let before = windows.boxes(0);
    // Window 1's corner grows; window 0 draws its next frame first. It has
    // nothing of its own to lay out, and lays out nothing for window 1.
    grow(&mut windows, 50.0);
    let passes = windows.context.layout_invocations();
    draw(&mut windows, 0);
    assert_eq!(
        windows.context.layout_invocations(),
        passes,
        "no pass for window 1's edit"
    );
    assert_eq!(windows.boxes(0), before);
    assert_eq!(
        windows.rect(1, CORNER).2,
        30.0,
        "window 1 has not drawn yet"
    );
    // Window 1's own frame lays the edit out.
    draw(&mut windows, 1);
    assert_eq!(windows.rect(1, CORNER), (0.0, 680.0, 50.0, 20.0));
    assert!(windows.context.take_system_work().is_empty());

    // A host that drains the whole context and hands window 0's pass every
    // seed: the pass gives window 1's back rather than dropping them.
    grow(&mut windows, 70.0);
    let seeds = windows.context.take_system_work().layout_frontier_seeds;
    assert!(!seeds.is_empty());
    windows
        .context
        .layout_document_with_frontier(WINDOWS[0], windows.viewports[0], &seeds)
        .unwrap();
    assert_eq!(windows.boxes(0), before);
    draw(&mut windows, 1);
    assert_eq!(windows.rect(1, CORNER), (0.0, 680.0, 70.0, 20.0));
    assert!(windows.context.take_system_work().is_empty());
}
