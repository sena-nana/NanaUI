use component_gallery::GalleryApp;
use nana_ui::{RuntimeWindowSettings, run_runtime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut settings = RuntimeWindowSettings::new("NanaUI Gallery")
        .initial_size(1280.0, 800.0)
        .minimum_size(960.0, 640.0);
    settings.transparent = true;
    run_runtime::<GalleryApp>(settings)?;
    Ok(())
}
