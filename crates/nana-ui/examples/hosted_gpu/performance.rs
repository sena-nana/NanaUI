use std::time::Instant;

use nana_window::MaterialEffect;

pub struct StartupProbe {
    started_at: Instant,
    measure_first_frame: bool,
    recorded: bool,
}

impl StartupProbe {
    pub fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            measure_first_frame: std::env::args_os()
                .any(|argument| argument == "--measure-first-frame"),
            recorded: false,
        }
    }

    pub fn record_first_frame(&mut self, material: MaterialEffect) -> bool {
        if !self.measure_first_frame || self.recorded {
            return false;
        }
        self.recorded = true;
        println!(
            "{{\"first_frame_ms\":{:.3},\"material\":\"{}\"}}",
            self.started_at.elapsed().as_secs_f64() * 1_000.0,
            material_name(material),
        );
        true
    }
}

const fn material_name(material: MaterialEffect) -> &'static str {
    match material {
        MaterialEffect::Solid => "solid",
        MaterialEffect::Transparent => "transparent",
        MaterialEffect::Vibrancy => "vibrancy",
        MaterialEffect::Mica => "mica",
        MaterialEffect::Acrylic => "acrylic",
    }
}
