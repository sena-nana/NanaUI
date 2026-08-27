//! Shared HostTexture create / dirty-upload / prune for Vue surface bridges.
//! Canvas dirty rects stay rectangular; do not expand them to a full write.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use nana_ui::{
    HostTexture, HostTextureAlphaMode, HostTextureBinding, HostTextureRegistry, HostedGpuResources,
};

pub(crate) struct HostTextureUpload<'a> {
    pub slot: &'a str,
    pub texture_id: u64,
    pub width: u32,
    pub height: u32,
    pub version: u64,
    pub bytes: &'a [u8],
    pub dirty_x: u32,
    pub dirty_y: u32,
    pub dirty_width: u32,
    pub dirty_height: u32,
    pub label: &'static str,
}

struct SlotEntry {
    _texture: wgpu::Texture,
    binding: HostTextureBinding,
    width: u32,
    height: u32,
    version: u64,
}

struct SlotStoreState<K> {
    resources: HostedGpuResources,
    device_generation: u64,
    entries: HashMap<K, SlotEntry>,
}

#[derive(Clone)]
pub(crate) struct HostTextureSlotStore<K> {
    textures: HostTextureRegistry,
    state: Arc<Mutex<SlotStoreState<K>>>,
}

impl<K: Eq + Hash + Clone> HostTextureSlotStore<K> {
    pub(crate) fn new(resources: HostedGpuResources, textures: HostTextureRegistry) -> Self {
        Self {
            textures,
            state: Arc::new(Mutex::new(SlotStoreState {
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

    pub(crate) fn binding(&self, key: &K) -> Option<HostTextureBinding> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.entries.get(key).map(|entry| entry.binding.clone()))
    }

    pub(crate) fn uploaded_version(&self, key: &K) -> Option<u64> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.entries.get(key).map(|entry| entry.version))
    }

    pub(crate) fn retain(&self, mut keep: impl FnMut(&K, &HostTextureBinding) -> bool) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let drop_keys: Vec<K> = state
            .entries
            .iter()
            .filter(|(key, entry)| !keep(key, &entry.binding))
            .map(|(key, _)| key.clone())
            .collect();
        for key in drop_keys {
            if let Some(entry) = state.entries.remove(&key) {
                self.textures.remove(&entry.binding.slot);
            }
        }
    }

    pub(crate) fn sync(
        &self,
        key: K,
        upload: HostTextureUpload<'_>,
    ) -> Result<Option<HostTextureBinding>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "host texture GPU state poisoned".to_owned())?;
        let recreate = state
            .entries
            .get(&key)
            .is_none_or(|entry| entry.width != upload.width || entry.height != upload.height);
        if recreate {
            let prior_generation = state
                .entries
                .remove(&key)
                .map(|entry| entry.binding.texture.generation())
                .unwrap_or_else(|| state.device_generation.saturating_sub(1));
            let texture = state
                .resources
                .device()
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(upload.label),
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
                upload.texture_id,
                prior_generation
                    .saturating_add(1)
                    .max(state.device_generation),
                view,
            );
            let binding = self.textures.register(
                upload.slot,
                host,
                upload.width,
                upload.height,
                HostTextureAlphaMode::Premultiplied,
            );
            state.entries.insert(
                key.clone(),
                SlotEntry {
                    _texture: texture,
                    binding,
                    width: upload.width,
                    height: upload.height,
                    version: 0,
                },
            );
        }

        let queue = state.resources.queue().clone();
        let entry = state
            .entries
            .get_mut(&key)
            .expect("host texture slot created");
        if entry.version != upload.version {
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
                    upload.bytes,
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
        }
        Ok(Some(entry.binding.clone()))
    }
}
