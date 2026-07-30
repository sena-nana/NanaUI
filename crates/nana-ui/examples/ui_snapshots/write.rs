use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;

use iced::Size;

pub fn png(path: &Path, size: Size<u32>, pixels: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), size.width, size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(pixels)?;
    writer.finish()?;
    Ok(())
}
