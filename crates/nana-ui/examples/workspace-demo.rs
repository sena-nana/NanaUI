use iced::window;
use nana_ui::{WorkspaceState, ui_font, ui_font_sources};

fn main() -> iced::Result {
    let mut application = iced::application(
        WorkspaceState::new,
        WorkspaceState::update,
        WorkspaceState::view,
    )
    .title("NanaUI Workspace Demo")
    .theme(|state: &WorkspaceState| state.theme_mode().iced_theme())
    .default_font(ui_font(iced::font::Weight::Normal))
    .subscription(WorkspaceState::subscription)
    .window(window::Settings {
        size: iced::Size::new(1440.0, 900.0),
        min_size: Some(iced::Size::new(960.0, 640.0)),
        ..window::Settings::default()
    })
    .centered();
    for source in ui_font_sources() {
        application = application.font(source);
    }
    application.run()
}
