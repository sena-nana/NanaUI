//! Hosted Canvas/image compositor using stable WGPU textures and dirty uploads.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use iced::wgpu;
use nana_ui::{
    HostTexture, HostTextureAlphaMode, HostTextureBinding, HostTextureRegistry, HostedGpuResources,
};
use nana_ui_web_api::{CanvasId, SharedCanvasRuntime};

const CANVAS_TEXTURE_ID_BIT: u64 = 1 << 63;

struct CanvasGpuEntry {
    _texture: wgpu::Texture,
    binding: HostTextureBinding,
    width: u32,
    height: u32,
    version: u64,
}

struct CanvasGpuState {
    resources: HostedGpuResources,
    device_generation: u64,
    entries: HashMap<CanvasId, CanvasGpuEntry>,
}

/// Shared by every Vue window in one runtime, just like the Canvas resource
/// store and host texture registry it connects.
#[derive(Clone)]
pub(crate) struct CanvasGpuBridge {
    canvas: SharedCanvasRuntime,
    textures: HostTextureRegistry,
    state: Arc<Mutex<CanvasGpuState>>,
}

impl std::fmt::Debug for CanvasGpuBridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries = self
            .state
            .lock()
            .map(|state| state.entries.len())
            .unwrap_or_default();
        formatter
            .debug_struct("CanvasGpuBridge")
            .field("entries", &entries)
            .finish()
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
            textures,
            state: Arc::new(Mutex::new(CanvasGpuState {
                resources,
                device_generation: 1,
                entries: HashMap::new(),
            })),
        }
    }

    pub(crate) fn replace_device(&self, resources: HostedGpuResources) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        for entry in state.entries.values() {
            self.textures.remove(&entry.binding.slot);
        }
        state.entries.clear();
        state.resources = resources;
        state.device_generation = state.device_generation.saturating_add(1).max(1);
    }

    pub(crate) fn sync(&self, id: CanvasId) -> Result<Option<HostTextureBinding>, String> {
        self.prune_released();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Canvas GPU state poisoned".to_owned())?;
        let uploaded_version = state.entries.get(&id).map(|entry| entry.version);
        let upload = self
            .canvas
            .lock()
            .map_err(|_| "Canvas runtime poisoned".to_owned())?
            .take_upload(id, uploaded_version)
            .map_err(|error| error.to_string())?;

        let Some(upload) = upload else {
            return Ok(state.entries.get(&id).map(|entry| entry.binding.clone()));
        };

        let recreate = state
            .entries
            .get(&id)
            .is_none_or(|entry| entry.width != upload.width || entry.height != upload.height);
        if recreate {
            let prior_generation = state
                .entries
                .remove(&id)
                .map(|entry| entry.binding.texture.generation())
                .unwrap_or_else(|| state.device_generation.saturating_sub(1));
            let texture = state
                .resources
                .device()
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("NanaUI Canvas texture"),
                    size: wgpu::Extent3d {
                        width: upload.width.max(1),
                        height: upload.height.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let host = HostTexture::from_wgpu(
                CANVAS_TEXTURE_ID_BIT | id.0,
                prior_generation
                    .saturating_add(1)
                    .max(state.device_generation),
                view,
            );
            let binding = self.textures.register(
                slot(id),
                host,
                upload.width,
                upload.height,
                HostTextureAlphaMode::Premultiplied,
            );
            state.entries.insert(
                id,
                CanvasGpuEntry {
                    _texture: texture,
                    binding,
                    width: upload.width,
                    height: upload.height,
                    version: 0,
                },
            );
        }

        let queue = state.resources.queue().clone();
        let entry = state.entries.get_mut(&id).expect("Canvas entry created");
        if upload.dirty_width > 0 && upload.dirty_height > 0 {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &entry._texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: upload.dirty_x,
                        y: upload.dirty_y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &upload.bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(upload.dirty_width * 4),
                    rows_per_image: Some(upload.dirty_height),
                },
                wgpu::Extent3d {
                    width: upload.dirty_width,
                    height: upload.dirty_height,
                    depth_or_array_layers: 1,
                },
            );
        }
        entry.version = upload.version;
        self.textures.invalidate(&entry.binding.slot);
        Ok(Some(entry.binding.clone()))
    }

    fn prune_released(&self) {
        let Ok(canvas) = self.canvas.lock() else {
            return;
        };
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let released = state
            .entries
            .keys()
            .copied()
            .filter(|id| !canvas.contains(*id))
            .collect::<Vec<_>>();
        for id in released {
            if let Some(entry) = state.entries.remove(&id) {
                self.textures.remove(&entry.binding.slot);
            }
        }
    }
}

pub(crate) fn slot(id: CanvasId) -> String {
    format!("canvas:{}", id.0)
}
