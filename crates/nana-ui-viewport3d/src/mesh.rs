//! CPU mesh data, procedural primitives, and optional glTF / GLB loading.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// Vertex layout shared with `shader.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Vertex3d {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// CPU-side triangle mesh with optional sRGB RGBA8 albedo texture.
#[derive(Debug, Clone)]
pub struct MeshData {
    pub vertices: Vec<Vertex3d>,
    pub indices: Vec<u32>,
    /// Linear RGBA factor multiplied with texture and instance color.
    pub base_color: [f32; 4],
    /// Optional sRGB RGBA8 texture `(width, height, bytes)`.
    pub albedo_rgba: Option<(u32, u32, Vec<u8>)>,
    /// Local-space bounding box `[min, max]`.
    pub bounds: ([f32; 3], [f32; 3]),
}

impl MeshData {
    /// Constructs a mesh from vertices and triangle indices, computing bounds.
    pub fn from_triangles(vertices: Vec<Vertex3d>, indices: Vec<u32>) -> Self {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for v in &vertices {
            let p = Vec3::from_array(v.position);
            min = min.min(p);
            max = max.max(p);
        }
        if vertices.is_empty() {
            min = Vec3::ZERO;
            max = Vec3::ZERO;
        }
        Self {
            vertices,
            indices,
            base_color: [1.0; 4],
            albedo_rgba: None,
            bounds: (min.to_array(), max.to_array()),
        }
    }
}

/// Axis-aligned box centered at the origin with dimensions `size = [w, h, d]`.
pub fn cube(size: [f32; 3]) -> MeshData {
    let [hx, hy, hz] = [size[0] * 0.5, size[1] * 0.5, size[2] * 0.5];
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [0.0, 0.0, 1.0],
            [[-hx, -hy, hz], [hx, -hy, hz], [hx, hy, hz], [-hx, hy, hz]],
        ),
        (
            [0.0, 0.0, -1.0],
            [
                [hx, -hy, -hz],
                [-hx, -hy, -hz],
                [-hx, hy, -hz],
                [hx, hy, -hz],
            ],
        ),
        (
            [1.0, 0.0, 0.0],
            [[hx, -hy, hz], [hx, -hy, -hz], [hx, hy, -hz], [hx, hy, hz]],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [-hx, -hy, -hz],
                [-hx, -hy, hz],
                [-hx, hy, hz],
                [-hx, hy, -hz],
            ],
        ),
        (
            [0.0, 1.0, 0.0],
            [[-hx, hy, hz], [hx, hy, hz], [hx, hy, -hz], [-hx, hy, -hz]],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                [-hx, -hy, -hz],
                [hx, -hy, -hz],
                [hx, -hy, hz],
                [-hx, -hy, hz],
            ],
        ),
    ];
    let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (normal, corners) in faces {
        let base = vertices.len() as u32;
        for (pos, uv) in corners.into_iter().zip(uvs) {
            vertices.push(Vertex3d {
                position: pos,
                normal,
                uv,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    MeshData::from_triangles(vertices, indices)
}

/// UV sphere centered at the origin.
pub fn sphere(radius: f32, rings: u32, sectors: u32) -> MeshData {
    let rings = rings.max(4);
    let sectors = sectors.max(6);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for r in 0..=rings {
        let v = r as f32 / rings as f32;
        let phi = std::f32::consts::PI * v;
        let (sp, cp) = phi.sin_cos();
        for s in 0..=sectors {
            let u = s as f32 / sectors as f32;
            let theta = std::f32::consts::TAU * u;
            let (st, ct) = theta.sin_cos();
            let normal = [sp * ct, cp, sp * st];
            vertices.push(Vertex3d {
                position: [normal[0] * radius, normal[1] * radius, normal[2] * radius],
                normal,
                uv: [u, v],
            });
        }
    }
    let row = sectors + 1;
    for r in 0..rings {
        for s in 0..sectors {
            let a = r * row + s;
            let b = a + 1;
            let c = (r + 1) * row + s;
            let d = c + 1;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    MeshData::from_triangles(vertices, indices)
}

/// Closed cylinder aligned along Y, centered at the origin.
pub fn cylinder(radius: f32, height: f32, segments: u32) -> MeshData {
    let segments = segments.max(6);
    let half_h = height * 0.5;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // Side band
    let side_base = vertices.len() as u32;
    for i in 0..=segments {
        let u = i as f32 / segments as f32;
        let a = u * std::f32::consts::TAU;
        let (sa, ca) = a.sin_cos();
        let n = [ca, 0.0, sa];
        vertices.push(Vertex3d {
            position: [ca * radius, -half_h, sa * radius],
            normal: n,
            uv: [u, 1.0],
        });
        vertices.push(Vertex3d {
            position: [ca * radius, half_h, sa * radius],
            normal: n,
            uv: [u, 0.0],
        });
    }
    for i in 0..segments {
        let b = side_base + i * 2;
        indices.extend_from_slice(&[b, b + 2, b + 1, b + 1, b + 2, b + 3]);
    }

    // Caps
    for (sign, y) in [(1.0f32, half_h), (-1.0f32, -half_h)] {
        let center = vertices.len() as u32;
        vertices.push(Vertex3d {
            position: [0.0, y, 0.0],
            normal: [0.0, sign, 0.0],
            uv: [0.5, 0.5],
        });
        let ring = vertices.len() as u32;
        for i in 0..=segments {
            let a = (i as f32 / segments as f32) * std::f32::consts::TAU;
            let (sa, ca) = a.sin_cos();
            vertices.push(Vertex3d {
                position: [ca * radius, y, sa * radius],
                normal: [0.0, sign, 0.0],
                uv: [ca * 0.5 + 0.5, sa * 0.5 + 0.5],
            });
        }
        for i in 0..segments {
            if sign > 0.0 {
                indices.extend_from_slice(&[center, ring + i + 1, ring + i]);
            } else {
                indices.extend_from_slice(&[center, ring + i, ring + i + 1]);
            }
        }
    }

    MeshData::from_triangles(vertices, indices)
}

/// Horizontal XZ plane centered at the origin, facing +Y.
pub fn plane(width: f32, depth: f32) -> MeshData {
    let hw = width * 0.5;
    let hd = depth * 0.5;
    let vertices = vec![
        Vertex3d {
            position: [-hw, 0.0, hd],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 1.0],
        },
        Vertex3d {
            position: [hw, 0.0, hd],
            normal: [0.0, 1.0, 0.0],
            uv: [1.0, 1.0],
        },
        Vertex3d {
            position: [hw, 0.0, -hd],
            normal: [0.0, 1.0, 0.0],
            uv: [1.0, 0.0],
        },
        Vertex3d {
            position: [-hw, 0.0, -hd],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        },
    ];
    MeshData::from_triangles(vertices, vec![0, 1, 2, 0, 2, 3])
}

/// Thin XZ ground grid built from quads so it renders uniformly on all backends.
pub fn ground_grid(half_extent: f32, step: f32) -> MeshData {
    let step = step.max(0.05);
    let count = ((half_extent / step).floor() as i32).clamp(1, 64);
    let span = count as f32 * step;
    let half_thick = (step * 0.018).max(0.004);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut push_strip = |min_x: f32, max_x: f32, min_z: f32, max_z: f32| {
        let base = vertices.len() as u32;
        for pos in [
            [min_x, 0.0, max_z],
            [max_x, 0.0, max_z],
            [max_x, 0.0, min_z],
            [min_x, 0.0, min_z],
        ] {
            vertices.push(Vertex3d {
                position: pos,
                normal: [0.0, 1.0, 0.0],
                uv: [0.5, 0.5],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };
    for i in -count..=count {
        let p = i as f32 * step;
        push_strip(p - half_thick, p + half_thick, -span, span);
        push_strip(-span, span, p - half_thick, p + half_thick);
    }
    MeshData::from_triangles(vertices, indices)
}

#[cfg(feature = "gltf")]
pub fn load_glb_file(path: &std::path::Path) -> Result<Vec<MeshData>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    load_glb_bytes(&bytes)
}

#[cfg(feature = "gltf")]
pub fn load_glb_bytes(bytes: &[u8]) -> Result<Vec<MeshData>, String> {
    let (doc, buffers, images) = gltf::import_slice(bytes).map_err(|e| format!("gltf: {e}"))?;
    let mut out = Vec::new();
    let mut stack: Vec<(gltf::Node<'_>, glam::Mat4)> = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .map(|scene| scene.nodes().map(|n| (n, glam::Mat4::IDENTITY)).collect())
        .unwrap_or_default();

    while let Some((node, parent)) = stack.pop() {
        let world = parent * glam::Mat4::from_cols_array_2d(&node.transform().matrix());
        let normal_mat = world.inverse().transpose();
        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| d.0.as_slice()));
                let Some(positions) = reader.read_positions() else {
                    continue;
                };
                let positions: Vec<[f32; 3]> = positions.collect();
                let normals: Vec<[f32; 3]> = reader
                    .read_normals()
                    .map(|n| n.collect())
                    .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);
                let uvs: Vec<[f32; 2]> = reader
                    .read_tex_coords(0)
                    .map(|t| t.into_f32().collect())
                    .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);
                let mut vertices = Vec::with_capacity(positions.len());
                for i in 0..positions.len() {
                    let p = world.transform_point3(Vec3::from_array(positions[i]));
                    let n = normal_mat
                        .transform_vector3(Vec3::from_array(normals[i]))
                        .normalize_or(Vec3::Y);
                    vertices.push(Vertex3d {
                        position: p.to_array(),
                        normal: n.to_array(),
                        uv: uvs[i],
                    });
                }
                let indices: Vec<u32> = reader
                    .read_indices()
                    .map(|idx| idx.into_u32().collect())
                    .unwrap_or_else(|| (0..vertices.len() as u32).collect());
                let pbr = primitive.material().pbr_metallic_roughness();
                let base_color = pbr.base_color_factor();
                let albedo_rgba = pbr
                    .base_color_texture()
                    .and_then(|info| images.get(info.texture().source().index()))
                    .and_then(decode_gltf_image);
                let mut data = MeshData::from_triangles(vertices, indices);
                data.base_color = base_color;
                data.albedo_rgba = albedo_rgba;
                out.push(data);
            }
        }
        for child in node.children() {
            stack.push((child, world));
        }
    }
    if out.is_empty() {
        return Err("glTF contained no triangle meshes".into());
    }
    Ok(out)
}

#[cfg(feature = "gltf")]
fn decode_gltf_image(img: &gltf::image::Data) -> Option<(u32, u32, Vec<u8>)> {
    use gltf::image::Format;
    let rgba = match img.format {
        Format::R8G8B8A8 => img.pixels.clone(),
        Format::R8G8B8 => img
            .pixels
            .chunks_exact(3)
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
        Format::R8G8 => img
            .pixels
            .chunks_exact(2)
            .flat_map(|la| [la[0], la[0], la[0], la[1]])
            .collect(),
        Format::R8 => img.pixels.iter().flat_map(|&l| [l, l, l, 255]).collect(),
        _ => return None,
    };
    let max_dim = 1024;
    if img.width > max_dim || img.height > max_dim {
        let dyn_img = image::RgbaImage::from_raw(img.width, img.height, rgba)?;
        let resized = image::imageops::resize(
            &dyn_img,
            img.width.min(max_dim),
            img.height.min(max_dim),
            image::imageops::FilterType::Triangle,
        );
        return Some((resized.width(), resized.height(), resized.into_raw()));
    }
    Some((img.width, img.height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedural_meshes_have_valid_indices_and_bounds() {
        for mesh in [
            cube([1.0, 2.0, 3.0]),
            sphere(0.5, 8, 12),
            cylinder(0.4, 1.2, 12),
            plane(2.0, 2.0),
            ground_grid(2.0, 0.5),
        ] {
            assert!(!mesh.vertices.is_empty());
            assert!(!mesh.indices.is_empty());
            assert_eq!(mesh.indices.len() % 3, 0);
            for &idx in &mesh.indices {
                assert!((idx as usize) < mesh.vertices.len());
            }
            assert!(mesh.bounds.0[0] <= mesh.bounds.1[0]);
        }
    }

    #[cfg(feature = "gltf")]
    #[test]
    fn loads_minimal_synthetic_glb() {
        // Minimal valid GLB containing one triangle (positions only).
        let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let mut bin = Vec::new();
        for f in positions {
            bin.extend_from_slice(&f.to_le_bytes());
        }
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],"buffers":[{{"byteLength":{}}}],"bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":{}}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}}]}}"#,
            bin.len(),
            bin.len()
        );
        let mut json_bytes = json.into_bytes();
        while json_bytes.len() % 4 != 0 {
            json_bytes.push(b' ');
        }
        let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
        let mut glb = Vec::with_capacity(total);
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&(total as u32).to_le_bytes());
        glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
        glb.extend_from_slice(&json_bytes);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        glb.extend_from_slice(&bin);

        let meshes = load_glb_bytes(&glb).expect("load synthetic glb");
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].vertices.len(), 3);
        assert_eq!(meshes[0].indices, vec![0, 1, 2]);
    }
}
