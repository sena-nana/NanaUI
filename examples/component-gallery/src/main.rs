use component_gallery::GalleryApp;
use nana_ui::{FileStore, WindowDescriptor, app_data_dir, run_runtime_with_store, shared_store};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let settings = WindowDescriptor::new("NanaUI Gallery")
        .initial_size(1280.0, 800.0)
        .minimum_size(960.0, 640.0)
        .persist_key("main");
    let store = match app_data_dir("NanaUI/Gallery") {
        Some(dir) => shared_store(FileStore::open(dir)?),
        None => nana_ui::memory_store(),
    };
    run_runtime_with_store::<GalleryApp>(settings, store)?;
    Ok(())
}
