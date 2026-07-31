use std::collections::BTreeMap;

use iced::widget::{button, column, container, text};
use iced::{Alignment, Element, Fill, Point, Subscription, Task, window};
use nana_ui::{
    AppTitleBar, ThemeMode, WindowChrome, WindowChromeEvent, WindowChromeState, ui_font,
    ui_font_defaults, ui_font_sources,
};

fn main() -> iced::Result {
    let mut application = iced::daemon(Smoke::new, Smoke::update, Smoke::view)
        .title(Smoke::title)
        .theme(Smoke::theme)
        .default_font(ui_font(iced::font::Weight::Normal))
        .subscription(Smoke::subscription);
    for source in ui_font_sources() {
        application = application.font(source);
    }
    application.run()
}

struct Smoke {
    windows: BTreeMap<window::Id, SmokeWindow>,
    next_window: usize,
}

struct SmokeWindow {
    number: usize,
    chrome: WindowChromeState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Message {
    OpenWindow,
    RuntimeChrome(WindowChromeEvent),
    WindowChrome(window::Id, WindowChromeEvent),
}

impl Smoke {
    fn new() -> (Self, Task<Message>) {
        let mut smoke = Self {
            windows: BTreeMap::new(),
            next_window: 1,
        };
        let first = smoke.open_window();
        let second = smoke.open_window();
        (smoke, Task::batch([first, second, ui_font_defaults()]))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenWindow => self.open_window(),
            Message::RuntimeChrome(event) => {
                if let WindowChromeEvent::WindowClosed(window) = event {
                    self.windows.remove(&window);
                }
                if self.windows.is_empty() {
                    return iced::exit();
                }
                Task::batch(self.windows.iter_mut().map(|(&window, state)| {
                    state
                        .chrome
                        .update_iced(event)
                        .map(move |event| Message::WindowChrome(window, event))
                }))
            }
            Message::WindowChrome(window, event) => {
                let Some(state) = self.windows.get_mut(&window) else {
                    return Task::none();
                };
                state
                    .chrome
                    .update_iced(event)
                    .map(move |event| Message::WindowChrome(window, event))
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        WindowChromeState::subscription().map(Message::RuntimeChrome)
    }

    fn view(&self, window: window::Id) -> Element<'_, Message> {
        let Some(state) = self.windows.get(&window) else {
            return iced::widget::space().into();
        };
        let colors = ThemeMode::Dark.colors();
        let title_bar = AppTitleBar::new("NanaUI Window", colors)
            .leading(text("NANA").size(12).color(colors.accent))
            .window_chrome(&state.chrome, move |event| {
                Message::WindowChrome(window, event)
            })
            .view();
        let body = container(
            column![
                text(format!("窗口 {}", state.number))
                    .size(24)
                    .color(colors.text),
                button(text("新建窗口")).on_press(Message::OpenWindow),
            ]
            .spacing(8)
            .align_x(Alignment::Center),
        )
        .center(Fill);

        container(column![title_bar, body])
            .width(Fill)
            .height(Fill)
            .style(move |_theme| {
                iced::widget::container::Style::default()
                    .background(colors.background)
                    .color(colors.text)
            })
            .into()
    }

    fn title(&self, window: window::Id) -> String {
        self.windows
            .get(&window)
            .map(|state| format!("NanaUI Window {}", state.number))
            .unwrap_or_default()
    }

    fn theme(&self, _window: window::Id) -> Option<iced::Theme> {
        Some(ThemeMode::Dark.iced_theme())
    }

    fn open_window(&mut self) -> Task<Message> {
        let number = self.next_window;
        self.next_window = self.next_window.saturating_add(1);
        let (window, open) = window::open(window_settings(number));
        self.windows
            .insert(window, SmokeWindow::new(window, number));
        open.discard()
    }
}

impl SmokeWindow {
    fn new(window: window::Id, number: usize) -> Self {
        Self {
            number,
            chrome: WindowChromeState::for_window(window, WindowChrome::custom()),
        }
    }
}

fn window_settings(number: usize) -> window::Settings {
    let offset = 64.0 * number.saturating_sub(1) as f32;
    window::Settings {
        size: iced::Size::new(640.0, 420.0),
        min_size: Some(iced::Size::new(480.0, 320.0)),
        position: window::Position::Specific(Point::new(120.0 + offset, 120.0 + offset)),
        decorations: false,
        ..window::Settings::default()
    }
}
