//! Counting shaper adapter and component intrinsic text shaping.

use crate::world::*;

pub(in crate::world) trait TextProbeTarget {
    fn forward_text_probes<R>(
        &mut self,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
        consume: impl FnOnce(&mut dyn TextShaper) -> R,
    ) -> R;
}

impl<S: TextShaper> TextProbeTarget for S {
    fn forward_text_probes<R>(
        &mut self,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
        consume: impl FnOnce(&mut dyn TextShaper) -> R,
    ) -> R {
        self.with_text_probes(text, style, constraints, consume)
    }
}

impl TextProbeTarget for dyn TextShaper + '_ {
    fn forward_text_probes<R>(
        &mut self,
        _text: &TextContent,
        _style: &ComputedStyle,
        _constraints: crate::TextShapeConstraints,
        consume: impl FnOnce(&mut dyn TextShaper) -> R,
    ) -> R {
        consume(self)
    }
}

/// Counts `TextShaper::shape` invocations and wrap-layout requests across a pass,
/// preserving host probe batching for both generic and trait-object shapers.
pub(in crate::world) struct CountingShaper<'a, S: TextShaper + ?Sized> {
    pub(in crate::world) inner: &'a mut S,
    pub(in crate::world) runs: usize,
    pub(in crate::world) wrap_layouts: usize,
}

impl<'a, S: TextShaper + ?Sized> CountingShaper<'a, S> {
    pub(in crate::world) fn new(inner: &'a mut S) -> Self {
        Self {
            inner,
            runs: 0,
            wrap_layouts: 0,
        }
    }
}

impl<S: TextShaper + TextProbeTarget + ?Sized> TextShaper for CountingShaper<'_, S> {
    fn strut_metrics(&mut self, style: &ComputedStyle) -> Option<crate::FontStrut> {
        self.inner.strut_metrics(style)
    }

    fn strut_x_height(&mut self, style: &ComputedStyle) -> Option<f32> {
        self.inner.strut_x_height(style)
    }

    fn font_generation(&self) -> u64 {
        self.inner.font_generation()
    }

    fn retains_measurement(&self, id: StableNodeId) -> bool {
        self.inner.retains_measurement(id)
    }

    fn text_engine(&self) -> Option<nana_text::SharedTextEngine> {
        self.inner.text_engine()
    }

    fn take_text_work(&mut self) -> nana_text::TextWorkCounters {
        self.inner.take_text_work()
    }

    fn with_text_probes<R>(
        &mut self,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
        consume: impl FnOnce(&mut dyn TextShaper) -> R,
    ) -> R {
        let mut runs = 0usize;
        let mut wrap_layouts = 0usize;
        let result = self
            .inner
            .forward_text_probes(text, style, constraints, |prepared| {
                let mut adapter = CountingShaper::new(prepared);
                let outcome = consume(&mut adapter);
                runs = adapter.runs;
                wrap_layouts = adapter.wrap_layouts;
                outcome
            });
        self.runs = self.runs.saturating_add(runs);
        self.wrap_layouts = self.wrap_layouts.saturating_add(wrap_layouts);
        result
    }

    fn horizontal_offset(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        offset: usize,
        style: &ComputedStyle,
    ) -> f32 {
        self.inner.horizontal_offset(id, text, offset, style)
    }

    fn text_position(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        offset: usize,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
    ) -> (f32, f32, f32) {
        self.inner
            .text_position(id, text, offset, style, constraints)
    }

    fn text_caret_position(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        offset: usize,
        affinity: crate::TextAffinity,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
    ) -> (f32, f32, f32) {
        self.inner
            .text_caret_position(id, text, offset, affinity, style, constraints)
    }

    fn text_hit_at_point(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        x: f32,
        y: f32,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
    ) -> Option<crate::TextHit> {
        self.inner
            .text_hit_at_point(id, text, x, y, style, constraints)
    }

    fn text_caret_visual_step(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        offset: usize,
        affinity: crate::TextAffinity,
        rightwards: bool,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
    ) -> Option<crate::TextHit> {
        self.inner.text_caret_visual_step(
            id,
            text,
            offset,
            affinity,
            rightwards,
            style,
            constraints,
        )
    }

    fn text_highlights(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        selection: (usize, usize),
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
    ) -> Vec<LayoutBox> {
        self.inner
            .text_highlights(id, text, selection, style, constraints)
    }

    fn shape(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: crate::TextShapeConstraints,
    ) -> TextMetrics {
        self.runs = self.runs.saturating_add(1);
        if constraints.wrap {
            self.wrap_layouts = self.wrap_layouts.saturating_add(1);
        }
        inner_shape(self.inner, id, text, style, constraints)
    }
}

fn inner_shape(
    shaper: &mut (impl TextShaper + ?Sized),
    id: StableNodeId,
    text: &TextContent,
    style: &ComputedStyle,
    constraints: crate::TextShapeConstraints,
) -> TextMetrics {
    #[cfg(any(test, feature = "benchmark"))]
    {
        crate::text_shape_stats::timed_inner_shape(|| shaper.shape(id, text, style, constraints))
    }
    #[cfg(not(any(test, feature = "benchmark")))]
    {
        shaper.shape(id, text, style, constraints)
    }
}

pub(super) fn clone_shaped_text(
    world: &UiWorld,
    id: StableNodeId,
    presentation: Option<&super::TextInputPresentationSource>,
) -> TextContent {
    let clone = || {
        presentation.as_ref().map_or_else(
            || world.nodes.text_content(id).clone(),
            |source| source.text.clone(),
        )
    };
    #[cfg(any(test, feature = "benchmark"))]
    let text = crate::text_shape_stats::timed_clone(clone);
    #[cfg(not(any(test, feature = "benchmark")))]
    let text = clone();
    world.record_string_clone(text.value.len());
    #[cfg(any(test, feature = "benchmark"))]
    crate::text_shape_stats::note_clone(text.value.len());
    text
}

pub(in crate::world) fn shape_empty_state_text(
    id: StableNodeId,
    visual: &crate::EmptyStateVisual,
    inherited: &ComputedStyle,
    max_width: Option<f32>,
    shaper: &mut impl TextShaper,
    typography: nana_ui_core::TypographyTokens,
) -> EmptyStateTextPresentation {
    let crate::EmptyStateVisual {
        title,
        message,
        compact,
        ..
    } = visual;
    let mut title_style = inherited.clone();
    title_style.font_size = if *compact {
        typography.meta
    } else {
        typography.body
    };
    title_style.font_weight = Some(typography.semibold);
    title_style.line_height = None;
    let mut message_style = inherited.clone();
    message_style.font_size = if *compact {
        typography.hint
    } else {
        typography.meta
    };
    message_style.font_weight = None;
    message_style.line_height = None;
    let constraints = crate::TextShapeConstraints {
        max_width,
        wrap: max_width.is_some(),
        shaping: crate::TextShaping::Auto,
        ..crate::TextShapeConstraints::default()
    };
    EmptyStateTextPresentation {
        title: shaper.shape(
            id,
            &TextContent {
                value: title.to_string().into(),
            },
            &title_style,
            constraints,
        ),
        message: message.as_ref().map(|message| {
            shaper.shape(
                id,
                &TextContent {
                    value: message.to_string().into(),
                },
                &message_style,
                constraints,
            )
        }),
    }
}

/// `max_width` wraps the title and its description; `body_width` wraps the
/// message, which runs across the body rather than beside the icon and the
/// close button, and under no title cap.
pub(in crate::world) fn shape_modal_text(
    id: StableNodeId,
    visual: &crate::ModalFrameVisual,
    inherited: &ComputedStyle,
    recipe: &nana_ui_core::DialogRecipe,
    max_width: Option<f32>,
    body_width: Option<f32>,
    shaper: &mut impl TextShaper,
    typography: nana_ui_core::TypographyTokens,
) -> ModalTextPresentation {
    let crate::ModalFrameVisual {
        title,
        description,
        body_text,
        kind,
        ..
    } = visual;
    let constraints = crate::TextShapeConstraints {
        max_width,
        wrap: max_width.is_some(),
        shaping: crate::TextShaping::Auto,
        ..Default::default()
    };
    let body_constraints = crate::TextShapeConstraints {
        max_width: body_width,
        wrap: body_width.is_some(),
        ..constraints
    };
    let title_type = crate::overlay_surfaces::modal_title_type(*kind, recipe, typography);
    let mut title_style = inherited.clone();
    title_style.font_size = title_type.size;
    if let Some(weight) = title_type.weight {
        title_style.font_weight = Some(weight);
    }
    title_style.line_height = None;
    let mut description_style = inherited.clone();
    description_style.font_size = typography.meta;
    description_style.font_weight = None;
    description_style.line_height = None;
    let mut body_style = inherited.clone();
    body_style.font_size = typography.body;
    body_style.font_weight = None;
    body_style.line_height = None;
    ModalTextPresentation {
        title: shaper.shape(
            id,
            &TextContent {
                value: title.to_string().into(),
            },
            &title_style,
            constraints,
        ),
        description: description.as_ref().map(|value| {
            shaper.shape(
                id,
                &TextContent {
                    value: value.to_string().into(),
                },
                &description_style,
                constraints,
            )
        }),
        body: body_text.as_ref().map(|value| {
            shaper.shape(
                id,
                &TextContent {
                    value: value.to_string().into(),
                },
                &body_style,
                body_constraints,
            )
        }),
    }
}

#[cfg(test)]
mod counting_probe_tests {
    use super::*;

    #[derive(Default)]
    struct GeometryShaper {
        batches: usize,
        shapes: usize,
    }
    impl TextShaper for GeometryShaper {
        fn shape(
            &mut self,
            _: StableNodeId,
            _: &TextContent,
            _: &ComputedStyle,
            _: crate::TextShapeConstraints,
        ) -> TextMetrics {
            self.shapes += 1;
            TextMetrics {
                width: 123.0,
                height: 72.0,
                ascent: None,
            }
        }
        fn with_text_probes<R>(
            &mut self,
            _: &TextContent,
            _: &ComputedStyle,
            _: crate::TextShapeConstraints,
            consume: impl FnOnce(&mut dyn TextShaper) -> R,
        ) -> R {
            self.batches += 1;
            consume(self)
        }
        fn horizontal_offset(
            &mut self,
            _: StableNodeId,
            _: &TextContent,
            _: usize,
            _: &ComputedStyle,
        ) -> f32 {
            31.0
        }
        fn text_position(
            &mut self,
            _: StableNodeId,
            _: &TextContent,
            _: usize,
            _: &ComputedStyle,
            _: crate::TextShapeConstraints,
        ) -> (f32, f32, f32) {
            (31.0, 48.0, 24.0)
        }
        fn text_highlights(
            &mut self,
            _: StableNodeId,
            _: &TextContent,
            _: (usize, usize),
            _: &ComputedStyle,
            _: crate::TextShapeConstraints,
        ) -> Vec<LayoutBox> {
            vec![LayoutBox {
                x: 31.0,
                y: 48.0,
                width: 17.0,
                height: 24.0,
            }]
        }
    }

    #[test]
    fn counting_adapter_preserves_host_geometry_batch_and_run_counts() {
        let mut host = GeometryShaper::default();
        let id = StableNodeId::new(1).unwrap();
        let text = TextContent {
            value: "first\nsecond".into(),
        };
        let style = ComputedStyle::default();
        let constraints = crate::TextShapeConstraints {
            wrap: true,
            ..Default::default()
        };
        let mut adapter = CountingShaper::new(&mut host);
        assert_eq!(
            adapter.text_position(id, &text, 8, &style, constraints),
            (31.0, 48.0, 24.0)
        );
        adapter.with_text_probes(&text, &style, constraints, |prepared| {
            assert_eq!(
                prepared.text_position(id, &text, 8, &style, constraints),
                (31.0, 48.0, 24.0)
            );
            assert_eq!(prepared.horizontal_offset(id, &text, 8, &style), 31.0);
            assert_eq!(
                prepared.text_highlights(id, &text, (6, 8), &style, constraints)[0].y,
                48.0
            );
            assert_eq!(prepared.shape(id, &text, &style, constraints).height, 72.0);
            assert_eq!(prepared.shape(id, &text, &style, constraints).height, 72.0);
        });
        assert_eq!(adapter.runs, 2);
        assert_eq!(adapter.wrap_layouts, 2);
        assert_eq!(host.shapes, 2);
        assert_eq!(host.batches, 1);
    }
}
