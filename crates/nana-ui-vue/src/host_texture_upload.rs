//! Shared HostTexture create / dirty-upload / prune for Vue surface bridges.
//! Canvas dirty rects stay rectangular; do not expand them to a full write.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use nana_ui::{
    GpuContext, GpuTexture, GpuTextureDescriptor, GpuTextureFormat, GpuTextureRegion,
    GpuTextureUsages, HostTexture, HostTextureAlphaMode, HostTextureBinding, HostTextureRegistry,
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
    texture: GpuTexture,
    binding: HostTextureBinding,
    width: u32,
    height: u32,
    version: u64,
}

struct SlotStoreState<K> {
    gpu: GpuContext,
    /// Floor for the HostTexture generation of slots created from now on;
    /// bumped per device so a slot recreated on a new device never reuses a
    /// generation the painter already bound.
    texture_epoch: u64,
    entries: HashMap<K, SlotEntry>,
}

#[derive(Clone)]
pub(crate) struct HostTextureSlotStore<K> {
    textures: HostTextureRegistry,
    state: Arc<Mutex<SlotStoreState<K>>>,
}

impl<K: Eq + Hash + Clone> HostTextureSlotStore<K> {
    pub(crate) fn new(gpu: GpuContext, textures: HostTextureRegistry) -> Self {
        Self {
            textures,
            state: Arc::new(Mutex::new(SlotStoreState {
                gpu,
                texture_epoch: 1,
                entries: HashMap::new(),
            })),
        }
    }

    pub(crate) fn replace_device(&self, gpu: GpuContext) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        for entry in state.entries.values() {
            self.textures.remove(&entry.binding.slot);
        }
        state.entries.clear();
        state.gpu = gpu;
        state.texture_epoch = state.texture_epoch.saturating_add(1).max(1);
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
                .get(&key)
                .map(|entry| entry.binding.texture.generation())
                .unwrap_or_else(|| state.texture_epoch.saturating_sub(1));
            // Created before the old entry goes: a size the device refuses
            // keeps the slot the store already tracks (and so still removes
            // on prune or device replacement) instead of orphaning it.
            let texture = state
                .gpu
                .create_texture(&GpuTextureDescriptor {
                    label: Some(upload.label),
                    width: upload.width.max(1),
                    height: upload.height.max(1),
                    format: GpuTextureFormat::RGBA8_UNORM_SRGB,
                    usage: GpuTextureUsages::COPY_DST | GpuTextureUsages::SAMPLED,
                })
                .map_err(|error| format!("{}: {error}", upload.label))?;
            let host = HostTexture::new(
                upload.texture_id,
                prior_generation.saturating_add(1).max(state.texture_epoch),
                &texture,
            );
            state.entries.remove(&key);
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
                    texture,
                    binding,
                    width: upload.width,
                    height: upload.height,
                    version: 0,
                },
            );
        }

        let state = &mut *state;
        let entry = state
            .entries
            .get_mut(&key)
            .expect("host texture slot created");
        if entry.version != upload.version {
            if upload.dirty_width > 0 && upload.dirty_height > 0 {
                state
                    .gpu
                    .write_texture(
                        &entry.texture,
                        GpuTextureRegion {
                            x: upload.dirty_x,
                            y: upload.dirty_y,
                            width: upload.dirty_width,
                            height: upload.dirty_height,
                        },
                        upload.bytes,
                        upload.dirty_width * 4,
                    )
                    .map_err(|error| format!("{}: {error}", upload.label))?;
            }
            entry.version = upload.version;
            self.textures.invalidate(&entry.binding.slot);
        }
        Ok(Some(entry.binding.clone()))
    }
}
