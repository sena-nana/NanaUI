use iced::window;
use nana_ui::WorkspaceState;

fn main() -> iced::Result {
    iced::application(
        WorkspaceState::new,
        WorkspaceState::update,
        WorkspaceState::view,
    )
    .title("NanaUI Workspace Demo")
    .theme(|state: &WorkspaceState| state.theme_mode().iced_theme())
    .subscription(WorkspaceState::subscription)
    .window(window::Settings {
        size: iced::Size::new(1440.0, 900.0),
        min_size: Some(iced::Size::new(960.0, 640.0)),
        ..window::Settings::default()
    })
    .centered()
    .run()
}
