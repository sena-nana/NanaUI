//! Upload rasterized generic SVG pixmaps onto the host Device/Queue.

use std::collections::HashSet;

use nana_ui::{HostTextureBinding, HostTextureRegistry, HostedGpuResources};

use crate::canvas_gpu::{HostTextureSlotStore, HostTextureUpload};
use crate::svg_raster::SvgHostUpload;

const SVG_TEXTURE_ID_BIT: u64 = 1 << 62;

#[derive(Clone)]
pub(crate) struct SvgGpuBridge {
    store: HostTextureSlotStore<String>,
}

impl std::fmt::Debug for SvgGpuBridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SvgGpuBridge")
            .finish_non_exhaustive()
    }
}

impl SvgGpuBridge {
    pub(crate) fn new(resources: HostedGpuResources, textures: HostTextureRegistry) -> Self {
        Self {
            store: HostTextureSlotStore::new(resources, textures),
        }
    }

    pub(crate) fn replace_device(&self, resources: HostedGpuResources) {
        self.store.replace_device(resources);
    }

    pub(crate) fn prune_released(&self, live_slots: &HashSet<String>) {
        self.store
            .retain(|slot, _| live_slots.contains(slot.as_str()));
    }

    pub(crate) fn sync(
        &self,
        upload: &SvgHostUpload,
    ) -> Result<Option<HostTextureBinding>, String> {
        self.store
            .sync(
                upload.slot.clone(),
                HostTextureUpload {
                    slot: &upload.slot,
                    texture_id: SVG_TEXTURE_ID_BIT | upload.node,
                    width: upload.raster.width,
                    height: upload.raster.height,
                    version: upload.version,
                    bytes: upload.raster.rgba.as_ref(),
                    dirty_x: 0,
                    dirty_y: 0,
                    dirty_width: upload.raster.width,
                    dirty_height: upload.raster.height,
                    label: "NanaUI SVG texture",
                },
            )
            .map_err(|_| "SVG GPU state poisoned".to_owned())
    }
}
