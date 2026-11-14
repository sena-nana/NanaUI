//! Check the emitted metric atoms as well as the driver's local work report.

use super::*;
use nana_diagnostics::framework::gpu::{OUTPUT_EXTRA_PASSES, OUTPUT_GPU_CONVERT_PASSES};
use nana_diagnostics::{DiagnosticsConfig, DiagnosticsPaths, PersistMode, SessionMetadata};

#[test]
fn recorded_transfers_emit_diagnostics_even_when_the_consumer_frame_is_discarded() {
    const CHILD: &str = "NANA_UI_TEST_OUTPUT_DIAGNOSTICS_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // Installing global diagnostics must not enable instrumentation in
        // unrelated, concurrently executing GPU tests.
        let _guard = test_gpu::lock();
        test_gpu::run_child_with_timeout(
            "output::driver_tests::diagnostics::recorded_transfers_emit_diagnostics_even_when_the_consumer_frame_is_discarded",
            CHILD,
            "1",
        );
        return;
    }

    let guard = nana_diagnostics::install(
        DiagnosticsConfig {
            metrics: true,
            persist: PersistMode::Off,
            panic_hook: false,
            ..Default::default()
        },
        SessionMetadata::new("dev.nana.output-test", "Output diagnostics fixture", "0"),
        DiagnosticsPaths::in_memory(),
    )
    .expect("isolated process owns the diagnostics instance");
    assert!(nana_diagnostics::metrics_enabled());
    let gpu = test_gpu::context();
    let mut driver = PresentationDriver::new(&gpu);
    driver.configure(&[consumer(1)]).unwrap();
    let scene = UiScene::new();
    let outcomes = render_fresh(&mut driver, &scene);
    assert!(matches!(
        outcomes[0],
        ExternalRenderOutcome::Submitted { .. }
    ));
    let source = completed(&mut driver, OutputConsumerId(1));
    let destination = gpu
        .create_texture(&GpuTextureDescriptor {
            label: Some("output diagnostics consumer destination"),
            width: EXTENT[0],
            height: EXTENT[1],
            format: source.texture().unwrap().format(),
            usage: GpuTextureUsages::RENDER_TARGET,
        })
        .unwrap();
    let extra_before = OUTPUT_EXTRA_PASSES.value();
    let converts_before = OUTPUT_GPU_CONVERT_PASSES.value();
    assert_eq!(driver.work().output_extra_passes, Some(0));
    assert_eq!(driver.work().output_gpu_convert_passes, Some(0));
    let mut consuming = gpu.begin_frame("output diagnostics recorded transfer");
    driver
        .present_to(
            OutputConsumerId(1),
            &mut consuming,
            &destination.render_target().unwrap(),
        )
        .unwrap();
    assert_eq!(OUTPUT_EXTRA_PASSES.value() - extra_before, 1);
    assert_eq!(OUTPUT_GPU_CONVERT_PASSES.value() - converts_before, 1);
    assert_eq!(driver.work().output_extra_passes, Some(1));
    assert_eq!(driver.work().output_gpu_convert_passes, Some(1));
    consuming.discard();
    // These are encoded-work counters, not a claim that the consumer
    // submitted or executed. Discard must not rewrite real work to zero.
    assert_eq!(OUTPUT_EXTRA_PASSES.value() - extra_before, 1);
    assert_eq!(OUTPUT_GPU_CONVERT_PASSES.value() - converts_before, 1);
    assert_eq!(driver.work().output_extra_passes, Some(1));
    assert_eq!(driver.work().output_gpu_convert_passes, Some(1));
    drop(guard);
}
