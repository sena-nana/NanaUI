//! Consumer half of [`nana_frame_exchange`]: shows the latest exchanged frame
//! in a stable [`TextureSlot`] while respecting the host's present ordering.

use std::sync::Arc;

use nana_diagnostics::{event, framework::gpu, metric};
use nana_frame_exchange::{
    FrameAlphaMode, FrameColorMetadata, FrameColorPrimaries, FrameColorRange, FrameInbox,
    FrameLease, FrameLinearUnit, FrameToken, FrameTransferFunction,
};
use nana_gpu::{
    DeviceGeneration, GpuContext, GpuTexture, GpuTextureDescriptor, GpuTextureFormat,
    GpuTextureUsages,
};

use crate::gpu_texture::{HostTexture, HostTextureAlphaMode, TextureSlot};
use nana_ui_runtime::{MediaPacing, MediaSampleRequest};

/// The resource time a published frame shows, in seconds, read from the
/// frame's payload (Issue #303). The producer records it when it publishes
/// the frame for a [`MediaSampleRequest`]; the binding compares it with the
/// time the document asks for.
pub trait MediaTimestamp {
    /// `None` when the frame carries no resource time: it never aligns.
    fn media_time(&self) -> Option<f64>;
}

impl MediaTimestamp for f64 {
    fn media_time(&self) -> Option<f64> {
        Some(*self)
    }
}

/// A frame payload carrying the frame's resource time beside the
/// application's own data. Video frames, image sequences and sprite frames
/// all use it, through the same mapping.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MediaFrame<T = ()> {
    /// The resource time this frame shows, in seconds.
    pub media_time: f64,
    pub data: T,
}

impl<T> MediaFrame<T> {
    pub fn new(media_time: f64, data: T) -> Self {
        Self { media_time, data }
    }
}

impl<T> MediaTimestamp for MediaFrame<T> {
    fn media_time(&self) -> Option<f64> {
        Some(self.media_time)
    }
}

/// What [`FrameBinding::prepare_media`] showed for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSampleOutcome {
    /// The bound frame stands for the request. `changed` when it was bound
    /// just now, so the window redraws.
    Aligned { changed: bool },
    /// Real-time playback found no frame for the request: the last aligned
    /// frame repeats (or, before the first one, the poster or placeholder).
    /// A late frame is never shown as this time.
    Repeated,
    /// Stepped presentation found no frame for the request. Nothing was put
    /// in its place: wait for the producer and prepare again, or fail the
    /// export explicitly; do not present this frame as that time.
    Unaligned,
}

impl MediaSampleOutcome {
    /// Whether the slot binding changed.
    pub fn changed(self) -> bool {
        matches!(self, Self::Aligned { changed: true })
    }

    /// Whether this frame shows the time it was asked for.
    pub fn aligned(self) -> bool {
        matches!(self, Self::Aligned { .. })
    }
}

/// Reads a payload's resource time; kept beside the last request so a present
/// can judge a waiting frame without a `MediaTimestamp` bound.
type MediaStamp<M> = fn(&M) -> Option<f64>;

/// What a request shows, given the resource time of the frame bound after
/// preparing (`None` while the poster or placeholder shows).
fn media_outcome(
    request: &MediaSampleRequest,
    bound: Option<f64>,
    changed: bool,
) -> MediaSampleOutcome {
    if bound.is_some_and(|time| request.aligned(time)) {
        return MediaSampleOutcome::Aligned { changed };
    }
    match request.pacing {
        MediaPacing::Realtime => MediaSampleOutcome::Repeated,
        MediaPacing::Stepped => MediaSampleOutcome::Unaligned,
    }
}

/// Binds exchanged frames to one [`TextureSlot`].
///
/// Call [`Self::prepare`] from `prepare_window_frame` and [`Self::presented`]
/// from `window_frame_presented` of the window that samples the slot. The slot
/// always holds a binding: a 1×1 transparent placeholder is shown while no
/// usable frame exists, so scene validation never loses the slot. A live
/// exchange that has moved epoch but not yet published the next frame keeps
/// the picture already showing; the placeholder would present an empty window.
pub struct FrameBinding<E = u64, M = ()> {
    device_generation: DeviceGeneration,
    slot: TextureSlot,
    alpha: HostTextureAlphaMode,
    texture: HostTexture,
    placeholder: GpuTexture,
    showing_placeholder: bool,
    /// The binding changed since the window last presented.
    awaiting_present: bool,
    current: Option<Arc<FrameLease<E, M>>>,
    /// The frame replaced since the last present. The extracted UI frame may
    /// still sample it, so its slot is not returned before presentation.
    retired: Option<Arc<FrameLease<E, M>>>,
    /// Shown in place of the placeholder until the slot has had a frame.
    poster: Option<GpuTexture>,
    /// A frame has been bound at least once: the poster is over for good.
    had_frame: bool,
    /// The resource time the last media request asked for, so a present can
    /// tell whether the frame waiting is one worth redrawing for.
    media_request: Option<(MediaSampleRequest, MediaStamp<M>)>,
}

impl<E: Copy + Eq + Send + Sync + 'static> FrameBinding<E> {
    /// Construct a binding for an exchange without application payloads.
    pub fn new(gpu: &GpuContext, slot: TextureSlot, alpha: HostTextureAlphaMode) -> Self {
        Self::new_with_payload(gpu, slot, alpha)
    }
}

impl<E: Copy + Eq + Send + Sync + 'static, M: Send + Sync + 'static> FrameBinding<E, M> {
    /// `gpu` is the device the window paints with. An inbox from another
    /// device is never bound.
    pub fn new_with_payload(
        gpu: &GpuContext,
        slot: TextureSlot,
        alpha: HostTextureAlphaMode,
    ) -> Self {
        // WGPU zero-initializes it: transparent black.
        let placeholder = gpu
            .create_texture(&GpuTextureDescriptor {
                label: Some("NanaUI frame binding placeholder"),
                width: 1,
                height: 1,
                format: GpuTextureFormat::RGBA8_UNORM,
                usage: GpuTextureUsages::SAMPLED,
            })
            .expect("a 1x1 sampled RGBA8 texture is valid on every device");
        let texture = HostTexture::new(&placeholder);
        slot.replace(texture.clone(), 1, 1, alpha);
        Self {
            device_generation: gpu.generation(),
            slot,
            alpha,
            texture,
            placeholder,
            showing_placeholder: true,
            awaiting_present: false,
            current: None,
            retired: None,
            poster: None,
            had_frame: false,
            media_request: None,
        }
    }

    /// Show `poster` in the slot while it has never had a frame (Issue #303).
    /// Once a frame was bound the poster is not shown again: a mapping that
    /// holds on its end keeps that end's frame. `None` goes back to the
    /// transparent placeholder.
    pub fn set_poster(&mut self, poster: Option<GpuTexture>) {
        self.poster = poster;
        if self.showing_placeholder && !self.had_frame {
            // Re-enter the placeholder state to show the poster or drop it.
            self.showing_placeholder = false;
            self.bind_placeholder();
            self.awaiting_present = true;
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

    /// Source color metadata of the bound frame. `None` while the placeholder
    /// is shown. Frame exchange performs no conversion, so a consumer that
    /// exports or composites this texture must inspect this declaration and
    /// explicitly convert it to the destination contract when needed.
    pub fn color_metadata(&self) -> Option<FrameColorMetadata> {
        self.current.as_ref().map(|frame| frame.color_metadata())
    }

    /// Application data belonging to the currently bound texture.
    pub fn payload(&self) -> Option<&M> {
        self.current.as_ref().map(|frame| frame.payload())
    }

    /// Bind the latest frame `accept` admits. Returns whether the slot binding
    /// changed.
    ///
    /// While a replaced frame awaits presentation nothing changes, which also
    /// bounds a window that ticks without presenting to one swap. Otherwise the
    /// wake is acknowledged whenever no frame is waiting or the waiting one is
    /// accepted, so the next publication wakes the window again; that covers a
    /// new epoch, whose wake arrives before its replacement frame exists. A
    /// frame `accept` rejects is not acknowledged, so a rejecting window is not
    /// woken for every new frame; redraw it when its policy changes.
    pub fn prepare(
        &mut self,
        inbox: Option<&FrameInbox<E, M>>,
        accept: impl Fn(&FrameToken<E>) -> bool,
    ) -> bool {
        self.media_request = None;
        self.prepare_admitting(inbox, accept, |_| true)
    }

    /// [`Self::prepare`] for frames that bind only when `admit` lets them;
    /// the frame already bound stays usable on `accept` alone.
    fn prepare_admitting(
        &mut self,
        inbox: Option<&FrameInbox<E, M>>,
        accept: impl Fn(&FrameToken<E>) -> bool,
        admit: impl Fn(&FrameLease<E, M>) -> bool,
    ) -> bool {
        if self.awaiting_present {
            return false;
        }
        let inbox = inbox.filter(|inbox| inbox.device_generation() == self.device_generation);
        if let (Some(inbox), Some(frame)) = (inbox, self.current.as_deref())
            && inbox.awaits_replacement(&frame.token())
        {
            metric!(gpu::FRAME_BINDING_REPLACEMENT_GAPS);
        }
        // A new epoch retires the bound frame and publishes its replacement in
        // the same breath. The replacement is looked for first. Until it
        // exists the picture already on screen stays: the 1x1 placeholder is
        // transparent, and presenting it clears a composition window.
        let stale = self
            .current
            .as_deref()
            .is_some_and(|frame| !usable(inbox, &accept, frame));
        if self
            .current
            .as_deref()
            .is_some_and(|frame| !accept(&frame.token()))
        {
            metric!(gpu::FRAME_BINDING_REJECTIONS);
        }
        // Acknowledging loads again, so a frame published since `latest` is
        // bound here instead of raising a wake of its own.
        let bindable =
            |frame: &FrameLease<E, M>| can_bind(frame, &accept, self.alpha) && admit(frame);
        if let Some(inbox) = inbox
            && inbox.latest().is_none_or(|frame| bindable(&frame))
            && let Some(frame) = inbox.try_take_latest().filter(|frame| bindable(frame))
            && self.token() != Some(frame.token())
        {
            let (width, height) = frame.size();
            let token = frame.token();
            let damage = if !self.showing_placeholder {
                self.current.as_deref().and_then(|prev| {
                    let prev_token = prev.token();
                    (prev_token.exchange_id() == token.exchange_id()
                        && prev_token.epoch() == token.epoch()
                        && prev.size() == (width, height))
                        .then(|| {
                            inbox
                                .damage_since(prev_token.sequence(), token.sequence())
                                .map(|rect| crate::PhysicalRect {
                                    x: rect.x,
                                    y: rect.y,
                                    width: rect.width,
                                    height: rect.height,
                                })
                        })
                        .flatten()
                })
            } else {
                None
            };
            self.texture.replace_texture(frame.texture());
            self.slot
                .replace_with_damage(self.texture.clone(), width, height, self.alpha, damage);
            let from_placeholder = std::mem::replace(&mut self.showing_placeholder, false);
            self.had_frame = true;
            self.awaiting_present = true;
            self.retired = self.current.replace(frame);
            // Every new frame is a replacement, sixty a second for a live
            // producer: a metric. Only leaving the placeholder for a live
            // frame is a transition worth an event.
            metric!(gpu::FRAME_BINDING_REPLACEMENTS);
            if from_placeholder {
                event!(
                    gpu::FRAME_BINDING_TRANSITION,
                    exchange = token.exchange_id(),
                    sequence = token.sequence(),
                    outcome = 1u64
                );
            }
            return true;
        }
        if stale {
            let token = self.current.as_deref().map(FrameLease::token);
            self.retired = self.current.take();
            self.bind_placeholder();
            self.awaiting_present = true;
            metric!(gpu::FRAME_BINDING_PLACEHOLDER_BINDS);
            if let Some(token) = token {
                event!(
                    gpu::FRAME_BINDING_TRANSITION,
                    exchange = token.exchange_id(),
                    sequence = token.sequence(),
                    outcome = 2u64
                );
            }
            return true;
        }
        false
    }

    /// Release the frame replaced before this present. Returns whether the
    /// window should redraw: the bound frame became unusable, or a newer
    /// accepted frame is waiting.
    pub fn presented(
        &mut self,
        inbox: Option<&FrameInbox<E, M>>,
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
        inbox.and_then(FrameInbox::latest).is_some_and(|latest| {
            can_bind(&latest, &accept, self.alpha)
                && self.token() != Some(latest.token())
                && self.media_request.as_ref().is_none_or(|(request, stamp)| {
                    // A frame that does not stand for the time asked for is
                    // not worth a redraw; the next request asks again.
                    stamp(latest.payload()).is_some_and(|time| request.aligned(time))
                })
        })
    }

    fn bind_placeholder(&mut self) {
        if self.showing_placeholder {
            return;
        }
        let shown = match &self.poster {
            Some(poster) if !self.had_frame => poster,
            _ => &self.placeholder,
        };
        let (width, height) = shown.size();
        self.texture.replace_texture(shown);
        self.slot
            .replace(self.texture.clone(), width, height, self.alpha);
        self.showing_placeholder = true;
    }
}

impl<E, M> FrameBinding<E, M>
where
    E: Copy + Eq + Send + Sync + 'static,
    M: MediaTimestamp + Send + Sync + 'static,
{
    /// Bind the frame that stands for `request` (Issue #303): a frame whose
    /// payload time is within half the document's frame interval of
    /// [`MediaSampleRequest::media_time`]. Call it from
    /// `prepare_window_frame` with the request
    /// `RuntimeDocument::media_sample_requests` gave for this slot.
    ///
    /// A frame further away is never shown as this time. While the document
    /// plays ([`MediaPacing::Realtime`]) the last aligned frame repeats; while
    /// it is stepped or held ([`MediaPacing::Stepped`]) the outcome is
    /// [`MediaSampleOutcome::Unaligned`], and the caller waits for the
    /// producer or fails. The bound frame's color contract is checked as by
    /// [`Self::prepare`]: PQ, HLG or wide-gamut frames are never bound as
    /// sRGB.
    pub fn prepare_media(
        &mut self,
        inbox: Option<&FrameInbox<E, M>>,
        accept: impl Fn(&FrameToken<E>) -> bool,
        request: &MediaSampleRequest,
    ) -> MediaSampleOutcome {
        self.media_request = Some((request.clone(), M::media_time));
        let aligned = |frame: &FrameLease<E, M>| {
            frame
                .payload()
                .media_time()
                .is_some_and(|time| request.aligned(time))
        };
        let changed = self.prepare_admitting(inbox, accept, aligned);
        let bound = self
            .current
            .as_deref()
            .and_then(|frame| frame.payload().media_time());
        media_outcome(request, bound, changed)
    }
}

fn usable<E: Copy + Eq, M>(
    inbox: Option<&FrameInbox<E, M>>,
    accept: &impl Fn(&FrameToken<E>) -> bool,
    frame: &FrameLease<E, M>,
) -> bool {
    accept(&frame.token())
        && inbox.is_some_and(|inbox| {
            inbox.is_current(&frame.token()) || inbox.awaits_replacement(&frame.token())
        })
}

fn can_bind<E: Copy + Eq, M>(
    frame: &FrameLease<E, M>,
    accept: &impl Fn(&FrameToken<E>) -> bool,
    alpha: HostTextureAlphaMode,
) -> bool {
    accept(&frame.token()) && metadata_compatible(frame.color_metadata(), alpha)
}

/// Keep the legacy unknown declaration usable, but never sample a known
/// transfer function or color gamut through a binding whose contract cannot
/// represent it. The binding does not perform conversion: callers that need
/// PQ/HLG/P3 or straight alpha must convert before publishing a frame.
fn metadata_compatible(metadata: FrameColorMetadata, alpha: HostTextureAlphaMode) -> bool {
    if metadata == FrameColorMetadata::unknown() {
        return true;
    }
    if metadata.primaries() != FrameColorPrimaries::Srgb
        || metadata.range() != FrameColorRange::Full
        || !matches!(
            metadata.transfer(),
            FrameTransferFunction::Srgb | FrameTransferFunction::Linear
        )
    {
        return false;
    }
    if metadata.transfer() == FrameTransferFunction::Linear
        && metadata.linear_unit() != FrameLinearUnit::Scrgb80Nits
    {
        return false;
    }
    matches!(
        (alpha, metadata.alpha()),
        (HostTextureAlphaMode::Opaque, FrameAlphaMode::Opaque)
            | (
                HostTextureAlphaMode::Premultiplied,
                FrameAlphaMode::Premultiplied
            )
    )
}

impl<E, M> Drop for FrameBinding<E, M> {
    fn drop(&mut self) {
        // A returned slot texture is reused by the producer; the registration
        // that outlives this binding must not keep sampling it.
        if !self.showing_placeholder {
            self.texture.replace_texture(&self.placeholder);
            self.slot.replace(self.texture.clone(), 1, 1, self.alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_texture::HostTextureRegistry;
    use nana_frame_exchange::{CopyOutcome, DEFAULT_CAPACITY, FrameExchange};
    use nana_gpu::__framework;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn media_request(media_time: f64, pacing: MediaPacing) -> MediaSampleRequest {
        MediaSampleRequest {
            node: nana_ui_runtime::StableNodeId::new(1).unwrap(),
            slot: "media".into(),
            media_time,
            frame_interval: std::time::Duration::from_millis(40),
            pacing,
        }
    }

    #[test]
    fn an_unaligned_frame_repeats_in_real_time_and_fails_when_stepped() {
        let realtime = media_request(1.0, MediaPacing::Realtime);
        let stepped = media_request(1.0, MediaPacing::Stepped);
        assert_eq!(
            media_outcome(&realtime, Some(1.019), true),
            MediaSampleOutcome::Aligned { changed: true }
        );
        assert_eq!(
            media_outcome(&stepped, Some(0.981), false),
            MediaSampleOutcome::Aligned { changed: false }
        );
        for bound in [None, Some(0.9), Some(1.05)] {
            assert_eq!(
                media_outcome(&realtime, bound, false),
                MediaSampleOutcome::Repeated
            );
            assert_eq!(
                media_outcome(&stepped, bound, false),
                MediaSampleOutcome::Unaligned
            );
        }
    }

    #[test]
    fn media_frames_bind_only_within_half_a_frame_interval() {
        use nana_frame_exchange::FrameAlphaMode;

        let gpu = crate::test_gpu::context();
        let mut exchange = FrameExchange::<_, MediaFrame>::new_with_payload(
            &gpu,
            DEFAULT_CAPACITY,
            1u64,
            Arc::new(|| {}),
        );
        let inbox = exchange.inbox();
        let registry = HostTextureRegistry::new();
        let mut binding = FrameBinding::<_, MediaFrame>::new_with_payload(
            &gpu,
            registry.slot("media"),
            HostTextureAlphaMode::Premultiplied,
        );
        let poster = gpu
            .create_texture(&GpuTextureDescriptor {
                label: Some("frame binding test poster"),
                width: 3,
                height: 3,
                format: GpuTextureFormat::RGBA8_UNORM,
                usage: GpuTextureUsages::SAMPLED,
            })
            .expect("poster");
        let width = || registry.get("media").map(|binding| binding.width);
        binding.set_poster(Some(poster.clone()));
        assert_eq!(width(), Some(3), "no frame yet: the poster shows");
        binding.presented(Some(&inbox), |_| true);

        let publish = |exchange: &mut FrameExchange<u64, MediaFrame>,
                       width,
                       time: f64,
                       metadata: FrameColorMetadata| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                match exchange.copy_from_with_payload(
                    &source(&gpu, width),
                    1,
                    metadata,
                    MediaFrame::new(time, ()),
                ) {
                    CopyOutcome::Submitted => break,
                    CopyOutcome::PoolFull => {
                        assert!(std::time::Instant::now() < deadline, "no slot came back");
                        exchange.poll();
                        std::thread::yield_now();
                    }
                    other => panic!("copy {other:?}"),
                }
            }
            while !exchange.poll() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
        };
        let unknown = FrameColorMetadata::unknown();

        // A frame for 1.0 is not 1.1's frame, in real time or stepped.
        publish(&mut exchange, 4, 1.0, unknown);
        let late = media_request(1.1, MediaPacing::Realtime);
        assert_eq!(
            binding.prepare_media(Some(&inbox), |_| true, &late),
            MediaSampleOutcome::Repeated
        );
        assert!(
            !binding.presented(Some(&inbox), |_| true),
            "an unaligned frame waiting is no reason to redraw"
        );
        assert_eq!(
            binding.prepare_media(
                Some(&inbox),
                |_| true,
                &media_request(1.1, MediaPacing::Stepped)
            ),
            MediaSampleOutcome::Unaligned
        );
        assert_eq!(width(), Some(3), "still the poster, not another time");

        // Within half the 40 ms interval it binds.
        assert_eq!(
            binding.prepare_media(
                Some(&inbox),
                |_| true,
                &media_request(1.015, MediaPacing::Stepped)
            ),
            MediaSampleOutcome::Aligned { changed: true }
        );
        assert_eq!(width(), Some(4));
        binding.presented(Some(&inbox), |_| true);

        // A frame arriving late keeps the last aligned one up in real time.
        publish(&mut exchange, 8, 1.04, unknown);
        assert_eq!(
            binding.prepare_media(
                Some(&inbox),
                |_| true,
                &media_request(1.2, MediaPacing::Realtime)
            ),
            MediaSampleOutcome::Repeated
        );
        assert_eq!(width(), Some(4));
        assert_eq!(binding.payload().map(|frame| frame.media_time), Some(1.0));
        binding.presented(Some(&inbox), |_| true);
        assert_eq!(
            binding.prepare_media(
                Some(&inbox),
                |_| true,
                &media_request(1.04, MediaPacing::Realtime)
            ),
            MediaSampleOutcome::Aligned { changed: true }
        );
        assert_eq!(width(), Some(8));
        binding.presented(Some(&inbox), |_| true);

        // Holding on the end frame: the poster never comes back.
        binding.set_poster(Some(poster));
        assert_eq!(
            binding.prepare_media(
                Some(&inbox),
                |_| true,
                &media_request(1.04, MediaPacing::Stepped)
            ),
            MediaSampleOutcome::Aligned { changed: false }
        );
        assert_eq!(width(), Some(8));

        // PQ and HLG frames are never bound as sRGB, aligned or not.
        for metadata in [
            FrameColorMetadata::bt2020_pq(FrameAlphaMode::Premultiplied),
            FrameColorMetadata::bt2020_hlg(FrameAlphaMode::Premultiplied),
        ] {
            publish(&mut exchange, 12, 1.08, metadata);
            assert_eq!(
                binding.prepare_media(
                    Some(&inbox),
                    |_| true,
                    &media_request(1.08, MediaPacing::Stepped)
                ),
                MediaSampleOutcome::Unaligned
            );
            assert_eq!(width(), Some(8));
            binding.presented(Some(&inbox), |_| true);
        }
    }

    #[test]
    fn payload_is_the_bound_frames_data_until_the_next_present() {
        let gpu = crate::test_gpu::context();
        let mut exchange = FrameExchange::<_, String>::new_with_payload(
            &gpu,
            DEFAULT_CAPACITY,
            1u64,
            Arc::new(|| {}),
        );
        let inbox = exchange.inbox();
        let registry = HostTextureRegistry::new();
        let mut binding = FrameBinding::<_, String>::new_with_payload(
            &gpu,
            registry.slot("payload"),
            HostTextureAlphaMode::Premultiplied,
        );
        assert!(binding.payload().is_none());
        let publish = |exchange: &mut FrameExchange<u64, String>, width, payload: &str| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            // A slot the binding gave back is free once the GPU reports the
            // work that sampled it done, and the shared test device admits
            // one frame at a time. Other tests share it, so either can make a
            // copy report PoolFull: wait, as a producer would retry, rather
            // than treat it as a fault.
            loop {
                match exchange.copy_from_with_payload(
                    &source(&gpu, width),
                    1,
                    FrameColorMetadata::unknown(),
                    payload.into(),
                ) {
                    CopyOutcome::Submitted => break,
                    CopyOutcome::PoolFull => {
                        assert!(std::time::Instant::now() < deadline, "no slot came back");
                        exchange.poll();
                        std::thread::yield_now();
                    }
                    other => panic!("copy {other:?}"),
                }
            }
            while !exchange.poll() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
        };
        publish(&mut exchange, 4, "old");
        assert!(binding.prepare(Some(&inbox), |_| true));
        assert_eq!(binding.payload().map(String::as_str), Some("old"));
        publish(&mut exchange, 4, "same size, new crop");
        assert!(!binding.prepare(Some(&inbox), |_| true));
        assert_eq!(binding.payload().map(String::as_str), Some("old"));
        assert!(binding.presented(Some(&inbox), |_| true));
        assert!(binding.prepare(Some(&inbox), |_| true));
        assert_eq!(
            binding.payload().map(String::as_str),
            Some("same size, new crop")
        );
        assert_eq!(binding.retired.as_ref().unwrap().payload(), "old");
        binding.presented(Some(&inbox), |_| true);
        for width in [8, 12, 16] {
            publish(&mut exchange, width, "resized");
            assert!(binding.prepare(Some(&inbox), |_| true));
            assert_eq!(binding.size(), Some((width, 4)));
            assert_eq!(binding.payload().map(String::as_str), Some("resized"));
            binding.presented(Some(&inbox), |_| true);
        }
        assert!(binding.prepare(Some(&inbox), |_| false));
        assert!(binding.payload().is_none());
        assert!(binding.size().is_none());
    }

    #[test]
    fn metadata_contract_accepts_only_surface_safe_known_encodings() {
        let opaque = HostTextureAlphaMode::Opaque;
        let premultiplied = HostTextureAlphaMode::Premultiplied;
        assert!(metadata_compatible(
            FrameColorMetadata::unknown(),
            premultiplied
        ));
        assert!(metadata_compatible(
            FrameColorMetadata::srgb(FrameAlphaMode::Premultiplied),
            premultiplied
        ));
        assert!(metadata_compatible(
            FrameColorMetadata::linear_scrgb(FrameAlphaMode::Opaque),
            opaque
        ));
        assert!(!metadata_compatible(
            FrameColorMetadata::linear_srgb(FrameAlphaMode::Premultiplied),
            premultiplied
        ));
        assert!(!metadata_compatible(
            FrameColorMetadata::bt2020_pq(FrameAlphaMode::Premultiplied),
            premultiplied
        ));
        assert!(!metadata_compatible(
            FrameColorMetadata::srgb(FrameAlphaMode::Straight),
            premultiplied
        ));
    }

    fn source(gpu: &GpuContext, width: u32) -> GpuTexture {
        gpu.create_texture(&GpuTextureDescriptor {
            label: Some("frame binding test source"),
            width,
            height: 4,
            format: GpuTextureFormat::RGBA8_UNORM,
            usage: GpuTextureUsages::COPY_SRC,
        })
        .expect("copy source")
    }

    /// Await this exchange's publication, rather than waiting for unrelated
    /// work on the process-wide test queue. Nonblocking polls also let other
    /// libtest threads collect callbacks and release their frame slots.
    fn publish_frame(exchange: &mut FrameExchange<u64>, source: &GpuTexture, epoch: u64) {
        publish_frame_with(exchange, source, epoch, FrameColorMetadata::unknown());
    }

    fn publish_frame_with(
        exchange: &mut FrameExchange<u64>,
        source: &GpuTexture,
        epoch: u64,
        metadata: FrameColorMetadata,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let settle = |exchange: &FrameExchange<u64>, phase: &str| {
            assert!(
                std::time::Instant::now() < deadline,
                "frame exchange did not settle during {phase}: {:?}",
                exchange.stats(),
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        loop {
            match exchange.copy_from_with_metadata(source, epoch, metadata) {
                CopyOutcome::Submitted => break,
                CopyOutcome::PoolFull => {
                    exchange.poll();
                    settle(exchange, "frame-slot acquisition");
                }
                outcome => panic!("unexpected copy outcome {outcome:?}"),
            }
        }
        while !exchange.poll() {
            settle(exchange, "own copy publication");
        }
    }

    /// A new epoch retires the bound frame and publishes its replacement at
    /// once. Unbinding first would put the 1x1 placeholder on screen for the
    /// frame in between, which the window presents as a flash of nothing.
    #[test]
    fn a_new_epoch_swaps_straight_to_its_frame_without_a_placeholder() {
        let gpu = crate::test_gpu::context();
        let mut exchange = FrameExchange::new(&gpu, DEFAULT_CAPACITY, 0u64, Arc::new(|| {}));
        let inbox = exchange.inbox();
        let publish = |exchange: &mut FrameExchange<u64>, width: u32, epoch: u64| {
            publish_frame(exchange, &source(&gpu, width), epoch);
        };
        let registry = HostTextureRegistry::new();
        let size = || {
            let binding = registry.get("epoch").expect("the slot stays registered");
            (binding.width, binding.height)
        };
        let mut binding = FrameBinding::new(
            &gpu,
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

    /// The window reacts to the epoch's wake before the replacement frame
    /// exists. It keeps the picture already showing, and the replacement still
    /// wakes it so the swap does not wait for an unrelated redraw.
    #[test]
    fn a_frame_published_after_an_empty_epoch_wake_still_wakes_the_window() {
        let gpu = crate::test_gpu::context();
        let wakes = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&wakes);
        let mut exchange = FrameExchange::new(
            &gpu,
            DEFAULT_CAPACITY,
            0u64,
            Arc::new(move || {
                observed.fetch_add(1, Ordering::AcqRel);
            }),
        );
        let inbox = exchange.inbox();
        let registry = HostTextureRegistry::new();
        let mut binding = FrameBinding::new(
            &gpu,
            registry.slot("stall"),
            HostTextureAlphaMode::Premultiplied,
        );
        let all = |_: &FrameToken<u64>| true;

        publish_frame(&mut exchange, &source(&gpu, 4), 0);
        assert!(binding.prepare(Some(&inbox), all));
        binding.presented(Some(&inbox), all);
        assert_eq!(wakes.load(Ordering::Acquire), 1);

        exchange.set_epoch(1);
        assert_eq!(wakes.load(Ordering::Acquire), 2, "dropping the frame wakes");
        assert!(
            !binding.prepare(Some(&inbox), all),
            "the last picture stays until the new epoch publishes"
        );
        let registry_size = registry.get("stall").expect("the slot stays registered");
        assert_eq!((registry_size.width, registry_size.height), (4, 4));
        assert!(
            !binding.presented(Some(&inbox), all),
            "waiting must not spin redraws"
        );

        publish_frame(&mut exchange, &source(&gpu, 8), 1);
        assert_eq!(
            wakes.load(Ordering::Acquire),
            3,
            "the replacement frame must wake the window"
        );
        assert!(binding.prepare(Some(&inbox), all));
        let replaced = registry.get("stall").expect("the slot stays registered");
        assert_eq!((replaced.width, replaced.height), (8, 4));
        binding.presented(Some(&inbox), all);
        exchange.set_epoch(2);
        assert!(binding.prepare(Some(&inbox), |_| false));
        assert_eq!(
            binding.size(),
            None,
            "explicit rejection clears even during a gap"
        );
    }

    #[test]
    fn frames_swap_only_after_present_and_unusable_frames_fall_back_to_the_placeholder() {
        let gpu = crate::test_gpu::context();
        let wakes = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&wakes);
        let mut exchange = FrameExchange::new(
            &gpu,
            DEFAULT_CAPACITY,
            0u64,
            Arc::new(move || {
                observed.fetch_add(1, Ordering::AcqRel);
            }),
        );
        let inbox = exchange.inbox();
        let source = source(&gpu, 4);
        let publish = |exchange: &mut FrameExchange<u64>| {
            publish_frame(exchange, &source, 0);
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
            &gpu,
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

        // Same WGPU device, adopted again: a new generation all the same.
        let other = __framework::adopt(
            __framework::adapter(&gpu).clone(),
            __framework::device(&gpu).clone(),
            __framework::queue(&gpu).clone(),
        );
        let mut other_device =
            FrameBinding::<u64>::new(&other, registry.slot("other"), HostTextureAlphaMode::Opaque);
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

    #[test]
    fn known_hdr_metadata_is_left_pending_without_redraw_spin() {
        let gpu = crate::test_gpu::context();
        let mut exchange = FrameExchange::new(&gpu, DEFAULT_CAPACITY, 0u64, Arc::new(|| {}));
        let inbox = exchange.inbox();
        // Published the way every test here publishes: polling until its own
        // copy's completion is seen. Waiting on the shared device and then
        // polling once is not enough, since another test's poll may have
        // taken that completion callback and not run it yet.
        publish_frame_with(
            &mut exchange,
            &source(&gpu, 4),
            0,
            FrameColorMetadata::bt2020_pq(FrameAlphaMode::Premultiplied),
        );
        let registry = HostTextureRegistry::new();
        let mut binding = FrameBinding::new(
            &gpu,
            registry.slot("hdr"),
            HostTextureAlphaMode::Premultiplied,
        );
        let all = |_: &FrameToken<u64>| true;
        assert!(!binding.prepare(Some(&inbox), all));
        assert_eq!(binding.size(), None);
        assert!(!binding.presented(Some(&inbox), all));
        assert!(!binding.prepare(Some(&inbox), all));
    }
}
