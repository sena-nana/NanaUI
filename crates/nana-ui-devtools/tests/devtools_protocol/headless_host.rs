//! A whole program run by the headless host: its hooks, a window it asks
//! for, a file dialog and routed input, then pixels.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nana_ui::runtime::{RuntimeDocument, Task};
use nana_ui::{
    DocumentAccessError, RoutedInput, RuntimeProgram, RuntimeProgramContext, RuntimeProgramUpdate,
    WindowDescriptor,
};
use nana_ui_core::{FileDialogKind, FileDialogRequest, FileFilter};
use nana_ui_devtools::agent::{A11yFilter, Target, fixtures};
use nana_ui_devtools::headless::{
    HeadlessFileDialog, HeadlessHost, HeadlessOptions, headless_geometry,
};
use nana_ui_devtools::offscreen;
use nana_ui_platform::host::WindowCommand;
use nana_ui_platform::{WindowEvent, WindowId};

enum Message {
    OpenAux,
    AuxCreated(Result<WindowId, String>),
    Browse,
    Export,
    Zoom(f32),
    ScheduleWake(Instant),
}

#[derive(Default)]
struct Seen {
    binds: usize,
    inputs: usize,
    prepared: usize,
    presented: usize,
    woken: usize,
    aux: Option<WindowId>,
    picked: Vec<PathBuf>,
    picked_filter: Option<usize>,
}

struct Probe {
    primary: RuntimeDocument,
    aux: Option<(WindowId, RuntimeDocument)>,
    wake_at: Option<Instant>,
    seen: Seen,
}

impl RuntimeProgram for Probe {
    type Message = Message;
    type Error = String;

    fn initialize(
        _context: &RuntimeProgramContext<Message>,
    ) -> Result<(Self, Vec<Message>), String> {
        let primary = fixtures::build("counter").ok_or("no counter fixture")?;
        Ok((
            Self {
                primary,
                aux: None,
                wake_at: None,
                seen: Seen::default(),
            },
            Vec::new(),
        ))
    }

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        if id == WindowId::PRIMARY {
            return Ok(Some(f(&self.primary)));
        }
        Ok(self
            .aux
            .as_ref()
            .filter(|(aux, _)| *aux == id)
            .map(|(_, document)| f(document)))
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError> {
        if id == WindowId::PRIMARY {
            return Ok(Some(f(&mut self.primary)));
        }
        Ok(self
            .aux
            .as_mut()
            .filter(|(aux, _)| *aux == id)
            .map(|(_, document)| f(document)))
    }

    fn update(
        &mut self,
        message: Message,
        context: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        match message {
            Message::OpenAux => {
                let request = context.windows().create_window(
                    WindowDescriptor::new("Aux")
                        .initial_size(240.0, 160.0)
                        .tag("aux"),
                );
                let task = Task::new(async move {
                    Message::AuxCreated(
                        request
                            .await
                            .map(|handle| handle.id())
                            .map_err(|error| error.to_string()),
                    )
                });
                context.run_task(task).expect("task queue");
                RuntimeProgramUpdate::default()
            }
            Message::AuxCreated(result) => {
                self.seen.aux = result.ok();
                RuntimeProgramUpdate::default()
            }
            Message::Browse => RuntimeProgramUpdate {
                window_commands: vec![WindowCommand::OpenFileDialog {
                    id: WindowId::PRIMARY,
                    request: FileDialogRequest::new(7, FileDialogKind::OpenFile),
                }],
                ..RuntimeProgramUpdate::default()
            },
            Message::Zoom(zoom) => RuntimeProgramUpdate {
                window_commands: vec![WindowCommand::SetContentZoom {
                    id: WindowId::PRIMARY,
                    zoom,
                }],
                ..RuntimeProgramUpdate::default()
            },
            Message::Export => RuntimeProgramUpdate {
                window_commands: vec![WindowCommand::OpenFileDialog {
                    id: WindowId::PRIMARY,
                    request: FileDialogRequest::new(7, FileDialogKind::SaveFile)
                        .filters([
                            FileFilter::new("JSON", ["json"]),
                            FileFilter::new("Text", ["txt"]),
                        ])
                        .enforce_extension(true),
                }],
                ..RuntimeProgramUpdate::default()
            },
            Message::ScheduleWake(at) => {
                self.wake_at = Some(at);
                RuntimeProgramUpdate::default()
            }
        }
    }

    fn next_wakeup(&self) -> Option<Instant> {
        self.wake_at
    }

    fn wake(
        &mut self,
        now: Instant,
        _context: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        if self.wake_at.is_some_and(|due| due <= now) {
            self.wake_at = None;
            self.seen.woken += 1;
        }
        RuntimeProgramUpdate::default()
    }

    fn initialize_window(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Message>,
    ) -> Result<(), String> {
        if context.window_tag() != Some("aux") {
            return Err("unexpected window".into());
        }
        let document = fixtures::build("form-controls").ok_or("no fixture")?;
        self.aux = Some((id, document));
        Ok(())
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        if let WindowEvent::FileDialogCompleted { result, .. } = event {
            assert_eq!(result.id, 7);
            self.seen.picked = result.paths;
            self.seen.picked_filter = result.selected_filter;
        }
        RuntimeProgramUpdate::default()
    }

    fn input_event(
        &mut self,
        _id: WindowId,
        _input: RoutedInput<'_>,
        _context: &RuntimeProgramContext<Message>,
    ) -> Result<RuntimeProgramUpdate, nana_ui::runtime::FrameworkError> {
        self.seen.inputs += 1;
        Ok(RuntimeProgramUpdate::default())
    }

    fn bind_window(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        self.seen.binds += 1;
        RuntimeProgramUpdate::default()
    }

    fn prepare_window_frame(&mut self, _id: WindowId, _context: &RuntimeProgramContext<Message>) {
        self.seen.prepared += 1;
    }

    fn window_frame_presented(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Message>,
    ) -> RuntimeProgramUpdate {
        self.seen.presented += 1;
        RuntimeProgramUpdate::default()
    }
}

fn host() -> Option<HeadlessHost<Probe>> {
    let gpu = offscreen::optional()?.gpu;
    Some(
        HeadlessHost::new(
            gpu,
            WindowDescriptor::new("Probe"),
            headless_geometry((320.0, 200.0), 1.0),
            HeadlessOptions::default(),
        )
        .expect("headless host"),
    )
}

fn turns(host: &mut HeadlessHost<Probe>, until: impl Fn(&Probe) -> bool) {
    for _ in 0..200 {
        host.turn();
        if until(host.program()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the program never reached the expected state");
}

#[test]
fn hooks_run_in_native_order_and_the_primary_binds_after_its_first_frame() {
    let Some(mut host) = host() else { return };
    host.turn();
    let seen = &host.program().seen;
    assert_eq!((seen.prepared, seen.presented, seen.binds), (1, 1, 1));
    host.turn();
    // Binding follows a message, not every frame.
    assert_eq!(host.program().seen.binds, 1);
}

#[test]
fn routed_input_reaches_the_document_and_the_program() {
    let Some(mut host) = host() else { return };
    host.turn();
    let button = host
        .accessibility(WindowId::PRIMARY)
        .into_iter()
        .find(|node| node.label.as_deref() == Some("Increment 0"))
        .expect("counter button");
    let center = (
        button.bounds.x + button.bounds.width * 0.5,
        button.bounds.y + button.bounds.height * 0.5,
    );
    host.click(WindowId::PRIMARY, center).unwrap();
    host.turn();
    assert_eq!(host.program().seen.inputs, 3);
    assert!(
        host.accessibility(WindowId::PRIMARY)
            .iter()
            .any(|node| node.label.as_deref() == Some("Increment 1"))
    );
}

#[test]
fn a_requested_window_opens_through_initialize_window() {
    let Some(mut host) = host() else { return };
    host.dispatch(Message::OpenAux);
    turns(&mut host, |program| program.seen.aux.is_some());
    let id = host.window_by_tag("aux").expect("aux window");
    assert_eq!(host.program().seen.aux, Some(id));
    assert_eq!(
        host.window(id).unwrap().geometry.logical_size,
        (240.0, 160.0)
    );
    host.turn();
    assert!(!host.accessibility(id).is_empty());
    let (size, pixels) = host.screenshot(id, [0.0, 0.0, 0.0, 1.0]).unwrap();
    assert_eq!((size.width, size.height), (240, 160));
    assert!(pixels.chunks(4).any(|pixel| pixel[..3] != [0, 0, 0]));
}

#[test]
fn a_file_dialog_is_answered_once_and_logged() {
    let Some(mut host) = host() else { return };
    let path = PathBuf::from("/picked/file.txt");
    host.set_file_dialog(HeadlessFileDialog::Select(vec![path.clone()]));
    host.dispatch(Message::Browse);
    host.turn();
    assert_eq!(host.program().seen.picked, vec![path]);
    assert!(matches!(
        host.take_window_commands().as_slice(),
        [WindowCommand::OpenFileDialog { .. }]
    ));
    // The answer was used up: the next dialog is cancelled.
    host.dispatch(Message::Browse);
    host.turn();
    assert!(host.program().seen.picked.is_empty());
}

#[test]
fn an_injected_filter_reaches_the_program_with_the_save_policy_applied() {
    let Some(mut host) = host() else { return };
    host.set_file_dialog(HeadlessFileDialog::SelectWithFilter(
        vec![PathBuf::from("/picked/v1.2")],
        1,
    ));
    host.dispatch(Message::Export);
    host.turn();
    assert_eq!(
        host.program().seen.picked,
        vec![PathBuf::from("/picked/v1.2.txt")]
    );
    assert_eq!(host.program().seen.picked_filter, Some(1));
}

#[test]
fn a_screenshot_paints_the_primary_document() {
    let Some(mut host) = host() else { return };
    host.turn();
    let (size, pixels) = host
        .screenshot(WindowId::PRIMARY, [0.0, 0.0, 0.0, 1.0])
        .unwrap();
    assert_eq!((size.width, size.height), (320, 200));
    let colors: std::collections::HashSet<_> = pixels
        .chunks(4)
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    assert!(colors.len() > 8, "only {} colours painted", colors.len());
}

/// The right and bottom edges of what is not the clear colour.
fn painted_extent(size: (u32, u32), pixels: &[u8]) -> (u32, u32) {
    let mut extent = (0, 0);
    for (index, pixel) in pixels.chunks(4).enumerate() {
        if pixel[..3] != [0, 0, 0] {
            let (x, y) = (index as u32 % size.0, index as u32 / size.0);
            extent = (extent.0.max(x + 1), extent.1.max(y + 1));
        }
    }
    extent
}

/// A content zoom paints as the native host does: what the document draws
/// in its smaller content viewport comes out twice as large on the same
/// surface, not at its old size in the top-left corner.
#[test]
fn a_screenshot_paints_at_the_content_zoom() {
    let Some(mut host) = host() else { return };
    host.turn();
    let (size, plain) = host
        .screenshot(WindowId::PRIMARY, [0.0, 0.0, 0.0, 1.0])
        .unwrap();
    host.dispatch(Message::Zoom(2.0));
    host.turn();
    assert_eq!(host.window(WindowId::PRIMARY).unwrap().content_zoom, 2.0);
    let (zoomed_size, zoomed) = host
        .screenshot(WindowId::PRIMARY, [0.0, 0.0, 0.0, 1.0])
        .unwrap();
    assert_eq!((zoomed_size.width, zoomed_size.height), (320, 200));
    let surface = (size.width, size.height);
    let plain = painted_extent(surface, &plain);
    let zoomed = painted_extent(surface, &zoomed);
    assert!(plain.0 > 0 && plain.1 > 0, "nothing painted");
    // The bottom edge moves out to about twice as far (or to the surface's
    // edge). Widths reflow in the narrower content viewport, so they are not
    // compared.
    assert!(
        zoomed.1 + 3 >= (plain.1 * 2).min(surface.1),
        "{plain:?} -> {zoomed:?}"
    );
}

#[test]
fn manual_clock_drives_wakeups_and_quiesce_without_wall_sleep() {
    let Some(mut host) = host() else { return };
    host.use_manual_clock();
    assert!(host.is_manual_clock());

    let now = host.now_instant();
    host.dispatch(Message::ScheduleWake(now + Duration::from_millis(500)));
    host.step(Duration::from_millis(100));
    assert_eq!(host.next_wakeup(), Some(now + Duration::from_millis(500)));
    assert_eq!(host.program().seen.woken, 0);

    let wall_start = Instant::now();
    host.step(Duration::from_millis(400));
    assert_eq!(host.program().seen.woken, 1);
    assert!(wall_start.elapsed() < Duration::from_millis(250));

    let quiesced = host.turn_until_quiescent(Duration::from_millis(16), 32);
    assert!(quiesced.settled);
    assert!(quiesced.turns >= 1);

    host.use_system_clock();
    assert!(!host.is_manual_clock());
}

#[test]
fn headless_host_exposes_target_inspection_and_png_diff() {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let Some(mut host) = host() else { return };
    host.use_manual_clock();
    let quiesced = host.turn_until_quiescent(Duration::from_millis(16), 32);
    assert!(quiesced.settled);

    let (all, truncated_all) =
        host.filtered_accessibility(WindowId::PRIMARY, &A11yFilter::default());
    assert!(!truncated_all);
    let (interactive, _) = host.filtered_accessibility(
        WindowId::PRIMARY,
        &A11yFilter {
            interactive_only: true,
            ..A11yFilter::default()
        },
    );
    assert!(!interactive.is_empty() && interactive.len() <= all.len());

    let button_target = Target {
        label: Some("Increment 0".into()),
        ..Target::default()
    };
    let node = host
        .resolve_target(WindowId::PRIMARY, &button_target)
        .expect("resolve button");
    let center = host
        .target_point(WindowId::PRIMARY, &button_target)
        .expect("button point");
    let hits = host
        .hit_test(WindowId::PRIMARY, center.0, center.1)
        .expect("hit test");
    assert!(!hits.is_empty());
    let probe = host
        .scene_probe(WindowId::PRIMARY, &button_target)
        .expect("scene probe");
    assert!(probe.in_viewport && probe.primitive_count > 0);
    let inspect = host
        .inspect(
            WindowId::PRIMARY,
            &Target {
                node: Some(node),
                ..Target::default()
            },
        )
        .expect("inspect");
    assert!(inspect.layout_node.is_some());

    host.click_target(WindowId::PRIMARY, &button_target)
        .expect("click target");
    host.turn_until_quiescent(Duration::from_millis(16), 32);
    let updated = Target {
        label: Some("Increment 1".into()),
        ..Target::default()
    };
    host.activate_target(WindowId::PRIMARY, &updated)
        .expect("activate target");
    host.turn_until_quiescent(Duration::from_millis(16), 32);
    assert!(
        host.resolve_target(
            WindowId::PRIMARY,
            &Target {
                label: Some("Increment 2".into()),
                ..Target::default()
            }
        )
        .is_ok()
    );

    let png_path = std::env::temp_dir().join(format!(
        "nana-headless-host-{}-{}.png",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let stats = host
        .screenshot_png(WindowId::PRIMARY, [0.0, 0.0, 0.0, 1.0], &png_path)
        .expect("screenshot png");
    assert!(stats.unique_colors > 1);
    let diff = host
        .diff_screenshot(WindowId::PRIMARY, [0.0, 0.0, 0.0, 1.0], &png_path, 0)
        .expect("diff screenshot");
    assert_eq!(diff.changed_pixels, 0);
    let _ = std::fs::remove_file(&png_path);
}
