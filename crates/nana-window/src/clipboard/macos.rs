use std::path::PathBuf;
use std::sync::Arc;

use nana_ui_platform::{
    DataFormat, DataTransfer, HostServiceError, TransferCachePolicy, TransferError, TransferOrigin,
    TransferProvider, format_uri_list, path_to_file_uri,
};
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardTypeHTML,
    NSPasteboardTypePNG, NSPasteboardTypeString, NSPasteboardTypeTIFF, NSPasteboardTypeURL,
    NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSString, NSURL};

use super::{ClipboardSupport, custom_mime_to_uti, decode_png, encode_png, uti_to_custom_mime};

const UTI_OCTET_STREAM: &str = "public.data";

pub(super) fn support() -> ClipboardSupport {
    ClipboardSupport {
        available: true,
        backend: "macos-nspasteboard",
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
    let count = NSPasteboard::generalPasteboard().changeCount();
    u64::try_from(count).ok()
}

struct MacPasteboardLazyReader {
    expected_change_count: isize,
    uti: String,
    kind: MacReadKind,
    max_bytes: usize,
}

#[derive(Clone, Copy)]
enum MacReadKind {
    RawBytes,
    UriList,
    DecodeImageToRgba8,
}

impl TransferProvider for MacPasteboardLazyReader {
    fn provide(&self, _format: &DataFormat) -> Result<Vec<u8>, TransferError> {
        let pasteboard = NSPasteboard::generalPasteboard();
        if pasteboard.changeCount() != self.expected_change_count {
            return Err(TransferError::Cancelled);
        }
        match self.kind {
            MacReadKind::RawBytes => {
                let pb_type = NSString::from_str(&self.uti);
                let data = pasteboard
                    .dataForType(&pb_type)
                    .ok_or(TransferError::FormatNotAvailable)?;
                if data.len() > self.max_bytes {
                    return Err(TransferError::PayloadTooLarge {
                        size: data.len(),
                        max_bytes: self.max_bytes,
                    });
                }
                Ok(data.to_vec())
            }
            MacReadKind::UriList => {
                let mut uris = Vec::new();
                if let Some(items) = pasteboard.pasteboardItems() {
                    for item in items.iter() {
                        if let Some(file_url) =
                            item.stringForType(unsafe { NSPasteboardTypeFileURL })
                        {
                            let raw = file_url.to_string();
                            if let Some(nsurl) = NSURL::URLWithString(&NSString::from_str(&raw))
                                && let Some(path) = nsurl.path()
                            {
                                uris.push(path_to_file_uri(&PathBuf::from(path.to_string())));
                            } else {
                                uris.push(raw);
                            }
                        } else if let Some(url) = item.stringForType(unsafe { NSPasteboardTypeURL })
                        {
                            uris.push(url.to_string());
                        }
                    }
                }
                if uris.is_empty()
                    && let Some(raw) = pasteboard.dataForType(&NSString::from_str(&self.uti))
                {
                    if raw.len() > self.max_bytes {
                        return Err(TransferError::PayloadTooLarge {
                            size: raw.len(),
                            max_bytes: self.max_bytes,
                        });
                    }
                    return Ok(raw.to_vec());
                }
                if uris.is_empty() {
                    return Err(TransferError::FormatNotAvailable);
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
            MacReadKind::DecodeImageToRgba8 => {
                let pb_type = NSString::from_str(&self.uti);
                let data = pasteboard
                    .dataForType(&pb_type)
                    .ok_or(TransferError::FormatNotAvailable)?;
                if data.len() > self.max_bytes {
                    return Err(TransferError::PayloadTooLarge {
                        size: data.len(),
                        max_bytes: self.max_bytes,
                    });
                }
                let bytes = data.to_vec();
                let image = decode_png(&bytes, self.max_bytes)?;
                Ok(image.to_rgba8_bytes())
            }
        }
    }
}

pub(super) fn read_transfer() -> Result<Option<DataTransfer>, HostServiceError> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let expected_change_count = pasteboard.changeCount();
    let Some(types) = pasteboard.types() else {
        return Ok(None);
    };
    if types.is_empty() {
        return Ok(None);
    }

    let mut transfer = DataTransfer::new(TransferOrigin::External);
    let max_bytes = transfer.max_bytes();
    let mut has_png_uti = false;
    let mut has_direct_rgba8 = false;
    let mut has_uri_list = false;

    for pb_type in types.iter() {
        let uti = pb_type.to_string();
        if uti == unsafe { NSPasteboardTypeString }.to_string() || uti == "public.utf8-plain-text" {
            attach_lazy(
                &mut transfer,
                DataFormat::TextPlain,
                expected_change_count,
                uti,
                MacReadKind::RawBytes,
                max_bytes,
            );
        } else if uti == unsafe { NSPasteboardTypeHTML }.to_string() || uti == "public.html" {
            attach_lazy(
                &mut transfer,
                DataFormat::TextHtml,
                expected_change_count,
                uti,
                MacReadKind::RawBytes,
                max_bytes,
            );
        } else if uti == unsafe { NSPasteboardTypeFileURL }.to_string()
            || uti == unsafe { NSPasteboardTypeURL }.to_string()
            || uti == "text/uri-list"
            || uti == custom_mime_to_uti("text/uri-list")
        {
            if !has_uri_list {
                has_uri_list = true;
                attach_lazy(
                    &mut transfer,
                    DataFormat::TextUriList,
                    expected_change_count,
                    uti,
                    MacReadKind::UriList,
                    max_bytes,
                );
            }
        } else if uti == unsafe { NSPasteboardTypePNG }.to_string() || uti == "public.png" {
            has_png_uti = true;
            attach_lazy(
                &mut transfer,
                DataFormat::ImagePng,
                expected_change_count,
                uti,
                MacReadKind::RawBytes,
                max_bytes,
            );
        } else if uti == custom_mime_to_uti("image/rgba8") {
            has_direct_rgba8 = true;
            attach_lazy(
                &mut transfer,
                DataFormat::ImageRgba8,
                expected_change_count,
                uti,
                MacReadKind::RawBytes,
                max_bytes,
            );
        } else if uti == UTI_OCTET_STREAM {
            attach_lazy(
                &mut transfer,
                DataFormat::OctetStream,
                expected_change_count,
                uti,
                MacReadKind::RawBytes,
                max_bytes,
            );
        } else if let Some(custom) = uti_to_custom_mime(&uti) {
            attach_lazy(
                &mut transfer,
                custom,
                expected_change_count,
                uti,
                MacReadKind::RawBytes,
                max_bytes,
            );
        } else if uti == unsafe { NSPasteboardTypeTIFF }.to_string() && !has_png_uti {
            // Keep TIFF unnoticed unless PNG isn't present; TIFF decoding isn't bundled.
        }
    }

    if has_png_uti && !has_direct_rgba8 {
        attach_lazy(
            &mut transfer,
            DataFormat::ImageRgba8,
            expected_change_count,
            unsafe { NSPasteboardTypePNG }.to_string(),
            MacReadKind::DecodeImageToRgba8,
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
    expected_change_count: isize,
    uti: String,
    kind: MacReadKind,
    max_bytes: usize,
) {
    transfer.set_lazy(
        format,
        None,
        TransferCachePolicy::Cache,
        Arc::new(MacPasteboardLazyReader {
            expected_change_count,
            uti,
            kind,
            max_bytes,
        }),
    );
}

pub(super) fn write_transfer(transfer: &DataTransfer) -> Result<(), HostServiceError> {
    let mut materialized: Vec<(DataFormat, Arc<[u8]>)> = Vec::new();
    for format in transfer.formats() {
        let Some(bytes) = transfer
            .read_bytes(&format)
            .map_err(|_| HostServiceError::Failed)?
        else {
            continue;
        };
        materialized.push((format, bytes));
    }
    if materialized.is_empty() {
        return Err(HostServiceError::Failed);
    }

    let has_png = materialized
        .iter()
        .any(|(fmt, _)| *fmt == DataFormat::ImagePng);
    let mut synthesized_png: Option<Vec<u8>> = None;
    if !has_png
        && let Some((_, rgba_bytes)) = materialized
            .iter()
            .find(|(fmt, _)| *fmt == DataFormat::ImageRgba8)
    {
        let img = nana_ui_platform::TransferImage::from_rgba8_bytes(rgba_bytes)
            .map_err(|_| HostServiceError::Failed)?;
        synthesized_png = Some(encode_png(&img).map_err(|_| HostServiceError::Failed)?);
    }

    let item = NSPasteboardItem::new();
    let mut wrote_any = false;

    for (format, bytes) in &materialized {
        let ok = match format {
            DataFormat::TextPlain => {
                let text = std::str::from_utf8(bytes).map_err(|_| HostServiceError::Failed)?;
                item.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString })
            }
            DataFormat::TextHtml => {
                let data = NSData::with_bytes(bytes);
                item.setData_forType(&data, unsafe { NSPasteboardTypeHTML })
            }
            DataFormat::TextUriList => {
                let raw = std::str::from_utf8(bytes).map_err(|_| HostServiceError::Failed)?;
                let mut uri_ok = item.setData_forType(
                    &NSData::with_bytes(bytes),
                    &NSString::from_str(&custom_mime_to_uti(format.as_mime())),
                );
                let uris = nana_ui_platform::parse_uri_list(raw);
                if let Some(first) = uris.first() {
                    if let Some(path) = nana_ui_platform::file_uri_to_path(first) {
                        let url =
                            NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
                        if let Some(abs) = url.absoluteString() {
                            uri_ok |=
                                item.setString_forType(&abs, unsafe { NSPasteboardTypeFileURL });
                        }
                    } else {
                        uri_ok |= item.setString_forType(&NSString::from_str(first), unsafe {
                            NSPasteboardTypeURL
                        });
                    }
                }
                uri_ok
            }
            DataFormat::ImagePng => {
                item.setData_forType(&NSData::with_bytes(bytes), unsafe { NSPasteboardTypePNG })
            }
            DataFormat::ImageRgba8 => item.setData_forType(
                &NSData::with_bytes(bytes),
                &NSString::from_str(&custom_mime_to_uti(format.as_mime())),
            ),
            DataFormat::OctetStream => item.setData_forType(
                &NSData::with_bytes(bytes),
                &NSString::from_str(UTI_OCTET_STREAM),
            ),
            DataFormat::Custom(mime) => item.setData_forType(
                &NSData::with_bytes(bytes),
                &NSString::from_str(&custom_mime_to_uti(mime)),
            ),
        };
        wrote_any |= ok;
    }

    if let Some(png_bytes) = synthesized_png {
        wrote_any |= item.setData_forType(&NSData::with_bytes(&png_bytes), unsafe {
            NSPasteboardTypePNG
        });
    }

    if !wrote_any {
        return Err(HostServiceError::Failed);
    }

    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    let writer: &ProtocolObject<dyn NSPasteboardWriting> = ProtocolObject::from_ref(&*item);
    let array = NSArray::from_slice(&[writer]);
    if pasteboard.writeObjects(&array) {
        Ok(())
    } else {
        Err(HostServiceError::Failed)
    }
}
