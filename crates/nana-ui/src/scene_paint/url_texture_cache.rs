//! Shared URL texture lifecycle for quads and HostTexture masks.
use std::{
    cell::Cell,
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use super::image_url::decode_url_rgba;

const MAX_FETCHES: usize = 4;
const RETAINED_BYTES: u64 = 64 * 1024 * 1024;
const RETAINED_ENTRIES: usize = 256;
const IDLE_FRAMES: u64 = 120;

pub(crate) type ImageWake = Arc<dyn Fn() + Send + Sync>;
type Decoded = Option<(u32, u32, Vec<u8>)>;

pub(crate) struct CachedUrlTexture {
    pub(crate) view: wgpu::TextureView,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

struct Entry {
    texture: Option<CachedUrlTexture>,
    decoded: Option<(u32, u32, Vec<u8>)>,
    used: Cell<u64>,
}

/// Retains the current working set plus a bounded LRU of inactive textures.
/// HTTP work is limited to four concurrent requests per painter pipeline.
#[derive(Default)]
pub(crate) struct UrlTextureCache {
    entries: HashMap<String, Entry>,
    pending: HashMap<String, mpsc::Receiver<Decoded>>,
    ready: Arc<AtomicBool>,
    wake: Option<ImageWake>,
    frame: u64,
    deferred: bool,
}

impl UrlTextureCache {
    pub(crate) fn set_wake(&mut self, wake: ImageWake) {
        self.wake = Some(wake);
    }
    pub(crate) fn has_updates(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty() || self.deferred
    }
    pub(crate) fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.deferred = false;
    }
    pub(crate) fn get(&self, key: &str) -> Option<&Option<CachedUrlTexture>> {
        self.entries.get(key).map(|entry| {
            entry.used.set(self.frame);
            &entry.texture
        })
    }
    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
    pub(crate) fn insert(&mut self, key: String, texture: Option<CachedUrlTexture>) {
        self.entries.insert(
            key,
            Entry {
                texture,
                decoded: None,
                used: Cell::new(self.frame),
            },
        );
    }
    pub(crate) fn contains_retained(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub(crate) fn load(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        url: &str,
    ) -> Option<(u32, u32)> {
        if let Some(entry) = self.entries.get_mut(url)
            && let Some((width, height, rgba)) = entry.decoded.take()
        {
            entry.texture = upload(device, queue, (width, height, &rgba));
        }
        if let Some(cached) = self.get(url) {
            return cached.as_ref().map(|entry| (entry.width, entry.height));
        }
        if self.pending.contains_key(url) {
            return None;
        }
        if url.trim().starts_with("http://") || url.trim().starts_with("https://") {
            if self.pending.len() >= MAX_FETCHES {
                self.deferred = true;
                return None;
            }
            let (tx, rx) = mpsc::channel();
            let key = url.to_owned();
            let ready = self.ready.clone();
            let wake = self.wake.clone();
            // Workers own only CPU bytes. Texture creation/upload stays on the host.
            let spawned = std::thread::Builder::new()
                .name("nana-image".into())
                .spawn(move || {
                    let decoded = decode_url_rgba(&key);
                    if tx.send(decoded).is_ok() {
                        ready.store(true, Ordering::Release);
                        if let Some(wake) = wake {
                            wake();
                        }
                    }
                });
            if spawned.is_ok() {
                self.pending.insert(url.to_owned(), rx);
            } else {
                self.insert(url.to_owned(), None);
            }
            return None;
        }
        let texture = decode_url_rgba(url)
            .and_then(|(width, height, rgba)| upload(device, queue, (width, height, &rgba)));
        let size = texture
            .as_ref()
            .map(|texture| (texture.width, texture.height));
        self.insert(url.to_owned(), texture);
        size
    }

    /// Collect CPU results. Only a subsequent live URL lookup may upload them.
    pub(crate) fn poll(&mut self) -> bool {
        self.ready.store(false, Ordering::Release);
        let mut complete = Vec::new();
        self.pending
            .retain(|key, receiver| match receiver.try_recv() {
                Ok(decoded) => {
                    complete.push((key.clone(), decoded));
                    false
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    complete.push((key.clone(), None));
                    false
                }
                Err(mpsc::TryRecvError::Empty) => true,
            });
        let changed = !complete.is_empty();
        for (key, decoded) in complete {
            self.entries.insert(
                key,
                Entry {
                    texture: None,
                    decoded,
                    used: Cell::new(self.frame.saturating_sub(1)),
                },
            );
        }
        changed
    }

    pub(crate) fn trim(&mut self) {
        let mut inactive = Vec::new();
        let mut bytes = 0;
        for (key, entry) in &self.entries {
            if entry.used.get() == self.frame {
                continue;
            }
            let size = entry.decoded.as_ref().map_or_else(
                || {
                    entry
                        .texture
                        .as_ref()
                        .map_or(0, |t| u64::from(t.width) * u64::from(t.height) * 4)
                },
                |(_, _, bytes)| bytes.len() as u64,
            );
            bytes += size;
            inactive.push((entry.used.get(), key.clone(), size));
        }
        inactive.sort_unstable_by_key(|entry| entry.0);
        let mut count = inactive.len();
        for (used, key, size) in inactive {
            if count <= RETAINED_ENTRIES
                && bytes <= RETAINED_BYTES
                && self.frame.saturating_sub(used) <= IDLE_FRAMES
            {
                break;
            }
            self.entries.remove(&key);
            count -= 1;
            bytes -= size;
        }
    }
}

pub(crate) fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    (width, height, rgba): (u32, u32, &[u8]),
) -> Option<CachedUrlTexture> {
    let limit = device.limits().max_texture_dimension_2d;
    if width == 0 || height == 0 || width > limit || height > limit {
        return None;
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("nana-ui.scene.url"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    Some(CachedUrlTexture {
        view: texture.create_view(&Default::default()),
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_entries_are_bounded_and_eventually_released() {
        let mut cache = UrlTextureCache::default();
        cache.begin_frame();
        for id in 0..1000 {
            cache.insert(format!("failed:{id}"), None);
        }
        cache.trim();
        cache.begin_frame();
        cache.trim();
        assert!(cache.entries.len() <= RETAINED_ENTRIES);
        let kept = cache.entries.keys().next().unwrap().clone();
        for _ in 0..=IDLE_FRAMES {
            cache.begin_frame();
            cache.get(&kept);
            cache.trim();
        }
        assert_eq!(
            cache.entries.len(),
            1,
            "only the referenced image may survive expiry"
        );
        assert!(cache.contains_retained(&kept));
    }

    #[test]
    fn completed_unused_images_stay_on_cpu_and_obey_residency_budget() {
        let mut cache = UrlTextureCache::default();
        cache.begin_frame();
        for id in 0..5 {
            let (sender, receiver) = mpsc::channel();
            cache.pending.insert(format!("old:{id}"), receiver);
            sender
                .send(Some((2048, 2048, vec![255; 2048 * 2048 * 4])))
                .unwrap();
        }
        cache.begin_frame();
        assert!(cache.poll());
        assert!(cache.entries.values().all(|entry| entry.texture.is_none()));
        cache.trim();
        let bytes: usize = cache
            .entries
            .values()
            .filter_map(|entry| entry.decoded.as_ref())
            .map(|(_, _, bytes)| bytes.len())
            .sum();
        assert!(bytes <= RETAINED_BYTES as usize);
    }
}
