//! Draw the NanaUI control strip through [`SceneWgpuPainter`].
//!
//! Uses the host-owned wgpu Device/Queue. Layout is a bottom-aligned Runtime
//! strip on the **full window viewport** — **not** [`nana_ui::DesktopShell`].
//! Hit-testing must use [`crate::control_slot::control_slot_paint_bounds`].
//! Pointer + KeyEvent input is applied through [`crate::slot_runtime::SlotRuntime`].
//! The soft keyboard is shown/hidden from [`Self::text_input_focused`]; committed
//! text arrives as hardware-style KeyEvents (NativeActivity has no InputConnection,
//! so no composition/preedit). AX stays unimplemented.

use nana_ui::{ScenePaintViewport, SceneWgpuPainter};
use nana_ui_core::PhysicalRect;
use wgpu::{CommandEncoder, Device, Queue, TextureFormat, TextureView};

use crate::slot_input::{SlotKeyMods, SlotLogicalKey, SlotTouchKind};
use crate::slot_runtime::{SlotRuntime, SlotSnapshot};

/// Retained Scene painter for the Primary control-slot strip.
pub struct SlotPainter {
    runtime: SlotRuntime,
    painter: SceneWgpuPainter,
}

impl SlotPainter {
    pub fn new(
        device: &Device,
        queue: &Queue,
        format: TextureFormat,
        physical_size: (u32, u32),
        scale: f32,
    ) -> Result<Self, String> {
        Ok(Self {
            runtime: SlotRuntime::new(physical_size, scale)
                .map_err(|error| format!("slot runtime: {error}"))?,
            painter: SceneWgpuPainter::new(device, queue, format),
        })
    }

    pub fn resize(&mut self, physical_size: (u32, u32), scale: f32) {
        self.runtime.resize(physical_size, scale);
    }

    pub fn push_touch(
        &mut self,
        slot: Option<PhysicalRect>,
        kind: SlotTouchKind,
        physical_x: f32,
        physical_y: f32,
        pointer_id: i32,
    ) -> bool {
        let before = self.runtime.snapshot();
        match self
            .runtime
            .push_touch(slot, kind, physical_x, physical_y, pointer_id)
        {
            Ok(handled) => {
                if handled {
                    self.log_changes(before);
                }
                handled
            }
            Err(error) => {
                log::warn!("nana-android-host: slot pointer: {error}");
                false
            }
        }
    }

    pub fn push_key(
        &mut self,
        down: bool,
        key: Option<SlotLogicalKey>,
        mods: SlotKeyMods,
        repeat: bool,
    ) -> bool {
        let before = self.runtime.snapshot();
        match self.runtime.push_key(down, key, mods, repeat) {
            Ok(handled) => {
                if handled {
                    self.log_changes(before);
                }
                handled
            }
            Err(error) => {
                log::warn!("nana-android-host: slot key: {error}");
                false
            }
        }
    }

    /// Whether the Runtime keyboard focus sits on the slot's text input.
    ///
    /// The Android activity loop mirrors this into the soft keyboard.
    pub fn text_input_focused(&self) -> bool {
        self.runtime.text_input_focused()
    }

    /// Retained Runtime document backing the strip (for accessibility
    /// publication and host-side focus mirrors).
    pub(crate) fn runtime(&self) -> &SlotRuntime {
        &self.runtime
    }

    /// Draw the flushed Runtime scene over chrome already encoded on `view`.
    pub fn paint_slot(&mut self, encoder: &mut CommandEncoder, view: &TextureView) {
        if let Err(error) = self.runtime.flush() {
            log::warn!("nana-android-host: slot flush: {error}");
            return;
        }
        let (logical_w, logical_h) = self.runtime.logical_size();
        let (fw, fh) = self.runtime.physical_size();
        let viewport = ScenePaintViewport {
            logical_size: [logical_w, logical_h],
            physical_size: [fw, fh],
            scale_factor: self.runtime.scale(),
            scene_origin: [0.0, 0.0],
            target_origin: [0.0, 0.0],
            clear_color: [0.0, 0.0, 0.0, 0.0],
            clear: false,
        };
        if let Err(error) = self.painter.paint(
            self.runtime.document().scene(),
            encoder,
            view,
            viewport,
            None,
            None,
        ) {
            log::warn!("nana-android-host: slot paint: {error}");
        }
    }

    fn log_changes(&self, before: SlotSnapshot) {
        let after = self.runtime.snapshot();
        if after.press_count != before.press_count {
            log::info!(
                "nana-android-host: slot Button pressed count={} in_slot={}",
                after.press_count,
                self.runtime.last_touch_in_slot()
            );
        }
        if after.switch_on != before.switch_on {
            log::info!(
                "nana-android-host: slot Switch toggled on={} in_slot={}",
                after.switch_on,
                self.runtime.last_touch_in_slot()
            );
        }
        if after.input_len != before.input_len {
            log::info!(
                "nana-android-host: slot Input len={} in_slot={}",
                after.input_len,
                self.runtime.last_touch_in_slot()
            );
        }
    }
}
