use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use crate::mark::rasterize;

pub fn png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(format!(
            "png rgba length {} does not match {width}x{height}",
            rgba.len()
        ));
    }
    let mut raw = Vec::with_capacity((width as usize + 1) * height as usize * 4);
    for row in rgba.chunks_exact(width as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut deflated = Vec::new();
    {
        let mut encoder = ZlibEncoder::new(&mut deflated, Compression::best());
        encoder
            .write_all(&raw)
            .map_err(|error| format!("png deflate failed: {error}"))?;
        encoder
            .finish()
            .map_err(|error| format!("png deflate finish failed: {error}"))?;
    }
    let mut out = Vec::from(*b"\x89PNG\r\n\x1a\n");
    write_chunk(&mut out, *b"IHDR", &ihdr(width, height));
    write_chunk(&mut out, *b"IDAT", &deflated);
    write_chunk(&mut out, *b"IEND", &[]);
    Ok(out)
}

#[cfg(any(test, feature = "embed"))]
pub fn ico() -> Result<Vec<u8>, String> {
    let images = [16, 32, 48, 256]
        .into_iter()
        .map(|size| png(size, size, &rasterize(size)).map(|bytes| (size, bytes)))
        .collect::<Result<Vec<_>, _>>()?;
    let count = u16::try_from(images.len()).map_err(|_| "too many ico images".to_string())?;
    let mut data = Vec::new();
    data.extend_from_slice(&0u16.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, png) in &images {
        let dim = if *size >= 256 { 0_u8 } else { *size as u8 };
        data.push(dim);
        data.push(dim);
        data.push(0);
        data.push(0);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&32u16.to_le_bytes());
        data.extend_from_slice(&(png.len() as u32).to_le_bytes());
        data.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in &images {
        data.extend_from_slice(png);
    }
    Ok(data)
}

pub fn icns() -> Result<Vec<u8>, String> {
    const ICONS: [([u8; 4], u32); 5] = [
        (*b"icp4", 16),
        (*b"icp5", 32),
        (*b"ic07", 128),
        (*b"ic08", 256),
        (*b"ic09", 512),
    ];
    let mut payload = Vec::new();
    for (kind, size) in ICONS {
        let png = png(size, size, &rasterize(size))?;
        let len = 8u32
            .checked_add(png.len() as u32)
            .ok_or_else(|| "icns entry too large".to_string())?;
        payload.extend_from_slice(&kind);
        payload.extend_from_slice(&len.to_be_bytes());
        payload.extend_from_slice(&png);
    }
    let total = 8u32
        .checked_add(payload.len() as u32)
        .ok_or_else(|| "icns file too large".to_string())?;
    let mut data = Vec::from(*b"icns");
    data.extend_from_slice(&total.to_be_bytes());
    data.extend_from_slice(&payload);
    Ok(data)
}

fn ihdr(width: u32, height: u32) -> [u8; 13] {
    let mut data = [0_u8; 13];
    data[0..4].copy_from_slice(&width.to_be_bytes());
    data[4..8].copy_from_slice(&height.to_be_bytes());
    data[8] = 8;
    data[9] = 6;
    data
}

fn write_chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(&kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_containers_have_expected_headers() {
        let png = png(1, 1, &[73, 145, 215, 255]).expect("png");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let ico = ico().expect("ico");
        assert_eq!(&ico[2..4], &1u16.to_le_bytes());
        assert_eq!(&ico[4..6], &4u16.to_le_bytes());
        let icns = icns().expect("icns");
        assert_eq!(&icns[..4], b"icns");
        assert!(icns.windows(4).any(|chunk| chunk == b"ic08"));
    }
}
