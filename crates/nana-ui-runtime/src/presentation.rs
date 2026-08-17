//! Registered text presentation.
//!
//! Intent ([`HighlightRequest`]) and derived spans ([`TextPresentation`]) are
//! retained ECS components. Algorithms live on [`UiWorld`] as named
//! [`TextPresenter`] values so Vue flush and `AppContext` share one registry.
//! Presenters color committed text only; IME preedit stays solid.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bevy_ecs::component::Component;
use nana_ui_core::SemanticColorRole;

/// Built-in presenter name for syntax highlighting.
pub const HIGHLIGHT_PRESENTER: &str = "highlight";

/// One committed-text range painted with a theme role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
    pub color: SemanticColorRole,
}

/// Application intent: which presenter should color this node's committed text.
#[derive(Component, Debug, Clone, PartialEq, Eq, Hash)]
pub struct HighlightRequest {
    pub presenter: Arc<str>,
    pub language: Arc<str>,
}

impl HighlightRequest {
    pub fn new(presenter: impl Into<Arc<str>>, language: impl Into<Arc<str>>) -> Self {
        Self {
            presenter: presenter.into(),
            language: language.into(),
        }
    }

    /// Request the built-in `"highlight"` presenter.
    pub fn highlight(language: impl Into<Arc<str>>) -> Self {
        Self::new(HIGHLIGHT_PRESENTER, language)
    }
}

/// Derived committed-text spans. Recomputed only when the source hash changes.
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub struct TextPresentation {
    pub spans: Vec<TextSpan>,
    pub source: u64,
}

/// World-level algorithm that turns committed text into semantic spans.
pub trait TextPresenter: Send + 'static {
    fn name(&self) -> &'static str;
    fn present(&self, text: &str, request: &HighlightRequest) -> Vec<TextSpan>;
}

pub(crate) fn presentation_source(text: &str, request: &HighlightRequest) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    request.presenter.hash(&mut hasher);
    request.language.hash(&mut hasher);
    text.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn sanitize_spans(text: &str, spans: Vec<TextSpan>) -> Vec<TextSpan> {
    let mut cleaned = spans
        .into_iter()
        .filter(|span| {
            span.start < span.end
                && span.end <= text.len()
                && text.is_char_boundary(span.start)
                && text.is_char_boundary(span.end)
        })
        .collect::<Vec<_>>();
    cleaned.sort_by_key(|span| (span.start, span.end));
    let mut merged: Vec<TextSpan> = Vec::new();
    let mut cursor = 0usize;
    for mut span in cleaned {
        if span.start < cursor {
            span.start = cursor;
        }
        if span.start >= span.end {
            continue;
        }
        if let Some(last) = merged.last_mut()
            && last.end == span.start
            && last.color == span.color
        {
            last.end = span.end;
            cursor = span.end;
            continue;
        }
        cursor = span.end;
        merged.push(span);
    }
    if merged.len() == 1
        && merged[0].color == SemanticColorRole::Text
        && merged[0].start == 0
        && merged[0].end == text.len()
    {
        return Vec::new();
    }
    merged
}

/// First registered presenter: syntect scope names mapped onto theme roles.
#[cfg(feature = "syntax-highlighting")]
#[derive(Debug, Default)]
pub struct SyntectHighlighter;

#[cfg(feature = "syntax-highlighting")]
impl TextPresenter for SyntectHighlighter {
    fn name(&self) -> &'static str {
        HIGHLIGHT_PRESENTER
    }

    fn present(&self, text: &str, request: &HighlightRequest) -> Vec<TextSpan> {
        syntect_present(text, request.language.as_ref())
    }
}

/// Installs [`SyntectHighlighter`] as the `"highlight"` presenter.
#[cfg(feature = "syntax-highlighting")]
pub struct HighlightPresentation;

#[cfg(feature = "syntax-highlighting")]
impl crate::framework::UiExtension for HighlightPresentation {
    fn name(&self) -> &'static str {
        "nana.highlight"
    }

    fn install(
        &self,
        registrar: &mut crate::framework::ExtensionRegistrar,
    ) -> Result<(), crate::framework::FrameworkError> {
        registrar.register_presenter(Box::new(SyntectHighlighter))
    }
}

#[cfg(feature = "syntax-highlighting")]
fn syntect_present(text: &str, language: &str) -> Vec<TextSpan> {
    use std::sync::LazyLock;

    use two_face::re_exports::syntect::parsing::{ParseState, ScopeStack, SyntaxSet};

    static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

    let syntax = SYNTAXES
        .find_syntax_by_token(language)
        .or_else(|| SYNTAXES.find_syntax_by_extension(language))
        .or_else(|| SYNTAXES.find_syntax_by_name(language))
        .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
    if syntax.name == "Plain Text" {
        return Vec::new();
    }

    let mut parse = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut spans = Vec::new();
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let ops = parse.parse_line(line, &SYNTAXES).unwrap_or_default();
        let mut last = 0usize;
        for (index, op) in ops {
            if index > last {
                push_scope_span(&mut spans, offset, last, index, &stack);
            }
            let _ = stack.apply(&op);
            last = index;
        }
        if last < line.len() {
            push_scope_span(&mut spans, offset, last, line.len(), &stack);
        }
        offset += line.len();
    }
    sanitize_spans(text, spans)
}

#[cfg(feature = "syntax-highlighting")]
fn push_scope_span(
    spans: &mut Vec<TextSpan>,
    base: usize,
    start: usize,
    end: usize,
    stack: &two_face::re_exports::syntect::parsing::ScopeStack,
) {
    if start >= end {
        return;
    }
    let color = role_for_stack(stack);
    if color == SemanticColorRole::Text {
        return;
    }
    spans.push(TextSpan {
        start: base + start,
        end: base + end,
        color,
    });
}

#[cfg(feature = "syntax-highlighting")]
fn role_for_stack(stack: &two_face::re_exports::syntect::parsing::ScopeStack) -> SemanticColorRole {
    for scope in stack.as_slice().iter().rev() {
        let name = scope.build_string();
        if name.contains("invalid") {
            return SemanticColorRole::Danger;
        }
        if name.contains("comment") {
            return SemanticColorRole::Muted;
        }
        if name.contains("string") {
            return SemanticColorRole::Success;
        }
        if name.contains("constant.numeric")
            || name.contains("constant.character")
            || name.contains("constant.language")
        {
            return SemanticColorRole::Warning;
        }
        if name.contains("storage") || name.contains("keyword.declaration") {
            return SemanticColorRole::AccentStrong;
        }
        if name.contains("keyword") {
            return SemanticColorRole::Accent;
        }
        if name.contains("entity.name.function") {
            return SemanticColorRole::Accent;
        }
        if name.contains("entity.name.type")
            || name.contains("support.type")
            || name.contains("entity.name.class")
        {
            return SemanticColorRole::AccentOnSoft;
        }
        if name.contains("punctuation") {
            return SemanticColorRole::Faint;
        }
    }
    SemanticColorRole::Text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ComputedStyle, DocumentId, MutationQueue, NodeKind, StableNodeId, TextContent,
        TextInputState, TextMetrics, TextShapeConstraints, TextShaper, UiWorld,
    };

    struct KeywordPresenter;

    impl TextPresenter for KeywordPresenter {
        fn name(&self) -> &'static str {
            HIGHLIGHT_PRESENTER
        }

        fn present(&self, text: &str, _request: &HighlightRequest) -> Vec<TextSpan> {
            text.match_indices("fn")
                .map(|(start, token)| TextSpan {
                    start,
                    end: start + token.len(),
                    color: SemanticColorRole::Accent,
                })
                .collect()
        }
    }

    struct ZeroShaper;

    impl TextShaper for ZeroShaper {
        fn shape(
            &mut self,
            _id: StableNodeId,
            _text: &TextContent,
            style: &ComputedStyle,
            _constraints: TextShapeConstraints,
        ) -> TextMetrics {
            TextMetrics {
                width: style.font_size,
                height: style.font_size,
            }
        }
    }

    fn id(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn sanitize_drops_invalid_and_merges_adjacent_roles() {
        let spans = sanitize_spans(
            "fn main",
            vec![
                TextSpan {
                    start: 0,
                    end: 2,
                    color: SemanticColorRole::Accent,
                },
                TextSpan {
                    start: 2,
                    end: 3,
                    color: SemanticColorRole::Accent,
                },
                TextSpan {
                    start: 4,
                    end: 99,
                    color: SemanticColorRole::Danger,
                },
                TextSpan {
                    start: 3,
                    end: 2,
                    color: SemanticColorRole::Muted,
                },
            ],
        );
        assert_eq!(
            spans,
            vec![TextSpan {
                start: 0,
                end: 3,
                color: SemanticColorRole::Accent,
            }]
        );
    }

    #[test]
    fn registered_presenter_colors_committed_text() {
        let mut world = UiWorld::new();
        world
            .register_presenter(Box::new(KeywordPresenter))
            .unwrap();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document(), NodeKind::Text);
        queue.set_text(
            id(1),
            TextContent {
                value: "fn main".into(),
            },
        );
        queue.set_highlight_request(id(1), Some(HighlightRequest::highlight("rs")));
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        assert!(work.text.contains(&id(1)));
        world.resolve_presentations(&work.text).unwrap();
        let presentation = world.text_presentation(id(1)).unwrap();
        assert_eq!(
            presentation.spans,
            vec![TextSpan {
                start: 0,
                end: 2,
                color: SemanticColorRole::Accent,
            }]
        );
        let extracted = world.extract_nodes(&[id(1)]);
        assert_eq!(extracted[0].text_spans.len(), 1);
        assert_eq!(extracted[0].text_spans[0].start, 0);
        assert_eq!(extracted[0].text_spans[0].end, 2);
    }

    #[test]
    fn missing_presenter_leaves_solid_text() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document(), NodeKind::Text);
        queue.set_text(
            id(1),
            TextContent {
                value: "fn main".into(),
            },
        );
        queue.set_highlight_request(id(1), Some(HighlightRequest::highlight("rs")));
        world.commit(queue).unwrap();
        world.resolve_presentations(&[id(1)]).unwrap();
        assert!(world.text_presentation(id(1)).unwrap().spans.is_empty());
        assert!(world.extract_nodes(&[id(1)])[0].text_spans.is_empty());
    }

    #[test]
    fn text_edit_rebuilds_spans_and_shape_text_runs_presenters() {
        let mut world = UiWorld::new();
        world
            .register_presenter(Box::new(KeywordPresenter))
            .unwrap();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document(), NodeKind::Text);
        queue.set_text(
            id(1),
            TextContent {
                value: "let x".into(),
            },
        );
        queue.set_highlight_request(id(1), Some(HighlightRequest::highlight("rs")));
        world.commit(queue).unwrap();
        world.shape_text(&[id(1)], &mut ZeroShaper).unwrap();
        assert!(world.text_presentation(id(1)).unwrap().spans.is_empty());

        let mut queue = MutationQueue::new();
        queue.set_text(
            id(1),
            TextContent {
                value: "fn x".into(),
            },
        );
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.shape_text(&work.text, &mut ZeroShaper).unwrap();
        assert_eq!(world.text_presentation(id(1)).unwrap().spans.len(), 1);
    }

    #[test]
    fn theme_change_recolors_extracted_spans_without_rerunning() {
        let mut world = UiWorld::new();
        world
            .register_presenter(Box::new(KeywordPresenter))
            .unwrap();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document(), NodeKind::Text);
        queue.set_text(id(1), TextContent { value: "fn".into() });
        queue.set_highlight_request(id(1), Some(HighlightRequest::highlight("rs")));
        world.commit(queue).unwrap();
        world.resolve_presentations(&[id(1)]).unwrap();
        let source = world.text_presentation(id(1)).unwrap().source;
        let dark = world.extract_nodes(&[id(1)])[0].text_spans[0].color;

        let mut queue = MutationQueue::new();
        queue.set_theme(nana_ui_core::ThemeMode::Light);
        world.commit(queue).unwrap();
        let light = world.extract_nodes(&[id(1)])[0].text_spans[0].color;
        assert_ne!(dark, light);
        assert_eq!(world.text_presentation(id(1)).unwrap().source, source);
    }

    #[test]
    fn ime_preedit_suppresses_extracted_spans() {
        let mut world = UiWorld::new();
        world
            .register_presenter(Box::new(KeywordPresenter))
            .unwrap();
        let mut queue = MutationQueue::new();
        queue.create(
            id(1),
            document(),
            NodeKind::Element {
                tag: "input".into(),
            },
        );
        queue.set_interaction(
            id(1),
            crate::InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        queue.set_text_input(id(1), Some(TextInputState::new("fn main")));
        queue.set_highlight_request(id(1), Some(HighlightRequest::highlight("rs")));
        queue.request_focus(document(), Some(id(1)));
        queue.set_ime(
            id(1),
            Some(crate::ImeComposition {
                text: "x".into(),
                selection: None,
            }),
        );
        world.commit(queue).unwrap();
        world.resolve_presentations(&[id(1)]).unwrap();
        assert_eq!(world.text_presentation(id(1)).unwrap().spans.len(), 1);
        assert!(world.extract_nodes(&[id(1)])[0].text_spans.is_empty());
    }

    #[test]
    fn late_presenter_registration_dirties_matching_nodes() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(id(1), document(), NodeKind::Text);
        queue.set_text(id(1), TextContent { value: "fn".into() });
        queue.set_highlight_request(id(1), Some(HighlightRequest::highlight("rs")));
        world.commit(queue).unwrap();
        let _ = world.take_system_work();
        world
            .register_presenter(Box::new(KeywordPresenter))
            .unwrap();
        let work = world.take_system_work();
        assert!(work.text.contains(&id(1)));
        world.resolve_presentations(&work.text).unwrap();
        assert_eq!(world.text_presentation(id(1)).unwrap().spans.len(), 1);
    }

    #[cfg(feature = "syntax-highlighting")]
    #[test]
    fn syntect_maps_rust_keywords_to_accent() {
        let spans = SyntectHighlighter.present("fn main() {}", &HighlightRequest::highlight("rs"));
        assert!(
            spans.iter().any(|span| {
                matches!(
                    span.color,
                    SemanticColorRole::Accent | SemanticColorRole::AccentStrong
                ) && &"fn main() {}"[span.start..span.end] == "fn"
            }),
            "expected rust `fn` to map to Accent or AccentStrong, got {spans:?}"
        );
    }
}
