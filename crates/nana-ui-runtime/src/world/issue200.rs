//! Issue #200: anchored overlays share one coordinate authority.
//!
//! A pointer menu and an anchored action menu state an anchor
//! ([`nana_ui_core::AnchorSpec`]). Layout places them, at the size they
//! measure and against the real viewport, through the same
//! [`crate::resolve_anchored_surface`] that triggered menus, hover cards,
//! popovers and the select drop-down use. They flip to the side that fits,
//! stay in the window (and in the room an owner gives), follow a viewport
//! change with no reprojection, and hit testing and accessibility read the
//! box layout gave them.

#![cfg(test)]

use nana_ui_core::AnchoredMenuPlacement;

use super::reflow_oracle::{bundled_face_shaper, product_frame, resize_frame};
use super::{DocumentId, StableNodeId};
use crate::{
    AnchoredActionMenu, AppContext, ComponentView, ContextMenu, ContextMenuItem, Entity, LayoutBox,
    LayoutViewport, NanaTextEngineShaper, OverlayHost, resolve_anchored_origin,
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
