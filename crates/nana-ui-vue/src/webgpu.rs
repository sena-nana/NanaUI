//! WebGPU-style JavaScript resource executor over NanaUI's host-owned WGPU
//! device and queue. It never requests an adapter or creates a second device.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use nana_js_engine::{HostApiRegistry, HostCompletion, HostValue, JsException};
use nana_ui::{HostTexture, HostTextureAlphaMode, HostTextureRegistry, HostedGpuResources};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GpuId(u64);

struct TextureResource {
    texture: Arc<wgpu::Texture>,
    width: u32,
    height: u32,
    depth: u32,
    mip_level_count: u32,
    sample_count: u32,
    dimension: wgpu::TextureDimension,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    generation: u64,
    canvas_slot: Option<String>,
}

#[derive(Debug, Clone)]
enum PassCommand {
    SetPipeline(GpuId),
    SetBindGroup(u32, GpuId, Vec<u32>),
    SetVertexBuffer(u32, GpuId, u64, Option<u64>),
    SetIndexBuffer(GpuId, wgpu::IndexFormat, u64, Option<u64>),
    SetViewport(f32, f32, f32, f32, f32, f32),
    SetScissorRect(u32, u32, u32, u32),
    SetBlendConstant(wgpu::Color),
    SetStencilReference(u32),
    Draw(u32, u32, u32, u32),
    DrawIndexed(u32, u32, u32, i32, u32),
    Dispatch(u32, u32, u32),
}

#[derive(Debug, Clone)]
struct ColorAttachment {
    view: GpuId,
    resolve_target: Option<GpuId>,
    clear: Option<wgpu::Color>,
    store: bool,
}

#[derive(Debug, Clone)]
struct DepthStencilAttachment {
    view: GpuId,
    depth_clear: Option<f32>,
    depth_store: bool,
    depth_read_only: bool,
    stencil_clear: Option<u32>,
    stencil_store: bool,
    stencil_read_only: bool,
}

#[derive(Debug, Clone)]
enum EncoderCommand {
    Render {
        colors: Vec<ColorAttachment>,
        depth_stencil: Option<DepthStencilAttachment>,
        commands: Vec<PassCommand>,
    },
    Compute {
        commands: Vec<PassCommand>,
    },
    CopyBuffer {
        source: GpuId,
        source_offset: u64,
        destination: GpuId,
        destination_offset: u64,
        size: u64,
    },
}

#[derive(Debug)]
struct OpenPass {
    encoder: GpuId,
    render_colors: Option<Vec<ColorAttachment>>,
    render_depth_stencil: Option<DepthStencilAttachment>,
    commands: Vec<PassCommand>,
}

#[derive(Default)]
struct WebGpuState {
    next_id: u64,
    generation: u64,
    buffers: HashMap<GpuId, Arc<wgpu::Buffer>>,
    buffer_sizes: HashMap<GpuId, u64>,
    buffer_usages: HashMap<GpuId, wgpu::BufferUsages>,
    initially_mapped_buffers: HashSet<GpuId>,
    textures: HashMap<GpuId, TextureResource>,
    views: HashMap<GpuId, Arc<wgpu::TextureView>>,
    view_textures: HashMap<GpuId, GpuId>,
    view_extents: HashMap<GpuId, (u32, u32, u32)>,
    samplers: HashMap<GpuId, Arc<wgpu::Sampler>>,
    shaders: HashMap<GpuId, Arc<wgpu::ShaderModule>>,
    bind_group_layouts: HashMap<GpuId, Arc<wgpu::BindGroupLayout>>,
    pipeline_layouts: HashMap<GpuId, Arc<wgpu::PipelineLayout>>,
    bind_groups: HashMap<GpuId, Arc<wgpu::BindGroup>>,
    render_pipelines: HashMap<GpuId, Arc<wgpu::RenderPipeline>>,
    compute_pipelines: HashMap<GpuId, Arc<wgpu::ComputePipeline>>,
    encoders: HashMap<GpuId, Vec<EncoderCommand>>,
    command_buffers: HashMap<GpuId, Vec<EncoderCommand>>,
    passes: HashMap<GpuId, OpenPass>,
    canvas_textures: HashMap<u64, GpuId>,
}

type ErrorScopeFuture = Pin<Box<dyn Future<Output = Option<wgpu::Error>> + 'static>>;

struct PendingErrorScope {
    future: ErrorScopeFuture,
    completion: Option<HostCompletion>,
}

#[derive(Default)]
struct ThreadErrorScopes {
    stack: Vec<wgpu::ErrorScopeGuard>,
    pending: Vec<PendingErrorScope>,
}

thread_local! {
    static ERROR_SCOPES: RefCell<HashMap<u64, ThreadErrorScopes>> = RefCell::new(HashMap::new());
}

static NEXT_WEBGPU_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);

type PendingQueueCompletion = Arc<Mutex<Option<HostCompletion>>>;

impl WebGpuState {
    fn alloc(&mut self) -> GpuId {
        self.next_id = self.next_id.saturating_add(1).max(1);
        GpuId(self.next_id)
    }

    fn clear(&mut self) {
        let generation = self.generation.saturating_add(1).max(1);
        let next_id = self.next_id;
        *self = Self {
            next_id,
            generation,
            ..Self::default()
        };
    }
}

#[derive(Clone)]
pub struct JsWebGpuRuntime {
    runtime_id: u64,
    resources: Arc<Mutex<HostedGpuResources>>,
    textures: HostTextureRegistry,
    state: Arc<Mutex<WebGpuState>>,
    pending_queue_completions: Arc<Mutex<HashMap<u64, PendingQueueCompletion>>>,
    next_completion_id: Arc<AtomicU64>,
    next_completion_poll: Arc<Mutex<Option<Instant>>>,
}

impl std::fmt::Debug for JsWebGpuRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let counts = self.resource_counts();
        f.debug_struct("JsWebGpuRuntime")
            .field("generation", &self.generation())
            .field("resources", &counts)
            .finish()
    }
}

impl JsWebGpuRuntime {
    pub fn new(resources: HostedGpuResources, textures: HostTextureRegistry) -> Self {
        Self {
            runtime_id: NEXT_WEBGPU_RUNTIME_ID.fetch_add(1, Ordering::Relaxed),
            resources: Arc::new(Mutex::new(resources)),
            textures,
            state: Arc::new(Mutex::new(WebGpuState {
                generation: 1,
                ..WebGpuState::default()
            })),
            pending_queue_completions: Arc::new(Mutex::new(HashMap::new())),
            next_completion_id: Arc::new(AtomicU64::new(1)),
            next_completion_poll: Arc::new(Mutex::new(None)),
        }
    }

    pub fn generation(&self) -> u64 {
        self.state.lock().map(|state| state.generation).unwrap_or(0)
    }

    pub fn replace_device(&self, resources: HostedGpuResources) -> u64 {
        if let Ok(mut current) = self.resources.lock() {
            *current = resources;
        }
        self.textures.invalidate_all();
        self.reject_pending_completions("GPU device was replaced");
        if let Ok(mut state) = self.state.lock() {
            state.clear();
            state.generation
        } else {
            0
        }
    }

    /// Advance host-owned GPU callbacks without blocking a UI frame.
    pub fn poll(&self) -> usize {
        let poll_scheduled = self
            .next_completion_poll
            .lock()
            .is_ok_and(|deadline| deadline.is_some());
        let queued_before = self
            .pending_queue_completions
            .lock()
            .map(|pending| pending.len())
            .unwrap_or(0);
        let Ok(resources) = self.resources.lock() else {
            return 0;
        };
        let _ = resources.device().poll(wgpu::PollType::Poll);
        drop(resources);
        let queued_after = self
            .pending_queue_completions
            .lock()
            .map(|pending| pending.len())
            .unwrap_or(queued_before);

        let waker = std::task::Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut completed = queued_before.saturating_sub(queued_after);
        ERROR_SCOPES.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            let Some(scopes) = scopes.get_mut(&self.runtime_id) else {
                return;
            };
            scopes
                .pending
                .retain_mut(|entry| match entry.future.as_mut().poll(&mut context) {
                    Poll::Ready(error) => {
                        if let Some(completion) = entry.completion.take() {
                            completion
                                .resolve(error.map(gpu_error_value).unwrap_or(HostValue::Null));
                        }
                        completed += 1;
                        false
                    }
                    Poll::Pending => true,
                });
        });
        if let Ok(mut deadline) = self.next_completion_poll.lock() {
            *deadline = self
                .has_pending_completions()
                .then(|| Instant::now() + Duration::from_millis(1));
        }
        // Queue callbacks may run between event-loop turns and remove their
        // runtime slot before this poll starts. A scheduled poll must still
        // drain the corresponding HostPendingCall into V8.
        if poll_scheduled {
            completed = completed.max(1);
        }
        completed
    }

    pub fn next_wakeup(&self) -> Option<Instant> {
        self.next_completion_poll
            .lock()
            .ok()
            .and_then(|value| *value)
    }

    fn schedule_completion_poll(&self) {
        if let Ok(mut deadline) = self.next_completion_poll.lock()
            && deadline.is_none()
        {
            *deadline = Some(Instant::now());
        }
    }

    pub fn has_pending_completions(&self) -> bool {
        ERROR_SCOPES.with(|scopes| {
            scopes
                .borrow()
                .get(&self.runtime_id)
                .is_some_and(|scopes| !scopes.pending.is_empty())
        }) || self
            .pending_queue_completions
            .lock()
            .is_ok_and(|pending| !pending.is_empty())
    }

    fn reject_pending_completions(&self, message: &str) {
        ERROR_SCOPES.with(|scopes| {
            if let Some(scopes) = scopes.borrow_mut().remove(&self.runtime_id) {
                for mut entry in scopes.pending {
                    if let Some(completion) = entry.completion.take() {
                        completion.reject(gpu_device_lost(message));
                    }
                }
            }
        });
        if let Ok(mut pending) = self.pending_queue_completions.lock() {
            for completion in pending.values() {
                if let Ok(mut completion) = completion.lock()
                    && let Some(completion) = completion.take()
                {
                    completion.reject(gpu_device_lost(message));
                }
            }
            pending.clear();
        }
        if let Ok(mut deadline) = self.next_completion_poll.lock() {
            *deadline = None;
        }
    }

    pub fn resource_counts(&self) -> BTreeMap<&'static str, usize> {
        let Ok(state) = self.state.lock() else {
            return BTreeMap::new();
        };
        [
            ("buffers", state.buffers.len()),
            ("textures", state.textures.len()),
            ("views", state.views.len()),
            ("samplers", state.samplers.len()),
            ("shaders", state.shaders.len()),
            ("bindGroups", state.bind_groups.len()),
            ("renderPipelines", state.render_pipelines.len()),
            ("computePipelines", state.compute_pipelines.len()),
            ("commandBuffers", state.command_buffers.len()),
        ]
        .into_iter()
        .collect()
    }

    pub fn register_host_ops(&self, api: &mut HostApiRegistry) {
        self.register_identity_ops(api);
        self.register_buffer_ops(api);
        self.register_texture_ops(api);
        self.register_binding_ops(api);
        self.register_pipeline_ops(api);
        self.register_command_ops(api);
    }

    fn register_identity_ops(&self, api: &mut HostApiRegistry) {
        let runtime = self.clone();
        api.register("webgpuAdapterInfo", move |_| {
            let resources = runtime.resources.lock().map_err(poisoned)?;
            let info = resources.adapter_info();
            Ok(HostValue::Object(
                [
                    ("vendor".into(), HostValue::String(info.vendor.to_string())),
                    ("device".into(), HostValue::String(info.device.to_string())),
                    ("description".into(), HostValue::String(info.name.clone())),
                    (
                        "architecture".into(),
                        HostValue::String(info.driver_info.clone()),
                    ),
                    (
                        "backend".into(),
                        HostValue::String(format!("{:?}", info.backend).to_ascii_lowercase()),
                    ),
                    (
                        "generation".into(),
                        HostValue::Number(runtime.generation() as f64),
                    ),
                ]
                .into_iter()
                .collect(),
            ))
        });
        let runtime = self.clone();
        api.register("webgpuResourceCounts", move |_| {
            Ok(HostValue::Object(
                runtime
                    .resource_counts()
                    .into_iter()
                    .map(|(key, value)| (key.into(), HostValue::Number(value as f64)))
                    .collect(),
            ))
        });
        let runtime = self.clone();
        api.register("webgpuResourceRelease", move |args| {
            let id = gpu_id(args, 0)?;
            Ok(HostValue::Bool(runtime.release(id)))
        });
        let runtime = self.clone();
        api.register("webgpuPushErrorScope", move |args| {
            let filter = match args.first().and_then(HostValue::as_str) {
                Some("validation") => wgpu::ErrorFilter::Validation,
                Some("out-of-memory") => wgpu::ErrorFilter::OutOfMemory,
                Some("internal") => wgpu::ErrorFilter::Internal,
                Some(filter) => {
                    return Err(webgpu_validation(format!(
                        "unsupported error scope filter `{filter}`"
                    )));
                }
                None => return Err(webgpu_validation("error scope filter is required")),
            };
            let guard = runtime.device()?.push_error_scope(filter);
            ERROR_SCOPES.with(|scopes| {
                scopes
                    .borrow_mut()
                    .entry(runtime.runtime_id)
                    .or_default()
                    .stack
                    .push(guard);
            });
            Ok(HostValue::Null)
        });
        let runtime = self.clone();
        api.register_async("webgpuPopErrorScope", move |_args, context| {
            let guard = ERROR_SCOPES.with(|scopes| {
                scopes
                    .borrow_mut()
                    .get_mut(&runtime.runtime_id)
                    .and_then(|scopes| scopes.stack.pop())
            });
            let guard = guard.ok_or_else(|| webgpu_validation("error scope stack is empty"))?;
            let (completion, pending) = context.pending();
            ERROR_SCOPES.with(|scopes| {
                scopes
                    .borrow_mut()
                    .entry(runtime.runtime_id)
                    .or_default()
                    .pending
                    .push(PendingErrorScope {
                        future: Box::pin(async move { guard.pop().await }),
                        completion: Some(completion),
                    });
            });
            runtime.schedule_completion_poll();
            Ok(pending)
        });
        let runtime = self.clone();
        api.register_async("webgpuQueueSubmittedWorkDone", move |_args, context| {
            let queue = runtime.queue()?;
            let (completion, pending) = context.pending();
            let id = runtime.next_completion_id.fetch_add(1, Ordering::Relaxed);
            let slot = Arc::new(Mutex::new(Some(completion)));
            runtime
                .pending_queue_completions
                .lock()
                .map_err(poisoned)?
                .insert(id, Arc::clone(&slot));
            runtime.schedule_completion_poll();
            let completions = Arc::clone(&runtime.pending_queue_completions);
            queue.on_submitted_work_done(move || {
                if let Ok(mut completion) = slot.lock()
                    && let Some(completion) = completion.take()
                {
                    completion.resolve(HostValue::Null);
                }
                if let Ok(mut pending) = completions.lock() {
                    pending.remove(&id);
                }
            });
            Ok(pending)
        });
    }

    fn register_buffer_ops(&self, api: &mut HostApiRegistry) {
        let runtime = self.clone();
        api.register("webgpuCreateBuffer", move |args| {
            let descriptor = object_arg(args, 0)?;
            if !descriptor.contains_key("size") || !descriptor.contains_key("usage") {
                return Err(webgpu_validation(
                    "createBuffer requires explicit size and usage",
                ));
            }
            let size = checked_u64_field(descriptor, "size", 0)?;
            let usage = checked_u32_field(descriptor, "usage", 0)?;
            let usage = wgpu::BufferUsages::from_bits(usage)
                .filter(|usage| !usage.is_empty())
                .ok_or_else(|| webgpu_validation("createBuffer usage contains unsupported bits"))?;
            validate_buffer_descriptor(size, usage)?;
            let mapped_at_creation = checked_bool_field(descriptor, "mappedAtCreation", false)?;
            if mapped_at_creation && size % 4 != 0 {
                return Err(webgpu_validation(
                    "mappedAtCreation buffer size must be a multiple of 4",
                ));
            }
            let buffer = runtime.validation_scope(|device| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: label(descriptor),
                    size,
                    usage,
                    mapped_at_creation,
                })
            })?;
            let mut state = runtime.state.lock().map_err(poisoned)?;
            let id = state.alloc();
            state.buffers.insert(id, Arc::new(buffer));
            state.buffer_sizes.insert(id, size);
            state.buffer_usages.insert(id, usage);
            if mapped_at_creation {
                state.initially_mapped_buffers.insert(id);
            }
            Ok(resource_value(
                id,
                "buffer",
                state.generation,
                [("size", HostValue::Number(size as f64))],
            ))
        });
        let runtime = self.clone();
        api.register("webgpuBufferUnmapInitial", move |args| {
            let id = gpu_id(args, 0)?;
            let bytes = args
                .get(1)
                .and_then(HostValue::as_bytes)
                .ok_or_else(|| JsException::new("unmap requires mapped ArrayBuffer data"))?;
            let buffer = {
                let state = runtime.state.lock().map_err(poisoned)?;
                if !state.initially_mapped_buffers.contains(&id) {
                    return Err(webgpu_validation(
                        "buffer is not mapped from mappedAtCreation",
                    ));
                }
                let size = state
                    .buffer_sizes
                    .get(&id)
                    .copied()
                    .ok_or_else(|| unknown("buffer", id))?;
                if bytes.len() as u64 != size {
                    return Err(webgpu_validation(
                        "mappedAtCreation data must cover the complete buffer",
                    ));
                }
                state
                    .buffers
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| unknown("buffer", id))?
            };
            {
                let mut mapped = buffer.slice(..).get_mapped_range_mut().map_err(|error| {
                    webgpu_validation(format!("failed to access mappedAtCreation range: {error}"))
                })?;
                mapped.copy_from_slice(bytes);
            }
            buffer.unmap();
            runtime
                .state
                .lock()
                .map_err(poisoned)?
                .initially_mapped_buffers
                .remove(&id);
            Ok(HostValue::Null)
        });
        let runtime = self.clone();
        api.register("webgpuQueueWriteBuffer", move |args| {
            let id = gpu_id(args, 0)?;
            let offset = checked_u64_arg(args, 1, Some(0))?;
            let bytes = args
                .get(2)
                .and_then(HostValue::as_bytes)
                .ok_or_else(|| JsException::new("writeBuffer requires ArrayBuffer data"))?;
            if offset % wgpu::COPY_BUFFER_ALIGNMENT != 0 {
                return Err(webgpu_validation(
                    "writeBuffer bufferOffset must be a multiple of 4",
                ));
            }
            if !(bytes.len() as u64).is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT) {
                return Err(webgpu_validation(
                    "writeBuffer data size must be a multiple of 4",
                ));
            }
            let (buffer, buffer_size, usage) = {
                let state = runtime.state.lock().map_err(poisoned)?;
                let buffer = state
                    .buffers
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| unknown("buffer", id))?;
                let size = state.buffer_sizes.get(&id).copied().unwrap_or_default();
                let usage = state
                    .buffer_usages
                    .get(&id)
                    .copied()
                    .unwrap_or_else(wgpu::BufferUsages::empty);
                (buffer, size, usage)
            };
            if !usage.contains(wgpu::BufferUsages::COPY_DST) {
                return Err(webgpu_validation(
                    "writeBuffer destination lacks COPY_DST usage",
                ));
            }
            let end = offset
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| webgpu_validation("writeBuffer range overflows"))?;
            if end > buffer_size {
                return Err(webgpu_validation("writeBuffer range exceeds buffer size"));
            }
            let queue = runtime.queue()?;
            queue.write_buffer(&buffer, offset, bytes);
            Ok(HostValue::Null)
        });
        let runtime = self.clone();
        api.register("webgpuBufferDestroy", move |args| {
            let id = gpu_id(args, 0)?;
            let buffer = {
                let mut state = runtime.state.lock().map_err(poisoned)?;
                state.buffer_sizes.remove(&id);
                state.buffer_usages.remove(&id);
                state.initially_mapped_buffers.remove(&id);
                state.buffers.remove(&id)
            };
            if let Some(buffer) = buffer {
                buffer.destroy();
                return Ok(HostValue::Bool(true));
            }
            Ok(HostValue::Bool(false))
        });
    }

    fn register_texture_ops(&self, api: &mut HostApiRegistry) {
        let runtime = self.clone();
        api.register("webgpuCreateTexture", move |args| {
            runtime.create_texture(object_arg(args, 0)?, None)
        });
        let runtime = self.clone();
        api.register("webgpuTextureCreateView", move |args| {
            let texture_id = gpu_id(args, 0)?;
            let descriptor = match args.get(1) {
                None | Some(HostValue::Undefined) => None,
                Some(value) => Some(value.as_object().ok_or_else(|| {
                    webgpu_validation("texture view descriptor must be an object")
                })?),
            };
            let state = runtime.state.lock().map_err(poisoned)?;
            let resource = state
                .textures
                .get(&texture_id)
                .ok_or_else(|| unknown("texture", texture_id))?;
            let texture = Arc::clone(&resource.texture);
            let base_mip_level = descriptor
                .map(|d| checked_u32_field(d, "baseMipLevel", 0))
                .transpose()?
                .unwrap_or(0);
            let mip_level_count = descriptor
                .map(|d| optional_u32_field(d, "mipLevelCount"))
                .transpose()?
                .flatten();
            if base_mip_level >= resource.mip_level_count
                || mip_level_count.is_some_and(|count| {
                    count == 0
                        || base_mip_level
                            .checked_add(count)
                            .is_none_or(|end| end > resource.mip_level_count)
                })
            {
                return Err(webgpu_validation("texture view mip range is out of bounds"));
            }
            let base_array_layer = descriptor
                .map(|d| checked_u32_field(d, "baseArrayLayer", 0))
                .transpose()?
                .unwrap_or(0);
            let array_layer_count = descriptor
                .map(|d| optional_u32_field(d, "arrayLayerCount"))
                .transpose()?
                .flatten();
            if base_array_layer >= resource.depth
                || array_layer_count.is_some_and(|count| {
                    count == 0
                        || base_array_layer
                            .checked_add(count)
                            .is_none_or(|end| end > resource.depth)
                })
            {
                return Err(webgpu_validation(
                    "texture view array-layer range is out of bounds",
                ));
            }
            let view_format = descriptor
                .map(|d| {
                    d.get("format")
                        .map(|_| required_str_field(d, "format"))
                        .transpose()?
                        .map(parse_texture_format_checked)
                        .transpose()
                })
                .transpose()?
                .flatten();
            if view_format.is_some_and(|format| format != resource.format) {
                return Err(webgpu_validation(
                    "texture view format reinterpretation is not supported by NanaUI",
                ));
            }
            let view_dimension = descriptor
                .map(|d| {
                    d.get("dimension")
                        .map(|_| required_str_field(d, "dimension"))
                        .transpose()?
                        .map(parse_view_dimension_checked)
                        .transpose()
                })
                .transpose()?
                .flatten();
            let view_usage = descriptor
                .map(|d| optional_u32_field(d, "usage"))
                .transpose()?
                .flatten()
                .map(|bits| {
                    wgpu::TextureUsages::from_bits(bits).ok_or_else(|| {
                        webgpu_validation("texture view usage contains unsupported bits")
                    })
                })
                .transpose()?;
            if view_usage.is_some_and(|usage| !resource.usage.contains(usage)) {
                return Err(webgpu_validation(
                    "texture view usage is not a subset of texture usage",
                ));
            }
            let aspect = descriptor
                .map(|d| parse_texture_aspect_checked(checked_str_field(d, "aspect", "all")?))
                .transpose()?
                .unwrap_or(wgpu::TextureAspect::All);
            if !texture_aspect_compatible(resource.format, aspect) {
                return Err(webgpu_validation(
                    "texture view aspect is incompatible with its format",
                ));
            }
            let view_extent = (
                (resource.width >> base_mip_level).max(1),
                (resource.height >> base_mip_level).max(1),
                array_layer_count.unwrap_or(resource.depth - base_array_layer),
            );
            drop(state);
            let view = runtime.validation_scope(|_| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: descriptor.and_then(label),
                    format: view_format,
                    dimension: view_dimension,
                    usage: view_usage,
                    aspect,
                    base_mip_level,
                    mip_level_count,
                    base_array_layer,
                    array_layer_count,
                })
            })?;
            let mut state = runtime.state.lock().map_err(poisoned)?;
            let id = state.alloc();
            state.views.insert(id, Arc::new(view));
            state.view_textures.insert(id, texture_id);
            state.view_extents.insert(id, view_extent);
            Ok(resource_value(id, "texture-view", state.generation, []))
        });
        let runtime = self.clone();
        api.register("webgpuTextureDestroy", move |args| {
            let id = gpu_id(args, 0)?;
            let mut state = runtime.state.lock().map_err(poisoned)?;
            if let Some(texture) = state.textures.remove(&id) {
                texture.texture.destroy();
                remove_texture_views(&mut state, id);
                if let Some(slot) = texture.canvas_slot {
                    runtime.textures.remove(&slot);
                }
                return Ok(HostValue::Bool(true));
            }
            Ok(HostValue::Bool(false))
        });
        let runtime = self.clone();
        api.register("webgpuCreateSampler", move |args| {
            let d = object_arg(args, 0)?;
            let address_mode_u =
                parse_address(checked_str_field(d, "addressModeU", "clamp-to-edge")?)?;
            let address_mode_v =
                parse_address(checked_str_field(d, "addressModeV", "clamp-to-edge")?)?;
            let address_mode_w =
                parse_address(checked_str_field(d, "addressModeW", "clamp-to-edge")?)?;
            let mag_filter = parse_filter(checked_str_field(d, "magFilter", "nearest")?)?;
            let min_filter = parse_filter(checked_str_field(d, "minFilter", "nearest")?)?;
            let mipmap_filter =
                parse_mipmap_filter(checked_str_field(d, "mipmapFilter", "nearest")?)?;
            let lod_min_clamp = checked_f32_field(d, "lodMinClamp", 0.0)?;
            let lod_max_clamp = checked_f32_field(d, "lodMaxClamp", 32.0)?;
            if lod_min_clamp > lod_max_clamp {
                return Err(webgpu_validation("lodMinClamp must not exceed lodMaxClamp"));
            }
            let compare = d
                .get("compare")
                .map(|value| {
                    value
                        .as_str()
                        .ok_or_else(|| webgpu_validation("sampler compare must be a string"))
                        .and_then(parse_compare)
                })
                .transpose()?;
            let anisotropy = checked_u32_field(d, "maxAnisotropy", 1)?;
            let anisotropy_clamp = u16::try_from(anisotropy)
                .map_err(|_| webgpu_validation("maxAnisotropy exceeds the u16 range"))?;
            if anisotropy_clamp == 0 {
                return Err(webgpu_validation("maxAnisotropy must be at least 1"));
            }
            if anisotropy_clamp > 1
                && (mag_filter != wgpu::FilterMode::Linear
                    || min_filter != wgpu::FilterMode::Linear
                    || mipmap_filter != wgpu::MipmapFilterMode::Linear)
            {
                return Err(webgpu_validation(
                    "anisotropic samplers require all filters to be linear",
                ));
            }
            let sampler = runtime.validation_scope(|device| {
                device.create_sampler(&wgpu::SamplerDescriptor {
                    label: label(d),
                    address_mode_u,
                    address_mode_v,
                    address_mode_w,
                    mag_filter,
                    min_filter,
                    mipmap_filter,
                    lod_min_clamp,
                    lod_max_clamp,
                    compare,
                    anisotropy_clamp,
                    border_color: None,
                })
            })?;
            let mut state = runtime.state.lock().map_err(poisoned)?;
            let id = state.alloc();
            state.samplers.insert(id, Arc::new(sampler));
            Ok(resource_value(id, "sampler", state.generation, []))
        });
        let runtime = self.clone();
        api.register("webgpuCanvasConfigure", move |args| {
            let canvas_id = args
                .first()
                .and_then(HostValue::as_u64)
                .ok_or_else(|| JsException::new("canvas GPU context requires a canvas id"))?;
            let descriptor = object_arg(args, 1)?;
            let width = checked_u32_field(descriptor, "width", 1)?;
            let height = checked_u32_field(descriptor, "height", 1)?;
            if width == 0 || height == 0 {
                return Err(webgpu_validation("canvas dimensions must be non-zero"));
            }
            let slot = format!("webgpu-canvas:{canvas_id}");
            runtime.create_texture(descriptor, Some((canvas_id, slot, width, height)))
        });
        let runtime = self.clone();
        api.register("webgpuCanvasCurrentTexture", move |args| {
            let canvas_id = args
                .first()
                .and_then(HostValue::as_u64)
                .ok_or_else(|| JsException::new("missing canvas id"))?;
            let state = runtime.state.lock().map_err(poisoned)?;
            let id = *state
                .canvas_textures
                .get(&canvas_id)
                .ok_or_else(|| JsException::new("GPUCanvasContext is not configured"))?;
            let texture = state
                .textures
                .get(&id)
                .ok_or_else(|| unknown("texture", id))?;
            Ok(texture_value(id, state.generation, texture))
        });
    }

    fn create_texture(
        &self,
        descriptor: &BTreeMap<String, HostValue>,
        canvas: Option<(u64, String, u32, u32)>,
    ) -> Result<HostValue, JsException> {
        let (width, height, depth) = checked_size3(
            descriptor.get("size"),
            canvas.as_ref().map(|v| (v.2, v.3, 1)).unwrap_or((1, 1, 1)),
        )?;
        if canvas.is_none()
            && (!descriptor.contains_key("size")
                || !descriptor.contains_key("format")
                || !descriptor.contains_key("usage"))
        {
            return Err(webgpu_validation(
                "createTexture requires explicit size, format, and usage",
            ));
        }
        let format = if canvas.is_some() {
            parse_texture_format_checked(checked_str_field(descriptor, "format", "rgba8unorm")?)?
        } else {
            parse_texture_format_checked(required_str_field(descriptor, "format")?)?
        };
        let usage_bits = checked_u32_field(descriptor, "usage", 0)?;
        let mut usage = wgpu::TextureUsages::from_bits(usage_bits)
            .ok_or_else(|| webgpu_validation("texture usage contains unsupported bits"))?;
        if canvas.is_some() {
            usage |= wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC;
        }
        if usage.is_empty() {
            return Err(webgpu_validation("texture usage must not be empty"));
        }
        let mip_level_count = checked_u32_field(descriptor, "mipLevelCount", 1)?;
        let sample_count = checked_u32_field(descriptor, "sampleCount", 1)?;
        let dimension =
            parse_texture_dimension_checked(checked_str_field(descriptor, "dimension", "2d")?)?;
        validate_texture_descriptor(
            width,
            height,
            depth,
            mip_level_count,
            sample_count,
            dimension,
            format,
            usage,
        )?;
        let (texture, view) = self.validation_scope(|device| {
            let texture = Arc::new(device.create_texture(&wgpu::TextureDescriptor {
                label: label(descriptor),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: depth,
                },
                mip_level_count,
                sample_count,
                dimension,
                format,
                usage,
                view_formats: &[],
            }));
            let view = Arc::new(texture.create_view(&wgpu::TextureViewDescriptor::default()));
            (texture, view)
        })?;
        let mut state = self.state.lock().map_err(poisoned)?;
        let id = state.alloc();
        let canvas_slot = canvas.as_ref().map(|(_, slot, _, _)| slot.clone());
        let generation = state.generation;
        state.textures.insert(
            id,
            TextureResource {
                texture,
                width,
                height,
                depth,
                mip_level_count,
                sample_count,
                dimension,
                format,
                usage,
                generation,
                canvas_slot: canvas_slot.clone(),
            },
        );
        state.views.insert(id, Arc::clone(&view));
        state.view_textures.insert(id, id);
        state.view_extents.insert(id, (width, height, depth));
        if let Some((canvas_id, slot, _, _)) = canvas {
            let replaced = state.canvas_textures.insert(canvas_id, id);
            let alpha_mode = match checked_str_field(descriptor, "alphaMode", "premultiplied")? {
                "opaque" => HostTextureAlphaMode::Opaque,
                "premultiplied" => HostTextureAlphaMode::Premultiplied,
                value => {
                    return Err(webgpu_validation(format!(
                        "unsupported canvas alphaMode `{value}`"
                    )));
                }
            };
            self.textures.register(
                slot,
                HostTexture::from_wgpu(id.0, state.generation, (*view).clone()),
                width,
                height,
                alpha_mode,
            );
            if let Some(replaced) = replaced.filter(|old| *old != id) {
                remove_texture_views(&mut state, replaced);
                if let Some(old) = state.textures.remove(&replaced) {
                    old.texture.destroy();
                }
            }
        }
        let texture = state.textures.get(&id).expect("inserted texture");
        Ok(texture_value(id, state.generation, texture))
    }

    fn register_binding_ops(&self, api: &mut HostApiRegistry) {
        let runtime = self.clone();
        api.register("webgpuCreateBindGroupLayout", move |args| {
            let d = object_arg(args, 0)?;
            let entries = checked_array_field(d, "entries")?
                .iter()
                .map(|value| {
                    value
                        .as_object()
                        .ok_or_else(|| {
                            webgpu_validation("bind group layout entry must be an object")
                        })
                        .and_then(parse_layout_entry)
                })
                .collect::<Result<Vec<_>, _>>()?;
            ensure_unique_bindings(
                entries.iter().map(|entry| entry.binding),
                "bind group layout",
            )?;
            let layout = runtime.validation_scope(|device| {
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: label(d),
                    entries: &entries,
                })
            })?;
            let mut state = runtime.state.lock().map_err(poisoned)?;
            let id = state.alloc();
            state.bind_group_layouts.insert(id, Arc::new(layout));
            Ok(resource_value(
                id,
                "bind-group-layout",
                state.generation,
                [],
            ))
        });
        let runtime = self.clone();
        api.register("webgpuCreatePipelineLayout", move |args| {
            let d = object_arg(args, 0)?;
            let ids = checked_array_field(d, "bindGroupLayouts")?
                .iter()
                .map(|value| {
                    resource_id_value(value).ok_or_else(|| {
                        webgpu_validation("pipeline layout entry must be a GPU bind group layout")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let state = runtime.state.lock().map_err(poisoned)?;
            let layouts = ids
                .iter()
                .map(|id| {
                    state
                        .bind_group_layouts
                        .get(id)
                        .map(|v| Some(v.as_ref()))
                        .ok_or_else(|| unknown("bind-group-layout", *id))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let layout = runtime.validation_scope(|device| {
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: label(d),
                    bind_group_layouts: &layouts,
                    immediate_size: 0,
                })
            })?;
            drop(state);
            let mut state = runtime.state.lock().map_err(poisoned)?;
            let id = state.alloc();
            state.pipeline_layouts.insert(id, Arc::new(layout));
            Ok(resource_value(id, "pipeline-layout", state.generation, []))
        });
        let runtime = self.clone();
        api.register("webgpuCreateBindGroup", move |args| {
            runtime.create_bind_group(object_arg(args, 0)?)
        });
    }

    fn create_bind_group(
        &self,
        descriptor: &BTreeMap<String, HostValue>,
    ) -> Result<HostValue, JsException> {
        enum Owned {
            Buffer(Arc<wgpu::Buffer>, u64, Option<u64>),
            Sampler(Arc<wgpu::Sampler>),
            View(Arc<wgpu::TextureView>),
        }
        let layout_id = descriptor
            .get("layout")
            .and_then(resource_id_value)
            .ok_or_else(|| JsException::new("bind group layout is required"))?;
        let state = self.state.lock().map_err(poisoned)?;
        let layout = state
            .bind_group_layouts
            .get(&layout_id)
            .cloned()
            .ok_or_else(|| unknown("bind-group-layout", layout_id))?;
        let mut owned = Vec::new();
        let mut bindings = Vec::new();
        for value in checked_array_field(descriptor, "entries")? {
            let entry = value
                .as_object()
                .ok_or_else(|| webgpu_validation("bind group entry must be an object"))?;
            let binding = checked_u32_field(entry, "binding", 0)?;
            let resource = entry
                .get("resource")
                .ok_or_else(|| JsException::new("bind group entry resource is required"))?;
            let object = resource.as_object();
            let id = object
                .and_then(|v| v.get("buffer"))
                .and_then(resource_id_value)
                .or_else(|| resource_id_value(resource))
                .ok_or_else(|| JsException::new("invalid bind group resource"))?;
            let kind = object
                .and_then(|v| v.get("kind"))
                .and_then(HostValue::as_str)
                .unwrap_or_default();
            if object.is_some_and(|v| v.contains_key("buffer")) || state.buffers.contains_key(&id) {
                owned.push(Owned::Buffer(
                    state
                        .buffers
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| unknown("buffer", id))?,
                    object
                        .map(|v| checked_u64_field(v, "offset", 0))
                        .transpose()?
                        .unwrap_or(0),
                    object
                        .map(|v| {
                            v.get("size")
                                .map(|_| checked_u64_field(v, "size", 0))
                                .transpose()
                        })
                        .transpose()?
                        .flatten(),
                ));
            } else if kind == "sampler" || state.samplers.contains_key(&id) {
                owned.push(Owned::Sampler(
                    state
                        .samplers
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| unknown("sampler", id))?,
                ));
            } else {
                owned.push(Owned::View(
                    state
                        .views
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| unknown("texture-view", id))?,
                ));
            }
            bindings.push(binding);
        }
        ensure_unique_bindings(bindings.iter().copied(), "bind group")?;
        let entries = owned
            .iter()
            .zip(bindings)
            .map(|(resource, binding)| wgpu::BindGroupEntry {
                binding,
                resource: match resource {
                    Owned::Buffer(buffer, offset, size) => {
                        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer,
                            offset: *offset,
                            size: size.and_then(std::num::NonZeroU64::new),
                        })
                    }
                    Owned::Sampler(sampler) => wgpu::BindingResource::Sampler(sampler),
                    Owned::View(view) => wgpu::BindingResource::TextureView(view),
                },
            })
            .collect::<Vec<_>>();
        let bind_group = self.validation_scope(|device| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: label(descriptor),
                layout: &layout,
                entries: &entries,
            })
        })?;
        drop(state);
        let mut state = self.state.lock().map_err(poisoned)?;
        let id = state.alloc();
        state.bind_groups.insert(id, Arc::new(bind_group));
        Ok(resource_value(id, "bind-group", state.generation, []))
    }

    fn register_pipeline_ops(&self, api: &mut HostApiRegistry) {
        let runtime = self.clone();
        api.register("webgpuCreateShaderModule", move |args| {
            let d = object_arg(args, 0)?;
            let code = required_str_field(d, "code")?;
            let shader = runtime.validation_scope(|device| {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: label(d),
                    source: wgpu::ShaderSource::Wgsl(code.into()),
                })
            })?;
            let mut state = runtime.state.lock().map_err(poisoned)?;
            let id = state.alloc();
            state.shaders.insert(id, Arc::new(shader));
            Ok(resource_value(id, "shader-module", state.generation, []))
        });
        let runtime = self.clone();
        api.register("webgpuCreateRenderPipeline", move |args| {
            runtime.create_render_pipeline(object_arg(args, 0)?)
        });
        let runtime = self.clone();
        api.register("webgpuCreateComputePipeline", move |args| {
            runtime.create_compute_pipeline(object_arg(args, 0)?)
        });
    }

    fn create_render_pipeline(
        &self,
        d: &BTreeMap<String, HostValue>,
    ) -> Result<HostValue, JsException> {
        let state = self.state.lock().map_err(poisoned)?;
        let layout_id = match d.get("layout") {
            None | Some(HostValue::Undefined) => None,
            Some(HostValue::String(value)) if value == "auto" => None,
            Some(value) => Some(resource_id_value(value).ok_or_else(|| {
                webgpu_validation("pipeline layout must be `auto` or a GPU pipeline layout")
            })?),
        };
        let layout = layout_id
            .map(|id| {
                state
                    .pipeline_layouts
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| unknown("pipeline-layout", id))
            })
            .transpose()?;
        let vertex = required_object_field(d, "vertex")?;
        let vertex_shader_id = vertex
            .get("module")
            .and_then(resource_id_value)
            .ok_or_else(|| JsException::new("vertex shader module is required"))?;
        let vertex_shader = state
            .shaders
            .get(&vertex_shader_id)
            .cloned()
            .ok_or_else(|| unknown("shader-module", vertex_shader_id))?;
        let vertex_entry_point = optional_str_field(vertex, "entryPoint")?;
        let fragment_d = optional_object_field(d, "fragment")?;
        let fragment_shader_id = fragment_d
            .map(|fragment| {
                fragment
                    .get("module")
                    .and_then(resource_id_value)
                    .ok_or_else(|| webgpu_validation("fragment shader module is required"))
            })
            .transpose()?;
        let fragment_shader = fragment_shader_id
            .map(|id| {
                state
                    .shaders
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| unknown("shader-module", id))
            })
            .transpose()?;
        let fragment_entry_point = fragment_d
            .map(|v| optional_str_field(v, "entryPoint"))
            .transpose()?
            .flatten();
        let targets = fragment_d
            .map(|v| {
                checked_array_field(v, "targets")?
                    .iter()
                    .map(parse_color_target)
                    .collect::<Result<Vec<_>, JsException>>()
            })
            .transpose()?
            .unwrap_or_default();
        let vertex_buffers_desc = checked_array_field(vertex, "buffers")?;
        let vertex_attribute_sets = vertex_buffers_desc
            .iter()
            .map(|buffer| {
                if matches!(buffer, HostValue::Null | HostValue::Undefined) {
                    return Ok(Vec::new());
                }
                let buffer = buffer.as_object().ok_or_else(|| {
                    webgpu_validation("vertex buffer layout must be an object or null")
                })?;
                checked_array_field(buffer, "attributes")?
                    .iter()
                    .map(|attribute| {
                        let attribute = attribute.as_object().ok_or_else(|| {
                            JsException::new("vertex attribute must be an object")
                        })?;
                        Ok(wgpu::VertexAttribute {
                            format: parse_vertex_format(required_str_field(attribute, "format")?)?,
                            offset: checked_u64_field(attribute, "offset", 0)?,
                            shader_location: checked_u32_field(attribute, "shaderLocation", 0)?,
                        })
                    })
                    .collect::<Result<Vec<_>, JsException>>()
            })
            .collect::<Result<Vec<_>, JsException>>()?;
        ensure_unique_bindings(
            vertex_attribute_sets
                .iter()
                .flatten()
                .map(|attribute| attribute.shader_location),
            "vertex attributes",
        )?;
        let vertex_buffers = vertex_buffers_desc
            .iter()
            .enumerate()
            .map(|(index, buffer)| {
                if matches!(buffer, HostValue::Null | HostValue::Undefined) {
                    return Ok(None);
                }
                let buffer = buffer.as_object().ok_or_else(|| {
                    webgpu_validation("vertex buffer layout must be an object or null")
                })?;
                let array_stride = checked_u64_field(buffer, "arrayStride", 0)?;
                if array_stride % 4 != 0 {
                    return Err(webgpu_validation(
                        "vertex arrayStride must be a multiple of 4",
                    ));
                }
                let step_mode = match checked_str_field(buffer, "stepMode", "vertex")? {
                    "vertex" => wgpu::VertexStepMode::Vertex,
                    "instance" => wgpu::VertexStepMode::Instance,
                    value => {
                        return Err(webgpu_validation(format!(
                            "unsupported vertex stepMode `{value}`"
                        )));
                    }
                };
                for attribute in &vertex_attribute_sets[index] {
                    if attribute
                        .offset
                        .checked_add(attribute.format.size())
                        .is_none_or(|end| end > array_stride)
                    {
                        return Err(webgpu_validation(
                            "vertex attribute exceeds its buffer arrayStride",
                        ));
                    }
                }
                Ok(Some(wgpu::VertexBufferLayout {
                    array_stride,
                    step_mode,
                    attributes: &vertex_attribute_sets[index],
                }))
            })
            .collect::<Result<Vec<_>, JsException>>()?;
        let fragment = fragment_shader.as_ref().map(|shader| wgpu::FragmentState {
            module: shader,
            entry_point: fragment_entry_point,
            targets: &targets,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        });
        let primitive_d = optional_object_field(d, "primitive")?;
        let depth_stencil = optional_object_field(d, "depthStencil")?
            .map(parse_depth_stencil_state)
            .transpose()?;
        let multisample_d = optional_object_field(d, "multisample")?;
        let topology = parse_topology(
            primitive_d
                .map(|v| checked_str_field(v, "topology", "triangle-list"))
                .transpose()?
                .unwrap_or("triangle-list"),
        )?;
        let strip_index_format = primitive_d
            .and_then(|v| v.get("stripIndexFormat"))
            .map(|format| match format.as_str() {
                Some("uint16") => Ok(wgpu::IndexFormat::Uint16),
                Some("uint32") => Ok(wgpu::IndexFormat::Uint32),
                _ => Err(webgpu_validation(
                    "stripIndexFormat must be `uint16` or `uint32`",
                )),
            })
            .transpose()?;
        if strip_index_format.is_some()
            && !matches!(
                topology,
                wgpu::PrimitiveTopology::LineStrip | wgpu::PrimitiveTopology::TriangleStrip
            )
        {
            return Err(webgpu_validation(
                "stripIndexFormat is only valid for strip topologies",
            ));
        }
        let front_face = match primitive_d
            .map(|v| checked_str_field(v, "frontFace", "ccw"))
            .transpose()?
            .unwrap_or("ccw")
        {
            "ccw" => wgpu::FrontFace::Ccw,
            "cw" => wgpu::FrontFace::Cw,
            value => {
                return Err(webgpu_validation(format!(
                    "unsupported frontFace `{value}`"
                )));
            }
        };
        let cull_mode = primitive_d
            .map(|v| parse_cull(checked_str_field(v, "cullMode", "none")?))
            .transpose()?
            .flatten();
        let sample_count = multisample_d
            .map(|v| checked_u32_field(v, "count", 1))
            .transpose()?
            .unwrap_or(1);
        if !matches!(sample_count, 1 | 4) {
            return Err(webgpu_validation(
                "the NanaUI WebGPU subset supports multisample count 1 or 4",
            ));
        }
        let sample_mask = multisample_d
            .map(|v| checked_u64_field(v, "mask", u64::MAX))
            .transpose()?
            .unwrap_or(u64::MAX);
        let alpha_to_coverage_enabled = multisample_d
            .map(|v| checked_bool_field(v, "alphaToCoverageEnabled", false))
            .transpose()?
            .unwrap_or(false);
        if alpha_to_coverage_enabled && sample_count == 1 {
            return Err(webgpu_validation(
                "alphaToCoverageEnabled requires multisampling",
            ));
        }
        let unclipped_depth = primitive_d
            .map(|v| checked_bool_field(v, "unclippedDepth", false))
            .transpose()?
            .unwrap_or(false);
        let pipeline = self.validation_scope(|device| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: label(d),
                layout: layout.as_deref(),
                vertex: wgpu::VertexState {
                    module: &vertex_shader,
                    entry_point: vertex_entry_point,
                    buffers: &vertex_buffers,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment,
                primitive: wgpu::PrimitiveState {
                    topology,
                    strip_index_format,
                    front_face,
                    cull_mode,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth,
                    conservative: false,
                },
                depth_stencil,
                multisample: wgpu::MultisampleState {
                    count: sample_count,
                    mask: sample_mask,
                    alpha_to_coverage_enabled,
                },
                multiview_mask: None,
                cache: None,
            })
        })?;
        drop(state);
        let mut state = self.state.lock().map_err(poisoned)?;
        let id = state.alloc();
        state.render_pipelines.insert(id, Arc::new(pipeline));
        Ok(resource_value(id, "render-pipeline", state.generation, []))
    }

    fn create_compute_pipeline(
        &self,
        d: &BTreeMap<String, HostValue>,
    ) -> Result<HostValue, JsException> {
        let state = self.state.lock().map_err(poisoned)?;
        let layout_id = match d.get("layout") {
            None | Some(HostValue::Undefined) => None,
            Some(HostValue::String(value)) if value == "auto" => None,
            Some(value) => Some(resource_id_value(value).ok_or_else(|| {
                webgpu_validation("pipeline layout must be `auto` or a GPU pipeline layout")
            })?),
        };
        let layout = layout_id
            .map(|id| {
                state
                    .pipeline_layouts
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| unknown("pipeline-layout", id))
            })
            .transpose()?;
        let compute = required_object_field(d, "compute")?;
        let shader_id = compute
            .get("module")
            .and_then(resource_id_value)
            .ok_or_else(|| JsException::new("compute shader is required"))?;
        let shader = state
            .shaders
            .get(&shader_id)
            .cloned()
            .ok_or_else(|| unknown("shader-module", shader_id))?;
        let entry_point = optional_str_field(compute, "entryPoint")?;
        let pipeline = self.validation_scope(|device| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: label(d),
                layout: layout.as_deref(),
                module: &shader,
                entry_point,
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        })?;
        drop(state);
        let mut state = self.state.lock().map_err(poisoned)?;
        let id = state.alloc();
        state.compute_pipelines.insert(id, Arc::new(pipeline));
        Ok(resource_value(id, "compute-pipeline", state.generation, []))
    }

    fn register_command_ops(&self, api: &mut HostApiRegistry) {
        let runtime = self.clone();
        api.register("webgpuCreateCommandEncoder", move |_| {
            let mut state = runtime.state.lock().map_err(poisoned)?;
            let id = state.alloc();
            state.encoders.insert(id, Vec::new());
            Ok(resource_value(id, "command-encoder", state.generation, []))
        });
        let runtime = self.clone();
        api.register("webgpuBeginPass", move |args| {
            let encoder = gpu_id(args, 0)?;
            let kind = args
                .get(1)
                .and_then(HostValue::as_str)
                .ok_or_else(|| webgpu_validation("pass kind must be `render` or `compute`"))?;
            if !matches!(kind, "render" | "compute") {
                return Err(webgpu_validation("pass kind must be `render` or `compute`"));
            }
            let descriptor = match args.get(2) {
                None | Some(HostValue::Undefined) if kind == "compute" => None,
                Some(value) => Some(
                    value
                        .as_object()
                        .ok_or_else(|| webgpu_validation("pass descriptor must be an object"))?,
                ),
                None => None,
            };
            let mut state = runtime.state.lock().map_err(poisoned)?;
            if !state.encoders.contains_key(&encoder) {
                return Err(unknown("command-encoder", encoder));
            }
            let render_descriptor = if kind == "render" {
                Some(
                    descriptor
                        .ok_or_else(|| JsException::new("render pass descriptor is required"))?,
                )
            } else {
                None
            };
            let colors = render_descriptor.map(parse_color_attachments).transpose()?;
            let depth_stencil = render_descriptor
                .map(|descriptor| optional_object_field(descriptor, "depthStencilAttachment"))
                .transpose()?
                .flatten()
                .map(parse_depth_stencil_attachment)
                .transpose()?;
            if let Some(colors) = &colors {
                validate_render_attachments(&state, colors, depth_stencil.as_ref())?;
            }
            let id = state.alloc();
            state.passes.insert(
                id,
                OpenPass {
                    encoder,
                    render_colors: colors,
                    render_depth_stencil: depth_stencil,
                    commands: Vec::new(),
                },
            );
            Ok(resource_value(
                id,
                if kind == "render" {
                    "render-pass"
                } else {
                    "compute-pass"
                },
                state.generation,
                [],
            ))
        });
        let runtime = self.clone();
        api.register("webgpuPassCommand", move |args| {
            let pass_id = gpu_id(args, 0)?;
            let name = args
                .get(1)
                .and_then(HostValue::as_str)
                .ok_or_else(|| webgpu_validation("pass command name must be a string"))?;
            let values = args
                .get(2)
                .ok_or_else(|| webgpu_validation("pass command arguments must be an array"))?
                .as_array()
                .ok_or_else(|| webgpu_validation("pass command arguments must be an array"))?;
            let command = parse_pass_command(name, values)?;
            let mut state = runtime.state.lock().map_err(poisoned)?;
            validate_pass_command(&state, pass_id, &command)?;
            state
                .passes
                .get_mut(&pass_id)
                .ok_or_else(|| unknown("pass", pass_id))?
                .commands
                .push(command);
            Ok(HostValue::Null)
        });
        let runtime = self.clone();
        api.register("webgpuEndPass", move |args| {
            let pass_id = gpu_id(args, 0)?;
            let mut state = runtime.state.lock().map_err(poisoned)?;
            let pass = state
                .passes
                .remove(&pass_id)
                .ok_or_else(|| unknown("pass", pass_id))?;
            let command = match pass.render_colors {
                Some(colors) => EncoderCommand::Render {
                    colors,
                    depth_stencil: pass.render_depth_stencil,
                    commands: pass.commands,
                },
                None => EncoderCommand::Compute {
                    commands: pass.commands,
                },
            };
            state
                .encoders
                .get_mut(&pass.encoder)
                .ok_or_else(|| unknown("command-encoder", pass.encoder))?
                .push(command);
            Ok(HostValue::Null)
        });
        let runtime = self.clone();
        api.register("webgpuEncoderCopyBuffer", move |args| {
            let encoder = gpu_id(args, 0)?;
            let source = gpu_id(args, 1)?;
            let source_offset = checked_u64_arg(args, 2, Some(0))?;
            let destination = gpu_id(args, 3)?;
            let destination_offset = checked_u64_arg(args, 4, Some(0))?;
            let size = checked_u64_arg(args, 5, None)?;
            let mut state = runtime.state.lock().map_err(poisoned)?;
            validate_buffer_copy(
                &state,
                source,
                source_offset,
                destination,
                destination_offset,
                size,
            )?;
            state
                .encoders
                .get_mut(&encoder)
                .ok_or_else(|| unknown("command-encoder", encoder))?
                .push(EncoderCommand::CopyBuffer {
                    source,
                    source_offset,
                    destination,
                    destination_offset,
                    size,
                });
            Ok(HostValue::Null)
        });
        let runtime = self.clone();
        api.register("webgpuFinishEncoder", move |args| {
            let encoder = gpu_id(args, 0)?;
            let mut state = runtime.state.lock().map_err(poisoned)?;
            let commands = state
                .encoders
                .remove(&encoder)
                .ok_or_else(|| unknown("command-encoder", encoder))?;
            let id = state.alloc();
            state.command_buffers.insert(id, commands);
            Ok(resource_value(id, "command-buffer", state.generation, []))
        });
        let runtime = self.clone();
        api.register("webgpuQueueSubmit", move |args| {
            let command_buffers = args
                .first()
                .and_then(HostValue::as_array)
                .ok_or_else(|| webgpu_validation("queue.submit requires an array"))?;
            let ids = command_buffers
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    resource_id_value(value).ok_or_else(|| {
                        webgpu_validation(format!(
                            "queue.submit command buffer {index} is not a GPU resource"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            runtime.submit(ids)?;
            Ok(HostValue::Bool(true))
        });
        let runtime = self.clone();
        api.register("webgpuQueueWriteTexture", move |args| {
            runtime.queue_write_texture(args)
        });
    }

    fn submit(&self, ids: Vec<GpuId>) -> Result<(), JsException> {
        self.validation_scope(|_| self.submit_unchecked(ids))??;
        Ok(())
    }

    fn submit_unchecked(&self, ids: Vec<GpuId>) -> Result<(), JsException> {
        let mut state = self.state.lock().map_err(poisoned)?;
        let command_lists = ids
            .into_iter()
            .map(|id| {
                state
                    .command_buffers
                    .remove(&id)
                    .ok_or_else(|| unknown("command-buffer", id))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let resources = self.resources.lock().map_err(poisoned)?;
        let mut submitted = Vec::new();
        for commands in command_lists {
            let mut encoder =
                resources
                    .device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Nana JS WebGPU encoder"),
                    });
            for command in commands {
                match command {
                    EncoderCommand::CopyBuffer {
                        source,
                        source_offset,
                        destination,
                        destination_offset,
                        size,
                    } => encoder.copy_buffer_to_buffer(
                        state
                            .buffers
                            .get(&source)
                            .ok_or_else(|| unknown("buffer", source))?,
                        source_offset,
                        state
                            .buffers
                            .get(&destination)
                            .ok_or_else(|| unknown("buffer", destination))?,
                        destination_offset,
                        size,
                    ),
                    EncoderCommand::Render {
                        colors,
                        depth_stencil,
                        commands,
                    } => {
                        let attachments = colors
                            .iter()
                            .map(|color| {
                                let view = state
                                    .views
                                    .get(&color.view)
                                    .ok_or_else(|| unknown("texture-view", color.view))?;
                                let resolve_target = color
                                    .resolve_target
                                    .map(|id| {
                                        state
                                            .views
                                            .get(&id)
                                            .map(AsRef::as_ref)
                                            .ok_or_else(|| unknown("texture-view", id))
                                    })
                                    .transpose()?;
                                Ok(Some(wgpu::RenderPassColorAttachment {
                                    view,
                                    depth_slice: None,
                                    resolve_target,
                                    ops: wgpu::Operations {
                                        load: color
                                            .clear
                                            .map(wgpu::LoadOp::Clear)
                                            .unwrap_or(wgpu::LoadOp::Load),
                                        store: if color.store {
                                            wgpu::StoreOp::Store
                                        } else {
                                            wgpu::StoreOp::Discard
                                        },
                                    },
                                }))
                            })
                            .collect::<Result<Vec<_>, JsException>>()?;
                        let depth_stencil_attachment = depth_stencil
                            .as_ref()
                            .map(|attachment| {
                                let view = state
                                    .views
                                    .get(&attachment.view)
                                    .ok_or_else(|| unknown("texture-view", attachment.view))?;
                                Ok(wgpu::RenderPassDepthStencilAttachment {
                                    view,
                                    depth_ops: (!attachment.depth_read_only).then_some(
                                        wgpu::Operations {
                                            load: attachment
                                                .depth_clear
                                                .map(wgpu::LoadOp::Clear)
                                                .unwrap_or(wgpu::LoadOp::Load),
                                            store: if attachment.depth_store {
                                                wgpu::StoreOp::Store
                                            } else {
                                                wgpu::StoreOp::Discard
                                            },
                                        },
                                    ),
                                    stencil_ops: (!attachment.stencil_read_only).then_some(
                                        wgpu::Operations {
                                            load: attachment
                                                .stencil_clear
                                                .map(wgpu::LoadOp::Clear)
                                                .unwrap_or(wgpu::LoadOp::Load),
                                            store: if attachment.stencil_store {
                                                wgpu::StoreOp::Store
                                            } else {
                                                wgpu::StoreOp::Discard
                                            },
                                        },
                                    ),
                                })
                            })
                            .transpose()?;
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("Nana JS render pass"),
                            color_attachments: &attachments,
                            depth_stencil_attachment,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        for command in commands {
                            apply_render_command(&state, &mut pass, command)?;
                        }
                    }
                    EncoderCommand::Compute { commands } => {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("Nana JS compute pass"),
                            timestamp_writes: None,
                        });
                        for command in commands {
                            apply_compute_command(&state, &mut pass, command)?;
                        }
                    }
                }
            }
            submitted.push(encoder.finish());
        }
        resources.queue().submit(submitted);
        // Submission completion is advanced by the hosted event-loop pump.
        // Never serialize every UI frame on a blocking device poll here.
        let _ = resources.device().poll(wgpu::PollType::Poll);
        for texture in state.textures.values() {
            if let Some(slot) = &texture.canvas_slot {
                self.textures.invalidate(slot);
            }
        }
        Ok(())
    }

    fn queue_write_texture(&self, args: &[HostValue]) -> Result<HostValue, JsException> {
        let destination = object_arg(args, 0)?;
        let texture_id = destination
            .get("texture")
            .and_then(resource_id_value)
            .ok_or_else(|| JsException::new("writeTexture destination texture is required"))?;
        let bytes = args
            .get(1)
            .and_then(HostValue::as_bytes)
            .ok_or_else(|| JsException::new("writeTexture data must be an ArrayBuffer"))?;
        let layout = object_arg(args, 2)?;
        let size = args.get(3);
        let state = self.state.lock().map_err(poisoned)?;
        let texture = state
            .textures
            .get(&texture_id)
            .ok_or_else(|| unknown("texture", texture_id))?;
        let (width, height, depth) = checked_size3(size, (texture.width, texture.height, 1))?;
        let mip_level = checked_u32_field(destination, "mipLevel", 0)?;
        let origin = parse_origin(destination.get("origin"))?;
        let aspect =
            parse_texture_aspect_checked(checked_str_field(destination, "aspect", "all")?)?;
        let offset = checked_u64_field(layout, "offset", 0)?;
        let bytes_per_row = optional_u32_field(layout, "bytesPerRow")?;
        let rows_per_image = optional_u32_field(layout, "rowsPerImage")?;
        validate_texture_write(
            texture,
            mip_level,
            origin,
            aspect,
            width,
            height,
            depth,
            offset,
            bytes_per_row,
            rows_per_image,
            bytes.len(),
        )?;
        let queue = self.queue()?;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level,
                origin,
                aspect,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset,
                bytes_per_row,
                rows_per_image,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: depth,
            },
        );
        if let Some(slot) = &texture.canvas_slot {
            self.textures.invalidate(slot);
        }
        Ok(HostValue::Bool(true))
    }

    fn release(&self, id: GpuId) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let mut removed = false;
        removed |= state.buffers.remove(&id).is_some();
        state.buffer_sizes.remove(&id);
        state.buffer_usages.remove(&id);
        state.initially_mapped_buffers.remove(&id);
        removed |= state.views.remove(&id).is_some();
        state.view_textures.remove(&id);
        state.view_extents.remove(&id);
        removed |= state.samplers.remove(&id).is_some();
        removed |= state.shaders.remove(&id).is_some();
        removed |= state.bind_groups.remove(&id).is_some();
        removed |= state.bind_group_layouts.remove(&id).is_some();
        removed |= state.pipeline_layouts.remove(&id).is_some();
        removed |= state.render_pipelines.remove(&id).is_some();
        removed |= state.compute_pipelines.remove(&id).is_some();
        removed |= state.encoders.remove(&id).is_some();
        removed |= state.command_buffers.remove(&id).is_some();
        removed |= state.passes.remove(&id).is_some();
        if let Some(texture) = state.textures.remove(&id) {
            texture.texture.destroy();
            remove_texture_views(&mut state, id);
            if let Some(slot) = texture.canvas_slot {
                self.textures.remove(&slot);
            }
            state
                .canvas_textures
                .retain(|_, texture_id| *texture_id != id);
            removed = true;
        }
        removed
    }

    fn validation_scope<T>(
        &self,
        operation: impl FnOnce(&wgpu::Device) -> T,
    ) -> Result<T, JsException> {
        let device = self.device()?;
        let has_user_scope = ERROR_SCOPES.with(|scopes| {
            scopes
                .borrow()
                .get(&self.runtime_id)
                .is_some_and(|scopes| !scopes.stack.is_empty())
        });
        if has_user_scope {
            return Ok(operation(&device));
        }
        let guard = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let output = operation(&device);
        let mut error = Box::pin(guard.pop());
        let waker = std::task::Waker::noop();
        let mut context = Context::from_waker(waker);
        for _ in 0..64 {
            let _ = device.poll(wgpu::PollType::Poll);
            match error.as_mut().poll(&mut context) {
                Poll::Ready(Some(error)) => return Err(wgpu_error_exception(error)),
                Poll::Ready(None) => return Ok(output),
                Poll::Pending => std::thread::yield_now(),
            }
        }
        Err(webgpu_validation(
            "WGPU validation did not settle during synchronous resource creation",
        ))
    }

    fn device(&self) -> Result<Arc<wgpu::Device>, JsException> {
        self.resources
            .lock()
            .map_err(poisoned)
            .map(|r| Arc::clone(r.device()))
    }
    fn queue(&self) -> Result<Arc<wgpu::Queue>, JsException> {
        self.resources
            .lock()
            .map_err(poisoned)
            .map(|r| Arc::clone(r.queue()))
    }
}

fn apply_render_command<'a>(
    state: &'a WebGpuState,
    pass: &mut wgpu::RenderPass<'a>,
    command: PassCommand,
) -> Result<(), JsException> {
    match command {
        PassCommand::SetPipeline(id) => pass.set_pipeline(
            state
                .render_pipelines
                .get(&id)
                .ok_or_else(|| unknown("render-pipeline", id))?,
        ),
        PassCommand::SetBindGroup(index, id, offsets) => pass.set_bind_group(
            index,
            state
                .bind_groups
                .get(&id)
                .ok_or_else(|| unknown("bind-group", id))?
                .as_ref(),
            &offsets,
        ),
        PassCommand::SetVertexBuffer(slot, id, offset, size) => {
            let buffer = state
                .buffers
                .get(&id)
                .ok_or_else(|| unknown("buffer", id))?;
            match size {
                Some(size) => pass.set_vertex_buffer(slot, buffer.slice(offset..offset + size)),
                None => pass.set_vertex_buffer(slot, buffer.slice(offset..)),
            }
        }
        PassCommand::SetIndexBuffer(id, format, offset, size) => {
            let buffer = state
                .buffers
                .get(&id)
                .ok_or_else(|| unknown("buffer", id))?;
            match size {
                Some(size) => pass.set_index_buffer(buffer.slice(offset..offset + size), format),
                None => pass.set_index_buffer(buffer.slice(offset..), format),
            }
        }
        PassCommand::SetViewport(x, y, width, height, min_depth, max_depth) => {
            pass.set_viewport(x, y, width, height, min_depth, max_depth)
        }
        PassCommand::SetScissorRect(x, y, width, height) => {
            pass.set_scissor_rect(x, y, width, height)
        }
        PassCommand::SetBlendConstant(color) => pass.set_blend_constant(color),
        PassCommand::SetStencilReference(reference) => pass.set_stencil_reference(reference),
        PassCommand::Draw(vertices, instances, first_vertex, first_instance) => pass.draw(
            first_vertex..first_vertex + vertices,
            first_instance..first_instance + instances,
        ),
        PassCommand::DrawIndexed(indices, instances, first_index, base_vertex, first_instance) => {
            pass.draw_indexed(
                first_index..first_index + indices,
                base_vertex,
                first_instance..first_instance + instances,
            )
        }
        PassCommand::Dispatch(..) => {
            return Err(JsException::new(
                "dispatchWorkgroups is only valid in a compute pass",
            ));
        }
    }
    Ok(())
}

fn apply_compute_command<'a>(
    state: &'a WebGpuState,
    pass: &mut wgpu::ComputePass<'a>,
    command: PassCommand,
) -> Result<(), JsException> {
    match command {
        PassCommand::SetPipeline(id) => pass.set_pipeline(
            state
                .compute_pipelines
                .get(&id)
                .ok_or_else(|| unknown("compute-pipeline", id))?,
        ),
        PassCommand::SetBindGroup(index, id, offsets) => pass.set_bind_group(
            index,
            state
                .bind_groups
                .get(&id)
                .ok_or_else(|| unknown("bind-group", id))?
                .as_ref(),
            &offsets,
        ),
        PassCommand::Dispatch(x, y, z) => pass.dispatch_workgroups(x, y, z),
        _ => {
            return Err(JsException::new(
                "render command is not valid in a compute pass",
            ));
        }
    }
    Ok(())
}

fn parse_pass_command(name: &str, args: &[HostValue]) -> Result<PassCommand, JsException> {
    Ok(match name {
        "setPipeline" => PassCommand::SetPipeline(gpu_id(args, 0)?),
        "setBindGroup" => PassCommand::SetBindGroup(
            checked_u32_arg(args, 0, None)?,
            gpu_id(args, 1)?,
            match args.get(2) {
                None | Some(HostValue::Undefined) => Vec::new(),
                Some(value) => value
                    .as_array()
                    .ok_or_else(|| webgpu_validation("dynamic offsets must be an array"))?
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let value = value.as_u64().ok_or_else(|| {
                            webgpu_validation(format!(
                                "dynamic offset {index} must be an unsigned integer"
                            ))
                        })?;
                        u32::try_from(value).map_err(|_| {
                            webgpu_validation(format!(
                                "dynamic offset {index} exceeds the u32 range"
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
        ),
        "setVertexBuffer" => PassCommand::SetVertexBuffer(
            checked_u32_arg(args, 0, None)?,
            gpu_id(args, 1)?,
            checked_u64_arg(args, 2, Some(0))?,
            match args.get(3) {
                None | Some(HostValue::Undefined) => None,
                Some(value) => Some(value.as_u64().ok_or_else(|| {
                    webgpu_validation("vertex buffer size must be an unsigned integer")
                })?),
            },
        ),
        "setIndexBuffer" => PassCommand::SetIndexBuffer(
            gpu_id(args, 0)?,
            match args.get(1).and_then(HostValue::as_str) {
                Some("uint16") => wgpu::IndexFormat::Uint16,
                Some("uint32") => wgpu::IndexFormat::Uint32,
                _ => {
                    return Err(webgpu_validation(
                        "index format must be `uint16` or `uint32`",
                    ));
                }
            },
            checked_u64_arg(args, 2, Some(0))?,
            match args.get(3) {
                None | Some(HostValue::Undefined) => None,
                Some(value) => Some(value.as_u64().ok_or_else(|| {
                    webgpu_validation("index buffer size must be an unsigned integer")
                })?),
            },
        ),
        "setViewport" => PassCommand::SetViewport(
            checked_f32_arg(args, 0, None)?,
            checked_f32_arg(args, 1, None)?,
            checked_f32_arg(args, 2, None)?,
            checked_f32_arg(args, 3, None)?,
            checked_f32_arg(args, 4, None)?,
            checked_f32_arg(args, 5, None)?,
        ),
        "setScissorRect" => PassCommand::SetScissorRect(
            checked_u32_arg(args, 0, None)?,
            checked_u32_arg(args, 1, None)?,
            checked_u32_arg(args, 2, None)?,
            checked_u32_arg(args, 3, None)?,
        ),
        "setBlendConstant" => PassCommand::SetBlendConstant(parse_color_value(
            args.first()
                .ok_or_else(|| webgpu_validation("blend constant is required"))?,
        )?),
        "setStencilReference" => PassCommand::SetStencilReference(checked_u32_arg(args, 0, None)?),
        "draw" => PassCommand::Draw(
            checked_u32_arg(args, 0, None)?,
            checked_u32_arg(args, 1, Some(1))?,
            checked_u32_arg(args, 2, Some(0))?,
            checked_u32_arg(args, 3, Some(0))?,
        ),
        "drawIndexed" => PassCommand::DrawIndexed(
            checked_u32_arg(args, 0, None)?,
            checked_u32_arg(args, 1, Some(1))?,
            checked_u32_arg(args, 2, Some(0))?,
            checked_i32_arg(args, 3, 0)?,
            checked_u32_arg(args, 4, Some(0))?,
        ),
        "dispatchWorkgroups" => PassCommand::Dispatch(
            checked_u32_arg(args, 0, None)?,
            checked_u32_arg(args, 1, Some(1))?,
            checked_u32_arg(args, 2, Some(1))?,
        ),
        _ => {
            return Err(JsException::new(format!(
                "unsupported WebGPU pass command `{name}`"
            )));
        }
    })
}

fn parse_color_attachments(
    d: &BTreeMap<String, HostValue>,
) -> Result<Vec<ColorAttachment>, JsException> {
    checked_array_field(d, "colorAttachments")?
        .iter()
        .map(|value| {
            let entry = value
                .as_object()
                .ok_or_else(|| webgpu_validation("color attachment entries must be objects"))?;
            let view = entry
                .get("view")
                .and_then(resource_id_value)
                .ok_or_else(|| JsException::new("color attachment view is required"))?;
            let clear = entry.get("clearValue").map(parse_color_value).transpose()?;
            let clear = match checked_str_field(entry, "loadOp", "load")? {
                "load" => None,
                "clear" => Some(clear.unwrap_or(wgpu::Color::TRANSPARENT)),
                value => {
                    return Err(webgpu_validation(format!(
                        "unsupported color loadOp `{value}`"
                    )));
                }
            };
            let store = match checked_str_field(entry, "storeOp", "store")? {
                "store" => true,
                "discard" => false,
                value => {
                    return Err(webgpu_validation(format!(
                        "unsupported color storeOp `{value}`"
                    )));
                }
            };
            Ok(ColorAttachment {
                view,
                resolve_target: entry.get("resolveTarget").and_then(resource_id_value),
                clear,
                store,
            })
        })
        .collect()
}

fn parse_depth_stencil_attachment(
    entry: &BTreeMap<String, HostValue>,
) -> Result<DepthStencilAttachment, JsException> {
    let view = entry
        .get("view")
        .and_then(resource_id_value)
        .ok_or_else(|| JsException::new("depth stencil attachment view is required"))?;
    let depth_enabled = entry.contains_key("depthLoadOp")
        || entry.contains_key("depthStoreOp")
        || entry.contains_key("depthClearValue");
    let stencil_enabled = entry.contains_key("stencilLoadOp")
        || entry.contains_key("stencilStoreOp")
        || entry.contains_key("stencilClearValue");
    let depth_clear = match checked_str_field(entry, "depthLoadOp", "load")? {
        "load" => None,
        "clear" => Some(checked_f32_field(entry, "depthClearValue", 1.0)?),
        value => {
            return Err(webgpu_validation(format!(
                "unsupported depth loadOp `{value}`"
            )));
        }
    };
    if depth_clear.is_some_and(|value| !(0.0..=1.0).contains(&value)) {
        return Err(webgpu_validation("depthClearValue must be between 0 and 1"));
    }
    let depth_store = match checked_str_field(entry, "depthStoreOp", "store")? {
        "store" => true,
        "discard" => false,
        value => {
            return Err(webgpu_validation(format!(
                "unsupported depth storeOp `{value}`"
            )));
        }
    };
    let stencil_clear = match checked_str_field(entry, "stencilLoadOp", "load")? {
        "load" => None,
        "clear" => Some(checked_u32_field(entry, "stencilClearValue", 0)?),
        value => {
            return Err(webgpu_validation(format!(
                "unsupported stencil loadOp `{value}`"
            )));
        }
    };
    let stencil_store = match checked_str_field(entry, "stencilStoreOp", "store")? {
        "store" => true,
        "discard" => false,
        value => {
            return Err(webgpu_validation(format!(
                "unsupported stencil storeOp `{value}`"
            )));
        }
    };
    Ok(DepthStencilAttachment {
        view,
        depth_clear,
        depth_store,
        depth_read_only: !depth_enabled || checked_bool_field(entry, "depthReadOnly", false)?,
        stencil_clear,
        stencil_store,
        stencil_read_only: !stencil_enabled || checked_bool_field(entry, "stencilReadOnly", false)?,
    })
}

fn parse_color_target(value: &HostValue) -> Result<Option<wgpu::ColorTargetState>, JsException> {
    if matches!(value, HostValue::Null | HostValue::Undefined) {
        return Ok(None);
    }
    let Some(target) = value.as_object() else {
        return Err(webgpu_validation(
            "fragment target must be an object or null",
        ));
    };
    let blend = optional_object_field(target, "blend")?
        .map(|blend| {
            Ok(wgpu::BlendState {
                color: optional_object_field(blend, "color")?
                    .map(parse_blend_component)
                    .transpose()?
                    .unwrap_or(wgpu::BlendComponent::REPLACE),
                alpha: optional_object_field(blend, "alpha")?
                    .map(parse_blend_component)
                    .transpose()?
                    .unwrap_or(wgpu::BlendComponent::REPLACE),
            })
        })
        .transpose()?;
    let format = parse_texture_format_checked(required_str_field(target, "format")?)?;
    if format.is_depth_stencil_format() {
        return Err(webgpu_validation(
            "fragment color target cannot use a depth/stencil format",
        ));
    }
    let write_mask = wgpu::ColorWrites::from_bits(checked_u32_field(
        target,
        "writeMask",
        wgpu::ColorWrites::ALL.bits(),
    )?)
    .ok_or_else(|| webgpu_validation("fragment writeMask contains unsupported bits"))?;
    Ok(Some(wgpu::ColorTargetState {
        format,
        blend,
        write_mask,
    }))
}

fn parse_blend_component(
    d: &BTreeMap<String, HostValue>,
) -> Result<wgpu::BlendComponent, JsException> {
    Ok(wgpu::BlendComponent {
        src_factor: parse_blend_factor(checked_str_field(d, "srcFactor", "one")?)?,
        dst_factor: parse_blend_factor(checked_str_field(d, "dstFactor", "zero")?)?,
        operation: match checked_str_field(d, "operation", "add")? {
            "add" => wgpu::BlendOperation::Add,
            "subtract" => wgpu::BlendOperation::Subtract,
            "reverse-subtract" => wgpu::BlendOperation::ReverseSubtract,
            "min" => wgpu::BlendOperation::Min,
            "max" => wgpu::BlendOperation::Max,
            value => {
                return Err(webgpu_validation(format!(
                    "unsupported blend operation `{value}`"
                )));
            }
        },
    })
}

fn parse_blend_factor(value: &str) -> Result<wgpu::BlendFactor, JsException> {
    Ok(match value {
        "one" => wgpu::BlendFactor::One,
        "zero" => wgpu::BlendFactor::Zero,
        "src" => wgpu::BlendFactor::Src,
        "one-minus-src" => wgpu::BlendFactor::OneMinusSrc,
        "src-alpha" => wgpu::BlendFactor::SrcAlpha,
        "one-minus-src-alpha" => wgpu::BlendFactor::OneMinusSrcAlpha,
        "dst" => wgpu::BlendFactor::Dst,
        "one-minus-dst" => wgpu::BlendFactor::OneMinusDst,
        "dst-alpha" => wgpu::BlendFactor::DstAlpha,
        "one-minus-dst-alpha" => wgpu::BlendFactor::OneMinusDstAlpha,
        "src-alpha-saturated" => wgpu::BlendFactor::SrcAlphaSaturated,
        "constant" => wgpu::BlendFactor::Constant,
        "one-minus-constant" => wgpu::BlendFactor::OneMinusConstant,
        _ => {
            return Err(webgpu_validation(format!(
                "unsupported blend factor `{value}`"
            )));
        }
    })
}

fn parse_depth_stencil_state(
    d: &BTreeMap<String, HostValue>,
) -> Result<wgpu::DepthStencilState, JsException> {
    let format = parse_texture_format_checked(required_str_field(d, "format")?)?;
    if !format.is_depth_stencil_format() {
        return Err(webgpu_validation(
            "depthStencil format must be a depth/stencil format",
        ));
    }
    Ok(wgpu::DepthStencilState {
        format,
        depth_write_enabled: Some(checked_bool_field(d, "depthWriteEnabled", false)?),
        depth_compare: Some(parse_compare(checked_str_field(
            d,
            "depthCompare",
            "always",
        )?)?),
        stencil: wgpu::StencilState {
            front: optional_object_field(d, "stencilFront")?
                .map(parse_stencil_face)
                .transpose()?
                .unwrap_or(wgpu::StencilFaceState::IGNORE),
            back: optional_object_field(d, "stencilBack")?
                .map(parse_stencil_face)
                .transpose()?
                .unwrap_or(wgpu::StencilFaceState::IGNORE),
            read_mask: checked_u32_field(d, "stencilReadMask", u32::MAX)?,
            write_mask: checked_u32_field(d, "stencilWriteMask", u32::MAX)?,
        },
        bias: wgpu::DepthBiasState {
            constant: checked_i32_field_value(d, "depthBias", 0)?,
            slope_scale: checked_f32_field(d, "depthBiasSlopeScale", 0.0)?,
            clamp: checked_f32_field(d, "depthBiasClamp", 0.0)?,
        },
    })
}

fn parse_stencil_face(
    d: &BTreeMap<String, HostValue>,
) -> Result<wgpu::StencilFaceState, JsException> {
    Ok(wgpu::StencilFaceState {
        compare: parse_compare(checked_str_field(d, "compare", "always")?)?,
        fail_op: parse_stencil_operation(checked_str_field(d, "failOp", "keep")?)?,
        depth_fail_op: parse_stencil_operation(checked_str_field(d, "depthFailOp", "keep")?)?,
        pass_op: parse_stencil_operation(checked_str_field(d, "passOp", "keep")?)?,
    })
}

fn parse_stencil_operation(value: &str) -> Result<wgpu::StencilOperation, JsException> {
    Ok(match value {
        "keep" => wgpu::StencilOperation::Keep,
        "zero" => wgpu::StencilOperation::Zero,
        "replace" => wgpu::StencilOperation::Replace,
        "invert" => wgpu::StencilOperation::Invert,
        "increment-clamp" => wgpu::StencilOperation::IncrementClamp,
        "decrement-clamp" => wgpu::StencilOperation::DecrementClamp,
        "increment-wrap" => wgpu::StencilOperation::IncrementWrap,
        "decrement-wrap" => wgpu::StencilOperation::DecrementWrap,
        _ => {
            return Err(webgpu_validation(format!(
                "unsupported stencil operation `{value}`"
            )));
        }
    })
}

fn parse_layout_entry(
    entry: &BTreeMap<String, HostValue>,
) -> Result<wgpu::BindGroupLayoutEntry, JsException> {
    if entry.contains_key("count") {
        return Err(webgpu_validation(
            "binding arrays are not supported by the NanaUI WebGPU subset",
        ));
    }
    if entry.contains_key("externalTexture") {
        return Err(webgpu_validation(
            "externalTexture bindings are not supported by the NanaUI WebGPU subset",
        ));
    }
    let kinds = ["buffer", "sampler", "storageTexture", "texture"]
        .into_iter()
        .filter(|key| entry.contains_key(*key))
        .collect::<Vec<_>>();
    if kinds.len() != 1 {
        return Err(webgpu_validation(
            "bind group layout entry must declare exactly one resource type",
        ));
    }
    let kind = kinds[0];
    let descriptor = entry
        .get(kind)
        .and_then(HostValue::as_object)
        .ok_or_else(|| webgpu_validation(format!("`{kind}` binding must be an object")))?;
    let ty = if kind == "buffer" {
        let buffer = descriptor;
        wgpu::BindingType::Buffer {
            ty: match checked_str_field(buffer, "type", "uniform")? {
                "uniform" => wgpu::BufferBindingType::Uniform,
                "storage" => wgpu::BufferBindingType::Storage { read_only: false },
                "read-only-storage" => wgpu::BufferBindingType::Storage { read_only: true },
                value => {
                    return Err(webgpu_validation(format!(
                        "unsupported buffer binding type `{value}`"
                    )));
                }
            },
            has_dynamic_offset: checked_bool_field(buffer, "hasDynamicOffset", false)?,
            min_binding_size: match buffer.get("minBindingSize") {
                None | Some(HostValue::Undefined) => None,
                Some(value) => Some(
                    std::num::NonZeroU64::new(value.as_u64().ok_or_else(|| {
                        webgpu_validation("minBindingSize must be an unsigned integer")
                    })?)
                    .ok_or_else(|| webgpu_validation("minBindingSize must be non-zero"))?,
                ),
            },
        }
    } else if kind == "sampler" {
        let sampler = descriptor;
        wgpu::BindingType::Sampler(match checked_str_field(sampler, "type", "filtering")? {
            "filtering" => wgpu::SamplerBindingType::Filtering,
            "comparison" => wgpu::SamplerBindingType::Comparison,
            "non-filtering" => wgpu::SamplerBindingType::NonFiltering,
            value => {
                return Err(webgpu_validation(format!(
                    "unsupported sampler binding type `{value}`"
                )));
            }
        })
    } else if kind == "storageTexture" {
        let storage = descriptor;
        wgpu::BindingType::StorageTexture {
            access: match checked_str_field(storage, "access", "write-only")? {
                "write-only" => wgpu::StorageTextureAccess::WriteOnly,
                "read-only" => wgpu::StorageTextureAccess::ReadOnly,
                "read-write" => wgpu::StorageTextureAccess::ReadWrite,
                value => {
                    return Err(webgpu_validation(format!(
                        "unsupported storage texture access `{value}`"
                    )));
                }
            },
            format: parse_texture_format_checked(required_str_field(storage, "format")?)?,
            view_dimension: parse_view_dimension_checked(checked_str_field(
                storage,
                "viewDimension",
                "2d",
            )?)?,
        }
    } else if kind == "texture" {
        let texture = descriptor;
        wgpu::BindingType::Texture {
            sample_type: match checked_str_field(texture, "sampleType", "float")? {
                "float" => wgpu::TextureSampleType::Float { filterable: true },
                "depth" => wgpu::TextureSampleType::Depth,
                "sint" => wgpu::TextureSampleType::Sint,
                "uint" => wgpu::TextureSampleType::Uint,
                "unfilterable-float" => wgpu::TextureSampleType::Float { filterable: false },
                value => {
                    return Err(webgpu_validation(format!(
                        "unsupported texture sample type `{value}`"
                    )));
                }
            },
            view_dimension: parse_view_dimension_checked(checked_str_field(
                texture,
                "viewDimension",
                "2d",
            )?)?,
            multisampled: checked_bool_field(texture, "multisampled", false)?,
        }
    } else {
        return Err(webgpu_validation("missing bind group resource type"));
    };
    let visibility_bits = checked_u32_field(entry, "visibility", 0)?;
    let visibility = wgpu::ShaderStages::from_bits(visibility_bits)
        .ok_or_else(|| webgpu_validation("visibility contains unsupported shader-stage bits"))?;
    if visibility.is_empty() {
        return Err(webgpu_validation("visibility must not be empty"));
    }
    Ok(wgpu::BindGroupLayoutEntry {
        binding: checked_u32_field(entry, "binding", 0)?,
        visibility,
        ty,
        count: None,
    })
}

fn texture_value(id: GpuId, generation: u64, texture: &TextureResource) -> HostValue {
    resource_value(
        id,
        "texture",
        generation,
        [
            ("width", HostValue::Number(texture.width as f64)),
            ("height", HostValue::Number(texture.height as f64)),
            (
                "format",
                HostValue::String(format_name(texture.format).into()),
            ),
            (
                "slot",
                texture
                    .canvas_slot
                    .clone()
                    .map(HostValue::String)
                    .unwrap_or(HostValue::Null),
            ),
            (
                "textureGeneration",
                HostValue::Number(texture.generation as f64),
            ),
        ],
    )
}
fn resource_value<const N: usize>(
    id: GpuId,
    kind: &str,
    generation: u64,
    extra: [(&'static str, HostValue); N],
) -> HostValue {
    let mut values: BTreeMap<String, HostValue> = [
        ("__nanaGpuResource".into(), HostValue::Bool(true)),
        ("id".into(), HostValue::BigInt(id.0)),
        ("kind".into(), HostValue::String(kind.into())),
        ("generation".into(), HostValue::Number(generation as f64)),
    ]
    .into_iter()
    .collect();
    values.extend(extra.into_iter().map(|(k, v)| (k.into(), v)));
    HostValue::Object(values)
}
fn resource_id_value(value: &HostValue) -> Option<GpuId> {
    value
        .as_u64()
        .map(GpuId)
        .or_else(|| value.as_object()?.get("id")?.as_u64().map(GpuId))
}
fn gpu_id(args: &[HostValue], index: usize) -> Result<GpuId, JsException> {
    args.get(index)
        .and_then(resource_id_value)
        .ok_or_else(|| JsException::new(format!("missing GPU resource at argument {index}")))
}
fn object_arg(
    args: &[HostValue],
    index: usize,
) -> Result<&BTreeMap<String, HostValue>, JsException> {
    args.get(index)
        .and_then(HostValue::as_object)
        .ok_or_else(|| JsException::new(format!("expected descriptor object at argument {index}")))
}
fn optional_object_field<'a>(
    d: &'a BTreeMap<String, HostValue>,
    key: &str,
) -> Result<Option<&'a BTreeMap<String, HostValue>>, JsException> {
    match d.get(key) {
        None | Some(HostValue::Undefined) | Some(HostValue::Null) => Ok(None),
        Some(value) => value
            .as_object()
            .map(Some)
            .ok_or_else(|| webgpu_validation(format!("`{key}` must be an object"))),
    }
}
fn required_object_field<'a>(
    d: &'a BTreeMap<String, HostValue>,
    key: &str,
) -> Result<&'a BTreeMap<String, HostValue>, JsException> {
    optional_object_field(d, key)?
        .ok_or_else(|| webgpu_validation(format!("`{key}` object is required")))
}
fn checked_array_field<'a>(
    d: &'a BTreeMap<String, HostValue>,
    key: &str,
) -> Result<&'a [HostValue], JsException> {
    match d.get(key) {
        None | Some(HostValue::Undefined) => Ok(&[]),
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| webgpu_validation(format!("`{key}` must be an array"))),
    }
}
fn label(d: &BTreeMap<String, HostValue>) -> Option<&str> {
    d.get("label").and_then(HostValue::as_str)
}
fn checked_str_field<'a>(
    d: &'a BTreeMap<String, HostValue>,
    key: &str,
    default: &'a str,
) -> Result<&'a str, JsException> {
    match d.get(key) {
        None | Some(HostValue::Undefined) => Ok(default),
        Some(value) => value
            .as_str()
            .ok_or_else(|| webgpu_validation(format!("`{key}` must be a string"))),
    }
}
fn required_str_field<'a>(
    d: &'a BTreeMap<String, HostValue>,
    key: &str,
) -> Result<&'a str, JsException> {
    match d.get(key) {
        Some(value) => value
            .as_str()
            .ok_or_else(|| webgpu_validation(format!("`{key}` must be a string"))),
        None => Err(webgpu_validation(format!("`{key}` is required"))),
    }
}
fn optional_str_field<'a>(
    d: &'a BTreeMap<String, HostValue>,
    key: &str,
) -> Result<Option<&'a str>, JsException> {
    match d.get(key) {
        None | Some(HostValue::Undefined) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| webgpu_validation(format!("`{key}` must be a string"))),
    }
}
fn checked_u64_field(
    d: &BTreeMap<String, HostValue>,
    key: &str,
    default: u64,
) -> Result<u64, JsException> {
    match d.get(key) {
        None | Some(HostValue::Undefined) => Ok(default),
        Some(value) => value
            .as_u64()
            .ok_or_else(|| webgpu_validation(format!("`{key}` must be an unsigned integer"))),
    }
}
fn checked_u32_field(
    d: &BTreeMap<String, HostValue>,
    key: &str,
    default: u32,
) -> Result<u32, JsException> {
    let value = checked_u64_field(d, key, u64::from(default))?;
    u32::try_from(value)
        .map_err(|_| webgpu_validation(format!("`{key}` exceeds the supported u32 range")))
}
fn optional_u32_field(
    d: &BTreeMap<String, HostValue>,
    key: &str,
) -> Result<Option<u32>, JsException> {
    match d.get(key) {
        None | Some(HostValue::Undefined) => Ok(None),
        Some(_) => checked_u32_field(d, key, 0).map(Some),
    }
}
fn checked_f32_field(
    d: &BTreeMap<String, HostValue>,
    key: &str,
    default: f32,
) -> Result<f32, JsException> {
    let value = match d.get(key) {
        None | Some(HostValue::Undefined) => return Ok(default),
        Some(value) => value
            .as_f64()
            .ok_or_else(|| webgpu_validation(format!("`{key}` must be a number")))?,
    };
    if !value.is_finite() || value < f32::MIN as f64 || value > f32::MAX as f64 {
        return Err(webgpu_validation(format!("`{key}` must be a finite f32")));
    }
    Ok(value as f32)
}
fn checked_i32_field_value(
    d: &BTreeMap<String, HostValue>,
    key: &str,
    default: i32,
) -> Result<i32, JsException> {
    let value = match d.get(key) {
        None | Some(HostValue::Undefined) => return Ok(default),
        Some(value) => value
            .as_f64()
            .ok_or_else(|| webgpu_validation(format!("`{key}` must be an integer")))?,
    };
    if !value.is_finite()
        || value.fract() != 0.0
        || value < i32::MIN as f64
        || value > i32::MAX as f64
    {
        return Err(webgpu_validation(format!("`{key}` must be an i32")));
    }
    Ok(value as i32)
}
fn checked_bool_field(
    d: &BTreeMap<String, HostValue>,
    key: &str,
    default: bool,
) -> Result<bool, JsException> {
    match d.get(key) {
        None | Some(HostValue::Undefined) => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| webgpu_validation(format!("`{key}` must be a boolean"))),
    }
}
fn checked_u32_arg(
    args: &[HostValue],
    index: usize,
    default: Option<u32>,
) -> Result<u32, JsException> {
    let Some(value) = args.get(index) else {
        return default.ok_or_else(|| {
            webgpu_validation(format!("missing unsigned integer argument {index}"))
        });
    };
    let value = value.as_u64().ok_or_else(|| {
        webgpu_validation(format!("argument {index} must be an unsigned integer"))
    })?;
    u32::try_from(value)
        .map_err(|_| webgpu_validation(format!("argument {index} exceeds the u32 range")))
}
fn checked_u64_arg(
    args: &[HostValue],
    index: usize,
    default: Option<u64>,
) -> Result<u64, JsException> {
    match args.get(index) {
        Some(value) => value.as_u64().ok_or_else(|| {
            webgpu_validation(format!("argument {index} must be an unsigned integer"))
        }),
        None => default
            .ok_or_else(|| webgpu_validation(format!("missing unsigned integer argument {index}"))),
    }
}
fn checked_f32_arg(
    args: &[HostValue],
    index: usize,
    default: Option<f32>,
) -> Result<f32, JsException> {
    let value = match args.get(index) {
        Some(value) => value
            .as_f64()
            .ok_or_else(|| webgpu_validation(format!("argument {index} must be a number")))?,
        None => {
            return default
                .ok_or_else(|| webgpu_validation(format!("missing numeric argument {index}")));
        }
    };
    if !value.is_finite() || value < f32::MIN as f64 || value > f32::MAX as f64 {
        return Err(webgpu_validation(format!(
            "argument {index} must be a finite f32"
        )));
    }
    Ok(value as f32)
}
fn checked_i32_arg(args: &[HostValue], index: usize, default: i32) -> Result<i32, JsException> {
    let Some(value) = args.get(index) else {
        return Ok(default);
    };
    let value = value
        .as_f64()
        .ok_or_else(|| webgpu_validation(format!("argument {index} must be an integer")))?;
    if !value.is_finite()
        || value.fract() != 0.0
        || value < i32::MIN as f64
        || value > i32::MAX as f64
    {
        return Err(webgpu_validation(format!(
            "argument {index} must be an i32"
        )));
    }
    Ok(value as i32)
}
fn parse_color_value(value: &HostValue) -> Result<wgpu::Color, JsException> {
    let component = |value: Option<&HostValue>, name: &str| {
        let value = match value {
            None | Some(HostValue::Undefined) => 0.0,
            Some(value) => value.as_f64().ok_or_else(|| {
                webgpu_validation(format!("color component `{name}` must be a number"))
            })?,
        };
        if !value.is_finite() {
            return Err(webgpu_validation(format!(
                "color component `{name}` must be finite"
            )));
        }
        Ok(value)
    };
    if let Some(d) = value.as_object() {
        return Ok(wgpu::Color {
            r: component(d.get("r"), "r")?,
            g: component(d.get("g"), "g")?,
            b: component(d.get("b"), "b")?,
            a: component(d.get("a"), "a")?,
        });
    }
    let values = value
        .as_array()
        .ok_or_else(|| webgpu_validation("color must be an object or array"))?;
    Ok(wgpu::Color {
        r: component(values.first(), "r")?,
        g: component(values.get(1), "g")?,
        b: component(values.get(2), "b")?,
        a: component(values.get(3), "a")?,
    })
}
fn checked_size3(
    value: Option<&HostValue>,
    default: (u32, u32, u32),
) -> Result<(u32, u32, u32), JsException> {
    let component = |value: Option<&HostValue>, fallback: u32, name: &str| {
        let Some(value) = value else {
            return Ok(fallback);
        };
        let value = value.as_u64().ok_or_else(|| {
            webgpu_validation(format!("texture {name} must be an unsigned integer"))
        })?;
        let value = u32::try_from(value)
            .map_err(|_| webgpu_validation(format!("texture {name} exceeds the u32 range")))?;
        if value == 0 {
            return Err(webgpu_validation(format!(
                "texture {name} must be non-zero"
            )));
        }
        Ok(value)
    };
    if let Some(array) = value.and_then(HostValue::as_array) {
        return Ok((
            component(array.first(), default.0, "width")?,
            component(array.get(1), default.1, "height")?,
            component(array.get(2), default.2, "depthOrArrayLayers")?,
        ));
    }
    if let Some(d) = value.and_then(HostValue::as_object) {
        return Ok((
            component(d.get("width"), default.0, "width")?,
            component(d.get("height"), default.1, "height")?,
            component(d.get("depthOrArrayLayers"), default.2, "depthOrArrayLayers")?,
        ));
    }
    if value.is_some() {
        return Err(webgpu_validation(
            "texture size must be an array or extent object",
        ));
    }
    Ok(default)
}
fn parse_texture_format_checked(value: &str) -> Result<wgpu::TextureFormat, JsException> {
    Ok(match value {
        "rgba8unorm" => wgpu::TextureFormat::Rgba8Unorm,
        "bgra8unorm" => wgpu::TextureFormat::Bgra8Unorm,
        "bgra8unorm-srgb" => wgpu::TextureFormat::Bgra8UnormSrgb,
        "rgba8unorm-srgb" => wgpu::TextureFormat::Rgba8UnormSrgb,
        "rgba16float" => wgpu::TextureFormat::Rgba16Float,
        "r32float" => wgpu::TextureFormat::R32Float,
        "depth24plus" => wgpu::TextureFormat::Depth24Plus,
        "depth24plus-stencil8" => wgpu::TextureFormat::Depth24PlusStencil8,
        "depth32float" => wgpu::TextureFormat::Depth32Float,
        _ => {
            return Err(webgpu_validation(format!(
                "unsupported texture format `{value}`"
            )));
        }
    })
}
fn parse_vertex_format(value: &str) -> Result<wgpu::VertexFormat, JsException> {
    Ok(match value {
        "uint8x2" => wgpu::VertexFormat::Uint8x2,
        "uint8x4" => wgpu::VertexFormat::Uint8x4,
        "sint8x2" => wgpu::VertexFormat::Sint8x2,
        "sint8x4" => wgpu::VertexFormat::Sint8x4,
        "unorm8x2" => wgpu::VertexFormat::Unorm8x2,
        "unorm8x4" => wgpu::VertexFormat::Unorm8x4,
        "snorm8x2" => wgpu::VertexFormat::Snorm8x2,
        "snorm8x4" => wgpu::VertexFormat::Snorm8x4,
        "uint16x2" => wgpu::VertexFormat::Uint16x2,
        "uint16x4" => wgpu::VertexFormat::Uint16x4,
        "sint16x2" => wgpu::VertexFormat::Sint16x2,
        "sint16x4" => wgpu::VertexFormat::Sint16x4,
        "unorm16x2" => wgpu::VertexFormat::Unorm16x2,
        "unorm16x4" => wgpu::VertexFormat::Unorm16x4,
        "snorm16x2" => wgpu::VertexFormat::Snorm16x2,
        "snorm16x4" => wgpu::VertexFormat::Snorm16x4,
        "float16x2" => wgpu::VertexFormat::Float16x2,
        "float16x4" => wgpu::VertexFormat::Float16x4,
        "float32" => wgpu::VertexFormat::Float32,
        "float32x2" => wgpu::VertexFormat::Float32x2,
        "float32x3" => wgpu::VertexFormat::Float32x3,
        "float32x4" => wgpu::VertexFormat::Float32x4,
        "uint32" => wgpu::VertexFormat::Uint32,
        "uint32x2" => wgpu::VertexFormat::Uint32x2,
        "uint32x3" => wgpu::VertexFormat::Uint32x3,
        "uint32x4" => wgpu::VertexFormat::Uint32x4,
        "sint32" => wgpu::VertexFormat::Sint32,
        "sint32x2" => wgpu::VertexFormat::Sint32x2,
        "sint32x3" => wgpu::VertexFormat::Sint32x3,
        "sint32x4" => wgpu::VertexFormat::Sint32x4,
        "unorm10-10-10-2" => wgpu::VertexFormat::Unorm10_10_10_2,
        "unorm8x4-bgra" => wgpu::VertexFormat::Unorm8x4Bgra,
        _ => {
            return Err(JsException::new(format!(
                "unsupported WebGPU vertex format `{value}`"
            )));
        }
    })
}
fn format_name(value: wgpu::TextureFormat) -> &'static str {
    match value {
        wgpu::TextureFormat::Bgra8Unorm => "bgra8unorm",
        wgpu::TextureFormat::Bgra8UnormSrgb => "bgra8unorm-srgb",
        wgpu::TextureFormat::Rgba8UnormSrgb => "rgba8unorm-srgb",
        wgpu::TextureFormat::Rgba16Float => "rgba16float",
        wgpu::TextureFormat::R32Float => "r32float",
        _ => "rgba8unorm",
    }
}
fn parse_texture_dimension_checked(value: &str) -> Result<wgpu::TextureDimension, JsException> {
    Ok(match value {
        "1d" => wgpu::TextureDimension::D1,
        "2d" => wgpu::TextureDimension::D2,
        "3d" => wgpu::TextureDimension::D3,
        _ => {
            return Err(webgpu_validation(format!(
                "unsupported texture dimension `{value}`"
            )));
        }
    })
}
fn parse_view_dimension_checked(value: &str) -> Result<wgpu::TextureViewDimension, JsException> {
    Ok(match value {
        "1d" => wgpu::TextureViewDimension::D1,
        "2d" => wgpu::TextureViewDimension::D2,
        "2d-array" => wgpu::TextureViewDimension::D2Array,
        "cube" => wgpu::TextureViewDimension::Cube,
        "cube-array" => wgpu::TextureViewDimension::CubeArray,
        "3d" => wgpu::TextureViewDimension::D3,
        _ => {
            return Err(webgpu_validation(format!(
                "unsupported texture view dimension `{value}`"
            )));
        }
    })
}
fn parse_texture_aspect_checked(value: &str) -> Result<wgpu::TextureAspect, JsException> {
    Ok(match value {
        "all" => wgpu::TextureAspect::All,
        "depth-only" => wgpu::TextureAspect::DepthOnly,
        "stencil-only" => wgpu::TextureAspect::StencilOnly,
        _ => {
            return Err(webgpu_validation(format!(
                "unsupported texture aspect `{value}`"
            )));
        }
    })
}
fn parse_origin(value: Option<&HostValue>) -> Result<wgpu::Origin3d, JsException> {
    if let Some(array) = value.and_then(HostValue::as_array) {
        return Ok(wgpu::Origin3d {
            x: checked_u32_arg(array, 0, Some(0))?,
            y: checked_u32_arg(array, 1, Some(0))?,
            z: checked_u32_arg(array, 2, Some(0))?,
        });
    }
    if let Some(origin) = value.and_then(HostValue::as_object) {
        return Ok(wgpu::Origin3d {
            x: checked_u32_field(origin, "x", 0)?,
            y: checked_u32_field(origin, "y", 0)?,
            z: checked_u32_field(origin, "z", 0)?,
        });
    }
    if value.is_some() {
        return Err(webgpu_validation(
            "texture origin must be an array or object",
        ));
    }
    Ok(wgpu::Origin3d::ZERO)
}
fn parse_address(value: &str) -> Result<wgpu::AddressMode, JsException> {
    Ok(match value {
        "clamp-to-edge" => wgpu::AddressMode::ClampToEdge,
        "repeat" => wgpu::AddressMode::Repeat,
        "mirror-repeat" => wgpu::AddressMode::MirrorRepeat,
        _ => {
            return Err(webgpu_validation(format!(
                "unsupported address mode `{value}`"
            )));
        }
    })
}
fn parse_filter(value: &str) -> Result<wgpu::FilterMode, JsException> {
    Ok(match value {
        "nearest" => wgpu::FilterMode::Nearest,
        "linear" => wgpu::FilterMode::Linear,
        _ => {
            return Err(webgpu_validation(format!(
                "unsupported filter mode `{value}`"
            )));
        }
    })
}
fn parse_mipmap_filter(value: &str) -> Result<wgpu::MipmapFilterMode, JsException> {
    Ok(match value {
        "nearest" => wgpu::MipmapFilterMode::Nearest,
        "linear" => wgpu::MipmapFilterMode::Linear,
        _ => {
            return Err(webgpu_validation(format!(
                "unsupported mipmap filter mode `{value}`"
            )));
        }
    })
}
fn parse_compare(value: &str) -> Result<wgpu::CompareFunction, JsException> {
    Ok(match value {
        "less" => wgpu::CompareFunction::Less,
        "less-equal" => wgpu::CompareFunction::LessEqual,
        "greater" => wgpu::CompareFunction::Greater,
        "greater-equal" => wgpu::CompareFunction::GreaterEqual,
        "equal" => wgpu::CompareFunction::Equal,
        "not-equal" => wgpu::CompareFunction::NotEqual,
        "never" => wgpu::CompareFunction::Never,
        "always" => wgpu::CompareFunction::Always,
        _ => {
            return Err(webgpu_validation(format!(
                "unsupported compare function `{value}`"
            )));
        }
    })
}
fn parse_topology(value: &str) -> Result<wgpu::PrimitiveTopology, JsException> {
    Ok(match value {
        "point-list" => wgpu::PrimitiveTopology::PointList,
        "line-list" => wgpu::PrimitiveTopology::LineList,
        "line-strip" => wgpu::PrimitiveTopology::LineStrip,
        "triangle-list" => wgpu::PrimitiveTopology::TriangleList,
        "triangle-strip" => wgpu::PrimitiveTopology::TriangleStrip,
        _ => {
            return Err(webgpu_validation(format!(
                "unsupported primitive topology `{value}`"
            )));
        }
    })
}
fn parse_cull(value: &str) -> Result<Option<wgpu::Face>, JsException> {
    Ok(match value {
        "none" => None,
        "front" => Some(wgpu::Face::Front),
        "back" => Some(wgpu::Face::Back),
        _ => {
            return Err(webgpu_validation(format!(
                "unsupported cull mode `{value}`"
            )));
        }
    })
}

fn validate_buffer_descriptor(size: u64, usage: wgpu::BufferUsages) -> Result<(), JsException> {
    let map_read_extras = usage & !wgpu::BufferUsages::MAP_READ;
    if usage.contains(wgpu::BufferUsages::MAP_READ)
        && !map_read_extras
            .difference(wgpu::BufferUsages::COPY_DST)
            .is_empty()
    {
        return Err(webgpu_validation(
            "MAP_READ buffers may only add COPY_DST usage in the NanaUI WebGPU subset",
        ));
    }
    let map_write_extras = usage & !wgpu::BufferUsages::MAP_WRITE;
    if usage.contains(wgpu::BufferUsages::MAP_WRITE)
        && !map_write_extras
            .difference(wgpu::BufferUsages::COPY_SRC)
            .is_empty()
    {
        return Err(webgpu_validation(
            "MAP_WRITE buffers may only add COPY_SRC usage in the NanaUI WebGPU subset",
        ));
    }
    if usage.intersects(wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::STORAGE) && size == 0 {
        return Err(webgpu_validation(
            "uniform and storage buffers must have a non-zero size",
        ));
    }
    Ok(())
}

fn ensure_unique_bindings(
    bindings: impl IntoIterator<Item = u32>,
    descriptor: &str,
) -> Result<(), JsException> {
    let mut seen = HashSet::new();
    for binding in bindings {
        if !seen.insert(binding) {
            return Err(webgpu_validation(format!(
                "{descriptor} contains duplicate index {binding}"
            )));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_texture_descriptor(
    width: u32,
    height: u32,
    depth: u32,
    mip_level_count: u32,
    sample_count: u32,
    dimension: wgpu::TextureDimension,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> Result<(), JsException> {
    if width == 0 || height == 0 || depth == 0 {
        return Err(webgpu_validation("texture dimensions must be non-zero"));
    }
    if mip_level_count == 0 {
        return Err(webgpu_validation("mipLevelCount must be non-zero"));
    }
    if !matches!(sample_count, 1 | 4) {
        return Err(webgpu_validation(
            "the NanaUI WebGPU subset supports sampleCount 1 or 4",
        ));
    }
    if sample_count > 1
        && (mip_level_count != 1
            || dimension != wgpu::TextureDimension::D2
            || depth != 1
            || !usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
            || usage.intersects(
                wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::STORAGE_BINDING,
            ))
    {
        return Err(webgpu_validation(
            "multisampled textures must be single-mip 2D render attachments",
        ));
    }
    if dimension == wgpu::TextureDimension::D1 && (height != 1 || depth != 1) {
        return Err(webgpu_validation(
            "1D textures require height and depthOrArrayLayers equal to 1",
        ));
    }
    if dimension == wgpu::TextureDimension::D3 && sample_count != 1 {
        return Err(webgpu_validation("3D textures cannot be multisampled"));
    }
    let max_mips = 32
        - width
            .max(height)
            .max(if dimension == wgpu::TextureDimension::D3 {
                depth
            } else {
                1
            })
            .leading_zeros();
    if mip_level_count > max_mips {
        return Err(webgpu_validation(
            "mipLevelCount exceeds texture dimensions",
        ));
    }
    if format.is_depth_stencil_format() && usage.intersects(wgpu::TextureUsages::STORAGE_BINDING) {
        return Err(webgpu_validation(
            "depth/stencil textures cannot use STORAGE_BINDING",
        ));
    }
    Ok(())
}

fn validate_buffer_copy(
    state: &WebGpuState,
    source: GpuId,
    source_offset: u64,
    destination: GpuId,
    destination_offset: u64,
    size: u64,
) -> Result<(), JsException> {
    if source == destination {
        return Err(webgpu_validation(
            "copyBufferToBuffer source and destination must differ",
        ));
    }
    if !source_offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
        || !destination_offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
        || !size.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
    {
        return Err(webgpu_validation(
            "copyBufferToBuffer offsets and size must be multiples of 4",
        ));
    }
    let source_size = state
        .buffer_sizes
        .get(&source)
        .copied()
        .ok_or_else(|| unknown("buffer", source))?;
    let destination_size = state
        .buffer_sizes
        .get(&destination)
        .copied()
        .ok_or_else(|| unknown("buffer", destination))?;
    if !state
        .buffer_usages
        .get(&source)
        .is_some_and(|usage| usage.contains(wgpu::BufferUsages::COPY_SRC))
        || !state
            .buffer_usages
            .get(&destination)
            .is_some_and(|usage| usage.contains(wgpu::BufferUsages::COPY_DST))
    {
        return Err(webgpu_validation(
            "copyBufferToBuffer requires COPY_SRC and COPY_DST usages",
        ));
    }
    if source_offset
        .checked_add(size)
        .is_none_or(|end| end > source_size)
        || destination_offset
            .checked_add(size)
            .is_none_or(|end| end > destination_size)
    {
        return Err(webgpu_validation(
            "copyBufferToBuffer range is out of bounds",
        ));
    }
    Ok(())
}

fn validate_pass_command(
    state: &WebGpuState,
    pass_id: GpuId,
    command: &PassCommand,
) -> Result<(), JsException> {
    let pass = state
        .passes
        .get(&pass_id)
        .ok_or_else(|| unknown("pass", pass_id))?;
    let render = pass.render_colors.is_some();
    match command {
        PassCommand::SetPipeline(id) => {
            let valid = if render {
                state.render_pipelines.contains_key(id)
            } else {
                state.compute_pipelines.contains_key(id)
            };
            if !valid {
                return Err(unknown(
                    if render {
                        "render-pipeline"
                    } else {
                        "compute-pipeline"
                    },
                    *id,
                ));
            }
        }
        PassCommand::SetBindGroup(_, id, _) => {
            if !state.bind_groups.contains_key(id) {
                return Err(unknown("bind-group", *id));
            }
        }
        PassCommand::SetVertexBuffer(_, id, offset, size) => {
            if !render {
                return Err(webgpu_validation(
                    "setVertexBuffer is only valid in a render pass",
                ));
            }
            validate_buffer_slice(state, *id, *offset, *size, wgpu::BufferUsages::VERTEX, 4)?;
        }
        PassCommand::SetIndexBuffer(id, format, offset, size) => {
            if !render {
                return Err(webgpu_validation(
                    "setIndexBuffer is only valid in a render pass",
                ));
            }
            let alignment = match format {
                wgpu::IndexFormat::Uint16 => 2,
                wgpu::IndexFormat::Uint32 => 4,
            };
            validate_buffer_slice(
                state,
                *id,
                *offset,
                *size,
                wgpu::BufferUsages::INDEX,
                alignment,
            )?;
        }
        PassCommand::SetViewport(x, y, width, height, min_depth, max_depth) => {
            let extent = render_pass_extent(state, pass);
            if !render
                || ![x, y, width, height, min_depth, max_depth]
                    .into_iter()
                    .all(|value| value.is_finite())
                || *width < 0.0
                || *height < 0.0
                || !(0.0..=1.0).contains(min_depth)
                || !(0.0..=1.0).contains(max_depth)
                || min_depth > max_depth
                || *x < 0.0
                || *y < 0.0
                || extent.is_some_and(|(extent_width, extent_height)| {
                    *x + *width > extent_width as f32 || *y + *height > extent_height as f32
                })
            {
                return Err(webgpu_validation("invalid render-pass viewport"));
            }
        }
        PassCommand::SetScissorRect(x, y, width, height) => {
            let extent = render_pass_extent(state, pass);
            if !render
                || (*x).checked_add(*width).is_none()
                || (*y).checked_add(*height).is_none()
                || extent.is_some_and(|(extent_width, extent_height)| {
                    *x + *width > extent_width || *y + *height > extent_height
                })
            {
                return Err(webgpu_validation("invalid render-pass scissor rectangle"));
            }
        }
        PassCommand::SetBlendConstant(_) | PassCommand::SetStencilReference(_) => {
            if !render {
                return Err(webgpu_validation(
                    "render state command is not valid in a compute pass",
                ));
            }
        }
        PassCommand::Draw(vertices, instances, first_vertex, first_instance) => {
            if !render
                || first_vertex.checked_add(*vertices).is_none()
                || first_instance.checked_add(*instances).is_none()
            {
                return Err(webgpu_validation("invalid draw range"));
            }
        }
        PassCommand::DrawIndexed(indices, instances, first_index, _, first_instance) => {
            if !render
                || first_index.checked_add(*indices).is_none()
                || first_instance.checked_add(*instances).is_none()
            {
                return Err(webgpu_validation("invalid drawIndexed range"));
            }
        }
        PassCommand::Dispatch(_, _, _) if render => {
            return Err(webgpu_validation(
                "dispatchWorkgroups is only valid in a compute pass",
            ));
        }
        PassCommand::Dispatch(_, _, _) => {}
    }
    Ok(())
}

fn render_pass_extent(state: &WebGpuState, pass: &OpenPass) -> Option<(u32, u32)> {
    let view = pass
        .render_colors
        .as_ref()
        .and_then(|colors| colors.first().map(|attachment| attachment.view))
        .or_else(|| {
            pass.render_depth_stencil
                .as_ref()
                .map(|attachment| attachment.view)
        })?;
    state
        .view_extents
        .get(&view)
        .map(|(width, height, _)| (*width, *height))
}

fn validate_buffer_slice(
    state: &WebGpuState,
    id: GpuId,
    offset: u64,
    size: Option<u64>,
    required_usage: wgpu::BufferUsages,
    alignment: u64,
) -> Result<(), JsException> {
    let buffer_size = state
        .buffer_sizes
        .get(&id)
        .copied()
        .ok_or_else(|| unknown("buffer", id))?;
    if !state
        .buffer_usages
        .get(&id)
        .is_some_and(|usage| usage.contains(required_usage))
    {
        return Err(webgpu_validation(
            "buffer usage is incompatible with pass binding",
        ));
    }
    if !offset.is_multiple_of(alignment)
        || size.is_some_and(|size| {
            size == 0
                || size % alignment != 0
                || offset.checked_add(size).is_none_or(|end| end > buffer_size)
        })
        || size.is_none() && offset >= buffer_size
    {
        return Err(webgpu_validation("buffer binding range is invalid"));
    }
    Ok(())
}

fn validate_render_attachments(
    state: &WebGpuState,
    colors: &[ColorAttachment],
    depth_stencil: Option<&DepthStencilAttachment>,
) -> Result<(), JsException> {
    if colors.is_empty() && depth_stencil.is_none() {
        return Err(webgpu_validation(
            "render pass requires a color or depth/stencil attachment",
        ));
    }
    let mut extent_and_samples = None;
    for view in colors
        .iter()
        .map(|color| color.view)
        .chain(depth_stencil.map(|attachment| attachment.view))
    {
        let texture_id = state
            .view_textures
            .get(&view)
            .copied()
            .ok_or_else(|| unknown("texture-view", view))?;
        let texture = state
            .textures
            .get(&texture_id)
            .ok_or_else(|| unknown("texture", texture_id))?;
        if !texture
            .usage
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err(webgpu_validation(
                "render-pass attachment lacks RENDER_ATTACHMENT usage",
            ));
        }
        let (view_width, view_height, _) = state
            .view_extents
            .get(&view)
            .copied()
            .ok_or_else(|| unknown("texture-view", view))?;
        let current = (view_width, view_height, texture.sample_count);
        if extent_and_samples.is_some_and(|expected| expected != current) {
            return Err(webgpu_validation(
                "render-pass attachments must have matching extent and sample count",
            ));
        }
        extent_and_samples = Some(current);
    }
    for color in colors {
        let texture_id = state.view_textures[&color.view];
        if state.textures[&texture_id].format.is_depth_stencil_format() {
            return Err(webgpu_validation(
                "color attachment cannot use a depth/stencil format",
            ));
        }
        if let Some(resolve) = color.resolve_target {
            let source = &state.textures[&texture_id];
            let resolve_texture = &state.textures[&state.view_textures[&resolve]];
            let source_extent = state.view_extents[&color.view];
            let resolve_extent = state.view_extents[&resolve];
            if source.sample_count == 1
                || resolve_texture.sample_count != 1
                || source.format != resolve_texture.format
                || source_extent.0 != resolve_extent.0
                || source_extent.1 != resolve_extent.1
                || !resolve_texture
                    .usage
                    .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
            {
                return Err(webgpu_validation("invalid multisample resolve target"));
            }
        }
    }
    if let Some(depth) = depth_stencil {
        let texture = &state.textures[&state.view_textures[&depth.view]];
        if !texture.format.is_depth_stencil_format() {
            return Err(webgpu_validation(
                "depth/stencil attachment requires a depth/stencil format",
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_texture_write(
    texture: &TextureResource,
    mip_level: u32,
    origin: wgpu::Origin3d,
    aspect: wgpu::TextureAspect,
    width: u32,
    height: u32,
    depth: u32,
    offset: u64,
    bytes_per_row: Option<u32>,
    rows_per_image: Option<u32>,
    data_len: usize,
) -> Result<(), JsException> {
    if !texture.usage.contains(wgpu::TextureUsages::COPY_DST) {
        return Err(webgpu_validation(
            "writeTexture destination lacks COPY_DST usage",
        ));
    }
    if texture.sample_count != 1 || mip_level >= texture.mip_level_count {
        return Err(webgpu_validation(
            "writeTexture requires a valid mip of a single-sampled texture",
        ));
    }
    if !texture_aspect_compatible(texture.format, aspect) {
        return Err(webgpu_validation(
            "writeTexture aspect is incompatible with texture format",
        ));
    }
    let mip_width = (texture.width >> mip_level).max(1);
    let mip_height = (texture.height >> mip_level).max(1);
    let mip_depth = if texture.dimension == wgpu::TextureDimension::D3 {
        (texture.depth >> mip_level).max(1)
    } else {
        texture.depth
    };
    if origin
        .x
        .checked_add(width)
        .is_none_or(|end| end > mip_width)
        || origin
            .y
            .checked_add(height)
            .is_none_or(|end| end > mip_height)
        || origin
            .z
            .checked_add(depth)
            .is_none_or(|end| end > mip_depth)
    {
        return Err(webgpu_validation("writeTexture extent is out of bounds"));
    }
    let (block_width, block_height) = texture.format.block_dimensions();
    let block_size = texture
        .format
        .block_copy_size(Some(aspect))
        .ok_or_else(|| webgpu_validation("texture format/aspect is not copyable"))?;
    if !origin.x.is_multiple_of(block_width)
        || !origin.y.is_multiple_of(block_height)
        || !width.is_multiple_of(block_width) && origin.x + width != mip_width
        || !height.is_multiple_of(block_height) && origin.y + height != mip_height
    {
        return Err(webgpu_validation(
            "writeTexture origin and extent must respect format block dimensions",
        ));
    }
    let width_blocks = width.div_ceil(block_width);
    let height_blocks = height.div_ceil(block_height);
    let row_bytes = width_blocks
        .checked_mul(block_size)
        .ok_or_else(|| webgpu_validation("writeTexture row size overflows"))?;
    let stride = bytes_per_row.unwrap_or(row_bytes);
    if stride < row_bytes
        || !stride.is_multiple_of(block_size)
        || height_blocks > 1 && bytes_per_row.is_none()
    {
        return Err(webgpu_validation("invalid writeTexture bytesPerRow"));
    }
    let image_rows = rows_per_image.unwrap_or(height_blocks);
    if image_rows < height_blocks || depth > 1 && rows_per_image.is_none() {
        return Err(webgpu_validation("invalid writeTexture rowsPerImage"));
    }
    let required = offset
        .checked_add(u64::from(depth.saturating_sub(1)) * u64::from(image_rows) * u64::from(stride))
        .and_then(|value| {
            value.checked_add(u64::from(height_blocks.saturating_sub(1)) * u64::from(stride))
        })
        .and_then(|value| value.checked_add(u64::from(row_bytes)))
        .ok_or_else(|| webgpu_validation("writeTexture data layout overflows"))?;
    if required > data_len as u64 {
        return Err(webgpu_validation(
            "writeTexture data is too small for the copy",
        ));
    }
    Ok(())
}

fn texture_aspect_compatible(format: wgpu::TextureFormat, aspect: wgpu::TextureAspect) -> bool {
    match aspect {
        wgpu::TextureAspect::All => true,
        wgpu::TextureAspect::DepthOnly => format.is_depth_stencil_format(),
        wgpu::TextureAspect::StencilOnly => {
            matches!(format, wgpu::TextureFormat::Depth24PlusStencil8)
        }
        _ => false,
    }
}

fn remove_texture_views(state: &mut WebGpuState, texture: GpuId) {
    let views = state
        .view_textures
        .iter()
        .filter_map(|(view, owner)| (*owner == texture).then_some(*view))
        .collect::<Vec<_>>();
    for view in views {
        state.view_textures.remove(&view);
        state.view_extents.remove(&view);
        state.views.remove(&view);
    }
}

fn poisoned<T>(_: std::sync::PoisonError<T>) -> JsException {
    JsException::new("WebGPU runtime poisoned")
}
fn webgpu_validation(message: impl Into<String>) -> JsException {
    JsException::new(format!("WebGPU validation error: {}", message.into()))
        .with_name("OperationError")
        .with_code("webgpu_validation")
}
fn gpu_device_lost(message: impl Into<String>) -> JsException {
    JsException::new(message)
        .with_name("GPUDeviceLostError")
        .with_code("webgpu_device_lost")
}
fn gpu_error_value(error: wgpu::Error) -> HostValue {
    let (name, code, message) = match error {
        wgpu::Error::Validation { description, .. } => {
            ("GPUValidationError", "validation", description)
        }
        wgpu::Error::OutOfMemory { source } => {
            ("GPUOutOfMemoryError", "out-of-memory", source.to_string())
        }
        wgpu::Error::Internal { description, .. } => ("GPUInternalError", "internal", description),
    };
    HostValue::Object(
        [
            ("name".into(), HostValue::String(name.into())),
            ("code".into(), HostValue::String(code.into())),
            ("message".into(), HostValue::String(message)),
        ]
        .into_iter()
        .collect(),
    )
}
fn wgpu_error_exception(error: wgpu::Error) -> JsException {
    match error {
        wgpu::Error::Validation { description, .. } => webgpu_validation(description),
        wgpu::Error::OutOfMemory { source } => JsException::new(source.to_string())
            .with_name("GPUOutOfMemoryError")
            .with_code("webgpu_out_of_memory"),
        wgpu::Error::Internal { description, .. } => JsException::new(description)
            .with_name("GPUInternalError")
            .with_code("webgpu_internal"),
    }
}
fn unknown(kind: &str, id: GpuId) -> JsException {
    JsException::new(format!("unknown or expired GPU {kind} {}", id.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_descriptors_and_resource_ids_preserve_webgpu_shapes() {
        assert_eq!(
            checked_size3(
                Some(&HostValue::Array(vec![
                    HostValue::Number(4.0),
                    HostValue::Number(8.0)
                ])),
                (1, 1, 1)
            )
            .unwrap(),
            (4, 8, 1)
        );
        let value = resource_value(GpuId(u64::MAX), "buffer", 3, []);
        assert_eq!(
            value.as_object().unwrap().get("id"),
            Some(&HostValue::BigInt(u64::MAX))
        );
    }
}
