//! Example model viewer built on `nana-ui-viewport3d` + `GpuTextureView`.

use std::path::PathBuf;
use std::time::Instant;

use glam::Vec3;
use nana_ui::runtime::view::{Signal, css, signal, text, widget, with_refs};
use nana_ui::runtime::{
    Activate, Button, DocumentId, Entity, FrameworkError, GpuTextureView, LengthSpec, MountedView,
    RuntimeDocument, Stack,
};
use nana_ui::{
    FrameDemand, HostTextureRegistry, RoutedInput, RuntimeProgram, RuntimeProgramContext,
    RuntimeProgramUpdate, TextureSlot, WindowDescriptor, run_runtime,
};
use nana_ui_platform::{InputPayload, PointerPhase, WindowEvent, WindowId};
use nana_ui_viewport3d::{
    Instance3d, Material3d, OrbitCamera, Scene3d, Viewport3dRenderer, cube, cylinder, sphere,
};

const SLOT_NAME: &str = "viewport3d.preview";

#[derive(Debug, Clone)]
enum Message {
    ResetCamera,
    ToggleSpin,
    ToggleGrid,
}

struct ViewerApp {
    document: RuntimeDocument,
    viewport: Entity<GpuTextureView>,
    textures: HostTextureRegistry,
    slot: TextureSlot,
    renderer: Viewport3dRenderer,
    scene: Scene3d,
    camera: OrbitCamera,
    spinning: bool,
    status: Signal<String>,
    dragging: bool,
    last_pos: Option<(f32, f32)>,
    last_wake: Instant,
}

impl ViewerApp {
    fn build_scene(glb: Option<PathBuf>) -> (Scene3d, String) {
        let mut scene = Scene3d::new();
        if let Some(path) = glb {
            match nana_ui_viewport3d::load_glb_file(&path) {
                Ok(meshes) => {
                    for (idx, mesh) in meshes.into_iter().enumerate() {
                        let mesh_index = scene.add_mesh(mesh);
                        scene.add_instance(Instance3d {
                            id: idx as u64 + 1,
                            mesh_index,
                            transform: glam::Mat4::IDENTITY,
                            material: Material3d::default(),
                        });
                    }
                    return (scene, format!("Loaded {}", path.display()));
                }
                Err(err) => eprintln!("Failed to load GLB ({err}), falling back to demo shapes"),
            }
        }
        let cube_mesh = scene.add_mesh(cube([0.9, 0.9, 0.9]));
        let sphere_mesh = scene.add_mesh(sphere(0.5, 24, 32));
        let cyl_mesh = scene.add_mesh(cylinder(0.4, 1.0, 24));
        scene.add_instance(Instance3d {
            id: 1,
            mesh_index: cube_mesh,
            transform: glam::Mat4::from_translation(Vec3::new(-1.1, 0.45, 0.0)),
            material: Material3d {
                color: [0.95, 0.48, 0.38, 1.0],
                roughness: 0.45,
                emissive: [0.0; 3],
            },
        });
        scene.add_instance(Instance3d {
            id: 2,
            mesh_index: sphere_mesh,
            transform: glam::Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)),
            material: Material3d {
                color: [0.42, 0.78, 0.68, 1.0],
                roughness: 0.25,
                emissive: [0.0; 3],
            },
        });
        scene.add_instance(Instance3d {
            id: 3,
            mesh_index: cyl_mesh,
            transform: glam::Mat4::from_translation(Vec3::new(1.1, 0.5, 0.0)),
            material: Material3d {
                color: [0.55, 0.68, 0.95, 1.0],
                roughness: 0.55,
                emissive: [0.0; 3],
            },
        });
        (scene, "Demo shapes (pass a .glb path to inspect)".into())
    }

    fn hud(
        status: Signal<String>,
    ) -> impl nana_ui::runtime::view::Mount<Output = (MountedView, Entity<GpuTextureView>)> {
        let viewport = nana_ui::runtime::view::entity_ref::<GpuTextureView>();
        let root = widget(
            Stack::layers()
                .width(LengthSpec::Fill)
                .height(LengthSpec::Fill),
        )
        .children((
            widget(GpuTextureView::new(SLOT_NAME).with_pointer_events(true)).entity_ref(viewport),
            widget(Stack::column(8.0))
                .css(css! {
                    width: shrink;
                    padding: 12px 16px;
                    margin: 16px;
                    border-radius: 12px;
                    background-color: rgba(24, 26, 32, 0.85);
                })
                .children((
                    text(move || status.get()).css(css! { font-size: 13px; color: #f2f4f8; }),
                    widget(Stack::row(8.0)).children((
                        widget(Button::new("Reset")).on_cx(|_, _: &Activate, cx| {
                            cx.dispatch_program(Message::ResetCamera);
                        }),
                        widget(Button::new("Spin")).on_cx(|_, _: &Activate, cx| {
                            cx.dispatch_program(Message::ToggleSpin);
                        }),
                        widget(Button::new("Grid")).on_cx(|_, _: &Activate, cx| {
                            cx.dispatch_program(Message::ToggleGrid);
                        }),
                    )),
                )),
        ));
        with_refs(root, viewport)
    }
}

impl RuntimeProgram for ViewerApp {
    type Message = Message;
    type Error = FrameworkError;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let glb = std::env::args_os().nth(1).map(PathBuf::from);
        let (scene, label) = Self::build_scene(glb);
        let mut camera = OrbitCamera::default();
        if let Some((min, max)) = scene.world_bounds() {
            camera.fit_bounds(min, max);
        }
        let status = signal(label);
        let doc_id = DocumentId::allocate();
        let mut document = RuntimeDocument::new(doc_id);
        let (_, viewport) = document
            .context_mut()
            .mount_view_root(doc_id, || Self::hud(status))?;
        let textures = HostTextureRegistry::new();
        let slot = textures.slot(SLOT_NAME);
        let phys = context.content_geometry().physical_size;
        let mut renderer = Viewport3dRenderer::new();
        let _ = renderer.render_to_slot(
            context.gpu(),
            &textures,
            &slot,
            [phys.0.max(1), phys.1.max(1)],
            &camera,
            &scene,
        );
        Ok((
            Self {
                document,
                viewport,
                textures,
                slot,
                renderer,
                scene,
                camera,
                spinning: false,
                status,
                dragging: false,
                last_pos: None,
                last_wake: Instant::now(),
            },
            Vec::new(),
        ))
    }

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        Ok((id == WindowId::PRIMARY).then(|| f(&self.document)))
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        Ok((id == WindowId::PRIMARY).then(|| f(&mut self.document)))
    }

    fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
        Some(self.textures.clone())
    }

    fn prepare_window_frame(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        if id != WindowId::PRIMARY {
            return;
        }
        let phys = context.content_geometry().physical_size;
        let _ = self.renderer.render_to_slot(
            context.gpu(),
            &self.textures,
            &self.slot,
            [phys.0.max(1), phys.1.max(1)],
            &self.camera,
            &self.scene,
        );
    }

    fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
        self.renderer.invalidate_gpu();
        let phys = context.content_geometry().physical_size;
        let _ = self.renderer.render_to_slot(
            context.gpu(),
            &self.textures,
            &self.slot,
            [phys.0.max(1), phys.1.max(1)],
            &self.camera,
            &self.scene,
        );
    }

    fn update(
        &mut self,
        message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match message {
            Message::ResetCamera => {
                self.camera = OrbitCamera::default();
                if let Some((min, max)) = self.scene.world_bounds() {
                    self.camera.fit_bounds(min, max);
                }
            }
            Message::ToggleSpin => {
                self.spinning = !self.spinning;
                self.last_wake = Instant::now();
            }
            Message::ToggleGrid => {
                self.scene.show_grid = !self.scene.show_grid;
            }
        }
        RuntimeProgramUpdate::redraw(WindowId::PRIMARY)
    }

    fn frame_demand(&self, _id: WindowId) -> FrameDemand {
        if self.spinning {
            FrameDemand::Continuous(std::num::NonZeroU32::new(60).expect("non-zero"))
        } else {
            FrameDemand::OnDemand
        }
    }

    fn wake(
        &mut self,
        now: Instant,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        let dt = (now - self.last_wake).as_secs_f32().clamp(0.0, 0.1);
        self.last_wake = now;
        if self.spinning {
            self.camera.yaw += dt * 0.8;
            return RuntimeProgramUpdate::redraw(WindowId::PRIMARY);
        }
        RuntimeProgramUpdate::default()
    }

    fn input_event(
        &mut self,
        id: WindowId,
        input: RoutedInput<'_>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let on_view = input.pointer_hit == Some(self.viewport.stable_id());
        match &input.event.payload {
            InputPayload::Pointer(p) => match p.phase {
                PointerPhase::Down if on_view => {
                    self.dragging = true;
                    self.last_pos = Some((p.x, p.y));
                    if let Some(bounds) = self
                        .document
                        .context()
                        .world()
                        .layout_box(self.viewport.stable_id())
                        && let Some(size) = self.textures.painted_extent(SLOT_NAME)
                        && let Some(uv) = GpuTextureView::new(SLOT_NAME).texture_uv_at(
                            nana_ui::LogicalRect {
                                x: bounds.x,
                                y: bounds.y,
                                width: bounds.width,
                                height: bounds.height,
                            },
                            size,
                            nana_ui::LogicalPoint { x: p.x, y: p.y },
                        )
                    {
                        let ray = self
                            .camera
                            .ray_from_uv(uv, size[0] as f32 / size[1].max(1) as f32);
                        if let Some(hit) = self.scene.pick(ray) {
                            self.status.set(format!(
                                "Picked instance #{} at d={:.2}",
                                hit.instance_id, hit.distance
                            ));
                        }
                    }
                }
                PointerPhase::Move if self.dragging => {
                    if let Some((lx, ly)) = self.last_pos {
                        self.camera.orbit(p.x - lx, p.y - ly);
                    }
                    self.last_pos = Some((p.x, p.y));
                    return Ok(RuntimeProgramUpdate::redraw(id));
                }
                PointerPhase::Up => {
                    self.dragging = false;
                    self.last_pos = None;
                }
                _ => {}
            },
            InputPayload::Wheel(w) if on_view => {
                self.camera.zoom(w.delta_y);
                return Ok(RuntimeProgramUpdate::redraw(id));
            }
            _ => {}
        }
        Ok(RuntimeProgramUpdate::default())
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::Ready { id, .. } | WindowEvent::Resized { id, .. } => {
                RuntimeProgramUpdate::redraw(id)
            }
            _ => RuntimeProgramUpdate::default(),
        }
    }
}

fn main() -> Result<(), nana_ui::HostedRunError> {
    run_runtime::<ViewerApp>(
        WindowDescriptor::new("NanaUI Viewport3D Model Viewer").initial_size(1024.0, 720.0),
    )
}
