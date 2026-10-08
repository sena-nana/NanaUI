//! Source binding, validation, pointer identity, lifecycle and the host
//! slots: what routing does around the dispatch itself.

use std::sync::{Arc, Mutex};

use nana_ui_core::CursorSpec;
use nana_ui_input::{
    CanonicalInputEvent, CursorIcon, DeviceId, EndpointGeneration, HostServiceError, HostServices,
    InputEndpoint, InputMetadata, InputSequence, InputSourceId, InputTimestamp, SurroundingText,
    TextInputContext, TextInputPurpose, UnsupportedHostServices,
};

use super::*;
use crate::{
    InteractionState, LayoutBox, LayoutStyle, MutationQueue, NodeKind, NodeStyle, StableNodeId,
    TextInput,
};

fn document(id: u64) -> DocumentId {
    DocumentId::new(id).unwrap()
}

fn event(
    source: InputSourceId,
    generation: EndpointGeneration,
    sequence: u64,
    payload: InputPayload,
) -> CanonicalInputEvent {
    CanonicalInputEvent {
        metadata: InputMetadata {
            source,
            device: DeviceId(1),
            generation,
            sequence: InputSequence(sequence),
            timestamp: InputTimestamp(sequence),
        },
        payload,
    }
}

fn pointer(phase: PointerPhase, x: f32, y: f32) -> InputPayload {
    InputPayload::Pointer(PointerInput {
        pointer_id: PointerId(42),
        ..PointerInput::mouse(phase, x, y)
    })
}

fn key(logical: &'static str, modifiers: InputModifiers) -> KeyInput {
    KeyInput::named(logical, logical, KeyState::Pressed, modifiers)
}

fn control() -> InputModifiers {
    InputModifiers {
        control: true,
        ..InputModifiers::default()
    }
}

/// A hittable box at the origin, laid out and indexed.
fn hittable_node(context: &mut AppContext, document: DocumentId, id: u64) -> StableNodeId {
    let node = StableNodeId::new(id).unwrap();
    let mut create = MutationQueue::new();
    create.create(node, document, NodeKind::Element { tag: "div".into() });
    create.set_interaction(
        node,
        InteractionState {
            pointer_events: true,
            ..InteractionState::default()
        },
    );
    create.write_layout(
        node,
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 40.0,
        },
    );
    context.commit_mutations(create).unwrap();
    context.rebuild_hit_test(document);
    node
}

fn focused_editor(
    context: &mut AppContext,
    document: DocumentId,
    value: &str,
) -> crate::Entity<TextInput> {
    let input = context
        .create_component(document, TextInput::new(value))
        .unwrap();
    let mut layout = MutationQueue::new();
    layout.write_layout(
        input.stable_id(),
        LayoutBox {
            x: 12.0,
            y: 24.0,
            width: 180.0,
            height: 28.0,
        },
    );
    context.commit_mutations(layout).unwrap();
    assert!(context.focus_node(document, input.stable_id()).unwrap());
    input
}

#[test]
fn binding_rejects_a_stale_generation_and_a_same_generation_rebind() {
    let mut context = AppContext::new();
    let source = InputSourceId(19);
    let (first, second) = (document(12), document(13));
    context
        .bind_input_source(source, EndpointGeneration(5), first)
        .unwrap();
    assert_eq!(
        context.bind_input_source(source, EndpointGeneration(4), first),
        Err(InputBindError::StaleGeneration {
            bound: EndpointGeneration(5)
        })
    );
    assert_eq!(
        context.bind_input_source(source, EndpointGeneration(5), second),
        Err(InputBindError::DocumentRebind { bound: first })
    );
    // The same binding again keeps what the source knows.
    context
        .route_input(
            &event(
                source,
                EndpointGeneration(5),
                1,
                pointer(PointerPhase::Move, 5.0, 5.0),
            ),
            &mut UnsupportedHostServices,
            None,
        )
        .unwrap();
    context
        .bind_input_source(source, EndpointGeneration(5), first)
        .unwrap();
    assert_eq!(context.input.sources[&source].pointers.len(), 1);
    context
        .bind_input_source(source, EndpointGeneration(6), second)
        .unwrap();
    assert_eq!(
        context.input_binding(source),
        Some((EndpointGeneration(6), second))
    );
    assert!(context.input.sources[&source].pointers.is_empty());
}

#[test]
#[should_panic(expected = "input endpoint generation exhausted")]
fn headless_rebind_rejects_generation_exhaustion() {
    let mut context = AppContext::new();
    let document = document(14);
    context
        .bind_input_source(
            HeadlessInput::SOURCE,
            EndpointGeneration(u64::MAX),
            document,
        )
        .unwrap();

    // `HeadlessInput::bind` normally picks the next generation for the source;
    // once the non-repeating identity space is exhausted it must fail instead
    // of wrapping and reviving stale events.
    let _ = HeadlessInput::bind(&mut context, document);
}

#[test]
fn events_are_checked_against_the_binding_before_they_dispatch() {
    let mut context = AppContext::new();
    let source = InputSourceId(9);
    let generation = EndpointGeneration(3);
    context
        .bind_input_source(source, generation, document(1))
        .unwrap();
    let focus = InputPayload::Focus { focused: true };
    assert!(matches!(
        context.route_input(
            &event(source, EndpointGeneration(2), 1, focus.clone()),
            &mut UnsupportedHostServices,
            None
        ),
        Err(InputRouteError::StaleGeneration)
    ));
    context
        .route_input(
            &event(source, generation, 5, focus.clone()),
            &mut UnsupportedHostServices,
            None,
        )
        .unwrap();
    assert!(matches!(
        context.route_input(
            &event(source, generation, 5, focus.clone()),
            &mut UnsupportedHostServices,
            None
        ),
        Err(InputRouteError::OutOfOrder)
    ));
    let mut earlier = event(source, generation, 6, focus.clone());
    earlier.metadata.timestamp = InputTimestamp(1);
    assert!(matches!(
        context.route_input(&earlier, &mut UnsupportedHostServices, None),
        Err(InputRouteError::TimestampRegression)
    ));
    assert_eq!(
        context.unbind_input_source(source, Duration::ZERO).unwrap(),
        Some(document(1))
    );
    assert!(matches!(
        context.route_input(
            &event(source, generation, 7, focus),
            &mut UnsupportedHostServices,
            None
        ),
        Err(InputRouteError::UnknownSource)
    ));
    let counters = context.input_counters();
    assert_eq!(counters.events_routed, 1);
    assert_eq!(counters.events_rejected, 4);
}

#[test]
fn a_device_stays_disconnected_until_it_reconnects() {
    let mut context = AppContext::new();
    let source = InputSourceId(10);
    let generation = EndpointGeneration(1);
    context
        .bind_input_source(source, generation, document(2))
        .unwrap();
    let mut services = UnsupportedHostServices;
    let route = |context: &mut AppContext, sequence, payload, services: &mut dyn HostServices| {
        let mut sample = event(source, generation, sequence, payload);
        sample.metadata.device = DeviceId(7);
        context.route_input(&sample, services, None)
    };
    route(
        &mut context,
        1,
        InputPayload::DeviceDisconnected,
        &mut services,
    )
    .unwrap();
    assert!(matches!(
        route(
            &mut context,
            2,
            InputPayload::Focus { focused: true },
            &mut services
        ),
        Err(InputRouteError::Disconnected)
    ));
    route(
        &mut context,
        3,
        InputPayload::DeviceConnected,
        &mut services,
    )
    .unwrap();
    route(
        &mut context,
        4,
        InputPayload::Focus { focused: true },
        &mut services,
    )
    .unwrap();
}

#[test]
fn source_lifecycle_passes_a_device_tombstone_without_reviving_the_device() {
    let mut context = AppContext::new();
    let source = InputSourceId(32);
    let generation = EndpointGeneration(1);
    context
        .bind_input_source(source, generation, document(1))
        .unwrap();
    for (sequence, payload) in [
        (1, InputPayload::DeviceDisconnected),
        (2, InputPayload::SourceDisconnected),
        (3, InputPayload::SourceConnected),
    ] {
        context
            .route_input(
                &event(source, generation, sequence, payload),
                &mut UnsupportedHostServices,
                None,
            )
            .unwrap();
    }
    assert!(matches!(
        context.route_input(
            &event(source, generation, 4, InputPayload::Focus { focused: true }),
            &mut UnsupportedHostServices,
            None
        ),
        Err(InputRouteError::Disconnected)
    ));
    context
        .route_input(
            &event(source, generation, 5, InputPayload::DeviceConnected),
            &mut UnsupportedHostServices,
            None,
        )
        .unwrap();
}

#[test]
fn pointer_metadata_survives_events_that_carry_none() {
    let mut context = AppContext::new();
    let source = InputSourceId(31);
    let generation = EndpointGeneration(1);
    context
        .bind_input_source(source, generation, document(1))
        .unwrap();
    let mut services = UnsupportedHostServices;
    let enter = InputPayload::PointerEnter {
        pointer_id: PointerId(42),
        x: 10.0,
        y: 12.0,
    };
    context
        .route_input(
            &event(source, generation, 1, enter.clone()),
            &mut services,
            None,
        )
        .unwrap();
    let pen = InputPayload::Pointer(PointerInput {
        pointer_id: PointerId(42),
        pointer_type: PointerType::Pen,
        is_primary: false,
        ..PointerInput::mouse(PointerPhase::Move, 10.0, 12.0)
    });
    context
        .route_input(&event(source, generation, 2, pen), &mut services, None)
        .unwrap();
    context
        .route_input(&event(source, generation, 3, enter), &mut services, None)
        .unwrap();
    let identity = context.input.sources[&source].pointers[&(DeviceId(1), PointerId(42))];
    assert_eq!(identity.pointer_type, PointerType::Pen);
    assert!(!identity.is_primary);
}

#[test]
fn an_idle_drain_does_no_work() {
    let mut context = AppContext::new();
    let source = InputSourceId(22);
    let generation = EndpointGeneration(1);
    context
        .bind_input_source(source, generation, document(15))
        .unwrap();
    let mut endpoint = InputEndpoint::new(8, 1024);
    let mut routed = Vec::new();
    let queries = context.world().hit_test_queries();
    assert_eq!(
        context.drain_input(
            &mut endpoint,
            &mut UnsupportedHostServices,
            None,
            &mut routed
        ),
        0
    );
    assert!(routed.is_empty());
    assert_eq!(context.input_counters(), InputCounters::default());
    assert_eq!(context.world().hit_test_queries(), queries);
    assert_eq!(context.world().pending_work_revision(), 0);
}

/// Each event leaves the endpoint before it routes, so an event the world
/// rejects is reported and the ones behind it still route.
#[test]
fn a_failing_event_never_holds_the_endpoint() {
    let mut context = AppContext::new();
    let source = InputSourceId(23);
    let generation = EndpointGeneration(1);
    context
        .bind_input_source(source, generation, document(16))
        .unwrap();
    let mut endpoint = InputEndpoint::new(8, 1024);
    endpoint
        .push(event(
            source,
            generation,
            1,
            pointer(PointerPhase::Move, f32::NAN, 0.0),
        ))
        .unwrap();
    endpoint
        .push(event(
            source,
            generation,
            2,
            InputPayload::Focus { focused: true },
        ))
        .unwrap();
    let mut routed = Vec::new();
    assert_eq!(
        context.drain_input(
            &mut endpoint,
            &mut UnsupportedHostServices,
            None,
            &mut routed
        ),
        2
    );
    assert!(endpoint.is_empty());
    assert!(matches!(
        routed[0].result,
        Err(InputRouteError::Dispatch(_))
    ));
    assert!(routed[1].result.is_ok());
}

/// An uncaptured pointer event costs one hit query, shared by routing, the
/// cursor and the reported hit; a captured one costs none and schedules no
/// work; cancelling the device revokes the capture.
#[test]
fn hit_queries_are_budgeted_and_a_disconnect_revokes_capture() {
    let mut context = AppContext::new();
    let doc = document(7);
    let source = InputSourceId(15);
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    let node = hittable_node(&mut context, doc, 1);
    let mut services = HeadlessHostServices::new();

    let queries = context.world().hit_test_queries();
    let moved = context
        .route_input(
            &event(
                source,
                generation,
                1,
                pointer(PointerPhase::Move, 10.0, 12.0),
            ),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(moved.pointer_hit, Some(node));
    assert_eq!(context.world().hit_test_queries() - queries, 1);

    let mut capture = MutationQueue::new();
    capture.capture_pointer(1, node);
    context.commit_mutations(capture).unwrap();
    let captured_work = context.world().pending_work_revision();
    let queries = context.world().hit_test_queries();
    for sequence in 2..=1001 {
        let outcome = context
            .route_input(
                &event(
                    source,
                    generation,
                    sequence,
                    pointer(PointerPhase::Move, 500.0, 500.0),
                ),
                &mut services,
                None,
            )
            .unwrap();
        assert_eq!(outcome.pointer_hit, Some(node));
    }
    assert_eq!(context.world().hit_test_queries(), queries);
    assert_eq!(context.world().pending_work_revision(), captured_work);

    context
        .route_input(
            &event(source, generation, 1002, InputPayload::DeviceDisconnected),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(context.world().pointer_capture(doc, 1), None);
    assert!(context.input.sources[&source].pointers.is_empty());
}

#[test]
fn a_pointer_leave_clears_hover_and_keeps_capture() {
    let mut context = AppContext::new();
    let doc = document(17);
    let source = InputSourceId(17);
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    let node = hittable_node(&mut context, doc, 4);
    let mut services = UnsupportedHostServices;
    context
        .route_input(
            &event(
                source,
                generation,
                1,
                pointer(PointerPhase::Down, 10.0, 12.0),
            ),
            &mut services,
            None,
        )
        .unwrap();
    let mut capture = MutationQueue::new();
    capture.capture_pointer(1, node);
    context.commit_mutations(capture).unwrap();
    context
        .route_input(
            &event(
                source,
                generation,
                2,
                InputPayload::PointerLeave {
                    pointer_id: PointerId(42),
                },
            ),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(context.world().pointer_hover(doc, 1), None);
    assert_eq!(context.world().pointer_capture(doc, 1), Some(node));
}

/// Switching windows loses the pointer, not the caret: blur cancels what the
/// source held and leaves document focus alone, and focus coming back hands
/// the host the text-input state again.
#[test]
fn window_blur_keeps_document_focus_and_refocus_rearms_text_input() {
    let mut context = AppContext::new();
    let doc = document(8);
    let source = InputSourceId(16);
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    let node = hittable_node(&mut context, doc, 2);
    let editor = focused_editor(&mut context, doc, "text");
    let mut services = HeadlessHostServices::new();
    context
        .route_input(
            &event(source, generation, 1, InputPayload::Focus { focused: true }),
            &mut services,
            None,
        )
        .unwrap();
    context
        .route_input(
            &event(
                source,
                generation,
                2,
                pointer(PointerPhase::Move, 10.0, 12.0),
            ),
            &mut services,
            None,
        )
        .unwrap();
    context.press_pointer(doc, 1, node).unwrap();
    let mut capture = MutationQueue::new();
    capture.capture_pointer(1, node);
    context.commit_mutations(capture).unwrap();
    context.focus_node(doc, editor.stable_id()).unwrap();
    let armed = context.input_counters().text_input_updates;

    context
        .route_input(
            &event(
                source,
                generation,
                3,
                InputPayload::Focus { focused: false },
            ),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(context.world().pointer_capture(doc, 1), None);
    assert_eq!(context.world().pointer_press(doc, 1), None);
    assert_eq!(context.world().focused(doc), Some(editor.stable_id()));

    context
        .route_input(
            &event(source, generation, 4, InputPayload::Focus { focused: true }),
            &mut services,
            None,
        )
        .unwrap();
    assert!(context.input_counters().text_input_updates > armed);
    assert_eq!(
        services.text_input().map(|state| state.purpose),
        Some(TextInputPurpose::Normal)
    );
}

#[test]
fn a_blur_leaves_other_sources_pointers_alone() {
    let mut context = AppContext::new();
    let doc = document(9);
    let (source, other) = (InputSourceId(20), InputSourceId(21));
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    context.bind_input_source(other, generation, doc).unwrap();
    let node = hittable_node(&mut context, doc, 3);
    let mut services = UnsupportedHostServices;
    for owner in [source, other] {
        context
            .route_input(
                &event(
                    owner,
                    generation,
                    1,
                    pointer(PointerPhase::Move, 10.0, 12.0),
                ),
                &mut services,
                None,
            )
            .unwrap();
    }
    let other_pointer = context.input.sources[&other].pointers[&(DeviceId(1), PointerId(42))].local;
    context.press_pointer(doc, other_pointer, node).unwrap();
    let mut capture = MutationQueue::new();
    capture.capture_pointer(other_pointer, node);
    context.commit_mutations(capture).unwrap();
    context
        .route_input(
            &event(
                source,
                generation,
                2,
                InputPayload::Focus { focused: false },
            ),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(
        context.world().pointer_press(doc, other_pointer),
        Some(node)
    );
    assert_eq!(
        context.world().pointer_capture(doc, other_pointer),
        Some(node)
    );
    assert_eq!(
        context.world().pointer_hover(doc, other_pointer),
        Some(node)
    );
    assert!(context.input.sources[&source].pointers.is_empty());
}

/// The first event of a pointer never reads another pointer's state: its
/// local id is allocated before capture or hover is looked up.
#[test]
fn a_new_pointer_does_not_read_another_sources_capture() {
    let mut context = AppContext::new();
    let doc = document(10);
    let (first, second) = (InputSourceId(1), InputSourceId(2));
    let generation = EndpointGeneration(1);
    context.bind_input_source(first, generation, doc).unwrap();
    context.bind_input_source(second, generation, doc).unwrap();
    let node = hittable_node(&mut context, doc, 5);
    let mut services = UnsupportedHostServices;
    context
        .route_input(
            &event(
                first,
                generation,
                1,
                pointer(PointerPhase::Move, 10.0, 12.0),
            ),
            &mut services,
            None,
        )
        .unwrap();
    let mut capture = MutationQueue::new();
    capture.capture_pointer(1, node);
    context.commit_mutations(capture).unwrap();
    // Same platform pointer id, other source: a different pointer.
    let outcome = context
        .route_input(
            &event(
                second,
                generation,
                1,
                pointer(PointerPhase::Move, 500.0, 500.0),
            ),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(outcome.pointer_hit, None);
    assert_eq!(context.world().pointer_capture(doc, 2), None);
    assert_eq!(context.world().pointer_capture(doc, 1), Some(node));
}

/// A mouse keeps its identity across a release, for hover continuity. A
/// finger that lifted hovers nothing and its next touch is a new pointer, so
/// taps leave no hover or position behind.
#[test]
fn a_mouse_keeps_its_identity_and_a_lifted_touch_leaves_nothing() {
    let mut context = AppContext::new();
    let doc = document(3);
    let source = InputSourceId(11);
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    let node = hittable_node(&mut context, doc, 6);
    let mut services = UnsupportedHostServices;
    context
        .route_input(
            &event(
                source,
                generation,
                1,
                pointer(PointerPhase::Down, 10.0, 12.0),
            ),
            &mut services,
            None,
        )
        .unwrap();
    context
        .route_input(
            &event(source, generation, 2, pointer(PointerPhase::Up, 10.0, 12.0)),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(context.input.sources[&source].pointers.len(), 1);
    assert_eq!(context.world().pointer_hover(doc, 1), Some(node));

    for (sequence, phase) in [(3, PointerPhase::Down), (4, PointerPhase::Up)] {
        let touch = InputPayload::Pointer(PointerInput {
            pointer_id: PointerId(43),
            pointer_type: PointerType::Touch,
            ..PointerInput::mouse(phase, 10.0, 12.0)
        });
        context
            .route_input(
                &event(source, generation, sequence, touch),
                &mut services,
                None,
            )
            .unwrap();
    }
    assert_eq!(context.input.sources[&source].pointers.len(), 1);
    assert_eq!(context.world().pointer_hover(doc, 2), None);
    assert_eq!(context.pointer_position(doc, 2), None);
}

#[test]
fn a_disconnect_clears_focus_only_when_no_focused_source_remains() {
    let mut context = AppContext::new();
    let doc = document(4);
    let (source, other) = (InputSourceId(12), InputSourceId(13));
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    context.bind_input_source(other, generation, doc).unwrap();
    let editor = focused_editor(&mut context, doc, "");
    let mut services = UnsupportedHostServices;
    for owner in [source, other] {
        context
            .route_input(
                &event(owner, generation, 1, InputPayload::Focus { focused: true }),
                &mut services,
                None,
            )
            .unwrap();
    }
    context
        .route_input(
            &event(source, generation, 2, InputPayload::SourceDisconnected),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(context.world().focused(doc), Some(editor.stable_id()));
    context
        .route_input(
            &event(other, generation, 2, InputPayload::SourceDisconnected),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(context.world().focused(doc), None);
}

#[test]
fn disconnect_after_blur_clears_the_focus_retained_for_reentry() {
    let mut context = AppContext::new();
    let doc = document(43);
    let source = InputSourceId(43);
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    let editor = focused_editor(&mut context, doc, "draft");
    let mut services = UnsupportedHostServices;

    context
        .route_input(
            &event(source, generation, 1, InputPayload::Focus { focused: true }),
            &mut services,
            None,
        )
        .unwrap();
    context
        .route_input(
            &event(
                source,
                generation,
                2,
                InputPayload::Focus { focused: false },
            ),
            &mut services,
            None,
        )
        .unwrap();
    // Blur deliberately retained the document focus for a possible re-entry.
    assert_eq!(context.world().focused(doc), Some(editor.stable_id()));

    context
        .route_input(
            &event(source, generation, 3, InputPayload::SourceDisconnected),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(context.world().focused(doc), None);
}

#[test]
fn window_blur_cancels_an_active_ime_preedit() {
    let mut context = AppContext::new();
    let doc = document(44);
    let source = InputSourceId(44);
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    let editor = focused_editor(&mut context, doc, "draft");
    context
        .set_ime_preedit(doc, "中".into(), Some((0, "中".len())))
        .unwrap();
    assert!(context.world().ime(editor.stable_id()).is_some());
    let mut services = UnsupportedHostServices;

    context
        .route_input(
            &event(
                source,
                generation,
                1,
                InputPayload::Focus { focused: false },
            ),
            &mut services,
            None,
        )
        .unwrap();
    assert!(context.world().ime(editor.stable_id()).is_none());
    assert_eq!(context.world().text(editor.stable_id()), Some("draft"));
}

#[test]
fn a_blur_from_one_source_does_not_cancel_another_focused_sources_ime() {
    let mut context = AppContext::new();
    let doc = document(45);
    let (source, other) = (InputSourceId(45), InputSourceId(46));
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    context.bind_input_source(other, generation, doc).unwrap();
    let editor = focused_editor(&mut context, doc, "draft");
    let mut services = UnsupportedHostServices;

    for owner in [source, other] {
        context
            .route_input(
                &event(owner, generation, 1, InputPayload::Focus { focused: true }),
                &mut services,
                None,
            )
            .unwrap();
    }
    context
        .set_ime_preedit(doc, "中".into(), Some((0, "中".len())))
        .unwrap();
    context
        .route_input(
            &event(
                source,
                generation,
                2,
                InputPayload::Focus { focused: false },
            ),
            &mut services,
            None,
        )
        .unwrap();

    assert!(context.world().ime(editor.stable_id()).is_some());
}

#[test]
fn source_disconnect_cancels_ime_before_clearing_focus() {
    let mut context = AppContext::new();
    let doc = document(47);
    let source = InputSourceId(47);
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    let editor = focused_editor(&mut context, doc, "draft");
    context
        .set_ime_preedit(doc, "中".into(), Some((0, "中".len())))
        .unwrap();
    let mut services = UnsupportedHostServices;

    context
        .route_input(
            &event(source, generation, 1, InputPayload::Focus { focused: true }),
            &mut services,
            None,
        )
        .unwrap();
    context
        .route_input(
            &event(source, generation, 2, InputPayload::SourceDisconnected),
            &mut services,
            None,
        )
        .unwrap();

    assert!(context.world().ime(editor.stable_id()).is_none());
    assert_eq!(context.world().focused(doc), None);
}

#[test]
fn unbinding_a_source_cancels_ime_but_retains_document_focus() {
    let mut context = AppContext::new();
    let doc = document(48);
    let source = InputSourceId(48);
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    let editor = focused_editor(&mut context, doc, "draft");
    let mut services = UnsupportedHostServices;
    context
        .route_input(
            &event(source, generation, 1, InputPayload::Focus { focused: true }),
            &mut services,
            None,
        )
        .unwrap();
    context
        .set_ime_preedit(doc, "中".into(), Some((0, "中".len())))
        .unwrap();

    context.unbind_input_source(source, Duration::ZERO).unwrap();

    assert!(context.world().ime(editor.stable_id()).is_none());
    assert_eq!(context.world().focused(doc), Some(editor.stable_id()));
}

#[test]
fn the_cursor_is_sent_only_when_it_changes() {
    let mut context = AppContext::new();
    let doc = document(17);
    let source = InputSourceId(21);
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    let node = hittable_node(&mut context, doc, 1);
    let mut style = MutationQueue::new();
    style.set_style(
        node,
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                cursor: Some(CursorSpec::Pointer),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        },
    );
    context.commit_mutations(style).unwrap();
    let work = context.take_system_work();
    context.resolve_styles(&work.style).unwrap();
    let mut services = HeadlessHostServices::new();
    // Empty space shows the default the host already shows.
    context
        .route_input(
            &event(
                source,
                generation,
                1,
                pointer(PointerPhase::Move, 300.0, 300.0),
            ),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(context.input_counters().cursor_updates, 0);
    for sequence in 2..=4 {
        context
            .route_input(
                &event(
                    source,
                    generation,
                    sequence * 10,
                    pointer(PointerPhase::Move, 10.0, 12.0),
                ),
                &mut services,
                None,
            )
            .unwrap();
    }
    assert_eq!(services.cursor(), CursorIcon::Pointer);
    assert_eq!(context.input_counters().cursor_updates, 1);

    // Keys, text and wheel over it leave the cursor alone.
    context
        .route_input(
            &event(
                source,
                generation,
                50,
                InputPayload::Key(key("a", InputModifiers::default())),
            ),
            &mut services,
            None,
        )
        .unwrap();
    context
        .route_input(
            &event(
                source,
                generation,
                51,
                InputPayload::Wheel(WheelInput {
                    pointer_id: PointerId(42),
                    x: 10.0,
                    y: 12.0,
                    delta_x: 0.0,
                    delta_y: 1.0,
                    unit: WheelUnit::Lines,
                    modifiers: InputModifiers::default(),
                }),
            ),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(context.input_counters().cursor_updates, 1);

    let text = context
        .create_component(doc, TextInput::new("text"))
        .unwrap()
        .stable_id();
    let mut layout = MutationQueue::new();
    layout.write_layout(
        text,
        LayoutBox {
            x: 0.0,
            y: 100.0,
            width: 100.0,
            height: 30.0,
        },
    );
    context.commit_mutations(layout).unwrap();
    let work = context.take_system_work();
    context.resolve_styles(&work.style).unwrap();
    context.rebuild_hit_test(doc);
    context
        .route_input(
            &event(
                source,
                generation,
                60,
                pointer(PointerPhase::Move, 10.0, 110.0),
            ),
            &mut services,
            None,
        )
        .unwrap();
    assert_eq!(services.cursor(), CursorIcon::Text);
}

/// The IME is told about the focused editor as state: its purpose, where
/// its caret is, and a window of text around the selection. It hears again
/// only when one of those changes.
#[test]
fn text_input_state_follows_the_focused_editor() {
    let mut context = AppContext::new();
    let doc = document(16);
    let mut input = HeadlessInput::bind(&mut context, doc);
    let editor = focused_editor(&mut context, doc, "text");
    input
        .route(&mut context, InputPayload::Focus { focused: true })
        .unwrap();
    let state = input
        .services()
        .text_input()
        .expect("editor takes text")
        .clone();
    let anchor = state.cursor_area.expect("caret anchor");
    assert_eq!(
        (anchor.x, anchor.y, anchor.width, anchor.height),
        (12.0, 24.0, 180.0, 28.0)
    );
    assert_eq!(
        state.surrounding,
        Some(SurroundingText {
            text: "text".into(),
            cursor: 4,
            anchor: 4,
        })
    );
    let sent = context.input_counters().text_input_updates;
    input
        .route(&mut context, InputPayload::Focus { focused: false })
        .unwrap();
    input
        .route(
            &mut context,
            InputPayload::Key(key("Shift", InputModifiers::default())),
        )
        .unwrap();
    assert_eq!(context.input_counters().text_input_updates, sent);

    input
        .composition(&mut context, CompositionInput::Commit("!".into()))
        .unwrap();
    assert_eq!(context.world().text(editor.stable_id()), Some("text!"));
    assert_eq!(
        input
            .services()
            .text_input()
            .and_then(|state| state.surrounding.as_ref())
            .map(|surrounding| surrounding.text.as_str()),
        Some("text!")
    );

    context.clear_focus(doc).unwrap();
    input
        .route(&mut context, InputPayload::Focus { focused: true })
        .unwrap();
    assert_eq!(input.services().text_input(), None);
}

/// A field of any size hands the IME a bounded window; composition into it
/// routes like any other, and nothing can hold input back.
#[test]
fn a_two_mebibyte_field_composes_with_a_bounded_window() {
    let mut context = AppContext::new();
    let doc = document(18);
    let mut input = HeadlessInput::bind(&mut context, doc);
    let value = "界".repeat(700_000);
    let editor = focused_editor(&mut context, doc, &value);
    input
        .route(&mut context, InputPayload::Focus { focused: true })
        .unwrap();
    for _ in 0..3 {
        input
            .composition(
                &mut context,
                CompositionInput::Update {
                    text: "zh".into(),
                    selection: Some((2, 2)),
                },
            )
            .unwrap();
    }
    input
        .composition(&mut context, CompositionInput::Commit("中".into()))
        .unwrap();
    let state = input.services().text_input().expect("editor takes text");
    let surrounding = state.surrounding.as_ref().expect("surrounding window");
    assert!(surrounding.text.len() <= SurroundingText::MAX_BYTES);
    assert!(surrounding.text.ends_with('中'));
    assert!(
        context
            .world()
            .text(editor.stable_id())
            .unwrap()
            .ends_with('中')
    );
}

#[test]
fn a_password_field_hands_the_ime_no_text() {
    let mut context = AppContext::new();
    let doc = document(19);
    let mut input = HeadlessInput::bind(&mut context, doc);
    let editor = context
        .create_component(doc, TextInput::new("secret").secure(true))
        .unwrap();
    context.focus_node(doc, editor.stable_id()).unwrap();
    input
        .route(&mut context, InputPayload::Focus { focused: true })
        .unwrap();
    let state = input.services().text_input().expect("editor takes text");
    assert_eq!(state.purpose, TextInputPurpose::Password);
    assert_eq!(state.surrounding, None);
}

/// A key a control handled types nothing: its text is dropped. A key that
/// did nothing leaves its text to type.
#[test]
fn text_from_a_handled_key_is_dropped() {
    let mut context = AppContext::new();
    let doc = document(20);
    let mut input = HeadlessInput::bind(&mut context, doc);
    let first = focused_editor(&mut context, doc, "");
    let second = context.create_component(doc, TextInput::new("")).unwrap();

    input
        .press(
            &mut context,
            key("a", InputModifiers::default()),
            Some("a"),
            None,
        )
        .unwrap();
    input
        .press(
            &mut context,
            key(" ", InputModifiers::default()),
            Some(" "),
            None,
        )
        .unwrap();
    assert_eq!(context.world().text(first.stable_id()), Some("a "));

    // Tab moves focus; the tab it types is not inserted anywhere.
    let tab = KeyInput::named("Tab", "Tab", KeyState::Pressed, InputModifiers::default());
    input.press(&mut context, tab, Some("\t"), None).unwrap();
    assert_eq!(context.world().focused(doc), Some(second.stable_id()));
    assert_eq!(context.world().text(second.stable_id()), Some(""));
    assert_eq!(context.world().text(first.stable_id()), Some("a "));
    assert_eq!(context.input_counters().text_suppressed, 1);
}

/// A program that observed a key press and stopped it (a page's
/// `keydown.preventDefault()`) drops the text the key typed.
#[test]
fn text_from_a_key_the_program_stopped_is_dropped() {
    let mut context = AppContext::new();
    let doc = document(20);
    let mut input = HeadlessInput::bind(&mut context, doc);
    let editor = focused_editor(&mut context, doc, "");

    let mut endpoint = InputEndpoint::new(8, 1024);
    let key_event = input.stamp(InputPayload::Key(key("a", InputModifiers::default())));
    let sequence = key_event.metadata.sequence;
    endpoint.push(key_event).unwrap();
    endpoint
        .push(input.stamp(InputPayload::Text(CommittedText {
            text: "a".into(),
            key: Some(sequence),
        })))
        .unwrap();

    let routed = context
        .route_next(&mut endpoint, input.services_mut(), None)
        .unwrap();
    assert!(matches!(routed.event.payload, InputPayload::Key(_)));
    context.suppress_text_for(HeadlessInput::SOURCE, sequence);
    let routed = context
        .route_next(&mut endpoint, input.services_mut(), None)
        .unwrap();
    assert!(routed.result.unwrap().prevent_default);
    assert!(
        context
            .route_next(&mut endpoint, input.services_mut(), None)
            .is_none()
    );
    assert_eq!(context.world().text(editor.stable_id()), Some(""));
    assert_eq!(context.input_counters().text_suppressed, 1);
}

#[test]
fn copy_and_cut_go_through_the_host_clipboard_and_cut_waits_for_it() {
    let mut context = AppContext::new();
    let doc = document(13);
    let mut input = HeadlessInput::bind(&mut context, doc);
    let editor = focused_editor(&mut context, doc, "copied");
    context.select_all_focused_text(doc).unwrap();
    assert!(
        input
            .press(&mut context, key("c", control()), None, None)
            .unwrap()
            .handled
    );
    assert_eq!(input.services().clipboard(), Some("copied"));
    assert_eq!(context.world().text(editor.stable_id()), Some("copied"));

    // A host that refuses the text keeps the field intact.
    let event = input.stamp(InputPayload::Key(key("x", control())));
    let refused = context
        .route_input(&event, &mut UnsupportedHostServices, None)
        .unwrap();
    assert!(!refused.handled);
    assert_eq!(context.world().text(editor.stable_id()), Some("copied"));

    input
        .press(&mut context, key("x", control()), None, None)
        .unwrap();
    assert_eq!(context.world().text(editor.stable_id()), Some(""));
    input
        .press(&mut context, key("v", control()), None, None)
        .unwrap();
    assert_eq!(context.world().text(editor.stable_id()), Some("copied"));
}

#[test]
fn document_selection_copies_without_editor_focus() {
    let mut context = AppContext::new();
    let doc = document(21);
    let mut input = HeadlessInput::bind(&mut context, doc);
    let node = context
        .create_component(doc, TextInput::new("selected"))
        .unwrap()
        .stable_id();
    context.compat_world_mut().set_document_text_selection(
        doc,
        Some(crate::DocumentTextSelection {
            node,
            start: 0,
            end: 8,
            lines: Vec::new(),
        }),
    );
    input
        .press(&mut context, key("c", control()), None, None)
        .unwrap();
    assert_eq!(input.services().clipboard(), Some("selected"));
}

/// The application's key policy sees Ctrl+C before the clipboard does: a
/// policy that takes it leaves the clipboard alone.
#[test]
fn an_application_key_policy_runs_before_the_clipboard() {
    let mut context = AppContext::new();
    let doc = document(22);
    let mut input = HeadlessInput::bind(&mut context, doc);
    let editor = focused_editor(&mut context, doc, "keep");
    context.select_all_focused_text(doc).unwrap();
    let seen = Arc::new(Mutex::new(0));
    let observed = seen.clone();
    context
        .on_key(editor, move |key| {
            if key.is_pressed() && key.logical.0 == "c" && key.modifiers.control {
                *observed.lock().unwrap() += 1;
                return true;
            }
            false
        })
        .unwrap();
    assert!(
        input
            .press(&mut context, key("c", control()), None, None)
            .unwrap()
            .handled
    );
    assert_eq!(*seen.lock().unwrap(), 1);
    assert_eq!(input.services().clipboard(), None);
}

/// A host key keeps its physical code, logical name, repeat and modifier
/// bits when the application policy reads it. The keymap matches the logical
/// name; this is the policy's copy of the same event.
#[test]
fn the_key_policy_sees_the_physical_key_and_the_same_modifiers() {
    let mut context = AppContext::new();
    let doc = document(23);
    let mut input = HeadlessInput::bind(&mut context, doc);
    let editor = focused_editor(&mut context, doc, "keep");
    let seen = Arc::new(Mutex::new(None));
    let observed = seen.clone();
    context
        .on_key(editor, move |key| {
            *observed.lock().unwrap() = Some((
                key.physical.0.to_string(),
                key.logical.0.to_string(),
                key.modifiers,
                key.repeat,
                key.state,
            ));
            true
        })
        .unwrap();
    let mut key = KeyInput::named(
        "KeyA",
        "a",
        KeyState::Pressed,
        InputModifiers {
            alt: true,
            control: true,
            meta: true,
            shift: true,
        },
    );
    key.repeat = true;
    assert!(input.press(&mut context, key, None, None).unwrap().handled);
    let (physical, logical, modifiers, repeat, state) = seen.lock().unwrap().clone().unwrap();
    assert_eq!(physical, "KeyA");
    assert_eq!(logical, "a");
    assert!(modifiers.alt && modifiers.control && modifiers.meta && modifiers.shift);
    assert!(repeat);
    assert_eq!(state, KeyState::Pressed);
}

/// Two sources driving two documents of one context keep separate text
/// input and cursor state.
#[test]
fn two_sources_keep_their_own_state() {
    let mut context = AppContext::new();
    let (left, right) = (document(30), document(31));
    let mut left_input =
        HeadlessInput::bind_source(&mut context, InputSourceId(1), EndpointGeneration(1), left)
            .unwrap();
    let mut right_input =
        HeadlessInput::bind_source(&mut context, InputSourceId(2), EndpointGeneration(1), right)
            .unwrap();
    focused_editor(&mut context, left, "left");
    left_input
        .route(&mut context, InputPayload::Focus { focused: true })
        .unwrap();
    right_input
        .route(&mut context, InputPayload::Focus { focused: true })
        .unwrap();
    assert!(left_input.services().text_input().is_some());
    assert_eq!(right_input.services().text_input(), None);
}

/// A headless session that never drains anything still routes forever.
#[test]
fn ten_thousand_mixed_events_route_headless() {
    let mut context = AppContext::new();
    let doc = document(40);
    let mut input = HeadlessInput::bind(&mut context, doc);
    focused_editor(&mut context, doc, "");
    for index in 0..10_000u32 {
        input.advance(Duration::from_millis(1));
        let x = (index % 200) as f32;
        let payload = match index % 5 {
            0 => pointer(PointerPhase::Move, x, 20.0),
            1 => pointer(PointerPhase::Down, x, 20.0),
            2 => pointer(PointerPhase::Up, x, 20.0),
            3 => InputPayload::Composition(CompositionInput::Update {
                text: "a".into(),
                selection: None,
            }),
            _ => InputPayload::Composition(CompositionInput::End),
        };
        input.route(&mut context, payload).unwrap();
    }
    assert_eq!(context.input_counters().events_routed, 10_000);
}

/// Routing takes each event's time from its timestamp: a tooltip opens
/// after its delay measured on the event clock, not on a clock frozen at
/// zero.
#[test]
fn event_timestamps_drive_timed_behaviour() {
    let mut context = AppContext::new();
    let doc = document(41);
    let mut input = HeadlessInput::bind(&mut context, doc);
    let node = hittable_node(&mut context, doc, 1);
    input.set_now(Duration::from_millis(250));
    input
        .pointer(&mut context, PointerPhase::Move, 10.0, 12.0)
        .unwrap();
    assert_eq!(context.world().pointer_hover(doc, 1), Some(node));
    assert_eq!(context.component_lifecycle.now, Duration::from_millis(250));
}

#[derive(Default)]
struct RefusingClipboard;

impl HostServices for RefusingClipboard {
    fn set_cursor(&mut self, _cursor: CursorIcon) {}
    fn set_text_input(&mut self, _state: Option<&TextInputContext>) {}
    fn read_clipboard(&mut self) -> Result<Option<String>, HostServiceError> {
        Err(HostServiceError::Busy)
    }
    fn write_clipboard(&mut self, _text: &str) -> Result<(), HostServiceError> {
        Err(HostServiceError::Busy)
    }
}

/// A busy clipboard is an answer, not a wait: paste does nothing and the
/// key is not consumed.
#[test]
fn a_busy_clipboard_is_not_waited_on() {
    let mut context = AppContext::new();
    let doc = document(42);
    let mut input = HeadlessInput::bind(&mut context, doc);
    let editor = focused_editor(&mut context, doc, "x");
    let event = input.stamp(InputPayload::Key(key("v", control())));
    let outcome = context
        .route_input(&event, &mut RefusingClipboard, None)
        .unwrap();
    assert!(!outcome.handled);
    assert_eq!(context.world().text(editor.stable_id()), Some("x"));
}

fn file_drag(kind: nana_ui_core::FileDragKind, position: Option<(f32, f32)>) -> InputPayload {
    InputPayload::FileDrag(nana_ui_input::FileDragInput {
        kind,
        paths: vec![std::path::PathBuf::from("/tmp/note.md")],
        position,
        modifiers: InputModifiers::default(),
    })
}

/// A file drag reaches the drop target under it through the router, which
/// names that target as the event's hit; a blur or a disconnect ends the
/// hover as the drag leaving would.
#[test]
fn file_drags_route_to_the_drop_target_and_end_with_their_source() {
    use nana_ui_core::{DropAccepts, FileDragKind};

    let mut context = AppContext::new();
    let doc = document(5);
    let target = hittable_node(&mut context, doc, 1);
    context
        .set_drop_target_node(target, DropAccepts::files())
        .unwrap();
    let generation = EndpointGeneration(1);
    let (source, other) = (InputSourceId(20), InputSourceId(21));
    context.bind_input_source(source, generation, doc).unwrap();
    context.bind_input_source(other, generation, doc).unwrap();
    // A drawn frame: the hover change below is new work.
    context.take_system_work();
    let mut services = UnsupportedHostServices;
    let mut route = |context: &mut AppContext, owner, sequence, payload| {
        context
            .route_input(
                &event(owner, generation, sequence, payload),
                &mut services,
                None,
            )
            .unwrap()
    };

    let hovered = route(
        &mut context,
        source,
        1,
        file_drag(FileDragKind::Hover, Some((10.0, 10.0))),
    );
    assert!(hovered.handled);
    assert!(
        hovered.invalidated_work,
        "the host redraws the hover chrome"
    );
    assert_eq!(hovered.pointer_hit, Some(target));
    assert_eq!(context.drop_hover().map(|(id, _)| id), Some(target));

    // Another window losing focus has no drag to end.
    route(
        &mut context,
        other,
        1,
        InputPayload::Focus { focused: false },
    );
    assert_eq!(context.drop_hover().map(|(id, _)| id), Some(target));
    route(
        &mut context,
        source,
        2,
        InputPayload::Focus { focused: false },
    );
    assert_eq!(context.drop_hover(), None);

    route(
        &mut context,
        source,
        3,
        file_drag(FileDragKind::Hover, Some((10.0, 10.0))),
    );
    route(&mut context, source, 4, InputPayload::SourceDisconnected);
    assert_eq!(context.drop_hover(), None);

    let dropped = route(
        &mut context,
        other,
        2,
        file_drag(FileDragKind::Drop, Some((10.0, 10.0))),
    );
    assert!(dropped.handled);
    assert_eq!(dropped.pointer_hit, Some(target));
}

#[test]
fn file_drag_paths_count_against_the_endpoint_budget() {
    let payload = file_drag(nana_ui_core::FileDragKind::Drop, Some((0.0, 0.0)));
    let InputPayload::FileDrag(drag) = &payload else {
        unreachable!()
    };
    assert!(
        payload.allocation_bytes()
            >= drag.paths[0].capacity() + std::mem::size_of::<std::path::PathBuf>()
    );
}

/// Observers hear about the pointer the router keys capture and hover by:
/// after a blur hands the platform pointer a new local id, the drained event
/// carries that id, so a capture an observer asks for is the one routed.
#[test]
fn drained_events_name_the_pointer_the_router_follows() {
    let mut context = AppContext::new();
    let doc = document(31);
    let source = InputSourceId(31);
    let generation = EndpointGeneration(1);
    context.bind_input_source(source, generation, doc).unwrap();
    let node = hittable_node(&mut context, doc, 3);
    let mut services = UnsupportedHostServices;
    let mut endpoint = InputEndpoint::new(8, 1024);
    let mut routed = Vec::new();
    let mut sequence = 0;
    let mut drain = |context: &mut AppContext, payload, routed: &mut Vec<RoutedEvent>| {
        sequence += 1;
        endpoint
            .push(event(source, generation, sequence, payload))
            .unwrap();
        context.drain_input(&mut endpoint, &mut services, None, routed);
    };
    drain(
        &mut context,
        pointer(PointerPhase::Move, 10.0, 12.0),
        &mut routed,
    );
    drain(
        &mut context,
        InputPayload::Focus { focused: false },
        &mut routed,
    );
    drain(
        &mut context,
        pointer(PointerPhase::Move, 10.0, 12.0),
        &mut routed,
    );
    let observed = routed.last().unwrap().event.pointer_id().unwrap().0;
    let local = context.input.sources[&source].pointers[&(DeviceId(1), PointerId(42))].local;
    assert_eq!(observed, local);
    assert_ne!(observed, 42, "the platform id is not the context's");
    let mut capture = MutationQueue::new();
    capture.capture_pointer(observed, node);
    context.commit_mutations(capture).unwrap();
    assert_eq!(context.world().pointer_capture(doc, local), Some(node));
}

/// On a Cyrillic layout Ctrl+C reports the logical key `с`: the shortcut
/// still copies, by the key's place on the keyboard.
#[test]
fn shortcuts_follow_the_physical_key_on_a_non_latin_layout() {
    let mut context = AppContext::new();
    let doc = document(32);
    let mut input = HeadlessInput::bind(&mut context, doc);
    focused_editor(&mut context, doc, "текст");
    context.select_all_focused_text(doc).unwrap();
    let cyrillic = KeyInput {
        physical: nana_ui_input::PhysicalKey("KeyC".into()),
        logical: nana_ui_input::LogicalKey("с".into()),
        state: KeyState::Pressed,
        repeat: false,
        modifiers: control(),
    };
    assert!(
        input
            .press(&mut context, cyrillic, None, None)
            .unwrap()
            .handled
    );
    assert_eq!(input.services().clipboard(), Some("текст"));
}

/// The IME hears where the caret shows: a field scrolled 300 px left
/// anchors its candidates 300 px left of its layout box.
#[test]
fn the_ime_anchor_follows_the_scroll_above_the_field() {
    let mut context = AppContext::new();
    let doc = document(33);
    let mut input = HeadlessInput::bind(&mut context, doc);
    let scroll = context
        .create_component(
            doc,
            crate::ScrollView::new(crate::ScrollAxes::Horizontal).style({
                let mut style = NodeStyle::default();
                let layout = std::sync::Arc::make_mut(&mut style.layout);
                layout.width = Some(nana_ui_core::LengthSpec::Px(240.0));
                layout.height = Some(nana_ui_core::LengthSpec::Px(60.0));
                layout.direction = Some(nana_ui_core::FlexDirection::Row);
                style
            }),
        )
        .unwrap();
    let spacer = crate::Stack::column(0.0).with_layout(|layout| {
        layout.width = Some(nana_ui_core::LengthSpec::Px(300.0));
        layout.height = Some(nana_ui_core::LengthSpec::Px(32.0));
        layout.flex_shrink = Some(0.0);
    });
    let spacer = context.create_component(doc, spacer).unwrap();
    context.append_child(scroll, spacer).unwrap();
    let mut field = TextInput::new("text");
    {
        let layout = std::sync::Arc::make_mut(&mut field.style.layout);
        layout.width = Some(nana_ui_core::LengthSpec::Px(200.0));
        layout.flex_shrink = Some(0.0);
    }
    let field = context.create_component(doc, field).unwrap();
    context.append_child(scroll, field).unwrap();
    let viewport = crate::LayoutViewport::new(400.0, 200.0);
    context.layout_document(doc, viewport).unwrap();
    context
        .scroll_to(scroll, crate::ScrollOffset { x: 300.0, y: 0.0 })
        .unwrap();
    context.layout_document(doc, viewport).unwrap();
    context.rebuild_hit_test(doc);
    assert!(context.focus_node(doc, field.stable_id()).unwrap());
    input
        .route(&mut context, InputPayload::Focus { focused: true })
        .unwrap();
    let anchor = input
        .services()
        .text_input()
        .and_then(|state| state.cursor_area)
        .expect("caret anchor");
    let laid_out = context.world().layout_box(field.stable_id()).unwrap();
    assert!(
        anchor.x < laid_out.x - 250.0,
        "anchor {anchor:?} vs field box {laid_out:?}"
    );
}
