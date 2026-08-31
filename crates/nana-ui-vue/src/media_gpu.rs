//! Upload `<video>` / camera preview frames onto the host Device/Queue.

use nana_ui::{HostTextureBinding, HostTextureRegistry, HostedGpuResources};
use nana_ui_web_api::{MediaId, MediaRuntime, SharedMediaRuntime};

use crate::canvas_gpu::{HostTextureSlotStore, HostTextureUpload};

const VIDEO_TEXTURE_ID_BIT: u64 = 1 << 61;

/// Shared by every Vue window in one runtime, just like Canvas/SVG GPU bridges.
#[derive(Clone)]
pub(crate) struct MediaGpuBridge {
    media: SharedMediaRuntime,
    store: HostTextureSlotStore<MediaId>,
}

impl std::fmt::Debug for MediaGpuBridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MediaGpuBridge")
            .finish_non_exhaustive()
    }
}

impl MediaGpuBridge {
    pub(crate) fn new(
        resources: HostedGpuResources,
        media: SharedMediaRuntime,
        textures: HostTextureRegistry,
    ) -> Self {
        Self {
            media,
            store: HostTextureSlotStore::new(resources, textures),
        }
    }

    pub(crate) fn replace_device(&self, resources: HostedGpuResources) {
        self.store.replace_device(resources);
    }

    pub(crate) fn prune_released(&self, live_slots: &std::collections::HashSet<String>) {
        self.store
            .retain(|_, binding| live_slots.contains(binding.slot.as_str()));
    }

    pub(crate) fn tick_playing(&self) {
        if let Ok(mut media) = self.media.lock() {
            media.tick_playing();
        }
    }

    pub(crate) fn sync(&self, id: MediaId) -> Result<Option<HostTextureBinding>, String> {
        let uploaded_version = self.store.uploaded_version(&id);
        let upload = self
            .media
            .lock()
            .map_err(|_| "media runtime poisoned".to_owned())?
            .take_upload(id, uploaded_version)
            .map_err(|error| error.to_string())?;

        let Some(upload) = upload else {
            return Ok(self.store.binding(&id));
        };
        let slot = MediaRuntime::slot(id);

        self.store
            .sync(
                id,
                HostTextureUpload {
                    slot: &slot,
                    texture_id: VIDEO_TEXTURE_ID_BIT | id.0,
                    width: upload.width,
                    height: upload.height,
                    version: upload.version,
                    bytes: &upload.bytes,
                    dirty_x: 0,
                    dirty_y: 0,
                    dirty_width: upload.width,
                    dirty_height: upload.height,
                    label: "NanaUI video texture",
                },
            )
            .map_err(|_| "media GPU state poisoned".to_owned())
    }
}
