//! Core `WindowManager` lifecycle, image-target index, program context, and command routing.

use super::*;
use crate::winit_input::{ScreenSpace, system_appearance_from_winit};

impl<Program: RuntimeProgram> Drop for WindowManager<Program> {
    fn drop(&mut self) {
        // App destructors may join workers waiting for window requests. Resolve
        // those requests before Rust drops `program` (the first field).
        self.window_requests.take();
        // Shutdown is the bounded final-flush lane. Normal UI updates only
        // advance the coordinator generation and never synchronously flush.
        let _ = self.store.flush();
        if let Some(cache) = &mut self.pipeline_cache {
            cache.finish();
        }
    }
}

pub(super) fn replace_image_target_index(
    targets: &mut HashMap<Arc<str>, HashSet<WindowId>>,
    window_keys: &mut HashMap<WindowId, (u64, Arc<[Arc<str>]>)>,
    id: WindowId,
    instance_id: u64,
    keys: Arc<[Arc<str>]>,
) {
    let previous = window_keys
        .insert(id, (instance_id, Arc::clone(&keys)))
        .map(|(_, urls)| urls);
    if let Some(previous) = previous {
        for key in previous.iter() {
            if let Some(ids) = targets.get_mut(key) {
                ids.remove(&id);
                if ids.is_empty() {
                    targets.remove(key);
                }
            }
        }
    }
    for key in keys.iter() {
        targets.entry(Arc::clone(key)).or_default().insert(id);
    }
}

pub(super) fn remove_image_target_index(
    targets: &mut HashMap<Arc<str>, HashSet<WindowId>>,
    window_keys: &mut HashMap<WindowId, (u64, Arc<[Arc<str>]>)>,
    id: WindowId,
) {
    let Some((_, previous)) = window_keys.remove(&id) else {
        return;
    };
    for key in previous.iter() {
        if let Some(ids) = targets.get_mut(key) {
            ids.remove(&id);
            if ids.is_empty() {
                targets.remove(key);
            }
        }
    }
}

impl<Program: RuntimeProgram> WindowManager<Program> {
    /// Updates the per-URL window wake index from the scene's precomputed
    /// `FramePlan::image_urls()`, skipping unchanged scene instances and URL
    /// slices with zero primitive scans or `HashSet<String>` allocations.
    pub(super) fn update_image_targets(
        &mut self,
        id: WindowId,
        instance_id: u64,
        plan: Option<&nana_ui_scene::FramePlan>,
    ) {
        let Some(plan) = plan else {
            return;
        };
        let keys = &plan.image_urls;
        if let Some((prev_instance, prev_urls)) = self.image_window_keys.get_mut(&id) {
            if *prev_instance == instance_id {
                return;
            }
            if Arc::ptr_eq(prev_urls, keys) || prev_urls.as_ref() == keys.as_ref() {
                *prev_instance = instance_id;
                return;
            }
        }
        let keys = Arc::clone(keys);
        if let Ok(mut targets) = self.image_targets.lock() {
            replace_image_target_index(
                &mut targets,
                &mut self.image_window_keys,
                id,
                instance_id,
                keys,
            );
        } else {
            self.image_window_keys.insert(id, (instance_id, keys));
        }
    }

    pub(super) fn context(&self) -> RuntimeProgramContext<Program::Message> {
        self.context_for(
            self.window_contexts
                .keys()
                .copied()
                .min()
                .unwrap_or(WindowId::PRIMARY),
        )
    }

    pub(super) fn context_for(&self, id: WindowId) -> RuntimeProgramContext<Program::Message> {
        program_context(
            self.message_tx.clone(),
            Arc::clone(&self.host_work),
            &self.graphics,
            id,
            self.geometry_of(id),
            self.tasks.clone(),
            self.presentation_of(id),
            self.composition_work_of(id),
            self.window(id)
                .and_then(|w| w.theme())
                .map(system_appearance_from_winit),
            &self.startup.handle,
        )
        .with_windows(&self.windows)
        .with_window_tag(
            self.window_contexts
                .get(&id)
                .and_then(|host| host.settings.tag.clone()),
        )
        .with_reduced_motion(self.reduced_motion)
        .with_high_contrast(self.high_contrast)
        .with_reduced_transparency(self.reduced_transparency)
        .with_text_scale(self.text_scale)
        .with_content_zoom(self.content_zoom_of(id))
        .with_presentation_work(
            self.last_presentation_work
                .get(&id)
                .copied()
                .unwrap_or_default(),
        )
        .with_shadow_work(
            self.window_contexts
                .get(&id)
                .map(|host| crate::window_shadow::work(&host.shadow))
                .unwrap_or_default(),
        )
        .with_store(Arc::clone(&self.program_store))
    }

    pub(super) fn apply_update(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        update: RuntimeProgramUpdate,
        painting: Option<WindowId>,
    ) {
        if self.shutting_down {
            return;
        }
        if update.exit {
            self.shutting_down = true;
            self.window_requests.take();
            self.browsers.clear();
            self.web_surfaces.clear();
            self.close_all_file_dialogs();
            self.close_all_color_samplers();
            let ids = self.known_window_ids();
            for id in ids {
                self.close_window(event_loop, id);
            }
            if !self.embedded {
                event_loop.exit();
            }
            return;
        }
        for command in update.window_commands {
            self.apply_window_command(event_loop, command);
            if event_loop.exiting() {
                return;
            }
        }
        self.reconcile_browser_lifetimes();
        self.sync_web_surfaces();
        for id in self.known_window_ids() {
            self.sync_window_output(id);
        }
        for id in windows_to_redraw(update.redraw, &self.known_window_ids()) {
            if painting == Some(id) {
                continue;
            }
            self.request_redraw(id);
        }
        self.request_reactive_redraws(painting);
    }

    pub(super) fn request_reactive_redraws(&mut self, painting: Option<WindowId>) {
        for id in self.known_window_ids() {
            if painting != Some(id)
                && self
                    .program
                    .read_document(id, |document| document.context().has_pending_reactive())
                    .unwrap_or(false)
            {
                self.request_redraw(id);
            }
        }
    }

    pub(super) fn known_window_ids(&self) -> Vec<WindowId> {
        self.window_contexts.keys().copied().collect()
    }

    pub(super) fn window(&self, id: WindowId) -> Option<&Arc<dyn winit::window::Window>> {
        self.window_contexts
            .get(&id)
            .map(|host| host.surface.window())
    }

    pub(super) fn geometry_of(&self, id: WindowId) -> WindowGeometry {
        self.window_contexts
            .get(&id)
            .map(|host| host.geometry)
            .unwrap_or_default()
    }

    pub(super) fn content_zoom_of(&self, id: WindowId) -> f32 {
        self.window_contexts
            .get(&id)
            .map_or(1.0, |host| host.content_zoom)
    }

    pub(super) fn content_geometry_of(&self, id: WindowId) -> WindowGeometry {
        self.geometry_of(id).content(self.content_zoom_of(id))
    }

    pub(super) fn output_geometry_of(&self, id: WindowId) -> WindowGeometry {
        self.window_contexts
            .get(&id)
            .map(|host| schedule::output_geometry(host.geometry, host.drawable_geometry))
            .unwrap_or_default()
            .content(self.content_zoom_of(id))
    }

    pub(super) fn presentation_of(&self, id: WindowId) -> ResolvedWindowPresentation {
        self.window_contexts
            .get(&id)
            .map_or_else(ResolvedWindowPresentation::closed, |host| host.presentation)
    }

    pub(super) fn material_of(&self, id: WindowId) -> MaterialOutcome {
        self.presentation_of(id).effective()
    }

    pub(super) fn composition_work_of(&self, id: WindowId) -> CompositionWork {
        #[allow(unused_mut)]
        let mut work = CompositionWork {
            native_content: self
                .native_content
                .get(&id)
                .map(crate::native_content::NativeContentMirror::work)
                .unwrap_or_default(),
            commits: 0,
            tree_mutations: 0,
            native_chrome_writes: self.native_chrome_writes.get(),
        };
        #[cfg(target_os = "windows")]
        if let Some(composition) = self
            .window_contexts
            .get(&id)
            .and_then(|host| host.surface.windows_composition())
        {
            let composed = composition.work();
            work.commits = composed.commits;
            work.tree_mutations = composed.tree_mutations;
        }
        #[cfg(not(target_os = "windows"))]
        let _ = id;
        work
    }

    pub(super) fn request_redraw(&self, id: WindowId) {
        if let Some(window) = self.window(id) {
            window.request_redraw();
        }
    }

    pub(super) fn request_redraw_all(&self) {
        for id in self.known_window_ids() {
            self.request_redraw(id);
        }
    }

    pub(super) fn record_presentation_work(
        &mut self,
        id: WindowId,
        output_work: crate::OutputWorkObservation,
        gpu_work: Option<crate::GpuWorkObservation>,
    ) {
        let mut work = crate::PresentationWork::default();
        work.record_output_work(output_work);
        if let Some(gpu_work) = gpu_work {
            work.record_gpu_work(gpu_work);
        }
        self.last_presentation_work.insert(id, work);
        self.program.write_document(id, |document| {
            let context = document.context_mut();
            context.record_output_work(output_work);
            context.record_presentation_gpu_work(gpu_work);
        });
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Host-owned resources form one callback context"
)]
pub(super) fn program_context<Message: Send + 'static>(
    message_tx: Sender<Message>,
    wake: Arc<schedule::HostWorkWake>,
    graphics: &crate::HostedGpuShared,
    id: WindowId,
    geometry: WindowGeometry,
    tasks: SyncSender<Task<Message>>,
    presentation: ResolvedWindowPresentation,
    composition_work: CompositionWork,
    appearance: Option<nana_ui_platform::SystemAppearance>,
    startup: &crate::StartupHandle,
) -> RuntimeProgramContext<Message> {
    RuntimeProgramContext::new(
        id,
        geometry,
        graphics.gpu().clone(),
        presentation,
        composition_work,
        Arc::new(move |message| {
            if message_tx.send(message).is_ok() {
                wake.wake();
            }
        }),
        tasks,
        appearance,
        startup.clone(),
    )
    .with_gpu_adapters(graphics.adapter_selection())
}

pub(super) fn accessibility_snapshot<Program: RuntimeProgram>(
    program: &mut Program,
    id: WindowId,
) -> Vec<nana_ui_runtime::AccessibilityNode> {
    program
        .read_document(id, |document| {
            document
                .context()
                .world()
                .project_accessibility(document.document())
        })
        .unwrap_or_default()
}

#[cfg(not(target_os = "android"))]
pub(super) fn accessibility_world_generation<Program: RuntimeProgram>(
    program: &mut Program,
    id: WindowId,
) -> Option<u64> {
    program.read_document(id, |document| document.context().world().generation())
}

#[cfg(not(target_os = "android"))]
pub(super) fn next_accessibility_update(
    flush: Option<AccessibilityUpdate>,
    program: Option<AccessibilityUpdate>,
    scale_factor_changed: bool,
    projector_generation: Option<u64>,
    world_generation: Option<u64>,
    snapshot: impl FnOnce() -> Vec<nana_ui_runtime::AccessibilityNode>,
) -> Option<AccessibilityUpdate> {
    if scale_factor_changed {
        return Some(AccessibilityUpdate::Full {
            generation: world_generation,
            nodes: snapshot(),
        });
    }
    if flush.is_some() && program.is_some() {
        return Some(AccessibilityUpdate::Full {
            generation: world_generation,
            nodes: snapshot(),
        });
    }
    if let Some(update) = flush.or(program) {
        let queued = match &update {
            AccessibilityUpdate::Full { generation, .. } => *generation,
            AccessibilityUpdate::Delta(delta) => Some(delta.generation),
        };
        if (projector_generation.is_none() && matches!(&update, AccessibilityUpdate::Delta(_)))
            || world_generation.is_some_and(|world| queued.is_some_and(|queued| queued < world))
        {
            return Some(AccessibilityUpdate::Full {
                generation: world_generation,
                nodes: snapshot(),
            });
        }
        return Some(update);
    }
    if projector_generation.is_some() && projector_generation == world_generation {
        return None;
    }
    Some(AccessibilityUpdate::Full {
        generation: world_generation,
        nodes: snapshot(),
    })
}

pub(super) fn drag_scene_window(
    window: &dyn winit::window::Window,
) -> Result<(), winit::error::RequestError> {
    if covers_display(window) {
        return Err(winit::error::RequestError::Ignored);
    }
    if nana_window::drag_custom_title_bar(window) {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        Err(winit::error::RequestError::Ignored)
    }
    #[cfg(not(target_os = "macos"))]
    {
        window.drag_window()
    }
}

pub(super) fn resize_scene_window(window: &dyn winit::window::Window, edge: WindowResizeEdge) {
    if covers_display(window) {
        return;
    }
    if resize_custom_frame(window, frame_resize_edge(edge)) {
        return;
    }
    let _ = window.drag_resize_window(match edge {
        WindowResizeEdge::North => winit::window::ResizeDirection::North,
        WindowResizeEdge::South => winit::window::ResizeDirection::South,
        WindowResizeEdge::East => winit::window::ResizeDirection::East,
        WindowResizeEdge::West => winit::window::ResizeDirection::West,
        WindowResizeEdge::NorthEast => winit::window::ResizeDirection::NorthEast,
        WindowResizeEdge::NorthWest => winit::window::ResizeDirection::NorthWest,
        WindowResizeEdge::SouthEast => winit::window::ResizeDirection::SouthEast,
        WindowResizeEdge::SouthWest => winit::window::ResizeDirection::SouthWest,
    });
}

pub(super) fn frame_resize_edge(edge: WindowResizeEdge) -> FrameResizeEdge {
    match edge {
        WindowResizeEdge::North => FrameResizeEdge::North,
        WindowResizeEdge::South => FrameResizeEdge::South,
        WindowResizeEdge::East => FrameResizeEdge::East,
        WindowResizeEdge::West => FrameResizeEdge::West,
        WindowResizeEdge::NorthEast => FrameResizeEdge::NorthEast,
        WindowResizeEdge::NorthWest => FrameResizeEdge::NorthWest,
        WindowResizeEdge::SouthEast => FrameResizeEdge::SouthEast,
        WindowResizeEdge::SouthWest => FrameResizeEdge::SouthWest,
    }
}

pub(super) fn frame_resize_edge_for(
    settings: &WindowDescriptor,
    geometry: &WindowGeometry,
    fullscreen: bool,
    x: f32,
    y: f32,
) -> Option<WindowResizeEdge> {
    if settings.system_caption || !settings.resizable || geometry.maximized || fullscreen {
        return None;
    }
    window_resize_edge(geometry.logical_size, x, y, RESIZE_HANDLE_SIZE)
}

pub(super) fn window_cursor_override(
    cursor: &crate::WindowCursor,
) -> (Option<super::host_services::NativeCursor>, Option<bool>) {
    use super::host_services::NativeCursor;
    use crate::WindowCursor;
    match cursor {
        WindowCursor::Automatic => (None, None),
        WindowCursor::Default => (Some(NativeCursor::System(CursorIcon::Default)), None),
        WindowCursor::Pointer => (Some(NativeCursor::System(CursorIcon::Pointer)), None),
        WindowCursor::Text => (Some(NativeCursor::System(CursorIcon::Text)), None),
        WindowCursor::Move => (Some(NativeCursor::System(CursorIcon::Move)), None),
        WindowCursor::Grab => (Some(NativeCursor::System(CursorIcon::Grab)), None),
        WindowCursor::Grabbing => (Some(NativeCursor::System(CursorIcon::Grabbing)), None),
        WindowCursor::NotAllowed => (Some(NativeCursor::System(CursorIcon::NotAllowed)), None),
        WindowCursor::Crosshair => (Some(NativeCursor::System(CursorIcon::Crosshair)), None),
        WindowCursor::Help => (Some(NativeCursor::System(CursorIcon::Help)), None),
        WindowCursor::Wait => (Some(NativeCursor::System(CursorIcon::Wait)), None),
        WindowCursor::Progress => (Some(NativeCursor::System(CursorIcon::Progress)), None),
        WindowCursor::ZoomIn => (Some(NativeCursor::System(CursorIcon::ZoomIn)), None),
        WindowCursor::ZoomOut => (Some(NativeCursor::System(CursorIcon::ZoomOut)), None),
        WindowCursor::None => (Some(NativeCursor::System(CursorIcon::Default)), Some(false)),
        WindowCursor::Custom(spec) => {
            if spec.fallback() == nana_ui_core::SystemCursor::None && !spec.is_valid() {
                (Some(NativeCursor::System(CursorIcon::Default)), Some(false))
            } else {
                (Some(NativeCursor::Custom(spec.clone())), None)
            }
        }
    }
}

pub(super) fn frame_edge_cursor(edge: WindowResizeEdge) -> CursorIcon {
    match edge {
        WindowResizeEdge::East | WindowResizeEdge::West => CursorIcon::EwResize,
        WindowResizeEdge::North | WindowResizeEdge::South => CursorIcon::NsResize,
        WindowResizeEdge::NorthEast | WindowResizeEdge::SouthWest => CursorIcon::NeswResize,
        WindowResizeEdge::NorthWest | WindowResizeEdge::SouthEast => CursorIcon::NwseResize,
    }
}

pub(super) fn scene_paint_viewport(
    geometry: &WindowGeometry,
    material: MaterialOutcome,
    theme: &nana_ui_core::CompiledTheme,
    window_background: Option<nana_ui_core::SemanticColor>,
) -> ScenePaintViewport {
    ScenePaintViewport {
        logical_size: [geometry.logical_size.0, geometry.logical_size.1],
        physical_size: [geometry.physical_size.0, geometry.physical_size.1],
        scale_factor: geometry.scale_factor,
        scene_origin: [0.0, 0.0],
        target_origin: [0.0, 0.0],
        clear_color: scene_clear_color_for_theme(theme, material, window_background),
        clear: true,
    }
}

fn scene_clear_color_for_theme(
    theme: &nana_ui_core::CompiledTheme,
    material: MaterialOutcome,
    window_background: Option<nana_ui_core::SemanticColor>,
) -> [f32; 4] {
    if material.wants_transparent_surface() {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let color = window_background.unwrap_or(theme.style_model().palette.background);
    crate::scene_paint::pack_linear([color.r, color.g, color.b, color.a])
}

#[cfg(test)]
pub(super) fn scene_clear_color(
    theme: crate::ThemeAppearance,
    material: MaterialOutcome,
    window_background: Option<nana_ui_core::SemanticColor>,
) -> [f32; 4] {
    if material.wants_transparent_surface() {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let color = window_background.unwrap_or_else(|| theme.palette().background);
    crate::scene_paint::pack_linear([color.r, color.g, color.b, color.a])
}

pub(super) fn allows_modal_parent_event(event: &WinitWindowEvent) -> bool {
    matches!(
        event,
        WinitWindowEvent::RedrawRequested
            | WinitWindowEvent::SurfaceResized(_)
            | WinitWindowEvent::Moved(_)
            | WinitWindowEvent::ScaleFactorChanged { .. }
            | WinitWindowEvent::Occluded(_)
            | WinitWindowEvent::Destroyed
            | WinitWindowEvent::Focused(false)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ForwardPointerAction {
    Dispatch,
    RestorePassthrough,
    IgnoreUntilRecovered,
}

pub(super) fn forward_pointer_event(event: &WinitWindowEvent) -> bool {
    matches!(
        event,
        WinitWindowEvent::PointerMoved { .. }
            | WinitWindowEvent::PointerEntered { .. }
            | WinitWindowEvent::PointerLeft { .. }
            | WinitWindowEvent::PointerButton { .. }
    )
}

pub(super) fn forward_pointer_action(
    mode: MousePassthroughMode,
    os_passthrough: bool,
    hits_content: bool,
    gesture_active: bool,
    event: &WinitWindowEvent,
) -> ForwardPointerAction {
    if !forward_pointer_event(event) {
        return ForwardPointerAction::Dispatch;
    }
    if os_passthrough {
        return ForwardPointerAction::IgnoreUntilRecovered;
    }
    if mode != MousePassthroughMode::Forward {
        return ForwardPointerAction::Dispatch;
    }
    if gesture_active {
        return ForwardPointerAction::Dispatch;
    }
    match event {
        WinitWindowEvent::PointerLeft { .. } => ForwardPointerAction::RestorePassthrough,
        WinitWindowEvent::PointerMoved { .. }
        | WinitWindowEvent::PointerEntered { .. }
        | WinitWindowEvent::PointerButton { .. } => {
            if hits_content {
                ForwardPointerAction::Dispatch
            } else {
                ForwardPointerAction::RestorePassthrough
            }
        }
        _ => ForwardPointerAction::Dispatch,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RoutedWindowCommand {
    SetMousePassthrough(WindowId, MousePassthroughMode),
    SetSkipTaskbar(WindowId, bool),
    Open(WindowId),
    Focus(WindowId),
    Close(WindowId),
    SetTitle(WindowId),
    Move(WindowId),
    SetBounds(WindowId),
    SetFullscreen(WindowId),
    SetMinimized(WindowId),
    SetMaximized(WindowId),
    SetAlwaysOnTop(WindowId),
    SetContentAspectRatio(WindowId),
    SetNativeWindowControlsVisible(WindowId),
    SetIcon(WindowId),
    SetMenuBar(WindowId),
    OpenFileDialog(WindowId),
    SetApplicationIcon,
    Drag(WindowId),
    SetShadow(WindowId),
    SetContentZoom(WindowId),
    SetSizeLimits(WindowId),
    Ignore,
}

pub(super) fn route_window_command(
    command: &WindowCommand,
    known: &[WindowId],
) -> RoutedWindowCommand {
    let known = |id: WindowId| known.contains(&id);
    match command {
        WindowCommand::SetMousePassthrough { id, enabled } => {
            RoutedWindowCommand::SetMousePassthrough(
                *id,
                MousePassthroughMode::passthrough(*enabled),
            )
        }
        WindowCommand::SetMousePassthroughForward { id, enabled } => {
            RoutedWindowCommand::SetMousePassthrough(*id, MousePassthroughMode::forward(*enabled))
        }
        WindowCommand::SetSkipTaskbar { id, skip_taskbar } => {
            RoutedWindowCommand::SetSkipTaskbar(*id, *skip_taskbar)
        }
        WindowCommand::Open { id, .. } if known(*id) => RoutedWindowCommand::Focus(*id),
        WindowCommand::Open { id, .. } => RoutedWindowCommand::Open(*id),
        WindowCommand::Close(id) if !known(*id) => RoutedWindowCommand::Ignore,
        WindowCommand::Close(id) => RoutedWindowCommand::Close(*id),
        WindowCommand::Focus(id) if known(*id) => RoutedWindowCommand::Focus(*id),
        WindowCommand::SetTitle { id, .. } if known(*id) => RoutedWindowCommand::SetTitle(*id),
        WindowCommand::Move { id, .. } if known(*id) => RoutedWindowCommand::Move(*id),
        WindowCommand::SetBounds { id, .. } if known(*id) => RoutedWindowCommand::SetBounds(*id),
        WindowCommand::SetFullscreen { id, .. } if known(*id) => {
            RoutedWindowCommand::SetFullscreen(*id)
        }
        WindowCommand::SetMinimized { id, .. } if known(*id) => {
            RoutedWindowCommand::SetMinimized(*id)
        }
        WindowCommand::SetMaximized { id, .. } if known(*id) => {
            RoutedWindowCommand::SetMaximized(*id)
        }
        WindowCommand::SetAlwaysOnTop { id, .. } if known(*id) => {
            RoutedWindowCommand::SetAlwaysOnTop(*id)
        }
        WindowCommand::SetContentAspectRatio { id, .. } if known(*id) => {
            RoutedWindowCommand::SetContentAspectRatio(*id)
        }
        WindowCommand::SetNativeWindowControlsVisible { id, .. } if known(*id) => {
            RoutedWindowCommand::SetNativeWindowControlsVisible(*id)
        }
        WindowCommand::SetIcon { id, .. } if known(*id) => RoutedWindowCommand::SetIcon(*id),
        WindowCommand::SetMenuBar { id, .. } if known(*id) => RoutedWindowCommand::SetMenuBar(*id),
        WindowCommand::OpenFileDialog { id, .. } => RoutedWindowCommand::OpenFileDialog(*id),
        WindowCommand::SetApplicationIcon { .. } => RoutedWindowCommand::SetApplicationIcon,
        WindowCommand::Drag(id) if known(*id) => RoutedWindowCommand::Drag(*id),
        WindowCommand::SetShadow { id, .. } if known(*id) => RoutedWindowCommand::SetShadow(*id),
        WindowCommand::SetContentZoom { id, .. } if known(*id) => {
            RoutedWindowCommand::SetContentZoom(*id)
        }
        WindowCommand::SetSizeLimits { id, .. } if known(*id) => {
            RoutedWindowCommand::SetSizeLimits(*id)
        }
        _ => RoutedWindowCommand::Ignore,
    }
}

pub(super) fn windows_to_redraw(redraw: RuntimeRedraw, known: &[WindowId]) -> Vec<WindowId> {
    match redraw {
        RuntimeRedraw::None => Vec::new(),
        RuntimeRedraw::Window(id) => known.iter().copied().filter(|known| *known == id).collect(),
        RuntimeRedraw::All => known.to_vec(),
        RuntimeRedraw::Windows(ids) => known
            .iter()
            .copied()
            .filter(|id| ids.contains(id))
            .collect(),
    }
}

pub(super) fn invalidate_program_host_textures(
    window_ids: impl IntoIterator<Item = WindowId>,
    mut host_textures: impl FnMut(WindowId) -> Option<HostTextureRegistry>,
) -> usize {
    let mut invalidated = 0;
    for id in window_ids {
        if let Some(registry) = host_textures(id) {
            invalidated += registry.invalidate_all();
        }
    }
    invalidated
}

pub(super) fn scene_runtime_input_update(
    disposition: nana_ui_platform::InputDisposition,
    id: WindowId,
    program_input: Result<RuntimeProgramUpdate, FrameworkError>,
) -> RuntimeProgramUpdate {
    let program_update = program_input.unwrap_or_default();
    if disposition.prevent_default {
        RuntimeProgramUpdate::redraw(id).merge(program_update)
    } else {
        program_update
    }
}

pub(super) fn window_screen_origin(
    window: &dyn winit::window::Window,
    content_zoom: f32,
) -> Option<ScreenSpace> {
    let own = window.scale_factor().max(0.01);
    let scale = desktop_scale(own, window_reference_scale(window));
    window.outer_position().ok().map(|outer| {
        let surface = window.surface_position();
        let position = winit::dpi::PhysicalPosition::new(outer.x + surface.x, outer.y + surface.y);
        let origin = position.to_logical::<f32>(scale);
        ScreenSpace {
            origin: (origin.x, origin.y),
            client_ratio: (own / scale) as f32 * content_zoom,
        }
    })
}

pub(super) fn normalized_scale_factor(scale_factor: f32) -> f32 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}
