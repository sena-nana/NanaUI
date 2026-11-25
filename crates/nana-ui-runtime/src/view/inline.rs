//! Authoring projection: text pieces remain text, children keep ordinary ids.
use super::{
    AnyView, Class, El, FieldWrite, InlineStyle, IntoProp, IntoView, NodeRef, PropSource,
    ViewBuilder, ViewSource, widget,
};
use crate::{InlineContent, InlineFlow, NodeStyle, StableNodeId};
use std::{
    borrow::Cow,
    sync::{Arc, Mutex},
};

enum Piece {
    Text(PropSource<String>),
    Styled(InlineContent),
    Span(PropSource<String>, Box<nana_ui_core::RichSpanStyle>),
    Child(AnyView),
}
enum Source {
    Text(PropSource<String>),
    Styled(InlineContent),
    Span(PropSource<String>, Box<nana_ui_core::RichSpanStyle>),
    Child(usize),
}

/// Controls that accept inline rich/mixed children via [`El::inline_children`].
pub trait InlineContentTarget: crate::ComponentView + Sized {
    fn set_inline_content(&mut self, content: InlineContent);
    fn inline_content_differs(&self, content: &InlineContent) -> bool;
}

impl InlineContentTarget for InlineFlow {
    fn set_inline_content(&mut self, content: InlineContent) {
        self.content = content;
    }

    fn inline_content_differs(&self, content: &InlineContent) -> bool {
        self.content != *content
    }
}

macro_rules! impl_inline_target {
    ($($ty:ty),* $(,)?) => {
        $(
            impl InlineContentTarget for $ty {
                fn set_inline_content(&mut self, content: InlineContent) {
                    *self = self.clone().content(content);
                }

                fn inline_content_differs(&self, content: &InlineContent) -> bool {
                    *self != self.clone().content(content.clone())
                }
            }
        )*
    };
}

impl_inline_target!(
    crate::Text,
    crate::Button,
    crate::Chip,
    crate::ListItem,
    crate::ActionMenuItem,
);

/// Sequence of text pieces, styled spans, and atomic inline children shared
/// between `<Inline>` and inline-content controls (`<Button>`, `<Text>`,
/// `<Chip>`, `<ListItem>`, `<ActionMenuItem>`).
pub struct InlinePieces {
    pieces: Vec<Piece>,
}

pub fn inline_pieces() -> InlinePieces {
    InlinePieces { pieces: Vec::new() }
}

impl InlinePieces {
    pub fn text(mut self, text: impl IntoProp<String>) -> Self {
        self.pieces.push(Piece::Text(text.into_source()));
        self
    }

    pub fn styled(mut self, content: impl Into<InlineContent>) -> Self {
        self.pieces.push(Piece::Styled(content.into().normalized()));
        self
    }

    pub fn span(mut self, text: impl IntoProp<String>, style: nana_ui_core::RichSpanStyle) -> Self {
        self.pieces
            .push(Piece::Span(text.into_source(), Box::new(style)));
        self
    }

    pub fn child(mut self, child: impl IntoView) -> Self {
        self.pieces.push(Piece::Child(child.into_any()));
        self
    }

    pub fn line_break(self) -> Self {
        self.text("\n")
    }
}

/// A native paragraph. Text pieces are not retained Text components, and each
/// child is the same component a Row or Grid would own.
pub struct Inline {
    element: El<InlineFlow>,
    pieces: InlinePieces,
}
pub fn inline() -> Inline {
    Inline {
        element: widget(InlineFlow::new("")),
        pieces: inline_pieces(),
    }
}
impl Inline {
    pub fn text(mut self, text: impl IntoProp<String>) -> Self {
        self.pieces = self.pieces.text(text);
        self
    }
    pub fn styled(mut self, content: impl Into<InlineContent>) -> Self {
        self.pieces = self.pieces.styled(content);
        self
    }
    pub fn span(mut self, text: impl IntoProp<String>, style: nana_ui_core::RichSpanStyle) -> Self {
        self.pieces = self.pieces.span(text, style);
        self
    }
    pub fn child(mut self, child: impl IntoView) -> Self {
        self.pieces = self.pieces.child(child);
        self
    }
    pub fn line_break(self) -> Self {
        self.text("\n")
    }
    pub fn key(mut self, key: impl Into<Cow<'static, str>>) -> Self {
        self.element = self.element.key(key);
        self
    }
    pub fn node_ref(mut self, node_ref: NodeRef) -> Self {
        self.element = self.element.node_ref(node_ref);
        self
    }
    pub fn source_site(mut self, source: &'static ViewSource) -> Self {
        self.element = self.element.source_site(source);
        self
    }
    pub fn class(mut self, class: Class) -> Self {
        self.element = self.element.class(class);
        self
    }
    pub fn class_when(mut self, class: Class, condition: impl IntoProp<bool>) -> Self {
        self.element = self.element.class_when(class, condition);
        self
    }
    pub fn css(mut self, style: InlineStyle) -> Self {
        self.element = self.element.css(style);
        self
    }
    pub fn visible(mut self, visible: impl IntoProp<bool>) -> Self {
        self.element = self.element.visible(visible);
        self
    }
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.element = self
            .element
            .map_component(|component| component.style(style));
        self
    }
}
struct Content<C>(std::marker::PhantomData<fn() -> C>);
impl<C: InlineContentTarget> FieldWrite<C, InlineContent> for Content<C> {
    const FIELD: &'static str = "content";
    fn write(target: &mut C, value: InlineContent) {
        target.set_inline_content(value);
    }
    fn differs(target: &C, value: &InlineContent) -> bool {
        target.inline_content_differs(value)
    }
}

impl<C: InlineContentTarget, K> El<C, K> {
    /// Populate this inline-content node with text pieces, spans, and retained atomic children.
    pub fn inline_children(self, pieces: InlinePieces) -> Self {
        let pieces = pieces.pieces;
        let dynamic = pieces.iter().any(|piece| {
            matches!(piece, Piece::Text(source) | Piece::Span(source, _) if !matches!(source, PropSource::Const(_)))
        });
        let total_children = pieces
            .iter()
            .filter(|piece| matches!(piece, Piece::Child(_)))
            .count();
        let offsets = Arc::new(Mutex::new(Vec::<usize>::with_capacity(total_children)));
        let mut element = self;
        let mut sources = Vec::new();
        let mut child_index = 0;
        let ids = Arc::new(Mutex::new(Vec::<StableNodeId>::new()));
        let mut static_content = InlineContent::builder();
        let mut offset = 0;
        let mut child_slots = Vec::new();
        for piece in pieces {
            match piece {
                Piece::Text(source) => {
                    if let PropSource::Const(text) = &source {
                        offset += text.len();
                        static_content = static_content.text(text.as_str());
                    }
                    sources.push(Source::Text(source));
                }
                Piece::Styled(content) => {
                    offset += content.text().len();
                    static_content = static_content.span(content.clone());
                    sources.push(Source::Styled(content));
                }
                Piece::Span(source, style) => {
                    if let PropSource::Const(text) = &source {
                        offset += text.len();
                        static_content =
                            static_content.styled(text.as_str(), style.as_ref().clone());
                    }
                    sources.push(Source::Span(source, style));
                }
                Piece::Child(child) => {
                    sources.push(Source::Child(child_index));
                    child_index += 1;
                    let last = child_index == total_children;
                    offsets.lock().unwrap().push(offset);
                    child_slots.push((child, last));
                    static_content = static_content.text("\u{fffc}");
                    offset += '\u{fffc}'.len_utf8();
                }
            }
        }
        let initial = static_content.build();
        let static_rich = initial.to_rich_text();
        for (child, last) in child_slots {
            let ids = Arc::clone(&ids);
            let offsets = Arc::clone(&offsets);
            let static_rich = static_rich.clone();
            element = element.child_slot(child, move |mut component, id| {
                let mut ids = ids.lock().unwrap();
                ids.push(id);
                if !dynamic && last {
                    let atoms = ids
                        .iter()
                        .zip(offsets.lock().unwrap().iter())
                        .map(|(&node, &offset)| crate::InlineAtomic {
                            offset,
                            node,
                            text: Default::default(),
                        })
                        .collect::<Vec<_>>();
                    component
                        .set_inline_content(InlineContent::from_parts(static_rich, atoms.into()));
                }
                component
            });
        }
        element = element.map_component(|mut component| {
            component.set_inline_content(initial);
            component
        });
        if dynamic {
            element = element.prop::<InlineContent, Content<C>>(move || {
                let ids = ids.lock().unwrap();
                let mut content = InlineContent::builder();
                for source in &sources {
                    content = match source {
                        Source::Text(text) => content.text(text.get()),
                        Source::Styled(text) => content.span(text.clone()),
                        Source::Span(text, style) => {
                            content.styled(text.get(), style.as_ref().clone())
                        }
                        Source::Child(index) => content.child(ids[*index]),
                    };
                }
                content.build()
            });
        }
        element
    }
}

impl IntoView for Inline {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        self.element.inline_children(self.pieces).build(vb);
    }
}
impl super::controls::StyledComponent for InlineFlow {
    fn node_style(&self) -> &NodeStyle {
        &self.style
    }
    fn node_style_mut(&mut self) -> &mut NodeStyle {
        &mut self.style
    }
}
