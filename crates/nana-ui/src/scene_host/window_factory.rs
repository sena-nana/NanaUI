//! Unified window, surface, material, chrome, and `WindowContext` assembly.
//!
//! All three host creation flows — standalone async startup
//! (`PrimaryStandalone`), embedded startup (`PrimaryEmbedded`), and runtime
//! secondary/auxiliary/underlay windows (`Auxiliary`) — share one provisional
//! HWND + surface-target retry loop and one [`WindowSessionBuilder`] so
//! composition fallback, native chrome, `WindowDescriptor::parent` ownership vs
//! `Underlay`, and `WindowContext` initialization cannot drift apart.

use super::*;
use crate::winit_input::{WinitInputAdapter, system_appearance_from_winit};

/// Which host lifecycle is creating or binding a native window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowRole {
    /// Standalone `run_runtime_scene` startup: window is up first while the GPU
    /// device is acquired on a worker thread.
    PrimaryStandalone,
    /// Embedded `EmbeddedRuntime::new`: host already owns the event loop and
    /// shared GPU device.
    PrimaryEmbedded,
    /// Secondary, auxiliary, or underlay window opened via `WindowManager::open_window`.
    Auxiliary,
}

/// Clears native material registrations on every failed creation path.
pub(super) struct PendingNativeWindow(pub(super) Option<Arc<dyn winit::window::Window>>);

impl PendingNativeWindow {
    /// The window survived; keep its native material applied.
    pub(super) fn keep(mut self) {
        self.0 = None;
    }

    pub(super) fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for PendingNativeWindow {
    fn drop(&mut self) {
        if let Some(window) = self.0.as_ref() {
            clear_system_material(window.as_ref());
        }
    }
}

/// Provisional native window and bound surface payload produced by one
/// successful attempt inside [`create_window_with_surface_retry`].
pub(super) struct ProvisionalWindow<S> {
    pub(super) window: Arc<dyn winit::window::Window>,
    pub(super) provisional: PendingNativeWindow,
    pub(super) target: crate::presentation::ResolvedSurfaceTarget,
    pub(super) surface: S,
    pub(super) requested_material: crate::MaterialEffect,
    pub(super) applied_material: MaterialOutcome,
}

/// Shared provisional HWND + material + surface-target retry loop across
/// `PrimaryStandalone`, `PrimaryEmbedded`, and `Auxiliary`.
#[expect(
    clippy::too_many_arguments,
    reason = "Unified provisional window/surface attempt across standalone, embedded, and auxiliary roles"
)]
pub(super) fn create_window_with_surface_retry<S>(
    role: WindowRole,
    event_loop: &dyn ActiveEventLoop,
    settings: &WindowDescriptor,
    initial_target: crate::presentation::ResolvedSurfaceTarget,
    theme: crate::ThemeAppearance,
    material_mode: crate::MaterialEffect,
    backdrop_opacity: f32,
    window_background: Option<nana_ui_core::SemanticColor>,
    arm_early_frameless: bool,
    apply_full_icon: bool,
    primary_icon: bool,
    mut build_attributes: impl FnMut(
        WindowSurfaceTarget,
    ) -> Result<winit::window::WindowAttributes, String>,
    mut try_bind_surface: impl FnMut(
        &Arc<dyn winit::window::Window>,
        WindowSurfaceTarget,
        crate::MaterialEffect,
    ) -> Result<S, String>,
    mut on_composed_fallback: impl FnMut(),
) -> Result<ProvisionalWindow<S>, String> {
    let mut attempt = initial_target;
    let mut composed_error = None;
    loop {
        let attributes = build_attributes(attempt.resolved)?;
        let window: Arc<dyn winit::window::Window> = Arc::from(
            event_loop
                .create_window(attributes.with_visible(false))
                .map_err(|error| {
                    if role == WindowRole::Auxiliary {
                        error.to_string()
                    } else {
                        format!("failed to create scene window: {error}")
                    }
                })?,
        );
        if arm_early_frameless && !settings.system_caption {
            arm_frameless_guard(window.as_ref(), true);
        }
        let provisional = PendingNativeWindow(Some(Arc::clone(&window)));
        if apply_full_icon {
            apply_scene_window_icon(window.as_ref(), settings.icon.as_ref(), primary_icon);
        } else {
            #[cfg(target_os = "windows")]
            window.set_taskbar_icon(
                resolved_scene_icon(settings.icon.as_ref())
                    .as_ref()
                    .and_then(winit_icon),
            );
        }
        let (requested_material, applied_material) = apply_window_material(
            window.as_ref(),
            theme,
            settings,
            material_mode,
            backdrop_opacity,
            window_background,
        );
        match try_bind_surface(&window, attempt.resolved, requested_material)
            .and_then(|surface| composed_fault(attempt).map_or(Ok(surface), Err))
        {
            Ok(surface) => {
                return Ok(ProvisionalWindow {
                    window,
                    provisional,
                    target: attempt,
                    surface,
                    requested_material,
                    applied_material,
                });
            }
            Err(error) => {
                drop(provisional);
                drop(window);
                if attempt.resolved.composed() {
                    on_composed_fallback();
                }
                attempt = next_target_attempt(attempt, error, &mut composed_error)?;
            }
        }
    }
}

/// Assembles a live [`WindowContext`] consistently for primary (standalone or
/// embedded) and auxiliary/underlay windows.
pub(super) struct WindowSessionBuilder {
    pub(super) role: WindowRole,
    pub(super) id: WindowId,
    pub(super) generation: u64,
    pub(super) surface: HostedGpuSurface,
    pub(super) geometry: WindowGeometry,
    pub(super) presentation: ResolvedWindowPresentation,
    pub(super) settings: WindowDescriptor,
    #[cfg(not(target_os = "android"))]
    pub(super) accessibility: Option<HostedAccessibility>,
    pub(super) level: WindowLevel,
    pub(super) requested_level: WindowLevel,
    pub(super) underlay: Option<nana_window::Underlay>,
    pub(super) auxiliary: Option<nana_window::AuxiliaryWindow>,
}

impl WindowSessionBuilder {
    pub(super) fn build(self, proxy: &EventLoopProxy) -> Result<WindowContext, String> {
        let _ = self.role;
        let window = self.surface.window();
        let size_move = {
            let native_wake = proxy.clone();
            let size_move = LiveSizeMove::install_with_wake(
                window.as_ref(),
                Arc::new(move || native_wake.wake_up()),
            )?;
            size_move.set_content_aspect_ratio(windows::content_aspect_ratio(&self.settings));
            size_move
        };
        let pen = {
            let pen_wake = proxy.clone();
            nana_window::PenInput::install_with_wake(
                window.as_ref(),
                Arc::new(move || pen_wake.wake_up()),
            )?
        };
        let skip_taskbar_report = windows::descriptor_skip_taskbar(window.as_ref(), &self.settings);
        let monitor = window.current_monitor().map(|monitor| monitor.id());
        let pending_fullscreen = self.settings.fullscreen;
        Ok(WindowContext {
            surface_retry: None,
            surface_generation: 0,
            applied_appearance: None,
            cursor_override: None,
            cursor_visible_override: None,
            passthrough_mode: MousePassthroughMode::Off,
            os_mouse_passthrough: false,
            material_override: None,
            surface: self.surface,
            geometry: self.geometry,
            drawable_geometry: self.geometry,
            content_zoom: 1.0,
            maximum_size: None,
            input: WinitInputAdapter::default(),
            input_source: WindowInputSource::new(
                nana_ui_platform::InputSourceId(self.id.0),
                nana_ui_platform::EndpointGeneration(self.generation),
            ),
            presentation: self.presentation,
            settings: self.settings,
            #[cfg(not(target_os = "android"))]
            accessibility: self.accessibility,
            accessibility_pending: PendingAccessibility::default(),
            size_move,
            pen,
            aspect_conform_pending: false,
            level: self.level,
            requested_level: self.requested_level,
            presented: std::cell::Cell::new(false),
            mode: None,
            pending_fullscreen,
            native_controls_visible: true,
            native_controls: None,
            native_controls_box: std::cell::Cell::new(None),
            skip_taskbar: matches!(skip_taskbar_report, Some(Ok(()))),
            skip_taskbar_report,
            pointer_presence: presence::PointerPresence::default(),
            monitor,
            shadow: nana_window::shadow::WindowShadowState::default(),
            shadow_body: None,
            shadow_reported: Default::default(),
            interaction: nana_window::interaction::WindowInteractionState::default(),
            interaction_regions: nana_ui_runtime::WindowInteractionRegionBuilder::default(),
            interaction_native_snapshot: Arc::new(nana_ui_platform::InputRegionSnapshot::default()),
            underlay: self.underlay,
            auxiliary: self.auxiliary,
        })
    }
}

/// The window, GPU context and surface the first window ended up with.
pub(super) struct PrimaryBootstrap {
    pub(super) window: Arc<dyn winit::window::Window>,
    pub(super) graphics: crate::HostedGpuShared,
    pub(super) surface: HostedGpuSurface,
    pub(super) target: crate::presentation::ResolvedSurfaceTarget,
    /// Whether this process can present through a platform compositor at all.
    /// Every later window asks its own target against this, rather than
    /// inheriting the first window's answer.
    pub(super) composition: crate::presentation::CompositionAvailability,
    /// The material request this window was created with, and what the
    /// platform gave back for it. Carried out so the caller resolves its first
    /// presentation without asking the platform for the same thing twice.
    pub(super) requested_material: crate::MaterialEffect,
    pub(super) applied_material: MaterialOutcome,
}

/// Creates an embedded host's first window and binds it to a presentation
/// target on the embedder's device, falling back to the plain native path when
/// the composed one cannot be completed.
fn bootstrap_primary_window(
    event_loop: &dyn ActiveEventLoop,
    settings: &WindowDescriptor,
    shared_gpu: crate::HostedGpuShared,
    policy: crate::GpuBackendPolicy,
    theme: crate::ThemeAppearance,
    material_mode: crate::MaterialEffect,
) -> Result<PrimaryBootstrap, String> {
    let bootstrap = gpu_bootstrap(policy, Some(&shared_gpu));
    let initial_target = primary_surface_target(settings, policy, &bootstrap, material_mode)?;
    let desktop = scene_desktop(event_loop, settings.constrain_to_work_area);
    let result = create_window_with_surface_retry(
        WindowRole::PrimaryEmbedded,
        event_loop,
        settings,
        initial_target,
        theme,
        material_mode,
        AppearanceSettings::DEFAULT_BACKDROP_OPACITY,
        None,
        true,
        false,
        true,
        |target| Ok(scene_window_attributes(settings, &desktop, target)),
        |window, target, requested_material| {
            shared_gpu
                .create_surface_with_mode_policy(
                    Arc::clone(window),
                    requested_material.wants_transparent_surface(),
                    surface_mode_for(target),
                    settings.presentation_policy,
                )
                .map_err(|error| error.to_string())
        },
        || {},
    )?;
    result.provisional.keep();
    let composition =
        settle_primary_target(result.target, policy, shared_gpu.adapter_info().backend);
    Ok(PrimaryBootstrap {
        window: result.window,
        graphics: shared_gpu,
        surface: result.surface,
        target: result.target,
        composition,
        requested_material: result.requested_material,
        applied_material: result.applied_material,
    })
}

/// The target the primary window starts on.
pub(super) fn primary_surface_target(
    settings: &WindowDescriptor,
    policy: crate::GpuBackendPolicy,
    bootstrap: &crate::hosted_context::GpuBootstrap,
    material_mode: crate::MaterialEffect,
) -> Result<crate::presentation::ResolvedSurfaceTarget, String> {
    use crate::presentation::{resolve_window_surface_target, window_surface_request};
    let requested = window_surface_request(
        settings.surface,
        window_wants_transparent_surface(settings.transparent, material_mode),
        policy,
    );
    let target = resolve_window_surface_target(
        requested,
        settings.surface.requires_composition(),
        composition_availability(bootstrap),
    );
    match target.forbidden_fallback() {
        Some(reason) => Err(format!(
            "window requires a platform compositor surface: {}",
            reason.label()
        )),
        None => Ok(target),
    }
}

/// The target to try after `failed` could not be built.
pub(super) fn next_target_attempt(
    failed: crate::presentation::ResolvedSurfaceTarget,
    error: String,
    composed_error: &mut Option<String>,
) -> Result<crate::presentation::ResolvedSurfaceTarget, String> {
    match next_bootstrap_attempt(failed) {
        Some(next) => {
            *composed_error = Some(error);
            Ok(next)
        }
        None => Err(match composed_error.take() {
            Some(composed) => format!("{error} (after composition failed: {composed})"),
            None => error,
        }),
    }
}

/// Reports a fallback the primary window ended up on, and answers what later
/// windows can be composed against.
pub(super) fn settle_primary_target(
    target: crate::presentation::ResolvedSurfaceTarget,
    policy: crate::GpuBackendPolicy,
    backend: wgpu::Backend,
) -> crate::presentation::CompositionAvailability {
    use crate::presentation::CompositionAvailability;
    match target.fallback {
        Some(reason) => {
            eprintln!(
                "nana window surface: {}; presenting through the plain window path instead",
                reason.label()
            );
            nana_diagnostics::set_session_info("window.presentation_fallback", reason.label());
            CompositionAvailability::Unavailable
        }
        None => CompositionAvailability::for_backend(policy, backend),
    }
}

pub(super) fn composed_fault(target: crate::presentation::ResolvedSurfaceTarget) -> Option<String> {
    target
        .resolved
        .composed()
        .then(composition_fault_injection)
        .flatten()
}

pub(super) fn gpu_bootstrap(
    policy: crate::GpuBackendPolicy,
    shared_gpu: Option<&crate::HostedGpuShared>,
) -> crate::hosted_context::GpuBootstrap {
    use crate::hosted_context::GpuBootstrap;
    if !policy.wants_composition() {
        return GpuBootstrap::plain();
    }
    GpuBootstrap::probe(shared_gpu.map(|gpu| gpu.adapter_info().backend))
}

fn composition_availability(
    bootstrap: &crate::hosted_context::GpuBootstrap,
) -> crate::presentation::CompositionAvailability {
    if bootstrap.composition_available() {
        crate::presentation::CompositionAvailability::Available
    } else {
        crate::presentation::CompositionAvailability::Unavailable
    }
}

pub(super) fn next_bootstrap_attempt(
    attempt: crate::presentation::ResolvedSurfaceTarget,
) -> Option<crate::presentation::ResolvedSurfaceTarget> {
    use crate::presentation::{ResolvedSurfaceTarget, SurfaceTargetFallback};
    (attempt.resolved.composed() && !attempt.required).then(|| {
        ResolvedSurfaceTarget::fell_back(
            attempt.requested,
            SurfaceTargetFallback::TargetUnavailable,
            attempt.required,
        )
    })
}

fn composition_fault_injection() -> Option<String> {
    fault_flag("NANA_FORCE_COMPOSITION_FAILURE")
        .then(|| "composition failure requested by NANA_FORCE_COMPOSITION_FAILURE".to_owned())
}

pub(super) fn fault_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty() && value != "0")
}

/// The persisted geometry applied to the primary descriptor, which is then
/// checked; returns the host's store.
pub(super) fn prepare_primary_descriptor(
    settings: &mut WindowDescriptor,
) -> Result<SharedStore, String> {
    let store = crate::runtime_host::take_pending_store();
    restore_window_geometry(
        settings,
        &nana_ui_core::ViewStateStore::new(store.clone(), settings.restoration_scope.clone()),
    );
    crate::window_service::validate_descriptor(settings).map_err(|error| error.to_string())?;
    if settings.parent.is_some() {
        return Err("initial window cannot have a parent".into());
    }
    Ok(store)
}

/// An embedded host's startup: the embedder already owns the event loop and
/// the device, and has put nothing of NanaUI on screen, so there is no splash
/// and nothing to request off the event thread.
pub(super) fn initialize<Program: RuntimeProgram>(
    event_loop: &dyn ActiveEventLoop,
    proxy: EventLoopProxy,
    message_tx: Sender<Program::Message>,
    message_rx: Receiver<Program::Message>,
    mut settings: WindowDescriptor,
    startup_failure: Arc<Mutex<Option<String>>>,
    shared_gpu: crate::HostedGpuShared,
) -> Result<WindowManager<Program>, String> {
    let store = prepare_primary_descriptor(&mut settings)?;
    let bootstrap = bootstrap_primary_window(
        event_loop,
        &settings,
        shared_gpu,
        Program::gpu_backend_policy(),
        crate::ThemeAppearance::default(),
        Program::startup_window_material_mode(),
    )?;
    let startup =
        startup::HostStartup::settled(crate::SplashOutcome::Skipped(crate::SplashSkip::Embedded));
    complete_startup(
        event_loop,
        StartupChannels {
            proxy,
            message_tx,
            message_rx,
            startup_failure,
        },
        settings,
        store,
        true,
        PrimaryStart {
            bootstrap,
            #[cfg(not(target_os = "android"))]
            accessibility: None,
            painter: None,
            pipeline_cache: None,
            icons: None,
            startup,
        },
    )
}

/// Where the program's messages and a startup failure go.
pub(super) struct StartupChannels<Message> {
    pub(super) proxy: EventLoopProxy,
    pub(super) message_tx: Sender<Message>,
    pub(super) message_rx: Receiver<Message>,
    pub(super) startup_failure: Arc<Mutex<Option<String>>>,
}

/// A primary window bound to its device, and what the startup made for it
/// before the program existed.
pub(super) struct PrimaryStart {
    pub(super) bootstrap: PrimaryBootstrap,
    #[cfg(not(target_os = "android"))]
    pub(super) accessibility: Option<HostedAccessibility>,
    pub(super) painter: Option<(
        crate::ScenePresentationProfile,
        Receiver<startup::StartupPainter>,
    )>,
    pub(super) pipeline_cache: Option<Arc<dyn crate::PipelineCacheStore>>,
    pub(super) icons: Option<Receiver<SceneIcons>>,
    pub(super) startup: startup::HostStartup,
}

/// Everything after the device exists: the program (`UiReady`), its material,
/// the host's window bookkeeping and the first show. Shared by the standalone
/// and the embedded host, so the two cannot drift apart.
pub(super) fn complete_startup<Program: RuntimeProgram>(
    event_loop: &dyn ActiveEventLoop,
    channels: StartupChannels<Program::Message>,
    settings: WindowDescriptor,
    store: SharedStore,
    embedded: bool,
    start: PrimaryStart,
) -> Result<WindowManager<Program>, String> {
    let StartupChannels {
        proxy,
        message_tx,
        message_rx,
        startup_failure,
    } = channels;
    let PrimaryStart {
        bootstrap,
        #[cfg(not(target_os = "android"))]
            accessibility: early_accessibility,
        painter: early_painter,
        pipeline_cache,
        icons,
        startup: host_startup,
    } = start;
    let non_client = nana_window::NonClientRenderingStrategy::from_env();
    let PrimaryBootstrap {
        window,
        graphics,
        mut surface,
        target,
        composition: process_composition,
        mut requested_material,
        applied_material,
    } = bootstrap;
    let mut host_startup = host_startup;
    let pending_native = PendingNativeWindow(Some(Arc::clone(&window)));
    let pending_icons = match icons {
        Some(icons) => match icons.try_recv() {
            Ok(rendered) => {
                rendered.apply(window.as_ref());
                None
            }
            Err(mpsc::TryRecvError::Empty) => Some(icons),
            Err(mpsc::TryRecvError::Disconnected) => {
                apply_scene_window_icon(window.as_ref(), settings.icon.as_ref(), true);
                None
            }
        },
        None => {
            apply_scene_window_icon(window.as_ref(), settings.icon.as_ref(), true);
            None
        }
    };
    let profile = surface.profile().scene_profile();
    let mut presentation = ResolvedWindowPresentation::resolve(
        &settings,
        requested_material,
        applied_material,
        surface.wgpu_alpha_mode(),
        graphics.adapter_info().backend,
        target,
        non_client,
    );
    let host_work = Arc::new(schedule::HostWorkWake::new(proxy.clone()));
    let window_wake = Arc::clone(&host_work);
    let (windows, window_requests) =
        crate::WindowService::channel(Arc::new(move || window_wake.wake()));
    windows.register(WindowId::PRIMARY);
    let startup_wake = Arc::clone(&host_work);
    host_startup
        .handle
        .set_wake(Some(Arc::new(move || startup_wake.wake())));
    let tasks = spawn_task_workers(message_tx.clone(), Arc::clone(&host_work));
    {
        let task_wake = Arc::clone(&host_work);
        nana_ui_runtime::view::set_task_wake(move || task_wake.wake());
    }
    let geometry = window_geometry(window.as_ref());
    let reduced_motion = nana_window::system_reduced_motion().unwrap_or(false);
    let high_contrast = nana_window::system_high_contrast().unwrap_or(false);
    let reduced_transparency = nana_window::system_reduced_transparency().unwrap_or(false);
    {
        let wake = Arc::clone(&host_work);
        nana_window::set_text_scale_waker(move || wake.wake());
    }
    {
        let wake = Arc::clone(&host_work);
        nana_window::set_accessibility_preference_waker(move || wake.wake());
    }
    {
        let wake = Arc::clone(&host_work);
        nana_window::set_display_topology_waker(move || wake.wake());
    }
    let last_displays = display::display_infos(event_loop);
    let text_scale = nana_window::system_text_scale();
    let notifications = nana_window::NotificationHost::new();
    {
        let wake = Arc::clone(&host_work);
        notifications.set_wake(move || wake.wake());
    }
    let context = program_context(
        message_tx.clone(),
        Arc::clone(&host_work),
        &graphics,
        WindowId::PRIMARY,
        geometry,
        tasks.clone(),
        presentation,
        CompositionWork::default(),
        window.theme().map(system_appearance_from_winit),
        &host_startup.handle,
    )
    .with_windows(&windows)
    .with_notifications(&notifications)
    .with_window_tag(settings.tag.clone())
    .with_reduced_motion(reduced_motion)
    .with_high_contrast(high_contrast)
    .with_reduced_transparency(reduced_transparency)
    .with_text_scale(text_scale)
    .with_store(nana_ui_core::ProgramStore::share(Arc::clone(&store)));
    host_startup.ui_ready_begins();
    let (program, startup) = Program::initialize(&context).map_err(|error| error.to_string())?;
    let startup_requests = window_requests;
    let installed_theme = program.theme();
    let primary_theme = program
        .theme_for(WindowId::PRIMARY)
        .unwrap_or_else(|| Arc::clone(&installed_theme));
    let last_theme = primary_theme.appearance();
    let window_themes = HashMap::from([(WindowId::PRIMARY, primary_theme)]);
    let last_material_mode = program.window_material_mode_for(WindowId::PRIMARY);
    let backdrop_opacity = program.appearance_backdrop_opacity_for(WindowId::PRIMARY);
    let window_background = program.window_background();
    let applied;
    (requested_material, applied) = apply_window_material(
        window.as_ref(),
        last_theme,
        &settings,
        last_material_mode,
        backdrop_opacity,
        window_background,
    );
    graphics
        .apply_surface_alpha_mode(
            &mut surface,
            window_wants_transparent_surface(settings.transparent, last_material_mode),
        )
        .map_err(|error| error.to_string())?;
    presentation = ResolvedWindowPresentation::resolve(
        &settings,
        requested_material,
        applied,
        surface.wgpu_alpha_mode(),
        graphics.adapter_info().backend,
        target,
        non_client,
    );
    apply_resolved_presentation(
        window.as_ref(),
        last_theme,
        &settings,
        &presentation,
        backdrop_opacity,
        window_background,
        false,
    );
    #[cfg(not(target_os = "android"))]
    let accessibility = early_accessibility.or_else(|| {
        Some(HostedAccessibility::new(
            Arc::clone(&window),
            true,
            window.scale_factor() as f32,
        ))
    });
    let mut window_ids = HashMap::new();
    window_ids.insert(window.id(), WindowId::PRIMARY);
    let animation_clock = RuntimeAnimationClock::now();
    let level = if settings.always_on_top {
        WindowLevel::AlwaysOnTop
    } else {
        WindowLevel::Normal
    };
    let primary = WindowSessionBuilder {
        role: if embedded {
            WindowRole::PrimaryEmbedded
        } else {
            WindowRole::PrimaryStandalone
        },
        id: WindowId::PRIMARY,
        generation: 1,
        surface,
        geometry,
        presentation,
        settings: settings.clone(),
        #[cfg(not(target_os = "android"))]
        accessibility,
        level,
        requested_level: level,
        underlay: None,
        auxiliary: None,
    }
    .build(&proxy)?;
    pending_native.keep();
    let mut ready = WindowManager {
        program,
        embedded,
        gpu_backend_policy: Program::gpu_backend_policy(),
        non_client,
        composition: process_composition,
        shutting_down: false,
        wake_deadline: None,
        host_work: Arc::clone(&host_work),
        host_work_deadline: None,
        windows,
        window_requests: Some(startup_requests),
        next_window_id: 1 << 63,
        graphics,
        painter: None,
        retained_url_cache: None,
        native_renderers: HashMap::new(),
        text: NanaTextShaper::default(),
        proxy,
        message_tx,
        messages: message_rx,
        file_dialogs: dialogs::FileDialogs::default(),
        color_samplers: color_sampler::ColorSamplers::default(),
        browsers: HashMap::new(),
        web_surfaces: HashMap::new(),
        tasks,
        animation_clock,
        frame_schedules: HashMap::new(),
        window_outputs: HashMap::new(),
        producer_frames: production::WindowProducerCache::default(),
        direct_output_traces: HashMap::new(),
        last_presentation_work: HashMap::new(),
        damage_trackers: HashMap::new(),
        texture_subscriptions: HashMap::new(),
        texture_redraws: Arc::new(Mutex::new(HashSet::new())),
        startup_painter: None,
        pipeline_cache: pipeline_cache.map(pipeline_cache::HostPipelineCache::new),
        material_redraws: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        material_waker: None,
        image_targets: Arc::new(Mutex::new(HashMap::new())),
        image_window_keys: HashMap::new(),
        occluded: HashSet::new(),
        native_content: HashMap::new(),
        native_chrome_writes: std::cell::Cell::new(0),
        window_contexts: HashMap::from([(WindowId::PRIMARY, primary)]),
        window_ids,
        closing_windows: HashSet::new(),
        next_gpu_retry: None,
        render_suspended: false,
        last_theme,
        installed_theme,
        window_themes,
        reduced_motion,
        high_contrast,
        reduced_transparency,
        text_scale,
        settings,
        chrome: HashMap::new(),
        bind_after_present: HashSet::new(),
        startup_failure,
        program_store: nana_ui_core::ProgramStore::share(Arc::clone(&store)),
        store,
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        live_frame_resize: None,
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        live_frame_move: None,
        #[cfg(target_os = "macos")]
        present_transaction_pinned: HashSet::new(),
        #[cfg(target_os = "macos")]
        live_resize_commit: HashMap::new(),
        startup: host_startup,
        input_generations: HashMap::from([(WindowId::PRIMARY, 1)]),
        clipboard: host_services::process_clipboard(),
        cursor_cache: nana_window::cursor::PlatformCursorCache::default(),
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        winit_custom_cursors: HashMap::new(),
        pending_icons,
        notifications,
        last_displays,
        startup_messages: std::collections::VecDeque::new(),
    };
    ready
        .program
        .sync_animation_clock(ready.animation_clock.epoch());
    ready.sync_document_reduced_motion(WindowId::PRIMARY);
    ready.sync_document_eyedropper(WindowId::PRIMARY);
    ready.apply_high_contrast(WindowId::PRIMARY);
    match early_painter {
        Some((built, painter)) if built == profile => {
            ready.startup_painter = Some((built, painter));
            ready.take_startup_painter(false);
        }
        Some((built, painter)) => {
            ready.startup_painter = Some((built, painter));
            ready
                .try_painter_mut(profile)
                .map_err(|error| error.to_string())?;
        }
        None => {
            ready
                .try_painter_mut(profile)
                .map_err(|error| error.to_string())?;
            ready.watch_pipeline_cache();
        }
    }
    ready.prepare_window_chrome(
        WindowId::PRIMARY,
        ready.geometry_of(WindowId::PRIMARY).maximized,
    );
    crate::host_diagnostics::record_adapter(ready.graphics.adapter_info());
    crate::host_diagnostics::window_opened(
        WindowId::PRIMARY,
        &ready.geometry_of(WindowId::PRIMARY),
    );
    let shown_early = ready.startup.shown_early();
    let startup = if shown_early {
        ready.startup_messages = startup.into();
        ready.host_work.wake();
        Vec::new()
    } else {
        startup
    };
    let policy = ready.program.startup_takeover();
    ready.startup_ui_ready(policy);
    let update = ready.program.window_event(
        WindowEvent::Ready {
            id: WindowId::PRIMARY,
            geometry: ready.geometry_of(WindowId::PRIMARY),
        },
        &ready.context(),
    );
    ready.apply_update(event_loop, update, None);
    for message in startup {
        if event_loop.exiting() {
            break;
        }
        ready.process_message(event_loop, message);
    }
    if !event_loop.exiting() && ready.window(WindowId::PRIMARY).is_some() {
        if !shown_early {
            ready.show_unsplashed_primary(window.as_ref());
        }
        ready.reconcile_native_chrome(WindowId::PRIMARY);
        window.request_redraw();
        ready.finish_ready(event_loop, WindowId::PRIMARY);
    }
    Ok(ready)
}

pub(super) fn window_surface_effect(
    settings_transparent: bool,
    appearance: crate::MaterialEffect,
) -> crate::MaterialEffect {
    if settings_transparent {
        crate::MaterialEffect::Transparent
    } else {
        appearance
    }
}

pub(super) fn window_wants_transparent_surface(
    settings_transparent: bool,
    appearance: crate::MaterialEffect,
) -> bool {
    window_surface_effect(settings_transparent, appearance).wants_transparent_surface()
}

fn apply_scene_material(
    window: &dyn winit::window::Window,
    theme: crate::ThemeAppearance,
    requested: crate::MaterialEffect,
    backdrop_opacity: f32,
    window_background: Option<nana_ui_core::SemanticColor>,
) -> MaterialOutcome {
    let appearance = match theme {
        crate::ThemeAppearance::Dark => Appearance::Dark,
        crate::ThemeAppearance::Light => Appearance::Light,
        crate::ThemeAppearance::Custom => Appearance::Light,
    };
    let (red, green, blue, _) = window_background
        .unwrap_or_else(|| theme.palette().background)
        .to_u8_rgba();
    let alpha = (AppearanceSettings::clamp_backdrop_opacity(backdrop_opacity) * 255.0 + 0.5) as u8;
    apply_hosted_system_material(
        window,
        requested,
        appearance,
        FallbackColor::rgba(red, green, blue, alpha),
    )
}

fn apply_window_transparency(window: &dyn winit::window::Window, requested: crate::MaterialEffect) {
    window.set_transparent(requested.wants_transparent_surface());
}

pub(super) fn apply_window_material(
    window: &dyn winit::window::Window,
    theme: crate::ThemeAppearance,
    settings: &WindowDescriptor,
    appearance: crate::MaterialEffect,
    backdrop_opacity: f32,
    window_background: Option<nana_ui_core::SemanticColor>,
) -> (crate::MaterialEffect, MaterialOutcome) {
    let requested = window_surface_effect(settings.transparent, appearance);
    let material = apply_scene_material(
        window,
        theme,
        requested,
        backdrop_opacity,
        window_background,
    );
    apply_window_transparency(window, requested);
    (requested, material)
}

pub(super) fn apply_resolved_presentation(
    window: &dyn winit::window::Window,
    theme: crate::ThemeAppearance,
    settings: &WindowDescriptor,
    presentation: &ResolvedWindowPresentation,
    backdrop_opacity: f32,
    window_background: Option<nana_ui_core::SemanticColor>,
    allow_caption_change: bool,
) {
    if presentation.needs_material_reset() {
        let _ = apply_scene_material(
            window,
            theme,
            presentation.effective().effect,
            backdrop_opacity,
            window_background,
        );
        apply_window_transparency(window, presentation.effective().effect);
    }
    apply_native_chrome(window, settings, presentation, allow_caption_change);
}

pub(super) fn apply_native_chrome<W: HasWindowHandle + ?Sized>(
    window: &W,
    settings: &WindowDescriptor,
    presentation: &ResolvedWindowPresentation,
    allow_caption_change: bool,
) {
    let Some(chrome) = presentation.chrome() else {
        return;
    };
    let _ = prepare_client_chrome(window, f64::from(TITLE_BAR_HEIGHT), chrome.rounded_corners);
    let _ = nana_window::set_non_client_rendering(window, chrome.non_client_rendering_enabled());
    let _ = if allow_caption_change {
        set_frameless_styles(window, chrome.frameless(), settings.resizable)
    } else {
        arm_frameless_guard(window, chrome.frameless())
    };
}

pub(super) fn desktop_scale(own_scale: f64, primary_scale: Option<f64>) -> f64 {
    #[cfg(target_os = "macos")]
    {
        let _ = primary_scale;
        own_scale
    }
    #[cfg(not(target_os = "macos"))]
    primary_scale
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .unwrap_or(own_scale)
}

pub(super) fn window_reference_scale(window: &dyn winit::window::Window) -> Option<f64> {
    window
        .primary_monitor()
        .or_else(|| window.available_monitors().next())
        .map(|monitor| monitor.scale_factor())
}

pub(super) fn desktop_position(position: (f64, f64), scale: f64) -> winit::dpi::Position {
    #[cfg(target_os = "macos")]
    {
        let _ = scale;
        winit::dpi::LogicalPosition::new(position.0, position.1).into()
    }
    #[cfg(not(target_os = "macos"))]
    winit::dpi::PhysicalPosition::new(
        (position.0 * scale).round() as i32,
        (position.1 * scale).round() as i32,
    )
    .into()
}

pub(super) struct Desktop {
    pub(super) displays: Vec<DisplayBounds>,
    pub(super) size_ratios: Vec<f64>,
    pub(super) scale: f64,
}

impl Desktop {
    fn display_index_at(&self, position: (f64, f64)) -> Option<usize> {
        nana_ui_platform::display_index_at(position, &self.displays)
    }

    fn size_ratio_of(&self, index: Option<usize>) -> f64 {
        index
            .and_then(|index| self.size_ratios.get(index).copied())
            .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
            .unwrap_or(1.0)
    }

    fn size_ratio_at(&self, position: (f64, f64)) -> f64 {
        self.size_ratio_of(self.display_index_at(position))
    }

    pub(super) fn logical_size_at(&self, position: (f64, f64)) -> Option<(f64, f64)> {
        let index = self.display_index_at(position)?;
        let (display, ratio) = (&self.displays[index], self.size_ratio_of(Some(index)));
        Some((display.size.0 / ratio, display.size.1 / ratio))
    }
}

fn minimum_within(minimum: (f64, f64), size: (f64, f64)) -> (f64, f64) {
    (minimum.0.min(size.0), minimum.1.min(size.1))
}

pub(super) fn scene_window_attributes(
    settings: &WindowDescriptor,
    desktop: &Desktop,
    target: WindowSurfaceTarget,
) -> winit::window::WindowAttributes {
    let displays = desktop.displays.as_slice();
    let mut settings = settings.clone();
    let requested_size = settings.initial_size;
    if settings.constrain_to_work_area {
        let position = settings.initial_position.unwrap_or_else(|| {
            displays
                .first()
                .map_or((0.0, 0.0), |display| display.position)
        });
        let ratio = desktop.size_ratio_at(position);
        let (position, size) = nana_ui_platform::fit_window_to_displays(
            position,
            (
                settings.initial_size.0 * ratio,
                settings.initial_size.1 * ratio,
            ),
            displays,
        );
        settings.initial_position = Some(position);
        settings.initial_size = (size.0 / ratio, size.1 / ratio);
        settings.minimum_size = minimum_within(settings.minimum_size, settings.initial_size);
    }
    if let Some(ratio) = settings.content_aspect_ratio {
        let at = settings
            .initial_position
            .or_else(|| displays.first().map(|display| display.position));
        let limit = at.and_then(|at| desktop.logical_size_at(at));
        settings.initial_size = nana_ui_platform::aspect_window_size(
            requested_size,
            ratio,
            settings.minimum_size,
            limit,
        );
        settings.minimum_size = nana_ui_platform::aspect_minimum_size(settings.minimum_size, ratio);
        if settings.constrain_to_work_area {
            settings.minimum_size = minimum_within(settings.minimum_size, settings.initial_size);
        }
    }
    let mut attributes = winit::window::WindowAttributes::default()
        .with_title(settings.title.clone())
        .with_transparent(settings.transparent)
        .with_active(settings.focus_on_show)
        .with_resizable(settings.resizable)
        .with_window_level(window_level(settings.always_on_top))
        .with_surface_size(winit::dpi::LogicalSize::new(
            settings.initial_size.0,
            settings.initial_size.1,
        ))
        .with_min_surface_size(winit::dpi::LogicalSize::new(
            settings.minimum_size.0,
            settings.minimum_size.1,
        ))
        .with_maximized(settings.maximized);
    if let Some((x, y)) = settings.initial_position {
        let ratio = desktop.size_ratio_at((x, y));
        let size = (
            settings.initial_size.0 * ratio,
            settings.initial_size.1 * ratio,
        );
        let position = clamp_position_to_displays((x, y), size, displays);
        attributes = attributes.with_position(desktop_position(position, desktop.scale));
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(icon) = resolved_scene_icon(settings.icon.as_ref())
        .as_ref()
        .and_then(winit_icon)
    {
        attributes = attributes.with_window_icon(Some(icon));
    }

    apply_scene_window_chrome(attributes, &settings, target)
}

pub(super) fn scene_aux_window_attributes(
    settings: &WindowDescriptor,
    parent: Option<&dyn winit::window::Window>,
    desktop: &Desktop,
    target: WindowSurfaceTarget,
    initial_level: WindowLevel,
) -> Result<winit::window::WindowAttributes, String> {
    let attributes = scene_window_attributes(settings, desktop, target)
        .with_visible(false)
        .with_window_level(display::winit_level(initial_level));
    if settings.modal && parent.is_none() {
        return Err("modal window requires a parent".into());
    }
    let underlay = settings.role == nana_ui_platform::WindowRole::Underlay;
    let attributes = if underlay {
        attributes.with_decorations(false)
    } else {
        attributes
    };
    Ok(attributes)
}

pub(super) fn scene_desktop(event_loop: &dyn ActiveEventLoop, work_area: bool) -> Desktop {
    let infos = display::display_infos(event_loop);
    let primary = infos
        .iter()
        .find(|display| display.primary)
        .or(infos.first())
        .map(|display| display.scale_factor);
    let scale = desktop_scale(
        infos.first().map_or(1.0, |display| display.scale_factor),
        primary,
    );
    let (displays, size_ratios) = infos
        .into_iter()
        .filter_map(|mut display| {
            if work_area
                && let Some((position, size)) = display
                    .physical_position
                    .and_then(nana_window::display_work_area)
            {
                display.physical_position = Some(position);
                display.physical_size = Some(size);
            }
            let display_desktop_scale = desktop_scale(display.scale_factor, primary);
            display
                .logical_bounds(display_desktop_scale)
                .map(|bounds| (bounds, display.scale_factor / display_desktop_scale))
        })
        .unzip();
    Desktop {
        displays,
        size_ratios,
        scale,
    }
}

pub(super) fn resolved_scene_icon(per_window: Option<&WindowIcon>) -> Option<WindowIcon> {
    #[cfg(not(feature = "no-default-app-icon"))]
    return Some(nana_app_icon::resolved_application_icon(per_window));
    #[cfg(feature = "no-default-app-icon")]
    return nana_ui_platform::resolve_window_icon(per_window);
}

pub(super) fn winit_icon(icon: &WindowIcon) -> Option<Icon> {
    RgbaIcon::new(icon.rgba.clone(), icon.width, icon.height)
        .ok()
        .map(Icon::from)
}

pub(super) fn apply_scene_window_icon(
    window: &dyn winit::window::Window,
    per_window: Option<&WindowIcon>,
    apply_app_icon: bool,
) {
    SceneIcons::render(per_window, apply_app_icon).apply(window);
}

pub(super) fn apply_application_icon() {
    #[cfg(target_os = "macos")]
    if let Some(png) = SceneIcons::render(None, true).application_png {
        set_application_icon_png(&png);
    }
}

pub(super) struct SceneIcons {
    window: Option<WindowIcon>,
    #[cfg(target_os = "macos")]
    application_png: Option<Vec<u8>>,
}

impl SceneIcons {
    pub(super) fn render(per_window: Option<&WindowIcon>, application: bool) -> Self {
        let window = resolved_scene_icon(per_window);
        #[cfg(target_os = "macos")]
        let application_png = window.as_ref().filter(|_| application).and_then(|window| {
            let icon = nana_app_icon::with_system_grid(window);
            nana_app_icon::encode_png(icon.width, icon.height, &icon.rgba).ok()
        });
        #[cfg(not(target_os = "macos"))]
        let _ = application;
        Self {
            window,
            #[cfg(target_os = "macos")]
            application_png,
        }
    }

    pub(super) fn apply(&self, window: &dyn winit::window::Window) {
        window.set_window_icon(self.window.as_ref().and_then(winit_icon));
        #[cfg(target_os = "windows")]
        window.set_taskbar_icon(self.window.as_ref().and_then(winit_icon));
        #[cfg(target_os = "macos")]
        if let Some(png) = self.application_png.as_deref() {
            set_application_icon_png(png);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(all(target_os = "macos", not(test)), allow(dead_code))]
pub(super) struct WindowsSceneChrome {
    pub(super) decorations: bool,
    pub(super) undecorated_shadow: bool,
    pub(super) no_redirection_bitmap: bool,
    pub(super) rounded_corners: bool,
}

#[cfg_attr(all(target_os = "macos", not(test)), allow(dead_code))]
pub(super) fn windows_scene_chrome(
    system_caption: bool,
    transparent: bool,
    target: WindowSurfaceTarget,
) -> WindowsSceneChrome {
    WindowsSceneChrome {
        decorations: system_caption,
        undecorated_shadow: false,
        no_redirection_bitmap: target.composed(),
        rounded_corners: !system_caption && !transparent,
    }
}

pub(super) const fn surface_mode_for(target: WindowSurfaceTarget) -> crate::HostedSurfaceMode {
    #[cfg(target_os = "windows")]
    {
        match target {
            WindowSurfaceTarget::Composition => crate::HostedSurfaceMode::WindowsComposition,
            WindowSurfaceTarget::NativeWindow => crate::HostedSurfaceMode::Window,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = target;
        crate::HostedSurfaceMode::Window
    }
}

fn apply_scene_window_chrome(
    attributes: winit::window::WindowAttributes,
    settings: &WindowDescriptor,
    target: WindowSurfaceTarget,
) -> winit::window::WindowAttributes {
    #[cfg(target_os = "macos")]
    {
        let _ = target;
        if settings.system_caption {
            return attributes.with_decorations(true);
        }
        attributes
            .with_decorations(true)
            .with_platform_attributes(Box::new(
                WindowAttributesMacOS::default()
                    .with_titlebar_transparent(true)
                    .with_fullsize_content_view(true)
                    .with_title_hidden(true)
                    .with_movable_by_window_background(false),
            ))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let chrome = windows_scene_chrome(settings.system_caption, settings.transparent, target);
        let attributes = attributes.with_decorations(chrome.decorations);
        #[cfg(target_os = "windows")]
        let attributes = {
            let mut win = WindowAttributesWindows::default()
                .with_no_redirection_bitmap(chrome.no_redirection_bitmap)
                .with_undecorated_shadow(chrome.undecorated_shadow);
            if !chrome.decorations {
                win = win.with_corner_preference(if chrome.rounded_corners {
                    CornerPreference::Round
                } else {
                    CornerPreference::DoNotRound
                });
            }
            if let Some(icon) = resolved_scene_icon(settings.icon.as_ref())
                .as_ref()
                .and_then(winit_icon)
            {
                win = win.with_taskbar_icon(Some(icon));
            }
            attributes.with_platform_attributes(Box::new(win))
        };
        attributes
    }
}

pub(super) fn window_level(always_on_top: bool) -> winit::window::WindowLevel {
    if always_on_top {
        winit::window::WindowLevel::AlwaysOnTop
    } else {
        winit::window::WindowLevel::Normal
    }
}

pub(super) fn window_geometry(window: &dyn winit::window::Window) -> WindowGeometry {
    let scale_factor = normalized_scale_factor(window.scale_factor() as f32);
    let physical_size = window.surface_size();
    let physical_position = window.outer_position().ok();
    let desktop = desktop_scale(f64::from(scale_factor), window_reference_scale(window));
    WindowGeometry {
        physical_position: physical_position.map(|position| (position.x, position.y)),
        physical_size: (physical_size.width, physical_size.height),
        logical_position: physical_position.map(|position| {
            let logical = position.to_logical::<f32>(desktop);
            (logical.x, logical.y)
        }),
        logical_size: (
            physical_size.width as f32 / scale_factor,
            physical_size.height as f32 / scale_factor,
        ),
        scale_factor,
        maximized: geometry_maximized(window),
    }
}

#[cfg(target_os = "macos")]
pub(super) fn geometry_maximized(window: &dyn winit::window::Window) -> bool {
    if covers_display(window) {
        false
    } else {
        nana_window::native_window_maximized(window).unwrap_or(false)
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn geometry_maximized(window: &dyn winit::window::Window) -> bool {
    !covers_display(window) && window.is_maximized()
}

pub(super) fn covers_display(window: &dyn winit::window::Window) -> bool {
    #[cfg(target_os = "macos")]
    if WindowExtMacOS::simple_fullscreen(window) {
        return true;
    }
    window.fullscreen().is_some() || nana_window::work_area_fullscreen(window)
}
