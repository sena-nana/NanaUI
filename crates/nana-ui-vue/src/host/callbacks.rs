//! Vue host callbacks boundary.

use crate::*;

#[derive(Debug, Default)]
pub(crate) struct State {
    pub(crate) fire_event: Option<JsFunctionId>,
    pub(crate) drain_timers: Option<JsFunctionId>,
    pub(crate) drain_fetch: Option<JsFunctionId>,
    pub(crate) drain_ws: Option<JsFunctionId>,
    pub(crate) apply_theme: Option<JsFunctionId>,
    /// Optional web-api ResizeObserver flush after layout (`__nanaNotifyLayout`).
    pub(crate) notify_layout: Option<JsFunctionId>,
    /// Host → JS CSS motion completion (`__nanaMotionComplete`). Not WAAPI.
    pub(crate) motion_complete: Option<JsFunctionId>,
    /// Host → JS cancel of the class-armed motion fallback (`__nanaMotionCancel`).
    pub(crate) motion_cancel: Option<JsFunctionId>,
    /// Optional window/document lifecycle pump (`__nanaPumpLifecycle`).
    pub(crate) lifecycle_pump: Option<JsFunctionId>,
    /// Auxiliary-window identity. `None` preserves the original primary-window
    /// three-argument event bridge.
    pub(crate) event_window_id: Option<u64>,
}

impl VueHost {
    /// Binds an engine-agnostic JS runtime and installs renderer + web-api host ops.
    pub fn attach_engine<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
    ) -> Result<(), JsEngineError> {
        let api = self.host_api_registry();
        engine.register_host_api(&api)?;
        Ok(())
    }
    /// Initialize engine with web-api shim prepended to `artifact`.
    ///
    /// Binary Release artifacts ([`RuntimeArtifact::is_binary_release`]) must already
    /// include the shim (compile after `compose_runtime_artifact`) and are loaded as-is.
    pub fn initialize_with_web_api<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        artifact: RuntimeArtifact,
    ) -> Result<(), JsEngineError> {
        self.initialize_with_web_api_and_host_api(engine, artifact, &HostApiRegistry::new())
    }
    /// Initialize the runtime with framework defaults and application APIs.
    pub fn initialize_with_web_api_and_host_api<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        artifact: RuntimeArtifact,
        application_api: &HostApiRegistry,
    ) -> Result<(), JsEngineError> {
        let mut api = self.host_api_registry();
        api.try_extend(application_api)?;
        engine.register_host_api(&api)?;
        let artifact_name = artifact.name.clone();
        if artifact.is_binary_release() {
            engine.initialize(artifact)?;
            if let Some(base) = crate::renderer::stylesheet_base_from_href(&artifact_name) {
                self.set_stylesheet_base(base);
            }
            return Ok(());
        }
        let source = artifact.source_utf8()?;
        let composed = if source.contains("__nanaWebApi") {
            // Already composed / shim already present.
            artifact
        } else {
            compose_runtime_artifact(artifact.name.clone(), source)
        };
        engine.initialize(composed)?;
        if let Some(base) = crate::renderer::stylesheet_base_from_href(&artifact_name) {
            self.set_stylesheet_base(base);
        }
        Ok(())
    }
    /// Resolve renderer and Web API completion hooks after initialization.
    pub fn bind_event_bridge<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
    ) -> Result<(), JsEngineError> {
        self.callbacks.event_window_id = None;
        self.callbacks.fire_event = Some(engine.resolve_function("__nanaFireEvent")?);
        // Drain helper is optional (counter fixture / shim may still install it).
        self.callbacks.drain_timers = engine.resolve_function("__nanaDrainTimers").ok();
        self.callbacks.drain_fetch = engine.resolve_function("__nanaDrainFetch").ok();
        self.callbacks.drain_ws = engine.resolve_function("__nanaDrainWs").ok();
        self.callbacks.apply_theme = engine.resolve_function("__nanaApplyTheme").ok();
        self.callbacks.notify_layout = engine.resolve_function("__nanaNotifyLayout").ok();
        self.callbacks.motion_complete = engine.resolve_function("__nanaMotionComplete").ok();
        self.callbacks.motion_cancel = engine.resolve_function("__nanaMotionCancel").ok();
        self.callbacks.lifecycle_pump = engine.resolve_function("__nanaPumpLifecycle").ok();
        Ok(())
    }
    /// Resolve event functions for an auxiliary Vue window while retaining the
    /// same engine context and function table.
    pub fn bind_event_bridge_for_window<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        window_id: u64,
    ) -> Result<(), JsEngineError> {
        self.callbacks.event_window_id = Some(window_id);
        self.callbacks.fire_event = Some(engine.resolve_function("__nanaFireWindowEvent")?);
        self.callbacks.drain_timers = engine.resolve_function("__nanaDrainTimers").ok();
        self.callbacks.drain_fetch = engine.resolve_function("__nanaDrainFetch").ok();
        self.callbacks.drain_ws = engine.resolve_function("__nanaDrainWs").ok();
        self.callbacks.apply_theme = engine.resolve_function("__nanaApplyWindowTheme").ok();
        self.callbacks.notify_layout = engine.resolve_function("__nanaNotifyLayout").ok();
        self.callbacks.motion_complete = engine.resolve_function("__nanaMotionComplete").ok();
        self.callbacks.motion_cancel = engine.resolve_function("__nanaMotionCancel").ok();
        self.callbacks.lifecycle_pump = engine.resolve_function("__nanaPumpWindowLifecycle").ok();
        Ok(())
    }
    /// Rust → Vue theme inject (bridge + document + web-api + optional `__nanaApplyTheme`).
    ///
    /// Reverse path: JS `dataset.theme` / `setDocumentTheme` →
    /// [`MessageBridge::apply_document_appearance`] immediately
    /// (`documentElementSet` wrap / `setDocumentTheme` host op).
    pub fn inject_theme<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        theme: ThemeMode,
    ) -> Result<(), JsEngineError> {
        self.theme = theme;
        let label = match theme {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        };
        // Same store `sync_appearance_shared` reads — must not lag behind bridge.
        if let Ok(mut web) = self.web_api.lock() {
            web.set_document_dataset("theme", label);
            web.set_document_dataset("materialSupport", hosted_material_support_key());
        }
        {
            let mut doc = self.document.lock().expect("vue doc");
            doc.set_document_theme(label);
        }
        {
            let mut bridge = self.bridge.lock().expect("vue bridge");
            bridge.set_theme(theme);
        }
        if let Some(apply) = self.callbacks.apply_theme {
            let args = match self.callbacks.event_window_id {
                Some(window_id) => vec![
                    HostValue::Number(window_id as f64),
                    HostValue::string(label),
                ],
                None => vec![HostValue::string(label)],
            };
            engine.invoke(apply, &args)?;
            engine.run_microtasks()?;
        }
        Ok(())
    }
    /// Settle completed fetches and socket events, drain timers, then run
    /// microtasks and layout.
    ///
    /// After layout resolves, invokes optional `__nanaNotifyLayout` so
    /// `ResizeObserver` callbacks see fresh `layoutBox` geometry.
    ///
    /// Nested drain: 0ms timeouts (ResizeObserver) still flush in-loop.
    /// rAF follows this host frame once; nested rAF (Vue `<Transition>`
    /// `nextFrame` is double-rAF, used by after-leave / Dialog/Drawer) waits
    /// for `next_wakeup` (~16ms) instead of spinning a fake 16ms deadline
    /// inside the same pump.
    pub fn pump_frame<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
    ) -> Result<usize, JsEngineError> {
        let mut fired = 0usize;
        #[cfg(feature = "hosted")]
        if let Some(webgpu) = &self.webgpu {
            let completions = webgpu.poll();
            if completions > 0 {
                fired += completions;
                engine.run_microtasks()?;
            }
        }
        let fetch_completions = {
            let mut guard = self
                .web_api
                .lock()
                .map_err(|_| JsEngineError::new("web-api state poisoned"))?;
            guard.drain_fetch_completions()
        };
        if !fetch_completions.is_empty()
            && let Some(drain) = self.callbacks.drain_fetch
        {
            let count = fetch_completions.len();
            engine.invoke(
                drain,
                &[HostValue::Array(fetch_completions.into_iter().collect())],
            )?;
            fired += count;
            engine.run_microtasks()?;
        }
        let socket_events = {
            let mut guard = self
                .web_api
                .lock()
                .map_err(|_| JsEngineError::new("web-api state poisoned"))?;
            guard.drain_socket_events()
        };
        if !socket_events.is_empty()
            && let Some(drain) = self.callbacks.drain_ws
        {
            let count = socket_events.len();
            engine.invoke(
                drain,
                &[HostValue::Array(socket_events.into_iter().collect())],
            )?;
            fired += count;
            engine.run_microtasks()?;
        }
        {
            let mut bridge = self.bridge.lock().expect("vue bridge");
            let mut doc = self.document.lock().expect("vue doc");
            if doc.host_animation_epoch().is_none() {
                bridge.tick_css_animations(&mut doc);
            }
        }
        // Rust complete/cancel first so class-arm fallback timeouts do not
        // synthesize a second transitionend on the same pump.
        self.flush_motion_complete(engine)?;
        let frame_now = Instant::now();
        {
            let mut guard = self
                .web_api
                .lock()
                .map_err(|_| JsEngineError::new("web-api state poisoned"))?;
            guard.begin_host_frame(frame_now);
        }
        // Cap nested 0ms timeouts. rAF is one host frame, not this loop.
        const MAX_TIMER_PASSES: usize = 16;
        for _ in 0..MAX_TIMER_PASSES {
            let due = {
                let mut guard = self
                    .web_api
                    .lock()
                    .map_err(|_| JsEngineError::new("web-api state poisoned"))?;
                guard.due_timers(Instant::now())
            };
            if due.is_empty() {
                break;
            }
            if let Some(drain) = self.callbacks.drain_timers {
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                engine.invoke(drain, &[due.to_host_value(now_ms)])?;
                fired += due.raf.len() + due.timeouts.len() + due.intervals.len();
            }
            engine.run_microtasks()?;
        }
        {
            let mut guard = self
                .web_api
                .lock()
                .map_err(|_| JsEngineError::new("web-api state poisoned"))?;
            guard.end_host_frame(Instant::now());
        }
        self.resolve_layout();
        if let Some(notify) = self.callbacks.notify_layout {
            engine.invoke(notify, &[])?;
            engine.run_microtasks()?;
        }
        Ok(fired)
    }
    pub(crate) fn flush_motion_complete<E: JsEngine + ?Sized>(
        &self,
        engine: &mut E,
    ) -> Result<(), JsEngineError> {
        let (cancels, events) = {
            let mut bridge = self.bridge.lock().expect("vue bridge");
            (bridge.take_motion_cancels(), bridge.take_motion_completes())
        };
        if cancels.is_empty() && events.is_empty() {
            return Ok(());
        }
        if let Some(cancel) = self.callbacks.motion_cancel {
            for id in cancels {
                engine.invoke(cancel, &[HostValue::Number(id as f64)])?;
            }
        }
        if let Some(complete) = self.callbacks.motion_complete {
            for event in events {
                let detail = [
                    ("type".into(), HostValue::string(event.event_type)),
                    (
                        "propertyName".into(),
                        HostValue::string(event.property_name.clone()),
                    ),
                    (
                        "animationName".into(),
                        HostValue::string(event.animation_name),
                    ),
                    (
                        "transitionProperty".into(),
                        HostValue::string(event.transition_property),
                    ),
                    (
                        "elapsedTime".into(),
                        HostValue::Number(event.elapsed_time as f64),
                    ),
                ]
                .into_iter()
                .collect();
                engine.invoke(
                    complete,
                    &[
                        HostValue::Number(event.widget_id as f64),
                        HostValue::Object(detail),
                    ],
                )?;
            }
        }
        engine.run_microtasks()?;
        Ok(())
    }
    /// Earliest timer/rAF/fetch wake requested by the Web API state.
    /// Returns `None` when the runtime is idle.
    pub fn next_wakeup(&self) -> Option<Instant> {
        let animation_wakeup = self
            .document
            .lock()
            .ok()
            .and_then(|doc| doc.next_animation_wakeup());
        let web_wakeup = self
            .web_api
            .lock()
            .ok()
            .and_then(|guard| guard.next_wakeup(Instant::now()));
        #[cfg(feature = "hosted")]
        let gpu_wakeup = self.webgpu.as_ref().and_then(JsWebGpuRuntime::next_wakeup);
        #[cfg(not(feature = "hosted"))]
        let gpu_wakeup: Option<Instant> = None;
        animation_wakeup
            .into_iter()
            .chain(web_wakeup)
            .chain(gpu_wakeup)
            .min()
    }
    pub fn set_host_animation_epoch(&self, epoch: Instant) {
        if let Ok(mut doc) = self.document.lock() {
            doc.set_host_animation_epoch(epoch);
        }
    }
    /// Pump a host window lifecycle event into the shim EventTarget surface.
    ///
    /// No-op (returns `Ok(false)`) when `__nanaPumpLifecycle` is absent (e.g. counter
    /// fixture without web-api shim). After dispatch, runs microtasks so listeners
    /// scheduled via `queueMicrotask` / promises settle.
    pub fn pump_lifecycle<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        event: WindowLifecycleEvent,
    ) -> Result<bool, JsEngineError> {
        let Some(pump) = self.callbacks.lifecycle_pump else {
            return Ok(false);
        };
        if event == WindowLifecycleEvent::Blur {
            if let Some(target) = self.input_projection.file_drag_target.take() {
                self.fire_dom_event(engine, target, "dragleave", file_drag_detail(&[], None))?;
            }
            self.input.lock().expect("input state").clear();
            {
                let mut document = self.document.lock().expect("vue doc");
                document.clear_pointer_interactions();
                // A pending acquisition was never observable before blur. Match
                // the previous DOM-compatible behavior by publishing only the
                // release of captures that actually remained authoritative.
                let _ = document.take_pointer_capture_changes();
                document.clear_pointer_captures();
            }
            self.flush_pointer_capture_events(engine)?;
        }
        let args = match self.callbacks.event_window_id {
            Some(window_id) => vec![HostValue::Number(window_id as f64), event.to_host_value()],
            None => vec![event.to_host_value()],
        };
        engine.invoke(pump, &args)?;
        engine.run_microtasks()?;
        Ok(true)
    }
}
