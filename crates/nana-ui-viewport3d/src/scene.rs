//! Lightweight 3D scene graph, materials, lighting, and ray picking.

use glam::{Mat4, Vec3};

use crate::camera::{Hit3d, Ray};
use crate::mesh::MeshData;

/// Surface material multiplier in linear RGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material3d {
    pub color: [f32; 4],
    pub roughness: f32,
    pub emissive: [f32; 3],
}

impl Default for Material3d {
    fn default() -> Self {
        Self {
            color: [1.0; 4],
            roughness: 0.6,
            emissive: [0.0; 3],
        }
    }
}

/// Three-point / hemisphere light rig and gradient backdrop in linear RGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightRig {
    pub sun_dir: [f32; 3],
    pub sun_color: [f32; 3],
    pub sky_color: [f32; 3],
    pub ground_color: [f32; 3],
    pub background_top: [f32; 3],
    pub background_bottom: [f32; 3],
}

impl Default for LightRig {
    fn default() -> Self {
        Self {
            sun_dir: [0.55, 0.85, 0.45],
            sun_color: [1.0, 0.96, 0.90],
            sky_color: [0.36, 0.44, 0.56],
            ground_color: [0.22, 0.19, 0.17],
            background_top: [0.16, 0.19, 0.24],
            background_bottom: [0.07, 0.08, 0.11],
        }
    }
}

/// Placed mesh instance in [`Scene3d`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance3d {
    pub id: u64,
    pub mesh_index: usize,
    pub transform: Mat4,
    pub material: Material3d,
}

/// Scene rendered by [`crate::Viewport3dRenderer`].
#[derive(Debug, Clone)]
pub struct Scene3d {
    meshes: Vec<MeshData>,
    instances: Vec<Instance3d>,
    pub show_grid: bool,
    pub grid_extent: f32,
    pub grid_step: f32,
    pub lights: LightRig,
    revision: u64,
    mesh_revision: u64,
}

impl Default for Scene3d {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene3d {
    pub fn new() -> Self {
        Self {
            meshes: Vec::new(),
            instances: Vec::new(),
            show_grid: true,
            grid_extent: 4.0,
            grid_step: 0.5,
            lights: LightRig::default(),
            revision: 1,
            mesh_revision: 1,
        }
    }

    pub fn add_mesh(&mut self, mesh: MeshData) -> usize {
        let index = self.meshes.len();
        self.meshes.push(mesh);
        self.mesh_revision = self.mesh_revision.wrapping_add(1);
        self.revision = self.revision.wrapping_add(1);
        index
    }

    pub fn clear(&mut self) {
        self.meshes.clear();
        self.instances.clear();
        self.mesh_revision = self.mesh_revision.wrapping_add(1);
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn add_instance(&mut self, instance: Instance3d) {
        self.instances.push(instance);
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn instances(&self) -> &[Instance3d] {
        &self.instances
    }

    pub fn instances_mut(&mut self) -> &mut [Instance3d] {
        self.revision = self.revision.wrapping_add(1);
        &mut self.instances
    }

    pub fn meshes(&self) -> &[MeshData] {
        &self.meshes
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn mesh_revision(&self) -> u64 {
        self.mesh_revision
    }

    /// Combined world-space AABB across all instances (returns `None` if empty).
    pub fn world_bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        for inst in &self.instances {
            let Some(mesh) = self.meshes.get(inst.mesh_index) else {
                continue;
            };
            let bmin = Vec3::from_array(mesh.bounds.0);
            let bmax = Vec3::from_array(mesh.bounds.1);
            for sx in [bmin.x, bmax.x] {
                for sy in [bmin.y, bmax.y] {
                    for sz in [bmin.z, bmax.z] {
                        let p = inst.transform.transform_point3(Vec3::new(sx, sy, sz));
                        min = min.min(p);
                        max = max.max(p);
                        any = true;
                    }
                }
            }
        }
        any.then_some((min, max))
    }

    /// Exact triangle ray-pick across all instances (accelerated by local AABB).
    pub fn pick(&self, ray: Ray) -> Option<Hit3d> {
        let mut best: Option<Hit3d> = None;
        for inst in &self.instances {
            let Some(mesh) = self.meshes.get(inst.mesh_index) else {
                continue;
            };
            let inv = inst.transform.inverse();
            let local_origin = inv.transform_point3(ray.origin);
            let local_dir = inv.transform_vector3(ray.dir);
            let local_len = local_dir.length();
            if local_len < 1e-8 {
                continue;
            }
            let local_ray = Ray {
                origin: local_origin,
                dir: local_dir / local_len,
            };
            if local_ray
                .intersect_aabb(
                    Vec3::from_array(mesh.bounds.0),
                    Vec3::from_array(mesh.bounds.1),
                )
                .is_none()
            {
                continue;
            }
            for tri in mesh.indices.chunks_exact(3) {
                let v0 = Vec3::from_array(mesh.vertices[tri[0] as usize].position);
                let v1 = Vec3::from_array(mesh.vertices[tri[1] as usize].position);
                let v2 = Vec3::from_array(mesh.vertices[tri[2] as usize].position);
                if let Some(t_local) = local_ray.intersect_triangle(v0, v1, v2) {
                    let local_hit = local_ray.origin + local_ray.dir * t_local;
                    let world_hit = inst.transform.transform_point3(local_hit);
                    let dist = (world_hit - ray.origin).dot(ray.dir);
                    if dist >= 0.0 && best.is_none_or(|b| dist < b.distance) {
                        best = Some(Hit3d {
                            instance_id: inst.id,
                            distance: dist,
                            point: world_hit,
                        });
                    }
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::OrbitCamera;
    use crate::mesh::cube;

    #[test]
    fn scene_pick_selects_closest_instance() {
        let mut scene = Scene3d::new();
        let box_mesh = scene.add_mesh(cube([1.0, 1.0, 1.0]));
        scene.add_instance(Instance3d {
            id: 10,
            mesh_index: box_mesh,
            transform: Mat4::from_translation(Vec3::new(0.0, 0.0, -2.0)),
            material: Material3d::default(),
        });
        scene.add_instance(Instance3d {
            id: 20,
            mesh_index: box_mesh,
            transform: Mat4::from_translation(Vec3::new(0.0, 0.0, 0.5)),
            material: Material3d::default(),
        });
        let cam = OrbitCamera {
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            distance: 4.0,
            ..Default::default()
        };
        let hit = scene
            .pick(cam.ray_from_uv([0.5, 0.5], 1.0))
            .expect("hit front box");
        assert_eq!(hit.instance_id, 20);
    }
}
