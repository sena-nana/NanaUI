//! `RichTextView`: a text node showing an application-owned [`RichText`].
//!
//! The value is the application's: this component holds a clone, and
//! projecting it is one `SetRichText` when the value changed and nothing
//! when it did not. What a change costs is decided by the tier it touches
//! (see [`MutationQueue::set_rich_text`]): restyling a word's colour,
//! outline, shadow or decoration repaints the node without shaping or laying
//! it out; changing a span's size or font reshapes it and grows its line.
//!
//! It is a plain text node underneath, so measurement, wrapping, ellipsis,
//! selection and accessibility are the ones [`crate::Text`] has. Its
//! accessible name is the text without styling.

use std::sync::Arc;

use nana_ui_core::RichText;

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, MutationQueue,
    NodeKind, NodeStyle, StableNodeId, UiWorld,
};

/// Rich text on a plain text node: one string, styled by range.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RichTextView {
    pub value: RichText,
    /// Painted, but omitted from accessibility projection.
    pub decorative: bool,
    pub style: NodeStyle,
}

impl RichTextView {
    pub fn new(value: impl Into<RichText>) -> Self {
        Self {
            value: value.into(),
            decorative: false,
            style: NodeStyle::default(),
        }
    }

    /// Replace the value. A clone of a value the node already shows is
    /// recognised and costs nothing.
    pub fn value(mut self, value: impl Into<RichText>) -> Self {
        self.value = value.into();
        self
    }

    /// Keep the glyphs visible while omitting this text from the
    /// accessibility tree (`aria-hidden="true"`).
    pub const fn decorative(mut self) -> Self {
        self.decorative = true;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// The node's own font size in logical px: what every span that does
    /// not set one is drawn at. Unlike [`crate::Text::font_size`] it does not
    /// pin the line box to an absolute height, so a larger span grows its
    /// line.
    pub fn font_size(mut self, size: f32) -> Self {
        Arc::make_mut(&mut self.style.layout).font_size = Some(size);
        self
    }

    /// The node's own font family list (CSS `font-family` syntax).
    pub fn font_family(mut self, family: impl Into<String>) -> Self {
        Arc::make_mut(&mut self.style.layout).font_family = Some(family.into());
        self
    }

    /// Semantic foreground role of the text a span does not colour.
    pub fn color(mut self, role: nana_ui_core::SemanticColorRole) -> Self {
        self.style.foreground = Some(role);
        self
    }

    /// Line height as a multiple of each run's own size.
    pub fn line_height(mut self, ratio: f32) -> Self {
        Arc::make_mut(&mut self.style.layout).line_height =
            Some(nana_ui_core::LineHeightSpec::Relative(ratio));
        self
    }

    pub fn width(mut self, width: nana_ui_core::LengthSpec) -> Self {
        Arc::make_mut(&mut self.style.layout).width = Some(width);
        self
    }

    pub fn max_width(mut self, max_width: nana_ui_core::LengthSpec) -> Self {
        Arc::make_mut(&mut self.style.layout).max_width = Some(max_width);
        self
    }

    /// Keeps the text on one line.
    pub fn nowrap(mut self, nowrap: bool) -> Self {
        Arc::make_mut(&mut self.style.layout).white_space_nowrap = nowrap;
        self
    }
}

impl crate::AppContext {
    /// Present a rich text node's glyphs: `effects` is the table its spans'
    /// `effect` indices name, `reveal` the typewriter schedule. Presentation
    /// only: it shapes, lays out and rasterizes nothing and rebuilds no glyph
    /// instance; frames are requested only while something still moves.
    pub fn set_rich_presentation(
        &mut self,
        node: impl Into<StableNodeId>,
        effects: impl Into<Arc<[nana_ui_core::GlyphEffect]>>,
        reveal: Option<nana_ui_core::RevealSchedule>,
    ) -> Result<(), crate::FrameworkError> {
        let mut queue = MutationQueue::new();
        queue.set_glyph_presentation(
            node.into(),
            Some(nana_ui_core::GlyphPresentation::new(effects, reveal)),
        );
        self.commit_mutations(queue).map(|_| ())
    }

    /// Stop presenting a node's glyphs.
    pub fn clear_rich_presentation(
        &mut self,
        node: impl Into<StableNodeId>,
    ) -> Result<(), crate::FrameworkError> {
        let mut queue = MutationQueue::new();
        queue.set_glyph_presentation(node.into(), None);
        self.commit_mutations(queue).map(|_| ())
    }
}

impl ComponentView for RichTextView {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Text
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.rich_text(id) != Some(&self.value) {
            mutations.set_rich_text(id, self.value.clone());
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Text,
                hidden: self.decorative,
                ..AccessibilityState::default()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId};
    use nana_ui_core::{PaintColor, RichSpanStyle};

    #[test]
    fn projecting_the_same_value_again_commits_nothing() {
        let mut cx = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let rich = RichText::new("hello world").with_span(
            6..11,
            RichSpanStyle::new().color(PaintColor::srgb([1.0, 0.0, 0.0, 1.0])),
        );
        let view = cx
            .create_component(document, RichTextView::new(rich.clone()))
            .unwrap();
        let id = view.stable_id();
        assert_eq!(cx.world().text(id), Some("hello world"));
        assert_eq!(cx.world().rich_text(id), Some(&rich));
        let generation = cx.world().generation();
        cx.set_component(view, RichTextView::new(rich.clone()))
            .unwrap();
        assert_eq!(
            cx.world().generation(),
            generation,
            "an equal value is not a mutation"
        );
        let recolored = rich.with_span(
            6..11,
            RichSpanStyle::new().color(PaintColor::srgb([0.0, 0.0, 1.0, 1.0])),
        );
        cx.set_component(view, RichTextView::new(recolored.clone()))
            .unwrap();
        assert_eq!(cx.world().rich_text(id), Some(&recolored));
    }

    #[test]
    fn the_registered_tag_builds_the_component() {
        let cx = AppContext::new();
        assert_eq!(
            cx.resolve_component_tag("rich-text")
                .map(crate::ComponentTypeId::as_str),
            Some("nana.rich-text")
        );
    }
}
