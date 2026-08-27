//! Hosted Canvas/image compositor using stable WGPU textures and dirty uploads.

use nana_ui::{HostTextureBinding, HostTextureRegistry, HostedGpuResources};
use nana_ui_web_api::{CanvasId, SharedCanvasRuntime};

#[path = "host_texture_upload.rs"]
mod host_texture_upload;
pub(crate) use host_texture_upload::{HostTextureSlotStore, HostTextureUpload};

const CANVAS_TEXTURE_ID_BIT: u64 = 1 << 63;

/// Shared by every Vue window in one runtime, just like the Canvas resource
/// store and host texture registry it connects.
#[derive(Clone)]
pub(crate) struct CanvasGpuBridge {
    canvas: SharedCanvasRuntime,
    store: HostTextureSlotStore<CanvasId>,
}

impl std::fmt::Debug for CanvasGpuBridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanvasGpuBridge")
            .finish_non_exhaustive()
    }
}

impl CanvasGpuBridge {
    pub(crate) fn new(
        resources: HostedGpuResources,
        canvas: SharedCanvasRuntime,
        textures: HostTextureRegistry,
    ) -> Self {
        Self {
            canvas,
            store: HostTextureSlotStore::new(resources, textures),
        }
    }

    pub(crate) fn replace_device(&self, resources: HostedGpuResources) {
        self.store.replace_device(resources);
    }

    pub(crate) fn sync(&self, id: CanvasId) -> Result<Option<HostTextureBinding>, String> {
        self.prune_released();
        let uploaded_version = self.store.uploaded_version(&id);
        let upload = self
            .canvas
            .lock()
            .map_err(|_| "Canvas runtime poisoned".to_owned())?
            .take_upload(id, uploaded_version)
            .map_err(|error| error.to_string())?;

        let Some(upload) = upload else {
            return Ok(self.store.binding(&id));
        };
        let slot = slot(id);

        self.store
            .sync(
                id,
                HostTextureUpload {
                    slot: &slot,
                    texture_id: CANVAS_TEXTURE_ID_BIT | id.0,
                    width: upload.width,
                    height: upload.height,
                    version: upload.version,
                    bytes: &upload.bytes,
                    dirty_x: upload.dirty_x,
                    dirty_y: upload.dirty_y,
                    dirty_width: upload.dirty_width,
                    dirty_height: upload.dirty_height,
                    label: "NanaUI Canvas texture",
                },
            )
            .map_err(|_| "Canvas GPU state poisoned".to_owned())
    }

    fn prune_released(&self) {
        let Ok(canvas) = self.canvas.lock() else {
            return;
        };
        self.store.retain(|id, _| canvas.contains(*id));
    }
}

pub(crate) fn slot(id: CanvasId) -> String {
    format!("canvas:{}", id.0)
}
