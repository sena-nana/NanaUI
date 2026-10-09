//! Curves through a series' points, built in px once per layout.

use crate::option::Step;

/// Which coordinate a series advances along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Along {
    X,
    Y,
}

impl Along {
    /// Index of the advancing coordinate in `[x, y]`.
    fn axis(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
        }
    }
}

/// How far (px) a flattened piece may stray from its chord.
const FLATNESS_PX: f64 = 0.1;
/// Bisection limit per interval; keeps degenerate input bounded.
const MAX_DEPTH: u32 = 24;

type Vec2 = [f64; 2];

fn add(a: Vec2, b: Vec2) -> Vec2 {
    [a[0] + b[0], a[1] + b[1]]
}

fn sub(a: Vec2, b: Vec2) -> Vec2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn mul(a: Vec2, k: f64) -> Vec2 {
    [a[0] * k, a[1] * k]
}

fn len(a: Vec2) -> f64 {
    a[0].hypot(a[1])
}

/// One interval of the curve: a cubic Hermite from `p0` to `p1` over
/// `t ∈ [0, 1]`, optionally clamped to a box (the monotone case, where the
/// clamp only removes rounding).
struct Hermite {
    p0: Vec2,
    p1: Vec2,
    t0: Vec2,
    t1: Vec2,
    clamp: Option<(Vec2, Vec2)>,
}

impl Hermite {
    fn at(&self, t: f64) -> Vec2 {
        let (t2, t3) = (t * t, t * t * t);
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        let mut p = add(
            add(mul(self.p0, h00), mul(self.t0, h10)),
            add(mul(self.p1, h01), mul(self.t1, h11)),
        );
        if let Some((lo, hi)) = self.clamp {
            p = [p[0].clamp(lo[0], hi[0]), p[1].clamp(lo[1], hi[1])];
        }
        p
    }

    fn derivative(&self, t: f64) -> Vec2 {
        let t2 = t * t;
        let d00 = 6.0 * t2 - 6.0 * t;
        let d10 = 3.0 * t2 - 4.0 * t + 1.0;
        let d01 = -6.0 * t2 + 6.0 * t;
        let d11 = 3.0 * t2 - 2.0 * t;
        add(
            add(mul(self.p0, d00), mul(self.t0, d10)),
            add(mul(self.p1, d01), mul(self.t1, d11)),
        )
    }

    /// Appends the interior points of `t0..t1` (exclusive) so every piece
    /// is at most `max_len` long and within [`FLATNESS_PX`] of its chord.
    fn flatten(
        &self,
        (t0, a): (f64, Vec2),
        (t1, b): (f64, Vec2),
        max_len: f64,
        depth: u32,
        out: &mut Vec<Vec2>,
    ) {
        if depth < MAX_DEPTH && !self.is_flat((t0, a), (t1, b), max_len) {
            let tm = 0.5 * (t0 + t1);
            let m = self.at(tm);
            self.flatten((t0, a), (tm, m), max_len, depth + 1, out);
            out.push(m);
            self.flatten((tm, m), (t1, b), max_len, depth + 1, out);
        }
    }

    /// The piece's Bézier control points lie within the flatness band of
    /// its chord (so the whole piece does), and the chord is short enough.
    fn is_flat(&self, (t0, a): (f64, Vec2), (t1, b): (f64, Vec2), max_len: f64) -> bool {
        let chord = sub(b, a);
        let chord_len = len(chord);
        if chord_len > max_len {
            return false;
        }
        let dt = (t1 - t0) / 3.0;
        let c1 = add(a, mul(self.derivative(t0), dt));
        let c2 = sub(b, mul(self.derivative(t1), dt));
        let distance = |c: Vec2| {
            let d = sub(c, a);
            if chord_len > 0.0 {
                (d[0] * chord[1] - d[1] * chord[0]).abs() / chord_len
            } else {
                len(d)
            }
        };
        distance(c1) <= FLATNESS_PX && distance(c2) <= FLATNESS_PX
    }
}

/// Steffen's slopes (`dv/da`) at each point; `a` strictly monotone.
fn steffen_slopes(a: &[f64], v: &[f64]) -> Vec<f64> {
    let n = a.len();
    let h: Vec<f64> = a.windows(2).map(|w| w[1] - w[0]).collect();
    let s: Vec<f64> = (0..n - 1).map(|i| (v[i + 1] - v[i]) / h[i]).collect();
    let sign = |x: f64| {
        if x > 0.0 {
            1.0
        } else if x < 0.0 {
            -1.0
        } else {
            0.0
        }
    };
    // One-sided parabola at an end, limited so the end interval stays
    // monotone (Steffen 1990, eqs. 26–27).
    let end = |s0: f64, s1: f64, h0: f64, h1: f64| {
        let p = s0 * (1.0 + h0 / (h0 + h1)) - s1 * h0 / (h0 + h1);
        if p * s0 <= 0.0 {
            0.0
        } else if p.abs() > 2.0 * s0.abs() {
            2.0 * s0
        } else {
            p
        }
    };
    let mut slopes = Vec::with_capacity(n);
    slopes.push(end(s[0], s[1], h[0], h[1]));
    for i in 1..n - 1 {
        let p = (s[i - 1] * h[i] + s[i] * h[i - 1]) / (h[i - 1] + h[i]);
        let limit = s[i - 1].abs().min(s[i].abs()).min(0.5 * p.abs());
        slopes.push((sign(s[i - 1]) + sign(s[i])) * limit);
    }
    slopes.push(end(s[n - 2], s[n - 3], h[n - 2], h[n - 3]));
    slopes
}

/// Centripetal Catmull-Rom tangent at `cur` with respect to the knot
/// parameter, given its neighbours and knot spans (Barry–Goldman form).
fn catmull_rom_tangent(prev: Vec2, cur: Vec2, next: Vec2, d_prev: f64, d_next: f64) -> Vec2 {
    if d_prev <= f64::EPSILON && d_next <= f64::EPSILON {
        [0.0, 0.0]
    } else if d_prev <= f64::EPSILON {
        mul(sub(next, cur), 1.0 / d_next)
    } else if d_next <= f64::EPSILON {
        mul(sub(cur, prev), 1.0 / d_prev)
    } else {
        add(
            sub(
                mul(sub(cur, prev), 1.0 / d_prev),
                mul(sub(next, prev), 1.0 / (d_prev + d_next)),
            ),
            mul(sub(next, cur), 1.0 / d_next),
        )
    }
}

/// The interval each of `count` unchanged points lies in.
fn identity_intervals(count: usize) -> Vec<u32> {
    (0..count)
        .map(|i| i.min(count.saturating_sub(2)) as u32)
        .collect()
}

/// A smooth curve through `points` (px), flattened to a polyline whose
/// segments are at most `max_segment_px` long (at least one segment per
/// input interval).
///
/// When the points advance strictly along `along` (the usual line chart),
/// the curve is a monotone cubic Hermite spline (Steffen 1990) in that
/// parameter: it never overshoots between two points, so a value never
/// dips below a zero it did not reach, and flat runs stay flat. Otherwise
/// it is a centripetal Catmull-Rom spline through the points.
///
/// Every input point appears in the output exactly (bit for bit). Returns
/// the polyline and, for each output point, the index of the input
/// interval it lies in (`0..points.len() - 1`; the last point maps to the
/// last interval). Fewer than three points come back unchanged.
pub fn smooth(points: &[[f32; 2]], along: Along, max_segment_px: f32) -> (Vec<[f32; 2]>, Vec<u32>) {
    let n = points.len();
    // Non-finite points have no curve through them; the caller splits gaps.
    if n < 3 || points.iter().flatten().any(|c| !c.is_finite()) {
        return (points.to_vec(), identity_intervals(n));
    }
    // A non-positive or non-finite bound means no length limit.
    let max_len = if max_segment_px > 0.0 {
        f64::from(max_segment_px)
    } else {
        f64::INFINITY
    };
    let p: Vec<Vec2> = points
        .iter()
        .map(|q| [f64::from(q[0]), f64::from(q[1])])
        .collect();

    let (ai, vi) = (along.axis(), 1 - along.axis());
    let increasing = p.windows(2).all(|w| w[1][ai] > w[0][ai]);
    let decreasing = p.windows(2).all(|w| w[1][ai] < w[0][ai]);
    let segments: Vec<Hermite> = if increasing || decreasing {
        let a: Vec<f64> = p.iter().map(|q| q[ai]).collect();
        let v: Vec<f64> = p.iter().map(|q| q[vi]).collect();
        let slopes = steffen_slopes(&a, &v);
        let tangent = |h: f64, slope: f64| {
            let mut t = [0.0; 2];
            t[ai] = h;
            t[vi] = slope * h;
            t
        };
        (0..n - 1)
            .map(|i| {
                let (p0, p1) = (p[i], p[i + 1]);
                let h = p1[ai] - p0[ai];
                let lo = [p0[0].min(p1[0]), p0[1].min(p1[1])];
                let hi = [p0[0].max(p1[0]), p0[1].max(p1[1])];
                Hermite {
                    p0,
                    p1,
                    t0: tangent(h, slopes[i]),
                    t1: tangent(h, slopes[i + 1]),
                    clamp: Some((lo, hi)),
                }
            })
            .collect()
    } else {
        let knot = |a: Vec2, b: Vec2| len(sub(b, a)).sqrt();
        // Phantom end points mirror the neighbour, so the end tangent is the chord.
        let at = |i: isize| -> Vec2 {
            if i < 0 {
                sub(mul(p[0], 2.0), p[1])
            } else if i as usize >= n {
                sub(mul(p[n - 1], 2.0), p[n - 2])
            } else {
                p[i as usize]
            }
        };
        (0..n as isize - 1)
            .map(|i| {
                let [q0, q1, q2, q3] = [at(i - 1), at(i), at(i + 1), at(i + 2)];
                let (d0, d1, d2) = (knot(q0, q1), knot(q1, q2), knot(q2, q3));
                Hermite {
                    p0: q1,
                    p1: q2,
                    t0: mul(catmull_rom_tangent(q0, q1, q2, d0, d1), d1),
                    t1: mul(catmull_rom_tangent(q1, q2, q3, d1, d2), d1),
                    clamp: None,
                }
            })
            .collect()
    };

    let mut out = Vec::with_capacity(n * 4);
    let mut intervals = Vec::with_capacity(n * 4);
    let mut interior = Vec::new();
    for (i, segment) in segments.iter().enumerate() {
        out.push(points[i]);
        intervals.push(i as u32);
        interior.clear();
        segment.flatten(
            (0.0, segment.p0),
            (1.0, segment.p1),
            max_len,
            0,
            &mut interior,
        );
        out.extend(interior.iter().map(|q| [q[0] as f32, q[1] as f32]));
        intervals.extend(std::iter::repeat_n(i as u32, interior.len()));
    }
    out.push(points[n - 1]);
    intervals.push((n - 2) as u32);
    (out, intervals)
}

/// A step line through `points` (px) advancing along `along`: each value
/// holds until the next point (`End`), jumps at once (`Start`), or jumps
/// half way (`Middle`), as ECharts `step`. Returns the polyline and the
/// input index each output point came from.
pub fn step(points: &[[f32; 2]], along: Along, step: Step) -> (Vec<[f32; 2]>, Vec<u32>) {
    let (ai, vi) = (along.axis(), 1 - along.axis());
    let corner = |a: f32, v: f32| {
        let mut q = [0.0; 2];
        q[ai] = a;
        q[vi] = v;
        q
    };
    let mut out = Vec::with_capacity(points.len() * 3);
    let mut sources = Vec::with_capacity(points.len() * 3);
    for (i, w) in points.windows(2).enumerate() {
        let (pt, next) = (w[0], w[1]);
        let i = i as u32;
        out.push(pt);
        sources.push(i);
        match step {
            Step::Start => {
                out.push(corner(pt[ai], next[vi]));
                sources.push(i + 1);
            }
            Step::Middle => {
                let middle = (pt[ai] + next[ai]) / 2.0;
                out.extend([corner(middle, pt[vi]), corner(middle, next[vi])]);
                sources.extend([i, i + 1]);
            }
            Step::End => {
                out.push(corner(next[ai], pt[vi]));
                sources.push(i);
            }
        }
    }
    if let Some(&last) = points.last() {
        out.push(last);
        sources.push(points.len() as u32 - 1);
    }
    (out, sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: [[f32; 2]; 7] = [
        [0.0, 100.0],
        [13.0, 40.0],
        [40.0, 40.0],
        [55.0, 0.0],
        [71.3, 0.0],
        [90.0, 80.7],
        [140.0, 95.0],
    ];

    fn assert_contains_inputs(points: &[[f32; 2]], out: &[[f32; 2]], intervals: &[u32]) {
        let mut cursor = 0;
        for (i, p) in points.iter().enumerate() {
            let at = out[cursor..]
                .iter()
                .position(|q| q[0].to_bits() == p[0].to_bits() && q[1].to_bits() == p[1].to_bits())
                .expect("input point present bit for bit")
                + cursor;
            assert_eq!(intervals[at] as usize, i.min(points.len() - 2));
            cursor = at + 1;
        }
        assert_eq!(out.last(), points.last());
    }

    #[test]
    fn monotone_curve_keeps_points_and_never_overshoots() {
        let (out, intervals) = smooth(&LINE, Along::X, 4.0);
        assert_eq!(out.len(), intervals.len());
        assert_contains_inputs(&LINE, &out, &intervals);
        assert!(intervals.windows(2).all(|w| w[0] <= w[1]));
        for (q, &i) in out.iter().zip(&intervals) {
            let (a, b) = (LINE[i as usize], LINE[i as usize + 1]);
            assert!(
                q[0] >= a[0] && q[0] <= b[0],
                "{q:?} outside x of interval {i}"
            );
            assert!(
                q[1] >= a[1].min(b[1]) && q[1] <= a[1].max(b[1]),
                "{q:?} overshoots interval {i}"
            );
        }
        // Flat runs stay exactly flat.
        for (q, &i) in out.iter().zip(&intervals) {
            if i == 1 {
                assert_eq!(q[1], 40.0);
            }
            if i == 3 {
                assert_eq!(q[1], 0.0);
            }
        }
        // The curve is actually smooth, not the input polyline.
        assert!(out.len() > LINE.len() * 3);
    }

    #[test]
    fn monotone_along_y_and_descending() {
        let points: Vec<[f32; 2]> = LINE.iter().map(|p| [p[1], 300.0 - p[0]]).collect();
        let (out, intervals) = smooth(&points, Along::Y, 3.0);
        assert_contains_inputs(&points, &out, &intervals);
        for (q, &i) in out.iter().zip(&intervals) {
            let (a, b) = (points[i as usize], points[i as usize + 1]);
            assert!(q[0] >= a[0].min(b[0]) && q[0] <= a[0].max(b[0]));
            assert!(q[1] >= a[1].min(b[1]) && q[1] <= a[1].max(b[1]));
        }
    }

    #[test]
    fn segments_respect_length_bound() {
        for max in [1.0, 2.5, 10.0] {
            let (out, intervals) = smooth(&LINE, Along::X, max);
            for (w, iw) in out.windows(2).zip(intervals.windows(2)) {
                let length = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
                assert!(length <= max + 1e-3, "segment {length} > {max}");
                assert!(iw[1] - iw[0] <= 1);
            }
        }
        // Without a bound there is still at least one segment per interval.
        let (out, intervals) = smooth(&LINE, Along::X, 0.0);
        assert_contains_inputs(&LINE, &out, &intervals);
    }

    #[test]
    fn short_input_is_unchanged() {
        assert_eq!(smooth(&[], Along::X, 2.0), (vec![], vec![]));
        assert_eq!(
            smooth(&[[1.0, 2.0]], Along::X, 2.0),
            (vec![[1.0, 2.0]], vec![0])
        );
        let two = [[0.0, 0.0], [50.0, 10.0]];
        assert_eq!(smooth(&two, Along::X, 2.0), (two.to_vec(), vec![0, 0]));
    }

    #[test]
    fn non_monotone_input_uses_catmull_rom() {
        // x turns back, so no function of x passes through these.
        let points = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
        let (out, intervals) = smooth(&points, Along::X, 2.0);
        assert_contains_inputs(&points, &out, &intervals);
        // The centripetal curve rounds the corners outward.
        assert!(out.iter().any(|q| q[0] > 100.5));
        for w in out.windows(2) {
            assert!((w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]) <= 2.0 + 1e-3);
        }
        // Repeated points do not produce NaN.
        let repeated = [[0.0, 0.0], [10.0, 5.0], [10.0, 5.0], [0.0, 20.0]];
        let (out, intervals) = smooth(&repeated, Along::X, 2.0);
        assert!(out.iter().flatten().all(|c| c.is_finite()));
        assert_contains_inputs(&repeated, &out, &intervals);
    }

    #[test]
    fn step_shapes() {
        let points = [[0.0, 10.0], [10.0, 20.0], [30.0, 5.0]];
        assert_eq!(
            step(&points, Along::X, Step::End),
            (
                vec![
                    [0.0, 10.0],
                    [10.0, 10.0],
                    [10.0, 20.0],
                    [30.0, 20.0],
                    [30.0, 5.0]
                ],
                vec![0, 0, 1, 1, 2],
            )
        );
        assert_eq!(
            step(&points, Along::X, Step::Start),
            (
                vec![
                    [0.0, 10.0],
                    [0.0, 20.0],
                    [10.0, 20.0],
                    [10.0, 5.0],
                    [30.0, 5.0]
                ],
                vec![0, 1, 1, 2, 2],
            )
        );
        assert_eq!(
            step(&points, Along::X, Step::Middle),
            (
                vec![
                    [0.0, 10.0],
                    [5.0, 10.0],
                    [5.0, 20.0],
                    [10.0, 20.0],
                    [20.0, 20.0],
                    [20.0, 5.0],
                    [30.0, 5.0],
                ],
                vec![0, 0, 1, 1, 1, 2, 2],
            )
        );
        // Along y the roles swap: the step runs vertically first.
        assert_eq!(
            step(&[[10.0, 0.0], [20.0, 10.0]], Along::Y, Step::End).0,
            vec![[10.0, 0.0], [10.0, 10.0], [20.0, 10.0]]
        );
        assert_eq!(step(&[], Along::X, Step::End), (vec![], vec![]));
        assert_eq!(
            step(&[[1.0, 1.0]], Along::X, Step::Middle),
            (vec![[1.0, 1.0]], vec![0])
        );
    }
}
