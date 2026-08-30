//! Shared frame mailbox and GPU bridge for host-fed `<video>` surfaces.
//!
//! The decoder, demuxer, and playback clock stay with the consuming
//! application. The host pushes premultiplied RGBA8 frames into
//! [`VideoRuntime`]; the GPU bridge uploads the newest frame into a stable
//! WGPU texture on the frame pump, mirroring how `CanvasRuntime` feeds
//! `CanvasGpuBridge`. Frames coalesce: pushing twice before a pump keeps
//! only the newest.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Stable surface id. Must match the element attribute
/// `data-nana-video="{id}"` (see `tree::video_host_texture_slot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VideoId(pub u64);

/// Dirty sub-rectangle in frame pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl VideoRect {
    fn clamp_to(self, width: u32, height: u32) -> Option<Self> {
        let x = self.x.min(width);
        let y = self.y.min(height);
        let width = self.width.min(width.saturating_sub(x));
        let height = self.height.min(height.saturating_sub(y));
        (width > 0 && height > 0).then_some(Self {
            x,
            y,
            width,
            height,
        })
    }
}

/// One pushed frame. `bytes` is the full premultiplied RGBA8 frame
/// (`width * height * 4`); `dirty` is the sub-rectangle that changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub version: u64,
    pub bytes: Vec<u8>,
    pub dirty: VideoRect,
}

/// Frame mailbox keyed by [`VideoId`]. Keeps the newest frame per surface so
/// the video element keeps displaying its last frame until the host removes
/// the surface.
#[derive(Debug, Default)]
pub struct VideoRuntime {
    frames: HashMap<VideoId, VideoFrame>,
}

impl VideoRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pushes a frame; `dirty: None` marks the whole frame changed. Pushing
    /// again before the next frame pump coalesces to the newest frame.
    pub fn push_frame(
        &mut self,
        id: VideoId,
        width: u32,
        height: u32,
        bytes: Vec<u8>,
        dirty: Option<VideoRect>,
    ) -> Result<(), String> {
        if width == 0 || height == 0 {
            return Err(format!(
                "video frame {id:?} has zero extent ({width}x{height})"
            ));
        }
        let expected = width as usize * height as usize * 4;
        if bytes.len() != expected {
            return Err(format!(
                "video frame {id:?} expects {expected} bytes for {width}x{height}, got {}",
                bytes.len()
            ));
        }
        let dirty = dirty
            .unwrap_or(VideoRect {
                x: 0,
                y: 0,
                width,
                height,
            })
            .clamp_to(width, height)
            .ok_or_else(|| format!("video frame {id:?} dirty rect is outside {width}x{height}"))?;
        let version = self
            .frames
            .get(&id)
            .map(|frame| frame.version + 1)
            .unwrap_or(1);
        self.frames.insert(
            id,
            VideoFrame {
                width,
                height,
                version,
                bytes,
                dirty,
            },
        );
        Ok(())
    }

    /// Newest frame whose version differs from `uploaded_version`.
    /// `None` means the GPU side is already current or the surface is unknown.
    pub fn take_upload(
        &mut self,
        id: VideoId,
        uploaded_version: Option<u64>,
    ) -> Option<VideoFrame> {
        let frame = self.frames.get(&id)?;
        (uploaded_version != Some(frame.version)).then(|| frame.clone())
    }

    pub fn contains(&self, id: VideoId) -> bool {
        self.frames.contains_key(&id)
    }

    pub fn remove(&mut self, id: VideoId) -> bool {
        self.frames.remove(&id).is_some()
    }
}

/// Shared handle handed to both the host push side and the GPU bridge.
pub type SharedVideoRuntime = Arc<Mutex<VideoRuntime>>;

pub fn shared_video_runtime() -> SharedVideoRuntime {
    Arc::new(Mutex::new(VideoRuntime::new()))
}

/// Shared by every Vue window in one runtime, just like the Canvas GPU
/// bridge it mirrors. Reads the mailbox through `&SharedVideoRuntime` at
/// sync time so a shared runtime can be swapped without rebuilding.
#[cfg(feature = "hosted")]
#[derive(Clone)]
pub(crate) struct VideoGpuBridge {
    store: crate::canvas_gpu::HostTextureSlotStore<VideoId>,
}

#[cfg(feature = "hosted")]
impl std::fmt::Debug for VideoGpuBridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VideoGpuBridge")
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "hosted")]
impl VideoGpuBridge {
    const VIDEO_TEXTURE_ID_BIT: u64 = 1 << 62;

    pub(crate) fn new(
        resources: nana_ui::HostedGpuResources,
        textures: nana_ui::HostTextureRegistry,
    ) -> Self {
        Self {
            store: crate::canvas_gpu::HostTextureSlotStore::new(resources, textures),
        }
    }

    pub(crate) fn replace_device(&self, resources: nana_ui::HostedGpuResources) {
        self.store.replace_device(resources);
    }

    /// Uploads the newest frame for `id`.
    pub(crate) fn sync(
        &self,
        id: VideoId,
        video: &SharedVideoRuntime,
    ) -> Result<Option<nana_ui::HostTextureBinding>, String> {
        self.store
            .retain(|key, _| video.lock().map_or(false, |video| video.contains(*key)));
        let uploaded_version = self.store.uploaded_version(&id);
        let upload = video
            .lock()
            .map_err(|_| "Video runtime poisoned".to_owned())?
            .take_upload(id, uploaded_version);
        let Some(upload) = upload else {
            return Ok(self.store.binding(&id));
        };
        self.store
            .sync(
                id,
                crate::canvas_gpu::HostTextureUpload {
                    // Must match `tree::video_host_texture_slot` (`video:{id}`)
                    // so Scene samples this node in document order.
                    slot: &format!("video:{}", id.0),
                    texture_id: Self::VIDEO_TEXTURE_ID_BIT | id.0,
                    width: upload.width,
                    height: upload.height,
                    version: upload.version,
                    bytes: &upload.bytes,
                    dirty_x: upload.dirty.x,
                    dirty_y: upload.dirty.y,
                    dirty_width: upload.dirty.width,
                    dirty_height: upload.dirty.height,
                    label: "NanaUI Video texture",
                },
            )
            .map_err(|_| "Video GPU state poisoned".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_bytes(width: u32, height: u32) -> Vec<u8> {
        vec![0x40; width as usize * height as usize * 4]
    }

    #[test]
    fn push_frame_versions_are_monotonic() {
        let mut runtime = VideoRuntime::new();
        let id = VideoId(1);
        runtime
            .push_frame(id, 2, 2, frame_bytes(2, 2), None)
            .unwrap();
        runtime
            .push_frame(id, 2, 2, frame_bytes(2, 2), None)
            .unwrap();
        assert!(runtime.contains(id));
        let upload = runtime.take_upload(id, None).unwrap();
        assert_eq!(upload.version, 2);
    }

    #[test]
    fn push_frame_coalesces_to_newest_before_upload() {
        let mut runtime = VideoRuntime::new();
        let id = VideoId(1);
        runtime
            .push_frame(id, 2, 2, frame_bytes(2, 2), None)
            .unwrap();
        runtime
            .push_frame(id, 4, 2, frame_bytes(4, 2), None)
            .unwrap();
        let upload = runtime.take_upload(id, None).expect("pending upload");
        assert_eq!(upload.width, 4);
        assert_eq!(upload.version, 2);
        assert_eq!(
            upload.dirty,
            VideoRect {
                x: 0,
                y: 0,
                width: 4,
                height: 2
            }
        );
    }

    #[test]
    fn take_upload_deduplicates_uploaded_version() {
        let mut runtime = VideoRuntime::new();
        let id = VideoId(1);
        runtime
            .push_frame(id, 2, 2, frame_bytes(2, 2), None)
            .unwrap();
        let version = runtime.take_upload(id, None).unwrap().version;
        assert!(runtime.take_upload(id, Some(version)).is_none());
        assert!(runtime.take_upload(id, None).is_some());
        runtime
            .push_frame(id, 2, 2, frame_bytes(2, 2), None)
            .unwrap();
        assert!(runtime.take_upload(id, Some(version)).is_some());
    }

    #[test]
    fn push_frame_validates_byte_count() {
        let mut runtime = VideoRuntime::new();
        let id = VideoId(1);
        assert!(runtime.push_frame(id, 2, 2, vec![0; 12], None).is_err());
        assert!(!runtime.contains(id));
    }

    #[test]
    fn remove_drops_surface() {
        let mut runtime = VideoRuntime::new();
        let id = VideoId(3);
        runtime
            .push_frame(id, 2, 2, frame_bytes(2, 2), None)
            .unwrap();
        assert!(runtime.remove(id));
        assert!(!runtime.contains(id));
        assert!(runtime.take_upload(id, None).is_none());
    }

    #[test]
    fn dirty_rect_clamps_to_frame() {
        let mut runtime = VideoRuntime::new();
        let id = VideoId(1);
        runtime
            .push_frame(
                id,
                4,
                4,
                frame_bytes(4, 4),
                Some(VideoRect {
                    x: 2,
                    y: 2,
                    width: u32::MAX,
                    height: u32::MAX,
                }),
            )
            .unwrap();
        let upload = runtime.take_upload(id, None).unwrap();
        assert_eq!(
            upload.dirty,
            VideoRect {
                x: 2,
                y: 2,
                width: 2,
                height: 2
            }
        );
    }
}
