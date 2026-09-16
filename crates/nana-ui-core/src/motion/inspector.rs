//! DevTools snapshot for one Motion track (Issue #87 §14).
//!
//! Format of [`MotionInspectorEntry::format_summary`] is the public inspector
//! contract. Extra diagnostics stay on [`MotionInspectorEntry::format_diagnostics`].

use std::time::Duration;

use super::{
    descriptor::MotionHandle,
    ir::MotionValue,
    property::{AnimatableProperty, AnimationClass},
};

/// Where the current presentation value is evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionEvaluatorBackend {
    Cpu,
    Gpu,
}

impl MotionEvaluatorBackend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Gpu => "GPU",
        }
    }
}

impl AnimationClass {
    pub fn inspector_label(self) -> &'static str {
        match self {
            Self::Compositor => "Compositor",
            Self::Paint => "Paint",
            Self::Layout => "Layout",
            Self::Discrete => "Discrete",
        }
    }
}

/// One active (or fill-forwards) track as the inspector prints it.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionInspectorEntry {
    pub node: u64,
    pub property: AnimatableProperty,
    pub class: AnimationClass,
    pub evaluator: MotionEvaluatorBackend,
    pub layer: Option<u64>,
    pub layer_promotion_reason: Option<&'static str>,
    pub cpu_fallback_reason: Option<String>,
    pub base: Option<MotionValue>,
    pub presentation: Option<MotionValue>,
    pub next_deadline: Option<Duration>,
    pub gpu_handle: Option<MotionHandle>,
    pub runtime_samples_this_frame: usize,
    pub layout_nodes_from_animation: usize,
    pub style_processed_from_animation: usize,
    pub render_nodes_reextracted_from_animation: usize,
}

impl MotionInspectorEntry {
    /// Issue #87 §14 example block. Tests pin this wording.
    pub fn format_summary(&self) -> String {
        let layer = match self.layer {
            Some(id) => format!("#{id}"),
            None => "none".to_string(),
        };
        format!(
            "Node #{} {}\nClass: {}\nEvaluator: {}\nLayer: {}\nRuntime samples/frame: {}",
            self.node,
            self.property.css_name(),
            self.class.inspector_label(),
            self.evaluator.label(),
            layer,
            self.runtime_samples_this_frame,
        )
    }

    pub fn format_diagnostics(&self) -> String {
        let mut lines = vec![self.format_summary()];
        if let Some(reason) = self.layer_promotion_reason {
            lines.push(format!("Promotion: {reason}"));
        }
        if let Some(reason) = self.cpu_fallback_reason.as_deref() {
            lines.push(format!("CPU fallback: {reason}"));
        }
        if let Some(deadline) = self.next_deadline {
            lines.push(format!("Next deadline: {deadline:?}"));
        }
        if let Some(handle) = self.gpu_handle {
            lines.push(format!(
                "GPU handle: index={} generation={}",
                handle.index(),
                handle.generation()
            ));
        }
        lines.push(format!(
            "Impact: layout={} style={} extract={}",
            self.layout_nodes_from_animation,
            self.style_processed_from_animation,
            self.render_nodes_reextracted_from_animation,
        ));
        lines.join("\n")
    }
}

pub fn cpu_fallback_reason(property: AnimatableProperty, has_gpu: bool) -> Option<String> {
    match property.animation_class() {
        AnimationClass::Compositor if has_gpu => None,
        AnimationClass::Compositor => {
            Some("CPU evaluator: compositor track has no live GPU MotionDescriptor.".to_string())
        }
        AnimationClass::Paint | AnimationClass::Layout | AnimationClass::Discrete => {
            Some(property.diagnostic_hint())
        }
    }
}

/// Compositor / motion work observed for one sample or frame (Issue #87 / #8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MotionWorkCounters {
    pub motion_tracks_active: usize,
    pub motion_tracks_cpu: usize,
    pub motion_tracks_compositor: usize,
    pub motion_descriptors_uploaded: usize,
    pub motion_descriptor_bytes_uploaded: usize,
    pub presentation_values_cpu_sampled: usize,
    pub compositor_layers_active: usize,
    pub compositor_layers_promoted: usize,
    pub compositor_layers_demoted: usize,
    pub compositor_cache_bytes: usize,
    pub uiworld_mutations_from_animation: usize,
    pub layout_nodes_from_animation: usize,
    pub style_processed_from_animation: usize,
    pub render_nodes_reextracted_from_animation: usize,
}

impl MotionWorkCounters {
    /// Classify compiled tracks. Does not record GPU uploads (those stay
    /// omitted until a renderer observes them).
    pub fn from_tracks<'a, I>(tracks: I) -> Self
    where
        I: IntoIterator<Item = &'a super::MotionTrack>,
    {
        let mut counters = Self::default();
        for track in tracks {
            counters.motion_tracks_active = counters.motion_tracks_active.saturating_add(1);
            match track.execution_class() {
                super::AnimationClass::Compositor => {
                    counters.motion_tracks_compositor =
                        counters.motion_tracks_compositor.saturating_add(1);
                }
                super::AnimationClass::Paint
                | super::AnimationClass::Layout
                | super::AnimationClass::Discrete => {
                    counters.motion_tracks_cpu = counters.motion_tracks_cpu.saturating_add(1);
                }
            }
        }
        counters
    }

    pub fn record_cpu_presentation_sample(&mut self) {
        self.presentation_values_cpu_sampled =
            self.presentation_values_cpu_sampled.saturating_add(1);
    }

    /// GPU upload observed by a renderer. Steady timestamp frames must not
    /// call this for the full descriptor table.
    pub fn record_descriptor_upload(&mut self, descriptors: usize, bytes: usize) {
        self.motion_descriptors_uploaded =
            self.motion_descriptors_uploaded.saturating_add(descriptors);
        self.motion_descriptor_bytes_uploaded =
            self.motion_descriptor_bytes_uploaded.saturating_add(bytes);
    }

    /// Fold a scene/GPU observation into a Runtime snapshot. Live counts take
    /// the max; per-frame events add.
    pub fn merge(&mut self, other: Self) {
        self.motion_tracks_active = self.motion_tracks_active.max(other.motion_tracks_active);
        self.motion_tracks_cpu = self.motion_tracks_cpu.max(other.motion_tracks_cpu);
        self.motion_tracks_compositor = self
            .motion_tracks_compositor
            .max(other.motion_tracks_compositor);
        self.motion_descriptors_uploaded = self
            .motion_descriptors_uploaded
            .saturating_add(other.motion_descriptors_uploaded);
        self.motion_descriptor_bytes_uploaded = self
            .motion_descriptor_bytes_uploaded
            .saturating_add(other.motion_descriptor_bytes_uploaded);
        self.presentation_values_cpu_sampled = self
            .presentation_values_cpu_sampled
            .saturating_add(other.presentation_values_cpu_sampled);
        self.compositor_layers_active = self
            .compositor_layers_active
            .max(other.compositor_layers_active);
        self.compositor_layers_promoted = self
            .compositor_layers_promoted
            .saturating_add(other.compositor_layers_promoted);
        self.compositor_layers_demoted = self
            .compositor_layers_demoted
            .saturating_add(other.compositor_layers_demoted);
        self.compositor_cache_bytes = self
            .compositor_cache_bytes
            .max(other.compositor_cache_bytes);
        self.uiworld_mutations_from_animation = self
            .uiworld_mutations_from_animation
            .saturating_add(other.uiworld_mutations_from_animation);
        self.layout_nodes_from_animation = self
            .layout_nodes_from_animation
            .saturating_add(other.layout_nodes_from_animation);
        self.style_processed_from_animation = self
            .style_processed_from_animation
            .saturating_add(other.style_processed_from_animation);
        self.render_nodes_reextracted_from_animation = self
            .render_nodes_reextracted_from_animation
            .saturating_add(other.render_nodes_reextracted_from_animation);
    }

    /// Issue #87 §13 compositor-only steady frame: no CPU Runtime work.
    /// `presentation_values_cpu_sampled` must be the **frame delta**, not the
    /// world's cumulative query count.
    pub fn compositor_steady_is_quiet(self) -> bool {
        self.uiworld_mutations_from_animation == 0
            && self.layout_nodes_from_animation == 0
            && self.style_processed_from_animation == 0
            && self.render_nodes_reextracted_from_animation == 0
            && self.presentation_values_cpu_sampled == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_matches_issue_example() {
        let entry = MotionInspectorEntry {
            node: 123,
            property: AnimatableProperty::Transform,
            class: AnimationClass::Compositor,
            evaluator: MotionEvaluatorBackend::Gpu,
            layer: Some(7),
            layer_promotion_reason: Some("transform/opacity presentation"),
            cpu_fallback_reason: None,
            base: None,
            presentation: None,
            next_deadline: None,
            gpu_handle: None,
            runtime_samples_this_frame: 0,
            layout_nodes_from_animation: 0,
            style_processed_from_animation: 0,
            render_nodes_reextracted_from_animation: 0,
        };
        assert_eq!(
            entry.format_summary(),
            "Node #123 transform\nClass: Compositor\nEvaluator: GPU\nLayer: #7\nRuntime samples/frame: 0"
        );
    }

    #[test]
    fn compositor_and_layout_tracks_split_cpu_from_compositor() {
        use crate::motion::{
            AnimationPlayback, MotionCurve, MotionTargetId, MotionTiming, MotionTo, MotionTrack,
            MotionTrackId, MotionValue,
        };
        use std::time::Duration;

        let compositor = MotionTrack::transition(
            MotionTrackId::new(1).unwrap(),
            MotionTargetId::new(1).unwrap(),
            AnimatableProperty::Transform,
            MotionValue::Scalar(0.0),
            MotionValue::Scalar(1.0),
            MotionTiming::new(
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
            ),
            MotionCurve::Easing(crate::motion::Easing::Linear),
            AnimationPlayback::default(),
        );
        let layout = MotionTrack::transition(
            MotionTrackId::new(2).unwrap(),
            MotionTargetId::new(1).unwrap(),
            AnimatableProperty::Width,
            MotionValue::Scalar(10.0),
            MotionValue::Scalar(20.0),
            MotionTiming::new(
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
            ),
            MotionCurve::Easing(crate::motion::Easing::Linear),
            AnimationPlayback::default(),
        );
        assert_eq!(layout.to, MotionTo::Value(MotionValue::Scalar(20.0)));
        let counts = MotionWorkCounters::from_tracks([&compositor, &layout]);
        assert_eq!(counts.motion_tracks_active, 2);
        assert_eq!(counts.motion_tracks_compositor, 1);
        assert_eq!(counts.motion_tracks_cpu, 1);
        assert_eq!(counts.motion_descriptors_uploaded, 0);
    }
}
