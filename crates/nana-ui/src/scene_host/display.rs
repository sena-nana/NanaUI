//! Display enumeration for the window host.

use nana_ui_platform::{DisplayId, DisplayInfo};
use winit::monitor::MonitorHandle;

use super::*;

pub(super) fn display_info(
    monitor: &MonitorHandle,
    primary: Option<&MonitorHandle>,
) -> DisplayInfo {
    let mode = monitor.current_video_mode();
    DisplayInfo {
        id: DisplayId(monitor.id()),
        name: monitor.name().map(|name| name.into_owned()),
        physical_position: monitor.position().map(|position| (position.x, position.y)),
        physical_size: mode.map(|mode| {
            let size = mode.size();
            (size.width, size.height)
        }),
        scale_factor: monitor.scale_factor(),
        refresh_rate_millihertz: mode.and_then(|mode| mode.refresh_rate_millihertz()),
        primary: primary == Some(monitor),
    }
}

pub(super) fn display_infos(event_loop: &dyn ActiveEventLoop) -> Vec<DisplayInfo> {
    let primary = event_loop.primary_monitor();
    event_loop
        .available_monitors()
        .map(|monitor| display_info(&monitor, primary.as_ref()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_bounds_are_logical_and_need_position_and_size() {
        let display = DisplayInfo {
            id: DisplayId(4),
            name: None,
            physical_position: Some((2880, 0)),
            physical_size: Some((3840, 2160)),
            scale_factor: 2.0,
            refresh_rate_millihertz: None,
            primary: false,
        };
        assert_eq!(
            display.logical_bounds(),
            Some(DisplayBounds {
                position: (1440.0, 0.0),
                size: (1920.0, 1080.0),
            })
        );
        let bounds = display.logical_bounds().unwrap();
        assert_eq!(
            clamp_position_to_displays((0.0, 40.0), (320.0, 240.0), &[bounds]),
            (1440.0, 40.0)
        );
        let unsized_display = DisplayInfo {
            physical_size: None,
            ..display
        };
        assert_eq!(unsized_display.logical_bounds(), None);
    }
}
