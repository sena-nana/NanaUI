//! Backend-neutral split-pane persistence and interaction state.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SplitPaneMutation {
    SetSize(f32),
    Reset,
    ResizeStart,
    ResizeMove { x: f32, y: f32 },
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

#[derive(Debug, Clone, Copy)]
struct ResizeState {
    start_position: Option<f32>,
    start_size: f32,
}

#[derive(Debug, Clone)]
pub struct SplitPaneModel {
    persisted: PersistedSplitPane,
    resize: Option<ResizeState>,
    focused: bool,
    hovered: bool,
}

impl SplitPaneModel {
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

    pub fn with_keyboard_step(mut self, step: f32) -> Self {
        self.persisted.keyboard_step = finite_positive(step, 8.0);
        self
    }

    pub fn with_from_end(mut self, from_end: bool) -> Self {
        self.persisted.from_end = from_end;
        self
    }

    pub fn from_end(&self) -> bool {
        self.persisted.from_end
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    pub fn hovered(&self) -> bool {
        self.hovered
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

    pub fn update(&mut self, mutation: SplitPaneMutation) -> bool {
        match mutation {
            SplitPaneMutation::SetSize(size) => self.set_size(size),
            SplitPaneMutation::Reset => {
                let interaction_changed = self.is_active();
                self.cancel_interaction();
                self.set_size(self.default_size()) || interaction_changed
            }
            SplitPaneMutation::ResizeStart => {
                let changed = self.resize.is_none() || !self.focused;
                self.resize = Some(ResizeState {
                    start_position: None,
                    start_size: self.size(),
                });
                self.focused = true;
                changed
            }
            SplitPaneMutation::ResizeMove { x, y } => self.resize_move(x, y),
            SplitPaneMutation::ResizeEnd => {
                let changed = self.resize.is_some();
                self.resize = None;
                changed
            }
            SplitPaneMutation::Adjust(direction) => {
                let direction = if self.from_end() {
                    -direction
                } else {
                    direction
                };
                self.set_size(self.size() + direction * self.persisted.keyboard_step)
            }
            SplitPaneMutation::Focus => {
                let changed = !self.focused;
                self.focused = true;
                changed
            }
            SplitPaneMutation::Blur => {
                let changed = self.focused || self.resize.is_some();
                self.focused = false;
                self.resize = None;
                changed
            }
            SplitPaneMutation::Hover(hovered) => {
                let changed = self.hovered != hovered;
                self.hovered = hovered;
                changed
            }
        }
    }

    fn resize_move(&mut self, x: f32, y: f32) -> bool {
        let axis = self.axis();
        let from_end = self.from_end();
        let position = match axis {
            SplitAxis::Horizontal => x,
            SplitAxis::Vertical => y,
        };
        if !position.is_finite() {
            return false;
        }
        let direction = if from_end { -1.0 } else { 1.0 };
        let requested_size = {
            let Some(resize) = &mut self.resize else {
                return false;
            };
            let Some(start_position) = resize.start_position else {
                resize.start_position = Some(position);
                return false;
            };
            resize.start_size + (position - start_position) * direction
        };
        self.set_size(requested_size)
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
    fn drag_uses_absolute_delta_without_clamp_drift() {
        let mut model = SplitPaneModel::new(SplitAxis::Horizontal, 200.0, 140.0, 260.0);
        assert!(model.update(SplitPaneMutation::ResizeStart));
        assert!(!model.update(SplitPaneMutation::ResizeMove { x: 100.0, y: 0.0 }));
        assert!(model.update(SplitPaneMutation::ResizeMove { x: 500.0, y: 0.0 }));
        assert!(model.update(SplitPaneMutation::ResizeMove { x: 130.0, y: 0.0 }));
        assert_eq!(model.size(), 230.0);
    }

    #[test]
    fn reset_reports_interaction_changes_even_at_the_default_size() {
        let mut model = SplitPaneModel::new(SplitAxis::Vertical, 120.0, 64.0, 280.0);
        assert!(model.update(SplitPaneMutation::Focus));
        assert!(model.update(SplitPaneMutation::Reset));
        assert!(!model.is_active());
    }
}
