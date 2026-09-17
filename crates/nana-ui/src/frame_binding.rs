//! Consumer half of [`nana_frame_exchange`]: shows the latest exchanged frame
//! in a stable [`TextureSlot`] while respecting the host's present ordering.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use nana_frame_exchange::{FrameInbox, FrameLease, FrameToken};

use crate::gpu_texture::{HostTexture, HostTextureAlphaMode, TextureSlot};

// Painter caches key host textures by id; keep binding ids clear of the small
// ids applications assign by hand.
static NEXT_TEXTURE_ID: AtomicU64 = AtomicU64::new(1 << 63);

/// Binds exchanged frames to one [`TextureSlot`].
///
/// Call [`Self::prepare`] from `prepare_window_frame` and [`Self::presented`]
/// from `window_frame_presented` of the window that samples the slot. The slot
/// always holds a binding: a 1×1 transparent placeholder is shown while no
/// usable frame exists, so scene validation never loses the slot.
pub struct FrameBinding<E = u64> {
    device_generation: u64,
    slot: TextureSlot,
    alpha: HostTextureAlphaMode,
    texture: HostTexture,
    placeholder: wgpu::TextureView,
    showing_placeholder: bool,
    /// The binding changed since the window last presented.
    awaiting_present: bool,
    current: Option<Arc<FrameLease<E>>>,
    /// The frame replaced since the last present. The extracted UI frame may
    /// still sample it, so its slot is not returned before presentation.
    retired: Option<Arc<FrameLease<E>>>,
}

impl<E: Copy + Eq + Send + Sync + 'static> FrameBinding<E> {
    /// `device` and `device_generation` identify the host Device the window
    /// paints with. An inbox from another device generation is never bound.
    pub fn new(
        device: &wgpu::Device,
        device_generation: u64,
        slot: TextureSlot,
        alpha: HostTextureAlphaMode,
    ) -> Self {
        let placeholder = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("NanaUI frame binding placeholder"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let texture = HostTexture::from_wgpu(
            NEXT_TEXTURE_ID.fetch_add(1, Ordering::Relaxed),
            0,
            placeholder.clone(),
        );
        slot.replace(texture.clone(), 1, 1, alpha);
        Self {
            device_generation,
            slot,
            alpha,
            texture,
            placeholder,
            showing_placeholder: true,
            awaiting_present: false,
            current: None,
            retired: None,
        }
    }

    pub fn texture(&self) -> &HostTexture {
        &self.texture
    }

    /// Token of the frame currently bound, if any.
    pub fn token(&self) -> Option<FrameToken<E>> {
        self.current.as_ref().map(|frame| frame.token())
    }

    /// Pixel size of the bound frame. `None` while the placeholder is shown.
    pub fn size(&self) -> Option<(u32, u32)> {
        self.current.as_ref().map(|frame| frame.size())
    }

    /// Bind the latest frame `accept` admits. Returns whether the slot binding
    /// changed.
    ///
    /// While a replaced frame awaits presentation nothing changes, which also
    /// bounds a window that ticks without presenting to one swap. A frame
    /// `accept` rejects is not acknowledged, so a rejecting window is not woken
    /// for every new frame; redraw it when its policy changes.
    pub fn prepare(
        &mut self,
        inbox: Option<&FrameInbox<E>>,
        accept: impl Fn(&FrameToken<E>) -> bool,
    ) -> bool {
        if self.awaiting_present {
            return false;
        }
        let inbox = inbox.filter(|inbox| inbox.device_generation() == self.device_generation);
        // A new epoch retires the bound frame and publishes its replacement in
        // the same breath. The replacement is looked for first: unbinding on
        // `stale` alone would show the 1x1 placeholder for the frame in
        // between, which the window presents as a flash of nothing.
        let stale = self
            .current
            .as_deref()
            .is_some_and(|frame| !usable(inbox, &accept, frame));
        if let Some(inbox) = inbox
            && inbox.latest().is_some_and(|frame| accept(&frame.token()))
            && let Some(frame) = inbox
                .try_take_latest()
                .filter(|frame| accept(&frame.token()))
            && self.token() != Some(frame.token())
        {
            let (width, height) = frame.size();
            self.texture.replace_view(frame.view().clone());
            self.slot
                .replace(self.texture.clone(), width, height, self.alpha);
            self.showing_placeholder = false;
            self.awaiting_present = true;
            self.retired = self.current.replace(frame);
            return true;
        }
        if stale {
            self.retired = self.current.take();
            self.bind_placeholder();
            self.awaiting_present = true;
            return true;
        }
        false
    }

    /// Release the frame replaced before this present. Returns whether the
    /// window should redraw: the bound frame became unusable, or a newer
    /// accepted frame is waiting.
    pub fn presented(
        &mut self,
        inbox: Option<&FrameInbox<E>>,
        accept: impl Fn(&FrameToken<E>) -> bool,
    ) -> bool {
        self.retired = None;
        self.awaiting_present = false;
        let inbox = inbox.filter(|inbox| inbox.device_generation() == self.device_generation);
        if self
            .current
            .as_deref()
            .is_some_and(|frame| !usable(inbox, &accept, frame))
        {
            return true;
        }
        inbox
            .and_then(FrameInbox::latest)
            .is_some_and(|latest| accept(&latest.token()) && self.token() != Some(latest.token()))
    }

    fn bind_placeholder(&mut self) {
        if self.showing_placeholder {
            return;
        }
        self.texture.replace_view(self.placeholder.clone());
        self.slot.replace(self.texture.clone(), 1, 1, self.alpha);
        self.showing_placeholder = true;
    }
}

fn usable<E: Copy + Eq>(
    inbox: Option<&FrameInbox<E>>,
    accept: &impl Fn(&FrameToken<E>) -> bool,
    frame: &FrameLease<E>,
) -> bool {
    inbox.is_some_and(|inbox| inbox.is_current(&frame.token())) && accept(&frame.token())
}

impl<E> Drop for FrameBinding<E> {
    fn drop(&mut self) {
        // A returned slot texture is reused by the producer; the registration
        // that outlives this binding must not keep sampling it.
        if !self.showing_placeholder {
            self.texture.replace_view(self.placeholder.clone());
            self.slot.replace(self.texture.clone(), 1, 1, self.alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_texture::HostTextureRegistry;
    use nana_frame_exchange::{CopyOutcome, DEFAULT_CAPACITY, FrameExchange};

    fn test_device() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
        let (device, queue) = crate::test_gpu::device();
        (Arc::new(device), Arc::new(queue))
    }

    /// Copy `source` and wait until the exchange publishes it. Other tests
    /// share the device, and wgpu runs `on_submitted_work_done` callbacks on
    /// whichever thread's poll collects them, so one poll here does not
    /// guarantee this exchange's copy and slot releases have been observed.
    fn publish_frame(
        device: &wgpu::Device,
        exchange: &mut FrameExchange<u64>,
        source: &wgpu::Texture,
        epoch: u64,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let settle = || {
            assert!(
                std::time::Instant::now() < deadline,
                "frame exchange did not settle"
            );
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        };
        loop {
            match exchange.copy_from(source, epoch) {
                CopyOutcome::Submitted => break,
                CopyOutcome::PoolFull => {
                    settle();
                    exchange.poll();
                }
                outcome => panic!("unexpected copy outcome {outcome:?}"),
            }
        }
        while !exchange.poll() {
            settle();
        }
    }

    /// A new epoch retires the bound frame and publishes its replacement at
    /// once. Unbinding first would put the 1x1 placeholder on screen for the
    /// frame in between, which the window presents as a flash of nothing.
    #[test]
    fn a_new_epoch_swaps_straight_to_its_frame_without_a_placeholder() {
        let (device, queue) = test_device();
        let mut exchange = FrameExchange::new(
            11,
            Arc::clone(&device),
            queue,
            DEFAULT_CAPACITY,
            0u64,
            Arc::new(|| {}),
        );
        let inbox = exchange.inbox();
        let source = |width: u32| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("epoch swap source"),
                size: wgpu::Extent3d {
                    width,
                    height: 4,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let publish = |exchange: &mut FrameExchange<u64>, width: u32, epoch: u64| {
            publish_frame(&device, exchange, &source(width), epoch);
        };
        let registry = HostTextureRegistry::new();
        let size = || {
            let binding = registry.get("epoch").expect("the slot stays registered");
            (binding.width, binding.height)
        };
        let mut binding = FrameBinding::new(
            &device,
            11,
            registry.slot("epoch"),
            HostTextureAlphaMode::Premultiplied,
        );
        let all = |_: &FrameToken<u64>| true;

        publish(&mut exchange, 4, 0);
        assert!(binding.prepare(Some(&inbox), all));
        assert_eq!(size(), (4, 4));
        assert!(!binding.presented(Some(&inbox), all));

        publish(&mut exchange, 8, 1);
        assert!(binding.prepare(Some(&inbox), all));
        assert_eq!(
            size(),
            (8, 4),
            "the new epoch's frame must land without a placeholder in between"
        );
    }

    #[test]
    fn frames_swap_only_after_present_and_unusable_frames_fall_back_to_the_placeholder() {
        let (device, queue) = test_device();
        let wakes = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&wakes);
        let mut exchange = FrameExchange::new(
            7,
            Arc::clone(&device),
            queue,
            DEFAULT_CAPACITY,
            0u64,
            Arc::new(move || {
                observed.fetch_add(1, Ordering::AcqRel);
            }),
        );
        let inbox = exchange.inbox();
        let source = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame binding test source"),
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let publish = |exchange: &mut FrameExchange<u64>| {
            publish_frame(&device, exchange, &source, 0);
        };
        let registry = HostTextureRegistry::new();
        let changes = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&changes);
        let _subscription = registry.subscribe(move |_| {
            counted.fetch_add(1, Ordering::AcqRel);
        });
        let size = || {
            let binding = registry.get("frame").expect("the slot stays registered");
            (binding.width, binding.height)
        };
        let mut binding = FrameBinding::new(
            &device,
            7,
            registry.slot("frame"),
            HostTextureAlphaMode::Premultiplied,
        );
        let all = |_: &FrameToken<u64>| true;
        let none = |_: &FrameToken<u64>| false;
        assert_eq!(size(), (1, 1));
        assert!(!binding.prepare(Some(&inbox), all));

        publish(&mut exchange);
        assert!(binding.prepare(Some(&inbox), all));
        assert_eq!(size(), (4, 4));
        let first = binding.token().unwrap();

        publish(&mut exchange);
        let changed = changes.load(Ordering::Acquire);
        assert!(
            !binding.prepare(Some(&inbox), all),
            "a replaced frame is held until the window presents"
        );
        assert_eq!(changes.load(Ordering::Acquire), changed);
        assert!(
            binding.presented(Some(&inbox), all),
            "a newer frame is waiting"
        );
        assert!(binding.prepare(Some(&inbox), all));
        assert!(binding.token().unwrap().sequence() > first.sequence());
        assert!(!binding.presented(Some(&inbox), all));

        assert!(binding.prepare(Some(&inbox), none));
        assert_eq!(size(), (1, 1));
        assert_eq!(binding.token(), None);
        assert!(
            binding.retired.is_some(),
            "the rejected frame waits for present"
        );
        assert!(!binding.presented(Some(&inbox), none));
        assert!(binding.retired.is_none());

        let acknowledged = wakes.load(Ordering::Acquire);
        publish(&mut exchange);
        assert!(!binding.prepare(Some(&inbox), none));
        publish(&mut exchange);
        assert_eq!(
            wakes.load(Ordering::Acquire),
            acknowledged + 1,
            "a rejecting window leaves the wake unacknowledged"
        );

        let mut other_device = FrameBinding::<u64>::new(
            &device,
            8,
            registry.slot("other"),
            HostTextureAlphaMode::Opaque,
        );
        assert!(
            !other_device.prepare(Some(&inbox), all),
            "an inbox from another device generation is never bound"
        );

        binding.presented(Some(&inbox), all);
        binding.prepare(Some(&inbox), all);
        assert_eq!(size(), (4, 4));
        drop(exchange);
        binding.presented(Some(&inbox), all);
        assert!(
            binding.prepare(Some(&inbox), all),
            "a retired exchange unbinds"
        );
        assert_eq!(size(), (1, 1));
        binding.presented(None, all);
        assert!(!binding.prepare(None, all));
        drop(binding);
        assert_eq!(size(), (1, 1));
    }
}
