//! Geometric default application mark. Not a product brand.

const ACCENT: [f32; 3] = [73.0 / 255.0, 145.0 / 255.0, 215.0 / 255.0];

/// Rasterize the default mark as unpremultiplied RGBA.
pub fn rasterize(size: u32) -> Vec<u8> {
    let size = size.max(1);
    let scale = size as f32;
    let mut rgba = vec![0_u8; size as usize * size as usize * 4];
    for y in 0..size {
        for x in 0..size {
            let px = (x as f32 + 0.5) / scale;
            let py = (y as f32 + 0.5) / scale;
            let p = (px, py);
            let plate = coverage(sd_round_box(p, (0.5, 0.5), (0.42, 0.42), 0.22), scale);
            let glyph = n_coverage(p, scale);
            let mut color = [0.0_f32, 0.0, 0.0, 0.0];
            color = over(color, [ACCENT[0], ACCENT[1], ACCENT[2], plate]);
            color = over(color, [1.0, 1.0, 1.0, glyph]);
            let i = ((y * size + x) * 4) as usize;
            rgba[i] = to_u8(color[0]);
            rgba[i + 1] = to_u8(color[1]);
            rgba[i + 2] = to_u8(color[2]);
            rgba[i + 3] = to_u8(color[3]);
        }
    }
    rgba
}

fn n_coverage(p: (f32, f32), scale: f32) -> f32 {
    let left = sd_box(p, (0.34, 0.5), (0.06, 0.22));
    let right = sd_box(p, (0.66, 0.5), (0.06, 0.22));
    let diag = sd_segment(p, (0.34, 0.28), (0.66, 0.72), 0.06);
    coverage(left.min(right).min(diag), scale)
}

fn sd_round_box(p: (f32, f32), center: (f32, f32), half: (f32, f32), radius: f32) -> f32 {
    let q = (
        (p.0 - center.0).abs() - (half.0 - radius),
        (p.1 - center.1).abs() - (half.1 - radius),
    );
    q.0.max(0.0).hypot(q.1.max(0.0)) + q.0.min(q.1).min(0.0) - radius
}

fn sd_box(p: (f32, f32), center: (f32, f32), half: (f32, f32)) -> f32 {
    let q = (
        (p.0 - center.0).abs() - half.0,
        (p.1 - center.1).abs() - half.1,
    );
    q.0.max(0.0).hypot(q.1.max(0.0)) + q.0.min(q.1).min(0.0)
}

fn sd_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32), radius: f32) -> f32 {
    let pa = (p.0 - a.0, p.1 - a.1);
    let ba = (b.0 - a.0, b.1 - a.1);
    let baba = ba.0 * ba.0 + ba.1 * ba.1;
    let h = if baba <= f32::EPSILON {
        0.0
    } else {
        ((pa.0 * ba.0 + pa.1 * ba.1) / baba).clamp(0.0, 1.0)
    };
    (pa.0 - ba.0 * h).hypot(pa.1 - ba.1 * h) - radius
}

fn coverage(distance: f32, scale: f32) -> f32 {
    (0.5 - distance * scale).clamp(0.0, 1.0)
}

fn over(dst: [f32; 4], src: [f32; 4]) -> [f32; 4] {
    let out_a = src[3] + dst[3] * (1.0 - src[3]);
    if out_a <= 0.0 {
        return [0.0; 4];
    }
    let blend = |i: usize| (src[i] * src[3] + dst[i] * dst[3] * (1.0 - src[3])) / out_a;
    [blend(0), blend(1), blend(2), out_a]
}

fn to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_has_opaque_plate_and_white_glyph() {
        let pixels = rasterize(64);
        let mut plate = 0usize;
        let mut glyph = 0usize;
        for [r, g, b, a] in pixels.as_chunks::<4>().0 {
            if *a > 200 && *b > *r && *g > 80 {
                plate += 1;
            }
            if *a > 200 && *r > 220 && *g > 220 && *b > 220 {
                glyph += 1;
            }
        }
        assert!(plate > 200, "expected blue plate pixels, got {plate}");
        assert!(glyph > 40, "expected white N pixels, got {glyph}");
    }
}
