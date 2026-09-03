//! Snapshot selection; no product state is stored here.
use super::*;

pub(super) fn create_segmented_fixture(
    document: &mut RuntimeDocument,
    fixture: Fixture,
) -> Result<SegmentedFixture, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let specs: &[(&str, Option<Icon>, bool)] = match fixture.state {
        "empty" => &[],
        "all-disabled" => &[
            ("Code", None, true),
            ("Split", None, true),
            ("Preview", None, true),
        ],
        "medium-icon" => &[
            ("Code", Some(Icon::File), false),
            ("Split", None, true),
            ("Preview", None, false),
        ],
        _ => &[
            ("Code", None, false),
            ("Split", None, true),
            ("Preview", None, false),
        ],
    };
    let (control, options) = document.context_mut().build(document_id, |ui| {
        let control = ui.child(
            "segmented",
            RuntimeSegmentedControl::new()
                .label("Editor mode")
                .size(segmented_control_size(fixture.state)),
        );
        let mut options = Vec::with_capacity(specs.len());
        for (label, icon, disabled) in specs {
            let mut option = RuntimeSegmentedOption::new(*label).disabled(*disabled);
            if let Some(icon) = icon {
                option = option.icon(*icon);
            }
            options.push(ui.leaf(option));
        }
        (control, options)
    })?;
    let selected = match fixture.state {
        "empty" | "all-disabled" | "no-selection" => None,
        "disabled-selected" => options.get(1).copied(),
        _ => options.first().copied(),
    };
    document
        .context_mut()
        .set_segmented_options(control, options.clone(), selected)?;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    document.context_mut().on(
        control,
        move |_control, event: &SegmentedSelectionRequested, _context| {
            observed
                .lock()
                .expect("segmented request log")
                .push(event.option);
        },
    )?;
    Ok(SegmentedFixture {
        control,
        options,
        requests,
    })
}

pub(super) fn create_tabs_fixture(
    document: &mut RuntimeDocument,
    fixture: Fixture,
) -> Result<Entity<RuntimeTabs>, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let tabs = document.context_mut().build(document_id, |ui| {
        ui.child(
            "tabs",
            RuntimeTabs::new("code").label("Editor mode").options([
                RuntimeTabOption::new("code", "Code"),
                RuntimeTabOption::new("split", "Split").disabled(true),
                RuntimeTabOption::new("preview", "Preview"),
            ]),
        )
    })?;
    if fixture.state == "focused"
        && let Some(first) = document
            .context()
            .read(tabs, |tabs| tabs.option_nodes().first().map(|(_, id)| *id))?
    {
        document.context_mut().focus_node(document_id, first)?;
    }
    Ok(tabs)
}

pub(super) fn exercise_segmented_contract(
    document: &mut RuntimeDocument,
    viewport: LayoutViewport,
    shaper: &mut NanaTextShaper,
    fixture: Fixture,
    segmented: &SegmentedFixture,
) -> Result<bool, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let mut adapter = RuntimeInputAdapter::default();
    let ids = segmented
        .options
        .iter()
        .map(|option| option.stable_id())
        .collect::<Vec<_>>();
    let center = |document: &RuntimeDocument, id: StableNodeId| {
        let bounds = document
            .context()
            .world()
            .layout_box(id)
            .expect("segmented option layout");
        (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
    };
    let selected_before = document
        .context()
        .read(segmented.control, RuntimeSegmentedControl::selected)?;
    let action_ok = match fixture.state {
        "empty" | "all-disabled" => {
            !document
                .context_mut()
                .navigate_sequential_focus(document_id, false)?
                && document.context().world().focused(document_id).is_none()
        }
        "hover" | "selected-hover" => {
            let id = if fixture.state == "hover" {
                ids[2]
            } else {
                ids[0]
            };
            let (x, y) = center(document, id);
            adapter
                .dispatch(
                    document.context_mut(),
                    document_id,
                    &pointer(PointerPhase::Move, x, y),
                )?
                .prevent_default
        }
        "pressed" | "selected-pressed" => {
            let id = if fixture.state == "pressed" {
                ids[2]
            } else {
                ids[0]
            };
            let (x, y) = center(document, id);
            adapter
                .dispatch(
                    document.context_mut(),
                    document_id,
                    &pointer(PointerPhase::Down, x, y),
                )?
                .prevent_default
        }
        "focused" => document.context_mut().focus_node(document_id, ids[0])?,
        "pointer-request" | "selected-repeat-request" => {
            let id = if fixture.state == "pointer-request" {
                ids[2]
            } else {
                ids[0]
            };
            let (x, y) = center(document, id);
            adapter.dispatch(
                document.context_mut(),
                document_id,
                &pointer(PointerPhase::Down, x, y),
            )?;
            adapter
                .dispatch(
                    document.context_mut(),
                    document_id,
                    &pointer(PointerPhase::Up, x, y),
                )?
                .prevent_default
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [id]
                && document
                    .context()
                    .read(segmented.control, RuntimeSegmentedControl::selected)?
                    == selected_before
        }
        "pointer-cancel" => {
            let (x, y) = center(document, ids[2]);
            adapter.dispatch(
                document.context_mut(),
                document_id,
                &pointer(PointerPhase::Down, x, y),
            )?;
            adapter
                .dispatch(
                    document.context_mut(),
                    document_id,
                    &pointer(PointerPhase::Cancel, x, y),
                )?
                .prevent_default
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .is_empty()
        }
        "arrow-skip-wrap" => {
            document.context_mut().focus_node(document_id, ids[0])?;
            let left = adapter
                .dispatch(document.context_mut(), document_id, &keyboard("ArrowLeft"))?
                .prevent_default;
            let right = adapter
                .dispatch(document.context_mut(), document_id, &keyboard("ArrowRight"))?
                .prevent_default;
            left && right
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [ids[2], ids[0]]
        }
        "home-end" => {
            document.context_mut().focus_node(document_id, ids[2])?;
            let home = adapter
                .dispatch(document.context_mut(), document_id, &keyboard("Home"))?
                .prevent_default;
            let end = adapter
                .dispatch(document.context_mut(), document_id, &keyboard("End"))?
                .prevent_default;
            home && end
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [ids[0], ids[2]]
        }
        "space-enter-repeat" => {
            document.context_mut().focus_node(document_id, ids[0])?;
            let repeated_space = adapter.dispatch(
                document.context_mut(),
                document_id,
                &keyboard_with_repeat("Space", true),
            )?;
            let repeated_enter = adapter.dispatch(
                document.context_mut(),
                document_id,
                &keyboard_with_repeat("Enter", true),
            )?;
            let normal_space = adapter.dispatch(
                document.context_mut(),
                document_id,
                &keyboard_with_repeat("Space", false),
            )?;
            let normal_enter = adapter.dispatch(
                document.context_mut(),
                document_id,
                &keyboard_with_repeat("Enter", false),
            )?;
            repeated_space.prevent_default
                && repeated_enter.prevent_default
                && normal_space.prevent_default
                && normal_enter.prevent_default
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [ids[0], ids[0]]
        }
        "no-selection" => {
            document
                .context_mut()
                .navigate_sequential_focus(document_id, false)?
                && document.context().world().focused(document_id) == Some(ids[0])
                && document
                    .context()
                    .read(segmented.control, RuntimeSegmentedControl::selected)?
                    .is_none()
        }
        "dynamic-disable" => {
            document.context_mut().focus_node(document_id, ids[0])?;
            document.context_mut().set_segmented_option_disabled(
                segmented.control,
                segmented.options[0],
                true,
            )? && document.context().world().focused(document_id) == Some(ids[2])
                && document
                    .context()
                    .read(segmented.control, RuntimeSegmentedControl::selected)?
                    == Some(ids[0])
                && document
                    .context()
                    .read(segmented.options[0], RuntimeSegmentedOption::selected)?
        }
        "controlled-commit" => {
            let (x, y) = center(document, ids[2]);
            adapter.dispatch(
                document.context_mut(),
                document_id,
                &pointer(PointerPhase::Down, x, y),
            )?;
            adapter.dispatch(
                document.context_mut(),
                document_id,
                &pointer(PointerPhase::Up, x, y),
            )?;
            let remained_controlled = document
                .context()
                .read(segmented.control, RuntimeSegmentedControl::selected)?
                == selected_before;
            let committed = document
                .context_mut()
                .set_segmented_selection(segmented.control, Some(segmented.options[2]))?;
            remained_controlled
                && committed
                && document
                    .context()
                    .read(segmented.control, RuntimeSegmentedControl::selected)?
                    == Some(ids[2])
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [ids[2]]
        }
        "a11y-radio" => {
            document.context_mut().apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target: ids[2],
                    action: AccessibilityAction::Click,
                },
            )? && document
                .context()
                .read(segmented.control, RuntimeSegmentedControl::selected)?
                == selected_before
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [ids[2]]
        }
        "atomic-reconcile" => {
            let generation = document.context().world().generation();
            let invalid = document
                .context_mut()
                .set_segmented_options(
                    segmented.control,
                    vec![segmented.options[0], segmented.options[0]],
                    Some(segmented.options[0]),
                )
                .is_err();
            let failure_atomic = document.context().world().generation() == generation
                && document
                    .context()
                    .read(segmented.control, |control| control.options().to_vec())?
                    .as_slice()
                    == ids.as_slice();
            let removed = segmented.options[1];
            let old_bounds = document
                .context()
                .world()
                .layout_box(removed.stable_id())
                .expect("option layout before parking");
            let changed = document.context_mut().set_segmented_options(
                segmented.control,
                vec![segmented.options[0], segmented.options[2]],
                Some(segmented.options[0]),
            )?;
            let update = document.flush(viewport, shaper)?;
            let parked_clean = parked_without_ghost(
                document,
                removed.stable_id(),
                old_bounds,
                &update.accessibility.removed,
            );
            let handler_preserved = document
                .context_mut()
                .request_segmented_selection(segmented.control, segmented.options[2])?
                && segmented
                    .requests
                    .lock()
                    .expect("segmented requests")
                    .as_slice()
                    == [ids[2]];
            invalid && failure_atomic && changed && parked_clean && handler_preserved
        }
        _ => true,
    };
    let selected_after = document
        .context()
        .read(segmented.control, RuntimeSegmentedControl::selected)?;
    let selection_ok = if fixture.state == "controlled-commit" {
        selected_after == ids.get(2).copied()
    } else {
        selected_after == selected_before
    };
    let expected_requests = match fixture.state {
        "pointer-request"
        | "selected-repeat-request"
        | "controlled-commit"
        | "a11y-radio"
        | "atomic-reconcile" => 1,
        "arrow-skip-wrap" | "home-end" | "space-enter-repeat" => 2,
        _ => 0,
    };
    let request_count_ok =
        segmented.requests.lock().expect("segmented requests").len() == expected_requests;
    Ok(action_ok && selection_ok && request_count_ok)
}
