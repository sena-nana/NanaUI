//! Three-platform native multi-format clipboard backend.
//!
//! Maps OS pasteboard / clipboard / selection types onto NanaUI's
//! [`DataTransfer`] contract (`text/plain`, `text/html`, `text/uri-list`,
//! `image/png`, `image/rgba8`, `application/octet-stream`, and validated custom
//! MIME types), supporting lazy on-demand format reads on desktop backends and
//! explicit capability/error reporting when a backend is unavailable.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::io::Cursor;

use nana_ui_platform::{
    ClipboardHost, DataFormat, DataTransfer, HostServiceError, SharedClipboardHost, TransferError,
    TransferImage, shared_clipboard,
};

#[allow(dead_code)]
pub(crate) const CUSTOM_UTI_PREFIX: &str = "org.nanaui.mime.v1.";
#[allow(dead_code)]
pub(crate) const CUSTOM_WIN32_PREFIX: &str = "NanaUI-MIME:";

/// Explicit capability report for the host platform's native clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardSupport {
    pub available: bool,
    pub backend: &'static str,
    pub supports_change_count: bool,
    pub supports_images: bool,
    pub supports_html: bool,
    pub supports_uri_list: bool,
    pub supports_custom_formats: bool,
    pub supports_lazy_read: bool,
    pub reason: Option<&'static str>,
}

impl ClipboardSupport {
    pub const fn unsupported(backend: &'static str, reason: &'static str) -> Self {
        Self {
            available: false,
            backend,
            supports_change_count: false,
            supports_images: false,
            supports_html: false,
            supports_uri_list: false,
            supports_custom_formats: false,
            supports_lazy_read: false,
            reason: Some(reason),
        }
    }
}

/// Inspect the native clipboard backend capabilities on this platform.
pub fn clipboard_support() -> ClipboardSupport {
    #[cfg(target_os = "macos")]
    {
        macos::support()
    }
    #[cfg(target_os = "windows")]
    {
        windows::support()
    }
    #[cfg(target_os = "linux")]
    {
        linux::support()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        ClipboardSupport::unsupported("none", "native desktop clipboard backend is not available")
    }
}

/// Native multi-format OS clipboard implementing [`ClipboardHost`].
#[derive(Debug, Default)]
pub struct NativeClipboard {
    _private: (),
}

impl NativeClipboard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn support(&self) -> ClipboardSupport {
        clipboard_support()
    }
}

impl ClipboardHost for NativeClipboard {
    fn is_available(&self) -> bool {
        clipboard_support().available
    }

    fn read_text(&mut self) -> Option<String> {
        let transfer = self.read_transfer().ok().flatten()?;
        if !transfer.has_format(&DataFormat::TextPlain) {
            return None;
        }
        transfer.read_text().ok().flatten()
    }

    fn write_text(&mut self, text: &str) -> bool {
        self.write_transfer(&DataTransfer::from_text(text)).is_ok()
    }

    fn change_count(&mut self) -> Option<u64> {
        #[cfg(target_os = "macos")]
        {
            macos::change_count()
        }
        #[cfg(target_os = "windows")]
        {
            windows::change_count()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            None
        }
    }

    fn read_transfer(&mut self) -> Result<Option<DataTransfer>, HostServiceError> {
        #[cfg(target_os = "macos")]
        {
            macos::read_transfer()
        }
        #[cfg(target_os = "windows")]
        {
            windows::read_transfer()
        }
        #[cfg(target_os = "linux")]
        {
            linux::read_transfer()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Err(HostServiceError::Unsupported)
        }
    }

    fn write_transfer(&mut self, transfer: &DataTransfer) -> Result<(), HostServiceError> {
        transfer
            .lifetime()
            .check()
            .map_err(|_| HostServiceError::Failed)?;
        if transfer.is_empty() {
            return Err(HostServiceError::Failed);
        }
        #[cfg(target_os = "macos")]
        {
            macos::write_transfer(transfer)
        }
        #[cfg(target_os = "windows")]
        {
            windows::write_transfer(transfer)
        }
        #[cfg(target_os = "linux")]
        {
            linux::write_transfer(transfer)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = transfer;
            Err(HostServiceError::Unsupported)
        }
    }
}

/// Construct the shared native [`ClipboardHost`] for desktop window hosts.
pub fn native_shared_clipboard() -> SharedClipboardHost {
    shared_clipboard(NativeClipboard::new())
}

pub(crate) fn encode_png(image: &TransferImage) -> Result<Vec<u8>, TransferError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|err| TransferError::ProviderFailed(err.to_string().into()))?;
        writer
            .write_image_data(&image.rgba)
            .map_err(|err| TransferError::ProviderFailed(err.to_string().into()))?;
    }
    Ok(out)
}

pub(crate) fn decode_png(bytes: &[u8], max_bytes: usize) -> Result<TransferImage, TransferError> {
    if bytes.len() > max_bytes {
        return Err(TransferError::PayloadTooLarge {
            size: bytes.len(),
            max_bytes,
        });
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA,
    );
    let mut reader = decoder
        .read_info()
        .map_err(|_| TransferError::InvalidImageData)?;
    let info = reader.info();
    let expected = (info.width as usize)
        .checked_mul(info.height as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or(TransferError::InvalidImageData)?;
    if expected > max_bytes {
        return Err(TransferError::PayloadTooLarge {
            size: expected,
            max_bytes,
        });
    }
    let out_len = reader
        .output_buffer_size()
        .ok_or(TransferError::InvalidImageData)?;
    if out_len > max_bytes {
        return Err(TransferError::PayloadTooLarge {
            size: out_len,
            max_bytes,
        });
    }
    let mut buf = vec![0u8; out_len];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|_| TransferError::InvalidImageData)?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return Err(TransferError::InvalidImageData);
    }
    buf.truncate(frame.buffer_size());
    TransferImage::new(frame.width, frame.height, buf)
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) fn custom_mime_to_uti(mime: &str) -> String {
    let mut out = String::with_capacity(CUSTOM_UTI_PREFIX.len() + mime.len() * 2);
    out.push_str(CUSTOM_UTI_PREFIX);
    for &byte in mime.as_bytes() {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xF) as usize] as char);
    }
    out
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) fn uti_to_custom_mime(uti: &str) -> Option<DataFormat> {
    let hex = uti.strip_prefix(CUSTOM_UTI_PREFIX)?;
    if hex.is_empty() || !hex.len().is_multiple_of(2) {
        return None;
    }
    let bytes = hex.as_bytes();
    let mut raw = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = from_hex_digit(pair[0])?;
        let lo = from_hex_digit(pair[1])?;
        raw.push((hi << 4) | lo);
    }
    let mime = std::str::from_utf8(&raw).ok()?;
    DataFormat::parse(mime).ok()
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn from_hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[allow(dead_code)]
pub(crate) fn custom_mime_to_win32_format_name(mime: &str) -> String {
    format!("{CUSTOM_WIN32_PREFIX}{mime}")
}

#[allow(dead_code)]
pub(crate) fn win32_format_name_to_custom_mime(name: &str) -> Option<DataFormat> {
    let mime = name.strip_prefix(CUSTOM_WIN32_PREFIX)?;
    DataFormat::parse(mime).ok()
}

#[allow(dead_code)]
pub(crate) fn encode_utf16_nul(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[allow(dead_code)]
pub(crate) fn decode_utf16_nul(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// Frame UTF-8 HTML into Windows `CF_HTML` clipboard format.
#[allow(dead_code)]
pub(crate) fn encode_cf_html(html: &str) -> Vec<u8> {
    const HEADER_TEMPLATE: &str = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    const PREFIX: &str = "<html><body><!--StartFragment-->";
    const SUFFIX: &str = "<!--EndFragment--></body></html>";

    let start_html = HEADER_TEMPLATE.len();
    let start_fragment = start_html + PREFIX.len();
    let end_fragment = start_fragment + html.len();
    let end_html = end_fragment + SUFFIX.len();

    let header = format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n"
    );
    let mut out = Vec::with_capacity(end_html + 1);
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(PREFIX.as_bytes());
    out.extend_from_slice(html.as_bytes());
    out.extend_from_slice(SUFFIX.as_bytes());
    out.push(0);
    out
}

/// Extract HTML fragment/body from Windows `CF_HTML` clipboard bytes.
#[allow(dead_code)]
pub(crate) fn decode_cf_html(raw: &[u8]) -> Option<String> {
    let raw = raw.split(|&b| b == 0).next().unwrap_or(raw);
    let text = std::str::from_utf8(raw).ok()?;
    let parse_offset = |key: &str| -> Option<usize> {
        text.lines()
            .find_map(|line| line.trim_end_matches('\r').strip_prefix(key))
            .and_then(|val| val.trim().parse::<usize>().ok())
    };
    if let (Some(start), Some(end)) = (parse_offset("StartFragment:"), parse_offset("EndFragment:"))
        && start <= end
        && end <= text.len()
        && text.is_char_boundary(start)
        && text.is_char_boundary(end)
    {
        return Some(text[start..end].to_owned());
    }
    if let (Some(start), Some(end)) = (parse_offset("StartHTML:"), parse_offset("EndHTML:"))
        && start <= end
        && end <= text.len()
        && text.is_char_boundary(start)
        && text.is_char_boundary(end)
    {
        return Some(text[start..end].to_owned());
    }
    None
}

#[allow(dead_code)]
pub(crate) fn parse_linux_targets(raw: &str) -> Vec<DataFormat> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let candidate = match trimmed {
            "UTF8_STRING" | "STRING" | "TEXT" | "text/plain;charset=utf-8" => {
                Some(DataFormat::TextPlain)
            }
            "text/html;charset=utf-8" => Some(DataFormat::TextHtml),
            other => DataFormat::parse(other).ok(),
        };
        if let Some(fmt) = candidate
            && !out.contains(&fmt)
        {
            out.push(fmt);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_and_rgba8_round_trip_and_reject_oversized_images() {
        let image = TransferImage::new(
            2,
            2,
            [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 1, 2, 3, 4].as_slice(),
        )
        .unwrap();
        let encoded = encode_png(&image).unwrap();
        let decoded = decode_png(&encoded, DataTransfer::DEFAULT_MAX_BYTES).unwrap();
        assert_eq!(decoded, image);
        assert_eq!(
            decode_png(&encoded, 4),
            Err(TransferError::PayloadTooLarge {
                size: encoded.len(),
                max_bytes: 4
            })
        );
    }

    #[test]
    fn cf_html_and_custom_format_mappings_round_trip() {
        let cf = encode_cf_html("<p>Hi &amp; Bye</p>");
        assert_eq!(decode_cf_html(&cf).as_deref(), Some("<p>Hi &amp; Bye</p>"));

        let utf16 = encode_utf16_nul("NanaUI 剪贴板");
        assert_eq!(decode_utf16_nul(&utf16), "NanaUI 剪贴板");

        let uti = custom_mime_to_uti("application/x-nana-block");
        assert_eq!(
            uti_to_custom_mime(&uti),
            Some(DataFormat::custom("application/x-nana-block").unwrap())
        );
        assert_eq!(uti_to_custom_mime("public.utf8-plain-text"), None);

        let win_name = custom_mime_to_win32_format_name("application/x-nana-block");
        assert_eq!(
            win32_format_name_to_custom_mime(&win_name),
            Some(DataFormat::custom("application/x-nana-block").unwrap())
        );
    }

    #[test]
    fn linux_targets_parser_maps_well_known_and_validated_custom_types() {
        let parsed = parse_linux_targets(
            "TARGETS\nUTF8_STRING\ntext/plain;charset=utf-8\nimage/png\napplication/x-nana-node\napplication/x-raw-pointer\n",
        );
        assert_eq!(
            parsed,
            vec![
                DataFormat::TextPlain,
                DataFormat::ImagePng,
                DataFormat::custom("application/x-nana-node").unwrap(),
            ]
        );
    }

    #[test]
    #[ignore = "mutates live OS desktop clipboard; run explicitly with --ignored"]
    fn native_clipboard_multiformat_and_lazy_roundtrip() {
        let mut clip = NativeClipboard::new();
        if !clip.is_available() {
            return;
        }
        let previous = clip.read_transfer().ok().flatten();

        let custom = DataFormat::custom("application/x-nana-test").unwrap();
        let image = TransferImage::new(
            2,
            2,
            [
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
            ]
            .as_slice(),
        )
        .unwrap();
        let png_bytes = encode_png(&image).unwrap();

        let mut transfer = DataTransfer::internal();
        transfer.set_text("nana-clipboard-120").unwrap();
        transfer
            .set_html("<span>nana-clipboard-120</span>")
            .unwrap();
        transfer.set_png(png_bytes.clone()).unwrap();
        transfer.set_rgba_image(image.clone()).unwrap();
        transfer
            .set_file_paths(&[std::path::PathBuf::from("/tmp/nana-issue-120-probe.txt")])
            .unwrap();
        transfer
            .set_bytes(custom.clone(), b"nana-custom-bytes".as_slice())
            .unwrap();

        let before = clip.change_count();
        clip.write_transfer(&transfer).unwrap();
        let written = clip.change_count();
        if let (Some(b), Some(w)) = (before, written) {
            assert_ne!(b, w);
        }

        let read = clip.read_transfer().unwrap().unwrap();
        assert!(read.has_format(&DataFormat::TextPlain));
        assert!(read.has_format(&DataFormat::TextHtml));
        assert!(read.has_format(&DataFormat::ImagePng));
        assert!(read.has_format(&DataFormat::ImageRgba8));
        assert!(read.has_format(&custom));

        assert_eq!(
            read.read_text().unwrap().as_deref(),
            Some("nana-clipboard-120")
        );
        assert_eq!(
            read.read_html().unwrap().as_deref(),
            Some("<span>nana-clipboard-120</span>")
        );
        assert_eq!(read.read_rgba_image().unwrap(), Some(image));
        assert_eq!(
            read.read_bytes(&custom).unwrap().as_deref(),
            Some(b"nana-custom-bytes".as_slice())
        );

        if let Some(prev) = previous {
            let _ = clip.write_transfer(&prev);
        }
    }
}
