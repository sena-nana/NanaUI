//! Down-sampling a long series to what the plot can show.

fn is_finite(p: &[f64; 2]) -> bool {
    p[0].is_finite() && p[1].is_finite()
}

/// Largest-Triangle-Three-Buckets: indices of at most `threshold` points of
/// `points` (`[x, y]`, any units, ascending in x) that keep the shape.
/// Always keeps the first and last. `threshold < 3` or `threshold >=
/// points.len()` keeps every index. Non-finite points are never chosen
/// unless they are the first or last.
pub fn lttb(points: &[[f64; 2]], threshold: usize) -> Vec<usize> {
    let n = points.len();
    if threshold < 3 || threshold >= n {
        return (0..n).collect();
    }
    // The bucket edges of the `n - 2` inner points, ECharts style.
    let every = (n - 2) as f64 / (threshold - 2) as f64;
    let edge = |bucket: usize| ((bucket as f64 * every) as usize + 1).min(n - 1);
    let mut kept = Vec::with_capacity(threshold);
    kept.push(0);
    let mut anchor = is_finite(&points[0]).then_some(points[0]);
    for bucket in 0..threshold - 2 {
        let (start, end) = (edge(bucket), edge(bucket + 1));
        // The next bucket's mean; for the last bucket that is the last point.
        let next = if bucket + 3 == threshold {
            n - 1..n
        } else {
            end..edge(bucket + 2)
        };
        let (mut sum, mut count) = ([0.0; 2], 0.0);
        for p in points[next].iter().filter(|p| is_finite(p)) {
            sum = [sum[0] + p[0], sum[1] + p[1]];
            count += 1.0;
        }
        let mean = (count > 0.0).then(|| [sum[0] / count, sum[1] / count]);
        // Without one of the triangle's other corners, the distance in y to
        // the one there is stands in for the area.
        let area = |b: &[f64; 2]| match (anchor, mean) {
            (Some(a), Some(c)) => {
                ((a[0] - c[0]) * (b[1] - a[1]) - (a[0] - b[0]) * (c[1] - a[1])).abs()
            }
            (Some(o), None) | (None, Some(o)) => (b[1] - o[1]).abs(),
            (None, None) => 0.0,
        };
        let mut best: Option<(usize, f64)> = None;
        for (i, p) in points.iter().enumerate().take(end).skip(start) {
            if !is_finite(p) {
                continue;
            }
            let a = area(p);
            if best.is_none_or(|(_, best_area)| a > best_area) {
                best = Some((i, a));
            }
        }
        if let Some((i, _)) = best {
            kept.push(i);
            anchor = Some(points[i]);
        }
    }
    kept.push(n - 1);
    kept
}

/// Min-max per column: `points` (`[x, y]` in px, ascending in x) are
/// grouped into columns `column_px` wide; each column keeps its first,
/// minimum-y, maximum-y and last point, in x order and without repeats.
/// So a spike narrower than a pixel stays visible. Returns indices,
/// ascending. Non-finite points are skipped.
pub fn min_max(points: &[[f32; 2]], column_px: f32) -> Vec<usize> {
    let finite = |p: &[f32; 2]| p[0].is_finite() && p[1].is_finite();
    let indices = points
        .iter()
        .enumerate()
        .filter(|(_, p)| finite(p))
        .map(|(i, _)| i);
    if !(column_px.is_finite() && column_px > 0.0) {
        return indices.collect();
    }

    /// One column: its index and the first, min-y, max-y and last points.
    struct Column {
        column: f32,
        picks: [usize; 4],
    }
    fn flush(column: &Column, out: &mut Vec<usize>) {
        let mut picks = column.picks;
        picks.sort_unstable();
        for i in picks {
            if out.last() != Some(&i) {
                out.push(i);
            }
        }
    }

    let mut out = Vec::new();
    let mut current: Option<Column> = None;
    for i in indices {
        let p = points[i];
        let column = (p[0] / column_px).floor();
        match &mut current {
            Some(c) if c.column == column => {
                let [_, min, max, last] = &mut c.picks;
                if p[1] < points[*min][1] {
                    *min = i;
                }
                if p[1] > points[*max][1] {
                    *max = i;
                }
                *last = i;
            }
            _ => {
                if let Some(c) = &current {
                    flush(c, &mut out);
                }
                current = Some(Column {
                    column,
                    picks: [i; 4],
                });
            }
        }
    }
    if let Some(c) = &current {
        flush(c, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(n: usize) -> Vec<[f64; 2]> {
        (0..n)
            .map(|i| [i as f64, (i as f64 * 0.05).sin() * 10.0])
            .collect()
    }

    #[test]
    fn lttb_keeps_ends_and_count() {
        let points = wave(1_000);
        for threshold in [3, 10, 100, 999] {
            let kept = lttb(&points, threshold);
            assert!(kept.len() <= threshold);
            assert_eq!(kept.first(), Some(&0));
            assert_eq!(kept.last(), Some(&999));
            assert!(kept.windows(2).all(|w| w[0] < w[1]));
        }
        assert_eq!(lttb(&points, 2).len(), 1_000);
        assert_eq!(lttb(&points, 1_000).len(), 1_000);
        assert_eq!(lttb(&[], 10), Vec::<usize>::new());
    }

    #[test]
    fn lttb_keeps_a_spike() {
        let mut points = wave(1_000);
        points[437][1] = 500.0;
        points[712][1] = -300.0;
        let kept = lttb(&points, 50);
        assert!(kept.contains(&437));
        assert!(kept.contains(&712));
    }

    #[test]
    fn lttb_skips_non_finite() {
        let mut points = wave(200);
        points[0][1] = f64::NAN;
        for i in (5..195).step_by(7) {
            points[i][1] = f64::NAN;
        }
        points[50][0] = f64::INFINITY;
        points[199][1] = f64::NAN;
        let kept = lttb(&points, 20);
        assert_eq!(kept.first(), Some(&0));
        assert_eq!(kept.last(), Some(&199));
        for &i in &kept[1..kept.len() - 1] {
            assert!(points[i][0].is_finite() && points[i][1].is_finite(), "{i}");
        }
    }

    #[test]
    fn min_max_keeps_narrow_spikes() {
        // Ten points per 1 px column.
        let mut points: Vec<[f32; 2]> = (0..100).map(|i| [i as f32 * 0.1, 50.0]).collect();
        points[33][1] = 0.0;
        points[34][1] = 90.0;
        points[77][1] = f32::NAN;
        let kept = min_max(&points, 1.0);
        assert!(kept.contains(&33) && kept.contains(&34));
        assert!(!kept.contains(&77));
        assert!(kept.windows(2).all(|w| w[0] < w[1]));
        assert!(kept.len() <= 40);
        // Each column's first and last survive.
        assert!(kept.contains(&30) && kept.contains(&39));
    }

    #[test]
    fn min_max_degenerate() {
        let points = [[0.0, 1.0], [f32::NAN, 2.0], [3.0, 3.0]];
        assert_eq!(min_max(&points, 0.0), vec![0, 2]);
        assert_eq!(min_max(&points, 10.0), vec![0, 2]);
        assert_eq!(min_max(&[], 1.0), Vec::<usize>::new());
        assert_eq!(min_max(&[[5.0, 5.0]], 1.0), vec![0]);
    }
}
