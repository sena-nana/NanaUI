//! The renderer's glyph atlas: pages, placement, lifetime and handles.
//!
//! Owned by the device context, not by a window, a document or a text node.
//! Every target drawing on one `wgpu::Device` shares these pages, so a second
//! window costs no second copy of the shell's glyphs; two devices never meet,
//! because each has its own painter and therefore its own manager.
//!
//! Rectangle packing itself is `etagere`'s. What this file owns is the policy
//! around it — when to open a page, what to drop, how long a placement stays
//! valid, and how a caller that kept a handle too long finds out.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use etagere::{AllocId, BucketedAtlasAllocator, size2};

use super::glyph::GlyphRasterKey;
use super::raster::{GlyphImage, GlyphImageFormat};
use super::raster_cache::GlyphRasterCache;
use super::upload::GlyphUploadQueue;

/// Page edge in texels, capped by the device. 1024² is 1 MiB of mask or 4 MiB
/// of color — one page holds a shell's whole Latin working set, and a second
/// page is the growth step rather than a doubling that would strand memory.
const PAGE_EDGE: u32 = 1024;
/// Transparent gutter around every glyph.
///
/// Uploaded as real zero texels rather than merely reserved: a page that has
/// evicted and reused a rectangle still holds the previous glyph's pixels, and
/// a rotated quad samples a fraction of a texel past its own rect. One texel
/// of nobody's pixels is what keeps that tap from reading a neighbour.
const GLYPH_PADDING: u32 = 1;
/// Edge of the two placeholder pages that exist only to fill a bind group.
const PLACEHOLDER_EDGE: u32 = 1;
/// Ceiling on atlas texture memory across every page and kind. The two
/// placeholder pages are inside it, so it is also what makes them free.
const BYTE_BUDGET: usize = 48 * 1024 * 1024;

/// Page size and memory ceiling. Separated from the manager so a test can
/// reach eviction, growth and compaction with a handful of glyphs instead of
/// the tens of thousands a real page holds.
#[derive(Clone, Copy, Debug)]
pub(super) struct GlyphAtlasLimits {
    pub page_edge: u32,
    pub byte_budget: usize,
}

impl Default for GlyphAtlasLimits {
    fn default() -> Self {
        Self {
            page_edge: PAGE_EDGE,
            byte_budget: BYTE_BUDGET,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum AtlasPageKind {
    Mask,
    Color,
}

impl AtlasPageKind {
    fn of(format: GlyphImageFormat) -> Self {
        match format {
            GlyphImageFormat::Mask => Self::Mask,
            GlyphImageFormat::ColorRgba => Self::Color,
        }
    }

    pub(super) const fn bytes_per_texel(self) -> usize {
        match self {
            Self::Mask => 1,
            Self::Color => 4,
        }
    }

    fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            // Coverage, not color: it must not be gamma-decoded on the way in.
            Self::Mask => wgpu::TextureFormat::R8Unorm,
            // Color bitmaps are authored in sRGB, so the sampler decodes them
            // and the shader blends in linear light like everything else.
            Self::Color => wgpu::TextureFormat::Rgba8UnormSrgb,
        }
    }
}

/// A generational handle to one placed glyph.
///
/// `generation` starts at 1 and is bumped on every free, so a handle kept
/// across an eviction is rejected rather than resolving to whatever glyph now
/// owns that rectangle.
///
/// The indirection is what lets the atlas evict, reuse and relocate: an
/// instance is built from the handle at flush time, never from coordinates
/// captured earlier, and a handle whose slot has been reissued is rejected
/// rather than silently resolving to whatever glyph now owns that rectangle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct GlyphAtlasEntryId {
    index: u32,
    generation: u32,
}

/// One glyph's placement. Read through [`GlyphAtlasManager::entry`].
#[derive(Clone, Copy, Debug)]
pub(super) struct AtlasEntry {
    pub kind: AtlasPageKind,
    pub page: u32,
    /// Glyph origin in page texels, gutter excluded.
    pub origin: [u32; 2],
    pub size: [u32; 2],
    /// Bitmap offset from the pen, the rasterizer's convention.
    pub left: i32,
    pub top: i32,
}

struct Slot {
    /// Odd while live, bumped on every free so a kept handle fails closed.
    generation: u32,
    live: Option<LiveEntry>,
}

struct LiveEntry {
    key: GlyphRasterKey,
    entry: AtlasEntry,
    alloc: AllocId,
    last_used: u64,
}

struct AtlasPage {
    kind: AtlasPageKind,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    allocator: BucketedAtlasAllocator,
    edge: u32,
}

impl AtlasPage {
    fn bytes(&self) -> usize {
        (self.edge as usize) * (self.edge as usize) * self.kind.bytes_per_texel()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct GlyphAtlasCounters {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub pages: u32,
    pub bytes: u64,
    /// Live glyph area as a permille of the page area. The complement is
    /// fragmentation plus gutters; a page that cannot place a glyph while this
    /// is low is fragmented rather than full.
    pub occupancy_permille: u32,
    pub relocations: u64,
    pub stale_handle_rejects: u64,
}

pub(super) struct GlyphAtlasManager {
    pages: Vec<AtlasPage>,
    slots: Vec<Slot>,
    free_slots: Vec<u32>,
    index: HashMap<GlyphRasterKey, GlyphAtlasEntryId>,
    layout: wgpu::BindGroupLayout,
    nearest: wgpu::Sampler,
    linear: wgpu::Sampler,
    bind_groups: HashMap<(u32, u32), wgpu::BindGroup>,
    max_edge: u32,
    /// The mask and color placeholder pages, in that order.
    placeholders: [u32; 2],
    limits: GlyphAtlasLimits,
    frame: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
    relocations: u64,
    stale_handle_rejects: Cell<u64>,
    /// The raster epoch the placed pixels came from. A new epoch means the
    /// face set changed under the whole atlas, so every page is dropped rather
    /// than left holding bitmaps of faces that no longer exist.
    raster_generation: u64,
}

impl GlyphAtlasManager {
    pub(super) fn new(
        device: &wgpu::Device,
        raster_generation: u64,
        limits: GlyphAtlasLimits,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nana-ui.scene.text.atlas"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                sampler_entry(2),
                sampler_entry(3),
            ],
        });
        let nearest = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nana-ui.scene.text.atlas.nearest"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nana-ui.scene.text.atlas.linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let max_edge = device.limits().max_texture_dimension_2d.max(1);
        let mut manager = Self {
            pages: Vec::new(),
            slots: Vec::new(),
            free_slots: Vec::new(),
            index: HashMap::new(),
            layout,
            nearest,
            linear,
            bind_groups: HashMap::new(),
            max_edge,
            placeholders: [0, 1],
            limits,
            frame: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            relocations: 0,
            stale_handle_rejects: Cell::new(0),
            raster_generation,
        };
        // A bind group names both textures, so a mask-only frame still needs a
        // color view to point at. That is all these two are: 1×1 placeholders
        // no glyph can fit in, so a shell that never draws an emoji never pays
        // for a color page.
        manager.placeholders = [
            manager
                .open_page(device, AtlasPageKind::Mask, PLACEHOLDER_EDGE)
                .expect("the placeholder pages are two texels of the budget"),
            manager
                .open_page(device, AtlasPageKind::Color, PLACEHOLDER_EDGE)
                .expect("the placeholder pages are two texels of the budget"),
        ];
        manager
    }

    pub(super) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub(super) fn begin_frame(&mut self, raster_generation: u64) {
        self.frame = self.frame.wrapping_add(1);
        if raster_generation != self.raster_generation {
            self.drop_every_placement();
            self.raster_generation = raster_generation;
        }
    }

    pub(super) fn counters(&self) -> GlyphAtlasCounters {
        let bytes: usize = self.pages.iter().map(AtlasPage::bytes).sum();
        let texels: u64 = self
            .pages
            .iter()
            .map(|page| u64::from(page.edge) * u64::from(page.edge))
            .sum();
        let live: u64 = self
            .pages
            .iter()
            .map(|page| page.allocator.allocated_space().max(0) as u64)
            .sum();
        GlyphAtlasCounters {
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            pages: self.pages.len() as u32,
            bytes: bytes as u64,
            occupancy_permille: live.saturating_mul(1000).checked_div(texels).unwrap_or(0) as u32,
            relocations: self.relocations,
            stale_handle_rejects: self.stale_handle_rejects.get(),
        }
    }

    /// The placement a handle names, or `None` when the handle is stale.
    pub(super) fn entry(&self, id: GlyphAtlasEntryId) -> Option<&AtlasEntry> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            self.stale_handle_rejects
                .set(self.stale_handle_rejects.get() + 1);
            return None;
        }
        slot.live.as_ref().map(|live| &live.entry)
    }

    /// A live placement for `key`, marked as used by the frame in flight so
    /// this frame's own glyphs cannot be evicted out from under it.
    ///
    /// Returns the placement alongside the handle: the caller needs both, and
    /// resolving the handle again would be a second lookup plus a staleness
    /// branch on the one path that cannot be stale.
    pub(super) fn lookup(
        &mut self,
        key: &GlyphRasterKey,
    ) -> Option<(GlyphAtlasEntryId, AtlasEntry)> {
        let id = *self.index.get(key)?;
        let frame = self.frame;
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let live = slot.live.as_mut()?;
        live.last_used = frame;
        self.hits += 1;
        Some((id, live.entry))
    }

    /// Place `image` and queue its upload. `None` when even an empty maximal
    /// atlas could not hold it, which is also the one case where the glyph
    /// does not paint.
    pub(super) fn insert(
        &mut self,
        device: &wgpu::Device,
        key: GlyphRasterKey,
        image: &Arc<GlyphImage>,
        raster: &GlyphRasterCache,
        uploads: &mut GlyphUploadQueue,
    ) -> Option<(GlyphAtlasEntryId, AtlasEntry)> {
        self.misses += 1;
        let kind = AtlasPageKind::of(image.format);
        let cell = [
            image.width + GLYPH_PADDING * 2,
            image.height + GLYPH_PADDING * 2,
        ];
        let (page, alloc, min) = self.allocate(device, kind, cell, raster, uploads)?;
        let origin = [min[0] + GLYPH_PADDING, min[1] + GLYPH_PADDING];
        uploads.push(kind, page, min, cell, Arc::clone(image), GLYPH_PADDING);
        let entry = AtlasEntry {
            kind,
            page,
            origin,
            size: [image.width, image.height],
            left: image.left,
            top: image.top,
        };
        let live = LiveEntry {
            key,
            entry,
            alloc,
            last_used: self.frame,
        };
        let id = match self.free_slots.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.generation = slot.generation.wrapping_add(1).max(1);
                slot.live = Some(live);
                GlyphAtlasEntryId {
                    index,
                    generation: slot.generation,
                }
            }
            None => {
                self.slots.push(Slot {
                    generation: 1,
                    live: Some(live),
                });
                GlyphAtlasEntryId {
                    index: (self.slots.len() - 1) as u32,
                    generation: 1,
                }
            }
        };
        self.index.insert(key, id);
        Some((id, entry))
    }

    /// The bind group that reaches both of a draw segment's pages.
    pub(super) fn bind_group(
        &mut self,
        device: &wgpu::Device,
        mask_page: u32,
        color_page: u32,
    ) -> Option<&wgpu::BindGroup> {
        let mask = self.pages.get(mask_page as usize)?;
        let color = self.pages.get(color_page as usize)?;
        let (mask_view, color_view) = (&mask.view, &color.view);
        let layout = &self.layout;
        let (nearest, linear) = (&self.nearest, &self.linear);
        Some(
            self.bind_groups
                .entry((mask_page, color_page))
                .or_insert_with(|| {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("nana-ui.scene.text.atlas.bind"),
                        layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(mask_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(color_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(nearest),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::Sampler(linear),
                            },
                        ],
                    })
                }),
        )
    }

    /// The bind group `upload` already created for this page pair.
    pub(super) fn cached_bind_group(
        &self,
        mask_page: u32,
        color_page: u32,
    ) -> Option<&wgpu::BindGroup> {
        self.bind_groups.get(&(mask_page, color_page))
    }

    pub(super) fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub(super) fn page_texture(&self, page: u32) -> Option<&wgpu::Texture> {
        self.pages.get(page as usize).map(|page| &page.texture)
    }

    /// The 1×1 page of `kind`, for a draw segment that samples only the other
    /// kind but still has to name both.
    pub(super) fn placeholder_page(&self, kind: AtlasPageKind) -> u32 {
        match kind {
            AtlasPageKind::Mask => self.placeholders[0],
            AtlasPageKind::Color => self.placeholders[1],
        }
    }

    /// Reserve a `cell`-sized rectangle, opening a page or evicting cold
    /// glyphs until one fits.
    fn allocate(
        &mut self,
        device: &wgpu::Device,
        kind: AtlasPageKind,
        cell: [u32; 2],
        raster: &GlyphRasterCache,
        uploads: &mut GlyphUploadQueue,
    ) -> Option<(u32, AllocId, [u32; 2])> {
        let limit = self.limits.page_edge.min(self.max_edge);
        if cell[0] > limit || cell[1] > limit {
            return None;
        }
        let mut compacted = false;
        loop {
            if let Some(placed) = self.try_pages(kind, cell) {
                return Some(placed);
            }
            if self
                .open_page(device, kind, self.limits.page_edge)
                .is_some()
            {
                continue;
            }
            // Shelf packing strands space when glyph heights vary: a page can
            // refuse a glyph while half of it is free. Repacking the live set
            // is what reclaims that, and it is tried before eviction because
            // it costs uploads rather than re-rasterization. Once per
            // allocation: a second pass would find the same layout.
            if !compacted {
                compacted = true;
                if self.compact(kind, raster, uploads) {
                    continue;
                }
            }
            if !self.evict_coldest(kind) {
                return None;
            }
        }
    }

    fn try_pages(
        &mut self,
        kind: AtlasPageKind,
        cell: [u32; 2],
    ) -> Option<(u32, AllocId, [u32; 2])> {
        let size = size2(cell[0] as i32, cell[1] as i32);
        for (index, page) in self.pages.iter_mut().enumerate() {
            if page.kind != kind {
                continue;
            }
            if let Some(allocation) = page.allocator.allocate(size) {
                let min = allocation.rectangle.min;
                return Some((index as u32, allocation.id, [min.x as u32, min.y as u32]));
            }
        }
        None
    }

    /// Repack every live glyph of `kind` into freshly cleared pages.
    ///
    /// Handles survive: a slot keeps its generation and only its rectangle
    /// moves, which is exactly what the generational handle exists to allow —
    /// instances are built from handles after every placement is final, so a
    /// glyph relocated mid-frame is invisible to the draw that follows.
    ///
    /// Refused unless every live glyph's bitmap is still cached, because a
    /// relocation that could not be re-uploaded would blank a glyph this frame
    /// is drawing. Refusing leaves the atlas exactly as it was.
    fn compact(
        &mut self,
        kind: AtlasPageKind,
        raster: &GlyphRasterCache,
        uploads: &mut GlyphUploadQueue,
    ) -> bool {
        let frame = self.frame;
        let mut live: Vec<(bool, u32, Arc<GlyphImage>)> = Vec::new();
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(entry) = slot.live.as_ref() else {
                continue;
            };
            if entry.entry.kind != kind {
                continue;
            }
            let Some(image) = raster.peek(&entry.key) else {
                return false;
            };
            live.push((entry.last_used == frame, index as u32, image));
        }
        if live.is_empty() {
            return false;
        }
        // The frame in flight first, then tallest first. Order decides who
        // survives a repack that does not fit everything back, and a glyph
        // this frame already placed must not be the one dropped: its run holds
        // a handle to it, and losing it would leave a hole in text that is
        // being drawn right now. Within each group, tallest first, because a
        // shelf allocator strands short shelves under tall glyphs and placing
        // the tall ones into an empty page is the point of repacking.
        live.sort_unstable_by_key(|(hot, index, image)| {
            (
                std::cmp::Reverse(*hot),
                std::cmp::Reverse(image.height),
                *index,
            )
        });
        for page in &mut self.pages {
            if page.kind == kind {
                page.allocator.clear();
            }
        }
        // Past this point every `AllocId` of this kind names a rectangle the
        // packer has forgotten, so nothing may hand one back to it. Entries are
        // either re-placed with a fresh id below, or released without one.
        for (_, index, image) in live {
            let cell = [
                image.width + GLYPH_PADDING * 2,
                image.height + GLYPH_PADDING * 2,
            ];
            let Some((page, alloc, min)) = self.try_pages(kind, cell) else {
                // The same glyphs in a better order no longer fit the same
                // pages. Releasing the entry is fail-safe rather than
                // best-effort: its handle goes stale, so a run already holding
                // it drops that glyph for this frame instead of sampling the
                // rectangle someone else is about to be given.
                self.release_slot(index, false);
                continue;
            };
            uploads.push(kind, page, min, cell, image, GLYPH_PADDING);
            if let Some(slot) = self.slots.get_mut(index as usize)
                && let Some(entry) = slot.live.as_mut()
            {
                entry.entry.page = page;
                entry.entry.origin = [min[0] + GLYPH_PADDING, min[1] + GLYPH_PADDING];
                entry.alloc = alloc;
            }
            self.relocations += 1;
        }
        true
    }

    fn open_page(
        &mut self,
        device: &wgpu::Device,
        kind: AtlasPageKind,
        requested_edge: u32,
    ) -> Option<u32> {
        let edge = requested_edge.min(self.max_edge).max(1);
        let bytes: usize = self.pages.iter().map(AtlasPage::bytes).sum();
        let next = (edge as usize) * (edge as usize) * kind.bytes_per_texel();
        if bytes + next > self.limits.byte_budget {
            return None;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("nana-ui.scene.text.atlas.page"),
            size: wgpu::Extent3d {
                width: edge,
                height: edge,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: kind.texture_format(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.pages.push(AtlasPage {
            kind,
            texture,
            view,
            allocator: BucketedAtlasAllocator::new(size2(edge as i32, edge as i32)),
            edge,
        });
        Some((self.pages.len() - 1) as u32)
    }

    /// Free the coldest glyphs of `kind` that the frame in flight is not
    /// using. Returns `false` when every one of them is in use, which is the
    /// only case where a glyph cannot be placed at all.
    fn evict_coldest(&mut self, kind: AtlasPageKind) -> bool {
        let frame = self.frame;
        let mut victims: Vec<(u64, u32)> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let live = slot.live.as_ref()?;
                (live.entry.kind == kind && live.last_used != frame)
                    .then_some((live.last_used, index as u32))
            })
            .collect();
        if victims.is_empty() {
            return false;
        }
        victims.sort_unstable();
        // A quarter of the cold set per pass: enough that a full atlas does
        // not evict once per glyph, small enough that one large glyph does not
        // clear the working set.
        let drop = (victims.len() / 4).max(1);
        for (_, index) in victims.into_iter().take(drop) {
            self.free_slot(index);
            self.evictions += 1;
        }
        true
    }

    fn free_slot(&mut self, index: u32) {
        self.release_slot(index, true);
    }

    /// Retire a slot, bumping its generation so every handle to it fails.
    ///
    /// `release_rect` is false only while [`Self::compact`] holds cleared
    /// packers: the entry's `AllocId` then names a rectangle the packer no
    /// longer knows, and handing it back would corrupt the free list.
    fn release_slot(&mut self, index: u32, release_rect: bool) {
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return;
        };
        let Some(live) = slot.live.take() else {
            return;
        };
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free_slots.push(index);
        self.index.remove(&live.key);
        if release_rect && let Some(page) = self.pages.get_mut(live.entry.page as usize) {
            page.allocator.deallocate(live.alloc);
        }
    }

    fn drop_every_placement(&mut self) {
        for page in &mut self.pages {
            page.allocator.clear();
        }
        // After the clear, so no `AllocId` reaches a packer that has forgotten
        // it.
        for index in 0..self.slots.len() as u32 {
            self.release_slot(index, false);
        }
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        // Both stages: the vertex stage reads `textureDimensions` to turn a
        // texel rectangle into normalized UV, so a page that grew does not
        // need every instance rebuilt.
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

#[cfg(test)]
impl GlyphAtlasManager {
    /// Every live placement, for the invariant no counter can express: two
    /// glyphs must never own overlapping texels on one page.
    pub(super) fn live_rects(&self) -> Vec<(GlyphRasterKey, u32, [u32; 4])> {
        self.slots
            .iter()
            .filter_map(|slot| {
                let live = slot.live.as_ref()?;
                Some((
                    live.key,
                    live.entry.page,
                    [
                        live.entry.origin[0],
                        live.entry.origin[1],
                        live.entry.size[0],
                        live.entry.size[1],
                    ],
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::glyph::{
        GlyphFontId, GlyphRasterKey, GlyphRenderMode, GlyphSynthesis, GlyphVariationId, SubpixelBin,
    };
    use super::super::raster::{GlyphImage, GlyphImageFormat};
    use super::super::raster::{GlyphRasterRequest, GlyphRasterizer};
    use super::super::raster_cache::GlyphRasterCache;
    use super::super::upload::GlyphUploadQueue;
    use super::*;

    /// Square mask glyphs of a requested edge, so a test can decide exactly
    /// how a page fills.
    struct Squares {
        edge: u32,
    }

    impl GlyphRasterizer for Squares {
        fn rasterize(&mut self, _request: &GlyphRasterRequest) -> Option<GlyphImage> {
            Some(GlyphImage {
                format: GlyphImageFormat::Mask,
                width: self.edge,
                height: self.edge,
                left: 0,
                top: self.edge as i32,
                data: vec![255; (self.edge * self.edge) as usize],
            })
        }
    }

    fn key(glyph: u32) -> GlyphRasterKey {
        GlyphRasterKey {
            font: GlyphFontId(0),
            font_generation: 1,
            variation: GlyphVariationId(0),
            glyph,
            size_bits: 16f32.to_bits(),
            subpixel_x: SubpixelBin::default(),
            subpixel_y: SubpixelBin::default(),
            synthesis: GlyphSynthesis::NONE,
            mode: GlyphRenderMode::Mask,
        }
    }

    /// One page of 64², which is six rows of a 10px glyph plus a gutter.
    fn small_atlas(
        device: &wgpu::Device,
    ) -> (GlyphAtlasManager, GlyphRasterCache, GlyphUploadQueue) {
        let raster = GlyphRasterCache::default();
        let atlas = GlyphAtlasManager::new(
            device,
            raster.generation(),
            GlyphAtlasLimits {
                page_edge: 64,
                byte_budget: 64 * 64 + 8,
            },
        );
        (atlas, raster, GlyphUploadQueue::default())
    }

    fn assert_no_overlap(atlas: &GlyphAtlasManager) {
        let rects = atlas.live_rects();
        for (index, (left_key, left_page, left)) in rects.iter().enumerate() {
            for (right_key, right_page, right) in &rects[index + 1..] {
                if left_page != right_page {
                    continue;
                }
                let disjoint = left[0] + left[2] <= right[0]
                    || right[0] + right[2] <= left[0]
                    || left[1] + left[3] <= right[1]
                    || right[1] + right[3] <= left[1];
                assert!(
                    disjoint,
                    "{left_key:?} at {left:?} overlaps {right_key:?} at {right:?} on page {left_page}"
                );
            }
        }
    }

    /// Place `count` glyphs, letting the atlas grow, compact or evict.
    fn fill(
        atlas: &mut GlyphAtlasManager,
        raster: &mut GlyphRasterCache,
        uploads: &mut GlyphUploadQueue,
        rasterizer: &mut Squares,
        device: &wgpu::Device,
        glyphs: std::ops::Range<u32>,
    ) -> Vec<(u32, Option<GlyphAtlasEntryId>)> {
        glyphs
            .map(|glyph| {
                let key = key(glyph);
                let placed = match atlas.lookup(&key) {
                    Some((id, _)) => Some(id),
                    None => raster
                        .get_or_rasterize(rasterizer, key)
                        .and_then(|image| atlas.insert(device, key, &image, raster, uploads))
                        .map(|(id, _)| id),
                };
                (glyph, placed)
            })
            .collect()
    }

    #[test]
    fn a_handle_to_an_evicted_placement_is_rejected_rather_than_resolved() {
        let (device, _queue) = crate::test_gpu::device();
        let (mut atlas, mut raster, mut uploads) = small_atlas(&device);
        let mut squares = Squares { edge: 10 };
        atlas.begin_frame(raster.generation());
        let first = fill(
            &mut atlas,
            &mut raster,
            &mut uploads,
            &mut squares,
            &device,
            0..25,
        );
        let placed: Vec<_> = first
            .iter()
            .filter_map(|(glyph, id)| id.map(|id| (*glyph, id)))
            .collect();
        assert!(placed.len() > 8, "the small page must hold a first batch");
        assert_no_overlap(&atlas);

        // A new frame: last frame's glyphs are cold, so a second batch of
        // unrelated glyphs has to evict them.
        atlas.begin_frame(raster.generation());
        fill(
            &mut atlas,
            &mut raster,
            &mut uploads,
            &mut squares,
            &device,
            100..140,
        );
        assert!(
            atlas.counters().evictions > 0,
            "a full page must evict to place a new batch"
        );
        assert_no_overlap(&atlas);

        let rejected = placed
            .iter()
            .filter(|(_, id)| atlas.entry(*id).is_none())
            .count();
        assert!(rejected > 0, "evicted glyphs must reject their old handles");
        assert_eq!(
            atlas.counters().stale_handle_rejects as usize,
            rejected,
            "every rejection must be counted"
        );
        // And nothing that still resolves may have become another glyph: the
        // index is what decides, so a live handle's key must still be its own.
        for (glyph, id) in &placed {
            if atlas.entry(*id).is_some() {
                assert_eq!(
                    atlas.lookup(&key(*glyph)).map(|(id, _)| id),
                    Some(*id),
                    "a live handle must still name the glyph it was issued for"
                );
            }
        }
    }

    #[test]
    fn a_repacked_page_moves_glyphs_without_letting_two_own_one_rectangle() {
        let (device, _queue) = crate::test_gpu::device();
        let (mut atlas, mut raster, mut uploads) = small_atlas(&device);
        atlas.begin_frame(raster.generation());
        // Shelves of one height first, then a taller glyph that no shelf can
        // take: the page is far from full, but the packer cannot serve it.
        fill(
            &mut atlas,
            &mut raster,
            &mut uploads,
            &mut Squares { edge: 6 },
            &device,
            0..40,
        );
        fill(
            &mut atlas,
            &mut raster,
            &mut uploads,
            &mut Squares { edge: 28 },
            &device,
            200..203,
        );
        assert_no_overlap(&atlas);
        let counters = atlas.counters();
        assert!(
            counters.relocations > 0 || counters.evictions > 0,
            "a fragmented page must repack or evict rather than refuse forever"
        );
    }

    #[test]
    fn a_new_raster_epoch_drops_every_placement_and_frees_the_pages() {
        let (device, _queue) = crate::test_gpu::device();
        let (mut atlas, mut raster, mut uploads) = small_atlas(&device);
        atlas.begin_frame(raster.generation());
        let placed = fill(
            &mut atlas,
            &mut raster,
            &mut uploads,
            &mut Squares { edge: 8 },
            &device,
            0..4,
        );
        raster.invalidate();
        atlas.begin_frame(raster.generation());
        for (_, id) in placed.iter().filter_map(|(g, id)| id.map(|id| (*g, id))) {
            assert!(
                atlas.entry(id).is_none(),
                "a placement from the previous face set must not resolve"
            );
        }
        assert!(atlas.live_rects().is_empty());
    }

    #[test]
    fn a_placement_this_frame_is_never_evicted_out_from_under_it() {
        let (device, _queue) = crate::test_gpu::device();
        let (mut atlas, mut raster, mut uploads) = small_atlas(&device);
        atlas.begin_frame(raster.generation());
        // More glyphs than the page can hold, all in one frame. Whatever is
        // placed must survive the frame, so the ones that do not fit are
        // refused rather than displacing a glyph already being drawn.
        let placed = fill(
            &mut atlas,
            &mut raster,
            &mut uploads,
            &mut Squares { edge: 12 },
            &device,
            0..60,
        );
        let live: Vec<_> = placed
            .iter()
            .filter_map(|(_, id)| *id)
            .filter(|id| atlas.entry(*id).is_some())
            .collect();
        let issued = placed.iter().filter(|(_, id)| id.is_some()).count();
        assert_eq!(
            live.len(),
            issued,
            "every handle issued this frame must still resolve at the end of it"
        );
        assert!(
            placed.iter().any(|(_, id)| id.is_none()),
            "the page has to have run out for this to be testing anything"
        );
        assert_no_overlap(&atlas);
    }
}
