// Charts (`nana-ui-charts`): lines, areas and shapes drawn from the arrays a
// chart layout built, every edge an analytic one-device-pixel ramp.
//
// Geometry is in the chart node's local px. Each element blends `from` →
// `to` by the transition's eased progress on the motion clock, so motion
// costs no CPU work per frame. A vertex index carries its draw (`/ 6`) and
// corner (`% 6`); the instance index is the segment or shape.
//
// The storage layouts match `GpuPoint`, `GpuShape`, `GpuStyle` in
// `nana-ui-charts/src/marks.rs` and `GpuDraw` / `ChartUniform` in
// `scene_paint/chart.rs`.

struct ChartGlobals {
    transform: mat4x4<f32>,
    viewport_scale: f32,
    corner_exponent: f32,
    _pad0: f32,
    _pad1: f32,
}

struct ChartUniform {
    affine_abcd: vec4<f32>,
    // `xy` scene affine translation, `zw` the node's origin in layout space.
    affine_ef_origin: vec4<f32>,
    plot: vec4<f32>,
    clip_rect: vec4<f32>,
    clip_inv_abcd: vec4<f32>,
    clip_inv_ef_radius: vec4<f32>,
    clip_poly0: vec4<f32>,
    clip_poly1: vec4<f32>,
    clip_poly2: vec4<f32>,
    clip_poly3: vec4<f32>,
    // start subsec, duration (s), hover start subsec, hover duration (s).
    timing: vec4<f32>,
    // start secs, easing kind, flags (1 reveal, 2 animating), hover start secs.
    timing_u: vec4<u32>,
    bezier: vec4<f32>,
    // Hovered (series, index), previously hovered (series, index).
    hover: vec4<u32>,
    // Focused series, previously focused series.
    focus: vec4<u32>,
    // opacity, device px per local px, emphasis growth px, _.
    params: vec4<f32>,
}

struct GpuPoint {
    goal: vec4<f32>,
    origin: vec4<f32>,
    dist: vec4<f32>,
}

struct GpuShape {
    goal: vec4<f32>,
    origin: vec4<f32>,
    extra: vec4<f32>,
    info: vec4<u32>,
}

struct GpuStyle {
    fill: vec4<f32>,
    fill_end: vec4<f32>,
    stroke: vec4<f32>,
    params: vec4<f32>,
}

// `a`: first, count, style, flags. `b`: neighbours, series, pass, _.
struct GpuDraw {
    a: vec4<u32>,
    b: vec4<u32>,
}

struct MotionTime {
    now_secs: u32,
    now_subsec: f32,
    eval_motion_id: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> globals: ChartGlobals;
@group(0) @binding(1) var<storage, read> points: array<GpuPoint>;
@group(0) @binding(2) var<storage, read> shapes: array<GpuShape>;
@group(0) @binding(3) var<storage, read> styles: array<GpuStyle>;
@group(0) @binding(4) var<uniform> chart: ChartUniform;
@group(0) @binding(5) var<storage, read> draws: array<GpuDraw>;
@group(1) @binding(2) var<uniform> chart_motion_time: MotionTime;

const DRAW_CLIP_TO_PLOT: u32 = 1u;
const DRAW_REVEAL: u32 = 2u;
const DRAW_CLOSED: u32 = 4u;
const DRAW_SOFT_BASE: u32 = 8u;
const DRAW_EMPHASIS: u32 = 16u;

const SHAPE_RECT: u32 = 0u;
const SHAPE_SECTOR: u32 = 1u;
const SHAPE_SYMBOL: u32 = 2u;
const SHAPE_CLIP_TO_PLOT: u32 = 256u;
const SHAPE_EMPHASIS_GROW: u32 = 512u;
const SHAPE_REVEAL: u32 = 1024u;

const NO_SERIES: u32 = 0xffffffffu;
const ANY_SERIES: u32 = 0xfffffffeu;
const TAU: f32 = 6.283185307179586;
const PI: f32 = 3.141592653589793;

// ---- clock -------------------------------------------------------------

fn chart_elapsed(start_secs: u32, start_subsec: f32) -> f32 {
    let now_secs = chart_motion_time.now_secs;
    let now_sub = chart_motion_time.now_subsec;
    if now_secs < start_secs || (now_secs == start_secs && now_sub < start_subsec) {
        return 0.0;
    }
    return f32(now_secs - start_secs) + (now_sub - start_subsec);
}

fn chart_bezier_axis(t: f32, p1: f32, p2: f32) -> f32 {
    let mt = 1.0 - t;
    return 3.0 * mt * mt * t * p1 + 3.0 * mt * t * t * p2 + t * t * t;
}

// `nana_ui_core::Easing`: 0 linear, 1 ease-out cubic, 2 ease-in-out cubic,
// 3 cubic bezier.
fn chart_ease(kind: u32, p: f32) -> f32 {
    let x = clamp(p, 0.0, 1.0);
    if kind == 1u {
        let t = 1.0 - x;
        return 1.0 - t * t * t;
    }
    if kind == 2u {
        if x < 0.5 {
            return 4.0 * x * x * x;
        }
        let t = -2.0 * x + 2.0;
        return 1.0 - t * t * t / 2.0;
    }
    if kind == 3u {
        if x <= 0.0 || x >= 1.0 {
            return x;
        }
        var low = 0.0;
        var high = 1.0;
        for (var i = 0; i < 20; i = i + 1) {
            let mid = 0.5 * (low + high);
            if chart_bezier_axis(mid, chart.bezier.x, chart.bezier.z) < x {
                low = mid;
            } else {
                high = mid;
            }
        }
        return chart_bezier_axis(0.5 * (low + high), chart.bezier.y, chart.bezier.w);
    }
    return x;
}

// Eased progress of the chart's transition, 1 at rest.
fn chart_progress() -> f32 {
    if (chart.timing_u.z & 2u) == 0u || chart.timing.y <= 0.0 {
        return 1.0;
    }
    let elapsed = chart_elapsed(chart.timing_u.x, chart.timing.x);
    return chart_ease(chart.timing_u.y, elapsed / chart.timing.y);
}

fn chart_reveal_active() -> bool {
    return (chart.timing_u.z & 1u) != 0u && chart_progress() < 1.0;
}

// How emphasised (0..1) an element of `series` / `index` is now, easing in
// for the hovered one and out for the one hovered before.
fn chart_emphasis(series: u32, index: u32) -> f32 {
    let t = chart_ease(1u, chart_elapsed(chart.timing_u.w, chart.timing.z) / max(chart.timing.w, 1e-4));
    let now = select(0.0, 1.0, (chart.hover.x == series || chart.hover.x == ANY_SERIES) && chart.hover.y == index && chart.hover.x != NO_SERIES);
    let before = select(0.0, 1.0, (chart.hover.z == series || chart.hover.z == ANY_SERIES) && chart.hover.w == index && chart.hover.z != NO_SERIES);
    return mix(before, now, t);
}

// Whether a series is the hovered one (lines widen).
fn chart_series_emphasis(series: u32) -> f32 {
    let t = chart_ease(1u, chart_elapsed(chart.timing_u.w, chart.timing.z) / max(chart.timing.w, 1e-4));
    let now = select(0.0, 1.0, chart.hover.x == series);
    let before = select(0.0, 1.0, chart.hover.z == series);
    return mix(before, now, t);
}

// Opacity of a series while another is focused.
fn chart_focus_alpha(series: u32) -> f32 {
    if series == NO_SERIES {
        return 1.0;
    }
    let t = chart_ease(1u, chart_elapsed(chart.timing_u.w, chart.timing.z) / max(chart.timing.w, 1e-4));
    let now = select(1.0, 0.2, chart.focus.x != NO_SERIES && chart.focus.x != series);
    let before = select(1.0, 0.2, chart.focus.y != NO_SERIES && chart.focus.y != series);
    return mix(before, now, t);
}

// ---- spaces ------------------------------------------------------------

fn chart_world(local: vec2<f32>) -> vec2<f32> {
    let p = local + chart.affine_ef_origin.zw;
    let m = chart.affine_abcd;
    return vec2<f32>(m.x * p.x + m.z * p.y, m.y * p.x + m.w * p.y) + chart.affine_ef_origin.xy;
}

fn chart_clip_position(local: vec2<f32>) -> vec4<f32> {
    return globals.transform * vec4<f32>(chart_world(local), 0.0, 1.0);
}

// Local px covering one device pixel.
fn chart_fringe() -> f32 {
    return 1.0 / max(chart.params.y, 1e-4);
}

fn chart_cover(signed_distance_local: f32) -> f32 {
    return clamp(0.5 - signed_distance_local * chart.params.y, 0.0, 1.0);
}

fn chart_unit_corner(corner: u32) -> vec2<f32> {
    let id = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u)[corner];
    return vec2<f32>(
        select(-1.0, 1.0, (id & 2u) != 0u),
        select(-1.0, 1.0, id == 1u || id == 2u),
    );
}

// Scene clip, plot clip, reveal and opacity: what every fragment shares.
fn chart_common_coverage(world: vec2<f32>, local: vec2<f32>, clip_plot: bool, reveal: bool, margin: f32) -> f32 {
    var cover = fragment_clip_coverage(
        world,
        chart.clip_rect,
        chart.clip_inv_abcd,
        chart.clip_inv_ef_radius.xy,
        chart.clip_inv_ef_radius.z,
        u32(chart.clip_inv_ef_radius.w),
        chart.clip_poly0,
        chart.clip_poly1,
        chart.clip_poly2,
        chart.clip_poly3,
        globals.viewport_scale,
        globals.corner_exponent,
    );
    if clip_plot {
        let plot = chart.plot;
        let inside = min(
            min(local.x - (plot.x - margin), (plot.z + margin) - local.x),
            min(local.y - (plot.y - margin), (plot.w + margin) - local.y),
        );
        cover = cover * clamp(0.5 + inside * chart.params.y, 0.0, 1.0);
    }
    if reveal && chart_reveal_active() {
        let edge = chart.plot.x + (chart.plot.z - chart.plot.x) * chart_progress();
        cover = cover * clamp(0.5 + (edge - local.x) * chart.params.y, 0.0, 1.0);
    }
    return cover * chart.params.x;
}

fn chart_point(index: u32, t: f32) -> vec4<f32> {
    let p = points[index];
    return mix(p.origin, p.goal, t);
}

fn chart_dist(index: u32, t: f32) -> f32 {
    let p = points[index];
    return mix(p.dist.y, p.dist.x, t);
}

// Distance from `p` to segment `a`→`b`, and the arc position of its foot
// (0..1).
fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let ab = b - a;
    let len2 = dot(ab, ab);
    let h = select(clamp(dot(p - a, ab) / len2, 0.0, 1.0), 0.0, len2 < 1e-12);
    return vec2<f32>(length(p - a - ab * h), h);
}

// ---- lines -------------------------------------------------------------

struct LineOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) world: vec2<f32>,
    @location(2) @interpolate(flat) seg: vec4<f32>,
    // half width, dist at the segment's start, progress, _
    @location(3) @interpolate(flat) line: vec4<f32>,
    // draw, segment, _, _
    @location(4) @interpolate(flat) ids: vec4<u32>,
}

fn line_segment_end(draw: GpuDraw, segment: u32) -> u32 {
    let count = draw.a.y;
    let next = segment + 1u;
    return draw.a.x + select(next, next % count, (draw.a.w & DRAW_CLOSED) != 0u);
}

fn line_half_width(draw: GpuDraw) -> f32 {
    let style = styles[draw.a.z];
    var width = style.params.x;
    if (draw.a.w & DRAW_EMPHASIS) != 0u {
        width = width + chart_series_emphasis(draw.b.y);
    }
    return width * 0.5;
}

@vertex
fn chart_line_vs(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) segment: u32) -> LineOut {
    var out: LineOut;
    let draw_id = vertex_index / 6u;
    let draw = draws[draw_id];
    let t = chart_progress();
    let a = chart_point(draw.a.x + segment, t).xy;
    let b = chart_point(line_segment_end(draw, segment), t).xy;
    let half = line_half_width(draw);
    let pad = half + chart_fringe() * 1.5;
    var dir = b - a;
    let len = length(dir);
    dir = select(vec2<f32>(1.0, 0.0), dir / len, len > 1e-6);
    let normal = vec2<f32>(-dir.y, dir.x);
    let corner = chart_unit_corner(vertex_index % 6u);
    let along = select(-pad, len + pad, corner.x > 0.0);
    let local = a + dir * along + normal * (corner.y * pad);
    out.position = chart_clip_position(local);
    out.local = local;
    out.world = chart_world(local);
    out.seg = vec4<f32>(a, b);
    out.line = vec4<f32>(half, chart_dist(draw.a.x + segment, t), t, 0.0);
    out.ids = vec4<u32>(draw_id, segment, 0u, 0u);
    return out;
}

@fragment
fn chart_line_fs(input: LineOut) -> @location(0) vec4<f32> {
    let draw = draws[input.ids.x];
    let segment = input.ids.y;
    let t = input.line.z;
    let own = segment_distance(input.local, input.seg.xy, input.seg.zw);
    // A translucent line is painted once where its segments overlap: the
    // nearest segment owns the fragment. Two draws interpolate the same
    // pixel's position a hair apart, so near-ties (a join's outer cap, where
    // both measure to the shared point) go to the lower index on both sides
    // and exactly one of them draws.
    let closed = (draw.a.w & DRAW_CLOSED) != 0u;
    let count = draw.a.y;
    let segments = select(count - 1u, count, closed);
    let k = draw.b.x;
    for (var step = 1u; step <= k; step = step + 1u) {
        for (var side = 0u; side < 2u; side = side + 1u) {
            var other: i32 = i32(segment) + select(-i32(step), i32(step), side == 1u);
            if closed {
                other = ((other % i32(segments)) + i32(segments)) % i32(segments);
                if u32(other) == segment {
                    continue;
                }
            } else if other < 0 || other >= i32(segments) {
                continue;
            }
            let oa = chart_point(draw.a.x + u32(other), t).xy;
            let ob = chart_point(line_segment_end(draw, u32(other)), t).xy;
            let d = segment_distance(input.local, oa, ob).x;
            let tie = 1e-3;
            if d < own.x - tie || (d <= own.x + tie && u32(other) < segment) {
                discard;
            }
        }
    }
    let half = input.line.x;
    var cover = chart_cover(own.x - half);
    let style = styles[draw.a.z];
    // Dashes run along the line's arc length.
    let dash_on = style.params.y;
    let dash_off = style.params.z;
    if dash_on > 0.0 && dash_off > 0.0 {
        let seg_len = length(input.seg.zw - input.seg.xy);
        let at = input.line.y + own.y * seg_len;
        let phase = at - floor(at / (dash_on + dash_off)) * (dash_on + dash_off);
        let into = min(phase, dash_on - phase);
        cover = cover * clamp(0.5 + into * chart.params.y, 0.0, 1.0);
    }
    let flags = draw.a.w;
    cover = cover * chart_common_coverage(
        input.world,
        input.local,
        (flags & DRAW_CLIP_TO_PLOT) != 0u,
        (flags & DRAW_REVEAL) != 0u,
        half + 1.0,
    ) * chart_focus_alpha(draw.b.y);
    if cover <= 0.0 {
        discard;
    }
    return premultiply(style.stroke) * cover;
}

// ---- areas -------------------------------------------------------------

struct AreaOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) world: vec2<f32>,
    @location(2) @interpolate(flat) top: vec4<f32>,
    @location(3) @interpolate(flat) base: vec4<f32>,
    @location(4) @interpolate(flat) ids: vec4<u32>,
}

// Unit direction from a vertex's base to its curve point: the side edge
// the area's cells share. Falls back to the outward normal of the segment
// when the two meet.
fn area_side(p: vec2<f32>, b: vec2<f32>, fallback: vec2<f32>) -> vec2<f32> {
    let d = p - b;
    let len = length(d);
    return select(fallback, d / len, len > 1e-4);
}

// How far along `side` a cell must reach past its edge so a one-pixel band
// across a segment of direction `dir` is inside it.
fn area_reach(side: vec2<f32>, dir: vec2<f32>) -> f32 {
    let s = abs(side.x * dir.y - side.y * dir.x);
    return min(chart_fringe() * 1.5 / max(s, 1e-3), chart_fringe() * 48.0);
}

@vertex
fn chart_area_vs(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) segment: u32) -> AreaOut {
    var out: AreaOut;
    let draw_id = vertex_index / 6u;
    let draw = draws[draw_id];
    let t = chart_progress();
    let first = draw.a.x;
    let count = draw.a.y;
    let closed = (draw.a.w & DRAW_CLOSED) != 0u;
    let i0 = first + segment;
    let i1 = line_segment_end(draw, segment);
    let p0 = chart_point(i0, t);
    let p1 = chart_point(i1, t);
    // Neighbouring points, for a reach both cells sharing a side agree on.
    var prev = p0;
    var next = p1;
    if closed || segment > 0u {
        prev = chart_point(first + (segment + count - 1u) % count, t);
    }
    if closed || segment + 2u < count {
        next = chart_point(first + (segment + 2u) % count, t);
    }
    let dir = normalize(p1.xy - p0.xy + vec2<f32>(1e-9, 0.0));
    var outward = vec2<f32>(-dir.y, dir.x);
    let mid_base = (p0.zw + p1.zw) * 0.5;
    if dot(outward, (p0.xy + p1.xy) * 0.5 - mid_base) < 0.0 {
        outward = -outward;
    }
    let side0 = area_side(p0.xy, p0.zw, outward);
    let side1 = area_side(p1.xy, p1.zw, outward);
    let dir_prev = normalize(p0.xy - prev.xy + vec2<f32>(1e-9, 0.0));
    let dir_next = normalize(next.xy - p1.xy + vec2<f32>(1e-9, 0.0));
    let reach0 = max(area_reach(side0, dir), select(0.0, area_reach(side0, dir_prev), any(prev.xy != p0.xy)));
    let reach1 = max(area_reach(side1, dir), select(0.0, area_reach(side1, dir_next), any(next.xy != p1.xy)));
    let soft = (draw.a.w & DRAW_SOFT_BASE) != 0u;
    let base_reach0 = select(0.0, reach0, soft);
    let base_reach1 = select(0.0, reach1, soft);
    var local: vec2<f32>;
    switch vertex_index % 6u {
        case 0u, 3u: { local = p0.xy + side0 * reach0; }
        case 1u: { local = p1.xy + side1 * reach1; }
        case 2u, 4u: { local = p1.zw - side1 * base_reach1; }
        default: { local = p0.zw - side0 * base_reach0; }
    }
    out.position = chart_clip_position(local);
    out.local = local;
    out.world = chart_world(local);
    out.top = vec4<f32>(p0.xy, p1.xy);
    out.base = vec4<f32>(p0.zw, p1.zw);
    out.ids = vec4<u32>(draw_id, 0u, 0u, 0u);
    return out;
}

// Signed distance to a cell's edge: negative on `inner`'s side.
fn edge_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, inner: vec2<f32>) -> f32 {
    let d = segment_distance(p, a, b).x;
    let ab = b - a;
    let n = vec2<f32>(-ab.y, ab.x);
    let side_inner = dot(inner - a, n);
    let side_p = dot(p - a, n);
    return select(d, -d, side_p * side_inner > 0.0);
}

@fragment
fn chart_area_fs(input: AreaOut) -> @location(0) vec4<f32> {
    let draw = draws[input.ids.x];
    let style = styles[draw.a.z];
    let base_mid = (input.base.xy + input.base.zw) * 0.5;
    let top_mid = (input.top.xy + input.top.zw) * 0.5;
    var cover = chart_cover(edge_distance(input.local, input.top.xy, input.top.zw, base_mid));
    let flags = draw.a.w;
    if (flags & DRAW_SOFT_BASE) != 0u && distance(input.base.xy, input.base.zw) > 1e-4 {
        cover = cover * chart_cover(edge_distance(input.local, input.base.xy, input.base.zw, top_mid));
    }
    cover = cover * chart_common_coverage(
        input.world,
        input.local,
        (flags & DRAW_CLIP_TO_PLOT) != 0u,
        (flags & DRAW_REVEAL) != 0u,
        0.0,
    ) * chart_focus_alpha(draw.b.y);
    if cover <= 0.0 {
        discard;
    }
    // A gradient from `params.x` to `params.y` along y (x when `params.z`).
    var color = style.fill;
    let span = style.params.y - style.params.x;
    if abs(span) > 1e-3 {
        let coord = select(input.local.y, input.local.x, style.params.z > 0.5);
        color = mix(style.fill, style.fill_end, clamp((coord - style.params.x) / span, 0.0, 1.0));
    }
    return premultiply(color) * cover;
}

// ---- shapes ------------------------------------------------------------

struct ShapeOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) world: vec2<f32>,
    @location(2) @interpolate(flat) geometry: vec4<f32>,
    @location(3) @interpolate(flat) extra: vec4<f32>,
    // kind | flags, style, series, emphasis (as bits)
    @location(4) @interpolate(flat) info: vec4<u32>,
}

fn rotate(v: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}

// The bounding box of an annular sector.
fn sector_bounds(center: vec2<f32>, a0: f32, a1: f32, inner: f32, outer: f32) -> vec4<f32> {
    if a1 - a0 >= TAU - 1e-4 {
        return vec4<f32>(center - vec2<f32>(outer), center + vec2<f32>(outer));
    }
    var lo = min(center + vec2<f32>(cos(a0), sin(a0)) * inner, center + vec2<f32>(cos(a1), sin(a1)) * inner);
    var hi = max(center + vec2<f32>(cos(a0), sin(a0)) * inner, center + vec2<f32>(cos(a1), sin(a1)) * inner);
    let o0 = center + vec2<f32>(cos(a0), sin(a0)) * outer;
    let o1 = center + vec2<f32>(cos(a1), sin(a1)) * outer;
    lo = min(lo, min(o0, o1));
    hi = max(hi, max(o0, o1));
    // The arc's extremes at the quarter turns it crosses.
    for (var q = 0; q < 4; q = q + 1) {
        let angle = f32(q) * PI * 0.5;
        let k = ceil((a0 - angle) / TAU);
        let candidate = angle + k * TAU;
        if candidate <= a1 {
            let p = center + vec2<f32>(cos(angle), sin(angle)) * outer;
            lo = min(lo, p);
            hi = max(hi, p);
        }
    }
    return vec4<f32>(lo, hi);
}

@vertex
fn chart_shape_vs(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) index: u32) -> ShapeOut {
    var out: ShapeOut;
    let shape = shapes[index];
    let t = chart_progress();
    var g = mix(shape.origin, shape.goal, t);
    let kind = shape.info.x & 255u;
    let flags = shape.info.x;
    var emphasis = 0.0;
    if (flags & SHAPE_EMPHASIS_GROW) != 0u {
        emphasis = chart_emphasis(shape.info.z, shape.info.w);
    }
    let fringe = chart_fringe() * 1.5;
    let corner = chart_unit_corner(vertex_index % 6u);
    var local: vec2<f32>;
    if kind == SHAPE_RECT {
        let lo = min(g.xy, g.zw);
        let hi = max(g.xy, g.zw);
        g = vec4<f32>(lo, hi);
        let center = (lo + hi) * 0.5;
        let half = (hi - lo) * 0.5 + vec2<f32>(fringe);
        local = center + half * corner;
    } else if kind == SHAPE_SECTOR {
        g.w = g.w + chart.params.z * emphasis;
        let center = shape.extra.xy;
        let lo = min(g.x, g.y);
        let hi = max(g.x, g.y);
        g = vec4<f32>(lo, hi, g.z, g.w);
        let bounds = sector_bounds(center, lo, hi, g.z, g.w);
        let box_center = (bounds.xy + bounds.zw) * 0.5;
        let half = (bounds.zw - bounds.xy) * 0.5 + vec2<f32>(fringe);
        local = box_center + half * corner;
    } else if kind == SHAPE_SYMBOL {
        g.z = mix(g.z, max(g.z, shape.extra.z), emphasis);
        let reach = g.z * 0.6 + shape.extra.x + fringe;
        local = g.xy + vec2<f32>(reach) * corner;
    } else {
        // Needle: along `angle` from `offset` to `offset + length`.
        let angle = g.z;
        let width = shape.extra.x;
        let start = shape.extra.y;
        let half_len = g.w * 0.5 + fringe;
        let mid = start + g.w * 0.5;
        let v = vec2<f32>(mid + corner.x * half_len, corner.y * (width * 0.5 + fringe));
        local = g.xy + rotate(v, angle);
    }
    out.position = chart_clip_position(local);
    out.local = local;
    out.world = chart_world(local);
    out.geometry = g;
    out.extra = shape.extra;
    out.info = vec4<u32>(flags, shape.info.y, shape.info.z, bitcast<u32>(emphasis));
    return out;
}

fn sd_box(p: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let r = min(radius, min(half.x, half.y));
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// Per-corner radii in `[tl, tr, br, bl]` order, y down.
fn sd_box4(p: vec2<f32>, half: vec2<f32>, radii: vec4<f32>) -> f32 {
    let pair = select(radii.xy, radii.wz, p.y > 0.0);
    let radius = select(pair.x, pair.y, p.x > 0.0);
    return sd_box(p, half, radius);
}

// Signed distance (negative inside) to an annular sector `a0..a1`,
// `inner..outer`, with its radial edges moved in by `pad / 2` and its
// corners rounded by `corner`.
fn sd_sector(p: vec2<f32>, a0: f32, a1: f32, inner: f32, outer: f32, corner: f32, pad: f32) -> f32 {
    let r = length(p);
    let mid_r = (inner + outer) * 0.5;
    let half_w = (outer - inner) * 0.5;
    let cr = clamp(corner, 0.0, half_w);
    let ring = abs(r - mid_r) - half_w;
    let span = a1 - a0;
    var wedge = -1e9;
    if span < TAU - 1e-4 {
        let half = span * 0.5;
        let q0 = rotate(p, -(a0 + half));
        let q = vec2<f32>(q0.x, abs(q0.y));
        // Distance to the edge ray at `half`, negative inside the wedge.
        let edge = vec2<f32>(cos(half), sin(half));
        let normal = vec2<f32>(-sin(half), cos(half));
        let along = dot(q, edge);
        // The upper edge is the nearer one for `q.y >= 0`; inside is
        // clockwise of it, for any span up to a full turn.
        let d = select(length(q), abs(dot(q, normal)), along > 0.0);
        let inside = edge.x * q.y - edge.y * q.x < 0.0;
        wedge = select(d, -d, inside) + pad * 0.5;
    }
    let a = ring + cr;
    let b = wedge + cr;
    return length(max(vec2<f32>(a, b), vec2<f32>(0.0))) + min(max(a, b), 0.0) - cr;
}

// Equilateral triangle of half side `r`, pointing up on screen, centred on
// its centroid (after Inigo Quilez).
fn sd_triangle(point: vec2<f32>, r: f32) -> f32 {
    let k = sqrt(3.0);
    var p = vec2<f32>(abs(point.x) - r, -point.y + r / k);
    if p.x + k * p.y > 0.0 {
        p = vec2<f32>(p.x - k * p.y, -k * p.x - p.y) * 0.5;
    }
    p.x = p.x - clamp(p.x, -2.0 * r, 0.0);
    return -length(p) * sign(p.y);
}

fn sd_symbol(p: vec2<f32>, shape: u32, size: f32) -> f32 {
    let r = size * 0.5;
    if shape == 1u {
        return sd_box(p, vec2<f32>(r * 0.9), 0.0);
    }
    if shape == 2u {
        return sd_box(p, vec2<f32>(r * 0.9), r * 0.35);
    }
    if shape == 3u {
        return sd_triangle(p - vec2<f32>(0.0, r * 0.1), r * 0.95);
    }
    if shape == 4u {
        // Rhombus.
        let q = abs(p);
        let b = vec2<f32>(r * 0.85, r * 1.1);
        let h = clamp((b.x * (b.x - 2.0 * q.x) - b.y * (b.y - 2.0 * q.y)) / dot(b, b), -1.0, 1.0);
        let d = length(q - 0.5 * b * vec2<f32>(1.0 - h, 1.0 + h));
        return d * sign(q.x * b.y + q.y * b.x - b.x * b.y);
    }
    if shape == 5u {
        // A head on a point.
        let head = length(p + vec2<f32>(0.0, r * 0.25)) - r * 0.7;
        let tip = sd_triangle(vec2<f32>(p.x, -(p.y - r * 0.35)), r * 0.6);
        return min(head, tip);
    }
    return length(p) - r;
}

@fragment
fn chart_shape_fs(input: ShapeOut) -> @location(0) vec4<f32> {
    let flags = input.info.x;
    let kind = flags & 255u;
    let style = styles[input.info.y];
    let emphasis = bitcast<f32>(input.info.w);
    let g = input.geometry;
    var d = 0.0;
    var border = 0.0;
    if kind == SHAPE_RECT {
        let center = (g.xy + g.zw) * 0.5;
        let half = (g.zw - g.xy) * 0.5;
        if half.x <= 0.0 || half.y <= 0.0 {
            discard;
        }
        d = sd_box4(input.local - center, half, input.extra);
    } else if kind == SHAPE_SECTOR {
        if g.y - g.x <= 0.0 || g.w <= g.z {
            discard;
        }
        d = sd_sector(input.local - input.extra.xy, g.x, g.y, g.z, g.w, input.extra.z, input.extra.w);
    } else if kind == SHAPE_SYMBOL {
        if g.z <= 0.0 {
            discard;
        }
        d = sd_symbol(rotate(input.local - g.xy, -g.w), u32(input.extra.y), g.z);
        border = input.extra.x;
    } else {
        let q = rotate(input.local - g.xy, -g.z);
        let start = input.extra.y;
        d = sd_box(q - vec2<f32>(start + g.w * 0.5, 0.0), vec2<f32>(g.w * 0.5, input.extra.x * 0.5), input.extra.z);
    }
    let outer = chart_cover(d);
    var color: vec4<f32>;
    if border > 0.0 {
        let inner = chart_cover(d + border);
        color = premultiply(style.stroke) * (outer - inner) + premultiply(style.fill) * inner;
    } else {
        var fill = style.fill;
        if kind == SHAPE_RECT && emphasis > 0.0 {
            // A hovered bar lifts towards white.
            fill = vec4<f32>(mix(fill.rgb, vec3<f32>(1.0), 0.12 * emphasis), fill.a);
        }
        color = premultiply(fill) * outer;
    }
    // A symbol on the plot's edge stays whole; anything else is cut there.
    let margin = select(0.0, g.z * 0.5 + border, kind == SHAPE_SYMBOL);
    let cover = chart_common_coverage(
        input.world,
        input.local,
        (flags & SHAPE_CLIP_TO_PLOT) != 0u,
        (flags & SHAPE_REVEAL) != 0u,
        margin,
    ) * chart_focus_alpha(input.info.z);
    if cover <= 0.0 || color.a <= 0.0 {
        discard;
    }
    return color * cover;
}
