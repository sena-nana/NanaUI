//! Host-texture slot naming for canvas / video GPU media nodes.
//!
//! Split out of `tree.rs`; the slot contract is shared with the canvas and
//! video GPU bridges (`crate::canvas_gpu`, `crate::video`).
use nana_ui_runtime::{CustomRenderNode, HOST_TEXTURE_RENDERER};

pub(crate) fn host_texture_content(slot: String, revision: u64) -> CustomRenderNode {
    CustomRenderNode::new(HOST_TEXTURE_RENDERER, slot, revision)
}

pub(crate) fn canvas_host_texture_slot(id: &str) -> Option<String> {
    // Slot contract: `data-nana-canvas="{id}"` → CustomRenderNode
    // renderer `"nana.host-texture"` / resource `"canvas:{id}"`.
    // Canvas 2D pixels come from nana-ui-web-api (tiny-skia), uploaded by
    // CanvasGpuBridge. This is not a browser `<canvas>` and does not imply
    // a working 2D context on the node by itself.
    id.parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .map(|id| format!("canvas:{id}"))
}

pub(crate) fn video_host_texture_slot(id: &str) -> Option<String> {
    // Slot contract: `data-nana-video="{id}"` → CustomRenderNode
    // renderer `"nana.host-texture"` / resource `"video:{id}"`. Video
    // pixels are pushed by the host through the shared video surface API
    // and uploaded by VideoGpuBridge. This is not a browser `<video>`;
    // playback truth (decoder, clock) stays with the host.
    id.parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .map(|id| format!("video:{id}"))
}
