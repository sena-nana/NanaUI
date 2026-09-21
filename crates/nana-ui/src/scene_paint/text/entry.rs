//! `TextGpuEntry`: one text node's glyphs, kept between frames.
//!
//! Everything above this module produces a paragraph's glyphs from scratch:
//! shape it, resolve every glyph, rasterize what is missing, place it in the
//! atlas. That work is proportional to the text on screen and it used to
//! happen on every frame that rebatched — which is every frame with any
//! animation in it, anywhere.
//!
//! An entry is the answer: the resolved instances of one node, held in a slab
//! and handed back whenever the facts they were derived from still hold.
//!
//! ```text
//! layout unchanged  ── the laid-out paragraph is the same one
//! colors unchanged  ── its rich spans still paint what they painted
//! phase unchanged   ── the bitmaps were rasterized for this sub-pixel offset
//! scale unchanged   ── they were placed at this device scale and raster step
//! fonts unchanged   ── the face set still issues these face ids
//! atlas unchanged   ── the rectangles are still these glyphs'
//! ```
//!
//! All but the last are compared in `prepare`; a mismatch rebuilds that one
//! entry and nothing else. The atlas is repaired rather than rebuilt: the
//! entry keeps one atlas handle per glyph, so a relocation only has to re-read
//! rectangles.
//!
//! What an entry deliberately does **not** hold is where its text sits, what
//! *solid* color it paints, how opaque it is or what transform it is under.
//! Those are the run and presentation rows of [`super::pipeline`], which is
//! why moving, fading or recoloring plain text never touches a single
//! instance. Rich spans are the exception, and the one they prove: a span
//! paints something the run row cannot say, so its bytes are baked into the
//! instances and `colors` is what notices when they change.

use std::collections::HashMap;

use super::atlas::GlyphAtlasEntryId;
use super::pipeline::{DrawSegment, GlyphInstance};

/// Which draw of which node an entry belongs to.
///
/// `slot` is the scene's own `PrimitiveId` slot: one node can draw several
/// paragraphs — a checkbox's tick beside its label, an input's placeholder
/// beside its value — and they are not each other's.
///
/// `pass` separates a label's text shadows from the label: they resolve the
/// same paragraph at different offsets and colors, so they are different
/// entries over the same shaped buffer and the same glyph bitmaps.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(in crate::scene_paint) struct EntryKey {
    pub node: u64,
    pub slot: u64,
    pub pass: u32,
}

/// A page pair and the span of one entry's instances that samples it.
/// `first` is relative to the entry's block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EntrySegment {
    pub mask_page: u32,
    pub color_page: u32,
    pub first: u32,
    pub count: u32,
}

/// One text node's retained GPU geometry. See the module docs.
pub(super) struct TextGpuEntry {
    /// Hash of the shaped paragraph these glyphs were resolved from.
    pub layout: u64,
    /// What these glyphs were painted with, when that is not the run row's
    /// colour alone: a fingerprint of the rich spans and the colour they were
    /// resolved against. Zero for solid text, whose every glyph inherits the
    /// run row and is therefore recoloured without touching an instance.
    ///
    /// Its own field because the *layout* is not keyed by colour any more:
    /// since #99 two spellings of one string in different colours are the same
    /// paragraph, and only this tells their glyphs apart.
    pub colors: u64,
    /// Sub-pixel phase of the run origin the bitmaps were rasterized for.
    pub phase: [u32; 2],
    /// Font-set generation the face ids were issued under.
    pub font_generation: u64,
    /// The scene rebuild that wrote the primitive these glyphs were resolved
    /// from, and the device scale they were resolved at. Together with
    /// `font_generation` they are the cheap half of `layout`: if none of them
    /// moved, the paragraph and its box are the ones this entry already holds,
    /// so the shape key does not have to be assembled and hashed to find that
    /// out. `u64::MAX` is "no primitive said", which never matches.
    pub revision: u64,
    /// Physical px per logical px the glyphs were resolved at: the device
    /// scale times [`Self::raster_step`]'s factor.
    pub scale_bits: u32,
    /// The magnification step a transform above this text earned it, held
    /// between frames so a zoom that hovers near a step boundary does not
    /// rasterize the paragraph again every time it crosses it.
    pub raster_step: u8,
    /// What the paragraph measured: the widest line and the height the lines
    /// laid out to, in **logical** px — the layout is laid out there and the
    /// device scale only reaches the glyph coordinates. A pure function of the
    /// layout, which `revision` already pins, so a steady frame reads it here
    /// instead of walking the lines again.
    pub measured: [f32; 2],
    /// Atlas placement epoch the rectangles were read at.
    pub atlas_epoch: u64,
    /// Start of this entry's block in the store.
    pub block: u32,
    /// Slots the block spans. `capacity - glyphs` of them are vacant, so a
    /// paragraph that gains or loses a few glyphs keeps its arena offset and
    /// the paragraphs after it are not rewritten.
    pub capacity: u32,
    /// Live instances, the first of the block.
    pub glyphs: u32,
    pub segments: Vec<EntrySegment>,
    /// Where the block was last written in the arena, and how many slots it
    /// claimed there. `arena_generation` is the arena layout those are true
    /// for; anything older has to be placed again.
    pub arena_offset: u32,
    pub arena_capacity: u32,
    pub arena_generation: Option<u64>,
    /// The run row this entry's instances name. Allocated once and kept for
    /// the entry's whole life, so a frame that only changes which paragraphs
    /// are on screen does not renumber the ones that stayed — renumbering
    /// would mean rewriting every instance of every one of them.
    pub slot: Option<u32>,
    /// The slot the instances currently carry, which lags `slot` only between
    /// a rebuild and the flush that binds it.
    pub run: u32,
    pub last_used: u64,
    /// A rectangle this entry named could not be repaired, so its glyphs are
    /// no longer all the ones its paragraph resolves to. It draws what it has
    /// this frame and is resolved again on the next.
    pub damaged: bool,
}

impl TextGpuEntry {
    /// Whether the glyphs this entry holds are still the ones `layout` at
    /// `phase`, `colors` and `scale_bits` would resolve to under `fonts`.
    ///
    /// The scale is its own term because the layout is not keyed by it: since
    /// #99 a paragraph is laid out in logical px and the device scale is
    /// applied to its coordinates at resolve time, so one layout hash stands
    /// for two sets of glyphs at 1x and 2x.
    ///
    /// Deliberately does not consider the atlas: a relocation is repaired by
    /// re-reading rectangles through the handles, which costs no shaping, no
    /// rasterizing and no atlas traffic.
    pub(super) fn valid(
        &self,
        layout: u64,
        colors: u64,
        phase: [u32; 2],
        scale_bits: u32,
        fonts: u64,
    ) -> bool {
        !self.damaged
            && self.layout == layout
            && self.colors == colors
            && self.phase == phase
            && self.scale_bits == scale_bits
            && self.font_generation == fonts
    }
}

/// Slots a block of `glyphs` is given: an eighth of slack, at least four,
/// rounded up to a size class.
///
/// The slack is what lets typing a character into a field, a counter ticking
/// from 9 to 10 or a label gaining a word stay inside the block it already
/// owns (see [`EntryStore::begin_build`]), so the only bytes that move are
/// that paragraph's. The classes — four per doubling — are what let a block
/// one paragraph gave back be taken by the next one of about that length:
/// both free lists are keyed by exact capacity, and without classes a list
/// whose labels vary in length would only ever grow.
pub(super) fn capacity_for(glyphs: u32) -> u32 {
    let need = glyphs.saturating_add((glyphs / 8).max(4));
    if need <= 8 {
        return 8;
    }
    let step = (1u32 << (31 - need.leading_zeros())) / 4;
    need.div_ceil(step).saturating_mul(step)
}

/// Whether a block of `capacity` slots may keep holding `glyphs`.
///
/// Growing within it is the point of the slack. Shrinking only happens when
/// the block is more than twice what the paragraph needs *and* that is a real
/// amount of memory. A block that moves costs more than its own bytes: it
/// lands away from its neighbours in the arena, so the draw it belonged to
/// splits, and enough splits repack — rewrite — every block there is. A cell
/// whose text keeps changing length therefore settles at the largest class it
/// has needed rather than moving on every change; a paragraph that lost most
/// of a page of text still gives the space back.
fn block_fits(capacity: u32, glyphs: u32) -> bool {
    glyphs <= capacity
        && (capacity <= capacity_for(glyphs).saturating_mul(2) || capacity - glyphs <= SHRINK_SLACK)
}

/// Vacant slots a block may carry before shrinking is worth moving it: a
/// kilobyte and a half of instances.
const SHRINK_SLACK: u32 = 64;

/// The entries, their instances and the atlas handles behind them.
///
/// Instances and handles are parallel slabs: one allocation covers both, and a
/// glyph's handle is at the same index as the instance it produced, which is
/// what makes repairing a relocated rectangle a straight walk.
#[derive(Default)]
pub(super) struct EntryStore {
    entries: Vec<Option<TextGpuEntry>>,
    /// Hashed by [`nana_ui_runtime::IdHasher`]: the key is three integers a
    /// steady frame looks up once per paragraph, and SipHash over them was a
    /// measurable share of the painter.
    index: HashMap<EntryKey, u32, nana_ui_runtime::BuildIdHasher>,
    vacant: Vec<u32>,
    instances: Vec<GlyphInstance>,
    handles: Vec<GlyphAtlasEntryId>,
    /// Free blocks by capacity. Blocks are only ever reused at their own
    /// capacity, so the slab never has to coalesce or compact.
    free: HashMap<u32, Vec<u32>>,
    live_glyphs: u64,
    created: u64,
    destroyed: u64,
    reused: u64,
}

impl EntryStore {
    pub(super) fn get(&self, id: u32) -> Option<&TextGpuEntry> {
        self.entries.get(id as usize)?.as_ref()
    }

    pub(super) fn get_mut(&mut self, id: u32) -> Option<&mut TextGpuEntry> {
        self.entries.get_mut(id as usize)?.as_mut()
    }

    pub(super) fn lookup(&self, key: EntryKey) -> Option<u32> {
        self.index.get(&key).copied()
    }

    /// Record that an entry answered a frame without being resolved again.
    pub(super) fn note_reuse(&mut self) {
        self.reused += 1;
    }

    pub(super) fn counters(&self) -> EntryCounters {
        EntryCounters {
            active: self.index.len() as u64,
            created: self.created,
            destroyed: self.destroyed,
            reused: self.reused,
            glyphs: self.live_glyphs,
        }
    }

    /// Each live glyph's instance with the atlas handle it was read through.
    #[cfg(test)]
    pub(super) fn live_glyphs(
        &self,
        entry: &TextGpuEntry,
    ) -> impl Iterator<Item = (&GlyphInstance, GlyphAtlasEntryId)> {
        let start = entry.block as usize;
        let end = start + entry.glyphs as usize;
        self.instances[start..end]
            .iter()
            .zip(self.handles[start..end].iter().copied())
    }

    /// The whole block, slack included: what a draw that spans it reads.
    pub(super) fn instances(&self, entry: &TextGpuEntry) -> &[GlyphInstance] {
        let start = entry.block as usize;
        &self.instances[start..start + entry.capacity as usize]
    }

    /// Point every instance of `id` at `run`. Returns whether anything moved.
    pub(super) fn bind_run(&mut self, id: u32, run: u32) -> bool {
        let Some(entry) = self.entries.get_mut(id as usize).and_then(Option::as_mut) else {
            return false;
        };
        if entry.run == run {
            return false;
        }
        entry.run = run;
        let start = entry.block as usize;
        let end = start + entry.capacity as usize;
        for instance in &mut self.instances[start..end] {
            *instance = instance.with_run(run);
        }
        true
    }

    /// Re-read every rectangle through its handle after the atlas moved.
    ///
    /// Returns `false` when a handle no longer names a live placement, which
    /// means this entry has to be resolved again before it is correct. The
    /// glyph is vacated for this frame rather than sampling whatever now owns
    /// its rectangle.
    pub(super) fn repair(
        &mut self,
        id: u32,
        epoch: u64,
        mut rect: impl FnMut(GlyphAtlasEntryId) -> Option<([u32; 2], [u32; 2])>,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(id as usize).and_then(Option::as_mut) else {
            return false;
        };
        let start = entry.block as usize;
        let live = entry.glyphs as usize;
        entry.atlas_epoch = epoch;
        let mut intact = true;
        for offset in 0..live {
            let handle = self.handles[start + offset];
            match rect(handle) {
                Some((origin, size)) => {
                    self.instances[start + offset] =
                        self.instances[start + offset].with_placement(origin, size);
                }
                None => {
                    self.instances[start + offset] = self.instances[start + offset].vacated();
                    intact = false;
                }
            }
        }
        intact
    }

    /// Take the block `key` should build into, releasing whatever it held.
    ///
    /// `release` sees every atlas handle the old block referenced, so the
    /// entry's claim on those glyphs ends exactly when its instances do.
    pub(super) fn begin_build(
        &mut self,
        key: EntryKey,
        glyphs: u32,
        mut release: impl FnMut(GlyphAtlasEntryId),
    ) -> u32 {
        let capacity = capacity_for(glyphs);
        let id = match self.index.get(&key).copied() {
            Some(id) => {
                let entry = self.entries[id as usize]
                    .as_mut()
                    .expect("an indexed entry is live");
                self.live_glyphs -= u64::from(entry.glyphs);
                let start = entry.block as usize;
                // The claims the *old* block made, which is its glyph count,
                // not the new one: a paragraph rebuilt shorter would otherwise
                // keep claiming the glyphs it dropped for as long as it lives.
                let held = entry.glyphs.min(entry.capacity) as usize;
                let resize =
                    (!block_fits(entry.capacity, glyphs)).then_some((entry.block, entry.capacity));
                entry.glyphs = glyphs;
                entry.segments.clear();
                entry.damaged = false;
                for handle in &self.handles[start..start + held] {
                    release(*handle);
                }
                if let Some((block, released)) = resize {
                    self.free.entry(released).or_default().push(block);
                    let block = self.take_block(capacity);
                    let entry = self.entries[id as usize]
                        .as_mut()
                        .expect("an indexed entry is live");
                    entry.block = block;
                    entry.capacity = capacity;
                }
                id
            }
            None => {
                let block = self.take_block(capacity);
                let entry = TextGpuEntry {
                    layout: 0,
                    colors: 0,
                    phase: [0; 2],
                    font_generation: 0,
                    revision: u64::MAX,
                    scale_bits: 0,
                    raster_step: 0,
                    measured: [0.0; 2],
                    atlas_epoch: 0,
                    block,
                    capacity,
                    glyphs,
                    segments: Vec::new(),
                    arena_offset: 0,
                    arena_capacity: 0,
                    arena_generation: None,
                    slot: None,
                    run: u32::MAX,
                    last_used: 0,
                    damaged: false,
                };
                let id = match self.vacant.pop() {
                    Some(id) => {
                        self.entries[id as usize] = Some(entry);
                        id
                    }
                    None => {
                        self.entries.push(Some(entry));
                        (self.entries.len() - 1) as u32
                    }
                };
                self.index.insert(key, id);
                self.created += 1;
                id
            }
        };
        self.live_glyphs += u64::from(glyphs);
        id
    }

    /// Write one resolved glyph into the block being built.
    pub(super) fn push_glyph(
        &mut self,
        id: u32,
        offset: u32,
        handle: GlyphAtlasEntryId,
        instance: GlyphInstance,
    ) {
        let entry = self.entries[id as usize].as_ref().expect("building");
        let slot = entry.block as usize + offset as usize;
        self.instances[slot] = instance;
        self.handles[slot] = handle;
    }

    /// Close the block opened by [`Self::begin_build`] at `placed` live glyphs.
    ///
    /// Not every resolved glyph reaches a slot: a space rasterizes to nothing
    /// and an atlas that cannot hold one drops it. Everything past `placed` is
    /// vacated, because a draw that batches this block with its neighbour
    /// covers the slack too, and whatever the previous build left there would
    /// paint as a second copy of the old text.
    pub(super) fn finish_build(&mut self, id: u32, placed: u32) {
        let Some(entry) = self.entries.get_mut(id as usize).and_then(Option::as_mut) else {
            return;
        };
        self.live_glyphs -= u64::from(entry.glyphs);
        self.live_glyphs += u64::from(placed);
        entry.glyphs = placed;
        // The instances the run index has yet to be bound into.
        entry.run = u32::MAX;
        let start = entry.block as usize;
        let end = start + entry.capacity as usize;
        self.instances[start + placed as usize..end].fill(GlyphInstance::VACANT);
        self.handles[start + placed as usize..end].fill(GlyphAtlasEntryId::STALE);
    }

    /// Void every block's arena placement. What a repack starts with.
    pub(super) fn invalidate_arena(&mut self) {
        for entry in self.entries.iter_mut().flatten() {
            entry.arena_generation = None;
        }
    }

    /// Drop every entry last used before `before`, giving back its glyphs, its
    /// run row and its arena block.
    pub(super) fn retire(
        &mut self,
        before: u64,
        mut release: impl FnMut(GlyphAtlasEntryId),
        mut release_slot: impl FnMut(u32),
        mut release_arena: impl FnMut(u64, u32, u32),
    ) {
        let stale = self
            .index
            .iter()
            .filter(|(_, id)| {
                self.entries[**id as usize]
                    .as_ref()
                    .is_none_or(|entry| entry.last_used < before)
            })
            .map(|(key, id)| (*key, *id))
            .collect::<Vec<_>>();
        for (key, id) in stale {
            self.index.remove(&key);
            let Some(entry) = self.entries[id as usize].take() else {
                continue;
            };
            let start = entry.block as usize;
            for handle in &self.handles[start..start + entry.glyphs as usize] {
                release(*handle);
            }
            self.live_glyphs -= u64::from(entry.glyphs);
            if let Some(slot) = entry.slot {
                release_slot(slot);
            }
            if let Some(generation) = entry.arena_generation {
                release_arena(generation, entry.arena_offset, entry.arena_capacity);
            }
            self.free
                .entry(entry.capacity)
                .or_default()
                .push(entry.block);
            self.vacant.push(id);
            self.destroyed += 1;
        }
        self.compact_if_sparse();
    }

    /// Close the holes retired entries left, once they are most of the slab.
    ///
    /// Free blocks are only ever reused at their own size class, so a panel
    /// of long paragraphs that closed leaves space no label will take. Without
    /// this the slab stays at the largest amount of text the session ever
    /// held. Only the store's own offsets move — an entry's arena placement
    /// and its GPU bytes are untouched, because the block's contents are.
    fn compact_if_sparse(&mut self) {
        let free: usize = self
            .free
            .iter()
            .map(|(capacity, blocks)| *capacity as usize * blocks.len())
            .sum();
        if self.instances.len() < MIN_COMPACT_SLOTS || free * 2 < self.instances.len() {
            return;
        }
        let live = self.instances.len() - free;
        let mut instances = Vec::with_capacity(live);
        let mut handles = Vec::with_capacity(live);
        for entry in self.entries.iter_mut().flatten() {
            let start = entry.block as usize;
            let end = start + entry.capacity as usize;
            entry.block = instances.len() as u32;
            instances.extend_from_slice(&self.instances[start..end]);
            handles.extend_from_slice(&self.handles[start..end]);
        }
        self.instances = instances;
        self.handles = handles;
        self.free.clear();
    }

    /// Drop everything. For a font-set change, where no entry means what it
    /// meant.
    pub(super) fn clear(&mut self, mut release: impl FnMut(GlyphAtlasEntryId)) {
        for entry in self.entries.iter_mut().filter_map(Option::take) {
            let start = entry.block as usize;
            for handle in &self.handles[start..start + entry.glyphs as usize] {
                release(*handle);
            }
            self.destroyed += 1;
        }
        self.entries.clear();
        self.index.clear();
        self.vacant.clear();
        self.instances.clear();
        self.handles.clear();
        self.free.clear();
        self.live_glyphs = 0;
    }

    fn take_block(&mut self, capacity: u32) -> u32 {
        if let Some(block) = self.free.get_mut(&capacity).and_then(Vec::pop) {
            return block;
        }
        let block = self.instances.len() as u32;
        self.instances.resize(
            self.instances.len() + capacity as usize,
            GlyphInstance::VACANT,
        );
        self.handles.resize(
            self.handles.len() + capacity as usize,
            GlyphAtlasEntryId::STALE,
        );
        block
    }
}

/// Slab size below which holes are left alone: moving a few thousand
/// instances to save a few thousand is not worth the copy.
const MIN_COMPACT_SLOTS: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct EntryCounters {
    pub active: u64,
    pub created: u64,
    pub destroyed: u64,
    pub reused: u64,
    pub glyphs: u64,
}

/// A draw command's instance spans, assembled from the entries under it.
///
/// Segments coalesce across entries: two labels whose blocks ended up adjacent
/// and whose glyphs came from the same pages are one draw, however different
/// their colors, positions or transforms are — those live in the run rows the
/// instances name, not in the batch key.
pub(super) struct SegmentBuilder {
    open: Option<DrawSegment>,
    placeholders: [u32; 2],
}

impl SegmentBuilder {
    pub(super) fn new(placeholders: [u32; 2]) -> Self {
        Self {
            open: None,
            placeholders,
        }
    }

    /// `adjacent` says the block this segment comes from starts exactly where
    /// the previous entry's block ended. Only then may a draw span the gap:
    /// what lies between two adjacent blocks is the slack their size classes
    /// left, which covers nothing, while what lies between two blocks the
    /// arena placed apart is other paragraphs' glyphs.
    pub(super) fn push(
        &mut self,
        out: &mut Vec<DrawSegment>,
        segment: EntrySegment,
        base: u32,
        adjacent: bool,
    ) {
        let first = base + segment.first;
        let compatible = adjacent
            && self.open.as_ref().is_some_and(|open| {
                open.first + open.count <= first
                    && (open.mask_page == segment.mask_page
                        || open.mask_page == self.placeholders[0]
                        || segment.mask_page == self.placeholders[0])
                    && (open.color_page == segment.color_page
                        || open.color_page == self.placeholders[1]
                        || segment.color_page == self.placeholders[1])
            });
        if compatible {
            let open = self.open.as_mut().expect("compatible implies open");
            if segment.mask_page != self.placeholders[0] {
                open.mask_page = segment.mask_page;
            }
            if segment.color_page != self.placeholders[1] {
                open.color_page = segment.color_page;
            }
            open.count = first + segment.count - open.first;
            return;
        }
        if let Some(open) = self.open.take() {
            out.push(open);
        }
        self.open = Some(DrawSegment {
            mask_page: segment.mask_page,
            color_page: segment.color_page,
            first,
            count: segment.count,
        });
    }

    pub(super) fn finish(mut self, out: &mut Vec<DrawSegment>) {
        if let Some(open) = self.open.take() {
            out.push(open);
        }
    }
}

/// Where each entry's block sits in the target's instance buffer.
///
/// Draw order and storage order are not the same thing, and this is where they
/// are reconciled. A block keeps its offset for as long as it lives, so a
/// paragraph that changes costs its own bytes and nobody else's. Neighbouring
/// blocks that happen to be adjacent draw as one command; a block that had to
/// be placed elsewhere costs that command one more draw, and when enough of
/// them have accumulated the arena is repacked in draw order and the frame
/// after it is one draw again.
///
/// The generation is what makes that safe: a repack or a buffer replacement
/// bumps it, and a block placed under an older one is written again rather
/// than left naming a range that now holds another paragraph.
#[derive(Default)]
pub(super) struct InstanceArena {
    /// Slots handed out, including the holes between them.
    len: u32,
    /// Slots the GPU buffer holds.
    capacity: u32,
    /// Slots inside live blocks.
    live: u32,
    /// Free blocks by their exact capacity, so the arena never has to coalesce.
    free: HashMap<u32, Vec<u32>>,
    /// Commands that had to open a second draw because two of their entries
    /// were not adjacent, last frame.
    breaks: u32,
    generation: u64,
}

/// Extra draws fragmentation may cost before the arena is repacked.
///
/// A repack rewrites every block, so what it costs grows with the text on
/// screen; an extra draw costs the same few microseconds however much text
/// there is. A shell with a few hundred glyphs is therefore repacked as soon
/// as it fragments at all, and one with fifty thousand tolerates a few dozen
/// extra draws rather than move a megabyte every frame.
fn break_budget(total: u32) -> u32 {
    BREAK_FLOOR.max(total / SLOTS_PER_BREAK)
}

const BREAK_FLOOR: u32 = 4;
/// Instance slots whose rewrite costs about what one extra draw does.
const SLOTS_PER_BREAK: u32 = 1024;

/// Instance slots a fresh target's arena starts at.
const MIN_ARENA: u32 = 512;

impl InstanceArena {
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Slots handed out, holes included.
    #[cfg(test)]
    pub(super) fn len(&self) -> u32 {
        self.len
    }

    pub(super) fn note_breaks(&mut self, breaks: u32) {
        self.breaks = breaks;
    }

    /// Whether this frame should lay every block out again in draw order.
    ///
    /// `fresh` is what the frame is about to ask for that it does not already
    /// hold, so a repack happens *instead of* running out of room rather than
    /// after it.
    pub(super) fn should_repack(&self, total: u32, fresh: u32) -> bool {
        self.breaks > break_budget(total)
            || self.len.saturating_add(fresh) > self.capacity
            || total > self.capacity
            // Half the arena is holes left by paragraphs that changed size or
            // went away. Nothing is wrong, but the buffer is twice the size it
            // needs to be and every new block lands further from its
            // neighbours.
            || (self.len > 64 && self.live * 2 < self.len)
    }

    /// Give back everything, keeping the buffer. For the case where no entry
    /// means what it meant and they are all dropped at once.
    pub(super) fn reset(&mut self) {
        self.len = 0;
        self.live = 0;
        self.free.clear();
        self.breaks = 0;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Start a repack: every offset handed out before this is void.
    pub(super) fn repack(&mut self, total: u32) {
        self.len = 0;
        self.live = 0;
        self.free.clear();
        self.breaks = 0;
        self.generation = self.generation.wrapping_add(1);
        if total > self.capacity {
            self.capacity = total.next_power_of_two().max(MIN_ARENA);
        } else if total.saturating_mul(4) < self.capacity {
            // Every block is about to be written again anyway, so this is
            // the one moment giving memory back costs nothing extra. A
            // quarter, not a half, so a set that hovers around a power of two
            // does not replace the buffer on every repack.
            self.capacity = total.saturating_mul(2).next_power_of_two().max(MIN_ARENA);
        }
    }

    pub(super) fn alloc(&mut self, capacity: u32) -> u32 {
        self.live += capacity;
        if let Some(offset) = self.free.get_mut(&capacity).and_then(Vec::pop) {
            return offset;
        }
        let offset = self.len;
        self.len += capacity;
        // Never reached: `should_repack` is asked before the walk with exactly
        // what the walk is about to allocate, and a repack is what makes room.
        // Growing here instead would replace the buffer after the frame had
        // already decided which blocks to write, and the ones it skipped would
        // not be on the new one.
        debug_assert!(
            self.len <= self.capacity,
            "the arena grew past the buffer the repack sized"
        );
        self.capacity = self
            .capacity
            .max(self.len.next_power_of_two())
            .max(MIN_ARENA);
        offset
    }

    /// Give a block back. Ignored when it belongs to an older layout, whose
    /// offsets the repack already forgot.
    pub(super) fn release(&mut self, generation: u64, offset: u32, capacity: u32) {
        if generation != self.generation {
            return;
        }
        self.live = self.live.saturating_sub(capacity);
        self.free.entry(capacity).or_default().push(offset);
    }
}

/// Row indices into the run table, handed out per entry and given back when it
/// retires.
#[derive(Default)]
pub(super) struct RunSlots {
    next: u32,
    free: Vec<u32>,
}

impl RunSlots {
    pub(super) fn alloc(&mut self) -> u32 {
        if let Some(slot) = self.free.pop() {
            return slot;
        }
        let slot = self.next;
        self.next += 1;
        slot
    }

    pub(super) fn release(&mut self, slot: u32) {
        self.free.push(slot);
    }

    /// Give back every row. For the case where every entry is dropped at once.
    pub(super) fn reset(&mut self) {
        self.next = 0;
        self.free.clear();
    }

    /// Rows the table must hold for every live slot to be addressable.
    pub(super) fn len(&self) -> usize {
        self.next as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(node: u64) -> EntryKey {
        EntryKey {
            node,
            slot: 0,
            pass: 0,
        }
    }

    /// Build `key` with `glyphs` live glyphs whose handles are `first..`.
    fn build(store: &mut EntryStore, key: EntryKey, glyphs: u32, first: u32) -> Vec<u32> {
        let mut released = Vec::new();
        let id = store.begin_build(key, glyphs, |handle| {
            released.push(handle);
        });
        for offset in 0..glyphs {
            store.push_glyph(
                id,
                offset,
                GlyphAtlasEntryId::for_test(first + offset),
                GlyphInstance::VACANT,
            );
        }
        store.finish_build(id, glyphs);
        released
            .into_iter()
            .map(|handle| {
                (0..u32::MAX)
                    .find(|index| GlyphAtlasEntryId::for_test(*index) == handle)
                    .expect("a test handle")
            })
            .collect()
    }

    #[test]
    fn a_rebuild_gives_back_every_claim_the_old_block_made() {
        let mut store = EntryStore::default();
        build(&mut store, key(1), 10, 100);
        let released = build(&mut store, key(1), 3, 200);
        assert_eq!(
            released,
            (100..110).collect::<Vec<_>>(),
            "a paragraph rebuilt shorter must release all ten glyphs it held, \
             or the atlas keeps counting claims nobody makes"
        );
        let released = build(&mut store, key(1), 12, 300);
        assert_eq!(
            released,
            (200..203).collect::<Vec<_>>(),
            "and rebuilt longer, only the three it held, never a vacant slot"
        );
    }

    #[test]
    fn a_paragraph_that_changes_length_within_its_slack_keeps_its_block() {
        let mut store = EntryStore::default();
        build(&mut store, key(1), 11, 0);
        let id = store.lookup(key(1)).expect("built");
        let (block, capacity) = {
            let entry = store.get(id).expect("live");
            (entry.block, entry.capacity)
        };
        for glyphs in [12, 10, capacity] {
            build(&mut store, key(1), glyphs, 0);
            let entry = store.get(id).expect("live");
            assert_eq!(
                (entry.block, entry.capacity),
                (block, capacity),
                "{glyphs} glyphs fit the block of {capacity} a paragraph of 11 was given"
            );
        }
        build(&mut store, key(1), capacity + 1, 0);
        let grown = {
            let entry = store.get(id).expect("live");
            (entry.block, entry.capacity)
        };
        assert!(
            grown.1 > capacity,
            "outgrowing it moves the paragraph to a bigger class"
        );
        build(&mut store, key(1), 1, 0);
        let entry = store.get(id).expect("live");
        assert_eq!(
            (entry.block, entry.capacity),
            grown,
            "a short label that shrinks again keeps its block: moving it would \
             cost more than the few slots it frees"
        );
        build(&mut store, key(1), 600, 0);
        build(&mut store, key(1), 20, 0);
        assert_eq!(
            store.get(id).expect("live").capacity,
            capacity_for(20),
            "but a paragraph that lost most of a page gives the space back"
        );
    }

    #[test]
    fn size_classes_are_monotonic_and_leave_the_slack_they_promise() {
        let mut previous = 0;
        for glyphs in 0..5000u32 {
            let capacity = capacity_for(glyphs);
            assert!(capacity >= glyphs + (glyphs / 8).max(4), "{glyphs}");
            assert!(capacity >= previous, "{glyphs}");
            // A class step is a quarter of the power of two below what was
            // asked for, so rounding up costs at most a quarter again.
            let need = glyphs + (glyphs / 8).max(4);
            assert!(
                capacity <= (need + need / 4).max(8),
                "{glyphs} -> {capacity}"
            );
            previous = capacity;
        }
    }

    #[test]
    fn retiring_most_of_the_slab_compacts_it_without_losing_a_survivor() {
        let mut store = EntryStore::default();
        for node in 0..400u64 {
            build(&mut store, key(node), 20, node as u32 * 100);
            let id = store.lookup(key(node)).expect("built");
            store.get_mut(id).expect("live").last_used = if node % 10 == 0 { 9 } else { 1 };
        }
        let before = store.instances.len();
        store.retire(5, |_| {}, |_| {}, |_, _, _| {});
        assert!(
            store.instances.len() * 4 < before,
            "nine in ten retired, so the slab must shrink: {before} -> {}",
            store.instances.len()
        );
        for node in (0..400u64).step_by(10) {
            let id = store.lookup(key(node)).expect("a survivor");
            let entry = store.get(id).expect("live");
            let handles = store
                .live_glyphs(entry)
                .map(|(_, handle)| handle)
                .collect::<Vec<_>>();
            let expected = (0..20)
                .map(|offset| GlyphAtlasEntryId::for_test(node as u32 * 100 + offset))
                .collect::<Vec<_>>();
            assert_eq!(handles, expected, "node {node} still holds its own glyphs");
        }
    }

    #[test]
    fn a_repack_after_the_text_went_away_gives_the_buffer_back() {
        let mut arena = InstanceArena::default();
        arena.repack(40_000);
        let peak = arena.capacity();
        assert!(peak >= 40_000);
        arena.repack(peak / 3);
        assert_eq!(
            arena.capacity(),
            peak,
            "a third of it is not worth a new buffer"
        );
        arena.repack(300);
        assert!(
            arena.capacity() < peak / 8,
            "a list that closed must not keep its GPU memory: {}",
            arena.capacity()
        );
        assert!(arena.capacity() >= 300);
    }
}
