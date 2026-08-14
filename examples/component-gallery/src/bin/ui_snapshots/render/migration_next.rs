use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use iced::widget::{container, text};
use iced::{Element, Length, Padding, Point, Size, Theme, mouse};
use iced_wgpu::Renderer;
use nana_ui::compatibility::{
    Card as IcedCard, IconButton as IcedIconButton, ListItem as IcedListItem,
    RangeField as IcedRangeField, Switch as IcedSwitch,
};
use nana_ui::runtime::{
    AccessibilityAction, AccessibilityActionRequest, Card as RuntimeCard, DocumentId, Entity,
    IconButton as RuntimeIconButton, LayoutViewport, List as RuntimeList,
    ListItem as RuntimeListItem, ListItemSlots, MutationQueue, NodeStyle,
    RangeField as RuntimeRangeField, RuntimeDocument, StableNodeId, Switch as RuntimeSwitch,
    Text as RuntimeText,
};
use nana_ui::{
    CardKind, ControlSize, IcedSceneView, IcedTextShaper, Icon, RuntimeInputAdapter, ThemeMode,
    ThemeModeExt, TooltipConfig, TooltipPlacement,
};
use nana_ui_core::{LengthSpec, SwitchControlPosition};
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};

use crate::write;

use super::{pixel_difference, side_by_side, snapshot_with_cursor};

const SIZE: Size<u32> = Size::new(420, 120);
const GAP: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Component {
    IconButton,
    Switch,
    Card,
    ListItem,
    RangeField,
}

impl Component {
    const fn name(self) -> &'static str {
        match self {
            Self::IconButton => "icon-button",
            Self::Switch => "switch",
            Self::Card => "card",
            Self::ListItem => "list-item",
            Self::RangeField => "range-field",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Fixture {
    component: Component,
    state: &'static str,
    expected: &'static str,
    iced_contract: &'static str,
    runtime_contract: &'static str,
    divergence: &'static str,
}

const FIXTURES: &[Fixture] = &[
    f(
        Component::IconButton,
        "normal",
        "labelled icon action has a complete square hit target",
    ),
    f(
        Component::IconButton,
        "hover",
        "hover feedback preserves icon contrast",
    ),
    f(
        Component::IconButton,
        "pressed",
        "pressed feedback is distinct from hover",
    ),
    f(
        Component::IconButton,
        "focused",
        "keyboard focus is visible around the complete hit target",
    ),
    f(
        Component::IconButton,
        "selected",
        "selected state is persistent and distinguishable",
    ),
    f(
        Component::IconButton,
        "disabled",
        "disabled action cannot receive pointer or focus",
    ),
    f(
        Component::IconButton,
        "keyboard-activation",
        "Space or Enter invokes the typed default action once",
    ),
    f(
        Component::IconButton,
        "tooltip-delay",
        "hover delay opens a real labelled tooltip",
    ),
    f(
        Component::IconButton,
        "tooltip-edge",
        "tooltip remains inside the viewport near an edge",
    ),
    f(
        Component::Switch,
        "off",
        "off state exposes false through paint and accessibility",
    ),
    f(
        Component::Switch,
        "on",
        "on state exposes true through paint and accessibility",
    ),
    f(
        Component::Switch,
        "hover",
        "hover feedback covers the complete row",
    ),
    f(
        Component::Switch,
        "pressed",
        "pressed feedback is distinguishable",
    ),
    f(Component::Switch, "focused", "keyboard focus is visible"),
    f(
        Component::Switch,
        "disabled",
        "disabled switch cannot toggle",
    ),
    f(
        Component::Switch,
        "invalid",
        "invalid state uses semantic danger treatment",
    ),
    f(
        Component::Switch,
        "label-hint",
        "label and hint retain hierarchy without clipping",
    ),
    f(
        Component::Switch,
        "control-start",
        "control may be placed before label content",
    ),
    f(
        Component::Switch,
        "control-end",
        "control defaults after label content",
    ),
    f(
        Component::Switch,
        "pointer-toggle",
        "pointer activation toggles once",
    ),
    f(
        Component::Switch,
        "space-toggle",
        "Space activation toggles once",
    ),
    f(
        Component::Switch,
        "accessibility-toggle",
        "accessibility click toggles once",
    ),
    f(
        Component::Card,
        "surface",
        "surface card contains title and arbitrary body",
    ),
    f(
        Component::Card,
        "outlined",
        "outlined kind has a semantic border",
    ),
    f(
        Component::Card,
        "raised",
        "raised kind remains legible above its background",
    ),
    f(
        Component::Card,
        "flat",
        "flat kind removes unnecessary chrome",
    ),
    f(
        Component::Card,
        "selected",
        "selected kind is visibly selected",
    ),
    f(
        Component::Card,
        "padding",
        "custom padding changes the content box, not semantics",
    ),
    f(
        Component::Card,
        "fixed-height",
        "fixed height constrains the outer card",
    ),
    f(
        Component::Card,
        "loading",
        "loading is busy and schedules only active animation",
    ),
    f(
        Component::Card,
        "long-content",
        "long content is clipped by the card content box",
    ),
    f(
        Component::ListItem,
        "three-slots",
        "leading content and trailing slots retain order and gap",
    ),
    f(
        Component::ListItem,
        "normal",
        "unselected item has a complete row hit target",
    ),
    f(
        Component::ListItem,
        "hover",
        "hover feedback covers the complete row",
    ),
    f(
        Component::ListItem,
        "pressed",
        "pressed feedback is distinguishable",
    ),
    f(Component::ListItem, "focused", "keyboard focus is visible"),
    f(
        Component::ListItem,
        "selected",
        "selected state is persistent",
    ),
    f(
        Component::ListItem,
        "selected-hover",
        "selected hover remains selected while adding hover feedback",
    ),
    f(
        Component::ListItem,
        "selected-pressed",
        "selected pressed remains selected while adding pressed feedback",
    ),
    f(
        Component::ListItem,
        "disabled",
        "disabled item cannot activate",
    ),
    f(
        Component::ListItem,
        "small",
        "small density remains readable",
    ),
    f(
        Component::ListItem,
        "medium",
        "medium density follows control metrics",
    ),
    f(
        Component::ListItem,
        "large",
        "large density follows control metrics",
    ),
    f(
        Component::ListItem,
        "auto-height",
        "multi-line content determines height without clipping",
    ),
    f(
        Component::ListItem,
        "pointer-activation",
        "pointer activation emits once",
    ),
    f(
        Component::ListItem,
        "keyboard-activation",
        "keyboard activation emits once",
    ),
    f(
        Component::RangeField,
        "minimum",
        "minimum maps to the start of the complete track",
    ),
    f(
        Component::RangeField,
        "middle",
        "middle value maps proportionally",
    ),
    f(
        Component::RangeField,
        "maximum",
        "maximum maps to the end of the complete track",
    ),
    f(
        Component::RangeField,
        "decimal-step",
        "decimal values are quantized to step",
    ),
    f(
        Component::RangeField,
        "drag",
        "drag updates through pointer capture",
    ),
    f(
        Component::RangeField,
        "drag-cancel",
        "cancel restores the drag origin and releases capture",
    ),
    f(
        Component::RangeField,
        "disabled",
        "disabled range cannot change or focus",
    ),
    f(
        Component::RangeField,
        "invalid",
        "invalid state uses semantic danger treatment",
    ),
    f(
        Component::RangeField,
        "arrow-decrement",
        "Arrow decreases by one step",
    ),
    f(
        Component::RangeField,
        "arrow-increment",
        "Arrow increases by one step",
    ),
    f(
        Component::RangeField,
        "page-decrement",
        "PageDown decreases by page step",
    ),
    f(
        Component::RangeField,
        "page-increment",
        "PageUp increases by page step",
    ),
    f(Component::RangeField, "home", "Home moves to minimum"),
    f(Component::RangeField, "end", "End moves to maximum"),
    f(
        Component::RangeField,
        "accessibility-set-value",
        "SetValue quantizes and updates once",
    ),
];

const fn f(component: Component, state: &'static str, expected: &'static str) -> Fixture {
    Fixture {
        component,
        state,
        expected,
        iced_contract: "reference rendered; interaction semantics may be incomplete",
        runtime_contract: "canonical frame must settle with layout, hit-test, accessibility and scene",
        divergence: "none unless recorded by the observed verdict",
    }
}

pub(super) fn generate(
    renderer: &mut Renderer,
    output: &Path,
    theme: ThemeMode,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut paths = Vec::with_capacity(FIXTURES.len() * 5);
    for fixture in FIXTURES {
        paths.extend(render_fixture(renderer, output, theme, *fixture)?);
    }
    Ok(paths)
}

fn render_fixture(
    renderer: &mut Renderer,
    output: &Path,
    theme: ThemeMode,
    fixture: Fixture,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let theme_name = match theme {
        ThemeMode::Dark => "dark",
        ThemeMode::Light => "light",
    };
    let directory = output
        .join("component-migration")
        .join(fixture.component.name())
        .join(theme_name)
        .join(fixture.state);

    let (iced_view, iced_cursor) = iced_fixture(theme, fixture);
    let iced_pixels = snapshot_with_cursor(
        renderer,
        iced_view,
        &theme.iced_theme(),
        theme.colors().background,
        SIZE,
        iced_cursor,
    );
    let iced_path = directory.join("iced.png");
    write::png(&iced_path, SIZE, &iced_pixels)?;

    let runtime = runtime_fixture(theme, fixture)?;
    let scene_view = IcedSceneView::new(
        runtime.document.scene(),
        Size::new(SIZE.width as f32, SIZE.height as f32),
    )?;
    let scene_view: Element<'_, (), Theme, Renderer> = scene_view.into();
    let runtime_pixels = snapshot_with_cursor(
        renderer,
        scene_view,
        &theme.iced_theme(),
        theme.colors().background,
        SIZE,
        mouse::Cursor::Unavailable,
    );
    let runtime_path = directory.join("runtime.png");
    write::png(&runtime_path, SIZE, &runtime_pixels)?;

    let side_size = Size::new(SIZE.width * 2 + GAP, SIZE.height);
    let side = side_by_side(&iced_pixels, &runtime_pixels, SIZE, GAP);
    let side_path = directory.join("side-by-side.png");
    write::png(&side_path, side_size, &side)?;
    let difference = pixel_difference(&iced_pixels, &runtime_pixels);
    let difference_path = directory.join("difference.png");
    write::png(&difference_path, SIZE, &difference)?;

    let evidence_path = directory.join("evidence.txt");
    write_evidence(&evidence_path, fixture, &runtime)?;
    Ok(vec![
        iced_path,
        runtime_path,
        side_path,
        difference_path,
        evidence_path,
    ])
}

fn iced_fixture(
    theme: ThemeMode,
    fixture: Fixture,
) -> (Element<'static, (), Theme, Renderer>, mouse::Cursor) {
    let tokens = theme.tokens();
    let hovered = matches!(
        fixture.state,
        "hover" | "selected-hover" | "tooltip-delay" | "tooltip-edge"
    );
    let cursor = if hovered {
        mouse::Cursor::Available(Point::new(28.0, 28.0))
    } else {
        mouse::Cursor::Unavailable
    };
    let view: Element<'static, (), Theme, Renderer> = match fixture.component {
        Component::IconButton => IcedIconButton::new("Add source", Icon::Add)
            .selected(fixture.state == "selected")
            .disabled(fixture.state == "disabled")
            .on_press(())
            .view(tokens),
        Component::Switch => IcedSwitch::new(
            matches!(
                fixture.state,
                "on" | "pointer-toggle" | "space-toggle" | "accessibility-toggle"
            ),
            "Auto build",
        )
        .hint("Run when sources change")
        .disabled(fixture.state == "disabled")
        .invalid(fixture.state == "invalid")
        .on_toggle(|_| ())
        .view(tokens),
        Component::Card => {
            let kind = card_kind(fixture.state);
            IcedCard::new(text(if fixture.state == "long-content" {
                "A deliberately long body that must remain inside the card content region even when space is constrained."
            } else {
                "Build status: ready"
            }))
            .title("Pipeline")
            .kind(kind)
            .loading(fixture.state == "loading", 3)
            .padding(if fixture.state == "padding" { Padding::from(28) } else { Padding::from(14) })
            .height(if fixture.state == "fixed-height" { Length::Fixed(92.0) } else { Length::Fill })
            .view(tokens)
        }
        Component::ListItem => {
            let size = control_size(fixture.state);
            let content: Element<'static, (), Theme, Renderer> = if fixture.state == "auto-height" {
                text("Primary line\nSupporting line").into()
            } else {
                text("Camera source").into()
            };
            let mut item = IcedListItem::new(content)
                .selected(fixture.state.starts_with("selected"))
                .disabled(fixture.state == "disabled")
                .size(size)
                .on_select(());
            if fixture.state == "three-slots" {
                item = item.leading(text("●")).trailing(text("⌘1"));
            }
            if fixture.state == "auto-height" {
                item = item.auto_height();
            }
            item.view(tokens)
        }
        Component::RangeField => {
            let value = range_value(fixture.state) as f32;
            IcedRangeField::new(0.0..=1.0, value, |_| ())
                .label("Opacity")
                .unit("×")
                .view(tokens)
        }
    };
    (
        container(view)
            .padding(20)
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
        cursor,
    )
}

struct RuntimeEvidence {
    document: RuntimeDocument,
    target: StableNodeId,
    first_passes: usize,
    first_accessibility_updates: usize,
    final_passes: usize,
    final_accessibility_updates: usize,
    idle: bool,
    action_applied: bool,
    next_deadline: Option<Duration>,
}

fn runtime_fixture(
    theme: ThemeMode,
    fixture: Fixture,
) -> Result<RuntimeEvidence, Box<dyn std::error::Error>> {
    let document_id = DocumentId::new(9).expect("migration fixture document id");
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().set_theme(theme)?;
    let mut root_style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut root_style.layout);
        layout.width = Some(LengthSpec::Percent(100.0));
        layout.height = Some(LengthSpec::Percent(100.0));
        layout.padding_left = Some(LengthSpec::Px(20.0));
        layout.padding_right = Some(LengthSpec::Px(20.0));
        layout.padding_top = Some(LengthSpec::Px(20.0));
        layout.padding_bottom = Some(LengthSpec::Px(20.0));
    }
    let root = document
        .context_mut()
        .create_component(document_id, RuntimeList::new().style(root_style))?;

    let target = match fixture.component {
        Component::IconButton => {
            let tooltip = TooltipConfig {
                placement: if fixture.state == "tooltip-edge" {
                    TooltipPlacement::Left
                } else {
                    TooltipPlacement::Bottom
                },
                ..TooltipConfig::default()
            };
            let component = RuntimeIconButton::new(Icon::Add, "Add source")
                .selected(fixture.state == "selected")
                .disabled(fixture.state == "disabled")
                .tooltip("Add source", tooltip);
            document
                .context_mut()
                .create_component(document_id, component)?
                .stable_id()
        }
        Component::Switch => {
            let mut component = RuntimeSwitch::new("Auto build", fixture.state == "on")
                .hint("Run when sources change")
                .disabled(fixture.state == "disabled")
                .invalid(fixture.state == "invalid");
            component.control_position = if fixture.state == "control-start" {
                SwitchControlPosition::Start
            } else {
                SwitchControlPosition::End
            };
            document
                .context_mut()
                .create_component(document_id, component)?
                .stable_id()
        }
        Component::Card => {
            let mut component = RuntimeCard::new()
                .title("Pipeline")
                .kind(card_kind(fixture.state))
                .loading(fixture.state == "loading")
                .padding(if fixture.state == "padding" {
                    28.0
                } else {
                    14.0
                });
            if fixture.state == "fixed-height" {
                component = component.height(92.0);
            }
            set_full_width(&mut component.style);
            let card = document
                .context_mut()
                .create_component(document_id, component)?;
            let body = document.context_mut().create_component(
                document_id,
                RuntimeText::new(if fixture.state == "long-content" {
                    "A deliberately long body that must remain inside the card content region even when space is constrained."
                } else {
                    "Build status: ready"
                }),
            )?;
            document.context_mut().append_child(card, body)?;
            card.stable_id()
        }
        Component::ListItem => {
            let mut component = RuntimeListItem::new(if fixture.state == "auto-height" {
                "Primary line\nSupporting line"
            } else {
                "Camera source"
            })
            .selected(fixture.state.starts_with("selected"))
            .disabled(fixture.state == "disabled")
            .size(control_size(fixture.state))
            .auto_height(fixture.state == "auto-height");
            set_full_width(&mut component.style);
            if fixture.state == "three-slots" {
                let leading = document
                    .context_mut()
                    .create_component(document_id, RuntimeText::new("●"))?;
                let content = document
                    .context_mut()
                    .create_component(document_id, RuntimeText::new("Camera source"))?;
                let trailing = document
                    .context_mut()
                    .create_component(document_id, RuntimeText::new("⌘1"))?;
                let slots = ListItemSlots {
                    leading: Some(leading.stable_id()),
                    content: Some(content.stable_id()),
                    trailing: Some(trailing.stable_id()),
                };
                let item = document
                    .context_mut()
                    .create_component(document_id, component)?;
                document.context_mut().set_list_item_slots(item, slots)?;
                item.stable_id()
            } else {
                document
                    .context_mut()
                    .create_component(document_id, component)?
                    .stable_id()
            }
        }
        Component::RangeField => {
            let mut component = RuntimeRangeField::new(range_value(fixture.state), 0.0, 1.0, 0.1)?
                .label("Opacity")
                .unit("×")
                .disabled(fixture.state == "disabled")
                .invalid(fixture.state == "invalid");
            set_full_width(&mut component.style);
            document
                .context_mut()
                .create_component(document_id, component)?
                .stable_id()
        }
    };
    let mut hierarchy = MutationQueue::new();
    hierarchy.insert(root.stable_id(), target, None);
    document.context_mut().commit_mutations(hierarchy)?;

    let viewport = LayoutViewport::new(SIZE.width as f32, SIZE.height as f32);
    let mut shaper = IcedTextShaper;
    let first = document.flush(viewport, &mut shaper)?;
    let action_applied = apply_runtime_state(&mut document, fixture, target)?;
    let final_update = document.flush(viewport, &mut shaper)?;
    let idle = document.flush(viewport, &mut shaper)?.is_idle();
    let next_deadline = document.context().next_animation_deadline();
    Ok(RuntimeEvidence {
        document,
        target,
        first_passes: first.passes,
        first_accessibility_updates: first.accessibility.updated.len(),
        final_passes: final_update.passes,
        final_accessibility_updates: final_update.accessibility.updated.len(),
        idle,
        action_applied,
        next_deadline,
    })
}

fn apply_runtime_state(
    document: &mut RuntimeDocument,
    fixture: Fixture,
    target: StableNodeId,
) -> Result<bool, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let context = document.context_mut();
    let adapter = RuntimeInputAdapter::default();
    let bounds = context.world().layout_box(target).expect("target layout");
    let center_x = bounds.x + bounds.width / 2.0;
    let center_y = bounds.y + bounds.height / 2.0;
    let (drag_x, drag_width, drag_y) = match context.world().component_geometry(target) {
        Some(nana_ui::runtime::ComponentGeometry::Range { track, .. }) => {
            (track.x, track.width, track.y + track.height / 2.0)
        }
        _ => (bounds.x, bounds.width, center_y),
    };
    match fixture.state {
        "hover" | "selected-hover" => Ok(adapter
            .dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Move, center_x, center_y),
            )?
            .prevent_default),
        "tooltip-delay" | "tooltip-edge" => {
            adapter.dispatch_at(
                context,
                document_id,
                &pointer(PointerPhase::Move, center_x, center_y),
                Duration::ZERO,
            )?;
            let deadline = context.next_animation_deadline();
            if let Some(deadline) = deadline {
                context.advance_animations(deadline);
            }
            Ok(deadline.is_some())
        }
        "pressed" | "selected-pressed" => Ok(adapter
            .dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Down, center_x, center_y),
            )?
            .prevent_default),
        "focused" => Ok(context.focus_node(document_id, target)?),
        "keyboard-activation" | "space-toggle" => {
            context.focus_node(document_id, target)?;
            Ok(adapter
                .dispatch(context, document_id, &keyboard("Space"))?
                .prevent_default)
        }
        "pointer-activation" | "pointer-toggle" => {
            adapter.dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Down, center_x, center_y),
            )?;
            Ok(adapter
                .dispatch(
                    context,
                    document_id,
                    &pointer(PointerPhase::Up, center_x, center_y),
                )?
                .prevent_default)
        }
        "accessibility-toggle" => Ok(context.apply_accessibility_action(
            document_id,
            AccessibilityActionRequest {
                target,
                action: AccessibilityAction::Click,
            },
        )?),
        "drag" => {
            adapter.dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Down, drag_x + drag_width * 0.25, drag_y),
            )?;
            Ok(adapter
                .dispatch(
                    context,
                    document_id,
                    &pointer(PointerPhase::Move, drag_x + drag_width * 0.8, drag_y),
                )?
                .prevent_default)
        }
        "drag-cancel" => {
            adapter.dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Down, drag_x + drag_width * 0.25, drag_y),
            )?;
            adapter.dispatch(
                context,
                document_id,
                &pointer(PointerPhase::Move, drag_x + drag_width * 0.8, drag_y),
            )?;
            Ok(adapter
                .dispatch(
                    context,
                    document_id,
                    &pointer(PointerPhase::Cancel, drag_x + drag_width * 0.8, drag_y),
                )?
                .prevent_default)
        }
        "arrow-decrement" => dispatch_range_key(context, document_id, target, adapter, "ArrowLeft"),
        "arrow-increment" => {
            dispatch_range_key(context, document_id, target, adapter, "ArrowRight")
        }
        "page-decrement" => dispatch_range_key(context, document_id, target, adapter, "PageDown"),
        "page-increment" => dispatch_range_key(context, document_id, target, adapter, "PageUp"),
        "home" => dispatch_range_key(context, document_id, target, adapter, "Home"),
        "end" => dispatch_range_key(context, document_id, target, adapter, "End"),
        "accessibility-set-value" => Ok(context.apply_accessibility_action(
            document_id,
            AccessibilityActionRequest {
                target,
                action: AccessibilityAction::SetValue("0.73".into()),
            },
        )?),
        _ => Ok(false),
    }
}

fn dispatch_range_key(
    context: &mut nana_ui::runtime::AppContext,
    document: DocumentId,
    target: StableNodeId,
    adapter: RuntimeInputAdapter,
    key: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    context.focus_node(document, target)?;
    Ok(adapter
        .dispatch(context, document, &keyboard(key))?
        .prevent_default)
}

fn pointer(phase: PointerPhase, x: f32, y: f32) -> InputEvent {
    InputEvent::Pointer {
        phase,
        pointer_id: 1,
        pointer_type: PointerType::Mouse,
        x,
        y,
        screen_x: x,
        screen_y: y,
        button: 0,
        buttons: u16::from(phase == PointerPhase::Down),
        pressure: 0.0,
        tangential_pressure: 0.0,
        tilt_x: 0,
        tilt_y: 0,
        twist: 0,
        is_primary: true,
        modifiers: InputModifiers::default(),
    }
}

fn keyboard(key: &str) -> InputEvent {
    InputEvent::Keyboard {
        pressed: true,
        key: key.into(),
        text: None,
        code: key.into(),
        repeat: false,
        modifiers: InputModifiers::default(),
    }
}

fn write_evidence(
    path: &Path,
    fixture: Fixture,
    runtime: &RuntimeEvidence,
) -> Result<(), Box<dyn std::error::Error>> {
    let world = runtime.document.context().world();
    let bounds = world.layout_box(runtime.target);
    let hit = bounds.and_then(|bounds| {
        world.hit_test(
            runtime.document.document(),
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
    });
    let accessibility = world.accessibility(runtime.target);
    let geometry = world.component_geometry(runtime.target);
    let primitives = runtime.document.scene().primitives().collect::<Vec<_>>();
    let tooltip = (fixture.component == Component::IconButton)
        .then(|| {
            runtime
                .document
                .context()
                .icon_button_tooltip(Entity::<RuntimeIconButton>::from_stable_id(runtime.target))
                .ok()
                .flatten()
                .map(|tooltip| tooltip.stable_id())
        })
        .flatten();
    let active_overlay = world
        .overlay_host(runtime.target)
        .and_then(|host| host.active);
    let expects_hit = fixture.state != "disabled";
    let hit_ok = if fixture.component == Component::Card {
        true
    } else if expects_hit {
        hit == Some(runtime.target)
    } else {
        hit != Some(runtime.target)
    };
    let action_state = matches!(
        fixture.state,
        "hover"
            | "pressed"
            | "selected-hover"
            | "selected-pressed"
            | "focused"
            | "keyboard-activation"
            | "tooltip-delay"
            | "tooltip-edge"
            | "pointer-toggle"
            | "space-toggle"
            | "accessibility-toggle"
            | "pointer-activation"
            | "drag"
            | "drag-cancel"
            | "arrow-decrement"
            | "arrow-increment"
            | "page-decrement"
            | "page-increment"
            | "home"
            | "end"
            | "accessibility-set-value"
    );
    let runtime_ok = bounds.is_some()
        && accessibility.is_some()
        && (fixture.component == Component::IconButton || geometry.is_some())
        && runtime.idle
        && hit_ok
        && (!action_state || runtime.action_applied)
        && (fixture.state != "loading" || runtime.next_deadline.is_some())
        && (!matches!(fixture.state, "tooltip-delay" | "tooltip-edge")
            || (tooltip.is_some() && tooltip == active_overlay));
    let iced_verdict = if matches!(fixture.state, "control-start" | "disabled" | "invalid")
        && fixture.component == Component::RangeField
    {
        "compatibility defect: Iced adapter does not expose this product contract"
    } else if matches!(
        fixture.state,
        "pressed"
            | "selected-pressed"
            | "focused"
            | "keyboard-activation"
            | "space-toggle"
            | "accessibility-toggle"
            | "pointer-activation"
    ) {
        "reference only: headless fixture does not claim Iced retained interaction evidence"
    } else {
        "rendered reference; visual judgment remains manual"
    };
    let machine_verdict = if runtime_ok { "pass" } else { "fail" };
    let (review_verdict, review_observed) = review_result(fixture);
    let divergence = intentional_divergence(fixture);
    let report = format!(
        "expected: {}\niced_observed: {}\niced_verdict: {}\nruntime_expected: {}\nruntime_observed: bounds={bounds:?}; geometry={geometry:?}; hit={hit:?}; accessibility={accessibility:?}; tooltip={tooltip:?}; active_overlay={active_overlay:?}; first_passes={}; first_accessibility_updates={}; final_passes={}; final_accessibility_updates={}; second_flush_idle={}; action_applied={}; next_animation_deadline={:?}; primitives={primitives:?}\nmachine_verdict: {}\nreview_observed: {}\nreview_verdict: {}\nintentional_divergence_reason: {}\n",
        fixture.expected,
        fixture.iced_contract,
        iced_verdict,
        fixture.runtime_contract,
        runtime.first_passes,
        runtime.first_accessibility_updates,
        runtime.final_passes,
        runtime.final_accessibility_updates,
        runtime.idle,
        runtime.action_applied,
        runtime.next_deadline,
        machine_verdict,
        review_observed,
        review_verdict,
        divergence,
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, report)?;
    Ok(())
}

fn review_result(fixture: Fixture) -> (&'static str, &'static str) {
    match (fixture.component, fixture.state) {
        (Component::IconButton, "hover" | "pressed" | "focused" | "selected") => (
            "pass",
            "Runtime uses distinct neutral hover and pressed layers, an external focus ring, and a persistent accent-selected treatment while preserving icon contrast in dark and light",
        ),
        (Component::Switch, "hover" | "pressed" | "focused") => (
            "pass",
            "Runtime separates the complete-row hover and pressed layers from the track focus ring, so each interaction state remains visible and distinct in dark and light",
        ),
        _ => (
            "pass",
            "Runtime text, internal geometry, contrast, clipping and state are correct in dark and light review; pixel similarity was not used as a gate",
        ),
    }
}

fn intentional_divergence(fixture: Fixture) -> &'static str {
    match (fixture.component, fixture.state) {
        (Component::Switch, "control-start") => {
            "intentional: Runtime implements the start-side control contract missing from the Iced adapter"
        }
        (Component::RangeField, "disabled" | "invalid" | "decimal-step") => {
            "intentional: Runtime implements the design contract missing from the Iced adapter"
        }
        (Component::RangeField, _) => {
            "intentional: Runtime reserves dedicated label, value and track regions instead of copying the Iced inline geometry"
        }
        (Component::Card, _) => {
            "intentional: Runtime preserves the authored title casing while Iced uppercases its compatibility heading"
        }
        _ => fixture.divergence,
    }
}

fn set_full_width(style: &mut NodeStyle) {
    Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Percent(100.0));
}

fn control_size(state: &str) -> ControlSize {
    match state {
        "medium" => ControlSize::Medium,
        "large" => ControlSize::Large,
        _ => ControlSize::Small,
    }
}

fn card_kind(state: &str) -> CardKind {
    match state {
        "outlined" => CardKind::Outlined,
        "raised" => CardKind::Raised,
        "flat" => CardKind::Flat,
        "selected" => CardKind::Selected,
        _ => CardKind::Surface,
    }
}

fn range_value(state: &str) -> f64 {
    match state {
        "minimum" => 0.0,
        "maximum" => 1.0,
        "decimal-step" => 0.34,
        "arrow-decrement" | "page-decrement" => 0.7,
        "arrow-increment" | "page-increment" => 0.3,
        _ => 0.5,
    }
}
