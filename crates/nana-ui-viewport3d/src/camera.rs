//! Orbit camera and 3D ray picking helpers.

use glam::{Mat4, Vec3};

/// 3D ray in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

/// Ray hit result against a scene instance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit3d {
    pub instance_id: u64,
    pub distance: f32,
    pub point: Vec3,
}

impl Ray {
    /// Intersects an axis-aligned bounding box; returns entry distance `t >= 0.0`.
    pub fn intersect_aabb(self, min: Vec3, max: Vec3) -> Option<f32> {
        let mut tmin = 0.0f32;
        let mut tmax = f32::INFINITY;
        for axis in 0..3 {
            let o = self.origin[axis];
            let d = self.dir[axis];
            if d.abs() < 1e-8 {
                if o < min[axis] || o > max[axis] {
                    return None;
                }
            } else {
                let inv = 1.0 / d;
                let mut t1 = (min[axis] - o) * inv;
                let mut t2 = (max[axis] - o) * inv;
                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                }
                tmin = tmin.max(t1);
                tmax = tmax.min(t2);
                if tmin > tmax {
                    return None;
                }
            }
        }
        Some(tmin)
    }

    /// Möller–Trumbore ray-triangle intersection; returns `t >= 0.0`.
    pub fn intersect_triangle(self, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f32> {
        let e1 = v1 - v0;
        let e2 = v2 - v0;
        let h = self.dir.cross(e2);
        let a = e1.dot(h);
        if a.abs() < 1e-7 {
            return None;
        }
        let f = 1.0 / a;
        let s = self.origin - v0;
        let u = f * s.dot(h);
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(e1);
        let v = f * self.dir.dot(q);
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = f * e2.dot(q);
        (t >= 0.0).then_some(t)
    }
}

/// Perspective orbit camera around a target point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov_y_radians: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            yaw: 0.65,
            pitch: 0.42,
            distance: 3.2,
            fov_y_radians: 42.0f32.to_radians(),
            near: 0.05,
            far: 200.0,
        }
    }
}

impl OrbitCamera {
    /// Rotates yaw/pitch by screen-space pointer delta in logical pixels.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-1.45, 1.45);
    }

    /// Pans the orbit target parallel to the view plane.
    pub fn pan(&mut self, dx: f32, dy: f32, viewport_height: f32) {
        let scale =
            (2.0 * self.distance * (self.fov_y_radians * 0.5).tan()) / viewport_height.max(1.0);
        let eye = self.eye_position();
        let forward = (self.target - eye).normalize_or_zero();
        let right = forward.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(forward).normalize_or(Vec3::Y);
        self.target += (-right * dx + up * dy) * scale;
    }

    /// Zooms by wheel delta (positive zooms in).
    pub fn zoom(&mut self, delta: f32) {
        let factor = (-delta * 0.1).exp();
        self.distance = (self.distance * factor).clamp(0.25, 100.0);
    }

    /// Frames the axis-aligned bounding box `[min, max]`.
    pub fn fit_bounds(&mut self, min: Vec3, max: Vec3) {
        self.target = (min + max) * 0.5;
        let radius = ((max - min) * 0.5).length().max(0.1);
        let sin_half = (self.fov_y_radians * 0.5).sin().max(0.1);
        self.distance = (radius / sin_half * 1.25).clamp(0.4, 100.0);
    }

    /// Camera eye position in world space.
    pub fn eye_position(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let offset = Vec3::new(cp * sy, sp, cp * cy) * self.distance;
        self.target + offset
    }

    /// View matrix (right-handed, Y-up).
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye_position(), self.target, Vec3::Y)
    }

    /// Perspective projection matrix (`wgpu` depth `[0, 1]`).
    pub fn proj(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(
            self.fov_y_radians,
            aspect.max(1e-3),
            self.near.max(1e-4),
            self.far.max(self.near + 1.0),
        )
    }

    /// Combined view-projection matrix.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    /// Constructs a world-space ray from normalized texture UVs `[u, v]` in `[0, 1]^2`
    /// (matching [`nana_ui::runtime::GpuTextureView::texture_uv_at`], top-left origin).
    pub fn ray_from_uv(&self, uv: [f32; 2], aspect: f32) -> Ray {
        let ndc_x = uv[0] * 2.0 - 1.0;
        let ndc_y = 1.0 - uv[1] * 2.0;
        let inv = self.view_proj(aspect).inverse();
        let near_w = inv.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let far_w = inv.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        Ray {
            origin: near_w,
            dir: (far_w - near_w).normalize_or_zero(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbit_clamps_pitch_and_zoom_clamps_distance() {
        let mut cam = OrbitCamera::default();
        cam.orbit(100.0, 5000.0);
        assert!(cam.pitch <= 1.46 && cam.pitch >= -1.46);
        cam.zoom(1000.0);
        assert!(cam.distance >= 0.25);
        cam.zoom(-1000.0);
        assert!(cam.distance <= 100.0);
    }

    #[test]
    fn center_uv_ray_hits_target_box() {
        let mut cam = OrbitCamera::default();
        cam.fit_bounds(Vec3::splat(-0.5), Vec3::splat(0.5));
        let ray = cam.ray_from_uv([0.5, 0.5], 16.0 / 9.0);
        assert!(
            ray.intersect_aabb(Vec3::splat(-0.5), Vec3::splat(0.5))
                .is_some()
        );
        let miss = cam.ray_from_uv([0.02, 0.02], 16.0 / 9.0);
        assert!(
            miss.intersect_aabb(Vec3::splat(-0.5), Vec3::splat(0.5))
                .is_none()
        );
    }
}
