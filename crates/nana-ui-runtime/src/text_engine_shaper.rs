//! A host [`TextShaper`] whose only measurement authority is a `nana-text`
//! engine (Issue #95).
//!
//! Plain text nodes resolve through [`TextShaper::text_engine`] and retain the
//! layout they were measured from. Everything else a pass measures — editor
//! text, EmptyState and modal intrinsic text, component probes — goes through
//! [`TextShaper::shape`], which lays out through the same engine, so no node's
//! metrics come from a second engine.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use nana_text::{SharedTextEngine, TextEngine as _, TextSource, TextWorkCounters};

use crate::text_node::{nana_text_constraints, nana_text_style, text_kind, text_metrics_of_layout};
use crate::{
    ComputedStyle, StableNodeId, TextContent, TextHorizontalAlignment, TextMetrics,
    TextShapeConstraints, TextShaper,
};

#[derive(Clone)]
pub struct NanaTextEngineShaper {
    engine: SharedTextEngine,
    /// Engine work done through [`TextShaper::shape`], handed to the pass
    /// through [`TextShaper::take_text_work`].
    work: TextWorkCounters,
}

impl NanaTextEngineShaper {
    pub fn new(engine: SharedTextEngine) -> Self {
        Self {
            engine,
            work: TextWorkCounters::default(),
        }
    }

    pub fn engine(&self) -> &SharedTextEngine {
        &self.engine
    }
}

impl std::fmt::Debug for NanaTextEngineShaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NanaTextEngineShaper")
            .finish_non_exhaustive()
    }
}

impl TextShaper for NanaTextEngineShaper {
    fn shape(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics {
        let source = TextSource::new(text.value.as_str());
        let layout = nana_text::lock_text_engine(&self.engine).layout(
            text_kind(&constraints),
            &source,
            &nana_text_style(style),
            &nana_text_constraints(style, &constraints, TextHorizontalAlignment::Start),
            &mut self.work,
        );
        self.work.text_source_clones += 1;
        text_metrics_of_layout(&layout)
    }

    /// The whole engine epoch, folded: a language change or another engine is
    /// as much a new measurement as a font registration.
    fn font_generation(&self) -> u64 {
        let epoch = nana_text::lock_text_engine(&self.engine).epoch();
        let mut hasher = DefaultHasher::new();
        epoch.hash(&mut hasher);
        hasher.finish()
    }

    fn take_text_work(&mut self) -> nana_text::TextWorkCounters {
        let mut work = std::mem::take(&mut self.work);
        // The pass counts the nodes it measures; these are the engine's own
        // cache and layout numbers behind them.
        work.text_nodes_considered = 0;
        work.text_nodes_shaped = 0;
        work
    }

    fn text_engine(&self) -> Option<SharedTextEngine> {
        Some(Arc::clone(&self.engine))
    }
}
