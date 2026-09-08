//! Same hover sweep, three trees: Vue with listeners, Vue without, Rust L3.
//!
//! This exists because the Vue tier's one structural cost that the drawing path
//! does not share is input. A pointer move re-enters the JS engine (pointer
//! event plus its `mouse*` alias, more on a hover change) and then runs a full
//! host frame pump; L3 does neither. Nothing measured that before this binary:
//! `nana-vue-runtime-benchmark` never imports V8, so it times the Rust half of
//! the Vue tier and reports it as the Vue tier's cost.
//!
//! All three trees are built and driven in one process so the comparison is not
//! across machine states, and the timing is taken inside the process so a stdio
//! round trip is not folded into a per-event number.
//!
//! Both tiers are driven one level below `AgentSession` on purpose, so each side
//! pays exactly one dispatch plus one settle:
//!
//! - Vue: `dispatch_pointer` (which ends in one `pump_frame`) then
//!   `flush_scene_frame`, which is what a window's redraw does and what fills
//!   the paint-box store the tier reads back.
//! - L3: `hover_xy`, which is a Runtime pointer dispatch plus `flush`.
//!
//! Going through `AgentSession::pointer` instead would charge Vue for a second
//! `pump_frame` plus an unconditional `semantic_snapshot` that a window only
//! takes when the bridge revision moved -- harness cost reported as tier cost.
//! Dropping `flush_scene_frame` is the opposite error and worse: with no scene
//! frame the paint-box store stays empty, so `resolve_layout` takes its
//! "nothing painted yet" branch forever -- a path a real app leaves after its
//! first frame.
//!
//! Three Vue modes rather than one make the result actionable, because a single
//! "Vue hover" number would hide which of three separable things is expensive:
//! `bare` (no handler anywhere) is what the tier spends dispatching onto a path
//! nobody listens on -- the part `EventListeners` could prune; `listeners` adds
//! the call into JS without a re-render; `reactive` adds Vue patching the whole
//! column, which is what a naive hover handler over a long list actually does.
//!
//! ```bash
//! for mode in bare listeners reactive; do
//!   node crates/nana-js-engine/fixtures/vue-sfc-compat/build-hover-bench.mjs $mode 2000
//! done
//! cargo run --release -p nana-ui-devtools --features agent-bin \
//!   --bin nana-hover-benchmark -- --rows 2000 --moves 600
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use nana_js_engine::RuntimeArtifact;
use nana_js_v8::V8Engine;
use nana_ui::runtime::{DocumentId, LengthSpec, NodeStyle, RuntimeDocument, Stack, Text};
use nana_ui_devtools::agent::RuntimeAgentSession;
use nana_ui_vue::{PointerEventKind, PointerInput, VueHost};
use serde::Serialize;
use std::sync::Arc;

/// Matches `ROW_HEIGHT` in `src/HoverBench.js`. The sweep steps by this, so a
/// mismatch would silently measure repeated hovers of one row on one side and a
/// row change on the other.
const ROW_HEIGHT: f32 = 24.0;
const WIDTH: u32 = 320;
const HEIGHT: u32 = 480;

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    profile: &'static str,
    rows: usize,
    moves: usize,
    warmup: usize,
    cases: Vec<Case>,
}

#[derive(Serialize)]
struct Case {
    tier: &'static str,
    /// What the tree does with a pointer event, not how it was authored.
    mode: &'static str,
    mount_ms: f64,
    hover_ms: Distribution,
    /// Vue only: the split inside one event. `dispatch` is the pointer entering
    /// the tier -- hit test, DOM event dispatch into JS, the resulting patch and
    /// one host frame pump. `settle` is `flush_scene_frame`: commit host ops,
    /// run the Runtime systems, re-record every painted box.
    ///
    /// L3 leaves these `None`: `hover_xy` does both behind one call and the
    /// split is not reachable without reaching past the session's API.
    #[serde(skip_serializing_if = "Option::is_none")]
    dispatch_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    settle_ms: Option<Distribution>,
}

#[derive(Serialize)]
struct Distribution {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
    mean: f64,
}

impl Distribution {
    fn of(mut samples: Vec<f64>) -> Self {
        samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a duration"));
        let at = |q: f64| {
            let index = ((samples.len() as f64 - 1.0) * q).round() as usize;
            samples[index]
        };
        Self {
            p50: at(0.50),
            p95: at(0.95),
            p99: at(0.99),
            max: *samples.last().expect("at least one sample"),
            mean: samples.iter().sum::<f64>() / samples.len() as f64,
        }
    }
}

struct Args {
    rows: usize,
    moves: usize,
    warmup: usize,
    bundles: PathBuf,
    output: Option<PathBuf>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            rows: 2000,
            moves: 600,
            warmup: 60,
            bundles: PathBuf::from("target/hover-bench"),
            output: None,
        }
    }
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("nana-hover-benchmark: {error}");
            return ExitCode::from(2);
        }
    };

    let mut cases = Vec::new();
    for mode in ["bare", "listeners", "reactive"] {
        let bundle = args
            .bundles
            .join(format!("{mode}-{}", args.rows))
            .join("app.js");
        match vue_case(&bundle, mode, &args) {
            Ok(case) => cases.push(case),
            Err(error) => {
                eprintln!("nana-hover-benchmark: {error}");
                return ExitCode::from(1);
            }
        }
    }
    match runtime_case(&args) {
        Ok(case) => cases.push(case),
        Err(error) => {
            eprintln!("nana-hover-benchmark: {error}");
            return ExitCode::from(1);
        }
    }

    let report = Report {
        schema_version: 1,
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        rows: args.rows,
        moves: args.moves,
        warmup: args.warmup,
        cases,
    };
    let json = serde_json::to_string_pretty(&report).expect("report must serialize");
    match &args.output {
        Some(path) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(error) = std::fs::write(path, format!("{json}\n")) {
                eprintln!("nana-hover-benchmark: cannot write {}: {error}", path.display());
                return ExitCode::from(1);
            }
            println!("{}", path.display());
        }
        None => println!("{json}"),
    }
    ExitCode::SUCCESS
}

/// The sweep both tiers run: walk down the column one row at a time, wrapping.
///
/// Every step changes the hovered row, which is the expensive case -- a move
/// inside one row exercises neither tier's hover-change path.
fn sweep(index: usize, rows: usize) -> (f32, f32) {
    let row = index % rows.min(HEIGHT as usize / ROW_HEIGHT as usize).max(1);
    (WIDTH as f32 / 2.0, row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0)
}

/// Run the sweep, timing only the timed pass.
///
/// `hover` delivers one pointer move and lets that tier settle, and nothing
/// else -- what it does on each side is the comparison.
/// What one `measure` run produces: the whole event, and the two halves when
/// the tier exposes them.
type Timings = (Distribution, Option<Distribution>, Option<Distribution>);

/// One event's timings: total, and optionally the two halves.
#[derive(Default)]
struct Sample {
    dispatch: Option<f64>,
    settle: Option<f64>,
}

fn measure(
    args: &Args,
    mut hover: impl FnMut(f32, f32) -> Result<Sample, Box<dyn std::error::Error>>,
) -> Result<Timings, Box<dyn std::error::Error>> {
    for index in 0..args.warmup {
        let (x, y) = sweep(index, args.rows);
        hover(x, y)?;
    }
    let mut totals = Vec::with_capacity(args.moves);
    let mut dispatches = Vec::with_capacity(args.moves);
    let mut settles = Vec::with_capacity(args.moves);
    for index in 0..args.moves {
        let (x, y) = sweep(index, args.rows);
        let started = Instant::now();
        let sample = hover(x, y)?;
        totals.push(as_ms(started.elapsed()));
        if let Some(dispatch) = sample.dispatch {
            dispatches.push(dispatch);
        }
        if let Some(settle) = sample.settle {
            settles.push(settle);
        }
    }
    Ok((
        Distribution::of(totals),
        (!dispatches.is_empty()).then(|| Distribution::of(dispatches)),
        (!settles.is_empty()).then(|| Distribution::of(settles)),
    ))
}

fn as_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn vue_case(
    bundle: &Path,
    mode: &'static str,
    args: &Args,
) -> Result<Case, Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(bundle).map_err(|error| {
        format!(
            "cannot read {}: {error}\nBuild it first:\n  node crates/nana-js-engine/fixtures/vue-sfc-compat/build-hover-bench.mjs {mode} {}",
            bundle.display(),
            args.rows
        )
    })?;
    let artifact = RuntimeArtifact::from_source(bundle.to_string_lossy(), source);
    let started = Instant::now();
    let mut engine = V8Engine::new();
    let mut host = VueHost::with_viewport(WIDTH, HEIGHT, 1.0);
    host.initialize_with_web_api(&mut engine, artifact)?;
    host.bind_event_bridge(&mut engine)?;
    // The mount's own frame, so the first timed move is not the one that pays
    // for the initial layout and the first paint-box fill.
    host.pump_frame(&mut engine)?;
    host.flush_scene_frame(WIDTH as f32, HEIGHT as f32)?;
    let mount_ms = as_ms(started.elapsed());
    let (hover_ms, dispatch_ms, settle_ms) = measure(args, |x, y| {
        let started = Instant::now();
        host.dispatch_pointer(
            &mut engine,
            PointerInput::mouse(PointerEventKind::Move, x, y),
        )?;
        let dispatch = as_ms(started.elapsed());
        let started = Instant::now();
        host.flush_scene_frame(WIDTH as f32, HEIGHT as f32)?;
        Ok(Sample {
            dispatch: Some(dispatch),
            settle: Some(as_ms(started.elapsed())),
        })
    })?;
    Ok(Case {
        tier: "vue",
        mode,
        mount_ms,
        hover_ms,
        dispatch_ms,
        settle_ms,
    })
}

fn runtime_case(args: &Args) -> Result<Case, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let document = hover_document(args.rows);
    let mut session = RuntimeAgentSession::new(document, WIDTH, HEIGHT)?;
    let mount_ms = as_ms(started.elapsed());
    // Dispatch plus `flush`: one pointer move, one settle.
    let (hover_ms, _, _) = measure(args, |x, y| {
        session.hover_xy(x, y)?;
        Ok(Sample::default())
    })?;
    Ok(Case {
        tier: "runtime-l3",
        // L3 has no equivalent split: hover work is hit-test plus hover-state
        // bookkeeping either way, and no handler is invoked unless the hovered
        // row changed. One row, named for what it is.
        mode: "no-handler",
        mount_ms,
        hover_ms,
        dispatch_ms: None,
        settle_ms: None,
    })
}

/// The same shape `src/HoverBench.js` renders: a flat column of fixed-height
/// rows, no scrollport, nothing that reflows when the pointer moves.
fn hover_document(rows: usize) -> RuntimeDocument {
    let id = DocumentId::new(1).expect("document id");
    let mut document = RuntimeDocument::new(id);
    let mut row_style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut row_style.layout);
        layout.width = Some(LengthSpec::Px(WIDTH as f32));
        layout.height = Some(LengthSpec::Px(ROW_HEIGHT));
    }
    document
        .context_mut()
        .build(id, |ui| {
            let column = ui.child("column", Stack::column(0.0));
            ui.nest(column, |ui| {
                for row in 0..rows {
                    ui.child(
                        format!("row-{row}"),
                        Text::new(format!("Row {row}")).style(row_style.clone()),
                    );
                }
            });
        })
        .expect("hover document");
    document
}

fn parse(argv: impl IntoIterator<Item = String>) -> Result<Option<Args>, String> {
    let argv: Vec<String> = argv.into_iter().collect();
    let mut args = Args::default();
    let mut index = 0;
    while index < argv.len() {
        let (name, inline) = match argv[index].split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (argv[index].as_str(), None),
        };
        let mut value = || -> Result<String, String> {
            if let Some(inline) = inline.clone() {
                return Ok(inline);
            }
            index += 1;
            argv.get(index)
                .cloned()
                .ok_or_else(|| format!("{name} expects a value"))
        };
        match name {
            "--rows" => args.rows = parse_field(value()?, "--rows")?,
            "--moves" => args.moves = parse_field(value()?, "--moves")?,
            "--warmup" => args.warmup = parse_field(value()?, "--warmup")?,
            "--bundles" => args.bundles = PathBuf::from(value()?),
            "--output" => args.output = Some(PathBuf::from(value()?)),
            "--help" | "-h" => {
                println!(
                    "nana-hover-benchmark — per-pointer-event cost, Vue vs Rust L3\n\n\
                     Options:\n\
                     \x20 --rows <n>       rows per tree (default 2000); the Vue bundles must match\n\
                     \x20 --moves <n>      timed hover moves (default 600)\n\
                     \x20 --warmup <n>     untimed moves first (default 60)\n\
                     \x20 --bundles <dir>  where build-hover-bench.mjs wrote its output\n\
                     \x20 --output <path>  write JSON here instead of stdout"
                );
                return Ok(None);
            }
            other => return Err(format!("unknown flag {other}")),
        }
        index += 1;
    }
    if args.rows == 0 || args.moves == 0 {
        return Err("--rows and --moves must be positive".into());
    }
    Ok(Some(args))
}

fn parse_field<T: std::str::FromStr>(value: String, name: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{name} has an unparseable value"))
}
