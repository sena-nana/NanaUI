use bevy_ecs::component::Component;

use crate::StableNodeId;

#[derive(Component, Debug, Clone, Copy, Default)]
pub(crate) struct DirtyMask(u8);

impl DirtyMask {
    pub(crate) const STYLE: u8 = 1 << 0;
    pub(crate) const TEXT: u8 = 1 << 1;
    pub(crate) const LAYOUT: u8 = 1 << 2;
    pub(crate) const INPUT: u8 = 1 << 3;
    pub(crate) const FOCUS_IME: u8 = 1 << 4;
    pub(crate) const RENDER: u8 = 1 << 5;
    pub(crate) const ACCESSIBILITY: u8 = 1 << 6;
    pub(crate) const ALL: u8 = Self::STYLE
        | Self::TEXT
        | Self::LAYOUT
        | Self::INPUT
        | Self::FOCUS_IME
        | Self::RENDER
        | Self::ACCESSIBILITY;

    pub(crate) const fn all() -> Self {
        Self(Self::ALL)
    }

    pub(crate) fn insert(&mut self, bits: u8) -> bool {
        let before = self.0;
        self.0 |= bits;
        self.0 != before
    }

    pub(crate) fn take(&mut self) -> u8 {
        std::mem::take(&mut self.0)
    }
}

/// Deterministic per-system work produced from entity dirty components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemWork {
    pub generation: u64,
    pub style: Vec<StableNodeId>,
    pub text: Vec<StableNodeId>,
    pub layout: Vec<StableNodeId>,
    pub input_hit_test: Vec<StableNodeId>,
    pub focus_ime: Vec<StableNodeId>,
    pub accessibility: Vec<StableNodeId>,
    pub render_extraction: Vec<StableNodeId>,
    pub render_removals: Vec<StableNodeId>,
}

impl SystemWork {
    pub fn is_empty(&self) -> bool {
        self.style.is_empty()
            && self.text.is_empty()
            && self.layout.is_empty()
            && self.input_hit_test.is_empty()
            && self.focus_ime.is_empty()
            && self.accessibility.is_empty()
            && self.render_extraction.is_empty()
            && self.render_removals.is_empty()
    }
}

pub(crate) fn push_work(work: &mut SystemWork, id: StableNodeId, bits: u8) {
    if bits & DirtyMask::STYLE != 0 {
        work.style.push(id);
    }
    if bits & DirtyMask::TEXT != 0 {
        work.text.push(id);
    }
    if bits & DirtyMask::LAYOUT != 0 {
        work.layout.push(id);
    }
    if bits & DirtyMask::INPUT != 0 {
        work.input_hit_test.push(id);
    }
    if bits & DirtyMask::FOCUS_IME != 0 {
        work.focus_ime.push(id);
    }
    if bits & DirtyMask::ACCESSIBILITY != 0 {
        work.accessibility.push(id);
    }
    if bits & DirtyMask::RENDER != 0 {
        work.render_extraction.push(id);
    }
}
