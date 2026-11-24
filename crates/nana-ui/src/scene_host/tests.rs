//! Unit tests for `scene_host` lifecycle, chrome, accessibility, image index,
//! command routing, and Forward passthrough.

#[cfg(not(target_os = "macos"))]
use super::desktop_scale;
#[cfg(not(target_os = "android"))]
use super::next_accessibility_update;
use super::{
    Desktop, DisplayBounds, ForwardPointerAction, RoutedWindowCommand, allows_modal_parent_event,
    desktop_position, forward_pointer_action, frame_edge_cursor, frame_resize_edge_for,
    invalidate_program_host_textures, next_bootstrap_attempt, remove_image_target_index,
    replace_image_target_index, resolved_scene_icon, route_window_command, scene_clear_color,
    scene_runtime_input_update, scene_window_attributes, window_cursor_override, window_level,
    window_surface_effect, window_wants_transparent_surface, windows_scene_chrome,
    windows_to_redraw, winit_icon,
};
use crate::presentation::{ResolvedSurfaceTarget, ResolvedWindowPresentation, WindowSurfaceTarget};
use crate::winit_input::{ScreenSpace, WinitInputAdapter, screen_position};
use crate::{
    HostTexture, HostTextureAlphaMode, HostTextureRegistry, MaterialEffect, MaterialFallback,
    MaterialOutcome, RuntimeProgramUpdate, RuntimeRedraw, ThemeAppearance,
};
use nana_ui_platform::host::WindowCommand;
use nana_ui_platform::{
    InputDisposition, MousePassthroughMode, WindowDescriptor, WindowGeometry, WindowIcon, WindowId,
    WindowResizeEdge,
};
#[cfg(not(target_os = "android"))]
use nana_ui_runtime::{AccessibilityDelta, AccessibilityUpdate, FrameworkError};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use winit::dpi::PhysicalPosition;
use winit::event::{
    ButtonSource, ElementState, MouseButton, PointerKind, PointerSource,
    WindowEvent as WinitWindowEvent,
};

#[test]
fn a_blocked_parent_still_hears_it_lost_focus() {
    assert!(allows_modal_parent_event(&WinitWindowEvent::Focused(false)));
    assert!(!allows_modal_parent_event(&WinitWindowEvent::Focused(true)));
    assert!(!allows_modal_parent_event(
        &WinitWindowEvent::CloseRequested
    ));
}

#[test]
fn empty_flush_reprojects_when_the_program_already_drained_runtime_work() {
    let Some(AccessibilityUpdate::Full { generation, nodes }) =
        next_accessibility_update(None, None, false, None, Some(3), Vec::new)
    else {
        panic!("drained SystemWork must still reach AccessKit from the world");
    };
    assert_eq!(generation, Some(3));
    assert!(nodes.is_empty());
}

#[cfg(not(target_os = "android"))]
#[test]
fn matching_generations_do_not_rebuild_an_idle_tree() {
    assert!(
        next_accessibility_update(None, None, false, Some(3), Some(3), || panic!(
            "idle frames must not snapshot"
        ))
        .is_none()
    );
}

#[cfg(not(target_os = "android"))]
#[test]
fn program_accessibility_queue_is_the_host_source_when_flush_is_empty() {
    let queued = AccessibilityUpdate::Delta(AccessibilityDelta {
        generation: 2,
        updated: Vec::new(),
        removed: Vec::new(),
    });
    assert_eq!(
        next_accessibility_update(
            None,
            Some(queued.clone()),
            false,
            Some(1),
            Some(2),
            || panic!("queued deltas must not force a world snapshot"),
        ),
        Some(queued)
    );
}

#[cfg(not(target_os = "android"))]
#[test]
fn scale_change_reprojects_even_when_generations_match() {
    let Some(AccessibilityUpdate::Full { generation, .. }) =
        next_accessibility_update(None, None, true, Some(1), Some(1), Vec::new)
    else {
        panic!("DPI change must reproject the current world");
    };
    assert_eq!(generation, Some(1));
}

#[cfg(not(target_os = "android"))]
#[test]
fn flush_and_program_deltas_reproject_the_world_instead_of_dropping_one() {
    let flush = AccessibilityUpdate::Delta(AccessibilityDelta {
        generation: 3,
        updated: Vec::new(),
        removed: Vec::new(),
    });
    let program = AccessibilityUpdate::Delta(AccessibilityDelta {
        generation: 2,
        updated: Vec::new(),
        removed: Vec::new(),
    });
    let Some(AccessibilityUpdate::Full { generation, .. }) = next_accessibility_update(
        Some(flush),
        Some(program),
        false,
        Some(1),
        Some(3),
        Vec::new,
    ) else {
        panic!("two accessibility sources must not drop hierarchy for layout");
    };
    assert_eq!(generation, Some(3));
}

#[cfg(not(target_os = "android"))]
#[test]
fn stale_program_queue_yields_to_the_current_world_snapshot() {
    let queued = AccessibilityUpdate::Delta(AccessibilityDelta {
        generation: 1,
        updated: Vec::new(),
        removed: Vec::new(),
    });
    let Some(AccessibilityUpdate::Full { generation, .. }) =
        next_accessibility_update(None, Some(queued), false, Some(1), Some(3), Vec::new)
    else {
        panic!("stale queued delta must reproject AccessKit from the world");
    };
    assert_eq!(generation, Some(3));
}

#[test]
fn scene_windows_use_client_chrome_and_runtime_settings() {
    let mut settings = WindowDescriptor::new("Scene");
    settings.transparent = true;
    settings.always_on_top = true;
    settings.resizable = false;
    settings.maximized = true;
    settings.initial_size = (640.0, 480.0);
    settings.minimum_size = (320.0, 240.0);
    let attributes = scene_window_attributes(
        &settings,
        &Desktop {
            displays: Vec::new(),
            size_ratios: Vec::new(),
            scale: 1.0,
        },
        WindowSurfaceTarget::NativeWindow,
    );

    assert_eq!(attributes.title, "Scene");
    #[cfg(target_os = "macos")]
    assert!(attributes.decorations);
    #[cfg(not(target_os = "macos"))]
    assert!(!attributes.decorations);
    assert!(attributes.transparent);
    assert!(attributes.maximized);
    assert!(!attributes.resizable);
    assert_eq!(
        attributes.window_level,
        winit::window::WindowLevel::AlwaysOnTop
    );
    assert_eq!(window_level(false), winit::window::WindowLevel::Normal);

    let transparent_client = windows_scene_chrome(
        settings.system_caption,
        settings.transparent,
        WindowSurfaceTarget::NativeWindow,
    );
    assert!(!transparent_client.decorations);
    assert!(!transparent_client.undecorated_shadow);
    assert!(!transparent_client.no_redirection_bitmap);
    assert!(!transparent_client.rounded_corners);

    let opaque_client = windows_scene_chrome(false, false, WindowSurfaceTarget::NativeWindow);
    assert!(!opaque_client.decorations);
    assert!(!opaque_client.undecorated_shadow);
    assert!(!opaque_client.no_redirection_bitmap);
    assert!(opaque_client.rounded_corners);

    settings.system_caption = true;
    let caption = scene_window_attributes(
        &settings,
        &Desktop {
            displays: Vec::new(),
            size_ratios: Vec::new(),
            scale: 1.0,
        },
        WindowSurfaceTarget::NativeWindow,
    );
    assert!(caption.decorations);
    let transparent_caption = windows_scene_chrome(true, true, WindowSurfaceTarget::NativeWindow);
    assert!(transparent_caption.decorations);
    assert!(!transparent_caption.undecorated_shadow);
    assert!(!transparent_caption.no_redirection_bitmap);
    let opaque_caption = windows_scene_chrome(true, false, WindowSurfaceTarget::NativeWindow);
    assert!(opaque_caption.decorations);
    assert!(!opaque_caption.no_redirection_bitmap);
}

#[test]
fn only_a_composed_window_is_created_without_a_redirection_bitmap() {
    for system_caption in [false, true] {
        for transparent in [false, true] {
            assert!(
                !windows_scene_chrome(
                    system_caption,
                    transparent,
                    WindowSurfaceTarget::NativeWindow
                )
                .no_redirection_bitmap
            );
            assert!(
                windows_scene_chrome(
                    system_caption,
                    transparent,
                    WindowSurfaceTarget::Composition
                )
                .no_redirection_bitmap
            );
        }
    }
}

fn resolved(
    settings: &WindowDescriptor,
    requested: MaterialEffect,
    alpha: wgpu::CompositeAlphaMode,
    backend: wgpu::Backend,
) -> ResolvedWindowPresentation {
    let applied = match requested {
        MaterialEffect::Transparent => MaterialOutcome::transparent(),
        MaterialEffect::Solid => MaterialOutcome::chosen_solid(),
        effect => MaterialOutcome::native(effect),
    };
    ResolvedWindowPresentation::resolve(
        settings,
        requested,
        applied,
        alpha,
        backend,
        ResolvedSurfaceTarget::honoured(WindowSurfaceTarget::NativeWindow, false),
        nana_window::NonClientRenderingStrategy::StripFrameStyles,
    )
}

#[test]
fn client_chrome_follows_the_live_surface_not_the_descriptor() {
    use crate::presentation::NativeChromePolicy;
    let mut settings = WindowDescriptor::new("Scene");
    assert!(!settings.transparent);

    let transparent = resolved(
        &settings,
        MaterialEffect::Transparent,
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::Backend::Vulkan,
    );
    assert_eq!(
        transparent.chrome(),
        Some(NativeChromePolicy::transparent(
            nana_window::NonClientRenderingStrategy::StripFrameStyles,
        ))
    );

    let opaque = resolved(
        &settings,
        MaterialEffect::Solid,
        wgpu::CompositeAlphaMode::Opaque,
        wgpu::Backend::Vulkan,
    );
    assert_eq!(opaque.chrome(), Some(NativeChromePolicy::OPAQUE));

    settings.system_caption = true;
    assert_eq!(
        resolved(
            &settings,
            MaterialEffect::Transparent,
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::Backend::Vulkan,
        )
        .chrome(),
        None
    );
}

#[test]
fn a_failed_composed_attempt_falls_back_to_the_plain_path_and_stops_there() {
    use crate::presentation::SurfaceTargetFallback;

    let composed = ResolvedSurfaceTarget::honoured(WindowSurfaceTarget::Composition, false);
    let next = next_bootstrap_attempt(composed).expect("a composed attempt has a fallback");
    assert_eq!(next.requested, WindowSurfaceTarget::Composition);
    assert_eq!(next.resolved, WindowSurfaceTarget::NativeWindow);
    assert_eq!(
        next.fallback,
        Some(SurfaceTargetFallback::TargetUnavailable)
    );
    assert_eq!(next_bootstrap_attempt(next), None);
    assert_eq!(
        next_bootstrap_attempt(ResolvedSurfaceTarget::honoured(
            WindowSurfaceTarget::NativeWindow,
            false
        )),
        None
    );
    let no_backend = ResolvedSurfaceTarget::fell_back(
        WindowSurfaceTarget::Composition,
        SurfaceTargetFallback::BackendUnavailable,
        false,
    );
    assert_eq!(next_bootstrap_attempt(no_backend), None);

    assert_eq!(
        next_bootstrap_attempt(ResolvedSurfaceTarget::honoured(
            WindowSurfaceTarget::Composition,
            true
        )),
        None
    );
}

#[test]
fn a_dx12_hwnd_fallback_keeps_opaque_chrome_across_later_reconciles() {
    use crate::presentation::NativeChromePolicy;
    let settings = WindowDescriptor::new("Scene");
    let first = resolved(
        &settings,
        MaterialEffect::Transparent,
        wgpu::CompositeAlphaMode::Opaque,
        wgpu::Backend::Dx12,
    );
    assert_eq!(first.effective().effect, MaterialEffect::Solid);
    assert_eq!(
        first.effective().fallback,
        Some(MaterialFallback::NativeMaterialUnavailable)
    );
    assert_eq!(first.chrome(), Some(NativeChromePolicy::OPAQUE));

    let again = resolved(
        &settings,
        first.effective().effect,
        wgpu::CompositeAlphaMode::Opaque,
        wgpu::Backend::Dx12,
    );
    assert_eq!(again.chrome(), first.chrome());
    assert!(!again.needs_material_reset());
}

#[test]
fn a_system_backdrop_keeps_the_round_clip_that_paints_it() {
    use crate::presentation::NativeChromePolicy;
    let settings = WindowDescriptor::new("Scene");
    for material in [MaterialEffect::Mica, MaterialEffect::Acrylic] {
        assert!(material.wants_transparent_surface());
        let chrome = resolved(
            &settings,
            material,
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::Backend::Dx12,
        )
        .chrome();
        assert_eq!(chrome, Some(NativeChromePolicy::OPAQUE));
    }
}

#[test]
fn scene_icons_resolve_without_the_default_mark_only_when_it_is_left_out() {
    let own = WindowIcon::from_rgba(vec![1, 2, 3, 4], 1, 1).unwrap();
    assert_eq!(resolved_scene_icon(Some(&own)), Some(own));
    let unset = resolved_scene_icon(None);
    if cfg!(feature = "no-default-app-icon") {
        assert_eq!(unset, nana_ui_platform::resolve_window_icon(None));
    } else {
        assert!(unset.is_some());
    }
}

#[test]
fn window_and_taskbar_icons_convert_independent_buffers() {
    let source = WindowIcon::from_rgba(vec![73; 8 * 8 * 4], 8, 8).expect("valid icon source");
    let window_icon = winit_icon(&source).expect("window icon");
    let taskbar_icon = winit_icon(&source).expect("taskbar icon");
    assert!(
        !Arc::ptr_eq(&window_icon.0, &taskbar_icon.0),
        "each applied icon must own its RGBA buffer"
    );
}

#[test]
fn a_ratio_locked_window_opens_at_its_ratio_with_a_ratio_exact_minimum() {
    let settings = WindowDescriptor::new("Player")
        .initial_size(960.0, 540.0)
        .minimum_size(320.0, 320.0)
        .content_aspect_ratio(Some(9.0 / 16.0));
    let logical = |size: Option<winit::dpi::Size>| match size {
        Some(winit::dpi::Size::Logical(size)) => (size.width, size.height),
        other => panic!("expected a logical size, got {other:?}"),
    };
    let open = |desktop: &Desktop, settings: &WindowDescriptor| {
        let attributes =
            scene_window_attributes(settings, desktop, WindowSurfaceTarget::NativeWindow);
        (
            logical(attributes.surface_size),
            logical(attributes.min_surface_size),
        )
    };
    let unknown = Desktop {
        displays: Vec::new(),
        size_ratios: Vec::new(),
        scale: 1.0,
    };
    let (size, minimum) = open(&unknown, &settings);
    assert!((size.0 - 540.0).abs() < 0.01 && (size.1 - 960.0).abs() < 0.01);
    assert!((minimum.0 - 320.0).abs() < 0.01 && (minimum.1 - 568.89).abs() < 0.01);

    let short = Desktop {
        displays: vec![DisplayBounds {
            position: (0.0, 0.0),
            size: (1366.0, 728.0),
        }],
        size_ratios: Vec::new(),
        scale: 1.0,
    };
    let mut constrained = settings.clone();
    constrained.constrain_to_work_area = true;
    let (size, minimum) = open(&short, &constrained);
    assert!((size.0 - 409.5).abs() < 0.01 && (size.1 - 728.0).abs() < 0.01);
    assert!(minimum.0 <= size.0 && minimum.1 <= size.1);
}

#[test]
fn scene_windows_reclamp_offscreen_initial_positions_to_live_displays() {
    let mut settings = WindowDescriptor::new("Scene");
    settings.initial_size = (888.0, 586.0);
    settings.initial_position = Some((2100.0, 40.0));
    let main = Desktop {
        displays: vec![DisplayBounds {
            position: (0.0, 0.0),
            size: (1920.0, 1080.0),
        }],
        size_ratios: Vec::new(),
        scale: 1.0,
    };

    let attributes = scene_window_attributes(&settings, &main, WindowSurfaceTarget::NativeWindow);
    assert_eq!(
        attributes.position,
        Some(desktop_position((1032.0, 40.0), 1.0))
    );

    let disconnected = Desktop {
        displays: vec![
            main.displays[0],
            DisplayBounds {
                position: (1920.0, 0.0),
                size: (1080.0, 1920.0),
            },
        ],
        size_ratios: Vec::new(),
        scale: 1.0,
    };
    let attributes =
        scene_window_attributes(&settings, &disconnected, WindowSurfaceTarget::NativeWindow);
    assert_eq!(
        attributes.position,
        Some(desktop_position((2100.0, 40.0), 1.0))
    );

    settings.initial_position = None;
    assert_eq!(
        scene_window_attributes(&settings, &main, WindowSurfaceTarget::NativeWindow).position,
        None
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn mixed_scale_displays_share_one_desktop_space() {
    let display = |x, y, scale, primary| nana_ui_platform::DisplayInfo {
        id: nana_ui_platform::DisplayId(u128::from(primary)),
        name: None,
        physical_position: Some((x, y)),
        physical_size: Some((1920, 1080)),
        scale_factor: scale,
        refresh_rate_millihertz: None,
        primary,
    };
    let primary = display(0, 0, 1.0, true);
    let below = display(-7, 1080, 1.25, false);
    let scale = desktop_scale(below.scale_factor, Some(primary.scale_factor));
    let displays: Vec<_> = [primary, below]
        .iter()
        .map(|display| display.logical_bounds(scale).unwrap())
        .collect();
    assert_eq!(displays[1].position, (-7.0, 1080.0));
    assert!(displays[0].position.1 + displays[0].size.1 <= displays[1].position.1);

    let mut settings = WindowDescriptor::new("Saved");
    settings.initial_size = (1280.0, 800.0);
    settings.initial_position = Some((116.0, 1127.0));
    let attributes = scene_window_attributes(
        &settings,
        &Desktop {
            displays,
            size_ratios: vec![1.0, 1.25],
            scale,
        },
        WindowSurfaceTarget::NativeWindow,
    );
    assert_eq!(
        attributes.position,
        Some(winit::dpi::PhysicalPosition::new(116, 1127).into())
    );
}

#[test]
fn native_and_transparent_materials_clear_the_surface_to_zero_alpha() {
    let solid = scene_clear_color(ThemeAppearance::Dark, MaterialOutcome::chosen_solid(), None);
    assert!(solid[3] > 0.0, "opaque windows keep a readable clear color");
    assert_eq!(
        scene_clear_color(ThemeAppearance::Dark, MaterialOutcome::transparent(), None),
        [0.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        scene_clear_color(
            ThemeAppearance::Dark,
            MaterialOutcome::native(MaterialEffect::Mica),
            None
        ),
        [0.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        scene_clear_color(
            ThemeAppearance::Light,
            MaterialOutcome::native(MaterialEffect::Acrylic),
            None
        ),
        [0.0, 0.0, 0.0, 0.0]
    );
}

#[test]
fn the_clear_colour_is_the_palette_colour_in_linear_light() {
    let [r, g, b, a] =
        scene_clear_color(ThemeAppearance::Dark, MaterialOutcome::chosen_solid(), None);
    let expected = ((24.0f32 / 255.0 + 0.055) / 1.055).powf(2.4);
    for channel in [r, g, b] {
        assert!((channel - expected).abs() < 1e-6, "{channel} != {expected}");
    }
    assert_eq!(a, 1.0);
}

#[test]
fn an_opaque_window_clears_to_the_host_colour_rather_than_the_theme() {
    let black = nana_ui_core::SemanticColor::rgb8(0, 0, 0);
    for theme in [ThemeAppearance::Dark, ThemeAppearance::Light] {
        assert_eq!(
            scene_clear_color(theme, MaterialOutcome::chosen_solid(), Some(black)),
            [0.0, 0.0, 0.0, 1.0]
        );
    }
    assert_ne!(
        scene_clear_color(ThemeAppearance::Dark, MaterialOutcome::chosen_solid(), None),
        scene_clear_color(
            ThemeAppearance::Light,
            MaterialOutcome::chosen_solid(),
            None
        )
    );
    assert_eq!(
        scene_clear_color(
            ThemeAppearance::Light,
            MaterialOutcome::transparent(),
            Some(black)
        ),
        [0.0, 0.0, 0.0, 0.0]
    );
}

#[test]
fn an_opaque_surface_reports_the_transparent_request_as_a_fallback() {
    use wgpu::Backend::{Dx12, Gl, Metal, Vulkan};
    use wgpu::CompositeAlphaMode::{Opaque, PostMultiplied, PreMultiplied};
    let demoted = MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable);
    assert!(scene_clear_color(ThemeAppearance::Dark, demoted, None)[3] > 0.0);
    for requested in [
        MaterialOutcome::transparent(),
        MaterialOutcome::native(MaterialEffect::Mica),
        MaterialOutcome::native(MaterialEffect::Acrylic),
    ] {
        for backend in [Dx12, Vulkan, Metal] {
            assert_eq!(
                effective_for(requested, Opaque, backend),
                demoted,
                "{backend:?} honours the configured alpha mode"
            );
            for alpha in [PreMultiplied, PostMultiplied] {
                assert_eq!(effective_for(requested, alpha, backend), requested);
            }
        }
        assert_eq!(effective_for(requested, Opaque, Gl), requested);
    }
    for chosen in [
        MaterialOutcome::chosen_solid(),
        MaterialOutcome::solid(MaterialFallback::PlatformDoesNotProvideNativeMaterial),
    ] {
        assert_eq!(effective_for(chosen, Opaque, Dx12), chosen);
    }
}

fn effective_for(
    applied: MaterialOutcome,
    alpha: wgpu::CompositeAlphaMode,
    backend: wgpu::Backend,
) -> MaterialOutcome {
    ResolvedWindowPresentation::resolve(
        &WindowDescriptor::new("Scene"),
        applied.effect,
        applied,
        alpha,
        backend,
        ResolvedSurfaceTarget::honoured(WindowSurfaceTarget::NativeWindow, false),
        nana_window::NonClientRenderingStrategy::StripFrameStyles,
    )
    .effective()
}

#[test]
fn transparent_aux_keeps_transparent_when_primary_appearance_is_solid() {
    let appearance = MaterialEffect::Solid;
    assert_eq!(
        window_surface_effect(false, appearance),
        MaterialEffect::Solid
    );
    assert_eq!(
        window_surface_effect(true, appearance),
        MaterialEffect::Transparent
    );
    assert!(window_surface_effect(true, appearance).wants_transparent_surface());
    assert!(window_wants_transparent_surface(true, appearance));
    assert!(!window_wants_transparent_surface(false, appearance));
}

#[test]
fn transparent_surface_picks_non_opaque_alpha_before_surface_lock() {
    let appearance = MaterialEffect::Solid;
    let transparent = window_wants_transparent_surface(true, appearance);
    assert!(transparent);
    assert_eq!(
        crate::hosted_context::preferred_alpha_mode(
            &[
                wgpu::CompositeAlphaMode::Opaque,
                wgpu::CompositeAlphaMode::PreMultiplied,
                wgpu::CompositeAlphaMode::PostMultiplied,
            ],
            transparent,
        ),
        wgpu::CompositeAlphaMode::PreMultiplied
    );
    assert_eq!(
        crate::hosted_context::preferred_alpha_mode(
            &[
                wgpu::CompositeAlphaMode::Opaque,
                wgpu::CompositeAlphaMode::PostMultiplied,
            ],
            transparent,
        ),
        wgpu::CompositeAlphaMode::PostMultiplied
    );
    let opaque = window_wants_transparent_surface(false, appearance);
    assert!(!opaque);
    assert_eq!(
        crate::hosted_context::preferred_alpha_mode(
            &[
                wgpu::CompositeAlphaMode::Opaque,
                wgpu::CompositeAlphaMode::PostMultiplied,
            ],
            opaque,
        ),
        wgpu::CompositeAlphaMode::Opaque
    );
}

#[test]
fn content_zoom_maps_the_pointer_into_content_units_and_back_to_the_screen() {
    let window = WindowGeometry {
        physical_size: (1600, 1200),
        logical_size: (800.0, 600.0),
        scale_factor: 2.0,
        ..WindowGeometry::default()
    };
    let content = window.content(1.5);
    assert_eq!(content.scale_factor, 3.0);
    let mut adapter = WinitInputAdapter::default();
    adapter.set_cursor_physical(PhysicalPosition::new(300.0, 150.0), content.scale_factor);
    assert_eq!(adapter.cursor(), (100.0, 50.0));
    let space = ScreenSpace {
        origin: (10.0, 20.0),
        client_ratio: 1.5,
    };
    assert_eq!(
        screen_position(Some(space), adapter.cursor()),
        (160.0, 95.0)
    );
    let settings = WindowDescriptor::new("zoomed");
    let right = content.logical_size.0 - 1.0;
    assert_eq!(
        frame_resize_edge_for(&settings, &content, false, right, 200.0),
        Some(WindowResizeEdge::East)
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::SetContentZoom {
                id: WindowId::PRIMARY,
                zoom: 1.5,
            },
            &[WindowId::PRIMARY]
        ),
        RoutedWindowCommand::SetContentZoom(WindowId::PRIMARY)
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::SetContentZoom {
                id: WindowId(9),
                zoom: 1.5,
            },
            &[WindowId::PRIMARY]
        ),
        RoutedWindowCommand::Ignore
    );
}

#[test]
fn size_limit_commands_route_to_open_windows_only() {
    let command = |id| WindowCommand::SetSizeLimits {
        id,
        minimum: Some((480.0, 360.0)),
        maximum: Some((1280.0, 960.0)),
    };
    assert_eq!(
        route_window_command(&command(WindowId::PRIMARY), &[WindowId::PRIMARY]),
        RoutedWindowCommand::SetSizeLimits(WindowId::PRIMARY)
    );
    assert_eq!(
        route_window_command(&command(WindowId(4)), &[WindowId::PRIMARY]),
        RoutedWindowCommand::Ignore
    );
}

#[test]
fn window_commands_route_by_known_ids_without_a_surface() {
    let primary = WindowId::PRIMARY;
    let tool = WindowId(7);
    let known = [primary, tool];
    let settings = WindowDescriptor::new("tool");

    assert_eq!(
        route_window_command(
            &WindowCommand::Open {
                id: tool,
                settings: settings.clone(),
            },
            &known
        ),
        RoutedWindowCommand::Focus(tool)
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::Open {
                id: WindowId(9),
                settings,
            },
            &known
        ),
        RoutedWindowCommand::Open(WindowId(9))
    );
    assert_eq!(
        route_window_command(&WindowCommand::Close(primary), &known),
        RoutedWindowCommand::Close(primary)
    );
    assert_eq!(
        route_window_command(&WindowCommand::Close(tool), &known),
        RoutedWindowCommand::Close(tool)
    );
    assert_eq!(
        route_window_command(&WindowCommand::Close(WindowId(3)), &known),
        RoutedWindowCommand::Ignore
    );
    assert_eq!(
        route_window_command(&WindowCommand::Focus(tool), &known),
        RoutedWindowCommand::Focus(tool)
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::SetTitle {
                id: tool,
                title: "Aux".into(),
            },
            &known
        ),
        RoutedWindowCommand::SetTitle(tool)
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::Move {
                id: tool,
                position: (8.0, 16.0),
            },
            &known
        ),
        RoutedWindowCommand::Move(tool)
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::SetBounds {
                id: primary,
                position: (0.0, 0.0),
                size: (100.0, 80.0),
            },
            &known
        ),
        RoutedWindowCommand::SetBounds(primary)
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::SetFullscreen {
                id: WindowId(3),
                fullscreen: Some(nana_ui_platform::FullscreenRequest::default()),
            },
            &known
        ),
        RoutedWindowCommand::Ignore
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::SetMinimized {
                id: tool,
                minimized: true,
            },
            &known
        ),
        RoutedWindowCommand::SetMinimized(tool)
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::SetMaximized {
                id: tool,
                maximized: true,
            },
            &known
        ),
        RoutedWindowCommand::SetMaximized(tool)
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::SetAlwaysOnTop {
                id: tool,
                always_on_top: true,
            },
            &known
        ),
        RoutedWindowCommand::SetAlwaysOnTop(tool)
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::SetContentAspectRatio {
                id: tool,
                ratio: Some(1.5),
            },
            &known
        ),
        RoutedWindowCommand::SetContentAspectRatio(tool)
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::SetContentAspectRatio {
                id: WindowId(3),
                ratio: None,
            },
            &known
        ),
        RoutedWindowCommand::Ignore
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::SetNativeWindowControlsVisible {
                id: tool,
                visible: false,
                duration: std::time::Duration::from_millis(140),
            },
            &known
        ),
        RoutedWindowCommand::SetNativeWindowControlsVisible(tool)
    );
    assert_eq!(
        route_window_command(
            &WindowCommand::SetIcon {
                id: tool,
                icon: None,
            },
            &known
        ),
        RoutedWindowCommand::SetIcon(tool)
    );
    assert_eq!(
        route_window_command(&WindowCommand::SetApplicationIcon { icon: None }, &known),
        RoutedWindowCommand::SetApplicationIcon
    );
    assert_eq!(
        route_window_command(&WindowCommand::Drag(tool), &known),
        RoutedWindowCommand::Drag(tool)
    );
    assert_eq!(
        route_window_command(&WindowCommand::Drag(WindowId(3)), &known),
        RoutedWindowCommand::Ignore
    );
}

#[test]
fn client_frame_resize_hits_edges_unless_caption_or_maximized() {
    use winit::cursor::CursorIcon;

    let mut settings = WindowDescriptor::new("Scene");
    let mut geometry = WindowGeometry {
        physical_size: (200, 100),
        logical_size: (800.0, 600.0),
        scale_factor: 2.0,
        ..WindowGeometry::default()
    };
    assert_eq!(
        frame_resize_edge_for(&settings, &geometry, false, 2.0, 300.0),
        Some(WindowResizeEdge::West)
    );
    assert_eq!(
        frame_resize_edge_for(&settings, &geometry, false, 400.0, 300.0),
        None
    );
    settings.system_caption = true;
    assert!(frame_resize_edge_for(&settings, &geometry, false, 2.0, 300.0).is_none());
    settings.system_caption = false;
    settings.resizable = false;
    assert!(frame_resize_edge_for(&settings, &geometry, false, 2.0, 300.0).is_none());
    settings.resizable = true;
    geometry.maximized = true;
    assert!(frame_resize_edge_for(&settings, &geometry, false, 2.0, 300.0).is_none());
    geometry.maximized = false;
    assert!(frame_resize_edge_for(&settings, &geometry, true, 2.0, 300.0).is_none());
    assert_eq!(
        frame_edge_cursor(WindowResizeEdge::East),
        CursorIcon::EwResize
    );
    assert_eq!(
        frame_edge_cursor(WindowResizeEdge::South),
        CursorIcon::NsResize
    );
    assert_eq!(
        frame_edge_cursor(WindowResizeEdge::NorthEast),
        CursorIcon::NeswResize
    );
    assert_eq!(
        frame_edge_cursor(WindowResizeEdge::SouthEast),
        CursorIcon::NwseResize
    );
}

#[test]
fn window_cursor_matches_css_cursor_spec() {
    use crate::WindowCursor;
    use winit::cursor::CursorIcon;
    assert_eq!(
        window_cursor_override(WindowCursor::Automatic),
        (None, None)
    );
    assert_eq!(
        window_cursor_override(WindowCursor::Help),
        (Some(CursorIcon::Help), None)
    );
    assert_eq!(
        window_cursor_override(WindowCursor::Progress),
        (Some(CursorIcon::Progress), None)
    );
    assert_eq!(
        window_cursor_override(WindowCursor::ZoomIn),
        (Some(CursorIcon::ZoomIn), None)
    );
    assert_eq!(
        window_cursor_override(WindowCursor::ZoomOut),
        (Some(CursorIcon::ZoomOut), None)
    );
    assert_eq!(
        window_cursor_override(WindowCursor::None),
        (Some(CursorIcon::Default), Some(false))
    );
}

#[test]
fn redraw_requests_ignore_unknown_ids_and_cover_every_known_window() {
    let primary = WindowId::PRIMARY;
    let tool = WindowId(2);
    let known = [primary, tool];

    assert!(windows_to_redraw(RuntimeRedraw::None, &known).is_empty());
    assert_eq!(
        windows_to_redraw(RuntimeRedraw::Window(tool), &known),
        vec![tool]
    );
    assert!(windows_to_redraw(RuntimeRedraw::Window(WindowId(9)), &known).is_empty());
    assert_eq!(
        windows_to_redraw(RuntimeRedraw::All, &known),
        vec![primary, tool]
    );
}

#[test]
fn image_target_index_replaces_and_shares_resource_keys() {
    let first = WindowId::PRIMARY;
    let second = WindowId(2);
    let mut targets = HashMap::new();
    let mut window_keys = HashMap::new();
    let arc_list = |items: &[&str]| -> Arc<[Arc<str>]> {
        items
            .iter()
            .map(|&s| Arc::<str>::from(s))
            .collect::<Vec<_>>()
            .into()
    };
    replace_image_target_index(
        &mut targets,
        &mut window_keys,
        first,
        1,
        arc_list(&["shared.png", "old.png"]),
    );
    replace_image_target_index(
        &mut targets,
        &mut window_keys,
        second,
        1,
        arc_list(&["shared.png", "second.png"]),
    );
    assert_eq!(targets["shared.png"], HashSet::from([first, second]));

    replace_image_target_index(
        &mut targets,
        &mut window_keys,
        first,
        2,
        arc_list(&["new.png"]),
    );
    assert!(!targets.contains_key("old.png"));
    assert_eq!(targets["shared.png"], HashSet::from([second]));
    assert_eq!(targets["new.png"], HashSet::from([first]));
}

#[test]
fn image_target_index_removes_closed_window_without_leaking_keys() {
    let first = WindowId::PRIMARY;
    let second = WindowId(2);
    let mut targets = HashMap::new();
    let mut window_keys = HashMap::new();
    let arc_list = |items: &[&str]| -> Arc<[Arc<str>]> {
        items
            .iter()
            .map(|&s| Arc::<str>::from(s))
            .collect::<Vec<_>>()
            .into()
    };
    replace_image_target_index(
        &mut targets,
        &mut window_keys,
        first,
        1,
        arc_list(&["shared.png"]),
    );
    replace_image_target_index(
        &mut targets,
        &mut window_keys,
        second,
        1,
        arc_list(&["shared.png", "only-second.png"]),
    );
    remove_image_target_index(&mut targets, &mut window_keys, second);
    assert_eq!(targets["shared.png"], HashSet::from([first]));
    assert!(!targets.contains_key("only-second.png"));
    assert!(!window_keys.contains_key(&second));
}

#[test]
fn runtime_prevent_default_still_invokes_the_program_input_hook() {
    let update = scene_runtime_input_update(
        InputDisposition {
            handled: true,
            prevent_default: true,
        },
        WindowId::PRIMARY,
        Ok(RuntimeProgramUpdate::exit()),
    );
    assert!(update.exit);
    assert_eq!(update.redraw, RuntimeRedraw::Window(WindowId::PRIMARY));

    let update = scene_runtime_input_update(
        InputDisposition {
            handled: false,
            prevent_default: false,
        },
        WindowId::PRIMARY,
        Ok(RuntimeProgramUpdate::default()),
    );
    assert_eq!(update.redraw, RuntimeRedraw::None);
}

#[test]
fn failed_program_input_degrades_without_panicking() {
    let update = scene_runtime_input_update(
        InputDisposition {
            handled: true,
            prevent_default: true,
        },
        WindowId::PRIMARY,
        Err(FrameworkError::InvalidAction),
    );
    assert_eq!(update.redraw, RuntimeRedraw::Window(WindowId::PRIMARY));
    assert!(!update.exit);
    assert!(update.window_commands.is_empty());
}

#[test]
fn device_recovery_invalidates_cloned_program_host_textures_before_rebuild() {
    let registry = occupied_host_textures("live");
    struct FakeProgram {
        textures: HostTextureRegistry,
        rebuilt_len: std::cell::Cell<Option<usize>>,
    }
    impl FakeProgram {
        fn host_textures(&self, id: WindowId) -> Option<HostTextureRegistry> {
            match id {
                WindowId::PRIMARY | WindowId(2) => Some(self.textures.clone()),
                _ => None,
            }
        }

        fn rebuild_gpu(&self) {
            self.rebuilt_len.set(Some(self.textures.len()));
        }
    }

    let program = FakeProgram {
        textures: registry.clone(),
        rebuilt_len: std::cell::Cell::new(None),
    };
    let cleared =
        invalidate_program_host_textures([WindowId::PRIMARY, WindowId(2), WindowId(9)], |id| {
            program.host_textures(id)
        });
    program.rebuild_gpu();

    assert_eq!(cleared, 1);
    assert_eq!(program.rebuilt_len.get(), Some(0));
    assert!(registry.is_empty());
    assert_eq!(
        invalidate_program_host_textures([WindowId::PRIMARY, WindowId(2)], |id| program
            .host_textures(id)),
        0
    );
}

fn occupied_host_textures(slot: &str) -> HostTextureRegistry {
    let registry = HostTextureRegistry::new();
    registry.register(
        slot,
        HostTexture::new(&crate::test_gpu::texture(1, 1)),
        8,
        8,
        HostTextureAlphaMode::Premultiplied,
    );
    registry
}

#[test]
fn acknowledged_commands_route_missing_windows_for_failure_reports() {
    for id in [WindowId::PRIMARY, WindowId(20)] {
        assert_eq!(
            route_window_command(
                &WindowCommand::SetSkipTaskbar {
                    id,
                    skip_taskbar: true,
                },
                &[WindowId::PRIMARY]
            ),
            RoutedWindowCommand::SetSkipTaskbar(id, true)
        );
        assert_eq!(
            route_window_command(
                &WindowCommand::SetMousePassthrough { id, enabled: true },
                &[WindowId::PRIMARY]
            ),
            RoutedWindowCommand::SetMousePassthrough(id, MousePassthroughMode::Passthrough)
        );
        assert_eq!(
            route_window_command(
                &WindowCommand::SetMousePassthroughForward { id, enabled: true },
                &[WindowId::PRIMARY]
            ),
            RoutedWindowCommand::SetMousePassthrough(id, MousePassthroughMode::Forward)
        );
        assert_eq!(
            route_window_command(
                &WindowCommand::SetMousePassthroughForward { id, enabled: false },
                &[WindowId::PRIMARY]
            ),
            RoutedWindowCommand::SetMousePassthrough(id, MousePassthroughMode::Off)
        );
    }
}

fn pointer_down_event() -> WinitWindowEvent {
    WinitWindowEvent::PointerButton {
        device_id: None,
        state: ElementState::Pressed,
        position: PhysicalPosition::new(20.0, 40.0),
        primary: true,
        button: ButtonSource::Mouse(MouseButton::Left),
        is_macos_activation_click: false,
    }
}

#[test]
fn forward_passthrough_drops_down_until_hit_testing_recovers() {
    let down = pointer_down_event();
    assert_eq!(
        forward_pointer_action(MousePassthroughMode::Forward, true, true, false, &down),
        ForwardPointerAction::IgnoreUntilRecovered,
    );
    assert_eq!(
        forward_pointer_action(MousePassthroughMode::Forward, true, true, true, &down),
        ForwardPointerAction::IgnoreUntilRecovered,
    );
    assert_eq!(
        forward_pointer_action(MousePassthroughMode::Forward, false, true, false, &down),
        ForwardPointerAction::Dispatch,
    );
    assert_eq!(
        forward_pointer_action(MousePassthroughMode::Forward, false, false, false, &down),
        ForwardPointerAction::RestorePassthrough
    );
    assert_eq!(
        forward_pointer_action(MousePassthroughMode::Off, true, true, false, &down),
        ForwardPointerAction::IgnoreUntilRecovered,
    );
    assert_eq!(
        forward_pointer_action(MousePassthroughMode::Passthrough, true, true, false, &down),
        ForwardPointerAction::IgnoreUntilRecovered,
    );
}

#[test]
fn forward_passthrough_keeps_active_gesture_across_transparent_gaps_and_leaves() {
    let moved = WinitWindowEvent::PointerMoved {
        device_id: None,
        position: PhysicalPosition::new(10.0, 10.0),
        primary: true,
        source: PointerSource::Mouse,
    };
    let left = WinitWindowEvent::PointerLeft {
        device_id: None,
        position: Some(PhysicalPosition::new(-4.0, 10.0)),
        primary: true,
        kind: PointerKind::Mouse,
    };
    let up = WinitWindowEvent::PointerButton {
        device_id: None,
        state: ElementState::Released,
        position: PhysicalPosition::new(10.0, 10.0),
        primary: true,
        button: ButtonSource::Mouse(MouseButton::Left),
        is_macos_activation_click: false,
    };
    for event in [&moved, &left, &up] {
        assert_eq!(
            forward_pointer_action(MousePassthroughMode::Forward, false, false, true, event),
            ForwardPointerAction::Dispatch,
            "active gesture must not restore passthrough mid-gesture: {event:?}"
        );
    }
    assert_eq!(
        forward_pointer_action(MousePassthroughMode::Forward, false, false, false, &moved),
        ForwardPointerAction::RestorePassthrough
    );
    assert_eq!(
        forward_pointer_action(MousePassthroughMode::Forward, false, false, false, &left),
        ForwardPointerAction::RestorePassthrough
    );
}
