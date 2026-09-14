use component_gallery::GalleryApp;
use nana_ui::{WindowDescriptor, run_runtime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let settings = WindowDescriptor::new("NanaUI Gallery")
        .initial_size(1280.0, 800.0)
        .minimum_size(960.0, 640.0);
    run_runtime::<GalleryApp>(settings)?;
    Ok(())
}
