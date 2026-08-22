//! Host window-command adapter for dock effects.

use nana_ui_platform::{WindowCommand, WindowId, WindowRole, WindowSettings};

use crate::runtime_host::{RuntimeProgramUpdate, RuntimeRedraw};

use super::model::{DockHostEffect, DockUpdate};

/// Converts Dock window effects into Scene host window commands.
pub fn hosted_dock_update(update: DockUpdate, title: impl Into<String>) -> RuntimeProgramUpdate {
    let title = title.into();
    let redraw = redraw_for_dock_effects(&update.effects);
    let window_commands = update
        .effects
        .into_iter()
        .map(|effect| match effect {
            DockHostEffect::OpenFloating(floating) => WindowCommand::Open {
                id: WindowId::from(floating.surface),
                settings: WindowSettings {
                    title: title.clone(),
                    initial_size: (
                        f64::from(floating.bounds.width),
                        f64::from(floating.bounds.height),
                    ),
                    minimum_size: (160.0, 120.0),
                    initial_position: Some((
                        f64::from(floating.bounds.x),
                        f64::from(floating.bounds.y),
                    )),
                    maximized: false,
                    transparent: false,
                    always_on_top: false,
                    resizable: true,
                    role: WindowRole::Tool,
                    modal: false,
                    parent: None,
                    // Match the main window: client-drawn chrome, not OS caption.
                    system_caption: false,
                },
            },
            DockHostEffect::CloseFloating(surface) => WindowCommand::Close(WindowId::from(surface)),
            DockHostEffect::MoveFloating { surface, bounds } => WindowCommand::SetBounds {
                id: WindowId::from(surface),
                position: (bounds.x, bounds.y),
                size: (bounds.width, bounds.height),
            },
            DockHostEffect::FocusFloating(surface) => WindowCommand::Focus(WindowId::from(surface)),
        })
        .collect();
    RuntimeProgramUpdate {
        redraw,
        window_commands,
        exit: false,
    }
}

fn redraw_for_dock_effects(effects: &[DockHostEffect]) -> RuntimeRedraw {
    let mut redraw = RuntimeRedraw::None;
    for effect in effects {
        let next = match effect {
            DockHostEffect::OpenFloating(_) | DockHostEffect::CloseFloating(_) => {
                RuntimeRedraw::All
            }
            DockHostEffect::MoveFloating { surface, .. }
            | DockHostEffect::FocusFloating(surface) => {
                RuntimeRedraw::Window(WindowId::from(*surface))
            }
        };
        redraw = merge_dock_redraw(redraw, next);
    }
    redraw
}

fn merge_dock_redraw(left: RuntimeRedraw, right: RuntimeRedraw) -> RuntimeRedraw {
    match (left, right) {
        (RuntimeRedraw::All, _) | (_, RuntimeRedraw::All) => RuntimeRedraw::All,
        (RuntimeRedraw::None, redraw) | (redraw, RuntimeRedraw::None) => redraw,
        (RuntimeRedraw::Window(a), RuntimeRedraw::Window(b)) if a == b => RuntimeRedraw::Window(a),
        _ => RuntimeRedraw::All,
    }
}


