use iced::widget::{column, container, row, space};
use iced::{Element, Length, Point, Subscription};
use serde::{Deserialize, Serialize};

use crate::drag_handle::DragHandle;
use crate::resize_drag::{ResizeAxis, ResizeDrag};
use crate::theme::ThemeTokens;

const HANDLE_SIZE: f32 = 8.0;

/// Direction in which a split pane lays out its two children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

/// Framework-owned interaction emitted by [`split_pane`].
#[derive(Debug, Clone, PartialEq)]
pub enum SplitPaneAction {
    SetSize(f32),
    Reset,
    ResizeStart,
    ResizeMove(Point),
    ResizeEnd,
    Adjust(f32),
    Focus,
    Blur,
    Hover(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct PersistedSplitPane {
    version: u8,
    axis: SplitAxis,
    size: f32,
    default_size: f32,
    min_size: f32,
    max_size: f32,
    keyboard_step: f32,
    #[serde(default)]
    from_end: bool,
}

/// Owns a split pane's constraints, persisted size, and transient interaction.
#[derive(Debug, Clone)]
pub struct SplitPaneController {
    persisted: PersistedSplitPane,
    resize: Option<ResizeDrag>,
    focused: bool,
    hovered: bool,
}

impl SplitPaneController {
    pub fn new(axis: SplitAxis, default_size: f32, min_size: f32, max_size: f32) -> Self {
        let min_size = finite_non_negative(min_size, 0.0);
        let max_size = finite_non_negative(max_size, min_size).max(min_size);
        let default_size = clamp_size(default_size, min_size, max_size);
        Self {
            persisted: PersistedSplitPane {
                version: 1,
                axis,
                size: default_size,
                default_size,
                min_size,
                max_size,
                keyboard_step: 8.0,
                from_end: false,
            },
            resize: None,
            focused: false,
            hovered: false,
        }
    }

    pub fn axis(&self) -> SplitAxis {
        self.persisted.axis
    }

    pub fn size(&self) -> f32 {
        self.persisted.size
    }

    pub fn default_size(&self) -> f32 {
        self.persisted.default_size
    }

    pub fn limits(&self) -> (f32, f32) {
        (self.persisted.min_size, self.persisted.max_size)
    }

    pub fn keyboard_step(mut self, step: f32) -> Self {
        self.persisted.keyboard_step = finite_positive(step, 8.0);
        self
    }

    /// Sizes the second child from the trailing/bottom edge instead of the first.
    pub fn from_end(mut self, from_end: bool) -> Self {
        self.persisted.from_end = from_end;
        self
    }

    pub fn is_active(&self) -> bool {
        self.resize.is_some() || self.hovered || self.focused
    }

    pub fn layout_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.persisted)
    }

    pub fn restore_layout_json(&mut self, value: &str) -> Result<(), serde_json::Error> {
        let restored: PersistedSplitPane = serde_json::from_str(value)?;
        let min_size = finite_non_negative(restored.min_size, self.persisted.min_size);
        let max_size = finite_non_negative(restored.max_size, min_size).max(min_size);
        self.persisted = PersistedSplitPane {
            version: 1,
            axis: restored.axis,
            size: clamp_size(restored.size, min_size, max_size),
            default_size: clamp_size(restored.default_size, min_size, max_size),
            min_size,
            max_size,
            keyboard_step: finite_positive(restored.keyboard_step, 8.0),
            from_end: restored.from_end,
        };
        self.cancel_interaction();
        Ok(())
    }

    /// Listens for arrow-key adjustments while the separator owns keyboard focus.
    pub fn subscription(&self) -> Subscription<SplitPaneAction> {
        if !self.focused {
            return Subscription::none();
        }
        match self.axis() {
            SplitAxis::Horizontal => iced::event::listen_with(horizontal_key_event),
            SplitAxis::Vertical => iced::event::listen_with(vertical_key_event),
        }
    }

    /// Applies one interaction and reports whether observable state changed.
    pub fn update(&mut self, action: SplitPaneAction) -> bool {
        match action {
            SplitPaneAction::SetSize(size) => self.set_size(size),
            SplitPaneAction::Reset => {
                self.cancel_interaction();
                self.set_size(self.default_size())
            }
            SplitPaneAction::ResizeStart => {
                let changed = self.resize.is_none() || !self.focused;
                let axis = match self.axis() {
                    SplitAxis::Horizontal => ResizeAxis::Horizontal,
                    SplitAxis::Vertical => ResizeAxis::Vertical,
                };
                let direction = if self.persisted.from_end { -1.0 } else { 1.0 };
                self.resize = Some(ResizeDrag::new(axis, self.size(), direction));
                self.focused = true;
                changed
            }
            SplitPaneAction::ResizeMove(position) => {
                let Some(resize) = &mut self.resize else {
                    return false;
                };
                resize
                    .value(position)
                    .is_some_and(|size| self.set_size(size))
            }
            SplitPaneAction::ResizeEnd => {
                let changed = self.resize.is_some();
                self.resize = None;
                changed
            }
            SplitPaneAction::Adjust(direction) => {
                let direction = if self.persisted.from_end {
                    -direction
                } else {
                    direction
                };
                self.set_size(self.size() + direction * self.persisted.keyboard_step)
            }
            SplitPaneAction::Focus => {
                let changed = !self.focused;
                self.focused = true;
                changed
            }
            SplitPaneAction::Blur => {
                let changed = self.focused || self.resize.is_some();
                self.focused = false;
                self.resize = None;
                changed
            }
            SplitPaneAction::Hover(hovered) => {
                let changed = self.hovered != hovered;
                self.hovered = hovered;
                changed
            }
        }
    }

    fn set_size(&mut self, size: f32) -> bool {
        let size = clamp_size(size, self.persisted.min_size, self.persisted.max_size);
        let changed = self.persisted.size != size;
        self.persisted.size = size;
        changed
    }

    fn cancel_interaction(&mut self) {
        self.resize = None;
        self.focused = false;
        self.hovered = false;
    }
}

/// Builds a constrained two-child split pane. Split panes can be nested freely.
pub fn split_pane<'a, Message>(
    controller: &SplitPaneController,
    first: impl Into<Element<'a, Message>>,
    second: impl Into<Element<'a, Message>>,
    on_action: impl Fn(SplitPaneAction) -> Message + Copy + 'a,
    theme: impl Into<ThemeTokens>,
) -> Element<'a, Message>
where
    Message: Clone + 'a,
{
    let tokens = theme.into();
    let color = if controller.is_active() {
        tokens.colors.border_strong
    } else {
        tokens.colors.border
    };
    let horizontal = controller.axis() == SplitAxis::Horizontal;
    let indicator = container(space())
        .width(if horizontal {
            Length::Fixed(2.0)
        } else {
            Length::Fill
        })
        .height(if horizontal {
            Length::Fill
        } else {
            Length::Fixed(2.0)
        })
        .style(move |_theme| iced::widget::container::Style::default().background(color));
    let handle = container(indicator)
        .width(if horizontal {
            Length::Fixed(HANDLE_SIZE)
        } else {
            Length::Fill
        })
        .height(if horizontal {
            Length::Fill
        } else {
            Length::Fixed(HANDLE_SIZE)
        })
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center);
    let handle = DragHandle::new(
        handle,
        on_action(SplitPaneAction::ResizeStart),
        move |point| on_action(SplitPaneAction::ResizeMove(point)),
        on_action(SplitPaneAction::ResizeEnd),
        on_action(SplitPaneAction::Reset),
        move |hovered| on_action(SplitPaneAction::Hover(hovered)),
        if horizontal {
            iced::mouse::Interaction::ResizingHorizontally
        } else {
            iced::mouse::Interaction::ResizingVertically
        },
    );
    let first = container(first.into())
        .width(if horizontal {
            if controller.persisted.from_end {
                Length::Fill
            } else {
                Length::Fixed(controller.size())
            }
        } else {
            Length::Fill
        })
        .height(if !horizontal && !controller.persisted.from_end {
            Length::Fixed(controller.size())
        } else {
            Length::Fill
        })
        .clip(true);
    let second = container(second.into())
        .width(if horizontal && controller.persisted.from_end {
            Length::Fixed(controller.size())
        } else {
            Length::Fill
        })
        .height(if !horizontal && controller.persisted.from_end {
            Length::Fixed(controller.size())
        } else {
            Length::Fill
        })
        .clip(true);

    if horizontal {
        row![first, handle, second]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    } else {
        column![first, handle, second]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

fn horizontal_key_event(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<SplitPaneAction> {
    key_event(event, status, SplitAxis::Horizontal)
}

fn vertical_key_event(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<SplitPaneAction> {
    key_event(event, status, SplitAxis::Vertical)
}

fn key_event(
    event: iced::Event,
    status: iced::event::Status,
    axis: SplitAxis,
) -> Option<SplitPaneAction> {
    if status == iced::event::Status::Captured {
        return None;
    }
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    use iced::keyboard::key::Named;
    match (axis, key) {
        (_, iced::keyboard::Key::Named(Named::Escape)) => Some(SplitPaneAction::Blur),
        (SplitAxis::Horizontal, iced::keyboard::Key::Named(Named::ArrowLeft))
        | (SplitAxis::Vertical, iced::keyboard::Key::Named(Named::ArrowUp)) => {
            Some(SplitPaneAction::Adjust(-1.0))
        }
        (SplitAxis::Horizontal, iced::keyboard::Key::Named(Named::ArrowRight))
        | (SplitAxis::Vertical, iced::keyboard::Key::Named(Named::ArrowDown)) => {
            Some(SplitPaneAction::Adjust(1.0))
        }
        _ => None,
    }
}

fn clamp_size(value: f32, min_size: f32, max_size: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min_size, max_size)
    } else {
        min_size
    }
}

fn finite_non_negative(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        fallback
    }
}

fn finite_positive(value: f32, fallback: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_drag_and_keyboard_adjustments() {
        let mut controller = SplitPaneController::new(SplitAxis::Horizontal, 200.0, 140.0, 260.0);
        controller.update(SplitPaneAction::SetSize(400.0));
        assert_eq!(controller.size(), 260.0);
        controller.update(SplitPaneAction::Adjust(-20.0));
        assert_eq!(controller.size(), 140.0);
    }

    #[test]
    fn reset_restores_default_and_ends_drag() {
        let mut controller = SplitPaneController::new(SplitAxis::Vertical, 120.0, 64.0, 280.0);
        controller.update(SplitPaneAction::ResizeStart);
        controller.update(SplitPaneAction::ResizeMove(Point::new(0.0, 10.0)));
        controller.update(SplitPaneAction::ResizeMove(Point::new(0.0, 70.0)));
        assert_eq!(controller.size(), 180.0);
        controller.update(SplitPaneAction::Reset);
        assert_eq!(controller.size(), 120.0);
        assert!(!controller.is_active());
    }

    #[test]
    fn drag_reenters_limits_at_the_current_pointer_position() {
        let mut controller = SplitPaneController::new(SplitAxis::Horizontal, 200.0, 140.0, 260.0);
        controller.update(SplitPaneAction::ResizeStart);
        controller.update(SplitPaneAction::ResizeMove(Point::new(100.0, 0.0)));
        controller.update(SplitPaneAction::ResizeMove(Point::new(500.0, 0.0)));
        assert_eq!(controller.size(), 260.0);
        controller.update(SplitPaneAction::ResizeMove(Point::new(130.0, 0.0)));
        assert_eq!(controller.size(), 230.0);

        let mut from_end =
            SplitPaneController::new(SplitAxis::Vertical, 200.0, 140.0, 260.0).from_end(true);
        from_end.update(SplitPaneAction::ResizeStart);
        from_end.update(SplitPaneAction::ResizeMove(Point::new(0.0, 100.0)));
        from_end.update(SplitPaneAction::ResizeMove(Point::new(0.0, 500.0)));
        assert_eq!(from_end.size(), 140.0);
        from_end.update(SplitPaneAction::ResizeMove(Point::new(0.0, 70.0)));
        assert_eq!(from_end.size(), 230.0);
    }

    #[test]
    fn persisted_layout_round_trips_and_revalidates_constraints() {
        let mut controller =
            SplitPaneController::new(SplitAxis::Horizontal, 200.0, 140.0, 420.0).keyboard_step(4.0);
        controller.update(SplitPaneAction::SetSize(318.0));
        let encoded = controller.layout_json().expect("split layout serializes");
        let mut restored = SplitPaneController::new(SplitAxis::Vertical, 100.0, 0.0, 100.0);
        restored
            .restore_layout_json(&encoded)
            .expect("split layout restores");
        assert_eq!(restored.axis(), SplitAxis::Horizontal);
        assert_eq!(restored.size(), 318.0);
        assert_eq!(restored.limits(), (140.0, 420.0));
    }
}
