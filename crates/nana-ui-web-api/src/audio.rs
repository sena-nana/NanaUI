//! Web Audio PCM subset: `AudioContext`, `AudioBuffer`, `AudioBufferSourceNode`,
//! `GainNode`, `destination`, and `ScriptProcessorNode`.
//!
//! Output is host-owned. Desktop uses cpal; tests inject [`MockAudioSink`] so
//! CI does not need speakers. Missing host or device fails with
//! `NotSupportedError`. This mixer never writes HostTexture / video frames.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use nana_js_engine::{HostApiRegistry, HostValue, JsException};

const DEFAULT_SAMPLE_RATE: u32 = 44_100;
const DEFAULT_CHANNELS: u16 = 2;
const MAX_BUFFER_FRAMES: usize = 10_000_000;
const MAX_SCRIPT_QUEUE_SAMPLES: usize = 16384 * 8 * 2;
const SILENCE_EPS: f32 = 1.0e-4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioError {
    name: &'static str,
    message: String,
}

impl AudioError {
    fn not_supported(message: impl Into<String>) -> Self {
        Self {
            name: "NotSupportedError",
            message: message.into(),
        }
    }

    fn invalid_state(message: impl Into<String>) -> Self {
        Self {
            name: "InvalidStateError",
            message: message.into(),
        }
    }

    fn index_size(message: impl Into<String>) -> Self {
        Self {
            name: "IndexSizeError",
            message: message.into(),
        }
    }

    fn no_output() -> Self {
        Self::not_supported("no audio host or output device")
    }
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AudioError {}

fn js_error(error: AudioError) -> JsException {
    JsException::new(error.message).with_name(error.name)
}

/// Captured interleaved PCM for tests. Never opens a device.
#[derive(Clone)]
pub struct MockAudioSink {
    sample_rate: u32,
    channels: u16,
    samples: Arc<Mutex<Vec<f32>>>,
}

impl MockAudioSink {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            channels: channels.max(1),
            samples: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn captured(&self) -> Vec<f32> {
        self.samples
            .lock()
            .map(|samples| samples.clone())
            .unwrap_or_default()
    }

    pub fn has_nonsilent(&self) -> bool {
        self.captured()
            .iter()
            .any(|sample| sample.abs() > SILENCE_EPS)
    }

    pub fn clear(&self) {
        if let Ok(mut samples) = self.samples.lock() {
            samples.clear();
        }
    }
}

impl fmt::Debug for MockAudioSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MockAudioSink")
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("captured", &self.captured().len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContextState {
    Running,
    Suspended,
    Closed,
}

impl ContextState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Suspended => "suspended",
            Self::Closed => "closed",
        }
    }
}

#[derive(Clone)]
enum NodeKind {
    Destination,
    Gain {
        value: f32,
    },
    BufferSource {
        buffer: Option<u64>,
        playing: bool,
        started: bool,
        cursor: f64,
        looped: bool,
    },
    ScriptProcessor {
        queued: Vec<f32>,
        output_channels: u16,
    },
}

struct NodeRec {
    context: u64,
    kind: NodeKind,
    inputs: Vec<u64>,
}

struct BufferRec {
    sample_rate: u32,
    channels: Vec<Vec<f32>>,
}

struct ContextRec {
    sample_rate: u32,
    destination: u64,
    state: ContextState,
    current_frame: u64,
}

struct Mixer {
    next_id: u64,
    output_channels: u16,
    output_sample_rate: u32,
    capture: Option<Arc<Mutex<Vec<f32>>>>,
    contexts: HashMap<u64, ContextRec>,
    buffers: HashMap<u64, BufferRec>,
    nodes: HashMap<u64, NodeRec>,
}

impl Mixer {
    fn new(sample_rate: u32, channels: u16, capture: Option<Arc<Mutex<Vec<f32>>>>) -> Self {
        Self {
            next_id: 1,
            output_channels: channels.max(1),
            output_sample_rate: sample_rate.max(1),
            capture,
            contexts: HashMap::new(),
            buffers: HashMap::new(),
            nodes: HashMap::new(),
        }
    }

    fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1).max(1);
        id
    }

    fn create_context(&mut self) -> Result<u64, AudioError> {
        let id = self.alloc();
        let destination = self.alloc();
        self.nodes.insert(
            destination,
            NodeRec {
                context: id,
                kind: NodeKind::Destination,
                inputs: Vec::new(),
            },
        );
        self.contexts.insert(
            id,
            ContextRec {
                sample_rate: self.output_sample_rate,
                destination,
                state: ContextState::Running,
                current_frame: 0,
            },
        );
        Ok(id)
    }

    fn context(&self, id: u64) -> Result<&ContextRec, AudioError> {
        self.contexts
            .get(&id)
            .ok_or_else(|| AudioError::invalid_state(format!("unknown AudioContext {id}")))
    }

    fn context_mut(&mut self, id: u64) -> Result<&mut ContextRec, AudioError> {
        self.contexts
            .get_mut(&id)
            .ok_or_else(|| AudioError::invalid_state(format!("unknown AudioContext {id}")))
    }

    fn require_running_context(&self, id: u64) -> Result<&ContextRec, AudioError> {
        let context = self.context(id)?;
        if context.state == ContextState::Closed {
            return Err(AudioError::invalid_state("AudioContext is closed"));
        }
        Ok(context)
    }

    fn node(&self, id: u64) -> Result<&NodeRec, AudioError> {
        self.nodes
            .get(&id)
            .ok_or_else(|| AudioError::invalid_state(format!("unknown AudioNode {id}")))
    }

    fn node_mut(&mut self, id: u64) -> Result<&mut NodeRec, AudioError> {
        self.nodes
            .get_mut(&id)
            .ok_or_else(|| AudioError::invalid_state(format!("unknown AudioNode {id}")))
    }

    fn create_buffer(
        &mut self,
        context: u64,
        number_of_channels: u32,
        length: u32,
        sample_rate: Option<u32>,
    ) -> Result<u64, AudioError> {
        self.require_running_context(context)?;
        if number_of_channels == 0 || number_of_channels > 32 {
            return Err(AudioError::not_supported(
                "AudioBuffer numberOfChannels is out of range",
            ));
        }
        let frames = length as usize;
        if frames == 0 || frames > MAX_BUFFER_FRAMES {
            return Err(AudioError::not_supported(
                "AudioBuffer length is out of range",
            ));
        }
        let sample_rate = sample_rate.unwrap_or(self.output_sample_rate);
        if sample_rate == 0 || sample_rate > 192_000 {
            return Err(AudioError::not_supported(
                "AudioBuffer sampleRate is out of range",
            ));
        }
        let channels = number_of_channels as usize;
        let id = self.alloc();
        self.buffers.insert(
            id,
            BufferRec {
                sample_rate,
                channels: vec![vec![0.0; frames]; channels],
            },
        );
        Ok(id)
    }

    fn copy_to_channel(
        &mut self,
        buffer: u64,
        channel: u32,
        samples: &[f32],
    ) -> Result<(), AudioError> {
        let rec = self
            .buffers
            .get_mut(&buffer)
            .ok_or_else(|| AudioError::invalid_state(format!("unknown AudioBuffer {buffer}")))?;
        let channel = channel as usize;
        let dest = rec
            .channels
            .get_mut(channel)
            .ok_or_else(|| AudioError::index_size("AudioBuffer channel is out of range"))?;
        let n = samples.len().min(dest.len());
        dest[..n].copy_from_slice(&samples[..n]);
        Ok(())
    }

    fn channel_data(&self, buffer: u64, channel: u32) -> Result<Vec<f32>, AudioError> {
        let rec = self
            .buffers
            .get(&buffer)
            .ok_or_else(|| AudioError::invalid_state(format!("unknown AudioBuffer {buffer}")))?;
        rec.channels
            .get(channel as usize)
            .cloned()
            .ok_or_else(|| AudioError::index_size("AudioBuffer channel is out of range"))
    }

    fn create_gain(&mut self, context: u64) -> Result<u64, AudioError> {
        self.require_running_context(context)?;
        let id = self.alloc();
        self.nodes.insert(
            id,
            NodeRec {
                context,
                kind: NodeKind::Gain { value: 1.0 },
                inputs: Vec::new(),
            },
        );
        Ok(id)
    }

    fn create_buffer_source(&mut self, context: u64) -> Result<u64, AudioError> {
        self.require_running_context(context)?;
        let id = self.alloc();
        self.nodes.insert(
            id,
            NodeRec {
                context,
                kind: NodeKind::BufferSource {
                    buffer: None,
                    playing: false,
                    started: false,
                    cursor: 0.0,
                    looped: false,
                },
                inputs: Vec::new(),
            },
        );
        Ok(id)
    }

    fn create_script_processor(
        &mut self,
        context: u64,
        buffer_size: u32,
        output_channels: u32,
    ) -> Result<u64, AudioError> {
        self.require_running_context(context)?;
        let buffer_size = if buffer_size == 0 { 4096 } else { buffer_size };
        if ![256, 512, 1024, 2048, 4096, 8192, 16384].contains(&buffer_size) {
            return Err(AudioError::not_supported(
                "ScriptProcessorNode bufferSize must be a valid power of two",
            ));
        }
        let output_channels = output_channels.clamp(1, 2) as u16;
        let id = self.alloc();
        self.nodes.insert(
            id,
            NodeRec {
                context,
                kind: NodeKind::ScriptProcessor {
                    queued: Vec::new(),
                    output_channels,
                },
                inputs: Vec::new(),
            },
        );
        Ok(id)
    }

    fn set_gain(&mut self, id: u64, value: f32) -> Result<(), AudioError> {
        match &mut self.node_mut(id)?.kind {
            NodeKind::Gain { value: slot } => {
                *slot = if value.is_finite() {
                    value.max(0.0)
                } else {
                    1.0
                };
                Ok(())
            }
            _ => Err(AudioError::invalid_state("node is not a GainNode")),
        }
    }

    fn set_source_buffer(&mut self, source: u64, buffer: u64) -> Result<(), AudioError> {
        if !self.buffers.contains_key(&buffer) {
            return Err(AudioError::invalid_state(format!(
                "unknown AudioBuffer {buffer}"
            )));
        }
        match &mut self.node_mut(source)?.kind {
            NodeKind::BufferSource {
                buffer: slot,
                cursor,
                playing,
                started,
                ..
            } => {
                if *started {
                    return Err(AudioError::invalid_state(
                        "AudioBufferSourceNode.buffer cannot change after start",
                    ));
                }
                *slot = Some(buffer);
                *cursor = 0.0;
                *playing = false;
                Ok(())
            }
            _ => Err(AudioError::invalid_state(
                "node is not an AudioBufferSourceNode",
            )),
        }
    }

    fn start_source(&mut self, source: u64) -> Result<(), AudioError> {
        match &mut self.node_mut(source)?.kind {
            NodeKind::BufferSource {
                buffer,
                playing,
                started,
                cursor,
                ..
            } => {
                if *started {
                    return Err(AudioError::invalid_state(
                        "AudioBufferSourceNode.start can be called only once",
                    ));
                }
                if buffer.is_none() {
                    return Err(AudioError::invalid_state(
                        "AudioBufferSourceNode.start requires a buffer",
                    ));
                }
                *started = true;
                *playing = true;
                *cursor = 0.0;
                Ok(())
            }
            _ => Err(AudioError::invalid_state(
                "node is not an AudioBufferSourceNode",
            )),
        }
    }

    fn stop_source(&mut self, source: u64) -> Result<(), AudioError> {
        match &mut self.node_mut(source)?.kind {
            NodeKind::BufferSource { playing, .. } => {
                *playing = false;
                Ok(())
            }
            _ => Err(AudioError::invalid_state(
                "node is not an AudioBufferSourceNode",
            )),
        }
    }

    fn submit_script_processor(&mut self, id: u64, samples: &[f32]) -> Result<(), AudioError> {
        match &mut self.node_mut(id)?.kind {
            NodeKind::ScriptProcessor { queued, .. } => {
                queued.extend_from_slice(samples);
                if queued.len() > MAX_SCRIPT_QUEUE_SAMPLES {
                    let drop = queued.len() - MAX_SCRIPT_QUEUE_SAMPLES;
                    queued.drain(..drop);
                }
                Ok(())
            }
            _ => Err(AudioError::invalid_state(
                "node is not a ScriptProcessorNode",
            )),
        }
    }

    fn connect(&mut self, from: u64, to: u64) -> Result<(), AudioError> {
        if from == to {
            return Err(AudioError::not_supported(
                "AudioNode cannot connect to itself",
            ));
        }
        let from_ctx = self.node(from)?.context;
        let to_kind_allows = matches!(
            self.node(to)?.kind,
            NodeKind::Destination | NodeKind::Gain { .. } | NodeKind::ScriptProcessor { .. }
        );
        if self.node(to)?.context != from_ctx {
            return Err(AudioError::invalid_state(
                "AudioNode.connect requires nodes from the same AudioContext",
            ));
        }
        if !to_kind_allows {
            return Err(AudioError::not_supported(
                "AudioNode.connect destination cannot take inputs",
            ));
        }
        let inputs = &mut self.node_mut(to)?.inputs;
        if !inputs.contains(&from) {
            inputs.push(from);
        }
        Ok(())
    }

    fn close_context(&mut self, id: u64) -> Result<(), AudioError> {
        self.context_mut(id)?.state = ContextState::Closed;
        Ok(())
    }

    fn set_context_state(&mut self, id: u64, state: ContextState) -> Result<(), AudioError> {
        let context = self.context_mut(id)?;
        if context.state == ContextState::Closed {
            return Err(AudioError::invalid_state("AudioContext is closed"));
        }
        context.state = state;
        Ok(())
    }

    fn mix_into(&mut self, output: &mut [f32]) {
        output.fill(0.0);
        let channels = self.output_channels.max(1) as usize;
        if channels == 0 || output.is_empty() {
            return;
        }
        let frames = output.len() / channels;
        if frames == 0 {
            return;
        }
        let sample_rate = self.output_sample_rate;
        let ids: Vec<u64> = self.contexts.keys().copied().collect();
        for id in ids {
            let Some(context) = self.contexts.get(&id) else {
                continue;
            };
            if context.state != ContextState::Running {
                continue;
            }
            let destination = context.destination;
            let mut visiting = HashSet::new();
            let contrib = self.mix_node(
                destination,
                frames,
                channels as u16,
                sample_rate,
                &mut visiting,
            );
            for (dest, sample) in output.iter_mut().zip(contrib.iter()) {
                *dest += *sample;
            }
            if let Some(context) = self.contexts.get_mut(&id) {
                context.current_frame = context.current_frame.saturating_add(frames as u64);
            }
        }
        for sample in output.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }
        if let Some(capture) = &self.capture
            && let Ok(mut samples) = capture.lock()
        {
            samples.extend_from_slice(output);
        }
    }

    fn mix_node(
        &mut self,
        id: u64,
        frames: usize,
        channels: u16,
        sample_rate: u32,
        visiting: &mut HashSet<u64>,
    ) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * channels as usize];
        if !visiting.insert(id) {
            return out;
        }
        enum Mix {
            Sum(Option<f32>),
            BufferSource,
            Script,
        }
        let (inputs, mix) = {
            let Some(node) = self.nodes.get(&id) else {
                visiting.remove(&id);
                return out;
            };
            let mix = match &node.kind {
                NodeKind::Destination => Mix::Sum(None),
                NodeKind::Gain { value } => Mix::Sum(Some(*value)),
                NodeKind::BufferSource { .. } => Mix::BufferSource,
                NodeKind::ScriptProcessor { .. } => Mix::Script,
            };
            (node.inputs.clone(), mix)
        };
        match mix {
            Mix::BufferSource => {
                self.render_buffer_source(id, frames, channels, sample_rate, &mut out);
            }
            Mix::Script => {
                self.render_script_processor(id, frames, channels, &mut out);
            }
            Mix::Sum(gain) => {
                for input in inputs {
                    let part = self.mix_node(input, frames, channels, sample_rate, visiting);
                    match gain {
                        None => {
                            for (dest, sample) in out.iter_mut().zip(part) {
                                *dest += sample;
                            }
                        }
                        Some(value) => {
                            for (dest, sample) in out.iter_mut().zip(part) {
                                *dest += sample * value;
                            }
                        }
                    }
                }
            }
        }
        visiting.remove(&id);
        out
    }

    fn render_buffer_source(
        &mut self,
        id: u64,
        frames: usize,
        out_channels: u16,
        out_rate: u32,
        out: &mut [f32],
    ) {
        let (buffer_id, mut cursor, mut playing, looped) = {
            let Some(node) = self.nodes.get(&id) else {
                return;
            };
            let NodeKind::BufferSource {
                buffer,
                playing,
                cursor,
                looped,
                ..
            } = node.kind
            else {
                return;
            };
            if !playing {
                return;
            }
            let Some(buffer_id) = buffer else {
                return;
            };
            (buffer_id, cursor, playing, looped)
        };
        let Some(buffer) = self.buffers.get(&buffer_id) else {
            return;
        };
        let buf_len = buffer.channels.first().map(Vec::len).unwrap_or(0);
        if buf_len == 0 {
            return;
        }
        let buf_rate = buffer.sample_rate.max(1);
        let step = f64::from(buf_rate) / f64::from(out_rate.max(1));
        let src_channels = buffer.channels.len();
        let out_ch = out_channels.max(1) as usize;
        {
            for frame in 0..frames {
                if cursor >= buf_len as f64 {
                    if looped && buf_len > 0 {
                        cursor %= buf_len as f64;
                    } else {
                        playing = false;
                        break;
                    }
                }
                let idx = cursor.floor() as usize;
                let idx = idx.min(buf_len - 1);
                let idx2 = (idx + 1).min(buf_len - 1);
                let frac = cursor.fract() as f32;
                for ch in 0..out_ch {
                    let sample = if out_ch == 1 && src_channels > 1 {
                        let mut sum = 0.0f32;
                        for channel in &buffer.channels {
                            let s0 = channel[idx];
                            let s1 = channel[idx2];
                            sum += s0 + (s1 - s0) * frac;
                        }
                        sum / src_channels as f32
                    } else {
                        let channel = &buffer.channels[ch.min(src_channels.saturating_sub(1))];
                        let s0 = channel[idx];
                        let s1 = channel[idx2];
                        s0 + (s1 - s0) * frac
                    };
                    out[frame * out_ch + ch] = sample;
                }
                cursor += step;
            }
        }
        if let Some(node) = self.nodes.get_mut(&id)
            && let NodeKind::BufferSource {
                cursor: slot,
                playing: playing_slot,
                ..
            } = &mut node.kind
        {
            *slot = cursor;
            *playing_slot = playing;
        }
    }

    fn render_script_processor(
        &mut self,
        id: u64,
        frames: usize,
        out_channels: u16,
        out: &mut [f32],
    ) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        let NodeKind::ScriptProcessor {
            queued,
            output_channels,
        } = &mut node.kind
        else {
            return;
        };
        let src_ch = (*output_channels).max(1) as usize;
        let needed = frames * src_ch;
        let mut taken = vec![0.0f32; needed];
        let n = queued.len().min(needed);
        taken[..n].copy_from_slice(&queued[..n]);
        queued.drain(..n);
        let out_ch = out_channels.max(1) as usize;
        for frame in 0..frames {
            for ch in 0..out_ch {
                let sample = if src_ch == 1 {
                    taken[frame]
                } else {
                    taken[frame * src_ch + ch.min(src_ch - 1)]
                };
                out[frame * out_ch + ch] = sample;
            }
        }
    }

    fn descriptor(&self, id: u64) -> HostValue {
        let Some(context) = self.contexts.get(&id) else {
            return HostValue::Null;
        };
        HostValue::Object(
            [
                ("__nanaAudioContext".into(), HostValue::Bool(true)),
                ("id".into(), HostValue::BigInt(id)),
                (
                    "sampleRate".into(),
                    HostValue::Number(f64::from(context.sample_rate)),
                ),
                (
                    "state".into(),
                    HostValue::String(context.state.as_str().into()),
                ),
                (
                    "currentTime".into(),
                    HostValue::Number(
                        context.current_frame as f64 / f64::from(context.sample_rate),
                    ),
                ),
                (
                    "destination".into(),
                    HostValue::Object(
                        [
                            ("__nanaAudioNode".into(), HostValue::Bool(true)),
                            ("id".into(), HostValue::BigInt(context.destination)),
                            ("kind".into(), HostValue::string("destination")),
                            ("context".into(), HostValue::BigInt(id)),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                ),
            ]
            .into_iter()
            .collect(),
        )
    }

    fn node_descriptor(&self, id: u64, kind: &str) -> HostValue {
        let Some(node) = self.nodes.get(&id) else {
            return HostValue::Null;
        };
        HostValue::Object(
            [
                ("__nanaAudioNode".into(), HostValue::Bool(true)),
                ("id".into(), HostValue::BigInt(id)),
                ("kind".into(), HostValue::string(kind)),
                ("context".into(), HostValue::BigInt(node.context)),
            ]
            .into_iter()
            .collect(),
        )
    }

    fn buffer_descriptor(&self, id: u64) -> HostValue {
        let Some(buffer) = self.buffers.get(&id) else {
            return HostValue::Null;
        };
        let length = buffer.channels.first().map(Vec::len).unwrap_or(0);
        HostValue::Object(
            [
                ("__nanaAudioBuffer".into(), HostValue::Bool(true)),
                ("id".into(), HostValue::BigInt(id)),
                (
                    "numberOfChannels".into(),
                    HostValue::Number(buffer.channels.len() as f64),
                ),
                ("length".into(), HostValue::Number(length as f64)),
                (
                    "sampleRate".into(),
                    HostValue::Number(f64::from(buffer.sample_rate)),
                ),
                (
                    "duration".into(),
                    HostValue::Number(length as f64 / f64::from(buffer.sample_rate.max(1))),
                ),
            ]
            .into_iter()
            .collect(),
        )
    }
}

enum AudioOutput {
    DefaultDevice,
    Mock,
    Unsupported,
}

/// Owns a cpal stream on a dedicated thread. `cpal::Stream` is not `Send` on
/// every platform (CoreAudio), so it cannot live inside `HostApiRegistry` closures.
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
struct DevicePlayback {
    _shutdown: std::sync::mpsc::Sender<()>,
}

/// Host-owned Web Audio mixer. Does not allocate GPU textures.
pub struct AudioRuntime {
    mixer: Arc<Mutex<Mixer>>,
    output: AudioOutput,
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    device: Option<DevicePlayback>,
}

impl fmt::Debug for AudioRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self.output {
            AudioOutput::DefaultDevice => "default-device",
            AudioOutput::Mock => "mock",
            AudioOutput::Unsupported => "unsupported",
        };
        f.debug_struct("AudioRuntime")
            .field("output", &kind)
            .finish()
    }
}

impl Default for AudioRuntime {
    fn default() -> Self {
        Self::default_device()
    }
}

impl AudioRuntime {
    pub fn default_device() -> Self {
        Self {
            mixer: Arc::new(Mutex::new(Mixer::new(
                DEFAULT_SAMPLE_RATE,
                DEFAULT_CHANNELS,
                None,
            ))),
            output: AudioOutput::DefaultDevice,
            #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
            device: None,
        }
    }

    pub fn with_mock(sink: MockAudioSink) -> Self {
        Self {
            mixer: Arc::new(Mutex::new(Mixer::new(
                sink.sample_rate,
                sink.channels,
                Some(Arc::clone(&sink.samples)),
            ))),
            output: AudioOutput::Mock,
            #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
            device: None,
        }
    }

    pub fn unsupported() -> Self {
        Self {
            mixer: Arc::new(Mutex::new(Mixer::new(
                DEFAULT_SAMPLE_RATE,
                DEFAULT_CHANNELS,
                None,
            ))),
            output: AudioOutput::Unsupported,
            #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
            device: None,
        }
    }

    fn lock_mixer(&self) -> Result<std::sync::MutexGuard<'_, Mixer>, AudioError> {
        self.mixer
            .lock()
            .map_err(|_| AudioError::invalid_state("audio mixer poisoned"))
    }

    fn with_mixer<R>(
        &self,
        f: impl FnOnce(&Mixer) -> Result<R, AudioError>,
    ) -> Result<R, AudioError> {
        f(&*self.lock_mixer()?)
    }

    fn with_mixer_mut<R>(
        &self,
        f: impl FnOnce(&mut Mixer) -> Result<R, AudioError>,
    ) -> Result<R, AudioError> {
        f(&mut *self.lock_mixer()?)
    }

    fn ensure_output(&mut self) -> Result<(), AudioError> {
        match &self.output {
            AudioOutput::Unsupported => Err(AudioError::no_output()),
            AudioOutput::Mock => Ok(()),
            AudioOutput::DefaultDevice => self.start_device_output(),
        }
    }

    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    fn start_device_output(&mut self) -> Result<(), AudioError> {
        if self.device.is_some() {
            return Ok(());
        }
        let (playback, sample_rate, channels) = spawn_cpal_playback(Arc::clone(&self.mixer))?;
        if let Ok(mut mixer) = self.mixer.lock() {
            mixer.output_sample_rate = sample_rate;
            mixer.output_channels = channels;
        }
        self.device = Some(playback);
        Ok(())
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    fn start_device_output(&mut self) -> Result<(), AudioError> {
        Err(AudioError::no_output())
    }

    pub fn create_context(&mut self) -> Result<AudioId, AudioError> {
        self.ensure_output()?;
        self.lock_mixer()?.create_context().map(AudioId)
    }

    pub fn mix_frames(&self, frames: usize) -> Result<(), AudioError> {
        if matches!(self.output, AudioOutput::Unsupported) {
            return Err(AudioError::no_output());
        }
        let mut mixer = self.lock_mixer()?;
        let channels = mixer.output_channels.max(1) as usize;
        let mut output = vec![0.0f32; frames.saturating_mul(channels)];
        mixer.mix_into(&mut output);
        Ok(())
    }

    pub fn sample_rate(&self) -> Result<u32, AudioError> {
        Ok(self.lock_mixer()?.output_sample_rate)
    }
}

pub type SharedAudioRuntime = Arc<Mutex<AudioRuntime>>;

pub fn shared_audio_runtime() -> SharedAudioRuntime {
    Arc::new(Mutex::new(AudioRuntime::default_device()))
}

pub fn shared_audio_runtime_with_mock(sink: MockAudioSink) -> SharedAudioRuntime {
    Arc::new(Mutex::new(AudioRuntime::with_mock(sink)))
}

pub fn shared_audio_runtime_unsupported() -> SharedAudioRuntime {
    Arc::new(Mutex::new(AudioRuntime::unsupported()))
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn spawn_cpal_playback(mixer: Arc<Mutex<Mixer>>) -> Result<(DevicePlayback, u32, u16), AudioError> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("nana-audio-out".into())
        .spawn(move || match start_cpal_stream(mixer) {
            Ok((stream, sample_rate, channels)) => {
                let _ = ready_tx.send(Ok((sample_rate, channels)));
                let _stream = stream;
                let _ = shutdown_rx.recv();
            }
            Err(error) => {
                let _ = ready_tx.send(Err(error));
            }
        })
        .map_err(|error| {
            AudioError::not_supported(format!("audio output thread failed: {error}"))
        })?;
    let (sample_rate, channels) = ready_rx.recv().map_err(|_| AudioError::no_output())??;
    Ok((
        DevicePlayback {
            _shutdown: shutdown_tx,
        },
        sample_rate,
        channels,
    ))
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn start_cpal_stream(mixer: Arc<Mutex<Mixer>>) -> Result<(cpal::Stream, u32, u16), AudioError> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, SizedSample};

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(AudioError::no_output)?;
    let supported = device.default_output_config().map_err(|error| {
        AudioError::not_supported(format!("audio output config failed: {error}"))
    })?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let sample_rate = config.sample_rate.0.max(1);
    let channels = config.channels.max(1);

    fn build<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        mixer: Arc<Mutex<Mixer>>,
    ) -> Result<cpal::Stream, AudioError>
    where
        T: SizedSample + FromSample<f32> + Send + 'static,
    {
        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [T], _| {
                    let mut interleaved = vec![0.0f32; data.len()];
                    if let Ok(mut mixer) = mixer.lock() {
                        mixer.mix_into(&mut interleaved);
                    }
                    for (slot, sample) in data.iter_mut().zip(interleaved.iter()) {
                        *slot = T::from_sample(*sample);
                    }
                },
                |_error| {},
                None,
            )
            .map_err(|error| AudioError::not_supported(format!("audio stream failed: {error}")))?;
        stream.play().map_err(|error| {
            AudioError::not_supported(format!("audio stream play failed: {error}"))
        })?;
        Ok(stream)
    }

    let stream = match sample_format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, mixer)?,
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, mixer)?,
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, mixer)?,
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, mixer)?,
        other => {
            return Err(AudioError::not_supported(format!(
                "unsupported audio sample format {other:?}"
            )));
        }
    };
    Ok((stream, sample_rate, channels))
}

pub(crate) fn register_audio_host_ops(api: &mut HostApiRegistry, runtime: SharedAudioRuntime) {
    macro_rules! locked {
        ($runtime:expr) => {
            $runtime
                .lock()
                .map_err(|_| JsException::new("audio runtime poisoned"))?
        };
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioContextCreate", move |_args| {
            let mut runtime = locked!(runtime);
            let id = runtime.create_context().map_err(js_error)?;
            runtime
                .with_mixer(|mixer| Ok(mixer.descriptor(id.0)))
                .map_err(js_error)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioContextClose", move |args| {
            let id = audio_id(args, 0)?;
            locked!(runtime)
                .with_mixer_mut(|mixer| mixer.close_context(id))
                .map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioContextResume", move |args| {
            let id = audio_id(args, 0)?;
            let mut runtime = locked!(runtime);
            runtime.ensure_output().map_err(js_error)?;
            runtime
                .with_mixer_mut(|mixer| mixer.set_context_state(id, ContextState::Running))
                .map_err(js_error)?;
            Ok(HostValue::string("running"))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioContextSuspend", move |args| {
            let id = audio_id(args, 0)?;
            locked!(runtime)
                .with_mixer_mut(|mixer| mixer.set_context_state(id, ContextState::Suspended))
                .map_err(js_error)?;
            Ok(HostValue::string("suspended"))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioContextCurrentTime", move |args| {
            let id = audio_id(args, 0)?;
            let time = locked!(runtime)
                .with_mixer(|mixer| {
                    let context = mixer.context(id)?;
                    Ok(context.current_frame as f64 / f64::from(context.sample_rate.max(1)))
                })
                .map_err(js_error)?;
            Ok(HostValue::Number(time))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioBufferCreate", move |args| {
            let context = audio_id(args, 0)?;
            let channels = args.get(1).and_then(HostValue::as_f64).unwrap_or(1.0) as u32;
            let length = args.get(2).and_then(HostValue::as_f64).unwrap_or(0.0) as u32;
            let sample_rate = args
                .get(3)
                .and_then(HostValue::as_f64)
                .map(|value| value as u32);
            locked!(runtime)
                .with_mixer_mut(|mixer| {
                    let id = mixer.create_buffer(context, channels, length, sample_rate)?;
                    Ok(mixer.buffer_descriptor(id))
                })
                .map_err(js_error)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioBufferCopyToChannel", move |args| {
            let id = audio_id(args, 0)?;
            let channel = args.get(1).and_then(HostValue::as_f64).unwrap_or(0.0) as u32;
            let samples = parse_pcm(args.get(2))?;
            locked!(runtime)
                .with_mixer_mut(|mixer| mixer.copy_to_channel(id, channel, &samples))
                .map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioBufferGetChannelData", move |args| {
            let id = audio_id(args, 0)?;
            let channel = args.get(1).and_then(HostValue::as_f64).unwrap_or(0.0) as u32;
            let samples = locked!(runtime)
                .with_mixer(|mixer| mixer.channel_data(id, channel))
                .map_err(js_error)?;
            Ok(HostValue::Bytes(f32_to_le_bytes(&samples)))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioBufferSourceCreate", move |args| {
            let context = audio_id(args, 0)?;
            locked!(runtime)
                .with_mixer_mut(|mixer| {
                    let id = mixer.create_buffer_source(context)?;
                    Ok(mixer.node_descriptor(id, "buffer-source"))
                })
                .map_err(js_error)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioBufferSourceSetBuffer", move |args| {
            let source = audio_id(args, 0)?;
            let buffer = audio_id(args, 1)?;
            locked!(runtime)
                .with_mixer_mut(|mixer| mixer.set_source_buffer(source, buffer))
                .map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioBufferSourceStart", move |args| {
            let source = audio_id(args, 0)?;
            locked!(runtime)
                .with_mixer_mut(|mixer| mixer.start_source(source))
                .map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioBufferSourceStop", move |args| {
            let source = audio_id(args, 0)?;
            locked!(runtime)
                .with_mixer_mut(|mixer| mixer.stop_source(source))
                .map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioGainCreate", move |args| {
            let context = audio_id(args, 0)?;
            locked!(runtime)
                .with_mixer_mut(|mixer| {
                    let id = mixer.create_gain(context)?;
                    Ok(mixer.node_descriptor(id, "gain"))
                })
                .map_err(js_error)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioGainSetValue", move |args| {
            let id = audio_id(args, 0)?;
            let value = args.get(1).and_then(HostValue::as_f64).unwrap_or(1.0) as f32;
            locked!(runtime)
                .with_mixer_mut(|mixer| mixer.set_gain(id, value))
                .map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioScriptProcessorCreate", move |args| {
            let context = audio_id(args, 0)?;
            let buffer_size = args.get(1).and_then(HostValue::as_f64).unwrap_or(0.0) as u32;
            let output_channels = args.get(3).and_then(HostValue::as_f64).unwrap_or(2.0) as u32;
            locked!(runtime)
                .with_mixer_mut(|mixer| {
                    let id =
                        mixer.create_script_processor(context, buffer_size, output_channels)?;
                    Ok(mixer.node_descriptor(id, "script-processor"))
                })
                .map_err(js_error)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioScriptProcessorSubmit", move |args| {
            let id = audio_id(args, 0)?;
            let samples = parse_pcm(args.get(1))?;
            locked!(runtime)
                .with_mixer_mut(|mixer| mixer.submit_script_processor(id, &samples))
                .map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("audioNodeConnect", move |args| {
            let from = audio_id(args, 0)?;
            let to = audio_id(args, 1)?;
            locked!(runtime)
                .with_mixer_mut(|mixer| mixer.connect(from, to))
                .map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
}

fn audio_id(args: &[HostValue], index: usize) -> Result<u64, JsException> {
    args.get(index)
        .and_then(HostValue::as_u64)
        .ok_or_else(|| JsException::new(format!("expected audio id at arg {index}")))
}

fn parse_pcm(value: Option<&HostValue>) -> Result<Vec<f32>, JsException> {
    match value {
        Some(HostValue::Bytes(bytes)) => {
            if !bytes.len().is_multiple_of(4) {
                return Err(JsException::new("PCM bytes must be a multiple of 4"));
            }
            Ok(bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|chunk| f32::from_le_bytes(*chunk))
                .collect())
        }
        Some(HostValue::Array(items)) => Ok(items
            .iter()
            .filter_map(HostValue::as_f64)
            .map(|value| value as f32)
            .collect()),
        Some(_) => Err(JsException::new(
            "PCM data must be Float32 bytes or numbers",
        )),
        None => Ok(Vec::new()),
    }
}

fn f32_to_le_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(len: usize, freq: f32, sample_rate: f32) -> Vec<f32> {
        (0..len)
            .map(|index| (2.0 * std::f32::consts::PI * freq * index as f32 / sample_rate).sin())
            .collect()
    }

    fn object_id(value: &HostValue) -> u64 {
        value
            .as_object()
            .and_then(|map| map.get("id"))
            .and_then(HostValue::as_u64)
            .expect("audio object id")
    }

    fn create_graph(api: &HostApiRegistry) -> (u64, u64, u64, u64) {
        let context = api
            .call("audioContextCreate", &[])
            .expect("create context")
            .as_object()
            .cloned()
            .expect("context object");
        let context_id = context.get("id").and_then(HostValue::as_u64).expect("id");
        let destination = context
            .get("destination")
            .and_then(HostValue::as_object)
            .and_then(|map| map.get("id"))
            .cloned()
            .and_then(|id| id.as_u64())
            .expect("destination");
        let buffer = api
            .call(
                "audioBufferCreate",
                &[
                    HostValue::BigInt(context_id),
                    HostValue::Number(1.0),
                    HostValue::Number(128.0),
                    HostValue::Number(44_100.0),
                ],
            )
            .expect("buffer");
        let buffer_id = object_id(&buffer);
        let pcm = sine(128, 440.0, 44_100.0);
        api.call(
            "audioBufferCopyToChannel",
            &[
                HostValue::BigInt(buffer_id),
                HostValue::Number(0.0),
                HostValue::Array(
                    pcm.into_iter()
                        .map(|s| HostValue::Number(s as f64))
                        .collect(),
                ),
            ],
        )
        .expect("copy pcm");
        let source = api
            .call("audioBufferSourceCreate", &[HostValue::BigInt(context_id)])
            .expect("source");
        let source_id = object_id(&source);
        api.call(
            "audioBufferSourceSetBuffer",
            &[HostValue::BigInt(source_id), HostValue::BigInt(buffer_id)],
        )
        .expect("set buffer");
        let gain = api
            .call("audioGainCreate", &[HostValue::BigInt(context_id)])
            .expect("gain");
        let gain_id = object_id(&gain);
        api.call(
            "audioGainSetValue",
            &[HostValue::BigInt(gain_id), HostValue::Number(0.75)],
        )
        .expect("gain value");
        api.call(
            "audioNodeConnect",
            &[HostValue::BigInt(source_id), HostValue::BigInt(gain_id)],
        )
        .expect("source -> gain");
        api.call(
            "audioNodeConnect",
            &[HostValue::BigInt(gain_id), HostValue::BigInt(destination)],
        )
        .expect("gain -> destination");
        (context_id, source_id, gain_id, buffer_id)
    }

    #[test]
    fn pcm_buffer_source_gain_reaches_mock_sink() {
        let sink = MockAudioSink::new(44_100, 1);
        let runtime = shared_audio_runtime_with_mock(sink.clone());
        let mut api = HostApiRegistry::new();
        register_audio_host_ops(&mut api, Arc::clone(&runtime));
        let (_context, source, _gain, _buffer) = create_graph(&api);
        api.call("audioBufferSourceStart", &[HostValue::BigInt(source)])
            .expect("start");
        runtime.lock().unwrap().mix_frames(128).expect("mix");
        assert!(
            sink.has_nonsilent(),
            "mock sink must receive the PCM sine, not silence"
        );
        let peak = sink
            .captured()
            .iter()
            .copied()
            .fold(0.0f32, |acc, sample| acc.max(sample.abs()));
        assert!(peak > 0.5, "gain 0.75 on a unit sine should stay audible");
        assert!(peak <= 0.75 + 1.0e-3);
    }

    #[test]
    fn no_sink_fails_with_not_supported() {
        let runtime = shared_audio_runtime_unsupported();
        let mut api = HostApiRegistry::new();
        register_audio_host_ops(&mut api, runtime);
        let error = api
            .call("audioContextCreate", &[])
            .expect_err("missing output must fail");
        assert_eq!(error.name, "NotSupportedError");
        assert!(
            error.message.contains("no audio host or output device"),
            "failure must name the missing host/device: {}",
            error.message
        );
    }

    #[test]
    fn script_processor_pcm_callback_reaches_mock_sink() {
        let sink = MockAudioSink::new(44_100, 1);
        let runtime = shared_audio_runtime_with_mock(sink.clone());
        let mut api = HostApiRegistry::new();
        register_audio_host_ops(&mut api, Arc::clone(&runtime));
        let context = api
            .call("audioContextCreate", &[])
            .unwrap()
            .as_object()
            .cloned()
            .unwrap();
        let context_id = context.get("id").and_then(HostValue::as_u64).unwrap();
        let destination = context
            .get("destination")
            .and_then(HostValue::as_object)
            .and_then(|map| map.get("id"))
            .cloned()
            .and_then(|id| id.as_u64())
            .unwrap();
        let processor = api
            .call(
                "audioScriptProcessorCreate",
                &[
                    HostValue::BigInt(context_id),
                    HostValue::Number(256.0),
                    HostValue::Number(1.0),
                    HostValue::Number(1.0),
                ],
            )
            .unwrap();
        let processor_id = object_id(&processor);
        api.call(
            "audioNodeConnect",
            &[
                HostValue::BigInt(processor_id),
                HostValue::BigInt(destination),
            ],
        )
        .unwrap();
        let pcm = sine(256, 330.0, 44_100.0);
        api.call(
            "audioScriptProcessorSubmit",
            &[
                HostValue::BigInt(processor_id),
                HostValue::Array(
                    pcm.into_iter()
                        .map(|s| HostValue::Number(s as f64))
                        .collect(),
                ),
            ],
        )
        .unwrap();
        runtime.lock().unwrap().mix_frames(256).unwrap();
        assert!(sink.has_nonsilent());
    }

    #[test]
    fn float32_bytes_fill_audio_buffer() {
        let sink = MockAudioSink::new(8_000, 1);
        let runtime = shared_audio_runtime_with_mock(sink.clone());
        let mut api = HostApiRegistry::new();
        register_audio_host_ops(&mut api, Arc::clone(&runtime));
        let context = api.call("audioContextCreate", &[]).unwrap();
        let context_id = object_id(&context);
        let buffer = api
            .call(
                "audioBufferCreate",
                &[
                    HostValue::BigInt(context_id),
                    HostValue::Number(1.0),
                    HostValue::Number(4.0),
                    HostValue::Number(8_000.0),
                ],
            )
            .unwrap();
        let buffer_id = object_id(&buffer);
        let samples = [0.1f32, -0.2, 0.3, -0.4];
        api.call(
            "audioBufferCopyToChannel",
            &[
                HostValue::BigInt(buffer_id),
                HostValue::Number(0.0),
                HostValue::Bytes(f32_to_le_bytes(&samples)),
            ],
        )
        .unwrap();
        let bytes = api
            .call(
                "audioBufferGetChannelData",
                &[HostValue::BigInt(buffer_id), HostValue::Number(0.0)],
            )
            .unwrap();
        let roundtrip = parse_pcm(Some(&bytes)).unwrap();
        assert_eq!(roundtrip, samples);
    }

    #[test]
    fn audio_runtime_is_send_so_host_ops_can_capture_it() {
        fn assert_send<T: Send>() {}
        assert_send::<AudioRuntime>();
        assert_send::<SharedAudioRuntime>();
        assert_send::<Mixer>();
    }

    #[test]
    fn buffer_source_start_without_buffer_fails_clearly() {
        let sink = MockAudioSink::new(8_000, 1);
        let runtime = shared_audio_runtime_with_mock(sink);
        let mut api = HostApiRegistry::new();
        register_audio_host_ops(&mut api, runtime);
        let context = api.call("audioContextCreate", &[]).unwrap();
        let context_id = object_id(&context);
        let source = api
            .call("audioBufferSourceCreate", &[HostValue::BigInt(context_id)])
            .unwrap();
        let error = api
            .call(
                "audioBufferSourceStart",
                &[HostValue::BigInt(object_id(&source))],
            )
            .expect_err("start without a buffer must fail");
        assert_eq!(error.name, "InvalidStateError");
    }

    #[test]
    fn create_buffer_rejects_out_of_range_channels_instead_of_clamping() {
        let sink = MockAudioSink::new(8_000, 1);
        let runtime = shared_audio_runtime_with_mock(sink);
        let mut api = HostApiRegistry::new();
        register_audio_host_ops(&mut api, runtime);
        let context = api.call("audioContextCreate", &[]).unwrap();
        let context_id = object_id(&context);
        let zero = api
            .call(
                "audioBufferCreate",
                &[
                    HostValue::BigInt(context_id),
                    HostValue::Number(0.0),
                    HostValue::Number(4.0),
                    HostValue::Number(8_000.0),
                ],
            )
            .expect_err("zero channels must fail closed");
        assert_eq!(zero.name, "NotSupportedError");
        let too_many = api
            .call(
                "audioBufferCreate",
                &[
                    HostValue::BigInt(context_id),
                    HostValue::Number(33.0),
                    HostValue::Number(4.0),
                    HostValue::Number(8_000.0),
                ],
            )
            .expect_err("more than 32 channels must fail closed");
        assert_eq!(too_many.name, "NotSupportedError");
    }

    #[test]
    fn buffer_source_start_twice_fails_clearly() {
        let sink = MockAudioSink::new(8_000, 1);
        let runtime = shared_audio_runtime_with_mock(sink);
        let mut api = HostApiRegistry::new();
        register_audio_host_ops(&mut api, runtime);
        let (_context, source, _gain, _buffer) = create_graph(&api);
        api.call("audioBufferSourceStart", &[HostValue::BigInt(source)])
            .expect("first start");
        let error = api
            .call("audioBufferSourceStart", &[HostValue::BigInt(source)])
            .expect_err("start may run only once");
        assert_eq!(error.name, "InvalidStateError");
    }
}
