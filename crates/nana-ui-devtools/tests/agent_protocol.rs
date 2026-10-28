#![cfg(feature = "runtime-agent")]
//! Protocol-surface behaviour: what an Agent can read and address without a
//! window. These are deliberately GPU-free, so an adapter-less machine still
//! exercises the whole structural surface.

use nana_ui::runtime::{Checkbox, DocumentId, RangeField, RuntimeDocument, Switch, TextInput};
use nana_ui_devtools::agent::{RuntimeAgentSession, fixtures};

fn session_with<F>(build: F) -> RuntimeAgentSession
where
    F: FnOnce(&mut RuntimeDocument, DocumentId),
{
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    build(&mut document, id);
    RuntimeAgentSession::new(document, 320, 240).expect("session")
}

/// A dump that stops at `label` forces every state question — "is the switch
/// on", "is the field editable", "what is the slider's range" — back into
/// hand-written Rust. The projection must carry the state the Runtime already
/// computed.
#[test]
fn accessibility_dump_carries_state_not_only_labels() {
    let mut session = session_with(|document, id| {
        let cx = document.context_mut();
        cx.create_component(id, Switch::new("Notifications", true))
            .unwrap();
        cx.create_component(id, Checkbox::new("Archive", false))
            .unwrap();
        cx.create_component(id, TextInput::new("draft")).unwrap();
        cx.create_component(id, RangeField::new(4.0, 0.0, 10.0, 1.0))
            .unwrap();
    });
    session.flush().expect("flush");
    let nodes = session.accessibility_dump();

    let switch = nodes
        .iter()
        .find(|node| node.role == "switch")
        .expect("switch projects");
    assert_eq!(
        switch.checked,
        Some(true),
        "an on switch must report checked, got {switch:?}"
    );

    let checkbox = nodes
        .iter()
        .find(|node| node.role == "checkbox")
        .expect("checkbox projects");
    assert_eq!(checkbox.checked, Some(false));

    let input = nodes
        .iter()
        .find(|node| node.role == "text-input")
        .expect("text input projects");
    assert!(input.editable, "a text input must report editable");

    let slider = nodes
        .iter()
        .find(|node| node.role == "slider")
        .expect("slider projects");
    assert_eq!(slider.numeric_minimum, Some(0.0));
    assert_eq!(slider.numeric_maximum, Some(10.0));
    assert_eq!(slider.numeric_value, Some(4.0));
}

/// A tree's rows are nodes of their own: the tree names itself with
/// `.label()`, each row carries its name, level, selection and, on a branch,
/// whether it is open, and a click on a row selects it.
#[test]
fn accessibility_dump_carries_tree_rows() {
    use nana_ui::runtime::{TreeNode, TreeView};
    let mut session = session_with(|document, id| {
        document
            .context_mut()
            .create_component(
                id,
                TreeView::new([
                    TreeNode::branch(
                        "scene".into(),
                        "场景",
                        true,
                        [TreeNode::leaf("model".into(), "模型")],
                    ),
                    TreeNode::leaf("camera".into(), "相机").selected(true),
                ])
                .label("大纲"),
            )
            .unwrap();
    });
    session.flush().expect("flush");
    let nodes = session.accessibility_dump();
    let tree = nodes
        .iter()
        .find(|node| node.role == "tree")
        .expect("tree projects");
    assert_eq!(tree.label.as_deref(), Some("大纲"));
    assert_eq!(tree.value, None);
    let rows = nodes
        .iter()
        .filter(|node| node.role == "tree-item")
        .collect::<Vec<_>>();
    assert_eq!(
        tree.children,
        rows.iter().map(|row| row.id).collect::<Vec<_>>()
    );
    let names = rows
        .iter()
        .map(|row| row.label.as_deref().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["场景", "模型", "相机"]);
    assert!(rows.iter().all(|row| row.value.is_none()));
    let levels = rows.iter().map(|row| row.level).collect::<Vec<_>>();
    assert_eq!(levels, [Some(1), Some(2), Some(1)]);
    let expanded = rows.iter().map(|row| row.expanded).collect::<Vec<_>>();
    assert_eq!(expanded, [Some(true), None, None]);
    let selected = rows.iter().map(|row| row.selected).collect::<Vec<_>>();
    assert_eq!(selected, [Some(false), Some(false), Some(true)]);
    let leaf = serde_json::to_string(rows[1]).expect("serialize");
    assert!(!leaf.contains("expanded"), "a leaf cannot open: {leaf}");

    let model = rows[1].id;
    assert!(session.click_node(model).expect("click"));
    let selected = session
        .accessibility_dump()
        .into_iter()
        .filter(|node| node.role == "tree-item")
        .map(|row| row.selected)
        .collect::<Vec<_>>();
    assert_eq!(selected, [Some(false), Some(true), Some(false)]);
}

/// A row's toggles are checkboxes under the row, named by their columns and
/// carrying their state as `checked`; a click flips one and leaves the
/// selection alone.
#[test]
fn accessibility_dump_carries_tree_row_toggles() {
    use nana_ui::runtime::{TreeNode, TreeToggleColumn, TreeView};
    let mut session = session_with(|document, id| {
        document
            .context_mut()
            .create_component(
                id,
                TreeView::new([
                    TreeNode::leaf("model".into(), "模型")
                        .selected(true)
                        .toggles([true, false]),
                    TreeNode::leaf("camera".into(), "相机").toggles([false, false]),
                ])
                .columns([
                    TreeToggleColumn::new(nana_ui_core::Icon::Eye, "显示"),
                    TreeToggleColumn::new(nana_ui_core::Icon::ShieldCheck, "锁定"),
                ])
                .label("大纲"),
            )
            .unwrap();
    });
    session.flush().expect("flush");
    let nodes = session.accessibility_dump();
    let rows = nodes
        .iter()
        .filter(|node| node.role == "tree-item")
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    let toggles = |row: &nana_ui_devtools::agent::AccessibilityDumpNode| {
        row.children
            .iter()
            .map(|child| {
                nodes
                    .iter()
                    .find(|node| node.id == *child)
                    .expect("a node for every child")
                    .clone()
            })
            .collect::<Vec<_>>()
    };
    let model = toggles(rows[0]);
    assert!(model.iter().all(|toggle| toggle.role == "checkbox"));
    assert!(model.iter().all(|toggle| toggle.parent == Some(rows[0].id)));
    let names = model
        .iter()
        .map(|toggle| toggle.label.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(names, [Some("显示"), Some("锁定")]);
    let checked = model
        .iter()
        .map(|toggle| toggle.checked)
        .collect::<Vec<_>>();
    assert_eq!(checked, [Some(true), Some(false)]);
    let wire = serde_json::to_string(&model[1]).expect("serialize");
    assert!(wire.contains(r#""checked":false"#), "{wire}");

    let locked = toggles(rows[1])[1].id;
    assert!(session.click_node(locked).expect("click"));
    let nodes = session.accessibility_dump();
    let flipped = nodes.iter().find(|node| node.id == locked).expect("toggle");
    assert_eq!(flipped.checked, Some(true));
    let selected = nodes
        .iter()
        .filter(|node| node.role == "tree-item")
        .map(|row| row.selected)
        .collect::<Vec<_>>();
    assert_eq!(selected, [Some(true), Some(false)], "the selection stays");
}

/// Defaults must not bloat every node: a plain switch carries no `selection`,
/// no `numeric_*` and no `modal` key on the wire.
#[test]
fn default_fields_stay_off_the_wire() {
    let mut session = session_with(|document, id| {
        document
            .context_mut()
            .create_component(id, Switch::new("Notifications", false))
            .unwrap();
    });
    session.flush().expect("flush");
    let node = session
        .accessibility_dump()
        .into_iter()
        .find(|node| node.role == "switch")
        .expect("switch projects");
    let json = serde_json::to_string(&node).expect("serialize");

    assert!(json.contains("\"checked\""), "state present: {json}");
    for absent in ["selection", "numeric_minimum", "modal", "busy", "invalid"] {
        assert!(
            !json.contains(absent),
            "{absent} is at its default and must be skipped: {json}"
        );
    }
}

// ---- selector, dispatch and stdio ----

use nana_ui_devtools::agent::{AgentSession, Target, ThemeName, run_stdio};

fn counter() -> RuntimeAgentSession {
    let document = fixtures::build("counter").expect("counter fixture");
    RuntimeAgentSession::new(document, 320, 240).expect("session")
}

/// A driver that pipes a batch of commands must not lose every later step to
/// one typo, and must be able to key replies by request rather than by array
/// position.
#[test]
fn a_malformed_line_is_answered_and_the_session_continues() {
    let mut session = counter();
    let input = concat!(
        "{\"id\":1,\"cmd\":\"info\"}\n",
        "not json\n",
        "{\"id\":3,\"cmd\":\"pump\"}\n"
    );
    let mut output = Vec::new();
    run_stdio(&mut session, &mut input.as_bytes(), &mut output).expect("stdio");

    let replies: Vec<serde_json::Value> = String::from_utf8(output)
        .expect("utf-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("reply json"))
        .collect();
    assert_eq!(replies.len(), 3, "every line is answered: {replies:?}");
    assert_eq!(replies[0]["ok"], true);
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[0]["cmd"], "info");
    assert_eq!(replies[1]["ok"], false, "the bad line fails on its own");
    assert_eq!(replies[2]["ok"], true, "later commands still run");
    assert_eq!(replies[2]["id"], 3);
}

/// `role`/`label` is the selector a Rust L3 tree can always use. It must reach
/// the same node an explicit id reaches.
#[test]
fn a_role_and_label_selector_resolves_to_the_same_node_as_an_id() {
    let session = counter();
    let by_selector = session
        .resolve(&Target {
            role: Some("button".into()),
            label: Some("Increment 0".into()),
            ..Target::default()
        })
        .expect("selector resolves");
    let by_id = session
        .resolve(&Target {
            node: Some(by_selector),
            ..Target::default()
        })
        .expect("id resolves");
    assert_eq!(by_selector, by_id);
}

/// An arbitrary pick would silently exercise the wrong widget, so an ambiguous
/// selector has to fail and say how many matched.
#[test]
fn an_ambiguous_selector_fails_instead_of_guessing() {
    let document = fixtures::build("scroll-list").expect("scroll fixture");
    let session = RuntimeAgentSession::new(document, 360, 240).expect("session");
    let ambiguous = Target {
        role: Some("button".into()),
        ..Target::default()
    };
    let error = session.resolve(&ambiguous).expect_err("many rows match");
    assert!(
        error.0.contains("nth"),
        "the error must say how to disambiguate: {error}"
    );

    let first = session
        .resolve(&Target {
            nth: Some(0),
            ..ambiguous
        })
        .expect("nth disambiguates");
    assert!(first > 0);
}

/// "I clicked and nothing happened" and "why is this invisible" are the two
/// questions a screenshot alone cannot answer.
#[test]
fn a_covered_node_reports_what_covers_it() {
    let document = fixtures::build("occlusion").expect("occlusion fixture");
    let session = RuntimeAgentSession::new(document, 320, 120).expect("session");
    let covered = session
        .resolve(&Target {
            label: Some("covered".into()),
            ..Target::default()
        })
        .expect("the covered label projects");

    let probe = session
        .scene_probe(covered)
        .expect("covered node is painted");
    assert_eq!(probe.verdict, "occluded", "{probe:?}");
    let over = probe.occluded_by.expect("something covers it");
    assert_ne!(over.node, covered);

    let hits = session.hit_test(160.0, 60.0);
    assert_eq!(
        hits.first().map(|hit| hit.node),
        Some(over.node),
        "the topmost hit is what the probe named: {hits:?}"
    );
}

/// A responsive regression must not need a process restart.
#[test]
fn resizing_reprojects_geometry_within_one_session() {
    let document = fixtures::build("scroll-list").expect("scroll fixture");
    let mut session = RuntimeAgentSession::new(document, 640, 240).expect("session");
    let wide = session.describe();
    assert_eq!(wide.width, 640);

    AgentSession::set_viewport(&mut session, 280, 240, 1.0).expect("resize");
    assert_eq!(session.describe().width, 280);

    assert!(
        AgentSession::set_viewport(&mut session, 0, 240, 1.0).is_err(),
        "a zero viewport is refused rather than producing an empty frame"
    );
}

/// The clear colour used to be a fixed light grey, so every dark-theme
/// screenshot showed a light background the product never paints.
#[test]
fn the_clear_colour_follows_the_active_theme() {
    let mut session = counter();
    AgentSession::set_theme(&mut session, ThemeName::Light).expect("light");
    let light = session.describe().clear;
    AgentSession::set_theme(&mut session, ThemeName::Dark).expect("dark");
    let dark = session.describe().clear;

    assert_ne!(light, dark, "the two themes cannot share a background");
    let luma = |clear: [f32; 4]| clear[0] + clear[1] + clear[2];
    assert!(
        luma(dark) < luma(light),
        "dark must be darker: {dark:?} vs {light:?}"
    );

    // An explicit clear still wins, and `None` restores theme-following.
    AgentSession::set_clear(&mut session, Some([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(session.describe().clear, [1.0, 0.0, 0.0, 1.0]);
    AgentSession::set_clear(&mut session, None);
    assert_eq!(session.describe().clear, dark);
}

/// The protocol must stay drivable through a trait object, so a product's own
/// session can join.
#[test]
fn a_runtime_session_drives_the_protocol_as_a_trait_object() {
    let mut session = counter();
    let driven: &mut dyn AgentSession = &mut session;
    let mut output = Vec::new();
    run_stdio(
        driven,
        &mut "{\"cmd\":\"a11y\",\"role\":\"button\"}\n".as_bytes(),
        &mut output,
    )
    .expect("stdio");
    let reply: serde_json::Value =
        serde_json::from_str(String::from_utf8(output).unwrap().trim()).expect("json");
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["nodes"].as_array().expect("nodes").len(), 1);
}

/// A Rust L3 tree has no `data-agent-id`, so the `build`/`mount` assembly key
/// is its stable handle. It only exists for keyed trees, which is exactly why
/// the `role`/`label` selector is not optional.
#[test]
fn keyed_trees_expose_an_assembly_path_and_unkeyed_ones_do_not() {
    let keyed = counter();
    let button = keyed
        .accessibility_dump()
        .into_iter()
        .find(|node| node.role == "button")
        .expect("button projects");
    assert_eq!(button.agent_path.as_deref(), Some("increment"));
    assert_eq!(
        keyed
            .resolve(&Target {
                agent_path: Some("increment".into()),
                ..Target::default()
            })
            .expect("path resolves"),
        button.id
    );

    // Nested keys join with `/`.
    let nested = RuntimeAgentSession::new(
        fixtures::build("scroll-list").expect("scroll fixture"),
        360,
        240,
    )
    .expect("session");
    assert!(
        nested
            .accessibility_dump()
            .iter()
            .any(|node| node.agent_path.as_deref() == Some("list/row-0")),
        "nested assembly keys join into a path"
    );

    // `create_component` never went through keyed assembly, so it has no path
    // and must still be addressable by role and label.
    let mut unkeyed = session_with(|document, id| {
        document
            .context_mut()
            .create_component(id, Switch::new("Notifications", true))
            .unwrap();
    });
    unkeyed.flush().expect("flush");
    let switch = unkeyed
        .accessibility_dump()
        .into_iter()
        .find(|node| node.role == "switch")
        .expect("switch projects");
    assert_eq!(switch.agent_path, None);
    assert_eq!(
        unkeyed
            .resolve(&Target {
                role: Some("switch".into()),
                ..Target::default()
            })
            .expect("role still resolves it"),
        switch.id
    );
}

/// The whole point of the session: a click has to change pixels, and the reply
/// has to say so without anyone opening the PNG. Skips visibly with no adapter.
#[test]
fn a_click_changes_the_frame_and_the_reply_reports_it() {
    if !nana_ui_devtools::offscreen::pixels_available() {
        return;
    }
    let mut session = counter();
    let dir = std::env::temp_dir().join("nana-agent-click-evidence");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let before = dir.join("before.png");

    let stats = session.screenshot_png(&before).expect("screenshot");
    assert!(
        stats.unique_colors > 8,
        "the fixture must paint more than a clear colour: {stats:?}"
    );

    let target = session
        .resolve(&Target {
            agent_path: Some("increment".into()),
            ..Target::default()
        })
        .expect("increment resolves");
    assert!(session.activate(target).expect("click"));

    let mut output = Vec::new();
    run_stdio(
        &mut session,
        &mut format!(
            "{{\"cmd\":\"diff\",\"baseline\":{:?}}}\n",
            before.to_str().unwrap()
        )
        .as_bytes(),
        &mut output,
    )
    .expect("stdio");
    let reply: serde_json::Value =
        serde_json::from_str(String::from_utf8(output).unwrap().trim()).expect("json");
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(
        reply["diff"]["changed_pixels"].as_u64().unwrap_or(0) > 0,
        "the click must change the painted frame: {reply}"
    );
}

/// The view inspector: a bound control's fields, where its bindings were
/// declared and why it last changed, and editing a field from text.
#[cfg(feature = "reactive-trace")]
#[test]
fn inspect_names_the_writes_behind_a_node_and_set_field_edits_it() {
    use nana_ui::runtime::view::{button, signal};
    use nana_ui_devtools::agent::{AgentCommand, AgentSession, Target};
    let busy = std::cell::Cell::new(None);
    let mut session = session_with(|document, id| {
        let cx = document.context_mut();
        let parent = cx
            .create_component(id, nana_ui::runtime::Stack::column(0.0))
            .unwrap()
            .stable_id();
        cx.mount_view(parent, || {
            let loading = signal(false);
            busy.set(Some(loading));
            button("保存").loading(loading)
        })
        .unwrap();
    });
    session.flush().expect("flush");
    busy.get().unwrap().set(true);
    session.flush().expect("flush");
    let target = Target {
        role: Some("button".into()),
        label: Some("保存".into()),
        ..Target::default()
    };
    let reply = session.execute(AgentCommand::Inspect {
        target: target.clone(),
    });
    let inspect = reply.inspect.expect("an inspection");
    assert_eq!(inspect.control.as_deref(), Some("Button"));
    let loading = inspect
        .fields
        .iter()
        .find(|field| field.name == "loading")
        .unwrap();
    assert_eq!(loading.value, "true");
    assert!(
        loading
            .bound_at
            .as_deref()
            .unwrap()
            .contains("agent_protocol.rs")
    );
    assert!(
        inspect
            .causes
            .iter()
            .any(|cause| cause.written_at.contains("agent_protocol.rs")),
        "{inspect:?}"
    );

    let reply = session.execute(AgentCommand::SetField {
        target,
        field: "label".into(),
        value: "另存".into(),
    });
    assert!(reply.ok, "{reply:?}");
    let label = reply
        .inspect
        .unwrap()
        .fields
        .into_iter()
        .find(|field| field.name == "label")
        .unwrap();
    assert_eq!(label.value, "\"另存\"");
}

#[test]
fn inspect_prefers_compiler_source_positions_without_changing_the_wire_shape() {
    use nana_ui::runtime::view::{SourceLocation, ViewSource, button, signal};
    use nana_ui_devtools::agent::{AgentCommand, AgentSession, Target};
    static SOURCE: ViewSource = ViewSource {
        element: SourceLocation::new("views/Page.vue", 8, 4),
        fields: &[("loading", SourceLocation::new("views/Page.vue", 9, 12))],
    };
    let mut session = session_with(|document, id| {
        document
            .context_mut()
            .mount_view_root(id, || {
                button("Save").loading(signal(false)).source_site(&SOURCE)
            })
            .unwrap();
    });
    let reply = session.execute(AgentCommand::Inspect {
        target: Target {
            role: Some("button".into()),
            label: Some("Save".into()),
            ..Target::default()
        },
    });
    let inspect = reply.inspect.unwrap();
    assert_eq!(inspect.element.as_deref(), Some("views/Page.vue:8:4"));
    assert_eq!(
        inspect
            .fields
            .iter()
            .find(|f| f.name == "loading")
            .unwrap()
            .bound_at
            .as_deref(),
        Some("views/Page.vue:9:12")
    );
    let json = serde_json::to_value(inspect).unwrap();
    assert_eq!(json["element"], "views/Page.vue:8:4");
    assert!(json.get("source_element").is_none());
}

/// `inspect` carries the node's layout view: the context its parent runs,
/// how the node takes part in it, and the generations behind it (#197).
#[test]
fn inspect_reports_the_layout_context_and_participation() {
    use nana_ui::runtime::view::button;
    use nana_ui_devtools::agent::{AgentCommand, AgentSession, Target};
    let mut session = session_with(|document, id| {
        let cx = document.context_mut();
        let parent = cx
            .create_component(id, nana_ui::runtime::Stack::row(0.0))
            .unwrap()
            .stable_id();
        cx.mount_view(parent, || button("保存")).unwrap();
    });
    session.flush().expect("flush");
    let reply = session.execute(AgentCommand::Inspect {
        target: Target {
            role: Some("button".into()),
            label: Some("保存".into()),
            ..Target::default()
        },
    });
    let layout = reply
        .inspect
        .expect("an inspection")
        .layout_node
        .expect("a laid-out node has a layout view");
    // The parent is a row: it runs Flex and the button is one of its items.
    assert_eq!(layout.parent_context.as_deref(), Some("flex"), "{layout:?}");
    assert_eq!(
        layout.participation.as_deref(),
        Some("flexitem"),
        "{layout:?}"
    );
    assert!(layout.result_generation.is_some(), "{layout:?}");
    assert!(layout.intent.starts_with("display="), "{layout:?}");
    let json = serde_json::to_value(&layout).unwrap();
    assert_eq!(json["participation"], "flexitem");
}
