//! The font database `nana-text` owns: registration, generations, selection,
//! instances and coverage-driven fallback.
//!
//! # Ownership and lifetime
//!
//! A face's bytes are an [`FontBlob`] (`Arc`). [`FontSystem::face_data`] hands
//! out a clone, so a shaping or raster worker holding [`FontData`] keeps the
//! bytes alive across any later mutation — unregistering a face retires its
//! handle, not the bytes a worker is still reading. When the last clone goes,
//! the bytes go; [`FontSystem::retired_font_data_alive`] makes that moment
//! observable. System faces are read from disk on first use and then held the
//! same way.
//!
//! Mutation is `&mut self`. The system is not a global and holds no lock; a
//! host that shares one wraps it in whatever synchronization its threading
//! model needs, and workers need only `FontData`, which is `Send + Sync`.
//!
//! # Generations
//!
//! Every mutation — registering, unregistering or replacing a source, loading
//! system fonts, swapping the fallback policy — bumps [`FontGeneration`] once.
//! Selections and instances carry the generation they were computed under.
//! Invalidation is exact in both directions:
//!
//! - the selection cache is cleared, because adding *any* face can change
//!   which face a family list resolves to;
//! - coverage entries are dropped only for faces that were removed, because a
//!   face's cmap does not change when an unrelated face is registered;
//! - a retired [`FontId`] is never reissued at the same slot generation, so a
//!   shape or raster cache keyed on [`FontInstanceKey`](super::FontInstanceKey)
//!   can never alias a replacement face.

use super::coverage::{CoverageCache, CoverageLookup, CoverageSet, DEFAULT_COVERAGE_BUDGET_BYTES};
use super::discovery::{self, DiscoveredSource, FaceMeta};
use super::face::{self, FaceDetails};
use super::fallback::{FallbackPolicy, FontAssignment, FontChoiceReason};
use super::matching::{self, MatchFace, StyleSupport};
use super::query::{FamilyName, FontQuery, FontStyle, GenericFamily, LanguageTag};
use super::unicode;
use super::variations::{
    FontAxis, FontInstance, FontVariations, ITAL, NamedInstance, SLNT, StaticFaceTraits, WDTH,
    WGHT, resolve_instance,
};
use crate::id::{FontGeneration, FontId, FontSourceId};
use crate::metrics::RunMetrics;
use crate::shape::ScriptTag;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};

/// Shared, immutable font bytes.
pub type FontBlob = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// Wraps owned or `'static` bytes as a [`FontBlob`].
pub fn font_blob(bytes: impl AsRef<[u8]> + Send + Sync + 'static) -> FontBlob {
    Arc::new(bytes)
}

/// Where a registration came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FontOrigin {
    /// The platform font directories.
    System,
    /// A file a host or app registered explicitly.
    File,
    /// Bytes a host, app or test registered.
    Memory,
}

impl FontOrigin {
    /// Tie-break rank: explicit registrations shadow the system scan.
    fn rank(self) -> u8 {
        match self {
            Self::System => 0,
            Self::File | Self::Memory => 1,
        }
    }
}

/// CSS `@font-face` descriptors applied to every face of a registration.
///
/// A declared `family` **replaces** the face's own names for matching, as
/// `@font-face` does: the face answers to the declared family only. Ranges are
/// inclusive; a single value is a one-point range.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FaceDescriptor {
    #[serde(default)]
    pub family: Option<Arc<str>>,
    #[serde(default)]
    pub weight: Option<(f32, f32)>,
    #[serde(default)]
    pub stretch: Option<(f32, f32)>,
    #[serde(default)]
    pub style: Option<FontStyle>,
}

impl FaceDescriptor {
    pub fn family(name: &str) -> Self {
        Self {
            family: Some(Arc::from(name)),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontError {
    /// The byte buffer was empty.
    Empty,
    /// The bytes are not a font face either metadata reader accepts.
    Unrecognized,
    /// Reading a file failed (`Display` of the I/O error).
    Io(String),
    /// The source id is null, retired, or was never issued.
    UnknownSource,
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "font bytes were empty"),
            Self::Unrecognized => write!(f, "bytes are not a recognized font face"),
            Self::Io(error) => write!(f, "font file: {error}"),
            Self::UnknownSource => write!(f, "font source is not registered"),
        }
    }
}

impl std::error::Error for FontError {}

/// What one registration produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontRegistration {
    pub source: FontSourceId,
    /// In collection-index order.
    pub faces: Vec<FontId>,
    pub generation: FontGeneration,
}

/// A face's bytes plus its index in them, independent of the registry.
#[derive(Clone)]
pub struct FontData {
    blob: FontBlob,
    index: u32,
}

impl FontData {
    pub fn bytes(&self) -> &[u8] {
        (*self.blob).as_ref()
    }

    /// Index within a collection (`.ttc`); 0 for a single-face file.
    pub fn index(&self) -> u32 {
        self.index
    }
}

impl std::fmt::Debug for FontData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontData")
            .field("len", &self.bytes().len())
            .field("index", &self.index)
            .finish()
    }
}

/// Everything the font system knows about one face, for diagnostics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceDescription {
    pub id: FontId,
    pub source: FontSourceId,
    pub origin: FontOrigin,
    pub families: Vec<Arc<str>>,
    pub post_script_name: Arc<str>,
    pub path: Option<PathBuf>,
    pub index: u32,
    /// Inclusive CSS weight range matching uses.
    pub weight: (f32, f32),
    /// Inclusive width range (percent) matching uses.
    pub stretch: (f32, f32),
    pub style: FontStyle,
    pub monospaced: bool,
    pub color_glyphs: bool,
    pub axes: Vec<FontAxis>,
    pub named_instances: Vec<NamedInstance>,
}

/// The family list resolved against the database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontSelection {
    pub query: FontQuery,
    /// The first face any family resolved to. `None` only when nothing in the
    /// list (generics included) exists.
    pub primary: Option<FontId>,
    /// Every further distinct face the list resolved to, in list order.
    pub fallback_chain: Vec<FontId>,
    pub generation: FontGeneration,
    /// How each family name was resolved, in the order tried.
    pub families: Vec<FamilyResolution>,
}

impl FontSelection {
    /// Primary, then fallback chain.
    pub fn chain(&self) -> impl Iterator<Item = FontId> + '_ {
        self.primary
            .iter()
            .copied()
            .chain(self.fallback_chain.iter().copied())
    }
}

/// One family name tried while resolving a selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FamilyResolution {
    pub requested: FamilyName,
    /// The concrete family name looked up (a generic expands to several).
    pub name: Arc<str>,
    pub face: Option<FontId>,
}

/// Font-system work counts.
///
/// `font_faces_registered` and `font_generation` are gauges read at the time
/// [`FontSystem::counters`] is called; everything else accumulates until
/// [`FontSystem::reset_counters`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FontCounters {
    pub font_faces_registered: usize,
    pub font_generation: u64,
    pub font_query_hits: usize,
    pub font_query_misses: usize,
    /// Faces probed for coverage beyond a cluster's primary face.
    pub fallback_candidates_examined: usize,
    pub coverage_cache_hits: usize,
    pub coverage_cache_misses: usize,
    pub coverage_cache_evictions: usize,
    /// Clusters the primary face did not cover.
    pub font_fallback_attempts: usize,
    /// Of those, clusters no candidate covered.
    pub font_fallback_misses: usize,
}

enum FaceBytes {
    Loaded(FontBlob),
    File(PathBuf, OnceLock<Option<FontBlob>>),
}

struct FaceRecord {
    id: FontId,
    source: FontSourceId,
    origin: FontOrigin,
    registration: u64,
    meta: FaceMeta,
    descriptor: FaceDescriptor,
    bytes: FaceBytes,
    details: OnceLock<FaceDetails>,
}

impl FaceRecord {
    fn blob(&self) -> Option<FontBlob> {
        match &self.bytes {
            FaceBytes::Loaded(blob) => Some(Arc::clone(blob)),
            FaceBytes::File(path, loaded) => loaded
                .get_or_init(|| std::fs::read(path).ok().map(font_blob))
                .clone(),
        }
    }

    fn loaded_blob(&self) -> Option<FontBlob> {
        match &self.bytes {
            FaceBytes::Loaded(blob) => Some(Arc::clone(blob)),
            FaceBytes::File(_, loaded) => loaded.get().cloned().flatten(),
        }
    }

    fn details(&self) -> &FaceDetails {
        self.details.get_or_init(|| {
            self.blob()
                .and_then(|blob| face::read_details((*blob).as_ref(), self.meta.index))
                .unwrap_or_default()
        })
    }

    fn families(&self) -> Vec<Arc<str>> {
        match &self.descriptor.family {
            Some(family) => vec![Arc::clone(family)],
            None => self.meta.families.clone(),
        }
    }

    fn weight_range(&self) -> (f32, f32) {
        if let Some(range) = self.descriptor.weight {
            return ordered(range);
        }
        match self.details().axis(WGHT) {
            Some(axis) => (axis.min, axis.max),
            None => (self.meta.weight, self.meta.weight),
        }
    }

    fn stretch_range(&self) -> (f32, f32) {
        if let Some(range) = self.descriptor.stretch {
            return ordered(range);
        }
        match self.details().axis(WDTH) {
            Some(axis) => (axis.min, axis.max),
            None => (self.meta.stretch, self.meta.stretch),
        }
    }

    fn static_style(&self) -> FontStyle {
        self.descriptor.style.unwrap_or(self.meta.style)
    }

    fn styles(&self) -> StyleSupport {
        let mut styles = StyleSupport::default();
        match self.static_style() {
            FontStyle::Normal => styles.normal = true,
            FontStyle::Italic => styles.italic = true,
            FontStyle::Oblique => styles.oblique = true,
        }
        if self.descriptor.style.is_none() {
            let details = self.details();
            if details
                .axis(ITAL)
                .is_some_and(|axis| axis.min <= 0.0 && axis.max >= 1.0)
            {
                styles.normal = true;
                styles.italic = true;
            }
            if details
                .axis(SLNT)
                .is_some_and(|axis| axis.min < 0.0 && axis.max >= 0.0)
            {
                styles.normal = true;
                styles.oblique = true;
            }
        }
        styles
    }

    fn match_face(&self) -> MatchFace {
        MatchFace {
            id: self.id,
            weight: self.weight_range(),
            stretch: self.stretch_range(),
            styles: self.styles(),
            origin_rank: self.origin.rank(),
            registration: self.registration,
        }
    }

    fn describe(&self) -> FaceDescription {
        let details = self.details();
        FaceDescription {
            id: self.id,
            source: self.source,
            origin: self.origin,
            families: self.families(),
            post_script_name: Arc::clone(&self.meta.post_script_name),
            path: match &self.bytes {
                FaceBytes::File(path, _) => Some(path.clone()),
                FaceBytes::Loaded(_) => None,
            },
            index: self.meta.index,
            weight: self.weight_range(),
            stretch: self.stretch_range(),
            style: self.static_style(),
            monospaced: self.meta.monospaced,
            color_glyphs: details.color.any(),
            axes: details.axes.clone(),
            named_instances: details.named_instances.clone(),
        }
    }
}

fn ordered((a, b): (f32, f32)) -> (f32, f32) {
    if a <= b { (a, b) } else { (b, a) }
}

#[derive(Default)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// A generational slot arena with a LIFO free list, so reissue order is
/// deterministic.
struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
}

impl<T> Arena<T> {
    const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    fn insert(&mut self, value: T) -> (u32, u32) {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.generation += 1;
            slot.value = Some(value);
            return (index, slot.generation);
        }
        let index = u32::try_from(self.slots.len()).expect("fewer than 2^32 font slots");
        self.slots.push(Slot {
            generation: 1,
            value: Some(value),
        });
        (index, 1)
    }

    fn get(&self, index: u32, generation: u32) -> Option<&T> {
        let slot = self.slots.get(index as usize)?;
        (generation != 0 && slot.generation == generation)
            .then_some(())
            .and(slot.value.as_ref())
    }

    fn remove(&mut self, index: u32, generation: u32) -> Option<T> {
        let slot = self.slots.get_mut(index as usize)?;
        if generation == 0 || slot.generation != generation {
            return None;
        }
        let value = slot.value.take()?;
        self.free.push(index);
        Some(value)
    }

    fn live(&self) -> impl Iterator<Item = &T> {
        self.slots.iter().filter_map(|slot| slot.value.as_ref())
    }
}

struct SourceRecord {
    faces: Vec<FontId>,
}

/// Pending face before it gets an id.
struct NewFace {
    meta: FaceMeta,
    bytes: FaceBytes,
    origin: FontOrigin,
}

pub struct FontSystem {
    /// Process-unique identity. `FontId`s and generations are numbered per
    /// system from zero, so anything caching them across calls (the shaper)
    /// has to know which system they came from.
    instance: u64,
    faces: Arena<Arc<FaceRecord>>,
    sources: Arena<SourceRecord>,
    /// Lower-cased family name -> faces, in registration order.
    families: BTreeMap<String, Vec<FontId>>,
    generation: FontGeneration,
    registrations: u64,
    policy: FallbackPolicy,
    selections: HashMap<FontQuery, Arc<FontSelection>>,
    coverage: CoverageCache,
    counters: FontCounters,
    retired: Vec<Weak<dyn AsRef<[u8]> + Send + Sync>>,
}

impl Default for FontSystem {
    fn default() -> Self {
        Self::hermetic()
    }
}

impl FontSystem {
    /// No faces and an empty policy. Deterministic on every machine.
    pub fn hermetic() -> Self {
        Self::with_policy(FallbackPolicy::empty())
    }

    pub fn with_policy(policy: FallbackPolicy) -> Self {
        static NEXT_INSTANCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            instance: NEXT_INSTANCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            faces: Arena::new(),
            sources: Arena::new(),
            families: BTreeMap::new(),
            generation: FontGeneration::default(),
            registrations: 0,
            policy,
            selections: HashMap::new(),
            coverage: CoverageCache::new(DEFAULT_COVERAGE_BUDGET_BYTES),
            counters: FontCounters::default(),
            retired: Vec::new(),
        }
    }

    /// Platform fonts plus the platform fallback policy.
    pub fn with_system_fonts() -> Self {
        let mut system = Self::with_policy(FallbackPolicy::platform_default());
        system.load_system_fonts();
        system
    }

    pub fn generation(&self) -> FontGeneration {
        self.generation
    }

    /// Which system this is, for caches that outlive one call.
    pub(crate) fn instance_id(&self) -> u64 {
        self.instance
    }

    pub fn policy(&self) -> &FallbackPolicy {
        &self.policy
    }

    pub fn set_policy(&mut self, policy: FallbackPolicy) {
        self.policy = policy;
        self.bump();
    }

    /// True when `selection` was computed under the current generation.
    pub fn is_current(&self, selection: &FontSelection) -> bool {
        selection.generation == self.generation
    }

    fn bump(&mut self) {
        self.generation = self.generation.bumped();
        self.selections.clear();
    }

    // ---- registration --------------------------------------------------

    fn parse_blob(blob: &FontBlob) -> Result<Vec<FaceMeta>, FontError> {
        if (**blob).as_ref().is_empty() {
            return Err(FontError::Empty);
        }
        let faces = discovery::parse_faces(blob);
        if faces.is_empty() {
            return Err(FontError::Unrecognized);
        }
        Ok(faces)
    }

    fn insert_faces(
        &mut self,
        faces: Vec<NewFace>,
        descriptor: &FaceDescriptor,
    ) -> FontRegistration {
        let (source_index, source_generation) =
            self.sources.insert(SourceRecord { faces: Vec::new() });
        let source = FontSourceId::from_parts(source_index, source_generation);
        let mut ids = Vec::with_capacity(faces.len());
        for new in faces {
            self.registrations += 1;
            let registration = self.registrations;
            // The id is only known after insertion, so insert a placeholder
            // record and fill it in place.
            let record = FaceRecord {
                id: FontId::NULL,
                source,
                origin: new.origin,
                registration,
                meta: new.meta,
                descriptor: descriptor.clone(),
                bytes: new.bytes,
                details: OnceLock::new(),
            };
            let (index, generation) = self.faces.insert(Arc::new(record));
            let id = FontId::from_parts(index, generation);
            let slot = self.faces.slots[index as usize]
                .value
                .as_mut()
                .expect("just inserted");
            Arc::get_mut(slot).expect("not yet shared").id = id;
            if matches!(slot.bytes, FaceBytes::Loaded(_)) {
                // Bytes are in hand: read details now so a malformed table
                // costs the registration, not the first frame that uses it.
                let _ = slot.details();
            }
            for family in slot.families() {
                self.families
                    .entry(family.to_ascii_lowercase())
                    .or_default()
                    .push(id);
            }
            ids.push(id);
        }
        self.sources.slots[source_index as usize]
            .value
            .as_mut()
            .expect("just inserted")
            .faces = ids.clone();
        FontRegistration {
            source,
            faces: ids,
            generation: self.generation,
        }
    }

    fn registered(&mut self, mut registration: FontRegistration) -> FontRegistration {
        self.bump();
        registration.generation = self.generation;
        registration
    }

    /// Registers every face in `data`.
    pub fn register_bytes(
        &mut self,
        data: FontBlob,
        descriptor: &FaceDescriptor,
    ) -> Result<FontRegistration, FontError> {
        let metas = Self::parse_blob(&data)?;
        let faces = metas
            .into_iter()
            .map(|meta| NewFace {
                meta,
                bytes: FaceBytes::Loaded(Arc::clone(&data)),
                origin: FontOrigin::Memory,
            })
            .collect();
        let registration = self.insert_faces(faces, descriptor);
        Ok(self.registered(registration))
    }

    /// Reads and registers a font file. The bytes are read now, so a missing
    /// or malformed file fails here rather than at first use.
    pub fn register_file(
        &mut self,
        path: impl AsRef<Path>,
        descriptor: &FaceDescriptor,
    ) -> Result<FontRegistration, FontError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|error| FontError::Io(error.to_string()))?;
        let blob = font_blob(bytes);
        let metas = Self::parse_blob(&blob)?;
        let faces = metas
            .into_iter()
            .map(|meta| {
                let loaded = OnceLock::new();
                let _ = loaded.set(Some(Arc::clone(&blob)));
                NewFace {
                    meta,
                    bytes: FaceBytes::File(path.to_path_buf(), loaded),
                    origin: FontOrigin::File,
                }
            })
            .collect();
        let registration = self.insert_faces(faces, descriptor);
        Ok(self.registered(registration))
    }

    /// Scans the platform font directories as one source. Bytes are read
    /// lazily, the first time a face's details or coverage are needed.
    pub fn load_system_fonts(&mut self) -> FontRegistration {
        let faces = discovery::system_faces()
            .into_iter()
            .map(|(source, meta)| NewFace {
                meta,
                bytes: match source {
                    DiscoveredSource::File(path) => FaceBytes::File(path, OnceLock::new()),
                    DiscoveredSource::Blob(blob) => FaceBytes::Loaded(blob),
                },
                origin: FontOrigin::System,
            })
            .collect();
        let registration = self.insert_faces(faces, &FaceDescriptor::default());
        self.registered(registration)
    }

    fn remove_source(&mut self, source: FontSourceId) -> Result<(), FontError> {
        let record = self
            .sources
            .remove(source.index(), source.generation())
            .ok_or(FontError::UnknownSource)?;
        for id in record.faces {
            let Some(face) = self.faces.remove(id.index(), id.generation()) else {
                continue;
            };
            for family in face.families() {
                let key = family.to_ascii_lowercase();
                if let Some(ids) = self.families.get_mut(&key) {
                    ids.retain(|known| *known != id);
                    if ids.is_empty() {
                        self.families.remove(&key);
                    }
                }
            }
            self.coverage.forget(id);
            if let Some(blob) = face.loaded_blob() {
                // Faces of one collection share a blob; count the bytes once.
                let weak = Arc::downgrade(&blob);
                self.retired
                    .retain(|known| known.strong_count() > 0 && !Weak::ptr_eq(known, &weak));
                self.retired.push(weak);
            }
        }
        Ok(())
    }

    /// Unregisters every face of `source`. Handles to them become stale; bytes
    /// a worker still holds through [`FontData`] stay valid.
    pub fn unregister(&mut self, source: FontSourceId) -> Result<FontGeneration, FontError> {
        self.remove_source(source)?;
        self.bump();
        Ok(self.generation)
    }

    /// Replaces `source` with new bytes in one generation step. The new bytes
    /// are validated first; on error the old registration is left untouched.
    pub fn replace_bytes(
        &mut self,
        source: FontSourceId,
        data: FontBlob,
        descriptor: &FaceDescriptor,
    ) -> Result<FontRegistration, FontError> {
        if self
            .sources
            .get(source.index(), source.generation())
            .is_none()
        {
            return Err(FontError::UnknownSource);
        }
        let metas = Self::parse_blob(&data)?;
        self.remove_source(source)?;
        let faces = metas
            .into_iter()
            .map(|meta| NewFace {
                meta,
                bytes: FaceBytes::Loaded(Arc::clone(&data)),
                origin: FontOrigin::Memory,
            })
            .collect();
        let registration = self.insert_faces(faces, descriptor);
        Ok(self.registered(registration))
    }

    // ---- lookup --------------------------------------------------------

    fn record(&self, font: FontId) -> Option<&Arc<FaceRecord>> {
        self.faces.get(font.index(), font.generation())
    }

    pub fn contains(&self, font: FontId) -> bool {
        self.record(font).is_some()
    }

    /// Live faces in slot order.
    pub fn faces(&self) -> Vec<FontId> {
        self.faces.live().map(|face| face.id).collect()
    }

    pub fn face_count(&self) -> usize {
        self.faces.live().count()
    }

    pub fn describe(&self, font: FontId) -> Option<FaceDescription> {
        self.record(font).map(|face| face.describe())
    }

    /// The face's bytes, detached from the registry's lifetime.
    pub fn face_data(&self, font: FontId) -> Option<FontData> {
        let face = self.record(font)?;
        Some(FontData {
            blob: face.blob()?,
            index: face.meta.index,
        })
    }

    /// Retired face bytes some [`FontData`] holder still keeps alive.
    pub fn retired_font_data_alive(&mut self) -> usize {
        self.retired.retain(|weak| weak.strong_count() > 0);
        self.retired.len()
    }

    fn best_face_in_family(&self, name: &str, query: &FontQuery) -> Option<FontId> {
        let ids = self.families.get(&name.to_ascii_lowercase())?;
        let candidates: Vec<MatchFace> = ids
            .iter()
            .filter_map(|id| self.record(*id))
            .map(|face| face.match_face())
            .collect();
        matching::best_face(&candidates, query.weight, query.stretch, query.style)
    }

    fn generic_names(&self, generic: GenericFamily) -> Vec<Arc<str>> {
        self.policy.generic(generic).to_vec()
    }

    /// Resolves a family list. Cached per query until the next generation.
    pub fn select(&mut self, query: &FontQuery) -> Arc<FontSelection> {
        if let Some(selection) = self.selections.get(query) {
            self.counters.font_query_hits += 1;
            return Arc::clone(selection);
        }
        self.counters.font_query_misses += 1;
        let mut chain: Vec<FontId> = Vec::new();
        let mut families = Vec::new();
        for requested in query.families.as_slice() {
            let names = match requested {
                FamilyName::Named(name) => vec![Arc::clone(name)],
                FamilyName::Generic(generic) => self.generic_names(*generic),
            };
            for name in names {
                let face = self.best_face_in_family(&name, query);
                if let Some(face) = face
                    && !chain.contains(&face)
                {
                    chain.push(face);
                }
                families.push(FamilyResolution {
                    requested: requested.clone(),
                    name,
                    face,
                });
            }
        }
        let primary = (!chain.is_empty()).then(|| chain.remove(0));
        let selection = Arc::new(FontSelection {
            query: query.clone(),
            primary,
            fallback_chain: chain,
            generation: self.generation,
            families,
        });
        self.selections
            .insert(query.clone(), Arc::clone(&selection));
        selection
    }

    /// Resolves `font` to concrete axis coordinates for `query` and explicit
    /// `variations`. `None` for a stale or unknown face.
    pub fn instance(
        &self,
        font: FontId,
        query: &FontQuery,
        variations: &FontVariations,
    ) -> Option<FontInstance> {
        let face = self.record(font)?;
        Some(resolve_instance(
            font,
            self.generation,
            face.details(),
            StaticFaceTraits {
                weight_max: face.weight_range().1,
                style: face.static_style(),
            },
            query,
            variations,
        ))
    }

    // ---- coverage and fallback ------------------------------------------

    fn coverage_of(&mut self, font: FontId) -> Option<Arc<CoverageSet>> {
        let face = Arc::clone(self.record(font)?);
        let evictions_before = self.coverage.evictions();
        let (set, lookup) = self.coverage.get_or_insert_with(font, || {
            face.blob()
                .map(|blob| face::read_coverage((*blob).as_ref(), face.meta.index))
                .unwrap_or_default()
        });
        match lookup {
            CoverageLookup::Hit => self.counters.coverage_cache_hits += 1,
            CoverageLookup::Miss => self.counters.coverage_cache_misses += 1,
        }
        self.counters.coverage_cache_evictions += self.coverage.evictions() - evictions_before;
        Some(set)
    }

    /// True when `font` maps `ch` to a real glyph.
    pub fn covers(&mut self, font: FontId, ch: char) -> bool {
        self.coverage_of(font)
            .is_some_and(|set| set.contains(u32::from(ch)))
    }

    fn covers_all(&mut self, font: FontId, codepoints: &[char]) -> bool {
        match self.coverage_of(font) {
            Some(set) => codepoints.iter().all(|ch| set.contains(u32::from(*ch))),
            None => false,
        }
    }

    fn has_color_glyphs(&self, font: FontId) -> bool {
        self.record(font)
            .is_some_and(|face| face.details().color.any())
    }

    pub fn set_coverage_budget(&mut self, budget_bytes: usize) {
        let before = self.coverage.evictions();
        self.coverage.set_budget(budget_bytes);
        self.counters.coverage_cache_evictions += self.coverage.evictions() - before;
    }

    pub fn coverage_budget_bytes(&self) -> usize {
        self.coverage.budget_bytes()
    }

    pub fn coverage_cache_bytes(&self) -> usize {
        self.coverage.used_bytes()
    }

    pub fn coverage_cache_len(&self) -> usize {
        self.coverage.len()
    }

    /// Assigns each grapheme cluster of `text` a face and the reason for it,
    /// merging adjacent clusters that share both.
    ///
    /// `language` overrides the selection query's language hint.
    ///
    /// Candidate order for a cluster:
    ///
    /// - **emoji presentation**: chain faces with colour glyphs, then the emoji
    ///   policy, then the rest of the chain;
    /// - **otherwise**: the chain in order (primary first);
    ///
    /// then, for both, the script policy for the cluster's script (or the
    /// script of the preceding specific-script cluster, so punctuation after
    /// CJK looks for CJK faces), then — for clusters with no script of their
    /// own — the symbol and emoji policies, then the last-resort list.
    pub fn resolve_text(
        &mut self,
        selection: &FontSelection,
        text: &str,
        language: Option<&LanguageTag>,
    ) -> Vec<FontAssignment> {
        let language = language.or(selection.query.language.as_ref()).cloned();
        let mut family_faces: HashMap<Arc<str>, Option<FontId>> = HashMap::new();
        let mut assignments: Vec<FontAssignment> = Vec::new();
        let mut inherited_script: Option<ScriptTag> = None;

        for cluster in unicode::clusters(text) {
            if cluster.script.is_some() {
                inherited_script = cluster.script;
            }
            let script = cluster.script.or(inherited_script);
            let codepoints: Vec<char> = text[cluster.range.clone()]
                .chars()
                .filter(|ch| !unicode::is_default_ignorable(*ch))
                .collect();

            let (font, reason) = if codepoints.is_empty() {
                match assignments.last() {
                    Some(previous) => (previous.font, FontChoiceReason::Ignorable),
                    None => (selection.primary, FontChoiceReason::Ignorable),
                }
            } else {
                self.choose_face(
                    selection,
                    &codepoints,
                    cluster.emoji,
                    cluster.script,
                    script,
                    language.as_ref(),
                    &mut family_faces,
                )
            };

            let assignment = FontAssignment {
                range: cluster.range,
                font,
                reason,
                script,
                emoji: cluster.emoji,
            };
            match assignments.last_mut() {
                Some(previous)
                    if previous.range.end == assignment.range.start
                        && previous.font == assignment.font
                        && (previous.reason == assignment.reason
                            || assignment.reason == FontChoiceReason::Ignorable)
                        && previous.emoji == assignment.emoji
                        && (previous.script == assignment.script
                            || previous.script.is_none()
                            || assignment.script.is_none()) =>
                {
                    previous.range.end = assignment.range.end;
                    if previous.script.is_none() {
                        previous.script = assignment.script;
                    }
                }
                _ => assignments.push(assignment),
            }
        }
        assignments
    }

    /// Every face worth trying for one cluster, in the order `resolve_text`
    /// documents, without probing coverage.
    fn candidate_list(
        &self,
        selection: &FontSelection,
        emoji: bool,
        own_script: Option<ScriptTag>,
        script: Option<ScriptTag>,
        language: Option<&LanguageTag>,
        family_faces: &mut HashMap<Arc<str>, Option<FontId>>,
    ) -> Vec<(FontId, FontChoiceReason)> {
        let chain: Vec<(FontId, FontChoiceReason)> = selection
            .chain()
            .enumerate()
            .map(|(position, font)| {
                let reason = match position {
                    0 => FontChoiceReason::Primary,
                    n => FontChoiceReason::FamilyChain {
                        index: u16::try_from(n - 1).unwrap_or(u16::MAX),
                    },
                };
                (font, reason)
            })
            .collect();

        let mut candidates: Vec<(FontId, FontChoiceReason)> = Vec::new();
        let push = |candidates: &mut Vec<(FontId, FontChoiceReason)>,
                    font: FontId,
                    reason: FontChoiceReason| {
            if !candidates.iter().any(|(known, _)| *known == font) {
                candidates.push((font, reason));
            }
        };

        let mut policy_faces = |system: &Self, names: &[Arc<str>]| -> Vec<(Arc<str>, FontId)> {
            names
                .iter()
                .filter_map(|name| {
                    let face = *family_faces
                        .entry(Arc::clone(name))
                        .or_insert_with(|| system.best_face_in_family(name, &selection.query));
                    face.map(|face| (Arc::clone(name), face))
                })
                .collect()
        };

        if emoji {
            for (font, reason) in &chain {
                if self.has_color_glyphs(*font) {
                    push(&mut candidates, *font, reason.clone());
                }
            }
            let names = self.policy.emoji_families().to_vec();
            for (family, font) in policy_faces(self, &names) {
                push(
                    &mut candidates,
                    font,
                    FontChoiceReason::EmojiPolicy { family },
                );
            }
        }
        for (font, reason) in &chain {
            push(&mut candidates, *font, reason.clone());
        }
        if let Some(script) = script {
            let names = self.policy.script_families(script, language);
            for (family, font) in policy_faces(self, &names) {
                push(
                    &mut candidates,
                    font,
                    FontChoiceReason::ScriptPolicy { script, family },
                );
            }
        }
        if own_script.is_none() {
            let names = self.policy.symbol_families().to_vec();
            for (family, font) in policy_faces(self, &names) {
                push(
                    &mut candidates,
                    font,
                    FontChoiceReason::SymbolPolicy { family },
                );
            }
            let names = self.policy.emoji_families().to_vec();
            for (family, font) in policy_faces(self, &names) {
                push(
                    &mut candidates,
                    font,
                    FontChoiceReason::EmojiPolicy { family },
                );
            }
        }
        let names = self.policy.last_resort_families().to_vec();
        for (family, font) in policy_faces(self, &names) {
            push(
                &mut candidates,
                font,
                FontChoiceReason::LastResort { family },
            );
        }

        candidates
    }

    /// Fallback candidates for the grapheme cluster that starts `cluster_text`,
    /// in `resolve_text` order. `inherited_script` stands in when the cluster
    /// has no script of its own. Shaping walks this list to retry a cluster
    /// that coverage accepted but that still shaped to `.notdef`.
    pub(crate) fn cluster_candidates(
        &self,
        selection: &FontSelection,
        cluster_text: &str,
        inherited_script: Option<ScriptTag>,
        language: Option<&LanguageTag>,
    ) -> Vec<(FontId, FontChoiceReason)> {
        let Some(cluster) = unicode::clusters(cluster_text).into_iter().next() else {
            return Vec::new();
        };
        let language = language.or(selection.query.language.as_ref());
        self.candidate_list(
            selection,
            cluster.emoji,
            cluster.script,
            cluster.script.or(inherited_script),
            language,
            &mut HashMap::new(),
        )
    }

    /// True when `font` maps every non-ignorable codepoint of `text`.
    pub(crate) fn covers_text(&mut self, font: FontId, text: &str) -> bool {
        let codepoints: Vec<char> = text
            .chars()
            .filter(|ch| !unicode::is_default_ignorable(*ch))
            .collect();
        self.covers_all(font, &codepoints)
    }

    /// Vertical metrics of a face at an instance's coordinates and a size.
    pub(crate) fn run_metrics(&self, instance: &FontInstance, size_px: f32) -> RunMetrics {
        self.record(instance.font())
            .and_then(|face| {
                let blob = face.blob()?;
                face::read_metrics(
                    (*blob).as_ref(),
                    face.meta.index,
                    instance.coords(),
                    size_px,
                )
            })
            .unwrap_or_default()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one cluster's full fallback context"
    )]
    fn choose_face(
        &mut self,
        selection: &FontSelection,
        codepoints: &[char],
        emoji: bool,
        own_script: Option<ScriptTag>,
        script: Option<ScriptTag>,
        language: Option<&LanguageTag>,
        family_faces: &mut HashMap<Arc<str>, Option<FontId>>,
    ) -> (Option<FontId>, FontChoiceReason) {
        let primary_covers = selection
            .primary
            .is_some_and(|primary| self.covers_all(primary, codepoints));
        // The common case: the primary face covers the cluster and nothing asks
        // for a different face. No candidate list is built.
        if primary_covers && (!emoji || selection.primary.is_some_and(|p| self.has_color_glyphs(p)))
        {
            return (selection.primary, FontChoiceReason::Primary);
        }
        if !primary_covers {
            self.counters.font_fallback_attempts += 1;
        }

        let candidates =
            self.candidate_list(selection, emoji, own_script, script, language, family_faces);

        for (font, reason) in candidates {
            if Some(font) != selection.primary {
                self.counters.fallback_candidates_examined += 1;
                if self.covers_all(font, codepoints) {
                    return (Some(font), reason);
                }
            } else if primary_covers {
                return (Some(font), reason);
            }
        }
        self.counters.font_fallback_misses += 1;
        (None, FontChoiceReason::Missing)
    }

    // ---- diagnostics ---------------------------------------------------

    pub fn counters(&self) -> FontCounters {
        FontCounters {
            font_faces_registered: self.face_count(),
            font_generation: self.generation.get(),
            ..self.counters
        }
    }

    pub fn reset_counters(&mut self) {
        self.counters = FontCounters::default();
    }
}
