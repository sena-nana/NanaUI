//! What one interaction in a LARGE editor costs, and where that cost sits.
//!
//! Issue #96 left one decision to real numbers: whether the editable text's
//! `String` storage has to become a rope or piece table. A `String` charges
//! O(bytes after the caret) per edit, so the answer is only visible on a
//! document big enough for that memmove to matter -- and only next to the other
//! per-interaction costs, because replacing the storage is pointless while
//! something else on the same keystroke is already O(document).
//!
//! Every cell therefore reports four times for the same interaction:
//!
//! - `input_ms`: the event itself (`AppContext::replace_focused_text`,
//!   `move_focused_text_caret`, a pointer press). This is where the editor's
//!   value is cloned and where probes that are NOT inside a
//!   [`TextShaper::with_text_probes`](nana_ui_runtime::TextShaper) batch land.
//! - `flush_ms`: the frame that follows it, through `RuntimeDocument::flush`.
//! - `storage_ms`: the same edit applied to a bare
//!   [`EditableText`](nana_text::EditableText) of the same text -- storage
//!   alone, with no layout, no presentation and no frame. This is the number
//!   a rope would improve.
//! - `value_clone_ms`: one whole-value `String::clone`. The unit of
//!   "O(document) once", so the other three can be read as multiples of it.
//!
//! Axes:
//!
//! - `--action type|delete|caret|select|vertical|click|ime`. An edit and a
//!   caret move scale completely differently: only the first one can shift
//!   paragraphs, and only the vertical moves and clicks ask the retained
//!   geometry for a point hit.
//! - `--position head|middle|tail`. A `String` edit at the head moves the
//!   whole document; at the tail it moves nothing. Paragraph geometry has the
//!   mirror-image asymmetry: an edit at the head shifts every following
//!   paragraph's start.
//! - `--lines`. The document is `lines` paragraphs of [`LINE_TEXT`], so bytes
//!   scale with it. The default sweep stops at
//!   [`EDIT_LINE_GRID`]'s last cell; pass `--lines 32000` for a ~1.2 MB
//!   document.
//!
//! The counters are per frame (means), so a cell whose edit relaid one
//! paragraph out of 8,000 reads `paragraphs_relayout_from_edit = 1`.

use std::fs;
use std::hint::black_box;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily, font_blob};
use nana_text::{EditableText, NativeTextEngine, TextWorkCounters};
use nana_ui_runtime::{
    DocumentId, Entity, FrameStage, LayoutViewport, NanaTextEngineShaper, StageStatus, TextArea,
    TextCaretIntent,
};
use nana_ui_scene::RuntimeDocument;
use serde::Serialize;

const DOCUMENT: u64 = 1;

/// One line of the benchmark document. Wide enough to make byte counts
/// realistic, narrow enough not to wrap in [`VIEWPORT`].
const LINE_TEXT: &str = "the quick brown fox jumps over it";

/// Line counts swept when `--lines` is not given.
///
/// The last cell is what makes the storage question answerable: 8,000 lines is
/// ~272 KB, so a head edit's memmove is a quarter of a megabyte and shows up
/// against everything else on the same keystroke.
const EDIT_LINE_GRID: [usize; 4] = [250, 1000, 4000, 8000];

fn viewport() -> LayoutViewport {
    LayoutViewport::new(600.0, 800.0)
}

/// `lines` paragraphs, each [`LINE_TEXT`] with its index, joined by line feeds.
fn document_text(lines: usize) -> String {
    let mut text = String::with_capacity(lines * (LINE_TEXT.len() + 8));
    for line in 0..lines {
        if line > 0 {
            text.push('\n');
        }
        text.push_str(LINE_TEXT);
        text.push(' ');
        text.push_str(&line.to_string());
    }
    text
}

/// An engine holding only the bundled UI face, as the generic `sans-serif`.
fn engine() -> Arc<Mutex<NativeTextEngine>> {
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_bytes(
            font_blob(nana_ui_core::fonts::UI_FONT_REGULAR),
            &FaceDescriptor::default(),
        )
        .expect("the bundled UI face registers");
    Arc::new(Mutex::new(NativeTextEngine::new(fonts)))
}

/// What the measured interaction is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Type one character. The only action that can shift paragraphs.
    Type,
    /// Backspace over one grapheme.
    Delete,
    /// Left/Right, alternating so the caret stays put over the run.
    Caret,
    /// Shift+Right/Left: a selection change, no text change.
    Select,
    /// Down/Up. Resolved by the backend's point hit test, so it reads the
    /// retained geometry rather than searching with position probes.
    Vertical,
    /// A pointer press, alternating between two columns of the same line.
    Click,
    /// An IME preedit update, alternating between two candidate spellings.
    Ime,
}

impl Action {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "type" => Some(Self::Type),
            "delete" => Some(Self::Delete),
            "caret" => Some(Self::Caret),
            "select" => Some(Self::Select),
            "vertical" => Some(Self::Vertical),
            "click" => Some(Self::Click),
            "ime" => Some(Self::Ime),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Delete => "delete",
            Self::Caret => "caret",
            Self::Select => "select",
            Self::Vertical => "vertical",
            Self::Click => "click",
            Self::Ime => "ime",
        }
    }

    /// Whether the action changes the text, and therefore has a storage cost a
    /// rope could change.
    fn edits(self) -> bool {
        matches!(self, Self::Type | Self::Delete)
    }

    /// Whether this step is one of the keystrokes the cell reports. Typing
    /// runs as insert/backspace pairs, so half the steps are the other
    /// direction and only put the document back.
    fn records(self, step: usize) -> bool {
        match self {
            Self::Type => step.is_multiple_of(2),
            Self::Delete => !step.is_multiple_of(2),
            _ => true,
        }
    }
}

/// Where the caret sits before the interaction.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    Head,
    Middle,
    Tail,
}

impl Position {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "head" => Some(Self::Head),
            "middle" => Some(Self::Middle),
            "tail" => Some(Self::Tail),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Middle => "middle",
            Self::Tail => "tail",
        }
    }

    /// A caret offset at the end of the first, middle or last line: inside a
    /// paragraph rather than on a line feed, so an edit stays in one paragraph.
    fn offset(self, text: &str) -> usize {
        let line = |index: usize| {
            text.split_inclusive('\n')
                .take(index + 1)
                .map(str::len)
                .sum::<usize>()
        };
        let lines = text.split_inclusive('\n').count();
        match self {
            Self::Head => line(0).saturating_sub(1),
            Self::Middle => line(lines / 2).saturating_sub(1),
            Self::Tail => text.len(),
        }
    }
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    phase: &'static str,
    profile: &'static str,
    samples: usize,
    warmup: usize,
    cells: Vec<Cell>,
}

#[derive(Serialize)]
struct Cell {
    action: &'static str,
    position: &'static str,
    lines: usize,
    bytes: usize,
    /// The interaction itself: value clones plus every probe outside a batch.
    input_ms: Stat,
    /// The frame that follows it.
    flush_ms: Stat,
    /// The same edit on a bare `EditableText`: storage alone. Zero for the
    /// actions that do not change the text.
    storage_ms: Stat,
    /// One whole-value `String::clone`, for scale.
    value_clone_ms: Stat,
    /// Which frame stage the flush spent its time in. Only the stages that
    /// ran are reported: a caret move never reaches the text shape pass.
    stages_ms: Vec<StageCell>,
    counters: Counters,
}

#[derive(Serialize)]
struct StageCell {
    stage: String,
    p50_ms: f64,
    mean_ms: f64,
}

/// Per-frame means of the text work the frame reported.
#[derive(Serialize, Default)]
struct Counters {
    editable_mutations: f64,
    editable_bytes_inserted: f64,
    editable_bytes_deleted: f64,
    caret_only_updates: f64,
    selection_only_updates: f64,
    composition_updates: f64,
    paragraphs_relayout_from_edit: f64,
    paragraphs_reshaped_from_edit: f64,
    layouts_created: f64,
    text_source_clones: f64,
    hit_test_queries: f64,
    caret_geometry_queries: f64,
}

impl Counters {
    fn of(total: &TextWorkCounters, samples: usize) -> Self {
        let per = |value: usize| value as f64 / samples as f64;
        Self {
            editable_mutations: per(total.editable_mutations),
            editable_bytes_inserted: per(total.editable_bytes_inserted),
            editable_bytes_deleted: per(total.editable_bytes_deleted),
            caret_only_updates: per(total.caret_only_updates),
            selection_only_updates: per(total.selection_only_updates),
            composition_updates: per(total.composition_updates),
            paragraphs_relayout_from_edit: per(total.paragraphs_relayout_from_edit),
            paragraphs_reshaped_from_edit: per(total.paragraphs_reshaped_from_edit),
            layouts_created: per(total.layouts_created),
            text_source_clones: per(total.text_source_clones),
            hit_test_queries: per(total.hit_test_queries),
            caret_geometry_queries: per(total.caret_geometry_queries),
        }
    }
}

#[derive(Serialize, Default)]
struct Stat {
    p50: f64,
    p95: f64,
    mean: f64,
    max: f64,
}

impl Stat {
    fn from_durations(mut samples: Vec<Duration>) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        samples.sort_unstable();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let at = |q: f64| {
            let index = ((samples.len() as f64 - 1.0) * q).round() as usize;
            ms(samples[index])
        };
        let mean = samples.iter().map(|d| ms(*d)).sum::<f64>() / samples.len() as f64;
        Self {
            p50: at(0.5),
            p95: at(0.95),
            mean,
            max: ms(*samples.last().unwrap()),
        }
    }
}

/// One focused, settled `TextArea` on an engine host.
struct Fixture {
    runtime: RuntimeDocument,
    shaper: NanaTextEngineShaper,
    document: DocumentId,
    area: Entity<TextArea>,
}

impl Fixture {
    fn new(text: &str) -> Self {
        let document = DocumentId::new(DOCUMENT).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let area = runtime
            .context_mut()
            .build(document, |ui| ui.child("editor", TextArea::new(text)))
            .expect("the editor mounts");
        let shaper = NanaTextEngineShaper::new(engine());
        let mut fixture = Self {
            runtime,
            shaper,
            document,
            area,
        };
        assert!(
            fixture
                .runtime
                .context_mut()
                .focus_node(document, fixture.area.stable_id())
                .expect("focus"),
            "the editor takes focus"
        );
        fixture.settle();
        fixture
    }

    fn flush(&mut self) -> TextWorkCounters {
        self.runtime
            .flush(viewport(), &mut self.shaper)
            .expect("flush");
        self.runtime.context().world().last_text_work_counters()
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            self.flush();
        }
    }

    fn place_caret(&mut self, offset: usize) {
        let document = self.document;
        assert!(
            self.runtime
                .context_mut()
                .select_focused_text_range(document, offset, offset)
                .expect("select"),
            "the caret lands at {offset}"
        );
        self.settle();
    }

    /// The content box the pointer coordinates are relative to, and one line's
    /// height.
    fn content(&self) -> (nana_ui_runtime::LayoutBox, f32) {
        let world = self.runtime.context().world();
        let node = self.area.stable_id();
        let content = world
            .text_input_pointer_context(node)
            .expect("the editor has a content box")
            .0;
        let line_height = world
            .text_input_presentation(node)
            .expect("the editor has a presentation")
            .line_height;
        (content, line_height)
    }

    /// Run the interaction. `step` alternates the direction so the caret and
    /// the text stay where the cell put them: an insert is followed by the
    /// backspace that undoes it, so a long run does not silently grow the
    /// line it types into (which would make later samples measure a longer
    /// paragraph than the cell claims).
    fn input(&mut self, action: Action, step: usize, point: (f32, f32)) {
        let document = self.document;
        let flip = step.is_multiple_of(2);
        let Fixture {
            runtime, shaper, ..
        } = self;
        let context = runtime.context_mut();
        match action {
            Action::Type | Action::Delete => {
                if flip {
                    context.replace_focused_text(document, "x").expect("type");
                } else {
                    context
                        .delete_focused_text_backward(document)
                        .expect("backspace");
                }
            }
            Action::Caret | Action::Select | Action::Vertical => {
                let intent = match (action, flip) {
                    (Action::Caret | Action::Select, true) => TextCaretIntent::Right,
                    (Action::Caret | Action::Select, false) => TextCaretIntent::Left,
                    (_, true) => TextCaretIntent::Down,
                    (_, false) => TextCaretIntent::Up,
                };
                context
                    .move_focused_text_caret(
                        document,
                        intent,
                        action == Action::Select,
                        Some(shaper),
                    )
                    .expect("caret move");
            }
            Action::Click => {
                let (x, y) = point;
                let x = if flip { x } else { x + 24.0 };
                context
                    .text_editor_pointer_press(
                        document,
                        self.area.stable_id(),
                        1,
                        x,
                        y,
                        false,
                        false,
                        Duration::from_secs(10),
                        shaper,
                    )
                    .expect("press");
                context.text_editor_pointer_release(1);
            }
            Action::Ime => {
                let preedit = if flip { "ni" } else { "n" };
                context
                    .set_ime_preedit(document, preedit.into(), None)
                    .expect("preedit");
            }
        }
    }
}

/// The storage-only cost of the cell's edit, on text that is not attached to
/// anything: one `EditableText` mutation at the same offset.
fn storage_sample(storage: &mut EditableText, action: Action, offset: usize) -> Duration {
    let offset = offset.min(storage.len());
    match action {
        Action::Type => {
            let start = Instant::now();
            let edit = storage.insert(offset, "x");
            let elapsed = start.elapsed();
            black_box(edit).expect("insert");
            elapsed
        }
        Action::Delete => {
            let from = nana_text::editable::navigation::prev_grapheme(storage.as_str(), offset)
                .unwrap_or(offset);
            let start = Instant::now();
            let edit = storage.delete(from..offset);
            let elapsed = start.elapsed();
            black_box(edit).expect("delete");
            elapsed
        }
        _ => Duration::ZERO,
    }
}

fn measure(
    action: Action,
    position: Position,
    lines: usize,
    samples: usize,
    warmup: usize,
) -> Cell {
    let text = document_text(lines);
    let bytes = text.len();
    let offset = position.offset(&text);
    let mut fixture = Fixture::new(&text);
    fixture.place_caret(offset);
    let (content, line_height) = fixture.content();
    // A point on the line the caret is on, a little way in.
    let caret_y = fixture
        .runtime
        .context()
        .world()
        .text_input_presentation(fixture.area.stable_id())
        .expect("presentation")
        .caret_y;
    let point = (
        content.x + 40.0,
        content.y + caret_y + line_height * 0.5 - fixture.scroll_y(),
    );

    let mut storage = EditableText::new(text.as_str());
    let mut inputs = Vec::with_capacity(samples);
    let mut flushes = Vec::with_capacity(samples);
    let mut storages = Vec::with_capacity(samples);
    let mut clones = Vec::with_capacity(samples);
    let mut counters = TextWorkCounters::default();
    let mut stage_samples: Vec<Vec<Duration>> = FrameStage::ALL
        .into_iter()
        .map(|_| Vec::with_capacity(samples))
        .collect();

    let mut iteration = 0;
    while flushes.len() < samples {
        let recorded = iteration >= warmup && action.records(iteration);
        let start = Instant::now();
        fixture.input(action, iteration, point);
        let input = start.elapsed();
        let start = Instant::now();
        let work = fixture.flush();
        let flush = start.elapsed();
        iteration += 1;
        if !recorded {
            continue;
        }
        let storage_time = storage_sample(&mut storage, action, offset);
        let start = Instant::now();
        let clone = text.clone();
        let clone_time = start.elapsed();
        black_box(clone);
        {
            let profile = fixture.runtime.context().last_frame_profile();
            for (index, stage) in FrameStage::ALL.into_iter().enumerate() {
                let timing = profile.stage(stage).expect("every stage is profiled");
                if timing.status == StageStatus::Ran {
                    stage_samples[index].push(timing.duration);
                }
            }
            inputs.push(input);
            flushes.push(flush);
            if action.edits() {
                storages.push(storage_time);
            }
            clones.push(clone_time);
            counters.accumulate(work);
        }
    }

    let mut stages_ms = Vec::new();
    for (index, stage) in FrameStage::ALL.into_iter().enumerate() {
        let taken = std::mem::take(&mut stage_samples[index]);
        if taken.is_empty() {
            continue;
        }
        let stat = Stat::from_durations(taken);
        stages_ms.push(StageCell {
            stage: format!("{stage:?}"),
            p50_ms: stat.p50,
            mean_ms: stat.mean,
        });
    }

    Cell {
        action: action.name(),
        position: position.name(),
        lines,
        bytes,
        input_ms: Stat::from_durations(inputs),
        flush_ms: Stat::from_durations(flushes),
        storage_ms: Stat::from_durations(storages),
        value_clone_ms: Stat::from_durations(clones),
        stages_ms,
        counters: Counters::of(&counters, samples),
    }
}

impl Fixture {
    /// The editor's own scroll offset, so a pointer point derived from the
    /// caret's presentation y lands on the caret's line.
    fn scroll_y(&self) -> f32 {
        self.runtime
            .context()
            .world()
            .text_input_pointer_context(self.area.stable_id())
            .map_or(0.0, |(_, scroll)| scroll.y)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|index| args.get(index + 1))
            .and_then(|raw| raw.parse::<usize>().ok())
    };
    let samples = value("--samples").unwrap_or(60);
    let warmup = value("--warmup").unwrap_or(10);
    let output = args
        .iter()
        .position(|arg| arg == "--output")
        .and_then(|index| args.get(index + 1))
        .cloned();
    let flag = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|index| args.get(index + 1))
    };

    let actions: Vec<Action> = match flag("--action") {
        None => vec![
            Action::Type,
            Action::Delete,
            Action::Caret,
            Action::Vertical,
            Action::Click,
        ],
        Some(raw) => vec![Action::parse(raw).unwrap_or_else(|| {
            eprintln!("--action must be type|delete|caret|select|vertical|click|ime, not `{raw}`");
            std::process::exit(2)
        })],
    };
    let positions: Vec<Position> = match flag("--position") {
        None => vec![Position::Head, Position::Tail],
        Some(raw) => vec![Position::parse(raw).unwrap_or_else(|| {
            eprintln!("--position must be head|middle|tail, not `{raw}`");
            std::process::exit(2)
        })],
    };
    let line_counts = value("--lines").map_or_else(|| EDIT_LINE_GRID.to_vec(), |lines| vec![lines]);

    let mut cells = Vec::new();
    for action in actions {
        for position in positions.iter().copied() {
            for lines in line_counts.iter().copied() {
                let cell = measure(action, position, lines, samples, warmup);
                eprintln!(
                    "{:<8} {:<6} lines={:<6} bytes={:<8} input p50={:.4} flush p50={:.4} storage p50={:.4} clone p50={:.4} ms",
                    cell.action,
                    cell.position,
                    cell.lines,
                    cell.bytes,
                    cell.input_ms.p50,
                    cell.flush_ms.p50,
                    cell.storage_ms.p50,
                    cell.value_clone_ms.p50,
                );
                eprintln!(
                    "         per frame: edits={:.2} +{:.2}/-{:.2} B  relayout={:.2} reshape={:.2} layouts={:.2} sources={:.2} hits={:.2} carets={:.2}",
                    cell.counters.editable_mutations,
                    cell.counters.editable_bytes_inserted,
                    cell.counters.editable_bytes_deleted,
                    cell.counters.paragraphs_relayout_from_edit,
                    cell.counters.paragraphs_reshaped_from_edit,
                    cell.counters.layouts_created,
                    cell.counters.text_source_clones,
                    cell.counters.hit_test_queries,
                    cell.counters.caret_geometry_queries,
                );
                for stage in &cell.stages_ms {
                    if stage.p50_ms > 0.0 {
                        eprintln!("         {:<16} {:.4} ms", stage.stage, stage.p50_ms);
                    }
                }
                cells.push(cell);
            }
        }
    }

    let report = Report {
        schema_version: 1,
        phase: "text-edit-scaling",
        profile: "release",
        samples,
        warmup,
        cells,
    };
    let json = serde_json::to_string_pretty(&report).expect("serialize report") + "\n";
    if let Some(path) = output {
        let path = std::path::PathBuf::from(path);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).expect("report directory");
        }
        fs::write(path, json).expect("write report");
    } else {
        print!("{json}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The storage question Issue #96 deferred is only answerable on a
    /// document whose tail is big enough for a `String` memmove to show, so
    /// the default sweep has to keep reaching it.
    #[test]
    fn the_default_sweep_reaches_a_document_of_at_least_a_quarter_megabyte() {
        let largest = EDIT_LINE_GRID
            .iter()
            .copied()
            .map(|lines| document_text(lines).len())
            .max()
            .unwrap();
        assert!(
            largest >= 256 * 1024,
            "the edit sweep tops out at {largest} bytes, too small to judge the storage"
        );
    }

    /// Head/middle/tail must really be different offsets in the document, or
    /// the position axis measures the same thing three times.
    #[test]
    fn the_positions_land_on_different_lines() {
        let text = document_text(100);
        let head = Position::Head.offset(&text);
        let middle = Position::Middle.offset(&text);
        let tail = Position::Tail.offset(&text);
        assert!(head < middle && middle < tail, "{head} {middle} {tail}");
        assert!(
            text.is_char_boundary(head) && text.is_char_boundary(middle),
            "offsets are caret positions"
        );
        assert_eq!(text[head..head + 1], *"\n", "head sits at the line's end");
    }
}
