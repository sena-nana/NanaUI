//! Retained layouts behind generational [`TextLayoutId`]s.
//!
//! The layout cache answers "is there a layout for this key"; it evicts on its
//! own schedule and hands out shared results. An owner that *retains* a layout
//! — one per text node of a document — needs something else: a handle it can
//! keep across frames, that is released with the node, and that is rejected
//! rather than silently re-pointed once it no longer names what it named.
//!
//! Every slot generation is drawn from one process-wide sequence, so a handle
//! is only ever accepted by the store that issued it: another store's slot at
//! the same index carries a different generation. Passing a handle to the
//! wrong document fails exactly like passing a released one. The sequence is
//! `u32` and wraps after about four billion issues; past that a foreign handle
//! is accepted only if it also lands on the same index at the same
//! generation.

use super::ir::TextLayout;
use crate::id::{FontGeneration, TextLayoutId};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// Generations issued so far across every store. `0` is `TextLayoutId::NULL`'s.
static NEXT_GENERATION: AtomicU32 = AtomicU32::new(1);

fn next_generation() -> u32 {
    loop {
        let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        if generation != 0 {
            return generation;
        }
    }
}

/// Why a handle no longer resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleLayout {
    /// Null, released, replaced, or issued by another store.
    Retired,
    /// Still retained, but laid out against a font database generation that is
    /// no longer current: its `FontId`s may name faces that were replaced.
    FontGeneration {
        layout: FontGeneration,
        current: FontGeneration,
    },
}

struct Slot {
    generation: u32,
    layout: Option<Arc<TextLayout>>,
}

/// Layouts one owner retains, addressed by generational handle.
#[derive(Default)]
pub struct TextLayoutStore {
    slots: Vec<Slot>,
    free: Vec<u32>,
    live: usize,
}

impl std::fmt::Debug for TextLayoutStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextLayoutStore")
            .field("live", &self.live)
            .field("slots", &self.slots.len())
            .finish()
    }
}

impl TextLayoutStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Retains `layout` and returns a fresh handle to it.
    pub fn insert(&mut self, layout: Arc<TextLayout>) -> TextLayoutId {
        let generation = next_generation();
        self.live += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.generation = generation;
            slot.layout = Some(layout);
            return TextLayoutId::from_parts(index, generation);
        }
        let index = u32::try_from(self.slots.len()).expect("more than u32::MAX retained layouts");
        self.slots.push(Slot {
            generation,
            layout: Some(layout),
        });
        TextLayoutId::from_parts(index, generation)
    }

    /// The layout `id` names, if it is still retained by this store.
    pub fn get(&self, id: TextLayoutId) -> Option<&Arc<TextLayout>> {
        if id.is_null() {
            return None;
        }
        let slot = self.slots.get(id.index() as usize)?;
        (slot.generation == id.generation())
            .then_some(slot.layout.as_ref())
            .flatten()
    }

    /// [`Self::get`], additionally refusing a layout produced under another
    /// font generation.
    pub fn resolve(
        &self,
        id: TextLayoutId,
        font_generation: FontGeneration,
    ) -> Result<&Arc<TextLayout>, StaleLayout> {
        let layout = self.get(id).ok_or(StaleLayout::Retired)?;
        if layout.font_generation != font_generation {
            return Err(StaleLayout::FontGeneration {
                layout: layout.font_generation,
                current: font_generation,
            });
        }
        Ok(layout)
    }

    /// Releases `id`. Returns the layout if the handle was live; a stale or
    /// foreign handle releases nothing.
    pub fn remove(&mut self, id: TextLayoutId) -> Option<Arc<TextLayout>> {
        self.get(id)?;
        let slot = &mut self.slots[id.index() as usize];
        let layout = slot.layout.take();
        // Retire the generation with the slot, so the released handle can
        // never match again even before the slot is reissued.
        slot.generation = 0;
        self.free.push(id.index());
        self.live -= 1;
        layout
    }

    /// Retained layouts.
    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }
}
