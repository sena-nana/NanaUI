//! Host-owned media elements for `<video>` / `<audio>` / `getUserMedia`.
//!
//! Pixels stay in this runtime. A host may upload [`MediaUpload`] onto its
//! existing WGPU queue as `video:{id}` HostTexture slots. Audio never
//! fabricates a video frame.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use nana_js_engine::{HostApiRegistry, HostValue, JsException};

const MOCK_FRAME_WIDTH: u32 = 32;
const MOCK_FRAME_HEIGHT: u32 = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaStreamId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
}

impl MediaKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "video" => Some(Self::Video),
            "audio" => Some(Self::Audio),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCaptureMode {
    /// Tests and hosts without a camera get a synthetic preview track.
    Mock,
    /// Permission / device failure that must not hang callers.
    Deny,
}

impl MediaCaptureMode {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "mock" => Some(Self::Mock),
            "deny" | "denied" | "notallowed" => Some(Self::Deny),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaError(String);

impl MediaError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MediaError {}

/// Premultiplied RGBA8 frame ready for `queue.writeTexture`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaUpload {
    pub id: MediaId,
    pub slot: String,
    pub width: u32,
    pub height: u32,
    pub version: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct MediaFrame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
struct MediaElement {
    kind: MediaKind,
    src: String,
    stream: Option<MediaStreamId>,
    paused: bool,
    current_time: f64,
    duration: f64,
    muted: bool,
    volume: f64,
    ready_state: u8,
    frame: Option<MediaFrame>,
    version: u64,
    source_generation: u64,
}

impl MediaElement {
    fn new(kind: MediaKind) -> Self {
        Self {
            kind,
            src: String::new(),
            stream: None,
            paused: true,
            current_time: 0.0,
            duration: 0.0,
            muted: false,
            volume: 1.0,
            ready_state: 0,
            frame: None,
            version: 0,
            source_generation: 0,
        }
    }

    fn has_visual_frame(&self) -> bool {
        self.kind == MediaKind::Video && self.frame.is_some()
    }
}

#[derive(Debug, Clone)]
struct MediaStream {
    video: bool,
    audio: bool,
    seed: u64,
}

/// CPU media store. Device/Queue stay on the hosted renderer.
#[derive(Debug)]
pub struct MediaRuntime {
    next_id: u64,
    next_stream: u64,
    elements: HashMap<MediaId, MediaElement>,
    streams: HashMap<MediaStreamId, MediaStream>,
    capture_mode: MediaCaptureMode,
}

impl Default for MediaRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaRuntime {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            next_stream: 1,
            elements: HashMap::new(),
            streams: HashMap::new(),
            capture_mode: MediaCaptureMode::Mock,
        }
    }

    pub fn set_capture_mode(&mut self, mode: MediaCaptureMode) {
        self.capture_mode = mode;
    }

    pub fn capture_mode(&self) -> MediaCaptureMode {
        self.capture_mode
    }

    pub fn create(&mut self, kind: MediaKind) -> MediaId {
        let id = MediaId(self.next_id);
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.elements.insert(id, MediaElement::new(kind));
        id
    }

    pub fn release(&mut self, id: MediaId) -> bool {
        self.elements.remove(&id).is_some()
    }

    pub fn contains(&self, id: MediaId) -> bool {
        self.elements.contains_key(&id)
    }

    pub fn kind(&self, id: MediaId) -> Option<MediaKind> {
        self.elements.get(&id).map(|element| element.kind)
    }

    pub fn has_visual_frame(&self, id: MediaId) -> bool {
        self.elements
            .get(&id)
            .is_some_and(MediaElement::has_visual_frame)
    }

    pub fn slot(id: MediaId) -> String {
        format!("video:{}", id.0)
    }

    pub fn set_src(&mut self, id: MediaId, src: &str) -> Result<(), MediaError> {
        let element = self.element_mut(id)?;
        element.src = src.to_owned();
        element.stream = None;
        element.source_generation = element.source_generation.saturating_add(1);
        element.current_time = 0.0;
        if element.kind == MediaKind::Audio {
            element.frame = None;
            element.ready_state = if src.is_empty() { 0 } else { 4 };
            element.duration = if src.is_empty() { 0.0 } else { 1.0 };
            element.version = element.version.saturating_add(1);
            return Ok(());
        }
        if src.is_empty() {
            element.frame = None;
            element.ready_state = 0;
            element.duration = 0.0;
            element.version = element.version.saturating_add(1);
            return Ok(());
        }
        if src.starts_with("nana:mock") {
            let seed = element.source_generation.max(1);
            install_mock_frame(element, seed);
            return Ok(());
        }
        // Unknown file/network sources stay without a fabricated frame until
        // JS supplies decodable bytes or a MediaStream.
        element.frame = None;
        element.ready_state = 1;
        element.duration = 0.0;
        element.version = element.version.saturating_add(1);
        Ok(())
    }

    pub fn set_src_bytes(&mut self, id: MediaId, bytes: &[u8]) -> Result<(), MediaError> {
        let kind = self.element(id)?.kind;
        if kind == MediaKind::Audio {
            let element = self.element_mut(id)?;
            element.frame = None;
            element.ready_state = 4;
            element.duration = 1.0;
            element.source_generation = element.source_generation.saturating_add(1);
            element.version = element.version.saturating_add(1);
            return Ok(());
        }
        let decoded = decode_still_frame(bytes)?;
        let element = self.element_mut(id)?;
        element.stream = None;
        element.source_generation = element.source_generation.saturating_add(1);
        element.frame = Some(decoded);
        element.ready_state = 4;
        element.duration = 1.0;
        element.version = element.version.saturating_add(1);
        Ok(())
    }

    pub fn set_src_object(
        &mut self,
        id: MediaId,
        stream: Option<MediaStreamId>,
    ) -> Result<(), MediaError> {
        if let Some(stream) = stream
            && !self.streams.contains_key(&stream)
        {
            return Err(MediaError::new("unknown MediaStream"));
        }
        let has_video = stream
            .and_then(|stream| self.streams.get(&stream))
            .is_some_and(|stream| stream.video);
        let seed = stream
            .and_then(|stream| self.streams.get(&stream))
            .map(|stream| stream.seed)
            .unwrap_or(1);
        let element = self.element_mut(id)?;
        element.stream = stream;
        element.src = String::new();
        element.source_generation = element.source_generation.saturating_add(1);
        element.current_time = 0.0;
        if element.kind == MediaKind::Audio || !has_video {
            element.frame = None;
            element.ready_state = if stream.is_some() { 4 } else { 0 };
            element.duration = if stream.is_some() { f64::INFINITY } else { 0.0 };
            element.version = element.version.saturating_add(1);
            return Ok(());
        }
        install_mock_frame(element, seed);
        element.duration = f64::INFINITY;
        Ok(())
    }

    pub fn play(&mut self, id: MediaId) -> Result<(), MediaError> {
        let element = self.element_mut(id)?;
        element.paused = false;
        if element.ready_state == 0 && element.kind == MediaKind::Video && element.frame.is_none() {
            element.paused = true;
        }
        Ok(())
    }

    pub fn pause(&mut self, id: MediaId) -> Result<(), MediaError> {
        self.element_mut(id)?.paused = true;
        Ok(())
    }

    pub fn set_current_time(&mut self, id: MediaId, time: f64) -> Result<(), MediaError> {
        let element = self.element_mut(id)?;
        let time = if time.is_finite() { time.max(0.0) } else { 0.0 };
        element.current_time = if element.duration.is_finite() {
            time.min(element.duration)
        } else {
            time
        };
        Ok(())
    }

    pub fn set_muted(&mut self, id: MediaId, muted: bool) -> Result<(), MediaError> {
        self.element_mut(id)?.muted = muted;
        Ok(())
    }

    pub fn set_volume(&mut self, id: MediaId, volume: f64) -> Result<(), MediaError> {
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.element_mut(id)?.volume = volume;
        Ok(())
    }

    pub fn get_user_media(
        &mut self,
        video: bool,
        audio: bool,
    ) -> Result<MediaStreamId, MediaError> {
        if !video && !audio {
            return Err(MediaError::new(
                "getUserMedia requires a video or audio track",
            ));
        }
        match self.capture_mode {
            MediaCaptureMode::Deny => Err(MediaError::new("NotAllowedError")),
            MediaCaptureMode::Mock => {
                let id = MediaStreamId(self.next_stream);
                self.next_stream = self.next_stream.saturating_add(1).max(1);
                self.streams.insert(
                    id,
                    MediaStream {
                        video,
                        audio,
                        seed: id.0,
                    },
                );
                Ok(id)
            }
        }
    }

    pub fn stop_stream(&mut self, id: MediaStreamId) -> bool {
        let removed = self.streams.remove(&id).is_some();
        if removed {
            for element in self.elements.values_mut() {
                if element.stream == Some(id) {
                    element.stream = None;
                    element.frame = None;
                    element.ready_state = 0;
                    element.version = element.version.saturating_add(1);
                }
            }
        }
        removed
    }

    /// Advance playing live previews so the next upload is a new generation.
    pub fn tick_playing(&mut self) {
        let streams = self.streams.clone();
        for element in self.elements.values_mut() {
            if element.paused || element.kind != MediaKind::Video {
                continue;
            }
            let Some(stream_id) = element.stream else {
                continue;
            };
            let Some(stream) = streams.get(&stream_id) else {
                continue;
            };
            if !stream.video {
                continue;
            }
            let seed = stream.seed.saturating_add(element.version);
            install_mock_frame(element, seed);
        }
    }

    pub fn take_upload(
        &mut self,
        id: MediaId,
        uploaded_version: Option<u64>,
    ) -> Result<Option<MediaUpload>, MediaError> {
        let element = self.element(id)?;
        if !element.has_visual_frame() {
            return Ok(None);
        }
        if uploaded_version == Some(element.version) {
            return Ok(None);
        }
        let frame = element.frame.as_ref().expect("visual frame");
        Ok(Some(MediaUpload {
            id,
            slot: Self::slot(id),
            width: frame.width,
            height: frame.height,
            version: element.version,
            bytes: frame.rgba.clone(),
        }))
    }

    pub fn live_visual_ids(&self) -> Vec<MediaId> {
        let mut ids: Vec<MediaId> = self
            .elements
            .iter()
            .filter(|(_, element)| element.has_visual_frame())
            .map(|(id, _)| *id)
            .collect();
        ids.sort_by_key(|id| id.0);
        ids
    }

    /// Parse a tree identity / visual slot attr (`data-nana-media`, `data-nana-video`).
    pub fn parse_id(value: &str) -> Option<MediaId> {
        value.parse::<u64>().ok().filter(|id| *id > 0).map(MediaId)
    }

    /// Drop CPU media that is no longer on the tree.
    ///
    /// `live` is every still-mounted video **and** audio element. Do not pass
    /// only visual `data-nana-video` ids — audio never writes that attr.
    pub fn retain_live(&mut self, live: impl IntoIterator<Item = MediaId>) {
        let live: HashSet<MediaId> = live.into_iter().collect();
        self.elements.retain(|id, _| live.contains(id));
    }

    pub fn active_resource_count(&self) -> usize {
        self.elements.len() + self.streams.len()
    }

    fn element(&self, id: MediaId) -> Result<&MediaElement, MediaError> {
        self.elements
            .get(&id)
            .ok_or_else(|| MediaError::new(format!("unknown media {}", id.0)))
    }

    fn element_mut(&mut self, id: MediaId) -> Result<&mut MediaElement, MediaError> {
        self.elements
            .get_mut(&id)
            .ok_or_else(|| MediaError::new(format!("unknown media {}", id.0)))
    }

    fn descriptor(&self, id: MediaId) -> HostValue {
        let Some(element) = self.elements.get(&id) else {
            return HostValue::Null;
        };
        let (width, height) = element
            .frame
            .as_ref()
            .map(|frame| (frame.width, frame.height))
            .unwrap_or((0, 0));
        HostValue::Object(
            [
                ("__nanaMedia".into(), HostValue::Bool(true)),
                ("id".into(), HostValue::BigInt(id.0)),
                (
                    "kind".into(),
                    HostValue::String(element.kind.as_str().into()),
                ),
                ("src".into(), HostValue::String(element.src.clone())),
                ("paused".into(), HostValue::Bool(element.paused)),
                (
                    "currentTime".into(),
                    HostValue::Number(finite_or_zero(element.current_time)),
                ),
                (
                    "duration".into(),
                    HostValue::Number(finite_or_zero(element.duration)),
                ),
                ("muted".into(), HostValue::Bool(element.muted)),
                ("volume".into(), HostValue::Number(element.volume)),
                (
                    "readyState".into(),
                    HostValue::Number(f64::from(element.ready_state)),
                ),
                ("width".into(), HostValue::Number(f64::from(width))),
                ("height".into(), HostValue::Number(f64::from(height))),
                (
                    "hasVideoFrame".into(),
                    HostValue::Bool(element.has_visual_frame()),
                ),
                ("version".into(), HostValue::Number(element.version as f64)),
                (
                    "slot".into(),
                    HostValue::String(if element.has_visual_frame() {
                        Self::slot(id)
                    } else {
                        String::new()
                    }),
                ),
            ]
            .into_iter()
            .collect(),
        )
    }

    fn stream_descriptor(&self, id: MediaStreamId) -> HostValue {
        let Some(stream) = self.streams.get(&id) else {
            return HostValue::Null;
        };
        HostValue::Object(
            [
                ("__nanaMediaStream".into(), HostValue::Bool(true)),
                ("id".into(), HostValue::BigInt(id.0)),
                ("video".into(), HostValue::Bool(stream.video)),
                ("audio".into(), HostValue::Bool(stream.audio)),
                ("active".into(), HostValue::Bool(true)),
            ]
            .into_iter()
            .collect(),
        )
    }
}

/// One tree node that may own a media element.
#[derive(Debug, Clone, Copy)]
pub struct MediaTreeRef<'a> {
    pub tag: &'a str,
    pub media_id: Option<&'a str>,
    pub video_id: Option<&'a str>,
}

/// CPU retain ids (video + audio) versus visual HostTexture slots (`video:{id}`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaLiveSets {
    pub retain: Vec<MediaId>,
    pub visual: Vec<MediaId>,
}

impl MediaLiveSets {
    pub fn visual_slots(&self) -> HashSet<String> {
        self.visual
            .iter()
            .copied()
            .map(MediaRuntime::slot)
            .collect()
    }
}

/// Split tree refs into CPU retain (video **and** audio) vs visual GPU slots.
///
/// Visual slots come only from `data-nana-video`. CPU retain also keeps
/// `data-nana-media` and `<video>` / `<audio>` tags that carry either attr.
pub fn media_live_sets_from_tree<'a>(
    nodes: impl IntoIterator<Item = MediaTreeRef<'a>>,
) -> MediaLiveSets {
    let mut retain = HashSet::new();
    let mut visual = HashSet::new();
    for node in nodes {
        if let Some(id) = node.video_id.and_then(MediaRuntime::parse_id) {
            visual.insert(id);
            retain.insert(id);
        }
        if let Some(id) = node.media_id.and_then(MediaRuntime::parse_id) {
            retain.insert(id);
        }
        let _ = node.tag;
    }
    let mut retain: Vec<MediaId> = retain.into_iter().collect();
    retain.sort_by_key(|id| id.0);
    let mut visual: Vec<MediaId> = visual.into_iter().collect();
    visual.sort_by_key(|id| id.0);
    MediaLiveSets { retain, visual }
}

/// GPU slots held that are no longer in the live visual set.
pub fn released_media_gpu_slots<'a>(
    held: impl IntoIterator<Item = &'a str>,
    live: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let live: HashSet<&str> = live.into_iter().collect();
    let mut released: Vec<String> = held
        .into_iter()
        .filter(|slot| slot.starts_with("video:") && !live.contains(*slot))
        .map(str::to_string)
        .collect();
    released.sort();
    released
}

pub type SharedMediaRuntime = Arc<Mutex<MediaRuntime>>;

pub fn shared_media_runtime() -> SharedMediaRuntime {
    Arc::new(Mutex::new(MediaRuntime::new()))
}

pub(crate) fn register_media_host_ops(api: &mut HostApiRegistry, runtime: SharedMediaRuntime) {
    macro_rules! locked {
        ($runtime:expr) => {
            $runtime
                .lock()
                .map_err(|_| JsException::new("media runtime poisoned"))?
        };
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaCreate", move |args| {
            let kind = args
                .first()
                .and_then(HostValue::as_str)
                .and_then(MediaKind::parse)
                .ok_or_else(|| JsException::new("mediaCreate requires video or audio"))?;
            let mut runtime = locked!(runtime);
            let id = runtime.create(kind);
            Ok(runtime.descriptor(id))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaRelease", move |args| {
            Ok(HostValue::Bool(
                locked!(runtime).release(media_id(args, 0)?),
            ))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaSetSrc", move |args| {
            let id = media_id(args, 0)?;
            let src = args.get(1).and_then(HostValue::as_str).unwrap_or_default();
            locked!(runtime).set_src(id, src).map_err(js_error)?;
            Ok(locked!(runtime).descriptor(id))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaSetSrcBytes", move |args| {
            let id = media_id(args, 0)?;
            let bytes = args
                .get(1)
                .and_then(HostValue::as_bytes)
                .ok_or_else(|| JsException::new("mediaSetSrcBytes requires bytes"))?;
            locked!(runtime)
                .set_src_bytes(id, bytes)
                .map_err(js_error)?;
            Ok(locked!(runtime).descriptor(id))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaSetSrcObject", move |args| {
            let id = media_id(args, 0)?;
            let stream = args
                .get(1)
                .and_then(HostValue::as_u64)
                .filter(|value| *value > 0)
                .map(MediaStreamId);
            locked!(runtime)
                .set_src_object(id, stream)
                .map_err(js_error)?;
            Ok(locked!(runtime).descriptor(id))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaPlay", move |args| {
            let id = media_id(args, 0)?;
            locked!(runtime).play(id).map_err(js_error)?;
            Ok(locked!(runtime).descriptor(id))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaPause", move |args| {
            let id = media_id(args, 0)?;
            locked!(runtime).pause(id).map_err(js_error)?;
            Ok(locked!(runtime).descriptor(id))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaSetCurrentTime", move |args| {
            let id = media_id(args, 0)?;
            let time = args.get(1).and_then(HostValue::as_f64).unwrap_or(0.0);
            locked!(runtime)
                .set_current_time(id, time)
                .map_err(js_error)?;
            Ok(locked!(runtime).descriptor(id))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaSetMuted", move |args| {
            let id = media_id(args, 0)?;
            let muted = args.get(1).and_then(HostValue::as_bool).unwrap_or(false);
            locked!(runtime).set_muted(id, muted).map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaSetVolume", move |args| {
            let id = media_id(args, 0)?;
            let volume = args.get(1).and_then(HostValue::as_f64).unwrap_or(1.0);
            locked!(runtime).set_volume(id, volume).map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaInfo", move |args| {
            let id = media_id(args, 0)?;
            Ok(locked!(runtime).descriptor(id))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaSetCaptureMode", move |args| {
            let mode = args
                .first()
                .and_then(HostValue::as_str)
                .and_then(MediaCaptureMode::parse)
                .ok_or_else(|| JsException::new("capture mode must be mock or deny"))?;
            locked!(runtime).set_capture_mode(mode);
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaDevicesGetUserMedia", move |args| {
            let (video, audio) = user_media_tracks(args.first());
            let mut runtime = locked!(runtime);
            match runtime.get_user_media(video, audio) {
                Ok(id) => Ok(runtime.stream_descriptor(id)),
                Err(error) if error.0 == "NotAllowedError" => {
                    Err(JsException::new("getUserMedia permission denied")
                        .with_name("NotAllowedError"))
                }
                Err(error) => Err(js_error(error)),
            }
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("mediaStreamStop", move |args| {
            let id = args
                .first()
                .and_then(HostValue::as_u64)
                .map(MediaStreamId)
                .ok_or_else(|| JsException::new("missing MediaStream id"))?;
            Ok(HostValue::Bool(locked!(runtime).stop_stream(id)))
        });
    }
}

fn media_id(args: &[HostValue], index: usize) -> Result<MediaId, JsException> {
    args.get(index)
        .and_then(HostValue::as_u64)
        .map(MediaId)
        .ok_or_else(|| JsException::new("missing media id"))
}

fn js_error(error: MediaError) -> JsException {
    JsException::new(error.to_string())
}

fn user_media_tracks(constraints: Option<&HostValue>) -> (bool, bool) {
    match constraints {
        Some(HostValue::Object(map)) => (
            constraint_enabled(map.get("video")),
            constraint_enabled(map.get("audio")),
        ),
        _ => (true, false),
    }
}

fn constraint_enabled(value: Option<&HostValue>) -> bool {
    match value {
        None | Some(HostValue::Null) => false,
        Some(HostValue::Bool(enabled)) => *enabled,
        Some(HostValue::Object(_)) => true,
        Some(HostValue::Number(value)) => *value != 0.0,
        Some(HostValue::String(value)) => !value.is_empty() && value != "false",
        _ => true,
    }
}

fn install_mock_frame(element: &mut MediaElement, seed: u64) {
    let width = element
        .frame
        .as_ref()
        .map(|frame| frame.width)
        .filter(|width| *width > 0)
        .unwrap_or(MOCK_FRAME_WIDTH);
    let height = element
        .frame
        .as_ref()
        .map(|frame| frame.height)
        .filter(|height| *height > 0)
        .unwrap_or(MOCK_FRAME_HEIGHT);
    element.frame = Some(MediaFrame {
        width,
        height,
        rgba: mock_frame_rgba(width, height, seed),
    });
    element.ready_state = 4;
    element.version = element.version.saturating_add(1).max(1);
}

fn mock_frame_rgba(width: u32, height: u32, seed: u64) -> Vec<u8> {
    let mut bytes = vec![0u8; width as usize * height as usize * 4];
    let r = (seed.wrapping_mul(37) % 200 + 40) as u8;
    let g = (seed.wrapping_mul(17) % 180 + 30) as u8;
    let b = (seed.wrapping_mul(11) % 160 + 50) as u8;
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel[0] = r;
        pixel[1] = g;
        pixel[2] = b;
        pixel[3] = 255;
    }
    bytes
}

fn decode_still_frame(bytes: &[u8]) -> Result<MediaFrame, MediaError> {
    let decoded = image::load_from_memory(bytes)
        .map_err(|error| MediaError::new(format!("media decode failed: {error}")))?;
    let rgba = decoded.to_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    let mut bytes = rgba.into_raw();
    premultiply_rgba_in_place(&mut bytes);
    Ok(MediaFrame {
        width: width.max(1),
        height: height.max(1),
        rgba: bytes,
    })
}

fn premultiply_rgba_in_place(bytes: &mut [u8]) {
    for pixel in bytes.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        pixel[0] = ((u16::from(pixel[0]) * alpha + 127) / 255) as u8;
        pixel[1] = ((u16::from(pixel[1]) * alpha + 127) / 255) as u8;
        pixel[2] = ((u16::from(pixel[2]) * alpha + 127) / 255) as u8;
    }
}

fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_src_invalidation_advances_upload_version() {
        let mut runtime = MediaRuntime::new();
        let id = runtime.create(MediaKind::Video);
        assert!(runtime.take_upload(id, None).unwrap().is_none());
        runtime.set_src(id, "nana:mock").unwrap();
        let first = runtime.take_upload(id, None).unwrap().unwrap();
        assert_eq!(first.slot, format!("video:{}", id.0));
        assert_eq!(first.width, MOCK_FRAME_WIDTH);
        assert!(
            runtime
                .take_upload(id, Some(first.version))
                .unwrap()
                .is_none()
        );
        runtime.set_src(id, "nana:mock-b").unwrap();
        let second = runtime
            .take_upload(id, Some(first.version))
            .unwrap()
            .unwrap();
        assert!(second.version > first.version);
        assert_ne!(second.bytes, first.bytes);
    }

    #[test]
    fn audio_never_fabricates_a_video_frame() {
        let mut runtime = MediaRuntime::new();
        let id = runtime.create(MediaKind::Audio);
        runtime.set_src(id, "nana:mock").unwrap();
        runtime.play(id).unwrap();
        assert!(!runtime.has_visual_frame(id));
        assert!(runtime.take_upload(id, None).unwrap().is_none());
        assert!(MediaRuntime::slot(id).starts_with("video:"));
    }

    #[test]
    fn get_user_media_mock_feeds_video_and_deny_degrades() {
        let mut runtime = MediaRuntime::new();
        let stream = runtime.get_user_media(true, false).unwrap();
        let video = runtime.create(MediaKind::Video);
        runtime.set_src_object(video, Some(stream)).unwrap();
        let upload = runtime.take_upload(video, None).unwrap().unwrap();
        assert_eq!(upload.slot, MediaRuntime::slot(video));
        runtime.play(video).unwrap();
        runtime.tick_playing();
        let next = runtime
            .take_upload(video, Some(upload.version))
            .unwrap()
            .unwrap();
        assert!(next.version > upload.version);

        runtime.set_capture_mode(MediaCaptureMode::Deny);
        let denied = runtime.get_user_media(true, false).unwrap_err();
        assert_eq!(denied.0, "NotAllowedError");
    }

    #[test]
    fn released_media_gpu_slots_match_canvas_svg_prune() {
        let held = ["video:1", "video:2", "canvas:9"];
        let live = ["video:2"];
        assert_eq!(
            released_media_gpu_slots(held, live),
            vec!["video:1".to_string()]
        );
        assert!(released_media_gpu_slots(["video:4"], ["video:4"]).is_empty());
    }

    #[test]
    fn prune_released_media_drops_deleted_elements() {
        let mut runtime = MediaRuntime::new();
        let keep = runtime.create(MediaKind::Video);
        let drop = runtime.create(MediaKind::Video);
        runtime.set_src(keep, "nana:mock").unwrap();
        runtime.set_src(drop, "nana:mock").unwrap();
        runtime.retain_live([keep]);
        assert!(runtime.contains(keep));
        assert!(!runtime.contains(drop));
    }

    #[test]
    fn retain_live_keeps_audio_when_visual_slots_are_video_only() {
        let mut runtime = MediaRuntime::new();
        let video = runtime.create(MediaKind::Video);
        let audio = runtime.create(MediaKind::Audio);
        runtime.set_src(video, "nana:mock").unwrap();
        runtime.set_src(audio, "track.ogg").unwrap();
        runtime.play(audio).unwrap();

        let video_id = video.0.to_string();
        let audio_id = audio.0.to_string();
        let sets = media_live_sets_from_tree([
            MediaTreeRef {
                tag: "video",
                media_id: Some(video_id.as_str()),
                video_id: Some(video_id.as_str()),
            },
            MediaTreeRef {
                tag: "audio",
                media_id: Some(audio_id.as_str()),
                video_id: None,
            },
        ]);
        assert_eq!(sets.retain, vec![video, audio]);
        assert_eq!(sets.visual, vec![video]);
        assert_eq!(
            sets.visual_slots(),
            [MediaRuntime::slot(video)].into_iter().collect()
        );

        runtime.retain_live(sets.retain.iter().copied());
        assert!(
            runtime.contains(audio),
            "audio must survive visual-only GPU prune"
        );
        assert!(runtime.contains(video));
        assert!(!runtime.has_visual_frame(audio));
        runtime.play(audio).unwrap();

        runtime.retain_live([video]);
        assert!(
            !runtime.contains(audio),
            "removing the audio node must release it"
        );
        assert!(runtime.contains(video));
    }

    #[test]
    fn play_pause_and_current_time_are_host_state() {
        let mut runtime = MediaRuntime::new();
        let id = runtime.create(MediaKind::Audio);
        runtime.set_src(id, "track.ogg").unwrap();
        runtime.play(id).unwrap();
        runtime.set_current_time(id, 0.25).unwrap();
        let element = runtime.elements.get(&id).unwrap();
        assert!(!element.paused);
        assert_eq!(element.current_time, 0.25);
        runtime.pause(id).unwrap();
        assert!(runtime.elements.get(&id).unwrap().paused);
    }

    #[test]
    fn media_host_ops_create_video_and_deny_get_user_media() {
        let runtime = shared_media_runtime();
        let mut api = HostApiRegistry::new();
        register_media_host_ops(&mut api, Arc::clone(&runtime));
        let created = api
            .call("mediaCreate", &[HostValue::string("video")])
            .expect("create video");
        let id = created
            .as_object()
            .and_then(|map| map.get("id"))
            .and_then(HostValue::as_u64)
            .expect("media id");
        api.call(
            "mediaSetSrc",
            &[HostValue::BigInt(id), HostValue::string("nana:mock")],
        )
        .expect("mock src");
        let info = api
            .call("mediaInfo", &[HostValue::BigInt(id)])
            .expect("info");
        assert_eq!(
            info.as_object()
                .and_then(|map| map.get("hasVideoFrame"))
                .and_then(HostValue::as_bool),
            Some(true)
        );

        api.call("mediaSetCaptureMode", &[HostValue::string("deny")])
            .expect("deny mode");
        let error = api
            .call(
                "mediaDevicesGetUserMedia",
                &[HostValue::Object(
                    [("video".into(), HostValue::Bool(true))]
                        .into_iter()
                        .collect(),
                )],
            )
            .expect_err("permission failure must be testable");
        assert_eq!(error.name, "NotAllowedError");
    }
}
