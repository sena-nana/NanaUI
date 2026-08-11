use component_gallery::GalleryState;
use iced::window;
use nana_ui::{ThemeModeExt, custom_title_bar_window, ui_font, ui_font_defaults, ui_font_sources};

fn main() -> iced::Result {
    let mut application = iced::application(
        || (GalleryState::new(), ui_font_defaults()),
        GalleryState::update_windowed,
        GalleryState::view,
    )
    .title("NanaUI Gallery")
    .theme(|state: &GalleryState| state.theme_mode().iced_theme())
    .default_font(ui_font(iced::font::Weight::Normal))
    .subscription(GalleryState::subscription)
    .transparent(true)
    .window(custom_title_bar_window(window::Settings {
        size: iced::Size::new(1280.0, 800.0),
        min_size: Some(iced::Size::new(960.0, 640.0)),
        ..window::Settings::default()
    }))
    .centered();
    for source in ui_font_sources() {
        application = application.font(source);
    }
    application.run()
}
