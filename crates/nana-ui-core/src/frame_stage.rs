//! Named pipeline stages for frame timing (`FrameProfiler`).

/// Named CPU/GPU frame stages. Runtime times the stages it owns;
/// `Batch`, `GpuUpload`, `Encode`, and `Submit` stay `runtime_unsupported`
/// until a GPU host that encodes and submits times them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameStage {
    Input,
    Reconcile,
    Style,
    TextShape,
    Layout,
    HitTest,
    Accessibility,
    Animation,
    Extract,
    Batch,
    GpuUpload,
    Encode,
    Submit,
}

impl FrameStage {
    pub const ALL: [Self; 13] = [
        Self::Input,
        Self::Reconcile,
        Self::Style,
        Self::TextShape,
        Self::Layout,
        Self::HitTest,
        Self::Accessibility,
        Self::Animation,
        Self::Extract,
        Self::Batch,
        Self::GpuUpload,
        Self::Encode,
        Self::Submit,
    ];

    /// Stages the retained Runtime does not own. A profiler reports these
    /// as unsupported with zero duration rather than pretending they ran.
    pub const fn runtime_unsupported(self) -> bool {
        self.gpu_host_owned()
    }

    /// Stages a Scene/WGPU host owns: `Batch`, `GpuUpload`, `Encode`, `Submit`.
    /// Measurable only when that host actually encodes and submits.
    pub const fn gpu_host_owned(self) -> bool {
        matches!(
            self,
            Self::Batch | Self::GpuUpload | Self::Encode | Self::Submit
        )
    }
}

#[cfg(test)]
mod tests {
    use super::FrameStage;

    #[test]
    fn gpu_stages_match_runtime_unsupported() {
        assert_eq!(FrameStage::ALL.len(), 13);
        for stage in FrameStage::ALL {
            assert_eq!(
                stage.runtime_unsupported(),
                matches!(
                    stage,
                    FrameStage::Batch
                        | FrameStage::GpuUpload
                        | FrameStage::Encode
                        | FrameStage::Submit
                )
            );
        }
    }
}
