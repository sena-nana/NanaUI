//! Directional 2D focus navigation, FocusPolicy, unified Keyboard/Gamepad
//! navigation intents, and focus diagnostics counters (#111).

use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nana_ui_core::{DirSpec, DisplaySpec, LayoutStyle, LengthSpec, OverflowSpec, VisibilitySpec};
use nana_ui_input::{
    InputModifiers, InputPayload, KeyInput, KeyState, LogicalKey, NavigationInput,
    NavigationIntent, PhysicalKey, PointerInput, PointerPhase, gamepad_dpad_navigation,
};
use nana_ui_runtime::view::{button, column, entity_ref, row, widget, with_refs};
use nana_ui_runtime::{
    Activate, AppContext, Button, Dialog, DirectionalFocusScope, DocumentId, FocusDirection,
    FocusNeighbors, FocusPolicy, HeadlessInput, InputModality, InputRouteOutcome, LayoutViewport,
    MeasureTextShaper, ModalSlots, MutationQueue, OverlayHost, RovingFocusGroup, ScrollAxes,
    ScrollView, StableNodeId, Stack,
};

struct DirFixture {
    cx: AppContext,
    input: HeadlessInput,
    doc: DocumentId,
    root: StableNodeId,
}

fn layout_doc(cx: &mut AppContext, doc: DocumentId) {
    let nodes = cx.world().document_order(doc);
    cx.resolve_styles(&nodes).unwrap();
    cx.shape_text(&nodes, &mut MeasureTextShaper).unwrap();
    cx.layout_document(doc, LayoutViewport::new(800.0, 600.0))
        .unwrap();
    cx.rebuild_hit_test(doc);
}

fn sized_button(label: &'static str, width: f32, height: f32) -> nana_ui_runtime::view::El<Button> {
    let layout = Arc::new(LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        ..Default::default()
    });
    widget(Button::new(label).layout(layout))
}

impl DirFixture {
    fn new() -> Self {
        let mut cx = AppContext::new();
        let doc = DocumentId::new(1).unwrap();
        let root = cx
            .create_component(doc, Stack::column(0.0))
            .unwrap()
            .stable_id();
        let input = HeadlessInput::bind(&mut cx, doc);
        Self {
            cx,
            input,
            doc,
            root,
        }
    }

    fn layout(&mut self) {
        layout_doc(&mut self.cx, self.doc);
    }

    fn key(&mut self, key: &'static str) -> InputRouteOutcome {
        let out = self
            .input
            .route(
                &mut self.cx,
                InputPayload::Key(KeyInput {
                    physical: PhysicalKey(Cow::Borrowed(key)),
                    logical: LogicalKey(Cow::Borrowed(key)),
                    state: KeyState::Pressed,
                    repeat: false,
                    modifiers: InputModifiers::default(),
                }),
            )
            .unwrap();
        self.layout();
        out
    }

    fn nav(&mut self, nav: NavigationInput) -> InputRouteOutcome {
        let out = self.input.navigate(&mut self.cx, nav).unwrap();
        self.layout();
        out
    }

    fn click(&mut self, id: StableNodeId) {
        let bounds = self.cx.world().layout_box(id).unwrap();
        let x = bounds.x + bounds.width * 0.5;
        let y = bounds.y + bounds.height * 0.5;
        self.input
            .route(
                &mut self.cx,
                InputPayload::Pointer(PointerInput::mouse(PointerPhase::Down, x, y)),
            )
            .unwrap();
        self.input
            .route(
                &mut self.cx,
                InputPayload::Pointer(PointerInput::mouse(PointerPhase::Up, x, y)),
            )
            .unwrap();
    }

    fn focused(&self) -> Option<StableNodeId> {
        self.cx.world().focused(self.doc)
    }
}

#[test]
fn focus_policy_distinguishes_none_click_tab_strong_and_programmatic() {
    let mut f = DirFixture::new();
    let (_, [none_b, click_b, tab_b, strong_b, prog_b]) =
        f.cx.mount_view(f.root, || {
            let refs = [
                entity_ref::<Button>(),
                entity_ref::<Button>(),
                entity_ref::<Button>(),
                entity_ref::<Button>(),
                entity_ref::<Button>(),
            ];
            let view = column()
                .directional_focus(DirectionalFocusScope::new())
                .children((
                    button("none")
                        .focus_policy(FocusPolicy::None)
                        .entity_ref(refs[0]),
                    button("click")
                        .focus_policy(FocusPolicy::Click)
                        .entity_ref(refs[1]),
                    button("tab")
                        .focus_policy(FocusPolicy::Tab)
                        .entity_ref(refs[2]),
                    button("strong")
                        .focus_policy(FocusPolicy::Strong)
                        .entity_ref(refs[3]),
                    button("prog")
                        .focus_policy(FocusPolicy::Programmatic)
                        .entity_ref(refs[4]),
                ));
            with_refs(view, refs)
        })
        .unwrap();
    f.layout();

    // Programmatic focus: refused by None, accepted by Click/Tab/Strong/Programmatic.
    assert!(!f.cx.focus_node(f.doc, none_b.stable_id()).unwrap());
    assert!(f.cx.focus_node(f.doc, prog_b.stable_id()).unwrap());
    assert_eq!(f.focused(), Some(prog_b.stable_id()));
    assert_eq!(f.cx.world().focus_visible(f.doc), Some(prog_b.stable_id()));

    // Pointer click: Click & Strong focus; Tab & Programmatic & None do not.
    f.click(click_b.stable_id());
    assert_eq!(f.focused(), Some(click_b.stable_id()));
    assert_eq!(f.cx.world().focus_visible(f.doc), None);

    f.click(tab_b.stable_id());
    assert_eq!(
        f.focused(),
        None,
        "clicking FocusPolicy::Tab does not focus it"
    );

    f.click(prog_b.stable_id());
    assert_eq!(f.focused(), None);

    f.click(strong_b.stable_id());
    assert_eq!(f.focused(), Some(strong_b.stable_id()));

    // Sequential Tab: only Tab & Strong are visited.
    f.cx.clear_focus(f.doc).unwrap();
    assert!(f.key("Tab").handled);
    assert_eq!(f.focused(), Some(tab_b.stable_id()));
    assert!(f.key("Tab").handled);
    assert_eq!(f.focused(), Some(strong_b.stable_id()));
    assert!(f.key("Tab").handled);
    assert_eq!(f.focused(), Some(tab_b.stable_id()));

    // Directional ArrowDown from Tab skips Click/Programmatic and reaches Strong.
    assert!(f.key("ArrowDown").handled);
    assert_eq!(f.focused(), Some(strong_b.stable_id()));

    // Setting FocusPolicy::None on the currently focused node clears focus.
    f.cx.set_focus_policy(strong_b.stable_id(), Some(FocusPolicy::None))
        .unwrap();
    assert_eq!(f.focused(), None);

    // Disabled node resolves to FocusPolicy::None even when explicitly Strong.
    f.cx.set_focus_policy(tab_b.stable_id(), Some(FocusPolicy::Strong))
        .unwrap();
    f.cx.update_component(tab_b, |b, _| b.disabled = true)
        .unwrap();
    assert_eq!(
        f.cx.world().focus_policy(tab_b.stable_id()),
        FocusPolicy::None
    );
    assert!(!f.cx.focus_node(f.doc, tab_b.stable_id()).unwrap());
}

#[test]
fn directional_2d_grid_navigates_rows_columns_and_staggered_items() {
    let mut f = DirFixture::new();
    let (_, grid) =
        f.cx.mount_view(f.root, || {
            let refs = [
                entity_ref::<Button>(),
                entity_ref::<Button>(),
                entity_ref::<Button>(),
                entity_ref::<Button>(),
                entity_ref::<Button>(),
                entity_ref::<Button>(),
                entity_ref::<Button>(),
                entity_ref::<Button>(),
            ];
            let btn = |label: &'static str, r| sized_button(label, 80.0, 32.0).entity_ref(r);
            // 3 + 3 + 2 uneven grid:
            // [0] [1] [2]
            // [3] [4] [5]
            // [6] [7]
            let view = column()
                .gap(8.0)
                .directional_focus(DirectionalFocusScope::new())
                .children((
                    row().gap(8.0).children((
                        btn("0", refs[0]),
                        btn("1", refs[1]),
                        btn("2", refs[2]),
                    )),
                    row().gap(8.0).children((
                        btn("3", refs[3]),
                        btn("4", refs[4]),
                        btn("5", refs[5]),
                    )),
                    row()
                        .gap(8.0)
                        .children((btn("6", refs[6]), btn("7", refs[7]))),
                ));
            with_refs(view, refs)
        })
        .unwrap();
    f.layout();

    f.cx.focus_node(f.doc, grid[0].stable_id()).unwrap();
    assert!(f.key("ArrowRight").handled);
    assert_eq!(f.focused(), Some(grid[1].stable_id()));
    assert!(f.key("ArrowDown").handled);
    assert_eq!(f.focused(), Some(grid[4].stable_id()));
    assert!(f.key("ArrowRight").handled);
    assert_eq!(f.focused(), Some(grid[5].stable_id()));

    // Moving down from [5] (col 2, row 1) when row 2 has only [6] and [7]:
    // falls back deterministically to nearest column [7].
    assert!(f.key("ArrowDown").handled);
    assert_eq!(f.focused(), Some(grid[7].stable_id()));

    assert!(f.key("ArrowLeft").handled);
    assert_eq!(f.focused(), Some(grid[6].stable_id()));
    assert!(f.key("ArrowUp").handled);
    assert_eq!(f.focused(), Some(grid[3].stable_id()));
    assert!(f.key("ArrowUp").handled);
    assert_eq!(f.focused(), Some(grid[0].stable_id()));

    // Moving past top/left edge keeps focus and leaves key unhandled.
    assert!(!f.key("ArrowUp").handled);
    assert!(!f.key("ArrowLeft").handled);
    assert_eq!(f.focused(), Some(grid[0].stable_id()));
}

#[test]
fn explicit_neighbors_override_geometry_and_skip_disabled_hidden_inert_cycles() {
    let mut f = DirFixture::new();
    let (_, [a, b, c, d]) =
        f.cx.mount_view(f.root, || {
            let a = entity_ref::<Button>();
            let b = entity_ref::<Button>();
            let c = entity_ref::<Button>();
            let d = entity_ref::<Button>();
            // Deliberately no DirectionalFocusScope: explicit neighbors work on their own.
            // Forward EntityRef works even when `d` is declared after `a`.
            let view = column().gap(8.0).children((
                button("A").neighbor_right(d).entity_ref(a),
                button("B").neighbor_right(c).entity_ref(b),
                button("C").neighbor_right(d).entity_ref(c),
                button("D").entity_ref(d),
            ));
            with_refs(view, [a, b, c, d])
        })
        .unwrap();
    f.layout();

    // A -> explicit right D (overrides geometric B).
    f.cx.focus_node(f.doc, a.stable_id()).unwrap();
    assert!(f.key("ArrowRight").handled);
    assert_eq!(f.focused(), Some(d.stable_id()));

    // B -> C -> D when C is disabled, hidden, or inert.
    f.cx.update_component(c, |btn, _| btn.disabled = true)
        .unwrap();
    f.cx.focus_node(f.doc, b.stable_id()).unwrap();
    assert!(f.key("ArrowRight").handled);
    assert_eq!(f.focused(), Some(d.stable_id()));

    // Re-enable C, hide via visibility: hidden.
    f.cx.update_component(c, |btn, _| btn.disabled = false)
        .unwrap();
    let mut style = f.cx.world().node_style(c.stable_id()).unwrap().clone();
    Arc::make_mut(&mut style.layout).paint.visibility = Some(VisibilitySpec::Hidden);
    let mut q = MutationQueue::new();
    q.set_style(c.stable_id(), style);
    f.cx.commit_mutations(q).unwrap();
    f.layout();
    f.cx.focus_node(f.doc, b.stable_id()).unwrap();
    assert!(f.key("ArrowRight").handled);
    assert_eq!(f.focused(), Some(d.stable_id()));

    // Make C inert (#180) and D display: none; cycle B -> C -> B must terminate
    // safely without hanging.
    let mut style_c = f.cx.world().node_style(c.stable_id()).unwrap().clone();
    let c_layout = Arc::make_mut(&mut style_c.layout);
    c_layout.paint.visibility = Some(VisibilitySpec::Visible);
    c_layout.inert = true;
    let mut style_d = f.cx.world().node_style(d.stable_id()).unwrap().clone();
    Arc::make_mut(&mut style_d.layout).display = Some(DisplaySpec::None);
    let mut q = MutationQueue::new();
    q.set_style(c.stable_id(), style_c);
    q.set_style(d.stable_id(), style_d);
    q.set_focus_neighbors(c.stable_id(), FocusNeighbors::new().right(b.stable_id()));
    f.cx.commit_mutations(q).unwrap();
    f.layout();

    f.cx.focus_node(f.doc, b.stable_id()).unwrap();
    assert!(!f.key("ArrowRight").handled);
    assert_eq!(f.focused(), Some(b.stable_id()));
}

#[test]
fn directional_nav_filters_clipped_and_reveals_scroll_targets() {
    let mut f = DirFixture::new();
    let (_, (scroll, top_btn, bottom_btn, _hidden_clip_btn)) =
        f.cx.mount_view(f.root, || {
            let scroll = entity_ref::<ScrollView>();
            let top_btn = entity_ref::<Button>();
            let bottom_btn = entity_ref::<Button>();
            let hidden_clip_btn = entity_ref::<Button>();
            let scroll_widget = ScrollView::new(ScrollAxes::Vertical).with_layout(|layout| {
                layout.width = Some(LengthSpec::Px(160.0));
                layout.height = Some(LengthSpec::Px(100.0));
            });
            let clipped = Stack::column(0.0).with_layout(|layout| {
                layout.width = Some(LengthSpec::Px(120.0));
                layout.height = Some(LengthSpec::Px(40.0));
                layout.overflow_x = OverflowSpec::Hidden;
                layout.overflow_y = OverflowSpec::Hidden;
            });
            let spacer = Stack::column(0.0).with_layout(|layout| {
                layout.height = Some(LengthSpec::Px(80.0));
            });
            let view = row()
                .gap(16.0)
                .directional_focus(DirectionalFocusScope::new())
                .children((
                    widget(scroll_widget).entity_ref(scroll).children(
                        column().gap(120.0).children((
                            sized_button("top", 120.0, 32.0).entity_ref(top_btn),
                            sized_button("bottom", 120.0, 32.0).entity_ref(bottom_btn),
                        )),
                    ),
                    widget(clipped).children(column().gap(20.0).children((
                        widget(spacer),
                        sized_button("clipped", 100.0, 32.0).entity_ref(hidden_clip_btn),
                    ))),
                ));
            with_refs(view, (scroll, top_btn, bottom_btn, hidden_clip_btn))
        })
        .unwrap();
    f.layout();

    // `bottom_btn` starts below the ScrollView's 100px viewport.
    let initial_scroll =
        f.cx.world()
            .scroll_offset(scroll.stable_id())
            .unwrap_or_default();
    assert_eq!(initial_scroll.y, 0.0);

    // Moving right from `top_btn` must NOT jump to `hidden_clip_btn`, which is
    // clipped outside the non-scrollable overflow: hidden container.
    f.cx.focus_node(f.doc, top_btn.stable_id()).unwrap();
    assert!(!f.key("ArrowRight").handled);
    assert_eq!(f.focused(), Some(top_btn.stable_id()));

    // Moving down inside ScrollView finds `bottom_btn` and reveals it by scrolling.
    assert!(f.key("ArrowDown").handled);
    assert_eq!(f.focused(), Some(bottom_btn.stable_id()));
    let after_scroll =
        f.cx.world()
            .scroll_offset(scroll.stable_id())
            .unwrap_or_default();
    assert!(
        after_scroll.y > 0.0,
        "ScrollView must scroll down to reveal bottom_btn, got {after_scroll:?}"
    );
}

#[test]
fn rtl_layout_navigates_visually_left_and_right() {
    let mut f = DirFixture::new();
    let (_, [first, second]) =
        f.cx.mount_view(f.root, || {
            let first = entity_ref::<Button>();
            let second = entity_ref::<Button>();
            let rtl_row = Stack::row(8.0).with_layout(|layout| {
                layout.dir = Some(DirSpec::Rtl);
                layout.width = Some(LengthSpec::Px(300.0));
            });
            let view = widget(rtl_row)
                .directional_focus(DirectionalFocusScope::new())
                .children((
                    sized_button("first", 80.0, 32.0).entity_ref(first),
                    sized_button("second", 80.0, 32.0).entity_ref(second),
                ));
            with_refs(view, [first, second])
        })
        .unwrap();
    f.layout();

    let b_first = f.cx.world().layout_box(first.stable_id()).unwrap();
    let b_second = f.cx.world().layout_box(second.stable_id()).unwrap();
    assert!(
        b_first.x > b_second.x,
        "in RTL, first ({b_first:?}) sits to the right of second ({b_second:?})"
    );

    f.cx.focus_node(f.doc, first.stable_id()).unwrap();
    assert!(f.key("ArrowLeft").handled);
    assert_eq!(f.focused(), Some(second.stable_id()));
    assert!(f.key("ArrowRight").handled);
    assert_eq!(f.focused(), Some(first.stable_id()));
}

#[test]
fn gamepad_navigation_intent_shares_directional_focus_and_roving_respect() {
    let mut f = DirFixture::new();
    let activated = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let log_a = Arc::clone(&activated);
    let log_b = Arc::clone(&activated);
    let (_, [a, b]) =
        f.cx.mount_view(f.root, || {
            let a = entity_ref::<Button>();
            let b = entity_ref::<Button>();
            let view = column().gap(8.0).children((
                button("A")
                    .on(move |_: &Activate| log_a.lock().unwrap().push("A"))
                    .entity_ref(a),
                button("B")
                    .on(move |_: &Activate| log_b.lock().unwrap().push("B"))
                    .entity_ref(b),
            ));
            with_refs(view, [a, b])
        })
        .unwrap();
    f.layout();

    // Gamepad D-pad down moves from nothing -> A, then A -> B, keeping focus_visible on.
    assert!(
        f.nav(gamepad_dpad_navigation(FocusDirection::Down, false))
            .handled
    );
    assert_eq!(f.focused(), Some(a.stable_id()));
    assert_eq!(f.cx.world().input_modality(f.doc), InputModality::Gamepad);
    assert_eq!(f.cx.world().focus_visible(f.doc), Some(a.stable_id()));

    assert!(
        f.nav(gamepad_dpad_navigation(FocusDirection::Down, false))
            .handled
    );
    assert_eq!(f.focused(), Some(b.stable_id()));

    // Confirm activates B; repeat confirm is ignored.
    assert!(
        !f.nav(NavigationInput::gamepad(NavigationIntent::Confirm, true))
            .handled
    );
    assert!(
        f.nav(NavigationInput::gamepad(NavigationIntent::Confirm, false))
            .handled
    );
    assert_eq!(*activated.lock().unwrap(), ["B"]);

    // Previous moves back via unified sequential order.
    assert!(
        f.nav(NavigationInput::gamepad(NavigationIntent::Previous, false))
            .handled
    );
    assert_eq!(f.focused(), Some(a.stable_id()));
}

#[test]
fn modal_trap_contains_directional_navigation_and_updates_diagnostics_counters() {
    let _guard = nana_diagnostics::install(
        nana_diagnostics::DiagnosticsConfig {
            persist: nana_diagnostics::PersistMode::Off,
            ..Default::default()
        },
        nana_diagnostics::SessionMetadata::new("focus-diag-test", "focus-diag-test", "0.0.0"),
        nana_diagnostics::DiagnosticsPaths::in_memory(),
    );

    let enters_before = nana_diagnostics::framework::runtime::FOCUS_TRAP_ENTERS.value();
    let exits_before = nana_diagnostics::framework::runtime::FOCUS_TRAP_EXITS.value();
    let queries_before = nana_diagnostics::framework::runtime::FOCUS_NAVIGATION_QUERIES.value();
    let candidates_before =
        nana_diagnostics::framework::runtime::FOCUS_NAVIGATION_CANDIDATES.value();

    let mut cx = AppContext::new();
    let doc = DocumentId::new(1).unwrap();
    let trigger = cx.create_component(doc, Button::new("open")).unwrap();
    let outside = cx.create_component(doc, Button::new("outside")).unwrap();
    let host1 = cx.create_component(doc, OverlayHost::new()).unwrap();
    let dialog1 = cx
        .create_detached_component(doc, Dialog::new("D1"))
        .unwrap();
    let ok1 = cx
        .create_detached_component(doc, Button::new("ok1"))
        .unwrap();
    let cancel1 = cx
        .create_detached_component(doc, Button::new("cancel1"))
        .unwrap();
    cx.set_modal_slots(
        dialog1,
        ModalSlots {
            body: Some(ok1.stable_id()),
            footer: Some(cancel1.stable_id()),
            ..Default::default()
        },
    )
    .unwrap();
    cx.append_child(host1, dialog1).unwrap();

    // Even if `ok1` points its explicit neighbor outside to `outside`, modal trap keeps focus inside.
    cx.set_focus_neighbors(
        ok1.stable_id(),
        FocusNeighbors::new().right(outside.stable_id()),
    )
    .unwrap();
    layout_doc(&mut cx, doc);

    cx.focus_node(doc, trigger.stable_id()).unwrap();
    assert_eq!(cx.focus_counters().traps_active, 0);

    // Open modal 1.
    assert!(cx.activate_overlay(host1, dialog1).unwrap());
    layout_doc(&mut cx, doc);
    assert_eq!(cx.focus_counters().trap_enters, 1);
    assert_eq!(cx.focus_counters().traps_active, 1);

    // Directional navigation from ok1 cannot escape to `outside`, falls back inside modal.
    cx.focus_node(doc, ok1.stable_id()).unwrap();
    let _ = cx
        .navigate_directional_focus(doc, FocusDirection::Right)
        .unwrap();
    assert_ne!(cx.world().focused(doc), Some(outside.stable_id()));

    // Explicit neighbor inside modal records 1 query and 1 candidate.
    cx.set_focus_neighbors(
        ok1.stable_id(),
        FocusNeighbors::new().down(cancel1.stable_id()),
    )
    .unwrap();
    let q_before = cx.focus_counters().navigation_queries;
    let c_before = cx.focus_counters().navigation_candidates_considered;
    assert!(
        cx.navigate_directional_focus(doc, FocusDirection::Down)
            .unwrap()
    );
    assert_eq!(cx.world().focused(doc), Some(cancel1.stable_id()));
    assert_eq!(cx.focus_counters().navigation_queries - q_before, 1);
    assert_eq!(
        cx.focus_counters().navigation_candidates_considered - c_before,
        1
    );

    // Dismiss modal 1 (let exit finish cleanly).
    assert!(cx.dismiss_overlay(host1).unwrap());
    let mut t = Duration::ZERO;
    for _ in 0..30 {
        t += Duration::from_millis(20);
        cx.advance_animations(t);
    }
    assert_eq!(cx.focus_counters().trap_exits, 1);
    assert_eq!(cx.focus_counters().traps_active, 0);

    // Verify nana-diagnostics metrics incremented (when a global diagnostics
    // session was installed in this test process).
    if nana_diagnostics::metrics_enabled() {
        assert!(nana_diagnostics::framework::runtime::FOCUS_TRAP_ENTERS.value() > enters_before);
        assert!(nana_diagnostics::framework::runtime::FOCUS_TRAP_EXITS.value() > exits_before);
        assert!(
            nana_diagnostics::framework::runtime::FOCUS_NAVIGATION_QUERIES.value() > queries_before
        );
        assert!(
            nana_diagnostics::framework::runtime::FOCUS_NAVIGATION_CANDIDATES.value()
                > candidates_before
        );
    }
}

#[test]
fn roving_focus_group_still_wins_over_directional_scope() {
    let mut f = DirFixture::new();
    let (_, [a, b]) =
        f.cx.mount_view(f.root, || {
            let a = entity_ref::<Button>();
            let b = entity_ref::<Button>();
            let view = column()
                .directional_focus(DirectionalFocusScope::new())
                .children(
                    row()
                        .roving_focus(RovingFocusGroup::horizontal())
                        .children((button("A").entity_ref(a), button("B").entity_ref(b))),
                );
            with_refs(view, [a, b])
        })
        .unwrap();
    f.layout();
    f.cx.focus_node(f.doc, a.stable_id()).unwrap();
    assert!(f.key("ArrowRight").handled);
    assert_eq!(f.focused(), Some(b.stable_id()));
    // At end of non-wrapping roving group, roving group consumes the key and stays on B.
    assert!(f.key("ArrowRight").handled);
    assert_eq!(f.focused(), Some(b.stable_id()));
}
