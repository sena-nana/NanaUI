//! The probe page this crate's own APK shows: the measurement page shared
//! with the iOS host, and its CPU preview stand-in.

use std::time::{Duration, Instant};

use nana_mobile_host::{FrameDemand, MobileRenderer, MobileRuntime};
use nana_mobile_probe::{PreviewUploader, Probe, ProbeState};
use nana_ui::runtime::FrameworkError;

use crate::host::{AndroidHostApp, Shell};

/// The CPU preview stand-in: Android has no camera in the prototype, so a
/// generated 640×480 frame is uploaded at the camera's 30 fps to measure the
/// upload path the iOS host compares against its native layer.
const PREVIEW_SIZE: (u32, u32) = (640, 480);
const PREVIEW_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Default)]
pub struct ProbeApp {
    probe: Option<Probe>,
    preview: PreviewUploader,
    next_preview: Option<Instant>,
    preview_phase: u8,
}

impl ProbeApp {
    fn cpu_preview(&self) -> bool {
        self.probe
            .as_ref()
            .is_some_and(|probe| probe.state.cpu_preview())
    }
}

impl AndroidHostApp for ProbeApp {
    fn mount(&mut self, runtime: &mut MobileRuntime, _shell: Shell) -> Result<(), FrameworkError> {
        self.probe = Some(Probe::mount(runtime, ProbeState::default())?);
        Ok(())
    }

    fn before_frame(&mut self, runtime: &mut MobileRuntime, renderer: &mut MobileRenderer) {
        let now = Instant::now();
        if self.cpu_preview() && self.next_preview.is_none_or(|due| now >= due) {
            self.next_preview = Some(now + PREVIEW_INTERVAL);
            self.preview_phase = self.preview_phase.wrapping_add(3);
            let (width, height) = PREVIEW_SIZE;
            let shade = self.preview_phase;
            let frame: Vec<u8> = (0..width * height)
                .flat_map(|index| {
                    let x = (index % width) as u8;
                    [x.wrapping_add(shade), shade, 160, 255]
                })
                .collect();
            let uploaded = self.preview.push_bgra(
                renderer.gpu(),
                renderer.host_textures(),
                &frame,
                width,
                height,
                width * 4,
            );
            if uploaded.is_ok() {
                runtime.request_present();
            }
        }
        if let Some(probe) = self.probe.as_ref() {
            probe.sync_preview(&mut self.preview, renderer.host_textures());
        }
    }

    fn demand(&self) -> FrameDemand {
        match self.next_preview {
            Some(due) if self.cpu_preview() => FrameDemand::At(due),
            _ if self.cpu_preview() => FrameDemand::Now,
            _ => FrameDemand::Idle,
        }
    }
}
