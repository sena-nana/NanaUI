//! Shaped paragraph handle cache and FNV-1a shape keys.

use nana_text::TextLayout;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub(super) const SHAPE_CACHE_CAP: usize = 512;
/// Hard ceiling on shaped paragraphs, whatever the view asks for.
pub(super) const SHAPE_CACHE_MAX: usize = 32_768;
/// Frames a paragraph stays un-evictable after it was last drawn. Two, so a
/// painter alternating between two windows holds both of their text.
const PIN_FRAMES: u64 = 2;
/// How far along the LRU order one insert looks for a paragraph nothing wants.
/// The front is the oldest; if the oldest this many are all still on screen,
/// so is everything behind them.
const EVICT_PROBES: usize = 64;

/// Marks a paragraph identity as a Runtime layout handle rather than a hash of
/// this painter's own shape key.
///
/// The two share one `u64` — [`super::entry::TextGpuEntry::layout`] — and a value from one
/// must never read as a value from the other, because what it gates is whether
/// a set of resolved glyphs may be drawn again. The hash space is halved
/// instead: handles set this bit, [`ShapeKeyRef::hash64`] clears it.
pub(super) const PARAGRAPH_IS_RETAINED: u64 = 1 << 63;

/// One Runtime layout handle as a paragraph identity.
///
/// Exact, not hashed: the handle is two `u32`s and a generational slot is
/// never reissued at the same generation, so packing them *is* the identity.
/// The index is masked to 31 bits, which a store of two billion live layouts
/// would be needed to reach.
pub(super) fn retained_paragraph_id(id: nana_text::TextLayoutId) -> u64 {
    PARAGRAPH_IS_RETAINED | (u64::from(id.index() & 0x7fff_ffff) << 32) | u64::from(id.generation())
}

pub(super) struct ShapeEntry {
    pub(super) key: ShapeKey,
    pub(super) layout: Arc<TextLayout>,
    /// Frame this paragraph was last asked for.
    pub(super) last_used: u64,
}

/// Laid-out paragraphs keyed by [`ShapeKeyRef::hash64`].
///
/// A handle table, not a second layout authority: the values are `Arc`s to
/// layouts the engine's own cache produced, so what this bounds is how many
/// the painter keeps *reachable*, not how many exist.
///
/// The map is keyed by the hash rather than by an owned key so a repaint of
/// unchanged text looks up without copying the string, the family name, or the
/// rich-span list. The stored key still decides the hit, so a hash collision
/// between two different texts reshapes instead of painting the wrong glyphs.
#[derive(Default)]
pub(super) struct ShapeCache {
    /// Keyed by the shape hash, hashed by [`nana_ui_runtime::IdHasher`]:
    /// running SipHash over a word that is already a hash is work for nothing.
    pub(super) entries: HashMap<u64, ShapeEntry, nana_ui_runtime::BuildIdHasher>,
    pub(super) order: VecDeque<u64>,
    pub(super) frame: u64,
    pub(super) hits: usize,
    pub(super) misses: usize,
    pub(super) evictions: usize,
}

impl ShapeCache {
    pub(super) fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    pub(super) fn get(&mut self, hash: u64, key: &ShapeKeyRef<'_>) -> Option<&Arc<TextLayout>> {
        let frame = self.frame;
        match self.entries.get_mut(&hash) {
            Some(entry) if entry.key.matches(key) => {
                entry.last_used = frame;
                self.hits += 1;
                Some(&entry.layout)
            }
            _ => {
                self.misses += 1;
                None
            }
        }
    }

    /// Whether the cache still holds `hash`, counted and aged as a hit.
    ///
    /// The caller that uses this already knows the paragraph is the one it
    /// resolved from — the primitive has not been rewritten since — so the key
    /// comparison would only re-derive an answer it has.
    pub(super) fn holds(&mut self, hash: u64) -> bool {
        let frame = self.frame;
        match self.entries.get_mut(&hash) {
            Some(entry) => {
                entry.last_used = frame;
                self.hits += 1;
                true
            }
            None => {
                self.misses += 1;
                false
            }
        }
    }

    pub(super) fn layout(&self, hash: u64) -> Option<&Arc<TextLayout>> {
        self.entries.get(&hash).map(|entry| &entry.layout)
    }

    /// Drop every shaped paragraph. For the case where they are not merely
    /// cold but no longer meaningful, i.e. the face set changed under them.
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    /// Store a shaped paragraph, evicting the coldest paragraph nothing on
    /// screen still wants.
    ///
    /// A paragraph a recent frame asked for is never the victim. A view with
    /// more distinct strings on it than [`SHAPE_CACHE_CAP`] would otherwise
    /// evict the row it is about to draw to make room for the next one, and
    /// reshape every one of them on every frame — the cache would turn a
    /// steady frame into the most expensive kind there is. So the nominal
    /// capacity is a floor the cache shrinks back to once the view does, not a
    /// ceiling it enforces against the view in front of it.
    ///
    /// "Recent" rather than "this frame" because two windows on one painter
    /// take turns: what window A drew last frame is still on screen while
    /// window B is drawing, and the cache has to hold both.
    pub(super) fn insert(&mut self, hash: u64, key: ShapeKey, layout: Arc<TextLayout>) {
        let frame = self.frame;
        let pinned = frame.saturating_sub(PIN_FRAMES);
        let mut probes = 0;
        while self.entries.len() >= SHAPE_CACHE_CAP && probes < EVICT_PROBES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            probes += 1;
            match self.entries.get(&oldest) {
                Some(entry) if entry.last_used >= pinned => {
                    // Still on a screen. Put it back and look further along.
                    self.order.push_back(oldest);
                }
                Some(_) => {
                    self.entries.remove(&oldest);
                    self.evictions += 1;
                    probes = 0;
                }
                None => probes = 0,
            }
        }
        // A hard ceiling, in case a single view really does hold this much
        // text: the alternative is a cache that remembers every paragraph a
        // session ever drew.
        while self.entries.len() >= SHAPE_CACHE_MAX {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if self.entries.remove(&oldest).is_some() {
                self.evictions += 1;
            }
        }
        if self
            .entries
            .insert(
                hash,
                ShapeEntry {
                    key,
                    layout,
                    last_used: frame,
                },
            )
            .is_none()
        {
            self.order.push_back(hash);
        }
    }
}

/// FNV-1a over the shape key.
///
/// Not a general-purpose hasher and deliberately not the default one: this
/// runs once per text node per frame over the whole string, and it is the
/// single largest item left in the text path. What guards against a collision
/// is [`ShapeKey::matches`], which decides the hit — a collision costs a
/// reshape, never the wrong glyphs.
#[derive(Default)]
pub(super) struct ShapeHasher(u64);

impl Hasher for ShapeHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut state = if self.0 == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            self.0
        };
        for byte in bytes {
            state ^= u64::from(*byte);
            state = state.wrapping_mul(0x1000_0000_01b3);
        }
        self.0 = state;
    }
}

/// Everything that determines the laid-out paragraph.
///
/// Position and color are not here. Since #99 that includes *rich span*
/// colors: `nana-text` lays text out without knowing what paints it, so two
/// spellings of the same string in different colors are one layout and the
/// colors are resolved onto glyphs afterwards.
pub(super) struct ShapeKey {
    pub(super) content: String,
    pub(super) family: Option<String>,
    pub(super) weight: Option<u16>,
    pub(super) font_size_bits: u32,
    pub(super) line_height_bits: u32,
    pub(super) wrap: bool,
    pub(super) wrap_break: nana_ui_core::TextWrapBreak,
    pub(super) italic: bool,
    pub(super) ellipsis: bool,
    pub(super) max_lines: Option<u16>,
    pub(super) letter_spacing_bits: u32,
    pub(super) word_break: u8,
    pub(super) line_break: u8,
    pub(super) kerning: u8,
    pub(super) features: Vec<nana_ui_core::FontFeatureSetting>,
    pub(super) variations: Vec<nana_ui_core::FontVariationSetting>,
    pub(super) width_bits: u32,
    pub(super) height_bits: u32,
    pub(super) align: u8,
    pub(super) direction: u8,
    pub(super) writing_mode: u8,
    pub(super) text_orientation: u8,
    pub(super) preserve_lines: bool,
    pub(super) font_features: Vec<nana_ui_core::FontFeatureSetting>,
    pub(super) typography: nana_ui_core::UsedTextTypography,
}

/// Borrowed form of [`ShapeKey`] built straight from the scene primitive.
///
/// Hashing and comparison live here so a frame can answer "already shaped?"
/// without owning any of it.
pub(super) struct ShapeKeyRef<'a> {
    pub(super) content: &'a str,
    pub(super) family: Option<&'a str>,
    pub(super) weight: Option<u16>,
    pub(super) font_size_bits: u32,
    pub(super) line_height_bits: u32,
    pub(super) wrap: bool,
    pub(super) wrap_break: nana_ui_core::TextWrapBreak,
    pub(super) italic: bool,
    pub(super) ellipsis: bool,
    pub(super) max_lines: Option<u16>,
    pub(super) letter_spacing_bits: u32,
    pub(super) word_break: u8,
    pub(super) line_break: u8,
    pub(super) kerning: u8,
    pub(super) features: &'a [nana_ui_core::FontFeatureSetting],
    pub(super) variations: &'a [nana_ui_core::FontVariationSetting],
    pub(super) width_bits: u32,
    pub(super) height_bits: u32,
    pub(super) align: u8,
    pub(super) direction: u8,
    pub(super) writing_mode: u8,
    pub(super) text_orientation: u8,
    pub(super) preserve_lines: bool,
    pub(super) font_features: &'a [nana_ui_core::FontFeatureSetting],
    pub(super) typography: nana_ui_core::UsedTextTypography,
}

impl ShapeKeyRef<'_> {
    pub(super) fn hash64(&self) -> u64 {
        let mut hasher = ShapeHasher::default();
        self.content.hash(&mut hasher);
        self.family.hash(&mut hasher);
        self.weight.hash(&mut hasher);
        self.font_size_bits.hash(&mut hasher);
        self.line_height_bits.hash(&mut hasher);
        self.wrap.hash(&mut hasher);
        self.wrap_break.hash(&mut hasher);
        self.italic.hash(&mut hasher);
        self.ellipsis.hash(&mut hasher);
        self.max_lines.hash(&mut hasher);
        self.letter_spacing_bits.hash(&mut hasher);
        self.word_break.hash(&mut hasher);
        self.line_break.hash(&mut hasher);
        self.kerning.hash(&mut hasher);
        self.features.hash(&mut hasher);
        self.variations.hash(&mut hasher);
        self.width_bits.hash(&mut hasher);
        self.height_bits.hash(&mut hasher);
        self.align.hash(&mut hasher);
        self.font_features.hash(&mut hasher);
        self.direction.hash(&mut hasher);
        self.writing_mode.hash(&mut hasher);
        self.text_orientation.hash(&mut hasher);
        self.preserve_lines.hash(&mut hasher);
        self.typography.hash(&mut hasher);
        // The top bit belongs to [`PARAGRAPH_IS_RETAINED`], so a hash can
        // never be mistaken for a Runtime handle. What this costs is one bit
        // of a hash whose collisions are already caught by
        // [`ShapeKey::matches`] and cost a relayout, never wrong glyphs.
        hasher.finish() & !PARAGRAPH_IS_RETAINED
    }

    pub(super) fn to_owned_key(&self) -> ShapeKey {
        ShapeKey {
            content: self.content.to_owned(),
            family: self.family.map(str::to_owned),
            weight: self.weight,
            font_size_bits: self.font_size_bits,
            line_height_bits: self.line_height_bits,
            wrap: self.wrap,
            wrap_break: self.wrap_break,
            italic: self.italic,
            ellipsis: self.ellipsis,
            max_lines: self.max_lines,
            letter_spacing_bits: self.letter_spacing_bits,
            word_break: self.word_break,
            line_break: self.line_break,
            kerning: self.kerning,
            features: self.features.to_vec(),
            variations: self.variations.to_vec(),
            width_bits: self.width_bits,
            height_bits: self.height_bits,
            align: self.align,
            font_features: self.font_features.to_vec(),
            direction: self.direction,
            writing_mode: self.writing_mode,
            text_orientation: self.text_orientation,
            preserve_lines: self.preserve_lines,
            typography: self.typography,
        }
    }
}

impl ShapeKey {
    pub(super) fn matches(&self, other: &ShapeKeyRef<'_>) -> bool {
        self.content == other.content
            && self.family.as_deref() == other.family
            && self.weight == other.weight
            && self.font_size_bits == other.font_size_bits
            && self.line_height_bits == other.line_height_bits
            && self.wrap == other.wrap
            && self.wrap_break == other.wrap_break
            && self.italic == other.italic
            && self.ellipsis == other.ellipsis
            && self.max_lines == other.max_lines
            && self.letter_spacing_bits == other.letter_spacing_bits
            && self.word_break == other.word_break
            && self.line_break == other.line_break
            && self.kerning == other.kerning
            && self.features == other.features
            && self.variations == other.variations
            && self.width_bits == other.width_bits
            && self.height_bits == other.height_bits
            && self.align == other.align
            && self.font_features == other.font_features
            && self.direction == other.direction
            && self.writing_mode == other.writing_mode
            && self.text_orientation == other.text_orientation
            && self.preserve_lines == other.preserve_lines
            && self.typography == other.typography
    }
}

pub(super) fn opentype_disc(word_break: nana_ui_core::WordBreakSpec) -> u8 {
    match word_break {
        nana_ui_core::WordBreakSpec::Normal => 0,
        nana_ui_core::WordBreakSpec::BreakAll => 1,
        nana_ui_core::WordBreakSpec::BreakWord => 2,
    }
}

pub(super) fn opentype_line_disc(line_break: nana_ui_core::LineBreakSpec) -> u8 {
    match line_break {
        nana_ui_core::LineBreakSpec::Auto => 0,
        nana_ui_core::LineBreakSpec::Normal => 1,
        nana_ui_core::LineBreakSpec::Anywhere => 2,
        nana_ui_core::LineBreakSpec::Strict => 3,
        nana_ui_core::LineBreakSpec::Loose => 4,
    }
}

pub(super) fn opentype_kern_disc(kerning: nana_ui_core::FontKerningSpec) -> u8 {
    match kerning {
        nana_ui_core::FontKerningSpec::Auto => 0,
        nana_ui_core::FontKerningSpec::Normal => 1,
        nana_ui_core::FontKerningSpec::None => 2,
    }
}
