use nana_ui::{SemanticPalette, ThemeAppearance};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Message {
    Refresh,
    ToggleTheme,
}

#[derive(Debug, Default)]
pub struct DemoPanel {
    theme: ThemeAppearance,
    revision: u32,
}

impl DemoPanel {
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Refresh => {
                self.revision = self.revision.saturating_add(1);
            }
            Message::ToggleTheme => {
                self.theme = self.theme.toggle();
            }
        }
    }

    pub fn palette(&self) -> SemanticPalette {
        self.theme.palette()
    }

    pub fn revision(&self) -> u32 {
        self.revision
    }

    pub fn preset_theme(&self) -> ThemeAppearance {
        self.theme
    }

    pub fn theme_label(&self) -> &'static str {
        if self.theme == ThemeAppearance::Dark {
            "浅色"
        } else {
            "深色"
        }
    }

    pub fn version_label(&self) -> String {
        format!("预览版本 {}", self.revision + 1)
    }
}
