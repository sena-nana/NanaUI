//! Issue #211: the CJK line decision reaches Runtime text through inherited
//! style.
//!
//! A column of two scopes, each a 101 px box around one paragraph of Chinese
//! with punctuation. Chinese and Japanese close their punctuation up at
//! fixed amounts unless `text-spacing-trim` says otherwise; every other
//! language, and every other property, starts off. A typography change lays
//! the text out again and never shapes it again.

#![cfg(test)]

use nana_text::font::LanguageTag;
use nana_ui_core::{
    LayoutStyle, LengthSpec, TextJustifySpec, TextSpacingTrimSpec, TextTypography, WorkCounters,
};

use super::reflow_oracle::{Builder, bundled_face_shaper, column, product_frame, styled};
use super::{DocumentId, StableNodeId};
use crate::{AppContext, LayoutViewport, MutationQueue, NanaTextEngineShaper};

const TEXT: &str = "排版，「测试」。中文排版测试中文排版测试中文";

struct Page {
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
    scope: StableNodeId,
    text: StableNodeId,
}

fn scope_style(typography: TextTypography) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(101.0)),
        text_typography: typography,
        ..LayoutStyle::default()
    }
}

impl Page {
    fn new(language: &str, typography: TextTypography) -> Self {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 400.0);
        let outer = b.element(page, column(None));
        let scope = b.element(outer, scope_style(typography));
        let text = b.label(scope, TEXT);
        b.queue.set_language(outer, LanguageTag::new(language));
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut page = Self {
            context,
            document,
            shaper: bundled_face_shaper(),
            scope,
            text,
        };
        page.frame();
        page
    }

    fn frame(&mut self) -> WorkCounters {
        product_frame(
            &mut self.context,
            self.document,
            LayoutViewport::new(800.0, 600.0),
            &mut self.shaper,
        )
    }

    fn restyle(&mut self, typography: TextTypography) -> WorkCounters {
        let mut queue = MutationQueue::new();
        queue.set_style(self.scope, styled(scope_style(typography)));
        self.context.commit_mutations(queue).unwrap();
        self.frame()
    }

    /// Each line's width and text.
    fn lines(&self) -> Vec<(f32, String)> {
        let (_, layout) = self.context.world().text_layout(self.text).unwrap();
        layout
            .lines
            .iter()
            .map(|line| (line.metrics.width_px, TEXT[line.source.clone()].to_owned()))
            .collect()
    }

    fn text_width(&self) -> f32 {
        self.lines().iter().map(|(width, _)| *width).sum()
    }
}

fn trim(spec: TextSpacingTrimSpec) -> TextTypography {
    TextTypography {
        spacing_trim: Some(spec),
        ..TextTypography::INHERIT
    }
}

/// Chinese closes the comma against the opening bracket and the closing
/// bracket against the full stop; English does not, nor does Chinese that
/// asks for `space-all`. Japanese is Chinese's default.
#[test]
fn issue211_chinese_and_japanese_close_punctuation_up_by_default() {
    let chinese = Page::new("zh-Hans", TextTypography::INHERIT);
    let japanese = Page::new("ja", TextTypography::INHERIT);
    let english = Page::new("en", TextTypography::INHERIT);
    let spaced = Page::new("zh-Hans", trim(TextSpacingTrimSpec::SpaceAll));
    assert!(
        chinese.text_width() < english.text_width() - 1.0,
        "{:?} {:?}",
        chinese.lines(),
        english.lines()
    );
    assert_eq!(japanese.text_width(), chinese.text_width());
    assert_eq!(spaced.lines(), english.lines());
}

/// A scope's typography reaches the text under it: justified, every line
/// but the last fills the box. Turning it on and off lays the text out again
/// and shapes nothing.
#[test]
fn issue211_inherited_typography_relays_text_out_without_reshaping() {
    let mut page = Page::new("zh-Hans", TextTypography::INHERIT);
    let ragged = page.lines();
    assert!(ragged.len() > 1);
    let justified = TextTypography {
        justify_lines: Some(true),
        justify: Some(TextJustifySpec::InterCharacter),
        ..TextTypography::INHERIT
    };
    let counters = page.restyle(justified);
    assert_eq!(counters.text_shaped_runs, 0, "{counters:?}");
    assert!(counters.text_constraint_relayouts >= 1, "{counters:?}");
    let lines = page.lines();
    let (last, filled) = lines.split_last().unwrap();
    for (width, line) in filled {
        assert!((width - 101.0).abs() < 0.05, "{line}: {width}");
    }
    assert!(last.0 < 101.0);

    let counters = page.restyle(TextTypography::INHERIT);
    assert_eq!(counters.text_shaped_runs, 0, "{counters:?}");
    assert_eq!(page.lines(), ragged);
}

/// `text-spacing-trim: auto` turns cost fitting on: a line a little short
/// closes its punctuation up rather than break, so it never takes more lines
/// than the fixed default.
#[test]
fn issue211_auto_trim_never_takes_more_lines_than_the_default() {
    let mut page = Page::new("zh-Hans", TextTypography::INHERIT);
    let fixed = page.lines();
    page.restyle(trim(TextSpacingTrimSpec::Auto));
    let fitted = page.lines();
    assert!(fitted.len() <= fixed.len());
    for (width, _) in &fitted {
        assert!(*width <= 101.05);
    }
}
