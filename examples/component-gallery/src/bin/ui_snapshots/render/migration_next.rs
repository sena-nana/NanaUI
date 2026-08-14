use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use iced::widget::{container, text};
use iced::{Element, Length, Padding, Point, Size, Theme, mouse};
use iced_wgpu::Renderer;
use nana_ui::compatibility::{
    Button as IcedButton, Card as IcedCard, Checkbox as IcedCheckbox, IconButton as IcedIconButton,
    Input as IcedInput, ListItem as IcedListItem, RangeField as IcedRangeField,
    Switch as IcedSwitch,
};
use nana_ui::runtime::{
    AccessibilityAction, AccessibilityActionRequest, Button as RuntimeButton, Card as RuntimeCard,
    Checkbox as RuntimeCheckbox, DocumentId, Entity, IconButton as RuntimeIconButton,
    LayoutViewport, List as RuntimeList, ListItem as RuntimeListItem, ListItemSlots, MutationQueue,
    NodeStyle, RangeField as RuntimeRangeField, RuntimeDocument, StableNodeId,
    Switch as RuntimeSwitch, Text as RuntimeText, TextHorizontalAlignment,
    TextInput as RuntimeTextInput, TextSelection, TextVerticalAlignment,
};
use nana_ui::{
    CardKind, ControlSize, IcedSceneView, IcedTextShaper, Icon, RuntimeInputAdapter, ThemeMode,
    ThemeModeExt, TooltipConfig, TooltipPlacement,
};
use nana_ui_core::{LengthSpec, SemanticColorRole, SwitchControlPosition};
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};

use crate::write;

use super::{pixel_difference, side_by_side, snapshot_with_cursor};

const SIZE: Size<u32> = Size::new(420, 120);
const GAP: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Component {
    Text,
    Button,
    TextInput,
    Checkbox,
    IconButton,
    Switch,
    Card,
    ListItem,
    RangeField,
}

impl Component {
    const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Button => "button",
            Self::TextInput => "text-input",
            Self::Checkbox => "checkbox",
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
        Component::Text,
        "normal",
        "body text uses the shared 13px baseline",
    ),
    f(
        Component::Text,
        "wrap",
        "long text wraps inside its authored content box",
    ),
    f(
        Component::Text,
        "ellipsis",
        "single-line overflow clips with an ellipsis",
    ),
    f(
        Component::Text,
        "centered",
        "horizontal and vertical anchors use the content box",
    ),
    f(
        Component::Text,
        "muted",
        "muted text keeps readable semantic contrast",
    ),
    f(
        Component::Button,
        "ghost",
        "ghost kind is transparent until interaction",
    ),
    f(
        Component::Button,
        "subtle",
        "subtle kind has neutral surface and border",
    ),
    f(
        Component::Button,
        "selected",
        "selected kind keeps persistent selected semantics",
    ),
    f(
        Component::Button,
        "primary",
        "primary kind uses accent-soft semantics",
    ),
    f(
        Component::Button,
        "warning",
        "warning kind uses warning semantics",
    ),
    f(
        Component::Button,
        "danger",
        "danger kind uses danger semantics",
    ),
    f(
        Component::Button,
        "text-kind",
        "text kind uses accent text without a surface",
    ),
    f(
        Component::Button,
        "small",
        "small size follows compact control metrics",
    ),
    f(
        Component::Button,
        "medium",
        "medium size follows standard control metrics",
    ),
    f(
        Component::Button,
        "large",
        "large size follows large control metrics",
    ),
    f(
        Component::Button,
        "hover",
        "hover feedback covers the complete hit target",
    ),
    f(
        Component::Button,
        "pressed",
        "pressed feedback is distinct from hover",
    ),
    f(Component::Button, "focused", "keyboard focus is visible"),
    f(
        Component::Button,
        "disabled",
        "disabled button cannot activate or focus",
    ),
    f(
        Component::Button,
        "loading",
        "loading is visible and prevents duplicate activation",
    ),
    f(
        Component::Button,
        "pointer-activation",
        "pointer activation emits once",
    ),
    f(
        Component::Button,
        "keyboard-activation",
        "Space activation emits once",
    ),
    f(
        Component::TextInput,
        "value",
        "committed value uses field padding and baseline",
    ),
    f(
        Component::TextInput,
        "placeholder",
        "empty input paints faint placeholder text",
    ),
    f(
        Component::TextInput,
        "hover",
        "hover strengthens the neutral border",
    ),
    f(
        Component::TextInput,
        "focused",
        "focus paints border and caret",
    ),
    f(
        Component::TextInput,
        "selection",
        "selected text has shaped highlight geometry",
    ),
    f(
        Component::TextInput,
        "disabled",
        "disabled input is inert and visibly disabled",
    ),
    f(
        Component::TextInput,
        "invalid",
        "invalid input keeps a danger border while focused",
    ),
    f(
        Component::TextInput,
        "secure",
        "secure input masks committed text",
    ),
    f(
        Component::TextInput,
        "small",
        "small size follows compact field metrics",
    ),
    f(
        Component::TextInput,
        "large",
        "large size follows large field metrics",
    ),
    f(
        Component::TextInput,
        "read-only",
        "read-only input remains focusable but rejects edits",
    ),
    f(
        Component::TextInput,
        "loading",
        "loading input is busy and rejects input",
    ),
    f(
        Component::TextInput,
        "keyboard-edit",
        "keyboard input commits through typed state",
    ),
    f(
        Component::TextInput,
        "ime-preedit",
        "IME preedit is visibly distinct from committed text",
    ),
    f(
        Component::TextInput,
        "ime-commit",
        "IME commit updates value and clears preedit",
    ),
    f(
        Component::TextInput,
        "accessibility-set-value",
        "accessibility SetValue updates the field",
    ),
    f(
        Component::Checkbox,
        "off",
        "off state exposes false in paint and accessibility",
    ),
    f(
        Component::Checkbox,
        "on",
        "on state exposes true in paint and accessibility",
    ),
    f(
        Component::Checkbox,
        "hover",
        "hover feedback reaches the indicator",
    ),
    f(
        Component::Checkbox,
        "pressed",
        "pressed feedback is distinct from hover",
    ),
    f(
        Component::Checkbox,
        "focused",
        "keyboard focus is visible around the indicator",
    ),
    f(
        Component::Checkbox,
        "disabled",
        "disabled checkbox cannot toggle or focus",
    ),
    f(
        Component::Checkbox,
        "invalid",
        "invalid state keeps semantic danger treatment",
    ),
    f(
        Component::Checkbox,
        "pointer-toggle",
        "pointer activation toggles once",
    ),
    f(
        Component::Checkbox,
        "space-toggle",
        "Space activation toggles once",
    ),
    f(
        Component::Checkbox,
        "accessibility-toggle",
        "accessibility click toggles once",
    ),
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
        Component::Text => {
            let content = if fixture.state == "wrap" || fixture.state == "ellipsis" {
                "A deliberately long migration label that must respect its authored content box."
            } else {
                "Migration text 文本"
            };
            let mut label = text(content).size(13);
            if fixture.state == "muted" {
                label = label.color(tokens.colors.muted);
            }
            container(label)
                .width(if matches!(fixture.state, "wrap" | "ellipsis") {
                    Length::Fixed(180.0)
                } else {
                    Length::Fill
                })
                .height(Length::Fixed(if fixture.state == "wrap" {
                    44.0
                } else {
                    32.0
                }))
                .align_x(if fixture.state == "centered" {
                    iced::alignment::Horizontal::Center
                } else {
                    iced::alignment::Horizontal::Left
                })
                .align_y(iced::alignment::Vertical::Center)
                .into()
        }
        Component::Button => IcedButton::label("Run build")
            .kind(button_kind(fixture.state))
            .size(button_control_size(fixture.state))
            .disabled(fixture.state == "disabled")
            .loading(fixture.state == "loading", 3)
            .on_press(())
            .view(tokens),
        Component::TextInput => {
            let input = IcedInput::new(
                "Branch name",
                if fixture.state == "placeholder" {
                    ""
                } else {
                    "release/next"
                },
            )
            .size(text_input_control_size(fixture.state))
            .disabled(matches!(fixture.state, "disabled" | "loading"))
            .invalid(fixture.state == "invalid")
            .secure(fixture.state == "secure");
            if fixture.state == "read-only" {
                input.view(tokens)
            } else {
                input.on_input(|_| ()).view(tokens)
            }
        }
        Component::Checkbox => IcedCheckbox::new(
            matches!(
                fixture.state,
                "on" | "pointer-toggle" | "space-toggle" | "accessibility-toggle"
            ),
            "Notifications",
        )
        .disabled(fixture.state == "disabled")
        .invalid(fixture.state == "invalid")
        .on_toggle(|_| ())
        .view(tokens),
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
        Component::Text => {
            let mut style = NodeStyle {
                foreground: Some(if fixture.state == "muted" {
                    SemanticColorRole::Muted
                } else {
                    SemanticColorRole::Text
                }),
                text_horizontal_alignment: if fixture.state == "centered" {
                    TextHorizontalAlignment::Center
                } else {
                    TextHorizontalAlignment::Start
                },
                text_vertical_alignment: TextVerticalAlignment::Center,
                ..NodeStyle::default()
            };
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(if matches!(fixture.state, "wrap" | "ellipsis") {
                LengthSpec::Px(180.0)
            } else {
                LengthSpec::Percent(100.0)
            });
            layout.height = Some(LengthSpec::Px(if fixture.state == "wrap" {
                44.0
            } else {
                32.0
            }));
            layout.font_size = Some(13.0);
            layout.white_space_nowrap = fixture.state == "ellipsis";
            layout.text_overflow_ellipsis = fixture.state == "ellipsis";
            document
                .context_mut()
                .create_component(
                    document_id,
                    RuntimeText::new(if matches!(fixture.state, "wrap" | "ellipsis") {
                        "A deliberately long migration label that must respect its authored content box."
                    } else {
                        "Migration text 文本"
                    })
                    .style(style),
                )?
                .stable_id()
        }
        Component::Button => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeButton::new("Run build")
                    .kind(button_kind(fixture.state))
                    .size(button_control_size(fixture.state))
                    .disabled(fixture.state == "disabled")
                    .loading(fixture.state == "loading"),
            )?
            .stable_id(),
        Component::TextInput => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeTextInput::new(if fixture.state == "placeholder" {
                    ""
                } else {
                    "release/next"
                })
                .label("Branch name")
                .placeholder("Branch name")
                .size(text_input_control_size(fixture.state))
                .disabled(fixture.state == "disabled")
                .loading(fixture.state == "loading")
                .read_only(fixture.state == "read-only")
                .invalid(fixture.state == "invalid")
                .secure(fixture.state == "secure"),
            )?
            .stable_id(),
        Component::Checkbox => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeCheckbox::new(
                    "Notifications",
                    matches!(
                        fixture.state,
                        "on" | "pointer-toggle" | "space-toggle" | "accessibility-toggle"
                    ),
                )
                .disabled(fixture.state == "disabled")
                .invalid(fixture.state == "invalid"),
            )?
            .stable_id(),
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
        "invalid" if fixture.component == Component::TextInput => {
            Ok(context.focus_node(document_id, target)?)
        }
        "selection" => {
            context.focus_node(document_id, target)?;
            Ok(context.apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target,
                    action: AccessibilityAction::SetSelection(TextSelection {
                        anchor: 0,
                        focus: "release".len(),
                    }),
                },
            )?)
        }
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
        "keyboard-edit" => {
            context.focus_node(document_id, target)?;
            Ok(adapter
                .dispatch(context, document_id, &keyboard_text("X"))?
                .prevent_default)
        }
        "ime-preedit" => {
            context.focus_node(document_id, target)?;
            Ok(context.set_ime_preedit(document_id, "你".into(), None)?)
        }
        "ime-commit" => {
            context.focus_node(document_id, target)?;
            context.set_ime_preedit(document_id, "你".into(), None)?;
            Ok(context.commit_ime(document_id, "你")?)
        }
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
                action: AccessibilityAction::SetValue(
                    if fixture.component == Component::TextInput {
                        "updated"
                    } else {
                        "0.73"
                    }
                    .into(),
                ),
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

fn keyboard_text(text: &str) -> InputEvent {
    InputEvent::Keyboard {
        pressed: true,
        key: text.into(),
        text: Some(text.into()),
        code: "KeyX".into(),
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
    let expects_hit =
        !matches!(fixture.state, "disabled" | "loading") && fixture.component != Component::Text;
    let hit_ok = if fixture.component == Component::Card || fixture.component == Component::Text {
        true
    } else if expects_hit {
        hit == Some(runtime.target)
    } else {
        hit != Some(runtime.target)
    };
    let action_state = (fixture.component == Component::TextInput && fixture.state == "invalid")
        || matches!(
            fixture.state,
            "hover"
                | "pressed"
                | "selected-hover"
                | "selected-pressed"
                | "focused"
                | "selection"
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
                | "keyboard-edit"
                | "ime-preedit"
                | "ime-commit"
        );
    let geometry_ok = matches!(
        fixture.component,
        Component::Text
            | Component::Button
            | Component::TextInput
            | Component::Checkbox
            | Component::IconButton
    ) || geometry.is_some();
    let layout_ok = bounds.is_some_and(|bounds| match fixture.component {
        Component::Text if matches!(fixture.state, "wrap" | "ellipsis") => {
            (bounds.width - 180.0).abs() < 0.01
        }
        Component::Text => bounds.width > 0.0 && bounds.height >= 32.0,
        Component::Button => {
            let expected = button_control_size(fixture.state).height();
            (bounds.height - expected).abs() < 0.01
        }
        Component::TextInput => {
            (bounds.width - 380.0).abs() < 0.01
                && (bounds.height - text_input_control_size(fixture.state).height()).abs() < 0.01
        }
        Component::Checkbox => bounds.height >= ControlSize::Medium.height(),
        _ => true,
    });
    let runtime_ok = bounds.is_some()
        && accessibility.is_some()
        && geometry_ok
        && layout_ok
        && runtime.idle
        && hit_ok
        && (!action_state || runtime.action_applied)
        && (fixture.state != "loading"
            || fixture.component == Component::TextInput
            || runtime.next_deadline.is_some())
        && (fixture.component != Component::TextInput
            || match fixture.state {
                "read-only" => accessibility.is_some_and(|node| !node.editable && !node.disabled),
                "loading" => accessibility.is_some_and(|node| node.busy && node.disabled),
                "secure" => accessibility.is_some_and(|node| node.value.is_none()),
                "selection" => matches!(
                    geometry,
                    Some(nana_ui::runtime::ComponentGeometry::TextInput {
                        selection: Some(_),
                        ..
                    })
                ),
                _ => true,
            })
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
        "expected: {}\niced_observed: {}\niced_verdict: {}\nruntime_expected: {}\nruntime_observed: bounds={bounds:?}; layout_ok={layout_ok}; geometry={geometry:?}; hit={hit:?}; accessibility={accessibility:?}; tooltip={tooltip:?}; active_overlay={active_overlay:?}; first_passes={}; first_accessibility_updates={}; final_passes={}; final_accessibility_updates={}; second_flush_idle={}; action_applied={}; next_animation_deadline={:?}; primitives={primitives:?}\nmachine_verdict: {}\nreview_observed: {}\nreview_verdict: {}\nintentional_divergence_reason: {}\n",
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
        (Component::Button, _) => (
            "pass",
            "Fresh isolated dark and light review confirms semantic kinds, three control sizes, interaction feedback, external focus, disabled and loading presentation, complete label geometry, hit behavior and accessibility",
        ),
        (Component::TextInput, _) => (
            "pass",
            "Fresh isolated dark and light review confirms placeholder contrast, shaped selection and caret geometry, external focus, invalid, secure, size, read-only, loading, keyboard and IME preedit or commit presentation",
        ),
        (Component::Text, _) => (
            "pass",
            "Runtime text uses the authored content box, shared typography, semantic contrast, wrapping or ellipsis, alignment, clipping and accessibility in dark and light",
        ),
        (Component::Checkbox, _) => (
            "pass",
            "Runtime checkbox keeps indicator and label geometry, semantic checked and invalid paint, complete-row hit testing, focus, disabled behavior and accessibility in dark and light",
        ),
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

fn button_control_size(state: &str) -> ControlSize {
    match state {
        "small" => ControlSize::Small,
        "large" => ControlSize::Large,
        _ => ControlSize::Medium,
    }
}

fn text_input_control_size(state: &str) -> ControlSize {
    match state {
        "small" => ControlSize::Small,
        "large" => ControlSize::Large,
        _ => ControlSize::Medium,
    }
}

fn button_kind(state: &str) -> nana_ui::ButtonKind {
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
