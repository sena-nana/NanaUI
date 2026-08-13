//! Draw a Nana Iced **control strip** (Icon + Text + Input + Switch + Button).
//!
//! Uses host-owned wgpu Device/Queue via `iced_wgpu` (`Shell::headless`). Layout is a
//! bottom-aligned row on the **full window viewport** — **not** [`nana_ui::DesktopShell`].
//! Hit-testing must use [`crate::iced_slot::iced_control_slot_paint_bounds`] (same geometry).
//! Pointer + KeyEvent input is applied through iced `UserInterface::update` with
//! [`iced::window::Headless`]. Soft IME (`UnsupportedIme`) is still open; hardware /
//! soft-keyboard `KeyEvent` characters reach the focused Input.

use iced::keyboard::{self, Modifiers};
use iced::widget::{Space, column, container};
use iced::{Alignment, Color, Element, Length, Pixels, Size, Theme, mouse, window};
use iced_wgpu::graphics::core::renderer;
use iced_wgpu::graphics::core::{Event, shell};
use iced_wgpu::graphics::{Antialiasing, Shell, Viewport};
use iced_wgpu::{Engine, Renderer};
use iced_winit::runtime::user_interface::{self, UserInterface};
use nana_ui::{ThemeMode, ThemeModeExt};
use nana_ui_core::PhysicalRect;
use wgpu::{Adapter, Device, Queue, TextureFormat, TextureView};

use crate::iced_control::{SLOT_BUTTON_LABEL, SlotStripMessage, slot_strip_element};
use crate::iced_slot_input::{
    SlotInputGate, SlotKeyMods, SlotLogicalKey, SlotTouchKind, cursor_after, key_to_iced_events,
    logical_point, pointer_in_slot, touch_to_mouse_events,
};

/// Retained iced_wgpu renderer for the Primary control-slot strip.
pub struct IcedSlotPainter {
    renderer: Renderer,
    viewport: Viewport,
    scale: f32,
    cache: user_interface::Cache,
    events: Vec<Event>,
    cursor: mouse::Cursor,
    /// Successful Button presses delivered by iced.
    pub press_count: u32,
    /// Switch toggled state (shell strip).
    pub switch_on: bool,
    /// Editable field value for the slot [`nana_ui::Input`].
    pub input_value: String,
    /// Last touch was inside the control-slot geometry (diagnostics).
    last_touch_in_slot: bool,
    /// Slot-only pointer capture + keyboard focus gate.
    input_gate: SlotInputGate,
    /// Last iced modifiers pushed to the event queue.
    last_mods: Modifiers,
}

impl IcedSlotPainter {
    pub fn new(
        adapter: &Adapter,
        device: &Device,
        queue: &Queue,
        format: TextureFormat,
        physical_size: (u32, u32),
        scale: f32,
    ) -> Self {
        let scale = scale.max(0.25);
        let renderer = Renderer::new(
            Engine::new(
                adapter,
                device.clone(),
                queue.clone(),
                format,
                Some(Antialiasing::MSAAx4),
                Shell::headless(),
            ),
            renderer::Settings {
                default_font: nana_ui::ui_font(iced::font::Weight::Normal),
                default_text_size: Pixels::from(nana_ui::UI_BASE_TEXT_SIZE),
                metrics_hinting: true,
            },
        );
        Self {
            renderer,
            viewport: viewport_for(physical_size, scale),
            scale,
            cache: user_interface::Cache::new(),
            events: Vec::new(),
            cursor: mouse::Cursor::Unavailable,
            press_count: 0,
            switch_on: false,
            input_value: String::new(),
            last_touch_in_slot: false,
            input_gate: SlotInputGate::default(),
            last_mods: Modifiers::empty(),
        }
    }

    pub fn resize(&mut self, physical_size: (u32, u32), scale: f32) {
        self.scale = scale.max(0.25);
        self.viewport = viewport_for(physical_size, self.scale);
    }

    /// Queue a physical pointer sample (Android MotionEvent coords + pointer id).
    ///
    /// Returns `false` when the sample is outside the iced slot (and not part of
    /// an in-slot drag) so the host can leave it `Unhandled` for VueHost.
    pub fn push_touch(
        &mut self,
        slot: Option<PhysicalRect>,
        kind: SlotTouchKind,
        physical_x: f32,
        physical_y: f32,
        pointer_id: i32,
    ) -> bool {
        self.last_touch_in_slot = slot
            .map(|rect| pointer_in_slot(rect, physical_x, physical_y))
            .unwrap_or(false);
        if !self
            .input_gate
            .accept_pointer(slot, kind, physical_x, physical_y, pointer_id)
        {
            return false;
        }
        let logical = logical_point(physical_x, physical_y, self.scale);
        self.cursor = cursor_after(kind, logical);
        self.events.extend(touch_to_mouse_events(kind, logical));
        true
    }

    /// Queue a keyboard sample (Android KeyEvent → iced).
    ///
    /// Returns `false` when the slot does not hold keyboard focus so the host
    /// does not swallow whole-window keys. When `key` is `None`, only modifier
    /// state is synced (Shift/Ctrl/…).
    pub fn push_key(
        &mut self,
        down: bool,
        key: Option<SlotLogicalKey>,
        mods: SlotKeyMods,
        repeat: bool,
    ) -> bool {
        if !self.input_gate.accept_key() {
            return false;
        }
        let iced_mods = mods.to_iced();
        if self.last_mods != iced_mods {
            self.events
                .push(Event::Keyboard(keyboard::Event::ModifiersChanged(
                    iced_mods,
                )));
            self.last_mods = iced_mods;
        }
        if let Some(key) = key {
            self.events
                .extend(key_to_iced_events(down, key, iced_mods, repeat));
        }
        true
    }

    fn slot_content(&self) -> Element<'static, SlotStripMessage> {
        let tokens = ThemeMode::Dark.tokens();
        let button_label = if self.press_count == 0 {
            SLOT_BUTTON_LABEL.to_string()
        } else {
            format!("{SLOT_BUTTON_LABEL} · {}", self.press_count)
        };
        // Own the field text so UserInterface does not borrow `self`.
        let strip = slot_strip_element(
            self.switch_on,
            self.input_value.clone(),
            button_label,
            tokens,
        );
        column![
            Space::new().height(Length::Fill),
            container(strip)
                .padding(12)
                .width(Length::Fill)
                .align_x(Alignment::Center),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn apply_messages(&mut self, messages: &[SlotStripMessage]) -> bool {
        let mut dirty = false;
        for msg in messages {
            match msg {
                SlotStripMessage::Pressed => {
                    self.press_count = self.press_count.saturating_add(1);
                    dirty = true;
                    log::info!(
                        "nana-android-host: iced slot Button pressed count={} in_slot={}",
                        self.press_count,
                        self.last_touch_in_slot
                    );
                }
                SlotStripMessage::Toggle(on) => {
                    if self.switch_on != *on {
                        self.switch_on = *on;
                        dirty = true;
                        log::info!(
                            "nana-android-host: iced slot Switch toggled on={} in_slot={}",
                            self.switch_on,
                            self.last_touch_in_slot
                        );
                    }
                }
                SlotStripMessage::Input(value) => {
                    if self.input_value != *value {
                        self.input_value = value.clone();
                        dirty = true;
                        log::info!(
                            "nana-android-host: iced slot Input len={} in_slot={}",
                            self.input_value.len(),
                            self.last_touch_in_slot
                        );
                    }
                }
            }
        }
        dirty
    }

    /// Draw the slot strip; apply queued pointer / key events via iced update.
    pub fn paint_slot_button(&mut self, view: &TextureView, format: TextureFormat) {
        let cache = std::mem::replace(&mut self.cache, user_interface::Cache::new());
        let mut ui = UserInterface::build(
            self.slot_content(),
            self.viewport.logical_size(),
            cache,
            &mut self.renderer,
        );

        let events = std::mem::take(&mut self.events);
        if !events.is_empty() {
            let waker = shell::Waker::noop();
            let mut messages = shell::Bus::new();
            let (_state, _statuses) = ui.update(
                &window::Headless,
                &waker,
                &events,
                self.cursor,
                &mut self.renderer,
                &mut messages,
            );
            let messages = messages.into_iter().collect::<Vec<_>>();
            if self.apply_messages(&messages) {
                let cache = ui.into_cache();
                ui = UserInterface::build(
                    self.slot_content(),
                    self.viewport.logical_size(),
                    cache,
                    &mut self.renderer,
                );
            }
        }

        let style = renderer::Style {
            text_color: Color::from_rgb(0.92, 0.93, 0.96),
        };
        ui.draw(&mut self.renderer, &Theme::Dark, &style, self.cursor);
        let _submission = self.renderer.present(None, format, view, &self.viewport);
        self.cache = ui.into_cache();
        log::trace!(
            "nana-android-host: iced slot strip painted presses={} switch={} input_len={}",
            self.press_count,
            self.switch_on,
            self.input_value.len()
        );
    }
}

fn viewport_for(physical_size: (u32, u32), scale: f32) -> Viewport {
    Viewport::with_physical_size(
        Size::new(physical_size.0.max(1), physical_size.1.max(1)),
        renderer::Scale {
            window: scale.max(0.25),
            application: 1.0,
        },
    )
}
