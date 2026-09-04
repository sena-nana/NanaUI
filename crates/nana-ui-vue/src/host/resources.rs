//! Vue host resources boundary.

use crate::*;

impl VueHost {
    pub fn canvas_runtime(&self) -> SharedCanvasRuntime {
        Arc::clone(&self.canvas)
    }
    pub fn canvas_runtime_ref(&self) -> &SharedCanvasRuntime {
        &self.canvas
    }
    /// Shared `<video>` frame mailbox. The host pushes decoded frames here;
    /// the frame pump uploads the newest frame to the video's texture slot.
    pub fn video_runtime(&self) -> video::SharedVideoRuntime {
        Arc::clone(&self.video)
    }
    /// Replaces the shared video mailbox (multi-window sharing). Must be
    /// called before the first frame pump picks frames up.
    pub fn share_video_runtime(&mut self, video: video::SharedVideoRuntime) {
        self.video = video;
    }
    pub fn media_runtime(&self) -> SharedMediaRuntime {
        Arc::clone(&self.media)
    }
    /// CPU retain set (video **and** audio) plus visual `video:{id}` slots.
    ///
    /// Identity comes from the tree (`data-nana-media` / `<video>` / `<audio>`)
    /// and the bridge snapshot. Visual slots still come only from
    /// `data-nana-video` / CustomRender, matching canvas/svg GPU prune.
    #[cfg(any(feature = "hosted", test))]
    pub(crate) fn live_media_sets(&self) -> nana_ui_web_api::MediaLiveSets {
        let mut retain = std::collections::HashSet::new();
        let mut visual = std::collections::HashSet::new();
        if let Ok(doc) = self.document.lock() {
            let sets = doc.live_media_sets();
            retain.extend(sets.retain);
            visual.extend(sets.visual);
        }
        if let Ok(bridge) = self.bridge.lock() {
            let sets = nana_ui_web_api::media_live_sets_from_tree(
                bridge
                    .widget_ids()
                    .filter_map(|id| bridge.get(id))
                    .map(|widget| nana_ui_web_api::MediaTreeRef {
                        tag: if widget.props.element_tag.is_empty() {
                            widget.kind.element_tag()
                        } else {
                            &widget.props.element_tag
                        },
                        media_id: widget
                            .props
                            .attrs
                            .get("data-nana-media")
                            .map(String::as_str),
                        video_id: widget
                            .props
                            .attrs
                            .get("data-nana-video")
                            .map(String::as_str),
                    }),
            );
            retain.extend(sets.retain);
            visual.extend(sets.visual);
        }
        let mut retain: Vec<_> = retain.into_iter().collect();
        retain.sort_by_key(|id| id.0);
        let mut visual: Vec<_> = visual.into_iter().collect();
        visual.sort_by_key(|id| id.0);
        nana_ui_web_api::MediaLiveSets { retain, visual }
    }
    /// CPU-only probe for resource lifecycle tests; hosted pruning includes GPU state.
    #[cfg(test)]
    pub(crate) fn retain_live_media(&self) {
        let sets = self.live_media_sets();
        if let Ok(mut media) = self.media.lock() {
            media.retain_live(sets.retain.iter().copied());
        }
    }

    /// Current compatibility-layer resource counts for development snapshots.
    pub fn resource_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        counts.insert(
            "canvas".into(),
            self.canvas
                .lock()
                .map(|runtime| runtime.active_resource_count())
                .unwrap_or_default(),
        );
        counts.insert(
            "media".into(),
            self.media
                .lock()
                .map(|runtime| runtime.active_resource_count())
                .unwrap_or_default(),
        );
        #[cfg(feature = "scene-view")]
        counts.insert("hostTexture".into(), self.host_textures.len());
        #[cfg(feature = "hosted")]
        if let Some(webgpu) = &self.webgpu {
            counts.extend(
                webgpu
                    .resource_counts()
                    .into_iter()
                    .map(|(kind, count)| (format!("webgpu.{kind}"), count)),
            );
        }
        counts
    }
    #[cfg(feature = "scene-view")]
    pub fn host_textures(&self) -> &HostTextureRegistry {
        &self.host_textures
    }
    #[cfg(feature = "scene-view")]
    pub fn register_host_texture(
        &self,
        slot: impl Into<String>,
        texture: HostTexture,
        width: u32,
        height: u32,
        alpha_mode: HostTextureAlphaMode,
    ) -> NanaTextureHandle {
        let binding = self
            .host_textures
            .register(slot, texture, width, height, alpha_mode);
        self.report_diagnostic(
            "nana.resource",
            JsDiagnosticLevel::Info,
            format!("host texture registered: {}", binding.slot),
            None,
        );
        NanaTextureHandle {
            slot: binding.slot,
            id: binding.texture.id(),
            generation: binding.texture.generation(),
            version: binding.texture.version(),
            width: binding.width,
            height: binding.height,
            alpha_mode: binding.alpha_mode,
        }
    }
    /// Content update boundary. A true result means callers should request a
    /// redraw from the hosted runtime.
    #[cfg(feature = "scene-view")]
    pub fn invalidate_host_texture(&self, slot: &str) -> bool {
        self.host_textures.invalidate(slot).is_some()
    }
    #[cfg(feature = "scene-view")]
    pub fn remove_host_texture(&self, slot: &str) -> bool {
        let removed = self.host_textures.remove(slot).is_some();
        if removed {
            self.report_diagnostic(
                "nana.resource",
                JsDiagnosticLevel::Info,
                format!("host texture released: {slot}"),
                None,
            );
        }
        removed
    }
    /// Device-loss boundary: all prior texture descriptors become invalid.
    #[cfg(feature = "scene-view")]
    pub fn invalidate_host_textures(&self) -> usize {
        self.host_textures.invalidate_all()
    }
    /// Attach the hosted renderer's existing adapter/device/queue to the JS
    /// WebGPU facade. Call again after device recovery, then re-register the
    /// host API on the engine so existing JS wrappers observe the generation.
    #[cfg(feature = "hosted")]
    pub fn bind_host_gpu(&mut self, resources: nana_ui::HostedGpuResources) -> u64 {
        match &self.canvas_gpu {
            Some(canvas_gpu) => canvas_gpu.replace_device(resources.clone()),
            None => {
                self.canvas_gpu = Some(canvas_gpu::CanvasGpuBridge::new(
                    resources.clone(),
                    Arc::clone(&self.canvas),
                    self.host_textures.clone(),
                ));
            }
        }
        match &self.video_gpu {
            Some(video_gpu) => video_gpu.replace_device(resources.clone()),
            None => {
                self.video_gpu = Some(video::VideoGpuBridge::new(
                    resources.clone(),
                    self.host_textures.clone(),
                ));
            }
        }
        match &self.svg_gpu {
            Some(svg_gpu) => svg_gpu.replace_device(resources.clone()),
            None => {
                self.svg_gpu = Some(svg_gpu::SvgGpuBridge::new(
                    resources.clone(),
                    self.host_textures.clone(),
                ));
            }
        }
        match &self.media_gpu {
            Some(media_gpu) => media_gpu.replace_device(resources.clone()),
            None => {
                self.media_gpu = Some(media_gpu::MediaGpuBridge::new(
                    resources.clone(),
                    Arc::clone(&self.media),
                    self.host_textures.clone(),
                ));
            }
        }
        match &self.webgpu {
            Some(runtime) => runtime.replace_device(resources),
            None => {
                let runtime = JsWebGpuRuntime::new(resources, self.host_textures.clone());
                let generation = runtime.generation();
                self.webgpu = Some(runtime);
                generation
            }
        }
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn share_canvas_gpu(&mut self, canvas_gpu: canvas_gpu::CanvasGpuBridge) {
        self.canvas_gpu = Some(canvas_gpu);
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn canvas_gpu(&self) -> Option<&canvas_gpu::CanvasGpuBridge> {
        self.canvas_gpu.as_ref()
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn share_video_gpu(&mut self, video_gpu: video::VideoGpuBridge) {
        self.video_gpu = Some(video_gpu);
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn video_gpu(&self) -> Option<&video::VideoGpuBridge> {
        self.video_gpu.as_ref()
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn prepare_canvas_gpu(&self) {
        let ids = self
            .bridge
            .lock()
            .map(|bridge| {
                bridge
                    .peek_snapshot()
                    .widgets
                    .iter()
                    .filter_map(|widget| {
                        widget
                            .props
                            .attrs
                            .get("data-nana-canvas")
                            .or_else(|| widget.props.attrs.get("data-nana-image"))
                            .and_then(|id| id.parse::<u64>().ok())
                    })
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .unwrap_or_default();
        if let Some(canvas_gpu) = &self.canvas_gpu {
            for id in ids {
                if let Err(error) = canvas_gpu.sync(nana_ui_web_api::CanvasId(id)) {
                    self.report_diagnostic("canvas.gpu", JsDiagnosticLevel::Error, error, None);
                }
            }
        }
        self.prepare_video_gpu();
    }
    /// Uploads the newest pushed video frame for every live `video:{id}`
    /// host-texture slot. Slots are read from the Runtime tree (not a facade
    /// map); runs on the frame pump so GPU writes stay on the host GPU path.
    #[cfg(feature = "hosted")]
    pub(crate) fn prepare_video_gpu(&self) {
        let Some(video_gpu) = &self.video_gpu else {
            return;
        };
        let ids = {
            let document = self.document.lock().expect("vue doc");
            document
                .gpu_slots()
                .iter()
                .filter_map(|(_, resource)| {
                    resource
                        .strip_prefix("video:")
                        .and_then(|id| id.parse::<u64>().ok())
                })
                .collect::<std::collections::BTreeSet<_>>()
        };
        for id in ids {
            if let Err(error) = video_gpu.sync(video::VideoId(id), &self.video) {
                self.report_diagnostic("video.gpu", JsDiagnosticLevel::Error, error, None);
            }
        }
    }
    /// Dispatches a video lifecycle event (`play` / `pause` / `ended`) to the
    /// listeners of `<video data-nana-video="{id}">`. Resolves `false` when no
    /// element carries that surface id.
    pub fn notify_video_event<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        video_id: u64,
        name: &str,
    ) -> Result<bool, JsEngineError> {
        let Some(target) = self.video_event_target(video_id) else {
            return Ok(false);
        };
        self.dispatch_bridge_event(
            engine,
            BridgeEvent::Native {
                id: target.0,
                name: name.to_owned(),
                payload: HostValue::Object(BTreeMap::from([(
                    "videoId".into(),
                    HostValue::Number(video_id as f64),
                )])),
            },
        )
    }
    /// Host playback-state boundary for `<video>`: dispatches `play` or
    /// `pause`. The playback clock itself stays with the host push side;
    /// other lifecycle events (`ended`, …) go through [`Self::notify_video_event`].
    pub fn set_video_playing<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        video_id: u64,
        playing: bool,
    ) -> Result<bool, JsEngineError> {
        self.notify_video_event(engine, video_id, if playing { "play" } else { "pause" })
    }
    pub(crate) fn video_event_target(&self, video_id: u64) -> Option<NodeHandle> {
        let document = self.document.lock().expect("vue doc");
        document.element_with_attribute("data-nana-video", &video_id.to_string())
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn share_svg_gpu(&mut self, svg_gpu: svg_gpu::SvgGpuBridge) {
        self.svg_gpu = Some(svg_gpu);
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn svg_gpu(&self) -> Option<&svg_gpu::SvgGpuBridge> {
        self.svg_gpu.as_ref()
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn share_media_gpu(&mut self, media_gpu: media_gpu::MediaGpuBridge) {
        self.media_gpu = Some(media_gpu);
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn media_gpu(&self) -> Option<&media_gpu::MediaGpuBridge> {
        self.media_gpu.as_ref()
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn prepare_svg_gpu(&self) {
        let Some(svg_gpu) = &self.svg_gpu else {
            return;
        };
        let uploads = self
            .document
            .lock()
            .map(|doc| doc.svg_host_uploads())
            .unwrap_or_default();
        let live: std::collections::HashSet<String> =
            uploads.iter().map(|upload| upload.slot.clone()).collect();
        svg_gpu.prune_released(&live);
        for upload in uploads {
            if let Err(error) = svg_gpu.sync(&upload) {
                self.report_diagnostic("svg.gpu", JsDiagnosticLevel::Error, error, None);
            }
        }
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn prepare_media_gpu(&self) {
        let Some(media_gpu) = &self.media_gpu else {
            return;
        };
        media_gpu.tick_playing();
        let sets = self.live_media_sets();
        if let Ok(mut media) = self.media.lock() {
            media.retain_live(sets.retain.iter().copied());
        }
        let live = sets.visual_slots();
        media_gpu.prune_released(&live);
        for id in sets.visual {
            if let Err(error) = media_gpu.sync(id) {
                self.report_diagnostic("media.gpu", JsDiagnosticLevel::Error, error, None);
            }
        }
    }
    /// Rebind the JS WebGPU facade after host device recovery and notify the
    /// existing JavaScript device before replacing its native resources.
    #[cfg(feature = "hosted")]
    pub fn replace_host_gpu<E: JsEngine + ?Sized>(
        &mut self,
        engine: &mut E,
        resources: nana_ui::HostedGpuResources,
        message: &str,
    ) -> Result<u64, JsEngineError> {
        self.report_diagnostic(
            "wgpu.device",
            JsDiagnosticLevel::Error,
            message.to_owned(),
            None,
        );
        if let Ok(notify) = engine.resolve_function("__nanaWebGpuDeviceLost") {
            engine.invoke(notify, &[HostValue::String(message.to_owned())])?;
            engine.run_microtasks()?;
        }
        let generation = self.bind_host_gpu(resources);
        engine.register_host_api(&self.host_api_registry())?;
        Ok(generation)
    }
    #[cfg(feature = "hosted")]
    pub fn webgpu_runtime(&self) -> Option<&JsWebGpuRuntime> {
        self.webgpu.as_ref()
    }
    #[cfg(feature = "hosted")]
    pub(crate) fn share_webgpu_runtime(&mut self, runtime: JsWebGpuRuntime) {
        self.webgpu = Some(runtime);
    }
    /// Registry shared by the Vue host and native component commands.
    #[cfg(feature = "scene-view")]
    pub fn components(&self) -> &NativeComponentRegistry {
        &self.components
    }
    #[cfg(feature = "scene-view")]
    pub(crate) fn share_components(&mut self, components: NativeComponentRegistry) {
        self.components = components;
    }
    #[cfg(feature = "scene-view")]
    pub(crate) fn share_host_textures(&mut self, textures: HostTextureRegistry) {
        self.host_textures = textures;
        if let Ok(mut document) = self.document.lock() {
            document.attach_host_textures(self.host_textures.clone());
        }
    }
    #[cfg(feature = "scene-view")]
    pub(crate) fn unmount_all_native_components(&self) {
        let mounted = self
            .bridge
            .lock()
            .map(|bridge| {
                bridge
                    .peek_snapshot()
                    .widgets
                    .into_iter()
                    .filter_map(|widget| {
                        widget.props.native_component.map(|name| (name, widget.id))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (component, id) in mounted {
            self.components.unmount(&component, id);
        }
    }
    #[cfg(feature = "scene-view")]
    pub(crate) fn native_component_name(&self, id: WidgetId) -> Option<String> {
        self.bridge
            .lock()
            .ok()?
            .get(id)
            .and_then(|widget| widget.props.native_component.clone())
    }
    #[cfg(not(feature = "scene-view"))]
    pub(crate) fn native_component_name(&self, _id: u64) -> Option<String> {
        None
    }
}
