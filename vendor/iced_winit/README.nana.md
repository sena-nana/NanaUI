# NanaUI iced_winit vendor

Vendored from `sena-nana/iced` rev `a3bdc6f5…` (`winit/` crate) with Android fixes:

1. `conversion.rs` — skip `winit::platform::modifier_supplement` on Android
2. `clipboard.rs` + `Cargo.toml` — do not link `arboard` on Android (unavailable)

Patched via root `Cargo.toml` `[patch."https://github.com/sena-nana/iced.git"]`.
Keep API aligned with the workspace iced rev.
