//! Vector-path content for the `gpu-scene-paths-*` rows: a custom-paint canvas
//! and a Markdown document with Mermaid diagrams, plus what their animations
//! change each frame. Everything here goes through the product path — a
//! `Painter` recording and a `NativeMarkdown` fence — into the scene's
//! flatten / `i_overlay` / `lyon` route and the painter's path buffers.

use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::{ElevationRole, PaintTransform, TransformOrigin};
use nana_ui_runtime::{
    FillRule, LayoutBox, LayoutStyle, LengthSpec, LineCap, LineJoin, List, NodePainter, NodeStyle,
    PaintContext, PaintPath, PaintShadow, Painter, SemanticColorRole, StrokeStyle,
};
use nana_ui_scene::ScenePrimitiveKind;
use serde::{Deserialize, Serialize};

/// What changes each frame in a path scenario.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum PathAnimation {
    /// The list holding the content moves up by 1.7 px a frame, wrapping
    /// every 240 px: a scroll as the painter sees it. Nothing is recorded or
    /// tessellated again; every vertex lands somewhere new.
    Scroll,
    /// The list scales between 0.9 and 1.1 about its origin: a compositor
    /// zoom. The recordings stay; only the transform changes.
    Scale,
    /// The content is recorded again every frame: the canvas draws at a new
    /// zoom (its `paint_key` changes), the Markdown fence's source changes by
    /// a Mermaid comment (a diagram edited live). Everything is flattened,
    /// overlaid and tessellated again: the per-recording build cost.
    Rerecord,
}

impl PathAnimation {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "scroll" => Some(Self::Scroll),
            "scale" => Some(Self::Scale),
            "rerecord" => Some(Self::Rerecord),
            _ => None,
        }
    }
}

/// The root list's transform for `animation` at `frame`, if it moves the list.
pub fn root_transform(animation: PathAnimation, frame: usize) -> Option<PaintTransform> {
    match animation {
        PathAnimation::Scroll => Some(PaintTransform {
            f: -(frame as f32 * 1.7 % 240.0),
            ..PaintTransform::default()
        }),
        PathAnimation::Scale => {
            let s = 1.0 + 0.1 * (frame as f32 * 0.21).sin();
            Some(PaintTransform {
                a: s,
                d: s,
                ..PaintTransform::default()
            })
        }
        PathAnimation::Rerecord => None,
    }
}

pub fn with_transform(mut style: NodeStyle, transform: PaintTransform) -> NodeStyle {
    let layout = Arc::make_mut(&mut style.layout);
    layout.transform = Some(transform);
    layout.transform_origin = Some(TransformOrigin::default());
    style
}

pub const CANVAS_SIZE: [f32; 2] = [1200.0, 720.0];
const COLUMNS: usize = 12;
const ROWS: usize = 8;

/// A whiteboard-like canvas: 96 cells, each with a curved blob, a curve
/// stroke and a small marker; every third cell clips its contents to a
/// circle, every fourth dashes its stroke, every eighth casts a shadow under
/// a star, every fifth fills an even-odd ring: 332 filled or stroked paths
/// (one mesh each), 32 cells' worth of them under a path clip.
pub struct VectorCanvas {
    pub zoom: f32,
}

impl Painter for VectorCanvas {
    fn paint(&self, cx: &mut PaintContext<'_>) {
        cx.scale(self.zoom, self.zoom);
        let cell = [
            CANVAS_SIZE[0] / COLUMNS as f32,
            CANVAS_SIZE[1] / ROWS as f32,
        ];
        let palette = [
            SemanticColorRole::Accent,
            SemanticColorRole::Text,
            SemanticColorRole::Muted,
            SemanticColorRole::Border,
        ];
        for row in 0..ROWS {
            for column in 0..COLUMNS {
                let index = row * COLUMNS + column;
                let x = column as f32 * cell[0];
                let y = row as f32 * cell[1];
                let [w, h] = cell;
                let clipped = index.is_multiple_of(3);
                if clipped {
                    let mut clip = PaintPath::new();
                    clip.ellipse(LayoutBox {
                        x: x + 4.0,
                        y: y + 4.0,
                        width: w - 8.0,
                        height: h - 8.0,
                    });
                    cx.push_clip(&clip);
                }
                // A closed blob of four cubics, phase-shifted per cell.
                let phase = index as f32 * 0.37;
                let mut blob = PaintPath::new();
                let (cx0, cy0) = (x + w * 0.5, y + h * 0.5);
                let r = w.min(h) * 0.38;
                blob.move_to(cx0 + r, cy0);
                for k in 0..4 {
                    let a0 = k as f32 * std::f32::consts::FRAC_PI_2 + phase;
                    let a1 = a0 + std::f32::consts::FRAC_PI_2;
                    let bulge = r * (0.55 + 0.25 * (phase + k as f32).sin());
                    blob.cubic_to(
                        cx0 + r * a0.cos() - bulge * a0.sin(),
                        cy0 + r * a0.sin() + bulge * a0.cos(),
                        cx0 + r * a1.cos() + bulge * a1.sin(),
                        cy0 + r * a1.sin() - bulge * a1.cos(),
                        cx0 + r * a1.cos(),
                        cy0 + r * a1.sin(),
                    );
                }
                blob.close();
                cx.fill_path(&blob, palette[index % palette.len()]);
                // A wave across the cell.
                let mut wave = PaintPath::new();
                wave.move_to(x + 6.0, y + h * 0.75);
                wave.cubic_to(
                    x + w * 0.35,
                    y + h * (0.2 + 0.1 * phase.sin()),
                    x + w * 0.65,
                    y + h * 1.1,
                    x + w - 6.0,
                    y + h * 0.3,
                );
                let mut stroke = StrokeStyle::new(1.5 + (index % 4) as f32)
                    .cap(LineCap::Round)
                    .join(LineJoin::Round);
                if index % 4 == 1 {
                    stroke = stroke.dash(vec![6.0, 4.0], phase);
                }
                cx.stroke_path(&wave, stroke, SemanticColorRole::Text);
                // A small marker: a triangle outline with miter joins.
                let mut marker = PaintPath::new();
                marker.move_to(x + 10.0, y + 10.0);
                marker.line_to(x + 22.0, y + 12.0);
                marker.line_to(x + 14.0, y + 22.0);
                marker.close();
                cx.stroke_path(
                    &marker,
                    StrokeStyle::new(1.25).join(LineJoin::Miter),
                    SemanticColorRole::Accent,
                );
                if index.is_multiple_of(5) {
                    let mut ring = PaintPath::new().with_fill_rule(FillRule::EvenOdd);
                    ring.arc(x + w - 18.0, y + 18.0, 10.0, 0.0, std::f32::consts::TAU);
                    ring.close();
                    ring.arc(x + w - 18.0, y + 18.0, 5.0, 0.0, std::f32::consts::TAU);
                    ring.close();
                    cx.fill_path(&ring, SemanticColorRole::Muted);
                }
                if clipped {
                    cx.pop_clip();
                }
                if index.is_multiple_of(8) {
                    let star = star(x + w * 0.5, y + h * 0.5, r * 0.6, r * 0.28);
                    cx.shadow(&star, PaintShadow::Elevation(ElevationRole::Overlay));
                    cx.fill_path(&star, SemanticColorRole::Accent);
                }
            }
        }
    }

    fn paint_key(&self) -> u64 {
        u64::from(self.zoom.to_bits())
    }
}

fn star(cx0: f32, cy0: f32, outer: f32, inner: f32) -> PaintPath {
    let mut path = PaintPath::new();
    for k in 0..10 {
        let angle = k as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
        let r = if k % 2 == 0 { outer } else { inner };
        let (x, y) = (cx0 + r * angle.cos(), cy0 + r * angle.sin());
        if k == 0 {
            path.move_to(x, y);
        } else {
            path.line_to(x, y);
        }
    }
    path.close();
    path
}

/// The canvas zoom for `frame` under [`PathAnimation::Rerecord`]: never the
/// same twice in a row, so every frame is a new recording.
pub fn canvas_zoom(frame: usize) -> f32 {
    1.0 + 0.1 * (frame as f32 * 0.21).sin() + (frame % 2) as f32 * 1e-4
}

pub fn canvas(zoom: f32) -> List {
    let mut style = NodeStyle::default();
    *Arc::make_mut(&mut style.layout) = LayoutStyle {
        width: Some(LengthSpec::Px(CANVAS_SIZE[0])),
        height: Some(LengthSpec::Px(CANVAS_SIZE[1])),
        ..LayoutStyle::default()
    };
    style.painter = Some(NodePainter::new(VectorCanvas { zoom }));
    List::new().label("vector-canvas").style(style)
}

/// A mid-size flowchart (23 nodes, 29 edges, two subgraphs) and a sequence
/// diagram (5 participants, 16 messages, a loop and notes), as one Markdown
/// document would carry them. `edit` appends a Mermaid comment so the source
/// — and with it the parse, layout and recording — is new.
#[cfg(feature = "rich-text")]
pub fn mermaid_source(edit: Option<usize>) -> String {
    let comment = edit.map(|n| format!("%% edit {n}\n")).unwrap_or_default();
    format!(
        "## Release pipeline\n\n```mermaid\nflowchart LR\n{comment}\
subgraph Build\n  A([Commit]) --> B[Fetch deps]\n  B --> C{{Cache hit?}}\n  C -->|yes| D[Restore]\n  C -->|no| E[Compile]\n  E --> F[(Artifact store)]\n  D --> G[Link]\n  F --> G\nend\n\
subgraph Verify\n  G --> H[Unit tests]\n  G --> I[Lint]\n  G --> J[Visual tests]\n  H --> K{{All green?}}\n  I --> K\n  J --> K\nend\n\
K -->|no| L((Notify))\nL -.-> A\nK -->|yes| M[Package]\nM --> N[/Sign/]\nN --> O[Upload]\nO --> P{{Channel}}\n\
P -->|beta| Q[Canary 5%]\nP -->|stable| R[Staged 25%]\nQ --> S[Monitor]\nR --> S\nS -->|errors| T[Roll back]\nT -.-> L\n\
S -->|healthy| U[Promote]\nU --> V([Done])\nO --> W[(Mirror)]\nW -.-> S\n```\n\n\
Messages between the services:\n\n```mermaid\nsequenceDiagram\n{comment}\
participant C as Client\nparticipant G as Gateway\nparticipant A as Auth\nparticipant S as Store\nparticipant Q as Queue\n\
C->>G: POST /upload\nG->>A: verify token\nA-->>G: ok\nNote over G,A: cached for 60 s\n\
loop every chunk\n  G->>S: put chunk\n  S-->>G: etag\nend\n\
G->>Q: enqueue scan\nQ-->>G: job id\nG-->>C: 202 Accepted\nC->>G: GET /status\nG->>Q: poll job\n\
Q-->>G: running\nG-->>C: pending\nNote right of Q: scan takes ~2 s\nQ->>S: mark clean\nS-->>Q: done\nQ-)G: job finished\nG-)C: push: ready\n```\n"
    )
}

#[cfg(feature = "rich-text")]
pub fn mermaid(edit: Option<usize>) -> nana_ui_runtime::NativeMarkdown {
    let mut style = NodeStyle::default();
    Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(1200.0));
    nana_ui_runtime::NativeMarkdown::from_source(&mermaid_source(edit)).style(style)
}

/// Per frame: how many path meshes the scene holds, how many of them are not
/// the very mesh it held the frame before (re-tessellated), and their
/// vertices. Pointer identity is the reuse test the scene itself makes.
#[derive(Default)]
pub struct MeshReuse {
    previous: HashSet<usize>,
    current: HashSet<usize>,
    pub frames: usize,
    pub meshes: usize,
    pub rebuilt: usize,
    pub vertices: usize,
    pub indices: usize,
}

impl MeshReuse {
    pub fn observe(&mut self, scene: &nana_ui_scene::UiScene, sampled: bool) {
        self.current.clear();
        let mut rebuilt = 0;
        let mut vertices = 0;
        let mut indices = 0;
        for primitive in scene.primitives() {
            if let ScenePrimitiveKind::Path { mesh, .. } = &primitive.kind {
                let key = Arc::as_ptr(mesh) as usize;
                if self.current.insert(key) {
                    rebuilt += usize::from(!self.previous.contains(&key));
                    vertices += mesh.vertices.len();
                    indices += mesh.indices.len();
                }
            }
        }
        if sampled {
            self.frames += 1;
            self.meshes += self.current.len();
            self.rebuilt += rebuilt;
            self.vertices += vertices;
            self.indices += indices;
        }
        std::mem::swap(&mut self.previous, &mut self.current);
    }

    pub fn report(&self) -> Option<MeshReuseReport> {
        (self.frames > 0).then(|| {
            let per = |total: usize| total as f64 / self.frames as f64;
            MeshReuseReport {
                path_meshes: per(self.meshes),
                path_meshes_rebuilt: per(self.rebuilt),
                path_vertices: per(self.vertices),
                path_indices: per(self.indices),
            }
        })
    }
}

#[derive(Serialize)]
pub struct MeshReuseReport {
    pub path_meshes: f64,
    pub path_meshes_rebuilt: f64,
    pub path_vertices: f64,
    pub path_indices: f64,
}
