use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nana_ui_platform::{
    DataFormat, DataTransfer, HostServiceError, TransferCachePolicy, TransferError, TransferImage,
    TransferOrigin, TransferProvider,
};

use super::{ClipboardSupport, decode_png, encode_png, parse_linux_targets};

const SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinuxTool {
    Wayland,
    Xclip,
}

fn detect_tool() -> Option<LinuxTool> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && command_exists("wl-paste")
        && command_exists("wl-copy")
    {
        return Some(LinuxTool::Wayland);
    }
    if std::env::var_os("DISPLAY").is_some() && command_exists("xclip") {
        return Some(LinuxTool::Xclip);
    }
    None
}

fn command_exists(bin: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let candidate: PathBuf = dir.join(bin);
        candidate.is_file()
    })
}

pub(super) fn support() -> ClipboardSupport {
    match detect_tool() {
        Some(LinuxTool::Wayland) => ClipboardSupport {
            available: true,
            backend: "linux-wl-clipboard",
            supports_change_count: false,
            supports_images: true,
            supports_html: true,
            supports_uri_list: true,
            supports_custom_formats: true,
            supports_lazy_read: true,
            reason: None,
        },
        Some(LinuxTool::Xclip) => ClipboardSupport {
            available: true,
            backend: "linux-x11-xclip",
            supports_change_count: false,
            supports_images: true,
            supports_html: true,
            supports_uri_list: true,
            supports_custom_formats: true,
            supports_lazy_read: true,
            reason: None,
        },
        None => ClipboardSupport::unsupported(
            "linux-none",
            "neither Wayland (wl-paste/wl-copy) nor X11 (xclip) clipboard tools are available",
        ),
    }
}

fn run_read_command(
    program: &str,
    args: &[&str],
    max_bytes: usize,
) -> Result<Vec<u8>, TransferError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| TransferError::ProviderFailed(err.to_string().into()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| TransferError::ProviderFailed("missing stdout pipe".into()))?;
    let cap = max_bytes.saturating_add(1);
    let mut buf = Vec::new();
    let deadline = Instant::now() + SUBPROCESS_TIMEOUT;
    let mut chunk = [0u8; 8192];
    loop {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(TransferError::ProviderFailed(
                "clipboard read timed out".into(),
            ));
        }
        match stdout.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() >= cap {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(TransferError::PayloadTooLarge {
                        size: buf.len(),
                        max_bytes,
                    });
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(TransferError::ProviderFailed(err.to_string().into()));
            }
        }
    }
    let status = child
        .wait()
        .map_err(|err| TransferError::ProviderFailed(err.to_string().into()))?;
    if !status.success() {
        return Err(TransferError::FormatNotAvailable);
    }
    Ok(buf)
}

fn run_write_command(program: &str, args: &[&str], data: &[u8]) -> Result<(), HostServiceError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| HostServiceError::Failed)?;
    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(data).is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(HostServiceError::Failed);
    }
    let deadline = Instant::now() + SUBPROCESS_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(HostServiceError::Failed)
                };
            }
            Ok(None) if Instant::now() > deadline => {
                // `wl-copy` / `xclip` may fork/background to own the selection, so
                // wait briefly and kill only if the direct child hangs.
                let _ = child.kill();
                let _ = child.wait();
                return Err(HostServiceError::Failed);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return Err(HostServiceError::Failed),
        }
    }
}

struct LinuxClipboardReader {
    tool: LinuxTool,
    mime: String,
    decode_png_to_rgba8: bool,
    max_bytes: usize,
}

impl TransferProvider for LinuxClipboardReader {
    fn provide(&self, _format: &DataFormat) -> Result<Vec<u8>, TransferError> {
        let bytes = match self.tool {
            LinuxTool::Wayland => run_read_command(
                "wl-paste",
                &["--no-newline", "--type", &self.mime],
                self.max_bytes,
            )?,
            LinuxTool::Xclip => run_read_command(
                "xclip",
                &["-selection", "clipboard", "-o", "-t", &self.mime],
                self.max_bytes,
            )?,
        };
        if self.decode_png_to_rgba8 {
            let image = decode_png(&bytes, self.max_bytes)?;
            Ok(image.to_rgba8_bytes())
        } else {
            Ok(bytes)
        }
    }
}

pub(super) fn read_transfer() -> Result<Option<DataTransfer>, HostServiceError> {
    let Some(tool) = detect_tool() else {
        return Err(HostServiceError::Unsupported);
    };
    let targets_bytes = match tool {
        LinuxTool::Wayland => run_read_command("wl-paste", &["--list-types"], 64 * 1024),
        LinuxTool::Xclip => run_read_command(
            "xclip",
            &["-selection", "clipboard", "-o", "-t", "TARGETS"],
            64 * 1024,
        ),
    };
    let Ok(targets_bytes) = targets_bytes else {
        return Ok(None);
    };
    let raw = String::from_utf8_lossy(&targets_bytes);
    let formats = parse_linux_targets(&raw);
    if formats.is_empty() {
        return Ok(None);
    }

    let mut transfer = DataTransfer::new(TransferOrigin::External);
    let max_bytes = transfer.max_bytes();
    let has_png = formats.contains(&DataFormat::ImagePng);
    let has_rgba8 = formats.contains(&DataFormat::ImageRgba8);

    for fmt in formats {
        let mime = fmt.as_mime().to_owned();
        transfer.set_lazy(
            fmt,
            None,
            TransferCachePolicy::Cache,
            Arc::new(LinuxClipboardReader {
                tool,
                mime,
                decode_png_to_rgba8: false,
                max_bytes,
            }),
        );
    }
    if has_png && !has_rgba8 {
        transfer.set_lazy(
            DataFormat::ImageRgba8,
            None,
            TransferCachePolicy::Cache,
            Arc::new(LinuxClipboardReader {
                tool,
                mime: "image/png".to_owned(),
                decode_png_to_rgba8: true,
                max_bytes,
            }),
        );
    }
    Ok(Some(transfer))
}

pub(super) fn write_transfer(transfer: &DataTransfer) -> Result<(), HostServiceError> {
    let Some(tool) = detect_tool() else {
        return Err(HostServiceError::Unsupported);
    };
    let formats = transfer.formats();
    let preferred = [
        DataFormat::TextPlain,
        DataFormat::ImagePng,
        DataFormat::ImageRgba8,
        DataFormat::TextHtml,
        DataFormat::TextUriList,
        DataFormat::OctetStream,
    ]
    .into_iter()
    .find(|fmt| formats.contains(fmt))
    .or_else(|| formats.into_iter().next())
    .ok_or(HostServiceError::Failed)?;

    let bytes = transfer
        .read_bytes(&preferred)
        .map_err(|_| HostServiceError::Failed)?
        .ok_or(HostServiceError::Failed)?;

    let (mime, payload) = if preferred == DataFormat::ImageRgba8 {
        let img = TransferImage::from_rgba8_bytes(&bytes).map_err(|_| HostServiceError::Failed)?;
        (
            "image/png",
            encode_png(&img).map_err(|_| HostServiceError::Failed)?,
        )
    } else {
        (preferred.as_mime(), bytes.to_vec())
    };

    match tool {
        LinuxTool::Wayland => run_write_command("wl-copy", &["--type", mime], &payload),
        LinuxTool::Xclip => run_write_command(
            "xclip",
            &["-selection", "clipboard", "-i", "-t", mime],
            &payload,
        ),
    }
}
