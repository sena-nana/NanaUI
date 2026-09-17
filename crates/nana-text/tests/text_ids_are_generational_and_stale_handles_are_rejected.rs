//! The handle and staleness contract, exercised through a miniature slot table
//! of the kind a real engine will keep.

mod support;

use nana_text::{
    FontGeneration, FontId, StaleLayout, TextLayoutId, TextLayoutStore, TextRevision, TextSource,
};
use std::sync::Arc;

/// The smallest thing that can issue and retire generational handles.
struct FontSlots {
    generations: Vec<u32>,
    free: Vec<u32>,
}

impl FontSlots {
    fn new() -> Self {
        Self {
            generations: Vec::new(),
            free: Vec::new(),
        }
    }

    fn insert(&mut self) -> FontId {
        if let Some(index) = self.free.pop() {
            // Generation 0 is the null marker, so a reissued slot skips it.
            self.generations[index as usize] += 1;
            return FontId::from_parts(index, self.generations[index as usize]);
        }
        let index = self.generations.len() as u32;
        self.generations.push(1);
        FontId::from_parts(index, 1)
    }

    fn remove(&mut self, id: FontId) {
        self.free.push(id.index());
    }

    fn is_live(&self, id: FontId) -> bool {
        !id.is_null()
            && self
                .generations
                .get(id.index() as usize)
                .is_some_and(|generation| *generation == id.generation())
    }
}

#[test]
fn a_freed_slot_reissued_at_a_new_generation_rejects_the_old_handle() {
    let mut slots = FontSlots::new();
    let first = slots.insert();
    assert!(slots.is_live(first));

    slots.remove(first);
    let reused = slots.insert();
    assert_eq!(
        reused.index(),
        first.index(),
        "the allocator is expected to reuse the slot; that is the whole risk"
    );
    assert!(slots.is_live(reused));
    assert!(
        !slots.is_live(first),
        "a handle kept across the free must not alias whatever now lives there"
    );
}

#[test]
fn the_null_handle_never_matches_a_live_slot() {
    let mut slots = FontSlots::new();
    let _first = slots.insert();
    assert!(!slots.is_live(FontId::NULL));
    assert!(!slots.is_live(FontId::default()));
}

#[test]
fn editing_the_source_bumps_the_revision_and_strands_the_layout_taken_before_it() {
    let mut source = TextSource::new("hello");
    let mut layout = support::latin_single_line();
    layout.revision = source.revision();
    layout.font_generation = FontGeneration::new(3);

    assert!(
        !layout.is_stale(source.revision(), FontGeneration::new(3)),
        "nothing changed yet"
    );

    source.replace_range(0..1, "H");
    assert!(
        layout.is_stale(source.revision(), FontGeneration::new(3)),
        "an edit must strand the layout that predates it"
    );
}

#[test]
fn a_layout_taken_before_a_font_generation_bump_reports_itself_stale() {
    let source = TextSource::new("hello");
    let generation = FontGeneration::new(3);
    let mut layout = support::latin_single_line();
    layout.revision = source.revision();
    layout.font_generation = generation;

    assert!(!layout.is_stale(source.revision(), generation));
    assert!(
        layout.is_stale(source.revision(), generation.bumped()),
        "registering a face must strand every layout shaped before it"
    );
}

#[test]
fn a_revision_is_only_meaningful_next_to_the_source_that_issued_it() {
    // Two unrelated sources can sit at the same revision, which is why a cache
    // key has to pair the revision with the caller's own node identity.
    let mut left = TextSource::new("one");
    let mut right = TextSource::new("two");
    left.set_text("three");
    right.set_text("four");
    assert_eq!(left.revision(), right.revision());
    assert_ne!(left.text(), right.text());
    assert_eq!(TextRevision::default(), TextRevision::INITIAL);
}

fn layout_at(generation: FontGeneration) -> Arc<nana_text::TextLayout> {
    let mut layout = support::latin_single_line();
    layout.font_generation = generation;
    Arc::new(layout)
}

#[test]
fn a_released_layout_handle_stays_rejected_after_its_slot_is_reissued() {
    let mut store = TextLayoutStore::new();
    let generation = FontGeneration::new(1);
    let first = store.insert(layout_at(generation));
    assert!(store.get(first).is_some());

    assert!(store.remove(first).is_some());
    assert!(store.get(first).is_none(), "released means gone");
    assert!(
        store.remove(first).is_none(),
        "a second release releases nothing"
    );

    let reused = store.insert(layout_at(generation));
    assert_eq!(
        reused.index(),
        first.index(),
        "the slot is reused; that is the risk"
    );
    assert!(store.get(reused).is_some());
    assert_eq!(
        store.resolve(first, generation).unwrap_err(),
        StaleLayout::Retired,
        "a handle kept across the release must not alias the new layout"
    );
    assert_eq!(store.len(), 1);
}

#[test]
fn a_layout_handle_is_only_accepted_by_the_store_that_issued_it() {
    let generation = FontGeneration::new(1);
    let mut left = TextLayoutStore::new();
    let mut right = TextLayoutStore::new();
    let from_left = left.insert(layout_at(generation));
    let from_right = right.insert(layout_at(generation));
    assert_eq!(from_left.index(), from_right.index());
    assert!(right.get(from_left).is_none(), "another document's handle");
    assert!(left.get(from_right).is_none());
    assert!(right.get(TextLayoutId::NULL).is_none());
}

#[test]
fn a_retained_layout_from_an_older_font_generation_does_not_resolve() {
    let mut store = TextLayoutStore::new();
    let old = FontGeneration::new(4);
    let handle = store.insert(layout_at(old));
    assert!(store.resolve(handle, old).is_ok());
    assert_eq!(
        store.resolve(handle, old.bumped()).unwrap_err(),
        StaleLayout::FontGeneration {
            layout: old,
            current: old.bumped(),
        },
        "a layout whose FontIds may name replaced faces is refused, not read"
    );
}
