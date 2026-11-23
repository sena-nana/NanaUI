//! Optional lightweight 3D preview viewport for NanaUI.
//!
//! Renders a 3D [`Scene3d`] from an [`OrbitCamera`] into an offscreen
//! 4x MSAA + `Depth32Float` target (outputting premultiplied linear scRGB)
//! and publishes it to a NanaUI [`nana_ui::TextureSlot`] for display inside a
//! [`nana_ui::runtime::GpuTextureView`] node.

#![forbid(unsafe_code)]

pub mod camera;
pub mod mesh;
pub mod renderer;
pub mod scene;

pub use camera::{Hit3d, OrbitCamera, Ray};
pub use mesh::{MeshData, Vertex3d, cube, cylinder, ground_grid, plane, sphere};
#[cfg(feature = "gltf")]
pub use mesh::{load_glb_bytes, load_glb_file};
pub use renderer::{Viewport3dError, Viewport3dRenderer};
pub use scene::{Instance3d, LightRig, Material3d, Scene3d};
