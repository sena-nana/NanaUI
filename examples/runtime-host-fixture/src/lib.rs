//! Framework-external Runtime consumer: Dock, HostTexture, auxiliary windows, AX.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nana_ui::runtime::{
    AccessibilityActionRequest, Activate, Button, Dock, DockAxis, DockNode, DockWorkspaceEvent,
    DocumentId, Entity, FrameworkError, GpuTextureView, List, RuntimeDocument, Text, TextInput,
};
use nana_ui::{
    HostTexture, HostTextureAlphaMode, HostTextureRegistry, RuntimeInputAdapter, RuntimeProgram,
    RuntimeProgramContext, RuntimeProgramUpdate, RuntimeRedraw, RuntimeWindowSettings, ThemeMode,
    dock_workspace_window_id, run_runtime, runtime_dock_window_update,
};
use nana_ui_platform::{
    ImeEvent, WindowCommand, WindowEvent, WindowId, WindowRole, WindowSettings,
};

const TOOL: WindowId = WindowId(100);
const PREVIEW_SLOT: &str = "preview";
const EDITOR_PANE: &str = "editor";
const PREVIEW_PANE: &str = "preview";
const PREVIEW_EXTENT: u32 = 96;

struct PreviewGpu {
    texture: wgpu::Texture,
    host: HostTexture,
}

pub struct Fixture {
    documents: HashMap<WindowId, RuntimeDocument>,
    next_document: u64,
    dock: Entity<Dock>,
    #[allow(dead_code)]
    name: Entity<TextInput>,
    #[allow(dead_code)]
    open_tool: Entity<Button>,
    #[allow(dead_code)]
    float_preview: Entity<Button>,
    preview: Entity<GpuTextureView>,
    textures: HostTextureRegistry,
    preview_gpu: Option<PreviewGpu>,
    pending_open: Arc<AtomicBool>,
    pending_float: Arc<AtomicBool>,
}

impl Fixture {
    /// Retained documents without a GPU context. HostTexture binds in
    /// [`RuntimeProgram::initialize`].
    pub fn mount() -> Result<Self, FrameworkError> {
        let pending_open = Arc::new(AtomicBool::new(false));
        let pending_float = Arc::new(AtomicBool::new(false));
        let document_id = DocumentId::new(1).expect("fixture document id");
        let mut document = RuntimeDocument::new(document_id);

        let pending_open_handler = Arc::clone(&pending_open);
        let pending_float_handler = Arc::clone(&pending_float);
        let (dock, name, open_tool, float_preview, preview) =
            document.context_mut().build(document_id, |ui| {
                let editor = ui.leaf(List::new().label("Editor"));
                let (name, open_tool, float_preview) = ui.nest(editor, |ui| {
                    let name = ui.child("name", TextInput::new("NanaUI").label("Name"));
                    let open_tool = ui.child("open", Button::new("Open tool"));
                    let float_preview = ui.child("float", Button::new("Float preview"));
                    ui.on(open_tool, move |_button, _event: &Activate, _cx| {
                        pending_open_handler.store(true, Ordering::SeqCst);
                    });
                    ui.on(float_preview, move |_button, _event: &Activate, _cx| {
                        pending_float_handler.store(true, Ordering::SeqCst);
                    });
                    (name, open_tool, float_preview)
                });
                let preview = ui.leaf(GpuTextureView::new(PREVIEW_SLOT));
                let dock = ui.child(
                    "dock",
                    Dock::new(DockNode::split(
                        DockAxis::Horizontal,
                        0.46,
                        DockNode::item(EDITOR_PANE, Some(editor.stable_id())),
                        DockNode::item(PREVIEW_PANE, Some(preview.stable_id())),
                    ))
                    .title(EDITOR_PANE, "Editor")
                    .title(PREVIEW_PANE, "Preview")
                    .primary(EDITOR_PANE),
                );
                ui.nest(dock, |ui| {
                    ui.adopt(editor);
                    ui.adopt(preview);
                });
                (dock, name, open_tool, float_preview, preview)
            })?;
        document.context_mut().assemble_dock(dock)?;
        document
            .context_mut()
            .focus_node(document_id, name.stable_id())?;

        Ok(Self {
            documents: HashMap::from([(WindowId::PRIMARY, document)]),
            next_document: 2,
            dock,
            name,
            open_tool,
            float_preview,
            preview,
            textures: HostTextureRegistry::new(),
            preview_gpu: None,
            pending_open,
            pending_float,
        })
    }

    fn alloc_document(&mut self) -> DocumentId {
        let id = self.next_document;
        self.next_document += 1;
        DocumentId::new(id).expect("fixture document id")
    }

    fn on_window_event(
        &mut self,
        event: WindowEvent,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        match event {
            WindowEvent::Ready { id, .. } if id == WindowId::PRIMARY => self.open_tool_window(),
            WindowEvent::Closed { id } => {
                self.documents.remove(&id);
                Ok(RuntimeProgramUpdate::default())
            }
            WindowEvent::CloseRequested { id } if id == WindowId::PRIMARY => {
                Ok(RuntimeProgramUpdate::exit())
            }
            WindowEvent::CloseRequested { id } => Ok(RuntimeProgramUpdate {
                redraw: RuntimeRedraw::None,
                window_commands: vec![WindowCommand::Close(id)],
                exit: false,
            }),
            WindowEvent::Ime { id, event } => self.apply_ime(id, event),
            _ => Ok(RuntimeProgramUpdate::default()),
        }
    }

    fn apply_ime(
        &mut self,
        id: WindowId,
        event: ImeEvent,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let ime_changed = self
            .documents
            .get_mut(&id)
            .map(|document| {
                let document_id = document.document();
                RuntimeInputAdapter::default()
                    .dispatch_ime(document.context_mut(), document_id, &event)
                    .map(|disposition| {
                        disposition.prevent_default && !matches!(event, ImeEvent::Enabled)
                    })
            })
            .transpose()?
            .unwrap_or(false);
        let pending = self.drain_pending()?;
        Ok(if ime_changed {
            merge_update(RuntimeProgramUpdate::redraw(id), pending)
        } else {
            pending
        })
    }

    fn on_accessibility_action(
        &mut self,
        id: WindowId,
        request: AccessibilityActionRequest,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let changed = self
            .documents
            .get_mut(&id)
            .map(|document| {
                let document_id = document.document();
                document
                    .context_mut()
                    .apply_accessibility_action(document_id, request)
            })
            .transpose()?
            .unwrap_or(false);
        let pending = self.drain_pending()?;
        Ok(if changed {
            merge_update(RuntimeProgramUpdate::redraw(id), pending)
        } else {
            pending
        })
    }

    fn drain_pending(&mut self) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let mut update = RuntimeProgramUpdate::default();
        if self.pending_open.swap(false, Ordering::SeqCst) {
            update = merge_update(update, self.open_tool_window()?);
        }
        if self.pending_float.swap(false, Ordering::SeqCst) {
            update = merge_update(update, self.float_preview_pane()?);
        }
        Ok(update)
    }

    fn open_tool_window(&mut self) -> Result<RuntimeProgramUpdate, FrameworkError> {
        if !self.documents.contains_key(&TOOL) {
            let document_id = self.alloc_document();
            self.documents.insert(TOOL, tool_document(document_id)?);
        }
        Ok(RuntimeProgramUpdate {
            redraw: RuntimeRedraw::All,
            window_commands: vec![WindowCommand::Open {
                id: TOOL,
                settings: WindowSettings {
                    title: "Notes".into(),
                    initial_size: (360.0, 180.0),
                    minimum_size: (240.0, 120.0),
                    initial_position: Some((80.0, 80.0)),
                    maximized: false,
                    transparent: false,
                    always_on_top: false,
                    resizable: true,
                    role: WindowRole::Tool,
                    modal: false,
                    parent: Some(WindowId::PRIMARY),
                    system_caption: true,
                    icon: None,
                },
            }],
            exit: false,
        })
    }

    fn float_preview_pane(&mut self) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let Some(primary) = self.documents.get_mut(&WindowId::PRIMARY) else {
            return Ok(RuntimeProgramUpdate::default());
        };
        let Some(DockWorkspaceEvent::OpenFloating(surface)) = primary
            .context_mut()
            .float_dock_item(self.dock, PREVIEW_PANE)?
        else {
            return Ok(RuntimeProgramUpdate::default());
        };
        let window_id = dock_workspace_window_id(&surface.id);
        if !self.documents.contains_key(&window_id) {
            let document_id = self.alloc_document();
            self.documents
                .insert(window_id, floating_preview_document(document_id)?);
        }
        Ok(runtime_dock_window_update(
            [DockWorkspaceEvent::OpenFloating(surface)],
            "Preview",
        ))
    }

    fn bind_preview_gpu(&mut self, gpu: &nana_ui::HostedGpuResources) {
        let preview = create_preview_gpu(gpu.device());
        self.textures.register(
            PREVIEW_SLOT,
            preview.host.clone(),
            PREVIEW_EXTENT,
            PREVIEW_EXTENT,
            HostTextureAlphaMode::Opaque,
        );
        let generation = preview.host.generation();
        if let Some(document) = self.documents.get_mut(&WindowId::PRIMARY) {
            let _ = document
                .context_mut()
                .update_component(self.preview, |view, _| {
                    view.replace_view(generation);
                    view.invalidate_content();
                });
        }
        self.preview_gpu = Some(preview);
    }

    fn paint_preview(&mut self, gpu: &nana_ui::HostedGpuResources) {
        let Some(preview) = self.preview_gpu.as_ref() else {
            return;
        };
        let view = preview
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fixture preview"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fixture preview clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.18,
                            g: 0.42,
                            b: 0.62,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        gpu.queue().submit([encoder.finish()]);
        preview.host.invalidate();
    }
}

impl RuntimeProgram for Fixture {
    type Message = ();
    type Error = FrameworkError;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let mut fixture = Self::mount()?;
        fixture.bind_preview_gpu(context.gpu());
        Ok((fixture, Vec::new()))
    }

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        let document = { self.documents.get(&id) };
        Ok(document.map(f))
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        let document = { self.documents.get_mut(&id) };
        Ok(document.map(f))
    }

    fn update(
        &mut self,
        _message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    fn theme_mode(&self) -> ThemeMode {
        ThemeMode::Dark
    }

    fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
        Some(self.textures.clone())
    }

    fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
        self.bind_preview_gpu(context.gpu());
    }

    fn prepare_window_frame(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        if id == WindowId::PRIMARY {
            self.paint_preview(context.gpu());
        }
    }

    fn input_event(
        &mut self,
        id: WindowId,
        event: &nana_ui_platform::InputEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        if let Some(document) = self.documents.get_mut(&id) {
            let document_id = document.document();
            let _ = RuntimeInputAdapter::default().dispatch(
                document.context_mut(),
                document_id,
                event,
            )?;
        }
        let pending = self.drain_pending()?;
        Ok(if pending == RuntimeProgramUpdate::default() {
            RuntimeProgramUpdate::redraw(id)
        } else {
            pending
        })
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        // Scene host already applied IME; re-dispatch would commit twice.
        if matches!(event, WindowEvent::Ime { .. }) {
            return RuntimeProgramUpdate::default();
        }
        self.on_window_event(event)
            .unwrap_or_else(|error| panic!("fixture window event failed: {error}"))
    }

    fn window_frame_presented(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.drain_pending()
            .unwrap_or_else(|error| panic!("fixture pending window command failed: {error}"))
    }

    fn accessibility_action(
        &mut self,
        id: WindowId,
        request: AccessibilityActionRequest,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        self.on_accessibility_action(id, request)
    }
}

fn tool_document(document_id: DocumentId) -> Result<RuntimeDocument, FrameworkError> {
    let mut document = RuntimeDocument::new(document_id);
    document.context_mut().build(document_id, |ui| {
        ui.with("notes", List::new().label("Notes"), |ui| {
            ui.child("label", Text::new("Scratch pad"));
            ui.child("done", Button::new("Done"));
        });
    })?;
    Ok(document)
}

fn floating_preview_document(document_id: DocumentId) -> Result<RuntimeDocument, FrameworkError> {
    let mut document = RuntimeDocument::new(document_id);
    let dock = document.context_mut().build(document_id, |ui| {
        let preview = ui.leaf(GpuTextureView::new(PREVIEW_SLOT));
        let dock = ui.child(
            "dock",
            Dock::new(DockNode::item(PREVIEW_PANE, Some(preview.stable_id())))
                .title(PREVIEW_PANE, "Preview")
                .primary(PREVIEW_PANE),
        );
        ui.nest(dock, |ui| ui.adopt(preview));
        dock
    })?;
    document.context_mut().assemble_dock(dock)?;
    Ok(document)
}

fn create_preview_gpu(device: &wgpu::Device) -> PreviewGpu {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fixture preview"),
        size: wgpu::Extent3d {
            width: PREVIEW_EXTENT,
            height: PREVIEW_EXTENT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    PreviewGpu {
        texture,
        host: HostTexture::from_wgpu(1, 1, view),
    }
}

fn merge_update(
    mut left: RuntimeProgramUpdate,
    right: RuntimeProgramUpdate,
) -> RuntimeProgramUpdate {
    left.exit |= right.exit;
    left.window_commands.extend(right.window_commands);
    left.redraw = match (left.redraw, right.redraw) {
        (RuntimeRedraw::All, _) | (_, RuntimeRedraw::All) => RuntimeRedraw::All,
        (RuntimeRedraw::None, redraw) | (redraw, RuntimeRedraw::None) => redraw,
        (RuntimeRedraw::Window(first), RuntimeRedraw::Window(second)) if first == second => {
            RuntimeRedraw::Window(first)
        }
        _ => RuntimeRedraw::All,
    };
    left
}

pub fn run() -> Result<(), nana_ui::HostedRunError> {
    run_runtime::<Fixture>(
        RuntimeWindowSettings::new("NanaUI fixture")
            .initial_size(720.0, 420.0)
            .minimum_size(480.0, 280.0)
            .system_caption(true),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui::NanaTextShaper;
    use nana_ui::runtime::{
        AccessibilityAction, AccessibilityActionRequest, AccessibilityRole, LayoutViewport,
        TextSelection,
    };

    fn flush(document: &mut RuntimeDocument) {
        let _ = document.flush(
            LayoutViewport::new(720.0, 420.0),
            &mut NanaTextShaper::default(),
        );
    }

    fn ax(document: &RuntimeDocument) -> Vec<nana_ui::runtime::AccessibilityNode> {
        document
            .context()
            .world()
            .project_accessibility(document.document())
    }

    fn has_role(nodes: &[nana_ui::runtime::AccessibilityNode], role: AccessibilityRole) -> bool {
        nodes.iter().any(|node| node.role == role)
    }

    fn apply_ax(
        fixture: &mut Fixture,
        id: WindowId,
        target: nana_ui::runtime::StableNodeId,
        action: AccessibilityAction,
    ) -> RuntimeProgramUpdate {
        fixture
            .on_accessibility_action(id, AccessibilityActionRequest { target, action })
            .expect("accessibility action")
    }

    #[test]
    fn fixture_mounts_dock_host_texture_slot_and_ax_roles() {
        let mut fixture = Fixture::mount().expect("mount");
        let document = fixture
            .documents
            .get_mut(&WindowId::PRIMARY)
            .expect("primary");
        flush(document);
        let preview = fixture.preview.stable_id();
        let node = fixture
            .documents
            .get(&WindowId::PRIMARY)
            .expect("primary")
            .context()
            .world()
            .custom_render(preview)
            .expect("GpuTextureView projects a host-texture node");
        assert_eq!(node.resource.as_ref(), PREVIEW_SLOT);
        assert!(fixture.textures.get(PREVIEW_SLOT).is_none());

        let nodes = ax(fixture.documents.get(&WindowId::PRIMARY).expect("primary"));
        assert!(has_role(&nodes, AccessibilityRole::TextInput));
        assert!(has_role(&nodes, AccessibilityRole::Button));
        assert!(has_role(&nodes, AccessibilityRole::Image));
        assert!(
            nodes
                .iter()
                .any(|node| node.role == AccessibilityRole::TextInput && node.editable)
        );
    }

    #[test]
    fn ready_and_ax_drive_tool_window_value_and_floating_dock() {
        let mut fixture = Fixture::mount().expect("mount");
        let ready = fixture
            .on_window_event(WindowEvent::Ready {
                id: WindowId::PRIMARY,
                geometry: nana_ui_platform::WindowGeometry {
                    physical_position: Some((0, 0)),
                    physical_size: (720, 420),
                    logical_position: Some((0.0, 0.0)),
                    logical_size: (720.0, 420.0),
                    scale_factor: 1.0,
                    maximized: false,
                },
            })
            .expect("open tool");
        assert!(ready.window_commands.iter().any(|command| matches!(
            command,
            WindowCommand::Open { id, settings }
                if *id == TOOL && settings.role == WindowRole::Tool
        )));
        let tool = fixture.documents.get_mut(&TOOL).expect("tool document");
        flush(tool);
        let tool_nodes = ax(fixture.documents.get(&TOOL).expect("tool"));
        assert!(has_role(&tool_nodes, AccessibilityRole::Button));
        assert!(has_role(&tool_nodes, AccessibilityRole::Text));

        let name = fixture.name.stable_id();
        let float_preview = fixture.float_preview.stable_id();
        let open_tool = fixture.open_tool.stable_id();
        let update = apply_ax(
            &mut fixture,
            WindowId::PRIMARY,
            name,
            AccessibilityAction::SetValue("Hello".into()),
        );
        assert_eq!(update.redraw, RuntimeRedraw::Window(WindowId::PRIMARY));
        let value = fixture
            .documents
            .get(&WindowId::PRIMARY)
            .expect("primary")
            .context()
            .read(fixture.name, |input| input.state.value.clone())
            .expect("read name");
        assert_eq!(value, "Hello");

        let focused = apply_ax(
            &mut fixture,
            WindowId::PRIMARY,
            float_preview,
            AccessibilityAction::Focus,
        );
        assert_eq!(focused.redraw, RuntimeRedraw::Window(WindowId::PRIMARY));
        let document = fixture.documents.get(&WindowId::PRIMARY).expect("primary");
        assert_eq!(
            document.context().world().focused(document.document()),
            Some(float_preview)
        );

        let floated = apply_ax(
            &mut fixture,
            WindowId::PRIMARY,
            float_preview,
            AccessibilityAction::Click,
        );
        assert!(floated.window_commands.iter().any(|command| matches!(
            command,
            WindowCommand::Open { id, settings }
                if *id != WindowId::PRIMARY && *id != TOOL && settings.role == WindowRole::Tool
        )));
        assert!(
            fixture
                .documents
                .keys()
                .any(|id| *id != WindowId::PRIMARY && *id != TOOL)
        );

        let opened = apply_ax(
            &mut fixture,
            WindowId::PRIMARY,
            open_tool,
            AccessibilityAction::Click,
        );
        assert!(opened.window_commands.iter().any(|command| matches!(
            command,
            WindowCommand::Open { id, settings }
                if *id == TOOL && settings.role == WindowRole::Tool
        )));
    }

    fn name_state(fixture: &Fixture) -> (String, TextSelection) {
        fixture
            .documents
            .get(&WindowId::PRIMARY)
            .expect("primary")
            .context()
            .read(fixture.name, |input| {
                (input.state.value.clone(), input.state.selection)
            })
            .expect("read name")
    }

    #[test]
    fn ime_preedit_and_commit_update_focused_text_input() {
        let mut fixture = Fixture::mount().expect("mount");
        let name = fixture.name.stable_id();
        let document = fixture.documents.get(&WindowId::PRIMARY).expect("primary");
        assert_eq!(
            document.context().world().focused(document.document()),
            Some(name)
        );
        let (committed, selection) = name_state(&fixture);
        let preedit = "你";
        let commit = "你好";

        let preedited = fixture
            .on_window_event(WindowEvent::Ime {
                id: WindowId::PRIMARY,
                event: ImeEvent::Preedit {
                    text: preedit.into(),
                    selection: Some((0, preedit.len())),
                },
            })
            .expect("preedit");
        assert_eq!(preedited.redraw, RuntimeRedraw::Window(WindowId::PRIMARY));

        let (value, sel) = name_state(&fixture);
        assert_eq!(value, committed);
        assert_eq!(sel, selection);
        let composition = fixture
            .documents
            .get(&WindowId::PRIMARY)
            .expect("primary")
            .context()
            .world()
            .ime(name)
            .expect("preedit composition");
        assert_eq!(composition.text, preedit);
        assert_eq!(composition.selection, Some((0, preedit.len())));

        let committed_update = fixture
            .on_window_event(WindowEvent::Ime {
                id: WindowId::PRIMARY,
                event: ImeEvent::Commit(commit.into()),
            })
            .expect("commit");
        assert_eq!(
            committed_update.redraw,
            RuntimeRedraw::Window(WindowId::PRIMARY)
        );

        let (value, sel) = name_state(&fixture);
        assert_eq!(value, format!("{committed}{commit}"));
        assert_eq!(sel, TextSelection::caret(value.len()));
        assert_eq!(
            fixture
                .documents
                .get(&WindowId::PRIMARY)
                .expect("primary")
                .context()
                .world()
                .ime(name),
            None
        );
    }
}
