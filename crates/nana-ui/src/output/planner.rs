//! Pure render-target planning.
//!
//! The planner only negotiates a topology.  It does not create textures,
//! submit work, or inspect a native handle.  A host can therefore run it at a
//! configuration boundary (attach, detach, resize, format change, or device
//! replacement) and keep the resulting plan on the frame path.

use std::fmt;

use crate::{AlphaEncoding, ScenePresentationColorSpace};
use nana_gpu::{DeviceGeneration, GpuTextureFormat, GpuTextureUsages};
use nana_ui_core::{OutputWorkObservation, WorkCounters};

/// Stable identity for one output consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConsumerId(pub u64);

/// The kind of host consuming a presentation.  These are descriptive: the
/// planner never assumes that a kind has native interop support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConsumerKind {
    Window,
    Nested,
    Native,
    Xr,
    Web,
    Headless,
    Capture,
}

/// Requirements of a logical render target or consumer destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderTargetRequirements {
    pub extent: [u32; 2],
    pub format: GpuTextureFormat,
    pub color_space: ScenePresentationColorSpace,
    pub alpha: AlphaEncoding,
    pub sample_count: u32,
    pub usage: GpuTextureUsages,
    pub persistent: bool,
    pub exportable: bool,
}

impl RenderTargetRequirements {
    /// A one-sample render target suitable for a direct window surface.
    pub const fn new(extent: [u32; 2], format: GpuTextureFormat) -> Self {
        Self {
            extent,
            format,
            color_space: ScenePresentationColorSpace::Srgb,
            alpha: AlphaEncoding::Linear,
            sample_count: 1,
            usage: GpuTextureUsages::RENDER_TARGET,
            persistent: false,
            exportable: false,
        }
    }

    pub const fn persistent(mut self, value: bool) -> Self {
        self.persistent = value;
        self
    }

    pub const fn exportable(mut self, value: bool) -> Self {
        self.exportable = value;
        self
    }

    pub const fn with_usage(mut self, usage: GpuTextureUsages) -> Self {
        self.usage = usage;
        self
    }

    pub const fn with_sample_count(mut self, sample_count: u32) -> Self {
        self.sample_count = sample_count;
        self
    }

    pub const fn with_color_space(mut self, color_space: ScenePresentationColorSpace) -> Self {
        self.color_space = color_space;
        self
    }

    pub const fn with_alpha(mut self, alpha: AlphaEncoding) -> Self {
        self.alpha = alpha;
        self
    }

    fn compatible(self, other: Self) -> bool {
        self.extent == other.extent
            && self.format == other.format
            && self.color_space == other.color_space
            && self.alpha == other.alpha
            && self.sample_count == other.sample_count
            && self.usage.contains(other.usage)
            && (!other.persistent || self.persistent)
            && (!other.exportable || self.exportable)
    }
}

/// Capabilities explicitly advertised by a consumer adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PresenterCapabilities {
    pub direct_surface: bool,
    pub same_device_sample: bool,
    /// The adapter can copy/convert from the canonical source into its
    /// declared destination requirements.
    pub gpu_copy: bool,
    pub native_share: bool,
    pub cpu_fallback: bool,
    /// CPU fallback is a policy choice.  A capable adapter must opt into it
    /// for a plan to select that path.
    pub allow_cpu_fallback: bool,
}

/// One consumer and its negotiated destination requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputConsumer {
    pub id: ConsumerId,
    pub kind: ConsumerKind,
    pub requirements: RenderTargetRequirements,
    pub capabilities: PresenterCapabilities,
    /// Generation of the consumer's GPU, when it is known.  An absent value
    /// never counts as proof of a same-device path.
    pub device_generation: Option<DeviceGeneration>,
}

impl OutputConsumer {
    pub const fn new(
        id: ConsumerId,
        kind: ConsumerKind,
        requirements: RenderTargetRequirements,
        capabilities: PresenterCapabilities,
    ) -> Self {
        Self {
            id,
            kind,
            requirements,
            capabilities,
            device_generation: None,
        }
    }

    pub const fn with_device_generation(mut self, generation: DeviceGeneration) -> Self {
        self.device_generation = Some(generation);
        self
    }
}

/// Path selected for a consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputPath {
    DirectSurface,
    SameDeviceSample,
    SharedNativeResource,
    GpuTransfer,
    CpuFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConsumerRoute {
    pub consumer: ConsumerId,
    pub path: OutputPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputTopology {
    DirectSurface,
    CanonicalRetained,
}

/// A stable result of one configuration negotiation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputPlan {
    pub topology: OutputTopology,
    pub canonical: RenderTargetRequirements,
    pub routes: Vec<ConsumerRoute>,
}

impl OutputPlan {
    pub fn path_for(&self, consumer: ConsumerId) -> Option<OutputPath> {
        self.routes
            .iter()
            .find(|route| route.consumer == consumer)
            .map(|route| route.path)
    }

    pub const fn consumer_count(&self) -> usize {
        self.routes.len()
    }

    pub fn uses_cpu_fallback(&self) -> bool {
        let mut index = 0;
        while index < self.routes.len() {
            if matches!(self.routes[index].path, OutputPath::CpuFallback) {
                return true;
            }
            index += 1;
        }
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    NoConsumers,
    InvalidExtent,
    MissingRenderUsage,
    UnsupportedSampleCount { sample_count: u32 },
    DuplicateConsumer { consumer: ConsumerId },
    IncompatibleRequirements { consumer: ConsumerId },
    NoCapability { consumer: ConsumerId },
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoConsumers => f.write_str("render-target planner has no consumers"),
            Self::InvalidExtent => f.write_str("render-target extent must be non-zero"),
            Self::MissingRenderUsage => {
                f.write_str("render-target usage must include RENDER_TARGET")
            }
            Self::UnsupportedSampleCount { sample_count } => {
                write!(
                    f,
                    "render-target sample count {sample_count} is unsupported"
                )
            }
            Self::DuplicateConsumer { consumer } => {
                write!(f, "consumer {:?} is attached more than once", consumer)
            }
            Self::IncompatibleRequirements { consumer } => {
                write!(
                    f,
                    "consumer {:?} has incompatible render-target requirements",
                    consumer
                )
            }
            Self::NoCapability { consumer } => {
                write!(
                    f,
                    "consumer {:?} advertises no compatible output path",
                    consumer
                )
            }
        }
    }
}

impl std::error::Error for PlanError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanKey {
    canonical: RenderTargetRequirements,
    consumers: Vec<OutputConsumer>,
    device_generation: Option<DeviceGeneration>,
}

/// Negotiates and caches output topology.  `configure` is intentionally the
/// only operation that computes a plan; calling it repeatedly with equal
/// descriptors is a no-op.
#[derive(Debug, Default)]
pub struct RenderTargetPlanner {
    device_generation: Option<DeviceGeneration>,
    key: Option<PlanKey>,
    plan: Option<OutputPlan>,
    rebuilds: u64,
    consumers: Vec<OutputConsumer>,
    last_work: OutputWorkObservation,
}

impl RenderTargetPlanner {
    pub const fn new() -> Self {
        Self {
            device_generation: None,
            key: None,
            plan: None,
            rebuilds: 0,
            consumers: Vec::new(),
            last_work: OutputWorkObservation {
                target_plan_rebuilds: 0,
                extra_passes: 0,
                gpu_copies: 0,
                cpu_readbacks: 0,
                canonical_target_count: 0,
                consumer_count: 0,
                target_recreates: 0,
                content_revisions: 0,
                idle_reuse_frames: 0,
                resolve_count: 0,
                gpu_copy_bytes: 0,
                gpu_convert_passes: 0,
                cpu_fallback_frames: 0,
            },
        }
    }

    pub fn device_generation(&self) -> Option<DeviceGeneration> {
        self.device_generation
    }

    /// Set the producer device generation.  A replacement device always
    /// invalidates the cached topology.
    pub fn set_device_generation(&mut self, generation: Option<DeviceGeneration>) {
        if self.device_generation != generation {
            self.device_generation = generation;
            self.key = None;
            self.plan = None;
            self.last_work = OutputWorkObservation::default();
        }
    }

    pub fn plan(&self) -> Option<&OutputPlan> {
        self.plan.as_ref()
    }

    pub const fn rebuilds(&self) -> u64 {
        self.rebuilds
    }

    /// Output work observed by the most recent configuration operation. A
    /// cache hit reports an empty observation; planning is never a per-frame
    /// dispatch cost.
    pub const fn last_work(&self) -> OutputWorkObservation {
        self.last_work
    }

    /// Fold the last configuration observation into a Runtime frame snapshot.
    /// Planning is configuration-bound, so callers can record it on the same
    /// frame that applied the attach/resize/device change.
    pub fn record_last_work(&self, counters: &mut WorkCounters) {
        counters.record_output_work(self.last_work);
    }

    pub fn configure(
        &mut self,
        canonical: RenderTargetRequirements,
        consumers: &[OutputConsumer],
    ) -> Result<&OutputPlan, PlanError> {
        self.last_work = OutputWorkObservation::default();
        let mut consumers = consumers.to_vec();
        consumers.sort_by_key(|consumer| consumer.id);
        let key = PlanKey {
            canonical,
            consumers: consumers.clone(),
            device_generation: self.device_generation,
        };
        if self.key.as_ref() == Some(&key) {
            return self.plan.as_ref().ok_or(PlanError::NoConsumers);
        }
        let plan = build_plan(canonical, &consumers, self.device_generation)?;
        let consumer_count = consumers.len();
        self.key = Some(key);
        self.consumers = consumers;
        self.plan = Some(plan);
        self.rebuilds = self.rebuilds.saturating_add(1);
        self.last_work = OutputWorkObservation {
            target_plan_rebuilds: 1,
            canonical_target_count: 1,
            consumer_count,
            ..Default::default()
        };
        nana_diagnostics::metric!(nana_diagnostics::framework::gpu::OUTPUT_TARGET_PLAN_REBUILDS);
        nana_diagnostics::metric!(
            nana_diagnostics::framework::gpu::OUTPUT_CANONICAL_TARGETS,
            1u64
        );
        nana_diagnostics::metric!(
            nana_diagnostics::framework::gpu::OUTPUT_CONSUMERS,
            consumer_count as u64
        );
        Ok(self.plan.as_ref().expect("plan inserted"))
    }

    /// Attach or replace one consumer and negotiate at the configuration
    /// boundary. Re-attaching an equal descriptor does not rebuild.
    pub fn attach_consumer(
        &mut self,
        canonical: RenderTargetRequirements,
        consumer: OutputConsumer,
    ) -> Result<&OutputPlan, PlanError> {
        let mut consumers = self.consumers.clone();
        if let Some(existing) = consumers.iter_mut().find(|item| item.id == consumer.id) {
            *existing = consumer;
        } else {
            consumers.push(consumer);
        }
        self.configure(canonical, &consumers)
    }

    /// Detaching an unknown consumer is a no-op and does not rebuild.
    pub fn detach_consumer(
        &mut self,
        canonical: RenderTargetRequirements,
        consumer: ConsumerId,
    ) -> Result<&OutputPlan, PlanError> {
        let consumers: Vec<_> = self
            .consumers
            .iter()
            .copied()
            .filter(|item| item.id != consumer)
            .collect();
        self.configure(canonical, &consumers)
    }
}

fn build_plan(
    canonical: RenderTargetRequirements,
    consumers: &[OutputConsumer],
    generation: Option<DeviceGeneration>,
) -> Result<OutputPlan, PlanError> {
    if consumers.is_empty() {
        return Err(PlanError::NoConsumers);
    }
    if consumers
        .windows(2)
        .find(|pair| pair[0].id == pair[1].id)
        .is_some()
    {
        return Err(PlanError::DuplicateConsumer {
            consumer: consumers
                .windows(2)
                .find(|pair| pair[0].id == pair[1].id)
                .map(|pair| pair[0].id)
                .expect("duplicate checked above"),
        });
    }
    if canonical.extent.contains(&0) {
        return Err(PlanError::InvalidExtent);
    }
    if !canonical.usage.contains(GpuTextureUsages::RENDER_TARGET) {
        return Err(PlanError::MissingRenderUsage);
    }
    if canonical.sample_count != 1 {
        return Err(PlanError::UnsupportedSampleCount {
            sample_count: canonical.sample_count,
        });
    }

    let direct = consumers.len() == 1
        && consumers[0].kind == ConsumerKind::Window
        && consumers[0].capabilities.direct_surface
        && !canonical.persistent
        && !canonical.exportable
        && canonical.compatible(consumers[0].requirements);
    if direct {
        return Ok(OutputPlan {
            topology: OutputTopology::DirectSurface,
            canonical,
            routes: vec![ConsumerRoute {
                consumer: consumers[0].id,
                path: OutputPath::DirectSurface,
            }],
        });
    }

    // Retention is a planner decision. A canonical producer descriptor may
    // start as a direct-surface request, but once that route is unavailable
    // the retained topology can satisfy a consumer's persistence requirement
    // without asking the caller to rebuild its descriptor first.
    let retained_canonical = RenderTargetRequirements {
        persistent: true,
        ..canonical
    };
    let mut routes = Vec::with_capacity(consumers.len());
    for consumer in consumers {
        let compatible = retained_canonical.compatible(consumer.requirements);
        let same_device = compatible
            && retained_canonical.usage.contains(GpuTextureUsages::SAMPLED)
            && consumer.capabilities.same_device_sample
            && generation.is_some()
            && consumer.device_generation == generation;
        if same_device {
            routes.push(ConsumerRoute {
                consumer: consumer.id,
                path: OutputPath::SameDeviceSample,
            });
            continue;
        }
        if compatible && consumer.capabilities.native_share && retained_canonical.exportable {
            routes.push(ConsumerRoute {
                consumer: consumer.id,
                path: OutputPath::SharedNativeResource,
            });
            continue;
        }
        if consumer.capabilities.gpu_copy
            && retained_canonical
                .usage
                .contains(GpuTextureUsages::COPY_SRC)
        {
            routes.push(ConsumerRoute {
                consumer: consumer.id,
                path: OutputPath::GpuTransfer,
            });
            continue;
        }
        if consumer.capabilities.cpu_fallback
            && consumer.capabilities.allow_cpu_fallback
            && retained_canonical
                .usage
                .contains(GpuTextureUsages::COPY_SRC)
        {
            routes.push(ConsumerRoute {
                consumer: consumer.id,
                path: OutputPath::CpuFallback,
            });
            continue;
        }
        return Err(if compatible {
            PlanError::NoCapability {
                consumer: consumer.id,
            }
        } else {
            PlanError::IncompatibleRequirements {
                consumer: consumer.id,
            }
        });
    }
    Ok(OutputPlan {
        topology: OutputTopology::CanonicalRetained,
        canonical: retained_canonical,
        routes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> RenderTargetRequirements {
        RenderTargetRequirements::new([640, 480], GpuTextureFormat::RGBA8_UNORM)
    }

    fn window(id: u64) -> OutputConsumer {
        OutputConsumer {
            id: ConsumerId(id),
            kind: ConsumerKind::Window,
            requirements: target(),
            capabilities: PresenterCapabilities {
                direct_surface: true,
                ..Default::default()
            },
            device_generation: None,
        }
    }

    #[test]
    fn single_window_uses_direct_surface() {
        let mut planner = RenderTargetPlanner::new();
        let plan = planner.configure(target(), &[window(1)]).unwrap();
        assert_eq!(plan.topology, OutputTopology::DirectSurface);
        assert_eq!(
            plan.path_for(ConsumerId(1)),
            Some(OutputPath::DirectSurface)
        );
        assert_eq!(planner.rebuilds(), 1);
        assert_eq!(planner.last_work().canonical_target_count, 1);
        assert_eq!(planner.last_work().consumer_count, 1);
        planner.configure(target(), &[window(1)]).unwrap();
        assert_eq!(planner.last_work(), OutputWorkObservation::default());
    }

    #[test]
    fn repeated_configuration_is_cached_and_order_is_stable() {
        let mut planner = RenderTargetPlanner::new();
        let mut second = window(2);
        second.capabilities.direct_surface = false;
        second.capabilities.gpu_copy = true;
        let mut first_consumer = window(1);
        first_consumer.capabilities.gpu_copy = true;
        // Deliberately configure in reverse order. Sorting is part of the key.
        let canonical = target()
            .persistent(true)
            .with_usage(GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC);
        let first = planner
            .configure(canonical, &[second, first_consumer])
            .unwrap();
        assert_eq!(first.consumer_count(), 2);
        assert_eq!(planner.rebuilds(), 1);
        planner
            .configure(canonical, &[first_consumer, second])
            .unwrap();
        assert_eq!(planner.rebuilds(), 1);
    }

    #[test]
    fn cpu_fallback_requires_explicit_opt_in() {
        let mut planner = RenderTargetPlanner::new();
        let mut sink = window(9);
        sink.capabilities = PresenterCapabilities {
            cpu_fallback: true,
            ..Default::default()
        };
        assert!(matches!(
            planner.configure(target().persistent(true), &[sink]),
            Err(PlanError::NoCapability { .. })
        ));
        sink.capabilities.allow_cpu_fallback = true;
        let cpu_target = target()
            .persistent(true)
            .with_usage(GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC);
        let plan = planner.configure(cpu_target, &[sink]).unwrap();
        assert!(plan.uses_cpu_fallback());
    }

    #[test]
    fn attach_and_detach_unknown_are_no_ops() {
        let mut planner = RenderTargetPlanner::new();
        planner.attach_consumer(target(), window(1)).unwrap();
        assert_eq!(planner.rebuilds(), 1);
        planner.detach_consumer(target(), ConsumerId(8)).unwrap();
        assert_eq!(planner.rebuilds(), 1);
        planner
            .detach_consumer(target(), ConsumerId(1))
            .unwrap_err();
        assert_eq!(planner.rebuilds(), 1);
    }

    #[test]
    fn duplicate_consumer_ids_are_rejected() {
        let mut planner = RenderTargetPlanner::new();
        let duplicate = [window(4), window(4)];
        assert_eq!(
            planner.configure(target(), &duplicate),
            Err(PlanError::DuplicateConsumer {
                consumer: ConsumerId(4)
            })
        );
        assert_eq!(planner.rebuilds(), 0);
    }

    #[test]
    fn gpu_transfer_requires_copy_source_usage() {
        let mut planner = RenderTargetPlanner::new();
        let mut sink = window(7);
        sink.capabilities = PresenterCapabilities {
            gpu_copy: true,
            ..Default::default()
        };
        assert!(matches!(
            planner.configure(target().persistent(true), &[sink]),
            Err(PlanError::NoCapability {
                consumer: ConsumerId(7)
            })
        ));
        let canonical = target()
            .persistent(true)
            .with_usage(GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC);
        let plan = planner.configure(canonical, &[sink]).unwrap();
        assert_eq!(plan.path_for(ConsumerId(7)), Some(OutputPath::GpuTransfer));
    }

    #[test]
    fn native_share_requires_an_exportable_canonical_target() {
        let mut planner = RenderTargetPlanner::new();
        let mut sink = window(8);
        sink.capabilities = PresenterCapabilities {
            native_share: true,
            ..Default::default()
        };
        assert!(matches!(
            planner.configure(target().persistent(true), &[sink]),
            Err(PlanError::NoCapability {
                consumer: ConsumerId(8)
            })
        ));

        let canonical = target().persistent(true).exportable(true);
        let plan = planner.configure(canonical, &[sink]).unwrap();
        assert_eq!(
            plan.path_for(ConsumerId(8)),
            Some(OutputPath::SharedNativeResource)
        );
    }

    #[test]
    fn retained_topology_promotes_persistence_for_a_consumer() {
        let mut planner = RenderTargetPlanner::new();
        let mut sink = window(10);
        sink.capabilities = PresenterCapabilities {
            gpu_copy: true,
            ..Default::default()
        };
        sink.requirements = sink.requirements.persistent(true);
        let canonical =
            target().with_usage(GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC);
        let plan = planner.configure(canonical, &[sink]).unwrap();
        assert!(plan.canonical.persistent);
        assert_eq!(plan.path_for(ConsumerId(10)), Some(OutputPath::GpuTransfer));
    }
}
