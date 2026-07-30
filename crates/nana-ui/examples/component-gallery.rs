use iced::window;
use nana_ui::GalleryState;

fn main() -> iced::Result {
    iced::application(GalleryState::new, GalleryState::update, GalleryState::view)
        .title("NanaUI Component Gallery")
        .theme(|state: &GalleryState| state.theme_mode().iced_theme())
        .subscription(GalleryState::subscription)
        .window(window::Settings {
            size: iced::Size::new(1180.0, 760.0),
            min_size: Some(iced::Size::new(900.0, 620.0)),
            ..window::Settings::default()
        })
        .centered()
        .run()
}
