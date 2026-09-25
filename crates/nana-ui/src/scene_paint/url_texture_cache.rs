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

use nana_ui_platform::{FetchCancellation, SharedFetchHost};

use super::image_url::{decode_url_rgba, decode_url_rgba_with};

const MAX_FETCHES: usize = 4;
const RETAINED_BYTES: u64 = 64 * 1024 * 1024;
const RETAINED_ENTRIES: usize = 256;
const IDLE_FRAMES: u64 = 120;

pub(crate) type ImageWake = Arc<dyn Fn(&str) + Send + Sync>;
type Decoded = Option<(u32, u32, Vec<u8>)>;

/// Which `FetchHost` a result was loaded through: the address of that host's
/// `Arc`. Results that did not touch the network — local files, `data:`,
/// generated keys, and remote URLs refused for want of a host — do not depend
/// on any host and share [`LOCAL`].
pub(crate) type Egress = usize;
const LOCAL: Egress = 0;

/// Identity of a host; `None` is [`LOCAL`].
pub(crate) fn egress_of(host: Option<&SharedFetchHost>) -> Egress {
    host.map_or(LOCAL, |host| Arc::as_ptr(host).cast::<()>() as usize)
}

fn is_remote(url: &str) -> bool {
    let url = url.trim();
    url.starts_with("http://") || url.starts_with("https://")
}

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

/// An in-flight fetch, with the token that can stop it.
///
/// `used` is the same liveness stamp `Entry` carries: [`UrlTextureCache::load`]
/// refreshes it on every frame that still wants this URL, so a stale stamp means
/// nobody is waiting for the result any more.
struct Pending {
    receiver: mpsc::Receiver<Decoded>,
    cancellation: FetchCancellation,
    used: Cell<u64>,
}

#[cfg(test)]
impl Pending {
    fn for_test(receiver: mpsc::Receiver<Decoded>, used: u64) -> Self {
        Self {
            receiver,
            cancellation: FetchCancellation::new(),
            used: Cell::new(used),
        }
    }
}

/// Everything loaded through one egress.
#[derive(Default)]
struct Bucket {
    /// Keeps the address behind a remote bucket's [`Egress`] from being reused
    /// by another host while results loaded through this one are cached.
    /// Released with the bucket once it holds nothing.
    _host: Option<SharedFetchHost>,
    entries: HashMap<String, Entry>,
    pending: HashMap<String, Pending>,
}

/// Retains the current working set plus a bounded LRU of inactive textures.
/// HTTP work is limited to four concurrent requests per painter pipeline.
///
/// Remote results are partitioned by the fetch host they went through, so a
/// painter shared by documents with different policies never serves one
/// document an image only another document's policy admitted.
#[derive(Default)]
pub(crate) struct UrlTextureCache {
    buckets: HashMap<Egress, Bucket>,
    fetch_host: Option<SharedFetchHost>,
    ready: Arc<AtomicBool>,
    wake: Option<ImageWake>,
    frame: u64,
    deferred: bool,
}

impl UrlTextureCache {
    pub(crate) fn set_wake(&mut self, wake: ImageWake) {
        self.wake = Some(wake);
    }
    /// Egress for the remote URLs of the document painted next.
    pub(crate) fn set_fetch_host(&mut self, host: Option<SharedFetchHost>) {
        self.fetch_host = host;
    }
    fn egress(&self, url: &str) -> Egress {
        if is_remote(url) {
            egress_of(self.fetch_host.as_ref())
        } else {
            LOCAL
        }
    }
    /// Stop the in-flight requests started through `host` and drop what it
    /// loaded, releasing the bucket's hold on the host.
    pub(crate) fn release_fetch_host(&mut self, host: &SharedFetchHost) {
        if let Some(bucket) = self.buckets.remove(&egress_of(Some(host))) {
            for pending in bucket.pending.values() {
                pending.cancellation.cancel();
            }
        }
    }
    fn bucket_mut(&mut self, egress: Egress) -> &mut Bucket {
        let host = &self.fetch_host;
        self.buckets.entry(egress).or_insert_with(|| Bucket {
            _host: host.clone().filter(|_| egress != LOCAL),
            ..Bucket::default()
        })
    }
    fn pending_len(&self) -> usize {
        self.buckets
            .values()
            .map(|bucket| bucket.pending.len())
            .sum()
    }
    pub(crate) fn has_updates(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub(crate) fn has_pending(&self) -> bool {
        self.pending_len() > 0 || self.deferred
    }
    pub(crate) fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.deferred = false;
    }
    pub(crate) fn get(&self, key: &str) -> Option<&Option<CachedUrlTexture>> {
        self.buckets
            .get(&self.egress(key))?
            .entries
            .get(key)
            .map(|entry| {
                entry.used.set(self.frame);
                &entry.texture
            })
    }
    pub(crate) fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
    pub(crate) fn insert(&mut self, key: String, texture: Option<CachedUrlTexture>) {
        let used = Cell::new(self.frame);
        self.bucket_mut(self.egress(&key)).entries.insert(
            key,
            Entry {
                texture,
                decoded: None,
                used,
            },
        );
    }
    pub(crate) fn contains_retained(&self, key: &str) -> bool {
        self.buckets
            .get(&self.egress(key))
            .is_some_and(|bucket| bucket.entries.contains_key(key))
    }

    #[allow(dead_code)]
    pub(crate) fn load(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        url: &str,
    ) -> Option<(u32, u32)> {
        self.load_with_work(device, queue, url, None)
    }

    pub(crate) fn load_with_work(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        url: &str,
        work: Option<&crate::gpu_work::GpuWorkSink>,
    ) -> Option<(u32, u32)> {
        let egress = self.egress(url);
        let frame = self.frame;
        if let Some(bucket) = self.buckets.get_mut(&egress) {
            if let Some(entry) = bucket.entries.get_mut(url) {
                if let Some((width, height, rgba)) = entry.decoded.take() {
                    entry.texture = upload_with_work(device, queue, (width, height, &rgba), work);
                }
                entry.used.set(frame);
                return entry
                    .texture
                    .as_ref()
                    .map(|texture| (texture.width, texture.height));
            }
            if let Some(pending) = bucket.pending.get(url) {
                pending.used.set(frame);
                return None;
            }
        }
        if is_remote(url) {
            // No host means no egress: refused here, before any socket.
            let Some(host) = self.fetch_host.clone() else {
                self.insert(url.to_owned(), None);
                return None;
            };
            if self.pending_len() >= MAX_FETCHES {
                self.deferred = true;
                return None;
            }
            let (tx, rx) = mpsc::channel();
            let key = url.to_owned();
            let ready = self.ready.clone();
            let wake = self.wake.clone();
            let cancellation = FetchCancellation::new();
            let worker_cancellation = cancellation.clone();
            // Workers own only CPU bytes. Texture creation/upload stays on the
            // host. The worker captures the requesting document's host, so a
            // later paint with another host cannot redirect this request.
            let spawned = std::thread::Builder::new()
                .name("nana-image".into())
                .spawn(move || {
                    let decoded = decode_url_rgba_with(&key, Some(&host), &worker_cancellation);
                    if tx.send(decoded).is_ok() {
                        ready.store(true, Ordering::Release);
                        if let Some(wake) = wake {
                            wake(&key);
                        }
                    }
                });
            if spawned.is_ok() {
                self.bucket_mut(egress).pending.insert(
                    url.to_owned(),
                    Pending {
                        receiver: rx,
                        cancellation,
                        used: Cell::new(frame),
                    },
                );
            } else {
                self.insert(url.to_owned(), None);
            }
            return None;
        }
        let texture = decode_url_rgba(url).and_then(|(width, height, rgba)| {
            upload_with_work(device, queue, (width, height, &rgba), work)
        });
        let size = texture
            .as_ref()
            .map(|texture| (texture.width, texture.height));
        self.insert(url.to_owned(), texture);
        size
    }

    /// Collect CPU results. Only a subsequent live URL lookup may upload them.
    pub(crate) fn poll(&mut self) -> bool {
        self.ready.store(false, Ordering::Release);
        let used = self.frame.saturating_sub(1);
        let mut changed = false;
        for bucket in self.buckets.values_mut() {
            let mut complete = Vec::new();
            bucket
                .pending
                .retain(|key, pending| match pending.receiver.try_recv() {
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
            changed |= !complete.is_empty();
            for (key, decoded) in complete {
                bucket.entries.insert(
                    key,
                    Entry {
                        texture: None,
                        decoded,
                        used: Cell::new(used),
                    },
                );
            }
        }
        changed
    }

    /// Drop in-flight fetches nobody has asked for in `IDLE_FRAMES`.
    ///
    /// The threshold is deliberately generous: an image scrolled out of view for
    /// a few frames is cheaper to let finish than to cancel and re-request when
    /// it scrolls back. Two seconds of nobody asking means it was abandoned.
    ///
    /// Cancelled entries are removed outright rather than left for [`Self::poll`]
    /// to reap, because `poll` records a dead receiver as `None` — a permanent
    /// "this URL failed" in `entries`. A cancellation is not a failure: dropping
    /// the entry returns the URL to "never loaded", so a node that comes back
    /// re-requests it.
    fn cancel_unreferenced_fetches(&mut self) {
        let frame = self.frame;
        for bucket in self.buckets.values_mut() {
            bucket.pending.retain(|_, pending| {
                if frame.saturating_sub(pending.used.get()) <= IDLE_FRAMES {
                    return true;
                }
                pending.cancellation.cancel();
                false
            });
        }
    }

    pub(crate) fn trim(&mut self) {
        self.cancel_unreferenced_fetches();
        let mut inactive = Vec::new();
        let mut bytes = 0;
        for (egress, bucket) in &self.buckets {
            for (key, entry) in &bucket.entries {
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
                inactive.push((entry.used.get(), *egress, key.clone(), size));
            }
        }
        inactive.sort_unstable_by_key(|entry| entry.0);
        let mut count = inactive.len();
        for (used, egress, key, size) in inactive {
            if count <= RETAINED_ENTRIES
                && bytes <= RETAINED_BYTES
                && self.frame.saturating_sub(used) <= IDLE_FRAMES
            {
                break;
            }
            if let Some(bucket) = self.buckets.get_mut(&egress) {
                bucket.entries.remove(&key);
            }
            count -= 1;
            bytes -= size;
        }
        // An emptied bucket releases its host: nothing loaded through it is
        // left to protect from a reused address.
        self.buckets
            .retain(|_, bucket| !bucket.entries.is_empty() || !bucket.pending.is_empty());
    }
}

/// Stop in-flight fetches when the pipeline goes away.
///
/// The worker threads are detached, so without this a closed window leaves them
/// parked on a socket until the policy timeout with nobody left to receive the
/// result. Cancelling shuts the socket down; the worker's `send` then fails
/// against the dropped receiver and it exits.
///
/// This ends the network wait, not an in-progress decode — `image` and `resvg`
/// have no cancellation point, so a worker already decoding runs to completion.
impl Drop for UrlTextureCache {
    fn drop(&mut self) {
        for bucket in self.buckets.values() {
            for pending in bucket.pending.values() {
                pending.cancellation.cancel();
            }
        }
    }
}

pub(crate) fn upload_with_work(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    (width, height, rgba): (u32, u32, &[u8]),
    work: Option<&crate::gpu_work::GpuWorkSink>,
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
    if let Some(work) = work {
        work.write_texture(
            queue,
            &texture,
            [0, 0, 0],
            rgba,
            4 * width,
            height,
            [width, height, 1],
        );
    } else {
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
    }
    Some(CachedUrlTexture {
        view: texture.create_view(&Default::default()),
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    impl UrlTextureCache {
        fn entry_keys(&self) -> Vec<&String> {
            self.buckets
                .values()
                .flat_map(|bucket| bucket.entries.keys())
                .collect()
        }
        fn entries(&self) -> impl Iterator<Item = &Entry> {
            self.buckets
                .values()
                .flat_map(|bucket| bucket.entries.values())
        }
        fn insert_pending(&mut self, url: &str, pending: Pending) {
            let egress = self.egress(url);
            self.bucket_mut(egress).pending.insert(url.into(), pending);
        }
        fn pending(&self, url: &str) -> Option<&Pending> {
            self.buckets.get(&self.egress(url))?.pending.get(url)
        }
    }

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
        assert!(cache.entry_keys().len() <= RETAINED_ENTRIES);
        let kept = cache.entry_keys()[0].clone();
        for _ in 0..=IDLE_FRAMES {
            cache.begin_frame();
            cache.get(&kept);
            cache.trim();
        }
        assert_eq!(
            cache.entry_keys().len(),
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
            let used = cache.frame;
            cache.insert_pending(&format!("old:{id}"), Pending::for_test(receiver, used));
            sender
                .send(Some((2048, 2048, vec![255; 2048 * 2048 * 4])))
                .unwrap();
        }
        cache.begin_frame();
        assert!(cache.poll());
        assert!(cache.entries().all(|entry| entry.texture.is_none()));
        cache.trim();
        let bytes: usize = cache
            .entries()
            .filter_map(|entry| entry.decoded.as_ref())
            .map(|(_, _, bytes)| bytes.len())
            .sum();
        assert!(bytes <= RETAINED_BYTES as usize);
    }

    #[test]
    fn dropping_the_cache_cancels_in_flight_fetches() {
        let (_sender, receiver) = mpsc::channel();
        let mut cache = UrlTextureCache::default();
        cache.begin_frame();
        let pending = Pending::for_test(receiver, cache.frame);
        // The clone outlives the cache, which is what makes the assertion
        // observable after the drop.
        let token = pending.cancellation.clone();
        cache.insert_pending("http://example.invalid/a.png", pending);

        assert!(
            !token.is_cancelled(),
            "not cancelled while the cache is alive"
        );
        drop(cache);
        assert!(
            token.is_cancelled(),
            "tearing the pipeline down must stop the in-flight request"
        );
    }

    #[test]
    fn unreferenced_pending_fetches_are_cancelled_and_stay_retryable() {
        let url = "http://example.invalid/b.png";
        let (_sender, receiver) = mpsc::channel();
        let mut cache = UrlTextureCache::default();
        cache.begin_frame();
        let pending = Pending::for_test(receiver, cache.frame);
        let token = pending.cancellation.clone();
        cache.insert_pending(url, pending);

        for _ in 0..=IDLE_FRAMES {
            cache.begin_frame();
            cache.trim();
        }

        assert!(token.is_cancelled(), "an abandoned fetch must be cancelled");
        assert!(!cache.has_pending(), "and must stop occupying a slot");
        assert!(
            !cache.contains_retained(url),
            "cancelling is not failing: the URL must stay re-requestable, not be              cached as a permanent miss"
        );
    }

    #[test]
    fn a_pending_fetch_still_wanted_each_frame_is_not_cancelled() {
        let url = "http://example.invalid/c.png";
        let (_sender, receiver) = mpsc::channel();
        let mut cache = UrlTextureCache::default();
        cache.begin_frame();
        let pending = Pending::for_test(receiver, cache.frame);
        let token = pending.cancellation.clone();
        cache.insert_pending(url, pending);

        for _ in 0..IDLE_FRAMES * 3 {
            cache.begin_frame();
            // What `load` does for a URL that is still painted this frame.
            cache.pending(url).unwrap().used.set(cache.frame);
            cache.trim();
        }

        assert!(
            !token.is_cancelled(),
            "a live request must not be cancelled"
        );
        assert_eq!(cache.pending_len(), 1);
    }

    #[test]
    fn an_idle_remote_bucket_releases_its_fetch_host() {
        let host = nana_ui_platform::shared_fetch_host(nana_ui_platform::NativeFetchHost::new(
            nana_ui_platform::FetchPolicy::default(),
        ));
        let released = Arc::downgrade(&host);
        let mut cache = UrlTextureCache::default();
        cache.begin_frame();
        cache.set_fetch_host(Some(host));
        cache.insert("http://example.invalid/e.png".into(), None);
        cache.set_fetch_host(None);
        assert!(
            released.upgrade().is_some(),
            "cached results pin their host"
        );

        for _ in 0..=IDLE_FRAMES {
            cache.begin_frame();
            cache.trim();
        }

        assert!(
            released.upgrade().is_none(),
            "a closed document's host must not outlive its expired images"
        );
    }
}
