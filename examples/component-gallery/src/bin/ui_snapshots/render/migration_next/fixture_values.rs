//! Snapshot fixture values; no product state is stored here.
use super::*;

pub(super) fn snapshot_graph() -> GraphModel {
    let source = GraphNode::new(
        "source",
        "In",
        GraphPoint::new(16.0, 36.0),
        GraphSize::new(96.0, 48.0),
    )
    .with_port(GraphPort::new(
        "out",
        "Out",
        GraphPortKind::Output,
        GraphPortSide::Right,
    ));
    let target = GraphNode::new(
        "target",
        "Out",
        GraphPoint::new(180.0, 36.0),
        GraphSize::new(96.0, 48.0),
    )
    .with_port(GraphPort::new(
        "in",
        "In",
        GraphPortKind::Input,
        GraphPortSide::Left,
    ));
    GraphModel::new(
        vec![source, target],
        vec![GraphEdge::new(
            "link",
            GraphEndpoint::new("source", "out"),
            GraphEndpoint::new("target", "in"),
        )],
    )
    .expect("snapshot graph is valid")
}

pub(super) fn set_full_width(style: &mut NodeStyle) {
    Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Percent(100.0));
}

pub(super) fn control_size(state: &str) -> ControlSize {
    match state {
        "medium" => ControlSize::Medium,
        "large" => ControlSize::Large,
        _ => ControlSize::Small,
    }
}

pub(super) fn segmented_control_size(state: &str) -> ControlSize {
    match state {
        "small" => ControlSize::Small,
        "large" => ControlSize::Large,
        _ => ControlSize::Medium,
    }
}

pub(super) fn button_control_size(state: &str) -> ControlSize {
    match state {
        "small" => ControlSize::Small,
        "large" => ControlSize::Large,
        _ => ControlSize::Medium,
    }
}

pub(super) fn text_input_control_size(state: &str) -> ControlSize {
    match state {
        "small" => ControlSize::Small,
        "large" => ControlSize::Large,
        _ => ControlSize::Medium,
    }
}

pub(super) fn textarea_value(state: &str) -> &'static str {
    match state {
        "placeholder" => "",
        "clipped" | "scroll" => {
            "First line\nSecond line\nThird line\nFourth line\nFifth line\nSixth line stays"
        }
        _ => "First line\nSecond line\nThird line",
    }
}

pub(super) fn hosted_textarea_value(state: &str) -> &'static str {
    match state {
        "placeholder" => "",
        _ => "fn main() {\n    let ready = true;\n}\n",
    }
}

pub(super) fn button_kind(state: &str) -> nana_ui::ButtonKind {
    match state {
        "subtle" => nana_ui::ButtonKind::Subtle,
        "selected" => nana_ui::ButtonKind::Selected,
        "primary" => nana_ui::ButtonKind::Primary,
        "warning" => nana_ui::ButtonKind::Warning,
        "danger" => nana_ui::ButtonKind::Danger,
        "text-kind" => nana_ui::ButtonKind::Text,
        _ => nana_ui::ButtonKind::Ghost,
    }
}

pub(super) fn card_kind(state: &str) -> CardKind {
    match state {
        "outlined" => CardKind::Outlined,
        "raised" => CardKind::Raised,
        "flat" => CardKind::Flat,
        "selected" => CardKind::Selected,
        _ => CardKind::Surface,
    }
}

pub(super) fn range_value(state: &str) -> f64 {
    match state {
        "minimum" => 0.0,
        "maximum" => 1.0,
        "decimal-step" => 0.34,
        "arrow-decrement" | "page-decrement" => 0.7,
        "arrow-increment" | "page-increment" => 0.3,
        _ => 0.5,
    }
}

pub(super) fn status_tone(state: &str) -> StatusTone {
    match state {
        "info" => StatusTone::Info,
        "success" => StatusTone::Success,
        "warning" => StatusTone::Warning,
        "danger" => StatusTone::Danger,
        _ => StatusTone::Neutral,
    }
}

pub(super) fn status_badge_label(state: &str) -> &'static str {
    match state {
        "info" => "Syncing",
        "success" => "Ready",
        "warning" => "Delayed",
        "danger" => "Offline",
        _ => "Idle",
    }
}

pub(super) fn validation_intent(state: &str) -> ValidationIntent {
    if state == "warning" {
        ValidationIntent::Warning
    } else {
        ValidationIntent::Danger
    }
}

pub(super) fn validation_message(state: &str) -> &'static str {
    if state == "warning" {
        "This name may be ambiguous"
    } else {
        "A project name is required"
    }
}

pub(super) fn empty_title(state: &str) -> &'static str {
    match state {
        "narrow-cjk" | "extreme-clip" => "暂无匹配的项目 👩🏽‍💻",
        "compact" => "No recent projects",
        "title-only" => "Nothing selected",
        _ => "No projects yet",
    }
}

pub(super) fn empty_message(state: &str) -> &'static str {
    match state {
        "narrow-cjk" | "extreme-clip" => {
            "请调整筛选条件，或新建一个包含协作者、标签与说明的项目 🚀"
        }
        "compact" => "Open a project to see it here",
        _ => "Create the first project in this workspace",
    }
}
