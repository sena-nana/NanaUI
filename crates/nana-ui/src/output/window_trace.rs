//! Opt-in WindowOutputState accounting. Fixed slots avoid frame-path storage
//! allocation and keep source rendering distinct from consumer processing.

use nana_gpu::{DeviceGeneration, GpuTextureFormat};

use super::trace::{
    OutputRecreateReason, OutputTraceRole, OutputTraceSnapshot, OutputTraceWait,
    OutputWorkObservation,
};
use super::{OutputConsumerId, OutputPath};

pub(super) const PRODUCER: usize = 0;
pub(super) const WINDOW: usize = 1;
pub(super) const TRANSPORT: usize = 2;

#[derive(Default)]
pub(super) struct WindowTrace {
    pub snapshots: [Option<OutputTraceSnapshot>; 3],
    pub direct_owns_submission: bool,
    pub host_waits: Option<OutputTraceWait>,
    native_transport: bool,
}

impl WindowTrace {
    #[allow(clippy::too_many_arguments)]
    pub fn ensure_source(
        &mut self,
        producer: u64,
        generation: DeviceGeneration,
        resource: u64,
        extent: [u32; 2],
        format: GpuTextureFormat,
        recreated: usize,
        reason: OutputRecreateReason,
    ) {
        if self.snapshots[PRODUCER].is_none_or(|old| old.producer != producer) {
            self.native_transport = false;
        }
        for index in [PRODUCER, WINDOW, TRANSPORT] {
            let role = if index == PRODUCER {
                OutputTraceRole::Producer
            } else {
                OutputTraceRole::Consumer
            };
            let consumer = OutputConsumerId(u64::from(index == WINDOW));
            if self.snapshots[index].is_none_or(|old| old.producer != producer) {
                self.snapshots[index] = Some(OutputTraceSnapshot::new(
                    producer,
                    consumer,
                    role,
                    OutputPath::SharedTexture,
                    format,
                    extent,
                    generation,
                ));
            }
            let snapshot = self.snapshots[index].as_mut().expect("source initialized");
            snapshot.device_generation = generation;
            if index != TRANSPORT || !self.native_transport {
                snapshot.resource_generation = resource;
            }
            if index != WINDOW && (index != TRANSPORT || !self.native_transport) {
                snapshot.extent = extent;
                snapshot.format = format;
            }
        }
        if recreated != 0 {
            self.observe(
                PRODUCER,
                OutputWorkObservation {
                    target_recreates: recreated,
                    ..Default::default()
                },
            );
            self.reason(PRODUCER, reason);
        }
    }

    pub fn observe(&mut self, index: usize, work: OutputWorkObservation) {
        if let Some(snapshot) = &mut self.snapshots[index] {
            snapshot.observe_without_metrics(work, None);
        }
    }

    pub fn reason(&mut self, index: usize, reason: OutputRecreateReason) {
        if let Some(snapshot) = &mut self.snapshots[index] {
            snapshot.recreate_reason = reason;
        }
    }

    pub fn versions(&mut self, resource: u64, content: u64) {
        for (index, snapshot) in self.snapshots.iter_mut().enumerate() {
            if let Some(snapshot) = snapshot {
                if index != TRANSPORT || !self.native_transport {
                    snapshot.resource_generation = resource;
                }
                snapshot.content_revision = content;
            }
        }
    }

    #[cfg(any(test, all(windows, feature = "native-export")))]
    pub fn native_target(&mut self, resource: u64, extent: [u32; 2], format: GpuTextureFormat) {
        if let Some(snapshot) = &mut self.snapshots[TRANSPORT] {
            self.native_transport = true;
            snapshot.resource_generation = resource;
            snapshot.extent = extent;
            snapshot.format = format;
        }
    }

    pub fn clear_native_target(&mut self) {
        self.native_transport = false;
    }

    pub fn transfer(&mut self, index: usize) {
        if let Some(snapshot) = &mut self.snapshots[index] {
            snapshot.path = OutputPath::GpuTransfer;
        }
    }

    pub fn restore_host_waits(&mut self) {
        if let Some(producer) = &mut self.snapshots[PRODUCER] {
            producer.waits = if self.direct_owns_submission {
                None
            } else {
                self.host_waits
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_pool_identity_survives_source_updates_and_changes_independently() {
        let mut trace = WindowTrace::default();
        let generation = crate::test_gpu::context().generation();
        trace.ensure_source(
            7,
            generation,
            1,
            [32, 24],
            GpuTextureFormat::RGBA8_UNORM,
            0,
            OutputRecreateReason::None,
        );
        trace.native_target(90, [32, 24], GpuTextureFormat::BGRA8_UNORM);
        trace.versions(2, 11);
        trace.ensure_source(
            7,
            generation,
            2,
            [32, 24],
            GpuTextureFormat::RGBA8_UNORM,
            0,
            OutputRecreateReason::None,
        );
        assert_eq!(trace.snapshots[PRODUCER].unwrap().resource_generation, 2);
        assert_eq!(trace.snapshots[TRANSPORT].unwrap().resource_generation, 90);
        assert_eq!(
            trace.snapshots[TRANSPORT].unwrap().format,
            GpuTextureFormat::BGRA8_UNORM
        );
        trace.native_target(91, [32, 24], GpuTextureFormat::BGRA8_UNORM);
        trace.versions(2, 12);
        assert_eq!(trace.snapshots[PRODUCER].unwrap().resource_generation, 2);
        assert_eq!(trace.snapshots[TRANSPORT].unwrap().resource_generation, 91);
        assert_eq!(trace.snapshots[TRANSPORT].unwrap().content_revision, 12);
        // Leaving Native export restores same-device target metadata, while a
        // temporarily absent native pool alone must never reset the identity.
        trace.clear_native_target();
        trace.ensure_source(
            7,
            generation,
            2,
            [32, 24],
            GpuTextureFormat::RGBA8_UNORM,
            0,
            OutputRecreateReason::None,
        );
        trace.versions(2, 13);
        assert_eq!(trace.snapshots[TRANSPORT].unwrap().resource_generation, 2);
        assert_eq!(
            trace.snapshots[TRANSPORT].unwrap().format,
            GpuTextureFormat::RGBA8_UNORM
        );
        assert_eq!(trace.snapshots[TRANSPORT].unwrap().content_revision, 13);
    }

    #[test]
    fn completion_does_not_erase_host_waits_or_attribute_them_to_a_consumer() {
        let mut trace = WindowTrace::default();
        trace.ensure_source(
            1,
            crate::test_gpu::context().generation(),
            1,
            [1, 1],
            GpuTextureFormat::BGRA8_UNORM,
            0,
            OutputRecreateReason::None,
        );
        trace.host_waits = Some(OutputTraceWait {
            frame_slot_waits: 2,
            frame_slot_stalls: 3,
        });
        trace.observe(
            PRODUCER,
            OutputWorkObservation {
                submissions: 1,
                ..Default::default()
            },
        );
        trace.restore_host_waits();
        assert_eq!(trace.snapshots[PRODUCER].unwrap().waits, trace.host_waits);
        assert_eq!(trace.snapshots[WINDOW].unwrap().waits, None);
        assert_eq!(trace.snapshots[TRANSPORT].unwrap().waits, None);
        trace.direct_owns_submission = true;
        trace.restore_host_waits();
        assert_eq!(trace.snapshots[PRODUCER].unwrap().waits, None);
    }
}
