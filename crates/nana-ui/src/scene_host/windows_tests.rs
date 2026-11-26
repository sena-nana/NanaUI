//! Unit tests for `scene_host::windows` appearance caching, auxiliary levels,
//! live-resize coalescing, and drag policy.

use super::*;

#[test]
fn unchanged_windows_do_not_reapply_native_material() {
    let original = WindowAppearance {
        theme: crate::ThemeAppearance::Dark,
        material: nana_window::MaterialEffect::Solid,
        opacity: 1.0,
        background: None,
    };
    let mut windows = [Some(original); 3];
    let changed = WindowAppearance {
        opacity: 0.5,
        ..original
    };
    let mut calls = Vec::new();
    for (index, cache) in windows.iter_mut().enumerate() {
        let desired = if index == 1 { changed } else { original };
        let result = apply_changed_appearance(cache, desired, || {
            calls.push(index);
            Ok(MaterialOutcome::chosen_solid())
        })
        .unwrap();
        assert_eq!(result.is_some(), index == 1);
    }
    assert_eq!(calls, vec![1]);
    assert_eq!(windows[1], Some(changed));
    assert!(
        apply_changed_appearance::<MaterialOutcome>(&mut windows[1], changed, || panic!(
            "unchanged material was reapplied"
        ))
        .unwrap()
        .is_none()
    );
    assert!(
        apply_changed_appearance(&mut windows[1], original, || Err::<MaterialOutcome, _>(
            HostedGpuError::SurfaceValidation
        ))
        .is_err()
    );
    assert_eq!(windows[1], Some(changed));
}

#[test]
fn theme_changes_and_device_replacement_invalidate_appearance() {
    let original = WindowAppearance {
        theme: crate::ThemeAppearance::Dark,
        material: nana_window::MaterialEffect::Solid,
        opacity: 1.0,
        background: None,
    };
    let mut cached = Some(original);
    let light = WindowAppearance {
        theme: crate::ThemeAppearance::Light,
        ..original
    };
    let calls = std::cell::Cell::new(0);
    let apply = |cache: &mut Option<WindowAppearance>, desired| {
        apply_changed_appearance(cache, desired, || {
            calls.set(calls.get() + 1);
            Err::<MaterialOutcome, _>(HostedGpuError::SurfaceValidation)
        })
    };
    assert!(apply(&mut cached, light).is_err());
    cached = None;
    assert!(apply(&mut cached, original).is_err());
    assert_eq!(calls.get(), 2);
}

#[test]
fn auxiliary_inherits_current_owner_band_without_latching_topmost() {
    use WindowLevel::{AlwaysOnBottom, AlwaysOnTop, Normal};
    for requested in [AlwaysOnBottom, Normal, AlwaysOnTop] {
        assert_eq!(auxiliary_level(requested, Some(AlwaysOnTop)), AlwaysOnTop);
        assert_eq!(auxiliary_level(requested, Some(Normal)), requested);
        assert_eq!(auxiliary_level(requested, None), requested);
    }
    let child = Normal;
    let owner = [Normal, AlwaysOnTop, Normal, AlwaysOnTop, Normal];
    assert_eq!(
        owner.map(|level| auxiliary_level(child, Some(level))),
        owner
    );
}

#[test]
#[should_panic(expected = "input endpoint generation exhausted")]
fn input_generation_exhaustion_does_not_reuse_the_last_identity() {
    next_input_generation(u64::MAX);
}

#[test]
fn input_generation_advances_without_zero_wrap() {
    assert_eq!(next_input_generation(0), 1);
    assert_eq!(next_input_generation(u64::MAX - 1), u64::MAX);
}

#[test]
fn ignored_request_is_an_operation_failure_not_a_missing_capability() {
    assert!(matches!(
        window_request_error(winit::error::RequestError::Ignored),
        crate::WindowError::OperationFailed(_)
    ));
    assert!(matches!(
        window_request_error(winit::error::RequestError::NotSupported(
            winit::error::NotSupportedError::new("fixture capability")
        )),
        crate::WindowError::Unsupported(_)
    ));
}

#[test]
fn the_first_step_commits_and_the_interval_starts_at_the_present() {
    let mut clock = Instant::now();
    let state = LiveResizeCoalescer::new();
    let (state, step) = state.on_step(clock);
    assert_eq!(step, LiveResizeStep::Commit);
    assert!(!state.finish());

    clock += Duration::from_millis(40);
    let state = state.on_presented(clock);
    let (state, step) = state.on_step(clock);
    assert_eq!(step, LiveResizeStep::Defer);
    assert!(state.finish());

    let (state, step) = state.on_step(clock + Duration::from_millis(15));
    assert_eq!(step, LiveResizeStep::Defer);
    assert!(state.finish());

    let (state, step) = state.on_step(clock + LIVE_RESIZE_COMMIT_INTERVAL);
    assert_eq!(step, LiveResizeStep::Commit);
    let state = state.on_presented(clock + LIVE_RESIZE_COMMIT_INTERVAL);
    assert!(!state.finish());
}

#[test]
fn a_gesture_that_only_deferred_still_owes_a_final_commit() {
    let start = Instant::now();
    let state = LiveResizeCoalescer::new().on_presented(start);
    let (state, step) = state.on_step(start + Duration::from_millis(1));
    assert_eq!(step, LiveResizeStep::Defer);
    assert!(state.finish());
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn host_managed_drag_overrides_the_primary_button_native_default() {
    let native = WindowDescriptor::new("native");
    assert!(!uses_host_managed_drag(
        &native,
        crate::winit_input::PRIMARY_MOUSE_BUTTON,
        false
    ));
    assert!(uses_host_managed_drag(&native, 1, false));

    let managed = WindowDescriptor::new("managed").host_managed_drag(true);
    assert!(uses_host_managed_drag(
        &managed,
        crate::winit_input::PRIMARY_MOUSE_BUTTON,
        false
    ));
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn independent_underlays_use_the_same_input_turn_for_macos_movement() {
    let native = WindowDescriptor::new("native");
    assert_eq!(
        uses_host_managed_drag(&native, crate::winit_input::PRIMARY_MOUSE_BUTTON, true),
        cfg!(target_os = "macos")
    );
    assert!(!uses_host_managed_drag(
        &native,
        crate::winit_input::PRIMARY_MOUSE_BUTTON,
        false
    ));
}

#[test]
fn uses_continuous_swapchain_latency_honors_continuous_demand_or_external_presentation() {
    use std::num::NonZeroU32;

    assert!(!uses_continuous_swapchain_latency(
        crate::FrameDemand::OnDemand,
        false
    ));
    assert!(uses_continuous_swapchain_latency(
        crate::FrameDemand::OnDemand,
        true
    ));
    assert!(uses_continuous_swapchain_latency(
        crate::FrameDemand::Continuous(NonZeroU32::new(60).unwrap()),
        false
    ));
    assert!(uses_continuous_swapchain_latency(
        crate::FrameDemand::At(Instant::now()),
        true
    ));
}
