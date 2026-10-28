//! The retained document a mobile shell hosts.

use std::sync::Arc;
use std::time::Instant;

use nana_ui::runtime::view::{Mount, entity_ref, widget, with_refs};
use nana_ui::runtime::{
    AccessibilityActionRequest, AppContext, DocumentId, Entity, FrameworkError, LayoutViewport,
    List, RuntimeDocument,
};
use nana_ui::{AccessibilityNode, NanaTextShaper, RuntimeAnimationClock};
use nana_ui_core::{
    FlexDirection, LengthSpec, SemanticColorRole, ThemeAppearance, builtin_theme_arc,
};
use nana_ui_platform::{
    CanonicalInputEvent, CommittedText, CompositionInput, CursorIcon, DeviceId, EndpointGeneration,
    HostServiceError, HostServices, InputModifiers, InputPayload, InputSequence, InputSequencer,
    InputSourceId, KeyInput, PointerId, PointerInput, PointerPhase, PointerType,
    SharedClipboardHost, TextInputContext, read_shared_clipboard, write_shared_clipboard,
};

use crate::schedule::FrameDemand;
use crate::viewport::MobileViewport;

/// What the native shell does for the document.
pub trait MobilePlatform {
    /// The focused editor changed, or its caret moved: show, update or hide
    /// the soft keyboard and the native text proxy (UIKit `UITextInput`,
    /// Android `InputConnection`). `None` hides them.
    fn set_text_input(&mut self, state: Option<&TextInputContext>);

    /// The system pasteboard, injected by the shell.
    fn clipboard(&self) -> SharedClipboardHost;
}

/// The system appearance the shell reads (`UITraitCollection`, Android
/// `Configuration`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobileAppearance {
    pub dark: bool,
    /// The system text size: Dynamic Type or `fontScale`, 1.0 standard.
    pub text_scale: f32,
}

impl Default for MobileAppearance {
    fn default() -> Self {
        Self {
            dark: false,
            text_scale: 1.0,
        }
    }
}

/// One phase of the primary touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    Cancel,
}

/// The focused editor as the native text proxy mirrors it, byte offsets into
/// `text`: the committed text with the preedit in place of the range it
/// stands in for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInputSnapshot {
    pub text: String,
    pub selection: (usize, usize),
    /// The marked (preedit) range, while composing.
    pub marked: Option<(usize, usize)>,
}

struct Services {
    platform: Box<dyn MobilePlatform>,
    clipboard: SharedClipboardHost,
}

impl HostServices for Services {
    fn set_cursor(&mut self, _cursor: CursorIcon) {}

    fn set_text_input(&mut self, state: Option<&TextInputContext>) {
        self.platform.set_text_input(state);
    }

    fn read_clipboard(&mut self) -> Result<Option<String>, HostServiceError> {
        read_shared_clipboard(&self.clipboard)
    }

    fn write_clipboard(&mut self, text: &str) -> Result<(), HostServiceError> {
        write_shared_clipboard(&self.clipboard, text)
    }
}

/// The shell's one input source.
const SOURCE: InputSourceId = InputSourceId(1);

/// One retained NanaUI document filling the surface. The application mounts
/// its view with [`Self::mount`] inside a root padded by the safe area.
pub struct MobileRuntime {
    document: RuntimeDocument,
    shaper: NanaTextShaper,
    sequencer: InputSequencer,
    services: Services,
    clock: RuntimeAnimationClock,
    viewport: MobileViewport,
    appearance: MobileAppearance,
    applied: Option<(ThemeAppearance, f32)>,
    root: Entity<List>,
    /// The touch the document follows; other fingers are ignored.
    primary_touch: Option<u64>,
    ime_enabled: bool,
    /// Something changed that the last presented frame does not show.
    needs_present: bool,
}

impl MobileRuntime {
    pub fn new(
        viewport: MobileViewport,
        appearance: MobileAppearance,
        platform: Box<dyn MobilePlatform>,
    ) -> Result<Self, FrameworkError> {
        let document_id = DocumentId::new(1).expect("mobile document id");
        let mut document = RuntimeDocument::new(document_id);
        document
            .context_mut()
            .bind_input_source(SOURCE, EndpointGeneration(1), document_id)
            .expect("a fresh mobile document binds its source");
        let (_, root) = document.context_mut().mount_view_root(document_id, || {
            let root = entity_ref();
            with_refs(
                widget(root_host()).key("mobile-root").entity_ref(root),
                root,
            )
        })?;
        let clipboard = platform.clipboard();
        let mut runtime = Self {
            document,
            shaper: NanaTextShaper::default(),
            sequencer: InputSequencer::new(SOURCE, EndpointGeneration(1)),
            services: Services {
                platform,
                clipboard,
            },
            clock: RuntimeAnimationClock::now(),
            viewport,
            appearance,
            applied: None,
            root,
            primary_touch: None,
            ime_enabled: false,
            needs_present: true,
        };
        runtime.pad_root()?;
        runtime.flush()?;
        Ok(runtime)
    }

    /// Mount the application's view inside the root; what it returns is the
    /// view's refs.
    pub fn mount<M: Mount>(
        &mut self,
        view: impl FnOnce() -> M,
    ) -> Result<M::Output, FrameworkError> {
        let root = self.root.stable_id();
        let output = self.document.context_mut().mount_view(root, view)?;
        self.needs_present = true;
        self.flush()?;
        Ok(output)
    }

    pub fn viewport(&self) -> MobileViewport {
        self.viewport
    }

    pub fn set_viewport(&mut self, viewport: MobileViewport) -> Result<(), FrameworkError> {
        if viewport == self.viewport {
            return Ok(());
        }
        let padding_changed = viewport.logical_safe_area() != self.viewport.logical_safe_area();
        self.viewport = viewport;
        self.needs_present = true;
        if padding_changed {
            self.pad_root()?;
        }
        Ok(())
    }

    pub fn set_appearance(&mut self, appearance: MobileAppearance) {
        if appearance != self.appearance {
            self.appearance = appearance;
            self.needs_present = true;
        }
    }

    /// Something outside the document changed what it shows (a host texture
    /// the shell invalidated, a new surface): present the next frame.
    pub fn request_present(&mut self) {
        self.needs_present = true;
    }

    pub fn document(&self) -> &RuntimeDocument {
        &self.document
    }

    /// The application's context: signals, stores, focus.
    pub fn context_mut(&mut self) -> &mut AppContext {
        self.needs_present = true;
        self.document.context_mut()
    }

    /// When the display link should fire next.
    pub fn frame_demand(&self) -> FrameDemand {
        let context = self.document.context();
        let dirty = self.needs_present
            || context.has_pending_reactive()
            || context.world().has_pending_work()
            || self.document.compositor_needs_tick();
        if dirty {
            return FrameDemand::Now;
        }
        FrameDemand::from_deadline(self.clock.next_wakeup(context))
    }

    /// One display-link tick: ready view tasks, due animations and the
    /// compositor clock, then layout. Returns whether the frame has anything
    /// new to present; on `false` the shell skips painting.
    pub fn prepare_frame(&mut self, now: Instant) -> Result<bool, FrameworkError> {
        let mut changed = std::mem::take(&mut self.needs_present);
        if nana_ui::runtime::view::poll_tasks() > 0 {
            changed = true;
        }
        let context = self.document.context();
        if self
            .clock
            .next_wakeup(context)
            .is_some_and(|deadline| deadline <= now)
        {
            let frame = self.clock.wake(self.document.context_mut(), now);
            changed |= frame.has_updates();
        }
        if self.document.compositor_needs_tick() {
            self.document
                .sync_presentation_clock(self.clock.runtime_time(now));
            changed = true;
        }
        changed |= self.flush()?;
        Ok(changed)
    }

    /// Route one sample of the primary touch, in physical px. Returns
    /// `false` for a sample the document does not follow (a second finger).
    pub fn push_touch(
        &mut self,
        phase: TouchPhase,
        physical_x: f32,
        physical_y: f32,
        pointer_id: u64,
    ) -> Result<bool, FrameworkError> {
        match (phase, self.primary_touch) {
            (TouchPhase::Down, None) => self.primary_touch = Some(pointer_id),
            (TouchPhase::Down, Some(_)) => return Ok(false),
            (_, Some(primary)) if primary == pointer_id => {
                if matches!(phase, TouchPhase::Up | TouchPhase::Cancel) {
                    self.primary_touch = None;
                }
            }
            _ => return Ok(false),
        }
        let [x, y] = self.viewport.logical_point(physical_x, physical_y);
        if phase == TouchPhase::Down && !self.focused_editor_contains(x, y) {
            // A tap elsewhere commits the preedit while the editor still has
            // focus; the world drops a blurred node's composition.
            self.commit_ime_on_blur()?;
        }
        let (phase, buttons) = match phase {
            TouchPhase::Down => (PointerPhase::Down, 1),
            TouchPhase::Move => (PointerPhase::Move, 1),
            TouchPhase::Up => (PointerPhase::Up, 0),
            TouchPhase::Cancel => (PointerPhase::Cancel, 0),
        };
        self.dispatch(InputPayload::Pointer(PointerInput {
            phase,
            pointer_id: PointerId(pointer_id),
            pointer_type: PointerType::Touch,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons,
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        }))?;
        self.sync_ime_lifecycle()?;
        Ok(true)
    }

    /// Route a hardware key, and the text it types after it.
    pub fn push_key(&mut self, key: KeyInput, text: Option<String>) -> Result<(), FrameworkError> {
        let sequence = self.route(InputPayload::Key(key))?;
        if let Some(text) = text {
            self.route(InputPayload::Text(CommittedText {
                text,
                key: Some(sequence),
            }))?;
        }
        self.flush()?;
        self.sync_ime_lifecycle()
    }

    /// Route a composition event from the native text proxy into the focused
    /// editor.
    pub fn push_composition(&mut self, event: CompositionInput) -> Result<(), FrameworkError> {
        if !self.ime_enabled {
            return Ok(());
        }
        self.dispatch(InputPayload::Composition(event))?;
        self.sync_ime_lifecycle()
    }

    /// The focused editor, for the native text proxy.
    pub fn text_input(&self) -> Option<TextInputSnapshot> {
        let document = self.document.document();
        let (target, state) = self.document.context().focused_text_input(document)?;
        if !self
            .document
            .context()
            .world()
            .accessibility(target)
            .is_some_and(|node| node.editable)
        {
            return None;
        }
        let session = state.session();
        let Some(composition) = session.composition() else {
            return Some(TextInputSnapshot {
                text: state.value.to_owned(),
                selection: (state.selection.anchor, state.selection.focus),
                marked: None,
            });
        };
        let marked = composition.display_range();
        let selection = match &composition.selection {
            Some(selection) => (marked.start + selection.start, marked.start + selection.end),
            None => (marked.end, marked.end),
        };
        Some(TextInputSnapshot {
            text: session.display_text().into_owned(),
            selection,
            marked: Some((marked.start, marked.end)),
        })
    }

    /// Accessibility nodes the shell publishes to VoiceOver / TalkBack.
    pub fn accessibility_nodes(&self) -> Vec<AccessibilityNode> {
        self.document
            .context()
            .world()
            .project_accessibility(self.document.document())
    }

    pub fn apply_accessibility_action(
        &mut self,
        request: AccessibilityActionRequest,
    ) -> Result<bool, FrameworkError> {
        let document = self.document.document();
        let applied = self
            .document
            .context_mut()
            .apply_accessibility_action(document, request)?;
        self.flush()?;
        self.sync_ime_lifecycle()?;
        Ok(applied)
    }

    /// Layout the document at the current viewport. Returns whether it did
    /// any work.
    fn flush(&mut self) -> Result<bool, FrameworkError> {
        self.apply_appearance()?;
        let (width, height) = self.viewport.logical_size();
        let update = self
            .document
            .flush(LayoutViewport::new(width, height), &mut self.shaper)?;
        let changed = !update.is_idle();
        self.needs_present |= changed;
        Ok(changed)
    }

    fn apply_appearance(&mut self) -> Result<(), FrameworkError> {
        let theme = if self.appearance.dark {
            ThemeAppearance::Dark
        } else {
            ThemeAppearance::Light
        };
        let wanted = (theme, self.appearance.text_scale);
        if self.applied == Some(wanted) {
            return Ok(());
        }
        let document = self.document.document();
        let context = self.document.context_mut();
        context.set_theme_tokens(builtin_theme_arc(theme))?;
        context.set_document_text_scale(document, Some(wanted.1));
        self.applied = Some(wanted);
        Ok(())
    }

    /// Pad the root by the safe area so the application's view fills what
    /// the system leaves uncovered; the root's background still covers the
    /// whole surface.
    fn pad_root(&mut self) -> Result<(), FrameworkError> {
        let safe = self.viewport.logical_safe_area();
        self.document
            .context_mut()
            .update_component(self.root, move |root, _| {
                let layout = Arc::make_mut(&mut root.style.layout);
                layout.padding_top = Some(LengthSpec::Px(safe.top));
                layout.padding_right = Some(LengthSpec::Px(safe.right));
                layout.padding_bottom = Some(LengthSpec::Px(safe.bottom));
                layout.padding_left = Some(LengthSpec::Px(safe.left));
            })
    }

    fn route(&mut self, payload: InputPayload) -> Result<InputSequence, FrameworkError> {
        let now = self.clock.runtime_time(Instant::now());
        let event = CanonicalInputEvent {
            metadata: self.sequencer.stamp(DeviceId(0), now),
            payload,
        };
        let sequence = event.metadata.sequence;
        match self.document.context_mut().route_input(
            &event,
            &mut self.services,
            Some(&mut self.shaper),
        ) {
            Ok(_) => Ok(sequence),
            Err(nana_ui::InputRouteError::Dispatch(error)) => Err(error),
            // One source, bound once, stamped in order: a rejection is a host
            // bug, reported as invalid input.
            Err(_) => Err(FrameworkError::InvalidInput),
        }
    }

    fn dispatch(&mut self, payload: InputPayload) -> Result<(), FrameworkError> {
        self.route(payload)?;
        self.flush().map(drop)
    }

    fn editor_focused(&self) -> bool {
        self.text_input().is_some()
    }

    fn focused_editor_contains(&self, x: f32, y: f32) -> bool {
        let document = self.document.document();
        let context = self.document.context();
        let Some((target, _)) = context.focused_text_input(document) else {
            return false;
        };
        context.world().layout_box(target).is_some_and(|layout| {
            x >= layout.x
                && x < layout.x + layout.width
                && y >= layout.y
                && y < layout.y + layout.height
        })
    }

    fn sync_ime_lifecycle(&mut self) -> Result<(), FrameworkError> {
        let focused = self.editor_focused();
        if focused && !self.ime_enabled {
            self.ime_enabled = true;
            self.dispatch(InputPayload::Composition(CompositionInput::Enabled))?;
        } else if !focused && self.ime_enabled {
            self.ime_enabled = false;
        }
        Ok(())
    }

    fn commit_ime_on_blur(&mut self) -> Result<(), FrameworkError> {
        let enabled = std::mem::replace(&mut self.ime_enabled, false);
        if !enabled || !self.editor_focused() {
            return Ok(());
        }
        self.dispatch(InputPayload::Composition(CompositionInput::Disabled))
    }
}

fn root_host() -> List {
    let mut list = List::new();
    list.style.background = Some(SemanticColorRole::Background);
    let layout = Arc::make_mut(&mut list.style.layout);
    layout.direction = Some(FlexDirection::Column);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    list
}

#[cfg(test)]
mod tests;
