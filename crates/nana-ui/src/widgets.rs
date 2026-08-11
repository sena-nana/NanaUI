use iced::widget::{
    button, checkbox, container, overlay::menu, pick_list, progress_bar, scrollable, slider,
    text_editor, text_input, toggler,
};
use iced::{Border, Color, Shadow, Theme, Vector};

use crate::theme::{Colors, ThemeTokens};

pub use nana_ui_core::{ButtonKind, CardKind};

const SEGMENTED_CONTROL_BORDER_WIDTH: f32 = 1.0;
const SEGMENTED_CONTROL_PADDING: f32 = 2.0;
pub const SEGMENTED_CONTROL_INSET: f32 = SEGMENTED_CONTROL_BORDER_WIDTH + SEGMENTED_CONTROL_PADDING;

/// Optional CSS / layout paint overrides for [`button_style`].
///
/// When set, these replace the corresponding `ButtonKind` theme defaults for
/// the Active (and Disabled-faded) surface. Hover / Pressed still follow kind
/// interaction colors so Ghost toolbar icons keep hover feedback.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ButtonPaintOverride {
    pub background: Option<Color>,
    pub text_color: Option<Color>,
    pub border_radius: Option<f32>,
    pub border_width: Option<f32>,
    pub border_color: Option<Color>,
}

impl ButtonPaintOverride {
    pub fn is_empty(self) -> bool {
        self.background.is_none()
            && self.text_color.is_none()
            && self.border_radius.is_none()
            && self.border_width.is_none()
            && self.border_color.is_none()
    }
}

pub fn button_style(
    theme: impl Into<ThemeTokens>,
    kind: ButtonKind,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    button_style_overridden(theme, kind, ButtonPaintOverride::default())
}

pub fn button_style_overridden(
    theme: impl Into<ThemeTokens>,
    kind: ButtonKind,
    paint: ButtonPaintOverride,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme, status| {
        let (kind_base, kind_foreground, kind_border_color) = match kind {
            ButtonKind::Ghost => (Color::TRANSPARENT, colors.text, Color::TRANSPARENT),
            ButtonKind::Subtle => (colors.subtle, colors.text, colors.border_soft),
            ButtonKind::Selected => (colors.active, colors.text, Color::TRANSPARENT),
            ButtonKind::Primary => (
                colors.accent_soft,
                colors.accent_on_soft,
                Color::TRANSPARENT,
            ),
            ButtonKind::Warning => (
                fade(
                    colors.warning,
                    if colors.background.r > 0.5 {
                        0.12
                    } else {
                        0.16
                    },
                ),
                colors.warning,
                Color::TRANSPARENT,
            ),
            ButtonKind::Danger => (Color::TRANSPARENT, colors.danger, Color::TRANSPARENT),
            ButtonKind::Text => (Color::TRANSPARENT, colors.accent, Color::TRANSPARENT),
        };
        let base = paint.background.unwrap_or(kind_base);
        let foreground = paint.text_color.unwrap_or(kind_foreground);
        let border_color = paint.border_color.unwrap_or(kind_border_color);
        let radius = paint.border_radius.unwrap_or(metrics.radius_sm);
        let border_width = paint
            .border_width
            .unwrap_or(if matches!(kind, ButtonKind::Subtle) {
                1.0
            } else {
                0.0
            });

        let background = match status {
            button::Status::Hovered => match kind {
                ButtonKind::Primary => {
                    Color::from_rgba(colors.accent.r, colors.accent.g, colors.accent.b, 0.20)
                }
                ButtonKind::Warning => fade(colors.warning, 0.20),
                ButtonKind::Danger => fade(colors.danger, 0.18),
                ButtonKind::Selected => colors.selected_hover,
                _ => colors.hover,
            },
            button::Status::Pressed => match kind {
                ButtonKind::Primary => {
                    Color::from_rgba(colors.accent.r, colors.accent.g, colors.accent.b, 0.23)
                }
                ButtonKind::Warning => fade(colors.warning, 0.24),
                ButtonKind::Danger => fade(colors.danger, 0.22),
                ButtonKind::Selected => colors.selected_pressed,
                _ => colors.active,
            },
            button::Status::Disabled => fade(base, 0.45),
            button::Status::Active => base,
        };
        let text_color = if status == button::Status::Disabled {
            fade(foreground, 0.45)
        } else {
            foreground
        };
        let mut style = button::Style::default().with_background(background);
        style.text_color = text_color;
        style.border = Border::default().rounded(radius).width(border_width).color(
            if status == button::Status::Disabled {
                fade(border_color, 0.45)
            } else {
                border_color
            },
        );
        style.shadow = Shadow::default();
        style.snap = true;
        style
    }
}

pub fn dialog_close_style(
    theme: impl Into<ThemeTokens>,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme, status| {
        let (background, foreground) = match status {
            button::Status::Hovered => (colors.hover, colors.text),
            button::Status::Pressed => (colors.active, colors.text),
            button::Status::Disabled => (Color::TRANSPARENT, fade(colors.muted, 0.55)),
            button::Status::Active => (Color::TRANSPARENT, colors.muted),
        };
        let mut style = button::Style::default().with_background(background);
        style.text_color = foreground;
        style.border = Border::default().rounded(metrics.radius_sm);
        style.shadow = Shadow::default();
        style.snap = true;
        style
    }
}

pub fn selection_button_style(
    theme: impl Into<ThemeTokens>,
    selected: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    let tokens = theme.into();
    selection_button_style_with_radius(tokens, selected, tokens.metrics.radius_sm)
}

pub fn segmented_button_style(
    theme: impl Into<ThemeTokens>,
    selected: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    let tokens = theme.into();
    selection_button_style_with_radius(
        tokens,
        selected,
        (tokens.metrics.radius_md - SEGMENTED_CONTROL_INSET).max(0.0),
    )
}

fn selection_button_style_with_radius(
    tokens: ThemeTokens,
    selected: bool,
    radius: f32,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    let colors = tokens.colors;
    move |_theme, status| {
        let background = match status {
            button::Status::Hovered if selected => colors.selected_hover,
            button::Status::Hovered => colors.hover,
            button::Status::Pressed if selected => colors.selected_pressed,
            button::Status::Pressed => colors.active,
            button::Status::Disabled => Color::TRANSPARENT,
            button::Status::Active if selected => colors.selected,
            button::Status::Active => Color::TRANSPARENT,
        };
        let foreground = if status == button::Status::Disabled {
            fade(colors.muted, 0.45)
        } else if selected {
            colors.text
        } else {
            colors.muted
        };

        let mut style = button::Style::default().with_background(background);
        style.text_color = foreground;
        style.border = Border::default().rounded(radius);
        style.shadow = Shadow::default();
        style.snap = true;
        style
    }
}

pub fn menu_item_style(
    theme: impl Into<ThemeTokens>,
    danger: bool,
    pending: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme, status| {
        let danger_soft = fade(
            colors.danger,
            if colors.background.r > 0.5 {
                0.10
            } else {
                0.14
            },
        );
        let foreground = if danger { colors.danger } else { colors.text };
        let background = match status {
            button::Status::Hovered if danger => danger_soft,
            button::Status::Hovered => colors.hover,
            button::Status::Pressed if danger => fade(colors.danger, 0.20),
            button::Status::Pressed => colors.active,
            button::Status::Disabled => Color::TRANSPARENT,
            button::Status::Active if pending => danger_soft,
            button::Status::Active => Color::TRANSPARENT,
        };

        let mut style = button::Style::default().with_background(background);
        style.text_color = if status == button::Status::Disabled {
            fade(foreground, 0.45)
        } else {
            foreground
        };
        style.border = Border::default().rounded(metrics.radius_sm);
        style.shadow = Shadow::default();
        style.snap = true;
        style
    }
}

pub fn text_input_style(
    theme: impl Into<ThemeTokens>,
    invalid: bool,
) -> impl Fn(&Theme, text_input::Status) -> text_input::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme, status| {
        let disabled = matches!(status, text_input::Status::Disabled);
        let focused = matches!(status, text_input::Status::Focused { .. });
        let border_color = if invalid {
            colors.danger
        } else {
            match status {
                text_input::Status::Hovered => colors.border_strong,
                text_input::Status::Focused { .. } => colors.border_soft,
                text_input::Status::Disabled => colors.border_soft,
                text_input::Status::Active => colors.border,
            }
        };

        text_input::Style {
            background: if disabled {
                colors.subtle.into()
            } else {
                colors.background.into()
            },
            border: Border::default()
                .rounded(metrics.radius_sm)
                .width(if focused && invalid { 2.0 } else { 1.0 })
                .color(border_color),
            icon: if disabled { colors.faint } else { colors.muted },
            placeholder: colors.faint,
            value: if disabled { colors.faint } else { colors.text },
            selection: colors.accent_soft,
        }
    }
}

pub fn text_editor_style(
    theme: impl Into<ThemeTokens>,
    invalid: bool,
) -> impl Fn(&Theme, text_editor::Status) -> text_editor::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme, status| {
        let disabled = matches!(status, text_editor::Status::Disabled);
        let focused = matches!(status, text_editor::Status::Focused { .. });
        let border_color = if invalid {
            colors.danger
        } else {
            match status {
                text_editor::Status::Hovered => colors.border_strong,
                text_editor::Status::Focused { .. } => colors.border_soft,
                text_editor::Status::Disabled => colors.border_soft,
                text_editor::Status::Active => colors.border,
            }
        };

        text_editor::Style {
            background: if disabled {
                colors.subtle.into()
            } else {
                colors.background.into()
            },
            border: Border::default()
                .rounded(metrics.radius_sm)
                .width(if focused && invalid { 2.0 } else { 1.0 })
                .color(border_color),
            placeholder: colors.faint,
            value: if disabled { colors.faint } else { colors.text },
            selection: colors.accent_soft,
        }
    }
}

pub fn checkbox_style(
    colors: Colors,
    invalid: bool,
) -> impl Fn(&Theme, checkbox::Status) -> checkbox::Style + 'static {
    move |_theme, status| {
        let (is_checked, hovered, disabled) = match status {
            checkbox::Status::Active { is_checked } => (is_checked, false, false),
            checkbox::Status::Hovered { is_checked } => (is_checked, true, false),
            checkbox::Status::Disabled { is_checked } => (is_checked, false, true),
        };
        let border_color = if invalid {
            colors.danger
        } else if disabled {
            colors.border
        } else if is_checked || hovered {
            colors.accent
        } else {
            colors.border_strong
        };

        let background = if is_checked {
            colors.accent
        } else if disabled {
            colors.subtle
        } else {
            colors.background
        };

        checkbox::Style {
            background: fade(background, if disabled { 0.55 } else { 1.0 }).into(),
            icon_color: if disabled {
                fade(colors.accent_text, 0.55)
            } else {
                colors.accent_text
            },
            border: Border::default()
                .rounded(4.0)
                .width(1.0)
                .color(fade(border_color, if disabled { 0.55 } else { 1.0 })),
            text_color: Some(if disabled {
                fade(colors.text, 0.55)
            } else {
                colors.text
            }),
        }
    }
}

pub fn toggler_style(
    colors: Colors,
    invalid: bool,
) -> impl Fn(&Theme, toggler::Status) -> toggler::Style + 'static {
    move |_theme, status| {
        let (is_toggled, hovered, disabled) = match status {
            toggler::Status::Active { is_toggled } => (is_toggled, false, false),
            toggler::Status::Hovered { is_toggled } => (is_toggled, true, false),
            toggler::Status::Disabled { is_toggled } => (is_toggled, false, true),
        };
        let background = if is_toggled {
            colors.accent
        } else if hovered {
            colors.active
        } else {
            mix(colors.hover, colors.background, 0.78)
        };
        let border_color = if invalid {
            colors.danger
        } else if is_toggled {
            colors.accent
        } else if hovered {
            mix(colors.accent, colors.border_strong, 0.42)
        } else {
            colors.border_strong
        };

        toggler::Style {
            background: fade(background, if disabled { 0.55 } else { 1.0 }).into(),
            background_border_width: 1.0,
            background_border_color: fade(border_color, if disabled { 0.55 } else { 1.0 }),
            foreground: fade(
                if is_toggled {
                    colors.accent_text
                } else {
                    mix(colors.faint, colors.background, 0.70)
                },
                if disabled { 0.55 } else { 1.0 },
            )
            .into(),
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
            text_color: Some(if disabled { colors.faint } else { colors.text }),
            border_radius: None,
            padding_ratio: 0.1875,
        }
    }
}

pub fn slider_style(colors: Colors) -> impl Fn(&Theme, slider::Status) -> slider::Style + 'static {
    move |_theme, status| {
        let handle = match status {
            slider::Status::Active => colors.accent,
            slider::Status::Hovered => colors.accent_strong,
            slider::Status::Dragged => colors.accent_strong,
        };

        slider::Style {
            rail: slider::Rail {
                backgrounds: (
                    colors.accent.into(),
                    mix(colors.hover, colors.background, 0.78).into(),
                ),
                width: 8.0,
                border: Border::default().rounded(999.0),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Circle { radius: 7.0 },
                background: handle.into(),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        }
    }
}

pub fn progress_style(colors: Colors) -> impl Fn(&Theme) -> progress_bar::Style + 'static {
    move |_theme| progress_bar::Style {
        background: colors.subtle.into(),
        bar: colors.accent.into(),
        border: Border::default().rounded(999.0),
    }
}

pub fn pick_list_style(
    theme: impl Into<ThemeTokens>,
    invalid: bool,
) -> impl Fn(&Theme, pick_list::Status) -> pick_list::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme, status| {
        let disabled = status == pick_list::Status::Disabled;
        let opened = matches!(status, pick_list::Status::Opened { .. });
        let border_color = if invalid {
            colors.danger
        } else {
            match status {
                pick_list::Status::Hovered => colors.border_strong,
                pick_list::Status::Opened { .. } => colors.border_soft,
                pick_list::Status::Active | pick_list::Status::Disabled => colors.border,
            }
        };
        pick_list::Style {
            text_color: if disabled { colors.faint } else { colors.text },
            placeholder_color: colors.faint,
            handle_color: if disabled { colors.faint } else { colors.muted },
            background: if disabled {
                colors.subtle.into()
            } else {
                colors.background.into()
            },
            border: Border::default()
                .rounded(metrics.radius_sm)
                .width(if opened && invalid { 2.0 } else { 1.0 })
                .color(border_color),
        }
    }
}

pub fn pick_list_menu_style(
    theme: impl Into<ThemeTokens>,
) -> impl Fn(&Theme) -> menu::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme| menu::Style {
        background: colors.surface.into(),
        border: Border::default()
            .rounded(metrics.radius_md)
            .width(1.0)
            .color(colors.border_soft),
        text_color: colors.text,
        selected_text_color: colors.text,
        selected_background: colors.selected.into(),
        shadow: Shadow {
            color: fade(
                Color::BLACK,
                if colors.background.r > 0.5 {
                    0.24
                } else {
                    0.48
                },
            ),
            offset: Vector::new(0.0, 8.0),
            blur_radius: 16.0,
        },
    }
}

pub fn list_item_style(
    theme: impl Into<ThemeTokens>,
    selected: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme, status| {
        let background = match (selected, status) {
            (true, button::Status::Active) => colors.selected,
            (true, button::Status::Hovered) => colors.selected_hover,
            (true, button::Status::Pressed) => colors.selected_pressed,
            (true, button::Status::Disabled) => fade(colors.selected, 0.50),
            (false, button::Status::Hovered) => colors.hover,
            (false, button::Status::Pressed) => colors.active,
            (false, button::Status::Disabled | button::Status::Active) => Color::TRANSPARENT,
        };
        let mut style = button::Style::default().with_background(background);
        style.text_color = if status == button::Status::Disabled {
            fade(colors.text, 0.50)
        } else {
            colors.text
        };
        style.border = Border::default()
            .rounded(metrics.radius_sm)
            .width(1.0)
            .color(Color::TRANSPARENT);
        style.shadow = Shadow::default();
        style.snap = true;
        style
    }
}

pub fn card_style(
    theme: impl Into<ThemeTokens>,
    kind: CardKind,
) -> impl Fn(&Theme) -> container::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme| {
        let (background, border, shadow) = match kind {
            CardKind::Surface => (colors.surface, Color::TRANSPARENT, Shadow::default()),
            CardKind::Outlined => (colors.surface, colors.border, Shadow::default()),
            CardKind::Raised => (
                colors.surface,
                Color::TRANSPARENT,
                Shadow {
                    color: fade(
                        Color::BLACK,
                        if colors.background.r > 0.5 {
                            0.20
                        } else {
                            0.42
                        },
                    ),
                    offset: Vector::new(0.0, 4.0),
                    blur_radius: 12.0,
                },
            ),
            CardKind::Flat => (Color::TRANSPARENT, Color::TRANSPARENT, Shadow::default()),
            CardKind::Selected => (colors.selected, colors.border_soft, Shadow::default()),
        };

        container::Style::default()
            .background(background)
            .color(colors.text)
            .border(
                Border::default()
                    .rounded(metrics.radius_md)
                    .width(if matches!(kind, CardKind::Outlined | CardKind::Selected) {
                        1.0
                    } else {
                        0.0
                    })
                    .color(border),
            )
            .shadow(shadow)
    }
}

pub fn interactive_card_style(
    theme: impl Into<ThemeTokens>,
    selected: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme, status| {
        let background = match (selected, status) {
            (true, button::Status::Active) => colors.selected,
            (true, button::Status::Hovered) => colors.selected_hover,
            (true, button::Status::Pressed) => colors.selected_pressed,
            (true, button::Status::Disabled) => fade(colors.selected, 0.55),
            (false, button::Status::Active) => colors.surface,
            (false, button::Status::Hovered) => colors.hover,
            (false, button::Status::Pressed) => colors.active,
            (false, button::Status::Disabled) => fade(colors.surface, 0.55),
        };
        let mut style = button::Style::default().with_background(background);
        style.text_color = if status == button::Status::Disabled {
            fade(colors.text, 0.55)
        } else {
            colors.text
        };
        style.border = Border::default()
            .rounded(metrics.radius_md)
            .width(if selected { 1.0 } else { 0.0 })
            .color(if status == button::Status::Disabled {
                fade(colors.border_soft, 0.55)
            } else {
                colors.border_soft
            });
        style.shadow = Shadow::default();
        style.snap = true;
        style
    }
}

pub fn scrollable_style(
    colors: Colors,
) -> impl Fn(&Theme, scrollable::Status) -> scrollable::Style + 'static {
    move |theme, status| {
        let opacity = match status {
            scrollable::Status::Active { .. } => 0.0,
            scrollable::Status::Hovered {
                is_horizontal_scrollbar_hovered,
                is_vertical_scrollbar_hovered,
                ..
            } if is_horizontal_scrollbar_hovered || is_vertical_scrollbar_hovered => 1.0,
            scrollable::Status::Hovered { .. } => 0.35,
            scrollable::Status::Dragged { .. } => 1.0,
        };
        let rail = scrollable::Rail {
            background: None,
            border: Border::default(),
            scroller: scrollable::Scroller {
                background: fade(colors.border_strong, opacity).into(),
                border: Border::default().rounded(999.0),
            },
        };
        scrollable::Style {
            vertical_rail: rail,
            horizontal_rail: rail,
            ..scrollable::default(theme, status)
        }
    }
}

pub fn vertical_scrollbar() -> scrollable::Direction {
    scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(12).scroller_width(4))
}

pub fn panel_style(theme: impl Into<ThemeTokens>) -> impl Fn(&Theme) -> container::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme| {
        container::Style::default()
            .background(colors.surface)
            .color(colors.text)
            .border(
                Border::default()
                    .rounded(metrics.radius_sm)
                    .width(1.0)
                    .color(colors.border),
            )
    }
}

/// Workspace regions are structural surfaces, not cards. LiliaUI leaves these
/// regions square and borderless; only the primary region owns a rounded clip.
pub fn workspace_region_style(colors: Colors) -> impl Fn(&Theme) -> container::Style + 'static {
    move |_theme| {
        container::Style::default()
            .background(colors.surface)
            .color(colors.text)
    }
}

/// Styles the workspace primary surface using LiliaUI's edge-aware corners.
///
/// A side becomes square only when the primary track is the first or last
/// expanded track in the workspace middle row.
pub fn primary_region_style(
    theme: impl Into<ThemeTokens>,
    edge_start: bool,
    edge_end: bool,
) -> impl Fn(&Theme) -> container::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let radius = primary_region_radius(tokens, edge_start, edge_end);
    move |_theme| {
        container::Style::default()
            .background(colors.background)
            .color(colors.text)
            .border(Border::default().rounded(radius))
    }
}

pub(crate) fn primary_region_radius(
    tokens: ThemeTokens,
    edge_start: bool,
    edge_end: bool,
) -> iced::border::Radius {
    let radius = if tokens.workspace_corners_enabled {
        tokens.metrics.radius_lg
    } else {
        0.0
    };
    iced::border::Radius {
        top_left: if edge_start { 0.0 } else { radius },
        top_right: if edge_end { 0.0 } else { radius },
        bottom_right: if edge_end { 0.0 } else { radius },
        bottom_left: if edge_start { 0.0 } else { radius },
    }
}

pub fn menu_surface_style(
    theme: impl Into<ThemeTokens>,
) -> impl Fn(&Theme) -> container::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme| {
        let is_light = colors.background.r > 0.5;
        container::Style::default()
            .background(colors.surface)
            .color(colors.text)
            .border(
                Border::default()
                    .rounded(metrics.radius_md)
                    .width(1.0)
                    .color(colors.border_soft),
            )
            .shadow(Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, if is_light { 0.30 } else { 0.55 }),
                offset: Vector::new(0.0, 10.0),
                blur_radius: if is_light { 14.0 } else { 18.0 },
            })
    }
}

pub fn dialog_surface_style(
    theme: impl Into<ThemeTokens>,
) -> impl Fn(&Theme) -> container::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme| {
        let is_light = colors.background.r > 0.5;
        container::Style::default()
            .background(colors.surface)
            .color(colors.text)
            .border(
                Border::default()
                    .rounded(metrics.radius_md)
                    .width(1.0)
                    .color(colors.border_soft),
            )
            .shadow(Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, if is_light { 0.28 } else { 0.45 }),
                offset: Vector::new(0.0, 14.0),
                blur_radius: 30.0,
            })
    }
}

pub fn dialog_scrim_style(colors: Colors) -> impl Fn(&Theme) -> container::Style + 'static {
    move |_theme| {
        container::Style::default()
            .background(Color::from_rgba(0.0, 0.0, 0.0, 0.45))
            .color(colors.text)
    }
}

pub fn tooltip_style(colors: Colors) -> impl Fn(&Theme) -> container::Style + 'static {
    move |_theme| {
        container::Style::default()
            .background(colors.surface)
            .color(colors.text)
            .border(
                Border::default()
                    .rounded(4.0)
                    .width(1.0)
                    .color(colors.border_soft),
            )
    }
}

pub fn segmented_surface_style(
    theme: impl Into<ThemeTokens>,
) -> impl Fn(&Theme) -> container::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme| {
        container::Style::default()
            .background(colors.background)
            .color(colors.text)
            .border(
                Border::default()
                    .rounded(metrics.radius_md)
                    .width(SEGMENTED_CONTROL_BORDER_WIDTH)
                    .color(colors.border),
            )
    }
}

fn fade(color: Color, opacity: f32) -> Color {
    Color {
        a: color.a * opacity,
        ..color
    }
}

fn mix(foreground: Color, background: Color, foreground_ratio: f32) -> Color {
    let ratio = foreground_ratio.clamp(0.0, 1.0);
    let inverse = 1.0 - ratio;
    Color {
        r: foreground.r * ratio + background.r * inverse,
        g: foreground.g * ratio + background.g * inverse,
        b: foreground.b * ratio + background.b * inverse,
        a: foreground.a * ratio + background.a * inverse,
    }
}

pub fn canvas_style(
    theme: impl Into<ThemeTokens>,
) -> impl Fn(&Theme) -> container::Style + 'static {
    let tokens = theme.into();
    let colors = tokens.colors;
    let metrics = tokens.metrics;
    move |_theme| {
        container::Style::default()
            .background(colors.background)
            .color(colors.text)
            .border(Border::default().rounded(metrics.radius_lg))
    }
}

pub fn toolbar_style(colors: Colors) -> impl Fn(&Theme) -> container::Style + 'static {
    move |_theme| {
        container::Style::default()
            .background(colors.surface)
            .color(colors.text)
    }
}

#[cfg(test)]
mod tests {
    use iced::Theme;
    use iced::widget::{button, checkbox, pick_list, text_editor, text_input, toggler};

    use super::{
        CardKind, SEGMENTED_CONTROL_INSET, card_style, checkbox_style, list_item_style,
        pick_list_style, segmented_button_style, segmented_surface_style, text_editor_style,
        text_input_style, toggler_style,
    };
    use crate::theme::{ThemeMode, ThemeModeExt};

    #[test]
    fn semantic_control_states_have_distinct_visual_contracts() {
        let colors = ThemeMode::Dark.colors();
        let theme = Theme::Dark;

        let focused = text_input_style(colors, false)(
            &theme,
            text_input::Status::Focused { is_hovered: false },
        );
        assert_eq!(focused.background, colors.background.into());
        assert_eq!(focused.border.color, colors.border_soft);
        assert_eq!(focused.border.width, 1.0);

        let focused_editor = text_editor_style(colors, false)(
            &theme,
            text_editor::Status::Focused { is_hovered: false },
        );
        assert_eq!(focused_editor.background, colors.background.into());
        assert_eq!(focused_editor.border.color, colors.border_soft);
        assert_eq!(focused_editor.border.width, 1.0);

        let opened =
            pick_list_style(colors, false)(&theme, pick_list::Status::Opened { is_hovered: false });
        assert_eq!(opened.background, colors.background.into());
        assert_eq!(opened.border.color, colors.border_soft);
        assert_eq!(opened.border.width, 1.0);

        let invalid = text_input_style(colors, true)(
            &theme,
            text_input::Status::Focused { is_hovered: false },
        );
        assert_eq!(invalid.background, colors.background.into());
        assert_eq!(invalid.border.color, colors.danger);
        assert_eq!(invalid.border.width, 2.0);

        let checked =
            checkbox_style(colors, false)(&theme, checkbox::Status::Active { is_checked: true });
        assert_eq!(checked.background, colors.accent.into());

        let toggled =
            toggler_style(colors, false)(&theme, toggler::Status::Active { is_toggled: true });
        let idle =
            toggler_style(colors, false)(&theme, toggler::Status::Active { is_toggled: false });
        assert_ne!(toggled.background, idle.background);

        let selected_hover = list_item_style(colors, true)(&theme, button::Status::Hovered);
        assert_eq!(
            selected_hover.background,
            Some(colors.selected_hover.into())
        );

        let selected_card = card_style(colors, CardKind::Selected)(&theme);
        assert_eq!(selected_card.background, Some(colors.selected.into()));
        assert_eq!(selected_card.border.width, 1.0);
        assert_eq!(selected_card.border.color, colors.border_soft);
    }

    #[test]
    fn segmented_geometry_uses_a_balanced_concentric_inset() {
        let tokens = ThemeMode::Dark.tokens();
        let theme = Theme::Dark;
        let surface = segmented_surface_style(tokens)(&theme);
        let segment = segmented_button_style(tokens, true)(&theme, button::Status::Active);

        assert_eq!(
            segment.border.radius.top_left,
            surface.border.radius.top_left - SEGMENTED_CONTROL_INSET
        );
    }
}
