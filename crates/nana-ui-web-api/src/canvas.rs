//! CPU-addressable Canvas2D and browser-style image resource service.
//!
//! This module deliberately owns pixels, not windows or GPU devices. A host
//! may upload [`CanvasBitmap`] dirty revisions to its existing WGPU queue.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use fontdue::{Font, FontSettings};
use image::{DynamicImage, ImageFormat};
use nana_js_engine::{HostApiRegistry, HostValue, JsException};
use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, GradientStop, IntRect, LineCap, LineJoin,
    LinearGradient, Mask, Paint, Path, PathBuilder, Pattern, Pixmap, PixmapPaint, Point,
    RadialGradient, Rect, Shader, SpreadMode, Stroke, StrokeDash, Transform,
};

const DEFAULT_WIDTH: u32 = 300;
const DEFAULT_HEIGHT: u32 = 150;
const FONT_BYTES: &[u8] = include_bytes!("../../nana-ui/assets/fonts/NotoSansSC-Regular.ttf");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanvasId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasResourceKind {
    Canvas,
    Image,
    ImageBitmap,
    Blob,
}

impl CanvasResourceKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Canvas => "canvas",
            Self::Image => "image",
            Self::ImageBitmap => "image-bitmap",
            Self::Blob => "blob",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasBitmap {
    pub id: CanvasId,
    pub kind: CanvasResourceKind,
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA8 pixels, matching ImageData and queue.writeTexture.
    pub rgba: Vec<u8>,
    pub version: u64,
    /// Inclusive origin plus size of the region mutated since the previous version.
    pub dirty: Option<(u32, u32, u32, u32)>,
}

/// Packed premultiplied RGBA8 pixels ready for a host-owned WGPU texture
/// update. `bytes` contains only `dirty_width * dirty_height` pixels and is
/// consumed exactly once by the hosted Canvas compositor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasUpload {
    pub id: CanvasId,
    pub width: u32,
    pub height: u32,
    pub version: u64,
    pub dirty_x: u32,
    pub dirty_y: u32,
    pub dirty_width: u32,
    pub dirty_height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasError(String);

impl CanvasError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CanvasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CanvasError {}

#[derive(Debug, Clone)]
enum PaintStyle {
    Color(Color),
    Linear {
        start: Point,
        end: Point,
        stops: Vec<(f32, Color)>,
    },
    Radial {
        start: Point,
        start_radius: f32,
        end: Point,
        end_radius: f32,
        stops: Vec<(f32, Color)>,
    },
    Pattern {
        pixmap: Pixmap,
        repeat_x: bool,
        repeat_y: bool,
        transform: Transform,
    },
}

impl Default for PaintStyle {
    fn default() -> Self {
        Self::Color(Color::BLACK)
    }
}

impl PaintStyle {
    fn paint(&self, alpha: f32, blend_mode: BlendMode) -> Paint<'_> {
        let shader = match self {
            Self::Color(color) => Shader::SolidColor(color_with_alpha(*color, alpha)),
            Self::Linear { start, end, stops } => LinearGradient::new(
                *start,
                *end,
                alpha_stops(stops, alpha),
                SpreadMode::Pad,
                Transform::identity(),
            )
            .unwrap_or(Shader::SolidColor(Color::TRANSPARENT)),
            Self::Radial {
                start,
                start_radius,
                end,
                end_radius,
                stops,
            } => RadialGradient::new(
                *start,
                *end,
                (*end_radius - *start_radius).abs().max(f32::EPSILON),
                alpha_stops(stops, alpha),
                SpreadMode::Pad,
                Transform::identity(),
            )
            .unwrap_or(Shader::SolidColor(Color::TRANSPARENT)),
            Self::Pattern {
                pixmap,
                repeat_x,
                repeat_y,
                transform,
            } => {
                // tiny-skia cannot clamp only one axis. Clamp is the closest
                // bounded behavior for no-repeat; repeated patterns use Repeat.
                let spread = if *repeat_x || *repeat_y {
                    SpreadMode::Repeat
                } else {
                    SpreadMode::Pad
                };
                Pattern::new(
                    pixmap.as_ref(),
                    spread,
                    FilterQuality::Bilinear,
                    alpha,
                    *transform,
                )
            }
        };
        Paint {
            shader,
            blend_mode,
            anti_alias: true,
            force_hq_pipeline: false,
        }
    }
}

#[derive(Debug, Clone)]
struct CanvasState {
    fill_style: PaintStyle,
    stroke_style: PaintStyle,
    line_width: f32,
    line_cap: LineCap,
    line_join: LineJoin,
    line_dash: Vec<f32>,
    line_dash_offset: f32,
    global_alpha: f32,
    composite: BlendMode,
    transform: Transform,
    clip: Option<Mask>,
    font_size: f32,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            fill_style: PaintStyle::default(),
            stroke_style: PaintStyle::default(),
            line_width: 1.0,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            line_dash: Vec::new(),
            line_dash_offset: 0.0,
            global_alpha: 1.0,
            composite: BlendMode::SourceOver,
            transform: Transform::identity(),
            clip: None,
            font_size: 10.0,
        }
    }
}

#[derive(Debug, Clone)]
enum PathCommand {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Rect(f32, f32, f32, f32),
    Ellipse(f32, f32, f32, f32, f32, f32, f32, bool),
    Close,
}

#[derive(Debug)]
struct CanvasSurface {
    pixmap: Pixmap,
    state: CanvasState,
    stack: Vec<CanvasState>,
    path: Vec<PathCommand>,
    version: u64,
    dirty: Option<(u32, u32, u32, u32)>,
}

impl CanvasSurface {
    fn new(width: u32, height: u32) -> Result<Self, CanvasError> {
        let pixmap = Pixmap::new(valid_dimension(width), valid_dimension(height))
            .ok_or_else(|| CanvasError::new("canvas dimensions exceed raster limits"))?;
        Ok(Self {
            pixmap,
            state: CanvasState::default(),
            stack: Vec::new(),
            path: Vec::new(),
            version: 1,
            dirty: Some((0, 0, valid_dimension(width), valid_dimension(height))),
        })
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), CanvasError> {
        *self = Self::new(width, height)?;
        Ok(())
    }

    fn touch(&mut self) {
        self.touch_rect(0, 0, self.pixmap.width(), self.pixmap.height());
    }

    fn touch_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
        self.version = self.version.saturating_add(1);
        self.dirty = Some(union_dirty(
            self.dirty,
            x,
            y,
            width.max(1),
            height.max(1),
            self.pixmap.width(),
            self.pixmap.height(),
        ));
    }

    fn touch_filled_rect(&mut self, rect: Rect) {
        if self.state.transform.is_identity() {
            self.touch_rect(
                rect.left().max(0.0).floor() as u32,
                rect.top().max(0.0).floor() as u32,
                rect.width().ceil() as u32,
                rect.height().ceil() as u32,
            );
            return;
        }
        let mut corners = [
            Point::from_xy(rect.left(), rect.top()),
            Point::from_xy(rect.right(), rect.top()),
            Point::from_xy(rect.right(), rect.bottom()),
            Point::from_xy(rect.left(), rect.bottom()),
        ];
        self.state.transform.map_points(&mut corners);
        let left = corners
            .iter()
            .map(|point| point.x)
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as u32;
        let top = corners
            .iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as u32;
        let right = corners
            .iter()
            .map(|point| point.x)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .max(0.0) as u32;
        let bottom = corners
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .max(0.0) as u32;
        self.touch_rect(
            left,
            top,
            right.saturating_sub(left),
            bottom.saturating_sub(top),
        );
    }
}

fn union_dirty(
    current: Option<(u32, u32, u32, u32)>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    max_width: u32,
    max_height: u32,
) -> (u32, u32, u32, u32) {
    let x2 = x.saturating_add(width).min(max_width);
    let y2 = y.saturating_add(height).min(max_height);
    let x = x.min(max_width);
    let y = y.min(max_height);
    match current {
        Some((cx, cy, cw, ch)) => {
            let nx = cx.min(x);
            let ny = cy.min(y);
            let nx2 = cx.saturating_add(cw).max(x2);
            let ny2 = cy.saturating_add(ch).max(y2);
            (nx, ny, nx2.saturating_sub(nx), ny2.saturating_sub(ny))
        }
        None => (x, y, x2.saturating_sub(x), y2.saturating_sub(y)),
    }
}

#[derive(Debug, Clone)]
struct BinaryResource {
    kind: CanvasResourceKind,
    mime: String,
    bytes: Vec<u8>,
    bitmap: Option<CanvasBitmap>,
}

pub struct CanvasRuntime {
    next_id: u64,
    next_object_url: u64,
    canvases: HashMap<CanvasId, CanvasSurface>,
    resources: HashMap<CanvasId, BinaryResource>,
    references: HashMap<CanvasId, usize>,
    object_urls: HashMap<String, CanvasId>,
    font: Font,
}

impl fmt::Debug for CanvasRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CanvasRuntime")
            .field("canvases", &self.canvases.len())
            .field("resources", &self.resources.len())
            .field("object_urls", &self.object_urls.len())
            .finish()
    }
}

impl Default for CanvasRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl CanvasRuntime {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            next_object_url: 1,
            canvases: HashMap::new(),
            resources: HashMap::new(),
            references: HashMap::new(),
            object_urls: HashMap::new(),
            font: Font::from_bytes(FONT_BYTES, FontSettings::default())
                .expect("bundled NanaUI font must be valid"),
        }
    }

    fn allocate_id(&mut self) -> CanvasId {
        let id = CanvasId(self.next_id);
        self.next_id = self.next_id.saturating_add(1).max(1);
        id
    }

    pub fn create_canvas(&mut self, width: u32, height: u32) -> Result<CanvasId, CanvasError> {
        let id = self.allocate_id();
        self.canvases.insert(id, CanvasSurface::new(width, height)?);
        self.references.insert(id, 1);
        Ok(id)
    }

    pub fn resize_canvas(
        &mut self,
        id: CanvasId,
        width: u32,
        height: u32,
    ) -> Result<(), CanvasError> {
        self.canvas_mut(id)?.resize(width, height)
    }

    pub fn release(&mut self, id: CanvasId) -> bool {
        let Some(references) = self.references.get_mut(&id) else {
            return false;
        };
        if *references > 1 {
            *references -= 1;
            return true;
        }
        self.references.remove(&id);
        self.canvases.remove(&id);
        self.resources.remove(&id);
        true
    }

    fn retain(&mut self, id: CanvasId) -> bool {
        let Some(references) = self.references.get_mut(&id) else {
            return false;
        };
        *references = references.saturating_add(1);
        true
    }

    fn create_object_url(&mut self, id: CanvasId) -> Result<String, CanvasError> {
        if !self.retain(id) {
            return Err(CanvasError::new("resource not found"));
        }
        let token = self.next_object_url;
        self.next_object_url = self.next_object_url.saturating_add(1).max(1);
        let url = format!("blob:nana/{}/{}", id.0, token);
        self.object_urls.insert(url.clone(), id);
        Ok(url)
    }

    fn revoke_object_url(&mut self, url: &str) -> bool {
        let Some(id) = self.object_urls.remove(url) else {
            return false;
        };
        self.release(id)
    }

    fn object_url_bytes(&self, url: &str) -> Result<Vec<u8>, CanvasError> {
        let id = self
            .object_urls
            .get(url)
            .copied()
            .ok_or_else(|| CanvasError::new("object URL is revoked or unknown"))?;
        if let Some(resource) = self.resources.get(&id) {
            return Ok(resource.bytes.clone());
        }
        Ok(self.bitmap(id)?.rgba)
    }

    pub fn contains(&self, id: CanvasId) -> bool {
        self.canvases.contains_key(&id) || self.resources.contains_key(&id)
    }

    pub fn bitmap(&self, id: CanvasId) -> Result<CanvasBitmap, CanvasError> {
        if let Some(canvas) = self.canvases.get(&id) {
            return Ok(CanvasBitmap {
                id,
                kind: CanvasResourceKind::Canvas,
                width: canvas.pixmap.width(),
                height: canvas.pixmap.height(),
                rgba: unpremultiply_rgba(canvas.pixmap.data()),
                version: canvas.version,
                dirty: canvas.dirty,
            });
        }
        self.resources
            .get(&id)
            .and_then(|resource| resource.bitmap.clone())
            .ok_or_else(|| CanvasError::new("resource does not contain image pixels"))
    }

    /// Take the pixels changed after `uploaded_version` and acknowledge that
    /// dirty region for the single shared hosted compositor. Immutable image
    /// resources return one full upload and then remain version-stable.
    pub fn take_upload(
        &mut self,
        id: CanvasId,
        uploaded_version: Option<u64>,
    ) -> Result<Option<CanvasUpload>, CanvasError> {
        if let Some(canvas) = self.canvases.get_mut(&id) {
            if uploaded_version == Some(canvas.version) {
                return Ok(None);
            }
            let width = canvas.pixmap.width();
            let height = canvas.pixmap.height();
            let (x, y, dirty_width, dirty_height) =
                canvas.dirty.take().unwrap_or((0, 0, width, height));
            let bytes =
                copy_rgba_region(canvas.pixmap.data(), width, x, y, dirty_width, dirty_height);
            return Ok(Some(CanvasUpload {
                id,
                width,
                height,
                version: canvas.version,
                dirty_x: x,
                dirty_y: y,
                dirty_width,
                dirty_height,
                bytes,
            }));
        }

        let bitmap = self
            .resources
            .get(&id)
            .and_then(|resource| resource.bitmap.as_ref())
            .ok_or_else(|| CanvasError::new("resource does not contain image pixels"))?;
        if uploaded_version == Some(bitmap.version) {
            return Ok(None);
        }
        let mut bytes = bitmap.rgba.clone();
        premultiply_rgba_in_place(&mut bytes);
        Ok(Some(CanvasUpload {
            id,
            width: bitmap.width,
            height: bitmap.height,
            version: bitmap.version,
            dirty_x: 0,
            dirty_y: 0,
            dirty_width: bitmap.width,
            dirty_height: bitmap.height,
            bytes,
        }))
    }

    pub fn version(&self, id: CanvasId) -> Option<u64> {
        self.canvases
            .get(&id)
            .map(|canvas| canvas.version)
            .or_else(|| {
                self.resources
                    .get(&id)
                    .and_then(|resource| resource.bitmap.as_ref().map(|bitmap| bitmap.version))
            })
    }

    pub fn active_resource_count(&self) -> usize {
        self.canvases.len() + self.resources.len()
    }

    fn canvas_mut(&mut self, id: CanvasId) -> Result<&mut CanvasSurface, CanvasError> {
        self.canvases
            .get_mut(&id)
            .ok_or_else(|| CanvasError::new(format!("unknown canvas {}", id.0)))
    }

    fn descriptor(&self, id: CanvasId, kind: CanvasResourceKind) -> HostValue {
        let (width, height, version) = self
            .bitmap(id)
            .map(|bitmap| (bitmap.width, bitmap.height, bitmap.version))
            .unwrap_or((0, 0, 1));
        resource_descriptor(id, kind, width, height, version, None)
    }

    fn decode_image(&mut self, bytes: Vec<u8>) -> Result<CanvasId, CanvasError> {
        if looks_like_svg(&bytes) {
            return self.store_decoded_svg(bytes, CanvasResourceKind::Image);
        }
        let decoded = image::load_from_memory(&bytes)
            .map_err(|error| CanvasError::new(format!("image decode failed: {error}")))?;
        self.store_decoded_image(bytes, decoded, CanvasResourceKind::Image)
    }

    fn store_decoded_svg(
        &mut self,
        bytes: Vec<u8>,
        kind: CanvasResourceKind,
    ) -> Result<CanvasId, CanvasError> {
        let raster = nana_svg_raster::rasterize_document(&bytes)
            .ok_or_else(|| CanvasError::new("SVG decode failed"))?;
        let id = self.allocate_id();
        let width = raster.width;
        let height = raster.height;
        let bitmap = CanvasBitmap {
            id,
            kind,
            width,
            height,
            // Document raster is straight alpha; CanvasBitmap is premultiplied
            // once when uploaded to WGPU.
            rgba: raster.rgba.to_vec(),
            version: 1,
            dirty: Some((0, 0, width, height)),
        };
        self.resources.insert(
            id,
            BinaryResource {
                kind,
                mime: "image/svg+xml".into(),
                bytes,
                bitmap: Some(bitmap),
            },
        );
        self.references.insert(id, 1);
        Ok(id)
    }

    fn store_decoded_image(
        &mut self,
        bytes: Vec<u8>,
        decoded: DynamicImage,
        kind: CanvasResourceKind,
    ) -> Result<CanvasId, CanvasError> {
        let rgba = decoded.to_rgba8();
        let id = self.allocate_id();
        let width = rgba.width();
        let height = rgba.height();
        let bitmap = CanvasBitmap {
            id,
            kind,
            width,
            height,
            rgba: rgba.into_raw(),
            version: 1,
            dirty: Some((0, 0, width, height)),
        };
        self.resources.insert(
            id,
            BinaryResource {
                kind,
                mime: sniff_image_mime(&bytes).into(),
                bytes,
                bitmap: Some(bitmap),
            },
        );
        self.references.insert(id, 1);
        Ok(id)
    }

    fn create_image_bitmap(
        &mut self,
        source: CanvasId,
        options: Option<&BTreeMap<String, HostValue>>,
    ) -> Result<CanvasId, CanvasError> {
        let mut bitmap = match self.bitmap(source) {
            Ok(bitmap) => bitmap,
            Err(_) => {
                let bytes = self
                    .resources
                    .get(&source)
                    .map(|resource| resource.bytes.clone())
                    .ok_or_else(|| CanvasError::new("unknown ImageBitmap source"))?;
                let decoded = image::load_from_memory(&bytes)
                    .map_err(|error| {
                        CanvasError::new(format!("ImageBitmap decode failed: {error}"))
                    })?
                    .to_rgba8();
                let width = decoded.width();
                let height = decoded.height();
                CanvasBitmap {
                    id: source,
                    kind: CanvasResourceKind::ImageBitmap,
                    width,
                    height,
                    rgba: decoded.into_raw(),
                    version: 1,
                    dirty: Some((0, 0, width, height)),
                }
            }
        };
        if let Some(options) = options {
            let has_crop = options.contains_key("sw") && options.contains_key("sh");
            let mut image = image::RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.rgba)
                .ok_or_else(|| CanvasError::new("invalid ImageBitmap pixels"))?;
            if has_crop {
                let mut sx = f64_field(options, "sx", 0.0) as i64;
                let mut sy = f64_field(options, "sy", 0.0) as i64;
                let mut sw = f64_field(options, "sw", bitmap.width as f64) as i64;
                let mut sh = f64_field(options, "sh", bitmap.height as f64) as i64;
                if sw < 0 {
                    sx += sw;
                    sw = -sw;
                }
                if sh < 0 {
                    sy += sh;
                    sh = -sh;
                }
                let left = sx.clamp(0, i64::from(bitmap.width)) as u32;
                let top = sy.clamp(0, i64::from(bitmap.height)) as u32;
                let width = (sw as u32).min(bitmap.width.saturating_sub(left));
                let height = (sh as u32).min(bitmap.height.saturating_sub(top));
                if width == 0 || height == 0 {
                    return Err(CanvasError::new("ImageBitmap crop is outside source"));
                }
                image = image::imageops::crop_imm(&image, left, top, width, height).to_image();
            }
            let resize_width = options
                .get("resizeWidth")
                .and_then(HostValue::as_f64)
                .map(|value| valid_dimension(value as u32));
            let resize_height = options
                .get("resizeHeight")
                .and_then(HostValue::as_f64)
                .map(|value| valid_dimension(value as u32));
            if resize_width.is_some() || resize_height.is_some() {
                let width = resize_width.unwrap_or_else(|| {
                    ((image.width() as f64 * f64::from(resize_height.unwrap())
                        / image.height() as f64)
                        .round() as u32)
                        .max(1)
                });
                let height = resize_height.unwrap_or_else(|| {
                    ((image.height() as f64 * f64::from(resize_width.unwrap())
                        / image.width() as f64)
                        .round() as u32)
                        .max(1)
                });
                let filter = match str_field(options, "resizeQuality", "low") {
                    "high" => image::imageops::FilterType::Lanczos3,
                    "medium" => image::imageops::FilterType::CatmullRom,
                    "pixelated" => image::imageops::FilterType::Nearest,
                    _ => image::imageops::FilterType::Triangle,
                };
                image = image::imageops::resize(&image, width, height, filter);
            }
            bitmap.width = image.width();
            bitmap.height = image.height();
            bitmap.rgba = image.into_raw();
            bitmap.dirty = Some((0, 0, bitmap.width, bitmap.height));
        }
        let id = self.allocate_id();
        bitmap.id = id;
        bitmap.kind = CanvasResourceKind::ImageBitmap;
        self.resources.insert(
            id,
            BinaryResource {
                kind: CanvasResourceKind::ImageBitmap,
                mime: "image/raw-rgba".into(),
                bytes: bitmap.rgba.clone(),
                bitmap: Some(bitmap),
            },
        );
        self.references.insert(id, 1);
        Ok(id)
    }

    fn create_blob(&mut self, bytes: Vec<u8>, mime: String) -> CanvasId {
        let id = self.allocate_id();
        self.resources.insert(
            id,
            BinaryResource {
                kind: CanvasResourceKind::Blob,
                mime,
                bytes,
                bitmap: None,
            },
        );
        self.references.insert(id, 1);
        id
    }

    fn encode_canvas(
        &self,
        id: CanvasId,
        mime: &str,
        quality: f32,
    ) -> Result<Vec<u8>, CanvasError> {
        let bitmap = self.bitmap(id)?;
        let image = image::RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.rgba)
            .ok_or_else(|| CanvasError::new("invalid canvas pixels"))?;
        let dynamic = DynamicImage::ImageRgba8(image);
        let mut cursor = std::io::Cursor::new(Vec::new());
        let format = match mime {
            "image/jpeg" => ImageFormat::Jpeg,
            "image/gif" => ImageFormat::Gif,
            "image/webp" => ImageFormat::WebP,
            _ => ImageFormat::Png,
        };
        let _ = quality;
        dynamic
            .write_to(&mut cursor, format)
            .map_err(|error| CanvasError::new(format!("canvas encode failed: {error}")))?;
        Ok(cursor.into_inner())
    }

    fn command(
        &mut self,
        id: CanvasId,
        operation: &str,
        args: &[HostValue],
    ) -> Result<HostValue, CanvasError> {
        match operation {
            "drawImage" => return self.draw_image(id, args),
            "fillText" | "strokeText" | "measureText" => {
                return self.text_command(id, operation, args);
            }
            _ => {}
        }
        let canvas = self.canvas_mut(id)?;
        match operation {
            "save" => canvas.stack.push(canvas.state.clone()),
            "restore" => {
                if let Some(state) = canvas.stack.pop() {
                    canvas.state = state;
                }
            }
            "beginPath" => canvas.path.clear(),
            "closePath" => canvas.path.push(PathCommand::Close),
            "moveTo" => canvas
                .path
                .push(PathCommand::Move(num(args, 0), num(args, 1))),
            "lineTo" => canvas
                .path
                .push(PathCommand::Line(num(args, 0), num(args, 1))),
            "quadraticCurveTo" => canvas.path.push(PathCommand::Quad(
                num(args, 0),
                num(args, 1),
                num(args, 2),
                num(args, 3),
            )),
            "bezierCurveTo" => canvas.path.push(PathCommand::Cubic(
                num(args, 0),
                num(args, 1),
                num(args, 2),
                num(args, 3),
                num(args, 4),
                num(args, 5),
            )),
            "rect" => canvas.path.push(PathCommand::Rect(
                num(args, 0),
                num(args, 1),
                num(args, 2),
                num(args, 3),
            )),
            "arc" => canvas.path.push(PathCommand::Ellipse(
                num(args, 0),
                num(args, 1),
                num(args, 2),
                num(args, 2),
                0.0,
                num(args, 3),
                num(args, 4),
                bool_arg(args, 5),
            )),
            "ellipse" => canvas.path.push(PathCommand::Ellipse(
                num(args, 0),
                num(args, 1),
                num(args, 2),
                num(args, 3),
                num(args, 4),
                num(args, 5),
                num(args, 6),
                bool_arg(args, 7),
            )),
            "translate" => {
                canvas.state.transform = canvas
                    .state
                    .transform
                    .pre_translate(num(args, 0), num(args, 1));
            }
            "rotate" => {
                canvas.state.transform =
                    canvas.state.transform.pre_rotate(num(args, 0).to_degrees());
            }
            "scale" => {
                canvas.state.transform =
                    canvas.state.transform.pre_scale(num(args, 0), num(args, 1));
            }
            "transform" => {
                canvas.state.transform = canvas.state.transform.pre_concat(Transform::from_row(
                    num(args, 0),
                    num(args, 1),
                    num(args, 2),
                    num(args, 3),
                    num(args, 4),
                    num(args, 5),
                ));
            }
            "setTransform" => {
                canvas.state.transform = Transform::from_row(
                    num(args, 0),
                    num(args, 1),
                    num(args, 2),
                    num(args, 3),
                    num(args, 4),
                    num(args, 5),
                );
            }
            "resetTransform" => canvas.state.transform = Transform::identity(),
            "clearRect" => {
                let Some(rect) =
                    Rect::from_xywh(num(args, 0), num(args, 1), num(args, 2), num(args, 3))
                else {
                    return Ok(HostValue::Null);
                };
                let mut paint = Paint::default();
                paint.blend_mode = BlendMode::Clear;
                canvas.pixmap.fill_rect(
                    rect,
                    &paint,
                    canvas.state.transform,
                    canvas.state.clip.as_ref(),
                );
                canvas.touch_filled_rect(rect);
            }
            "fillRect" | "strokeRect" => {
                let Some(rect) =
                    Rect::from_xywh(num(args, 0), num(args, 1), num(args, 2), num(args, 3))
                else {
                    return Ok(HostValue::Null);
                };
                if operation == "fillRect" {
                    let paint = canvas
                        .state
                        .fill_style
                        .paint(canvas.state.global_alpha, canvas.state.composite);
                    canvas.pixmap.fill_rect(
                        rect,
                        &paint,
                        canvas.state.transform,
                        canvas.state.clip.as_ref(),
                    );
                } else {
                    let mut path = PathBuilder::new();
                    path.push_rect(rect);
                    if let Some(path) = path.finish() {
                        stroke_canvas_path(canvas, &path);
                    }
                }
                if operation == "fillRect" {
                    canvas.touch_filled_rect(rect);
                } else {
                    // Stroke joins, caps, dash patterns, and transformed line width
                    // can extend beyond the author rectangle. Conservatively upload
                    // the full surface until raster bounds are exposed by tiny-skia.
                    canvas.touch();
                }
            }
            "fill" | "stroke" | "clip" => {
                if let Some(path) = build_path(&canvas.path) {
                    if operation == "fill" {
                        let paint = canvas
                            .state
                            .fill_style
                            .paint(canvas.state.global_alpha, canvas.state.composite);
                        canvas.pixmap.fill_path(
                            &path,
                            &paint,
                            FillRule::Winding,
                            canvas.state.transform,
                            canvas.state.clip.as_ref(),
                        );
                        canvas.touch();
                    } else if operation == "stroke" {
                        stroke_canvas_path(canvas, &path);
                        canvas.touch();
                    } else {
                        let mut clip = canvas.state.clip.take().unwrap_or_else(|| {
                            let mut mask = Mask::new(canvas.pixmap.width(), canvas.pixmap.height())
                                .expect("canvas mask");
                            mask.data_mut().fill(255);
                            mask
                        });
                        clip.intersect_path(&path, FillRule::Winding, true, canvas.state.transform);
                        canvas.state.clip = Some(clip);
                    }
                }
            }
            _ => {
                return Err(CanvasError::new(format!(
                    "unsupported canvas operation `{operation}`"
                )));
            }
        }
        Ok(HostValue::Null)
    }

    fn set_state(&mut self, id: CanvasId, key: &str, value: &HostValue) -> Result<(), CanvasError> {
        let pattern_bitmap = if let Some(source_id) = style_source_id(value) {
            Some(self.bitmap(source_id)?)
        } else {
            None
        };
        let canvas = self.canvas_mut(id)?;
        match key {
            "fillStyle" => canvas.state.fill_style = parse_style(value, pattern_bitmap)?,
            "strokeStyle" => canvas.state.stroke_style = parse_style(value, pattern_bitmap)?,
            "lineWidth" => canvas.state.line_width = value.as_f64().unwrap_or(1.0).max(0.0) as f32,
            "lineCap" => canvas.state.line_cap = parse_line_cap(value.as_str().unwrap_or("butt")),
            "lineJoin" => {
                canvas.state.line_join = parse_line_join(value.as_str().unwrap_or("miter"))
            }
            "lineDashOffset" => {
                canvas.state.line_dash_offset = value.as_f64().unwrap_or(0.0) as f32
            }
            "globalAlpha" => {
                canvas.state.global_alpha = value.as_f64().unwrap_or(1.0).clamp(0.0, 1.0) as f32
            }
            "globalCompositeOperation" => {
                canvas.state.composite = parse_blend(value.as_str().unwrap_or("source-over"))
            }
            "font" => {
                canvas.state.font_size =
                    parse_font_size(value.as_str().unwrap_or("10px sans-serif"))
            }
            "lineDash" => {
                canvas.state.line_dash = value
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .map(|v| v.as_f64().unwrap_or(0.0).max(0.0) as f32)
                            .collect()
                    })
                    .unwrap_or_default();
            }
            _ => {
                return Err(CanvasError::new(format!(
                    "unsupported canvas state `{key}`"
                )));
            }
        }
        Ok(())
    }

    fn draw_image(
        &mut self,
        canvas_id: CanvasId,
        args: &[HostValue],
    ) -> Result<HostValue, CanvasError> {
        let source_id = args
            .first()
            .and_then(HostValue::as_u64)
            .map(CanvasId)
            .ok_or_else(|| CanvasError::new("drawImage requires an image resource"))?;
        let source = self.bitmap(source_id)?;
        let (sx, sy, sw, sh, dx, dy, dw, dh) = match args.len() {
            3 => (
                0.0,
                0.0,
                source.width as f32,
                source.height as f32,
                num(args, 1),
                num(args, 2),
                source.width as f32,
                source.height as f32,
            ),
            5 => (
                0.0,
                0.0,
                source.width as f32,
                source.height as f32,
                num(args, 1),
                num(args, 2),
                num(args, 3),
                num(args, 4),
            ),
            _ => (
                num(args, 1),
                num(args, 2),
                num(args, 3),
                num(args, 4),
                num(args, 5),
                num(args, 6),
                num(args, 7),
                num(args, 8),
            ),
        };
        if sw <= 0.0 || sh <= 0.0 || dw == 0.0 || dh == 0.0 {
            return Ok(HostValue::Null);
        }
        let source_pixmap = Pixmap::from_vec(
            premultiply_rgba(source.rgba),
            tiny_skia::IntSize::from_wh(source.width, source.height)
                .ok_or_else(|| CanvasError::new("invalid source dimensions"))?,
        )
        .ok_or_else(|| CanvasError::new("invalid source pixels"))?;
        let crop = source_pixmap
            .clone_rect(
                IntRect::from_xywh(
                    sx.max(0.0) as i32,
                    sy.max(0.0) as i32,
                    sw.min(source.width as f32) as u32,
                    sh.min(source.height as f32) as u32,
                )
                .ok_or_else(|| CanvasError::new("invalid drawImage crop"))?,
            )
            .ok_or_else(|| CanvasError::new("drawImage crop is outside source"))?;
        let canvas = self.canvas_mut(canvas_id)?;
        let transform = canvas
            .state
            .transform
            .pre_concat(Transform::from_translate(dx, dy).pre_scale(dw / sw, dh / sh));
        canvas.pixmap.draw_pixmap(
            0,
            0,
            crop.as_ref(),
            &PixmapPaint {
                opacity: canvas.state.global_alpha,
                blend_mode: canvas.state.composite,
                quality: FilterQuality::Bilinear,
            },
            transform,
            canvas.state.clip.as_ref(),
        );
        canvas.touch();
        Ok(HostValue::Null)
    }

    fn text_command(
        &mut self,
        id: CanvasId,
        operation: &str,
        args: &[HostValue],
    ) -> Result<HostValue, CanvasError> {
        let text = args.first().and_then(HostValue::as_str).unwrap_or_default();
        let canvas = self
            .canvases
            .get(&id)
            .ok_or_else(|| CanvasError::new("unknown canvas"))?;
        let size = canvas.state.font_size.max(1.0);
        let metrics: Vec<_> = text.chars().map(|ch| self.font.metrics(ch, size)).collect();
        let width: f32 = metrics.iter().map(|metric| metric.advance_width).sum();
        if operation == "measureText" {
            return Ok(HostValue::Object(
                [
                    ("width".into(), HostValue::Number(width as f64)),
                    (
                        "actualBoundingBoxAscent".into(),
                        HostValue::Number((size * 0.8) as f64),
                    ),
                    (
                        "actualBoundingBoxDescent".into(),
                        HostValue::Number((size * 0.2) as f64),
                    ),
                ]
                .into_iter()
                .collect(),
            ));
        }
        let x = num(args, 1);
        let y = num(args, 2);
        let mut glyph_mask =
            Mask::new(canvas.pixmap.width(), canvas.pixmap.height()).expect("canvas text mask");
        let transform = canvas.state.transform;
        let mut pen_x = x;
        for ch in text.chars() {
            let (metric, alpha) = self.font.rasterize(ch, size);
            for row in 0..metric.height {
                for col in 0..metric.width {
                    let coverage = alpha[row * metric.width + col];
                    if coverage == 0 {
                        continue;
                    }
                    let mut point = Point::from_xy(
                        pen_x + metric.xmin as f32 + col as f32,
                        y - metric.height as f32 - metric.ymin as f32 + row as f32,
                    );
                    transform.map_point(&mut point);
                    let px = point.x.round() as i32;
                    let py = point.y.round() as i32;
                    if px >= 0
                        && py >= 0
                        && px < glyph_mask.width() as i32
                        && py < glyph_mask.height() as i32
                    {
                        let offset = py as usize * glyph_mask.width() as usize + px as usize;
                        glyph_mask.data_mut()[offset] = glyph_mask.data()[offset].max(coverage);
                    }
                }
            }
            pen_x += metric.advance_width;
        }
        if let Some(clip) = &canvas.state.clip {
            for (value, clip_value) in glyph_mask.data_mut().iter_mut().zip(clip.data()) {
                *value = ((*value as u16 * *clip_value as u16 + 127) / 255) as u8;
            }
        }
        if operation == "strokeText" {
            let radius = (canvas.state.line_width.max(1.0) * 0.5).ceil() as i32;
            let original = glyph_mask.data().to_vec();
            let width = glyph_mask.width() as i32;
            let height = glyph_mask.height() as i32;
            for y in 0..height {
                for x in 0..width {
                    let mut coverage = 0u8;
                    for oy in -radius..=radius {
                        for ox in -radius..=radius {
                            if ox * ox + oy * oy > radius * radius {
                                continue;
                            }
                            let sx = x + ox;
                            let sy = y + oy;
                            if sx >= 0 && sy >= 0 && sx < width && sy < height {
                                coverage = coverage
                                    .max(original[sy as usize * width as usize + sx as usize]);
                            }
                        }
                    }
                    let index = y as usize * width as usize + x as usize;
                    glyph_mask.data_mut()[index] = coverage.saturating_sub(original[index]);
                }
            }
        }
        let canvas = self.canvas_mut(id)?;
        let style = if operation == "strokeText" {
            &canvas.state.stroke_style
        } else {
            &canvas.state.fill_style
        };
        let paint = style.paint(canvas.state.global_alpha, canvas.state.composite);
        let bounds = Rect::from_xywh(
            0.0,
            0.0,
            canvas.pixmap.width() as f32,
            canvas.pixmap.height() as f32,
        )
        .expect("canvas bounds");
        canvas
            .pixmap
            .fill_rect(bounds, &paint, Transform::identity(), Some(&glyph_mask));
        canvas.touch();
        Ok(HostValue::Null)
    }

    fn get_image_data(
        &self,
        id: CanvasId,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, CanvasError> {
        let source = self.bitmap(id)?;
        let mut output = vec![0; width as usize * height as usize * 4];
        for row in 0..height as i32 {
            for col in 0..width as i32 {
                let sx = x + col;
                let sy = y + row;
                if sx < 0 || sy < 0 || sx >= source.width as i32 || sy >= source.height as i32 {
                    continue;
                }
                let src = (sy as usize * source.width as usize + sx as usize) * 4;
                let dst = (row as usize * width as usize + col as usize) * 4;
                output[dst..dst + 4].copy_from_slice(&source.rgba[src..src + 4]);
            }
        }
        Ok(output)
    }

    fn put_image_data(
        &mut self,
        id: CanvasId,
        bytes: &[u8],
        width: u32,
        height: u32,
        dx: i32,
        dy: i32,
    ) -> Result<(), CanvasError> {
        if bytes.len() != width as usize * height as usize * 4 {
            return Err(CanvasError::new("ImageData byte length mismatch"));
        }
        let canvas = self.canvas_mut(id)?;
        let source = premultiply_rgba(bytes.to_vec());
        for row in 0..height as i32 {
            for col in 0..width as i32 {
                let tx = dx + col;
                let ty = dy + row;
                if tx < 0
                    || ty < 0
                    || tx >= canvas.pixmap.width() as i32
                    || ty >= canvas.pixmap.height() as i32
                {
                    continue;
                }
                let src = (row as usize * width as usize + col as usize) * 4;
                let dst = (ty as usize * canvas.pixmap.width() as usize + tx as usize) * 4;
                canvas.pixmap.data_mut()[dst..dst + 4].copy_from_slice(&source[src..src + 4]);
            }
        }
        canvas.touch();
        Ok(())
    }
}

fn copy_rgba_region(
    source: &[u8],
    source_width: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let row_bytes = width as usize * 4;
    let mut bytes = Vec::with_capacity(row_bytes * height as usize);
    for row in y..y.saturating_add(height) {
        let start = (row as usize * source_width as usize + x as usize) * 4;
        bytes.extend_from_slice(&source[start..start + row_bytes]);
    }
    bytes
}

fn premultiply_rgba_in_place(bytes: &mut [u8]) {
    for pixel in bytes.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        pixel[0] = ((u16::from(pixel[0]) * alpha + 127) / 255) as u8;
        pixel[1] = ((u16::from(pixel[1]) * alpha + 127) / 255) as u8;
        pixel[2] = ((u16::from(pixel[2]) * alpha + 127) / 255) as u8;
    }
}

pub type SharedCanvasRuntime = Arc<Mutex<CanvasRuntime>>;

pub fn shared_canvas_runtime() -> SharedCanvasRuntime {
    Arc::new(Mutex::new(CanvasRuntime::new()))
}

pub(crate) fn register_canvas_host_ops(api: &mut HostApiRegistry, runtime: SharedCanvasRuntime) {
    macro_rules! locked {
        ($runtime:expr) => {
            $runtime
                .lock()
                .map_err(|_| JsException::new("canvas runtime poisoned"))?
        };
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("canvasCreate", move |args| {
            let width = dimension(args, 0, DEFAULT_WIDTH);
            let height = dimension(args, 1, DEFAULT_HEIGHT);
            let mut runtime = locked!(runtime);
            let id = runtime.create_canvas(width, height).map_err(js_error)?;
            Ok(runtime.descriptor(id, CanvasResourceKind::Canvas))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("canvasResize", move |args| {
            let id = resource_id(args, 0)?;
            let width = dimension(args, 1, DEFAULT_WIDTH);
            let height = dimension(args, 2, DEFAULT_HEIGHT);
            locked!(runtime)
                .resize_canvas(id, width, height)
                .map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("canvasCommand", move |args| {
            let id = resource_id(args, 0)?;
            let operation = args.get(1).and_then(HostValue::as_str).unwrap_or_default();
            let values = args
                .get(2)
                .and_then(HostValue::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            locked!(runtime)
                .command(id, operation, values)
                .map_err(js_error)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("canvasSetState", move |args| {
            let id = resource_id(args, 0)?;
            let key = args.get(1).and_then(HostValue::as_str).unwrap_or_default();
            let value = args.get(2).unwrap_or(&HostValue::Null);
            locked!(runtime)
                .set_state(id, key, value)
                .map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("canvasGetImageData", move |args| {
            let id = resource_id(args, 0)?;
            let width = dimension(args, 3, 1);
            let height = dimension(args, 4, 1);
            let data = locked!(runtime)
                .get_image_data(id, num_i(args, 1), num_i(args, 2), width, height)
                .map_err(js_error)?;
            Ok(HostValue::Object(
                [
                    ("width".into(), HostValue::Number(width as f64)),
                    ("height".into(), HostValue::Number(height as f64)),
                    ("data".into(), HostValue::Bytes(data)),
                ]
                .into_iter()
                .collect(),
            ))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("canvasPutImageData", move |args| {
            let id = resource_id(args, 0)?;
            let bytes = args
                .get(1)
                .and_then(HostValue::as_bytes)
                .ok_or_else(|| JsException::new("putImageData requires bytes"))?;
            locked!(runtime)
                .put_image_data(
                    id,
                    bytes,
                    dimension(args, 2, 1),
                    dimension(args, 3, 1),
                    num_i(args, 4),
                    num_i(args, 5),
                )
                .map_err(js_error)?;
            Ok(HostValue::Null)
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("canvasEncode", move |args| {
            let id = resource_id(args, 0)?;
            let mime = args
                .get(1)
                .and_then(HostValue::as_str)
                .unwrap_or("image/png");
            let quality = args.get(2).and_then(HostValue::as_f64).unwrap_or(0.92) as f32;
            Ok(HostValue::Bytes(
                locked!(runtime)
                    .encode_canvas(id, mime, quality)
                    .map_err(js_error)?,
            ))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("imageDecode", move |args| {
            let bytes = args
                .first()
                .and_then(HostValue::as_bytes)
                .ok_or_else(|| JsException::new("imageDecode requires bytes"))?
                .to_vec();
            let mut runtime = locked!(runtime);
            let id = runtime.decode_image(bytes).map_err(js_error)?;
            Ok(runtime.descriptor(id, CanvasResourceKind::Image))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("imageBitmapCreate", move |args| {
            let source = resource_id(args, 0)?;
            let mut runtime = locked!(runtime);
            let id = runtime
                .create_image_bitmap(source, args.get(1).and_then(HostValue::as_object))
                .map_err(js_error)?;
            Ok(runtime.descriptor(id, CanvasResourceKind::ImageBitmap))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("blobCreate", move |args| {
            let bytes = args
                .first()
                .and_then(HostValue::as_bytes)
                .unwrap_or_default()
                .to_vec();
            let mime = args
                .get(1)
                .and_then(HostValue::as_str)
                .unwrap_or_default()
                .to_string();
            let mut runtime = locked!(runtime);
            let id = runtime.create_blob(bytes, mime.clone());
            let size = runtime
                .resources
                .get(&id)
                .map(|resource| resource.bytes.len())
                .unwrap_or(0);
            Ok(resource_descriptor(
                id,
                CanvasResourceKind::Blob,
                0,
                0,
                1,
                Some((&mime, size)),
            ))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("resourceBytes", move |args| {
            let id = resource_id(args, 0)?;
            let runtime = locked!(runtime);
            if let Some(resource) = runtime.resources.get(&id) {
                return Ok(HostValue::Bytes(resource.bytes.clone()));
            }
            Ok(HostValue::Bytes(runtime.bitmap(id).map_err(js_error)?.rgba))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("resourceInfo", move |args| {
            let id = resource_id(args, 0)?;
            let runtime = locked!(runtime);
            if let Some(resource) = runtime.resources.get(&id) {
                return Ok(HostValue::Object(
                    [
                        ("id".into(), HostValue::BigInt(id.0)),
                        (
                            "kind".into(),
                            HostValue::String(resource.kind.as_str().into()),
                        ),
                        ("type".into(), HostValue::String(resource.mime.clone())),
                        (
                            "size".into(),
                            HostValue::Number(resource.bytes.len() as f64),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ));
            }
            let bitmap = runtime.bitmap(id).map_err(js_error)?;
            Ok(resource_descriptor(
                id,
                bitmap.kind,
                bitmap.width,
                bitmap.height,
                bitmap.version,
                None,
            ))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("resourceRelease", move |args| {
            Ok(HostValue::Bool(
                locked!(runtime).release(resource_id(args, 0)?),
            ))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("objectUrlCreate", move |args| {
            let id = resource_id(args, 0)?;
            let mut runtime = locked!(runtime);
            Ok(HostValue::String(
                runtime.create_object_url(id).map_err(js_error)?,
            ))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("objectUrlRevoke", move |args| {
            let url = args.first().and_then(HostValue::as_str).unwrap_or_default();
            Ok(HostValue::Bool(locked!(runtime).revoke_object_url(url)))
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        api.register("objectUrlBytes", move |args| {
            let url = args.first().and_then(HostValue::as_str).unwrap_or_default();
            Ok(HostValue::Bytes(
                locked!(runtime).object_url_bytes(url).map_err(js_error)?,
            ))
        });
    }
    {
        api.register("dataUrlFromBytes", move |args| {
            let bytes = args
                .first()
                .and_then(HostValue::as_bytes)
                .unwrap_or_default();
            let mime = args
                .get(1)
                .and_then(HostValue::as_str)
                .unwrap_or("application/octet-stream");
            Ok(HostValue::String(format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            )))
        });
    }
}

fn resource_descriptor(
    id: CanvasId,
    kind: CanvasResourceKind,
    width: u32,
    height: u32,
    version: u64,
    blob: Option<(&str, usize)>,
) -> HostValue {
    let mut values: BTreeMap<String, HostValue> = [
        ("__nanaResource".into(), HostValue::Bool(true)),
        ("id".into(), HostValue::BigInt(id.0)),
        ("kind".into(), HostValue::String(kind.as_str().into())),
        ("width".into(), HostValue::Number(width as f64)),
        ("height".into(), HostValue::Number(height as f64)),
        ("version".into(), HostValue::Number(version as f64)),
    ]
    .into_iter()
    .collect();
    if let Some((mime, size)) = blob {
        values.insert("type".into(), HostValue::String(mime.into()));
        values.insert("size".into(), HostValue::Number(size as f64));
    }
    HostValue::Object(values)
}

fn stroke_canvas_path(canvas: &mut CanvasSurface, path: &Path) {
    let paint = canvas
        .state
        .stroke_style
        .paint(canvas.state.global_alpha, canvas.state.composite);
    let dash = (!canvas.state.line_dash.is_empty())
        .then(|| {
            StrokeDash::new(
                canvas.state.line_dash.clone(),
                canvas.state.line_dash_offset,
            )
        })
        .flatten();
    let stroke = Stroke {
        width: canvas.state.line_width,
        line_cap: canvas.state.line_cap,
        line_join: canvas.state.line_join,
        dash,
        ..Stroke::default()
    };
    canvas.pixmap.stroke_path(
        path,
        &paint,
        &stroke,
        canvas.state.transform,
        canvas.state.clip.as_ref(),
    );
}

fn build_path(commands: &[PathCommand]) -> Option<Path> {
    let mut builder = PathBuilder::new();
    for command in commands {
        match *command {
            PathCommand::Move(x, y) => builder.move_to(x, y),
            PathCommand::Line(x, y) => builder.line_to(x, y),
            PathCommand::Quad(x1, y1, x, y) => builder.quad_to(x1, y1, x, y),
            PathCommand::Cubic(x1, y1, x2, y2, x, y) => builder.cubic_to(x1, y1, x2, y2, x, y),
            PathCommand::Rect(x, y, w, h) => {
                if let Some(rect) = Rect::from_xywh(x, y, w, h) {
                    builder.push_rect(rect);
                }
            }
            PathCommand::Ellipse(cx, cy, rx, ry, rotation, start, end, anticlockwise) => {
                append_ellipse_arc(
                    &mut builder,
                    cx,
                    cy,
                    rx,
                    ry,
                    rotation,
                    start,
                    end,
                    anticlockwise,
                )
            }
            PathCommand::Close => builder.close(),
        }
    }
    builder.finish()
}

#[allow(clippy::too_many_arguments)]
fn append_ellipse_arc(
    builder: &mut PathBuilder,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    rotation: f32,
    start: f32,
    end: f32,
    anticlockwise: bool,
) {
    if rx < 0.0 || ry < 0.0 {
        return;
    }
    let tau = std::f32::consts::TAU;
    let mut sweep = end - start;
    if !anticlockwise {
        while sweep < 0.0 {
            sweep += tau;
        }
    } else {
        while sweep > 0.0 {
            sweep -= tau;
        }
    }
    sweep = sweep.clamp(-tau, tau);
    let steps = ((sweep.abs() * rx.max(ry).sqrt()).ceil() as usize).clamp(8, 128);
    let cos_r = rotation.cos();
    let sin_r = rotation.sin();
    for index in 0..=steps {
        let angle = start + sweep * index as f32 / steps as f32;
        let x = rx * angle.cos();
        let y = ry * angle.sin();
        let px = cx + x * cos_r - y * sin_r;
        let py = cy + x * sin_r + y * cos_r;
        builder.line_to(px, py);
    }
}

fn parse_style(
    value: &HostValue,
    pattern_bitmap: Option<CanvasBitmap>,
) -> Result<PaintStyle, CanvasError> {
    if let Some(color) = value.as_str() {
        return Ok(PaintStyle::Color(parse_color(color)));
    }
    let object = value
        .as_object()
        .ok_or_else(|| CanvasError::new("paint style must be a color, gradient, or pattern"))?;
    let kind = object
        .get("kind")
        .and_then(HostValue::as_str)
        .unwrap_or_default();
    let args = object
        .get("args")
        .and_then(HostValue::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let stops = object
        .get("stops")
        .and_then(HostValue::as_array)
        .map(|stops| {
            stops
                .iter()
                .filter_map(|stop| {
                    let stop = stop.as_array()?;
                    Some((
                        stop.first()?.as_f64()? as f32,
                        parse_color(stop.get(1)?.as_str()?),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    match kind {
        "linear" => Ok(PaintStyle::Linear {
            start: Point::from_xy(num(args, 0), num(args, 1)),
            end: Point::from_xy(num(args, 2), num(args, 3)),
            stops,
        }),
        "radial" => Ok(PaintStyle::Radial {
            start: Point::from_xy(num(args, 0), num(args, 1)),
            start_radius: num(args, 2),
            end: Point::from_xy(num(args, 3), num(args, 4)),
            end_radius: num(args, 5),
            stops,
        }),
        "pattern" => {
            let repetition = object
                .get("repetition")
                .and_then(HostValue::as_str)
                .unwrap_or("repeat");
            let bitmap =
                pattern_bitmap.ok_or_else(|| CanvasError::new("pattern source is unavailable"))?;
            let pixmap = Pixmap::from_vec(
                premultiply_rgba(bitmap.rgba),
                tiny_skia::IntSize::from_wh(bitmap.width, bitmap.height)
                    .ok_or_else(|| CanvasError::new("invalid pattern dimensions"))?,
            )
            .ok_or_else(|| CanvasError::new("invalid pattern pixels"))?;
            Ok(PaintStyle::Pattern {
                pixmap,
                repeat_x: matches!(repetition, "repeat" | "repeat-x"),
                repeat_y: matches!(repetition, "repeat" | "repeat-y"),
                transform: object
                    .get("transform")
                    .and_then(HostValue::as_array)
                    .filter(|values| values.len() >= 6)
                    .map(|values| {
                        Transform::from_row(
                            num(values, 0),
                            num(values, 1),
                            num(values, 2),
                            num(values, 3),
                            num(values, 4),
                            num(values, 5),
                        )
                    })
                    .unwrap_or_else(Transform::identity),
            })
        }
        _ => Err(CanvasError::new("unknown paint style")),
    }
}

fn style_source_id(value: &HostValue) -> Option<CanvasId> {
    value.as_object()?.get("sourceId")?.as_u64().map(CanvasId)
}

fn alpha_stops(stops: &[(f32, Color)], alpha: f32) -> Vec<GradientStop> {
    stops
        .iter()
        .map(|(position, color)| GradientStop::new(*position, color_with_alpha(*color, alpha)))
        .collect()
}

fn color_with_alpha(color: Color, alpha: f32) -> Color {
    Color::from_rgba(
        color.red(),
        color.green(),
        color.blue(),
        color.alpha() * alpha.clamp(0.0, 1.0),
    )
    .unwrap_or(Color::TRANSPARENT)
}

fn parse_color(value: &str) -> Color {
    let value = value.trim().to_ascii_lowercase();
    match value.as_str() {
        "transparent" => return Color::TRANSPARENT,
        "black" => return Color::BLACK,
        "white" => return Color::WHITE,
        "red" => return Color::from_rgba8(255, 0, 0, 255),
        "green" => return Color::from_rgba8(0, 128, 0, 255),
        "blue" => return Color::from_rgba8(0, 0, 255, 255),
        _ => {}
    }
    if let Some(hex) = value.strip_prefix('#') {
        let expanded = match hex.len() {
            3 | 4 => hex.chars().flat_map(|ch| [ch, ch]).collect::<String>(),
            _ => hex.to_string(),
        };
        if (expanded.len() == 6 || expanded.len() == 8)
            && let Ok(raw) = u32::from_str_radix(&expanded, 16)
        {
            let (r, g, b, a) = if expanded.len() == 8 {
                (
                    (raw >> 24) as u8,
                    (raw >> 16) as u8,
                    (raw >> 8) as u8,
                    raw as u8,
                )
            } else {
                ((raw >> 16) as u8, (raw >> 8) as u8, raw as u8, 255)
            };
            return Color::from_rgba8(r, g, b, a);
        }
    }
    let parts = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))
        .and_then(|value| value.strip_suffix(')'))
        .map(|value| value.split(',').map(str::trim).collect::<Vec<_>>());
    if let Some(parts) = parts
        && parts.len() >= 3
    {
        let component = |index: usize| {
            parts[index]
                .trim_end_matches('%')
                .parse::<f32>()
                .unwrap_or(0.0)
        };
        let scale = if parts.iter().take(3).any(|part| part.ends_with('%')) {
            2.55
        } else {
            1.0
        };
        let alpha = parts
            .get(3)
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        return Color::from_rgba8(
            (component(0) * scale) as u8,
            (component(1) * scale) as u8,
            (component(2) * scale) as u8,
            (alpha * 255.0) as u8,
        );
    }
    Color::BLACK
}

fn parse_blend(value: &str) -> BlendMode {
    match value {
        "copy" => BlendMode::Source,
        "destination-over" => BlendMode::DestinationOver,
        "source-in" => BlendMode::SourceIn,
        "destination-in" => BlendMode::DestinationIn,
        "source-out" => BlendMode::SourceOut,
        "destination-out" => BlendMode::DestinationOut,
        "source-atop" => BlendMode::SourceAtop,
        "destination-atop" => BlendMode::DestinationAtop,
        "xor" => BlendMode::Xor,
        "lighter" => BlendMode::Plus,
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "color-dodge" => BlendMode::ColorDodge,
        "color-burn" => BlendMode::ColorBurn,
        "hard-light" => BlendMode::HardLight,
        "soft-light" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "hue" => BlendMode::Hue,
        "saturation" => BlendMode::Saturation,
        "color" => BlendMode::Color,
        "luminosity" => BlendMode::Luminosity,
        _ => BlendMode::SourceOver,
    }
}

fn parse_line_cap(value: &str) -> LineCap {
    match value {
        "round" => LineCap::Round,
        "square" => LineCap::Square,
        _ => LineCap::Butt,
    }
}
fn parse_line_join(value: &str) -> LineJoin {
    match value {
        "round" => LineJoin::Round,
        "bevel" => LineJoin::Bevel,
        _ => LineJoin::Miter,
    }
}
fn parse_font_size(value: &str) -> f32 {
    value
        .split_whitespace()
        .find_map(|part| part.strip_suffix("px").and_then(|part| part.parse().ok()))
        .unwrap_or(10.0)
}
fn valid_dimension(value: u32) -> u32 {
    value.clamp(1, 16_384)
}
fn num(args: &[HostValue], index: usize) -> f32 {
    args.get(index).and_then(HostValue::as_f64).unwrap_or(0.0) as f32
}
fn num_i(args: &[HostValue], index: usize) -> i32 {
    num(args, index) as i32
}
fn f64_field(d: &BTreeMap<String, HostValue>, key: &str, default: f64) -> f64 {
    d.get(key).and_then(HostValue::as_f64).unwrap_or(default)
}
fn str_field<'a>(d: &'a BTreeMap<String, HostValue>, key: &str, default: &'a str) -> &'a str {
    d.get(key).and_then(HostValue::as_str).unwrap_or(default)
}
fn bool_arg(args: &[HostValue], index: usize) -> bool {
    args.get(index)
        .and_then(HostValue::as_bool)
        .unwrap_or(false)
}
fn dimension(args: &[HostValue], index: usize, default: u32) -> u32 {
    args.get(index)
        .and_then(HostValue::as_f64)
        .filter(|value| value.is_finite())
        .map(|value| valid_dimension(value.max(1.0) as u32))
        .unwrap_or(default)
}
fn resource_id(args: &[HostValue], index: usize) -> Result<CanvasId, JsException> {
    args.get(index)
        .and_then(HostValue::as_u64)
        .map(CanvasId)
        .ok_or_else(|| JsException::new("missing canvas/image resource id"))
}
fn js_error(error: CanvasError) -> JsException {
    JsException::new(error.to_string())
}
fn sniff_image_mime(bytes: &[u8]) -> &'static str {
    if looks_like_svg(bytes) {
        return "image/svg+xml";
    }
    image::guess_format(bytes)
        .ok()
        .map(|format| match format {
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Gif => "image/gif",
            ImageFormat::WebP => "image/webp",
            _ => "image/png",
        })
        .unwrap_or("application/octet-stream")
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let text = text.trim_start();
    text.starts_with("<svg") || (text.starts_with("<?xml") && text.contains("<svg"))
}

fn premultiply_rgba(mut rgba: Vec<u8>) -> Vec<u8> {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let a = pixel[3] as u16;
        for channel in &mut pixel[..3] {
            *channel = ((*channel as u16 * a + 127) / 255) as u8;
        }
    }
    rgba
}

fn unpremultiply_rgba(rgba: &[u8]) -> Vec<u8> {
    let mut output = rgba.to_vec();
    for pixel in output.as_chunks_mut::<4>().0 {
        let a = pixel[3] as u16;
        if a > 0 {
            for channel in &mut pixel[..3] {
                let scaled = (*channel as u16).saturating_mul(255).saturating_add(a / 2);
                *channel = scaled.checked_div(a).unwrap_or(255).min(255) as u8;
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_image_decodes_to_reusable_rgba_pixels() {
        let mut runtime = CanvasRuntime::new();
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="3" height="2"><rect width="3" height="2" fill="#60a5fa"/></svg>"##;
        let id = runtime.decode_image(svg.to_vec()).unwrap();
        let bitmap = runtime.bitmap(id).unwrap();

        assert_eq!((bitmap.width, bitmap.height), (3, 2));
        assert_eq!(&bitmap.rgba[..4], &[0x60, 0xa5, 0xfa, 0xff]);
        assert_eq!(sniff_image_mime(svg), "image/svg+xml");
    }

    #[test]
    fn fill_rect_and_destination_out_round_trip_image_data() {
        let mut runtime = CanvasRuntime::new();
        let id = runtime.create_canvas(8, 8).unwrap();
        runtime
            .set_state(id, "fillStyle", &HostValue::string("#ff0000"))
            .unwrap();
        runtime
            .command(id, "fillRect", &[0.0, 0.0, 8.0, 8.0].map(HostValue::Number))
            .unwrap();
        runtime
            .set_state(
                id,
                "globalCompositeOperation",
                &HostValue::string("destination-out"),
            )
            .unwrap();
        runtime
            .command(id, "fillRect", &[2.0, 2.0, 2.0, 2.0].map(HostValue::Number))
            .unwrap();
        let pixels = runtime.get_image_data(id, 0, 0, 8, 8).unwrap();
        assert_eq!(&pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&pixels[(2 * 8 + 2) * 4..(2 * 8 + 2) * 4 + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn image_bitmap_reuses_decoded_pixels_without_base64() {
        let mut runtime = CanvasRuntime::new();
        let canvas = runtime.create_canvas(2, 2).unwrap();
        runtime
            .set_state(canvas, "fillStyle", &HostValue::string("#00ff00"))
            .unwrap();
        runtime
            .command(
                canvas,
                "fillRect",
                &[0.0, 0.0, 2.0, 2.0].map(HostValue::Number),
            )
            .unwrap();
        let bitmap = runtime.create_image_bitmap(canvas, None).unwrap();
        assert_eq!(runtime.bitmap(bitmap).unwrap().rgba[1], 255);
    }

    #[test]
    fn image_bitmap_crop_and_resize_uses_browser_overload_shape() {
        let mut runtime = CanvasRuntime::new();
        let canvas = runtime.create_canvas(4, 4).unwrap();
        runtime
            .set_state(canvas, "fillStyle", &HostValue::string("#00ff00"))
            .unwrap();
        runtime
            .command(
                canvas,
                "fillRect",
                &[0.0, 0.0, 4.0, 4.0].map(HostValue::Number),
            )
            .unwrap();
        let options = [
            ("sx".into(), HostValue::Number(1.0)),
            ("sy".into(), HostValue::Number(1.0)),
            ("sw".into(), HostValue::Number(2.0)),
            ("sh".into(), HostValue::Number(2.0)),
            ("resizeWidth".into(), HostValue::Number(8.0)),
            ("resizeHeight".into(), HostValue::Number(6.0)),
        ]
        .into_iter()
        .collect();
        let bitmap = runtime.create_image_bitmap(canvas, Some(&options)).unwrap();
        let bitmap = runtime.bitmap(bitmap).unwrap();
        assert_eq!((bitmap.width, bitmap.height), (8, 6));
        assert_eq!(&bitmap.rgba[..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn hosted_upload_acknowledges_full_frame_then_returns_only_dirty_rect() {
        let mut runtime = CanvasRuntime::new();
        let canvas = runtime.create_canvas(64, 32).unwrap();
        let initial = runtime.take_upload(canvas, None).unwrap().unwrap();
        assert_eq!(
            (
                initial.dirty_x,
                initial.dirty_y,
                initial.dirty_width,
                initial.dirty_height
            ),
            (0, 0, 64, 32)
        );
        assert_eq!(initial.bytes.len(), 64 * 32 * 4);
        assert!(
            runtime
                .take_upload(canvas, Some(initial.version))
                .unwrap()
                .is_none()
        );

        runtime
            .set_state(canvas, "fillStyle", &HostValue::string("rgba(255,0,0,0.5)"))
            .unwrap();
        runtime
            .command(
                canvas,
                "fillRect",
                &[4.0, 6.0, 3.0, 2.0].map(HostValue::Number),
            )
            .unwrap();
        let dirty = runtime
            .take_upload(canvas, Some(initial.version))
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                dirty.dirty_x,
                dirty.dirty_y,
                dirty.dirty_width,
                dirty.dirty_height
            ),
            (4, 6, 3, 2)
        );
        assert_eq!(dirty.bytes.len(), 3 * 2 * 4);
        assert_eq!(dirty.bytes[0], 127, "host upload must be premultiplied");
        assert_eq!(dirty.bytes[3], 127);

        runtime
            .command(canvas, "translate", &[10.0, 5.0].map(HostValue::Number))
            .unwrap();
        runtime
            .command(
                canvas,
                "fillRect",
                &[1.0, 2.0, 3.0, 2.0].map(HostValue::Number),
            )
            .unwrap();
        let transformed = runtime
            .take_upload(canvas, Some(dirty.version))
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                transformed.dirty_x,
                transformed.dirty_y,
                transformed.dirty_width,
                transformed.dirty_height,
            ),
            (11, 7, 3, 2),
            "dirty uploads must follow the Canvas transform",
        );
    }

    #[test]
    fn object_urls_hold_independent_resource_references_until_revoked() {
        let mut runtime = CanvasRuntime::new();
        let blob = runtime.create_blob(vec![1, 2, 3], "application/octet-stream".into());
        let first = runtime.create_object_url(blob).unwrap();
        let second = runtime.create_object_url(blob).unwrap();
        assert_ne!(first, second);

        assert!(
            runtime.release(blob),
            "the Blob owner can release independently"
        );
        assert_eq!(runtime.object_url_bytes(&first).unwrap(), vec![1, 2, 3]);
        assert!(runtime.revoke_object_url(&first));
        assert!(
            runtime.contains(blob),
            "the second URL still owns the resource"
        );
        assert!(runtime.revoke_object_url(&second));
        assert!(!runtime.contains(blob));
        assert!(runtime.object_url_bytes(&first).is_err());
    }
}
