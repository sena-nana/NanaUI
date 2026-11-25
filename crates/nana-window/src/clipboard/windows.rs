use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::Arc;

use nana_ui_platform::{
    DataFormat, DataTransfer, HostServiceError, TransferCachePolicy, TransferError, TransferImage,
    TransferOrigin, TransferProvider, format_uri_list, path_to_file_uri,
};
use windows_sys::Win32::Foundation::{GLOBAL_HANDLE, HGLOBAL};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalFree, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows_sys::Win32::System::Ole::{CF_HDROP, CF_UNICODETEXT};
use windows_sys::Win32::UI::Shell::DragQueryFileW;

use super::{
    ClipboardSupport, custom_mime_to_win32_format_name, decode_cf_html, decode_png,
    decode_utf16_nul, encode_cf_html, encode_png, encode_utf16_nul,
    win32_format_name_to_custom_mime,
};

pub(super) fn support() -> ClipboardSupport {
    ClipboardSupport {
        available: true,
        backend: "win32-clipboard",
        supports_change_count: true,
        supports_images: true,
        supports_html: true,
        supports_uri_list: true,
        supports_custom_formats: true,
        supports_lazy_read: true,
        reason: None,
    }
}

pub(super) fn change_count() -> Option<u64> {
    // SAFETY: GetClipboardSequenceNumber takes no arguments and has no preconditions.
    let seq = unsafe { GetClipboardSequenceNumber() };
    (seq != 0).then_some(u64::from(seq))
}

struct ClipboardGuard;

impl ClipboardGuard {
    fn open() -> Result<Self, HostServiceError> {
        // SAFETY: Passing NULL opens the clipboard associated with the current task.
        if unsafe { OpenClipboard(null_mut()) } == 0 {
            return Err(HostServiceError::Busy);
        }
        Ok(Self)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: Only constructed after a matching OpenClipboard succeeded.
        unsafe {
            CloseClipboard();
        }
    }
}

fn register_format(name: &str) -> Result<u32, HostServiceError> {
    let wide = encode_utf16_nul(name);
    // SAFETY: `wide` is NUL-terminated UTF-16.
    let id = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
    if id == 0 {
        Err(HostServiceError::Failed)
    } else {
        Ok(id)
    }
}

fn format_name(format_id: u32) -> Option<String> {
    let mut buf = [0u16; 256];
    // SAFETY: `buf` is valid for 256 UTF-16 code units.
    let len = unsafe { GetClipboardFormatNameW(format_id, buf.as_mut_ptr(), buf.len() as i32) };
    if len <= 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

unsafe fn read_hglobal_bytes(
    handle: *mut c_void,
    max_bytes: usize,
) -> Result<Vec<u8>, TransferError> {
    if handle.is_null() {
        return Err(TransferError::FormatNotAvailable);
    }
    let hglobal = handle as HGLOBAL;
    let size = unsafe { GlobalSize(hglobal) };
    if size == 0 {
        return Ok(Vec::new());
    }
    if size > max_bytes {
        return Err(TransferError::PayloadTooLarge { size, max_bytes });
    }
    let ptr = unsafe { GlobalLock(hglobal) };
    if ptr.is_null() {
        return Err(TransferError::ProviderFailed("GlobalLock failed".into()));
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, size) }.to_vec();
    unsafe {
        GlobalUnlock(hglobal);
    }
    Ok(bytes)
}

unsafe fn write_hglobal_bytes(format_id: u32, bytes: &[u8]) -> Result<(), HostServiceError> {
    let alloc_size = bytes.len().max(1);
    let hglobal = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, alloc_size) };
    if hglobal.is_null() {
        return Err(HostServiceError::Failed);
    }
    let ptr = unsafe { GlobalLock(hglobal) };
    if ptr.is_null() {
        unsafe {
            GlobalFree(hglobal);
        }
        return Err(HostServiceError::Failed);
    }
    if !bytes.is_empty() {
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
        }
    }
    unsafe {
        GlobalUnlock(hglobal);
    }
    // SAFETY: After SetClipboardData succeeds, the system owns `hglobal`.
    let res = unsafe { SetClipboardData(format_id, hglobal as GLOBAL_HANDLE) };
    if res.is_null() {
        unsafe {
            GlobalFree(hglobal);
        }
        return Err(HostServiceError::Failed);
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum WinReadKind {
    UnicodeText,
    CfHtml,
    Hdrop,
    RawBytes,
    DecodePngToRgba8,
}

struct WinClipboardLazyReader {
    expected_seq: u32,
    format_id: u32,
    kind: WinReadKind,
    max_bytes: usize,
}

impl TransferProvider for WinClipboardLazyReader {
    fn provide(&self, _format: &DataFormat) -> Result<Vec<u8>, TransferError> {
        let current_seq = unsafe { GetClipboardSequenceNumber() };
        if self.expected_seq != 0 && current_seq != self.expected_seq {
            return Err(TransferError::Cancelled);
        }
        let _guard = ClipboardGuard::open()
            .map_err(|_| TransferError::ProviderFailed("clipboard busy".into()))?;
        let handle = unsafe { GetClipboardData(self.format_id) };
        if handle.is_null() {
            return Err(TransferError::FormatNotAvailable);
        }
        match self.kind {
            WinReadKind::UnicodeText => {
                let raw = unsafe { read_hglobal_bytes(handle, self.max_bytes.saturating_mul(2))? };
                let units: Vec<u16> = raw
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                let text = decode_utf16_nul(&units).into_bytes();
                if text.len() > self.max_bytes {
                    return Err(TransferError::PayloadTooLarge {
                        size: text.len(),
                        max_bytes: self.max_bytes,
                    });
                }
                Ok(text)
            }
            WinReadKind::CfHtml => {
                let raw = unsafe { read_hglobal_bytes(handle, self.max_bytes)? };
                let html = decode_cf_html(&raw).ok_or(TransferError::InvalidUtf8)?;
                Ok(html.into_bytes())
            }
            WinReadKind::Hdrop => {
                let hdrop = handle as _;
                let count = unsafe { DragQueryFileW(hdrop, u32::MAX, null_mut(), 0) };
                let mut uris = Vec::with_capacity(count as usize);
                for index in 0..count {
                    let len = unsafe { DragQueryFileW(hdrop, index, null_mut(), 0) };
                    if len == 0 {
                        continue;
                    }
                    let mut buf = vec![0u16; len as usize + 1];
                    let written =
                        unsafe { DragQueryFileW(hdrop, index, buf.as_mut_ptr(), buf.len() as u32) };
                    if written > 0 {
                        let path = String::from_utf16_lossy(&buf[..written as usize]);
                        uris.push(path_to_file_uri(&PathBuf::from(path)));
                    }
                }
                let encoded = format_uri_list(&uris).into_bytes();
                if encoded.len() > self.max_bytes {
                    return Err(TransferError::PayloadTooLarge {
                        size: encoded.len(),
                        max_bytes: self.max_bytes,
                    });
                }
                Ok(encoded)
            }
            WinReadKind::RawBytes => unsafe { read_hglobal_bytes(handle, self.max_bytes) },
            WinReadKind::DecodePngToRgba8 => {
                let bytes = unsafe { read_hglobal_bytes(handle, self.max_bytes)? };
                let image = decode_png(&bytes, self.max_bytes)?;
                Ok(image.to_rgba8_bytes())
            }
        }
    }
}

pub(super) fn read_transfer() -> Result<Option<DataTransfer>, HostServiceError> {
    let _guard = ClipboardGuard::open()?;
    let expected_seq = unsafe { GetClipboardSequenceNumber() };

    let mut transfer = DataTransfer::new(TransferOrigin::External);
    let max_bytes = transfer.max_bytes();
    let mut png_format_id: Option<u32> = None;
    let mut has_direct_rgba8 = false;
    let mut has_uri_list = false;

    let mut fmt_id = 0u32;
    loop {
        fmt_id = unsafe { EnumClipboardFormats(fmt_id) };
        if fmt_id == 0 {
            break;
        }
        if fmt_id == CF_UNICODETEXT as u32 {
            attach_lazy(
                &mut transfer,
                DataFormat::TextPlain,
                expected_seq,
                fmt_id,
                WinReadKind::UnicodeText,
                max_bytes,
            );
        } else if fmt_id == CF_HDROP as u32 && !has_uri_list {
            has_uri_list = true;
            attach_lazy(
                &mut transfer,
                DataFormat::TextUriList,
                expected_seq,
                fmt_id,
                WinReadKind::Hdrop,
                max_bytes,
            );
        } else if let Some(name) = format_name(fmt_id) {
            if name == "HTML Format" {
                attach_lazy(
                    &mut transfer,
                    DataFormat::TextHtml,
                    expected_seq,
                    fmt_id,
                    WinReadKind::CfHtml,
                    max_bytes,
                );
            } else if name == "PNG" {
                png_format_id = Some(fmt_id);
                attach_lazy(
                    &mut transfer,
                    DataFormat::ImagePng,
                    expected_seq,
                    fmt_id,
                    WinReadKind::RawBytes,
                    max_bytes,
                );
            } else if name == "text/uri-list" {
                has_uri_list = true;
                attach_lazy(
                    &mut transfer,
                    DataFormat::TextUriList,
                    expected_seq,
                    fmt_id,
                    WinReadKind::RawBytes,
                    max_bytes,
                );
            } else if name == custom_mime_to_win32_format_name("image/rgba8") {
                has_direct_rgba8 = true;
                attach_lazy(
                    &mut transfer,
                    DataFormat::ImageRgba8,
                    expected_seq,
                    fmt_id,
                    WinReadKind::RawBytes,
                    max_bytes,
                );
            } else if name == "application/octet-stream" {
                attach_lazy(
                    &mut transfer,
                    DataFormat::OctetStream,
                    expected_seq,
                    fmt_id,
                    WinReadKind::RawBytes,
                    max_bytes,
                );
            } else if let Some(custom) = win32_format_name_to_custom_mime(&name) {
                attach_lazy(
                    &mut transfer,
                    custom,
                    expected_seq,
                    fmt_id,
                    WinReadKind::RawBytes,
                    max_bytes,
                );
            }
        }
    }

    if let Some(png_id) = png_format_id
        && !has_direct_rgba8
    {
        attach_lazy(
            &mut transfer,
            DataFormat::ImageRgba8,
            expected_seq,
            png_id,
            WinReadKind::DecodePngToRgba8,
            max_bytes,
        );
    }

    if transfer.is_empty() {
        Ok(None)
    } else {
        Ok(Some(transfer))
    }
}

fn attach_lazy(
    transfer: &mut DataTransfer,
    format: DataFormat,
    expected_seq: u32,
    format_id: u32,
    kind: WinReadKind,
    max_bytes: usize,
) {
    transfer.set_lazy(
        format,
        None,
        TransferCachePolicy::Cache,
        Arc::new(WinClipboardLazyReader {
            expected_seq,
            format_id,
            kind,
            max_bytes,
        }),
    );
}

pub(super) fn write_transfer(transfer: &DataTransfer) -> Result<(), HostServiceError> {
    // Materialize and validate everything before emptying the OS clipboard so a
    // failed/cancelled lazy provider never discards existing user clipboard data.
    let mut staged: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut has_png = false;
    let mut rgba_source: Option<Arc<[u8]>> = None;

    for format in transfer.formats() {
        let Some(bytes) = transfer
            .read_bytes(&format)
            .map_err(|_| HostServiceError::Failed)?
        else {
            continue;
        };
        match format {
            DataFormat::TextPlain => {
                let text = std::str::from_utf8(&bytes).map_err(|_| HostServiceError::Failed)?;
                let utf16 = encode_utf16_nul(text);
                let mut raw = Vec::with_capacity(utf16.len() * 2);
                for unit in utf16 {
                    raw.extend_from_slice(&unit.to_le_bytes());
                }
                staged.push((CF_UNICODETEXT as u32, raw));
            }
            DataFormat::TextHtml => {
                let html = std::str::from_utf8(&bytes).map_err(|_| HostServiceError::Failed)?;
                staged.push((register_format("HTML Format")?, encode_cf_html(html)));
            }
            DataFormat::TextUriList => {
                staged.push((register_format("text/uri-list")?, bytes.to_vec()));
            }
            DataFormat::ImagePng => {
                has_png = true;
                staged.push((register_format("PNG")?, bytes.to_vec()));
            }
            DataFormat::ImageRgba8 => {
                rgba_source = Some(Arc::clone(&bytes));
                staged.push((
                    register_format(&custom_mime_to_win32_format_name("image/rgba8"))?,
                    bytes.to_vec(),
                ));
            }
            DataFormat::OctetStream => {
                staged.push((register_format("application/octet-stream")?, bytes.to_vec()));
            }
            DataFormat::Custom(mime) => {
                staged.push((
                    register_format(&custom_mime_to_win32_format_name(&mime))?,
                    bytes.to_vec(),
                ));
            }
        }
    }

    if !has_png && let Some(rgba) = rgba_source {
        let image = TransferImage::from_rgba8_bytes(&rgba).map_err(|_| HostServiceError::Failed)?;
        let png = encode_png(&image).map_err(|_| HostServiceError::Failed)?;
        staged.push((register_format("PNG")?, png));
    }

    if staged.is_empty() {
        return Err(HostServiceError::Failed);
    }

    let _guard = ClipboardGuard::open()?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err(HostServiceError::Failed);
    }
    for (format_id, bytes) in &staged {
        unsafe { write_hglobal_bytes(*format_id, bytes)? };
    }
    Ok(())
}
