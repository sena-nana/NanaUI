use iced::window;
use nana_ui::{GalleryState, custom_title_bar_window, ui_font, ui_font_sources};

fn main() -> iced::Result {
    let mut application = iced::application(
        GalleryState::new,
        GalleryState::update_windowed,
        GalleryState::view,
    )
    .title("NanaUI Component Gallery")
    .theme(|state: &GalleryState| state.theme_mode().iced_theme())
    .default_font(ui_font(iced::font::Weight::Normal))
    .subscription(GalleryState::subscription)
    .window(custom_title_bar_window(window::Settings {
        size: iced::Size::new(1180.0, 760.0),
        min_size: Some(iced::Size::new(900.0, 620.0)),
        ..window::Settings::default()
    }))
    .centered();
    for source in ui_font_sources() {
        application = application.font(source);
    }
    application.run()
}
