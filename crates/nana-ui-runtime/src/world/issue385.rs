//! Issue #385: an empty state's icon, title and message, a titled card's
//! header and an assembled toast's mark are parts laid out before the
//! children, inside the padding. The padding a box lays out with is the one
//! it declares (an author's, or the component's default), the parts start
//! where it ends and the children start past them; author padding, a theme
//! switch and parts coming and going lay out what a cold layout does.
#![cfg(test)]

use super::reflow_oracle::{assert_matches_cold, bundled_face_shaper, product_frame};
use crate::{
    AppContext, Button, DocumentId, Entity, LayoutPartKind, LayoutViewport, StableNodeId, Stack,
    Text,
};
use nana_ui_core::{
    FlexDirection, Icon, LengthSpec, ThemeAppearance, ThemeDefinition, ToastGlyphs, ToastIndicator,
    ToastRecipe,
};
use std::sync::Arc;

fn viewport() -> LayoutViewport {
    LayoutViewport::new(700.0, 600.0)
}

/// Spacing and a toast recipe moved: the paddings the components default
/// to, the gaps their parts read, and a toast's insets, mark and gap.
fn moved_theme() -> ThemeDefinition {
    let mut theme = ThemeDefinition::for_appearance(ThemeAppearance::Light);
    theme.tokens.spacing.xxs += 1.0;
    theme.tokens.spacing.xs += 1.0;
    theme.tokens.spacing.sm += 2.0;
    theme.tokens.spacing.md += 3.0;
    theme.tokens.spacing.xxxl += 4.0;
    theme.tokens.spacing.page += 3.0;
    theme.tokens.spacing.page_tight += 2.0;
    theme.bump().with_toast(ToastRecipe {
        indicator: ToastIndicator::Dot { size: 9.0 },
        padding: nana_ui_core::DialogInsets::new(14.0, 6.0, 20.0),
        gap: 5.0,
        ..ToastRecipe::DEFAULT
    })
}

/// A toast mark taller than the copy, and one with no mark at all.
fn toast_theme(indicator: ToastIndicator) -> ThemeDefinition {
    ThemeDefinition::NANA_DARK.with_toast(ToastRecipe {
        indicator,
        ..ToastRecipe::DEFAULT
    })
}

fn tall_mark() -> ToastIndicator {
    ToastIndicator::Glyph {
        size: 40.0,
        glyphs: ToastGlyphs {
            info: Icon::About,
            success: Icon::ShieldCheck,
            warning: Icon::Eye,
            danger: Icon::Close,
        },
    }
}

/// Padding an author writes on the box.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Padding {
    Default,
    /// The edge the parts are laid out from.
    Lead(f32),
    /// The other edges.
    Rest(f32),
    /// The `padding` shorthand alone, on every edge.
    Uniform(f32),
}

impl Padding {
    fn apply(self, layout: &mut nana_ui_core::LayoutStyle, axis: FlexDirection) {
        match (self, axis) {
            (Self::Default, _) => {}
            (Self::Lead(px), FlexDirection::Column) => {
                layout.padding_top = Some(LengthSpec::Px(px))
            }
            (Self::Lead(px), FlexDirection::Row) => layout.padding_left = Some(LengthSpec::Px(px)),
            (Self::Rest(px), _) => {
                layout.padding_left = Some(LengthSpec::Px(px));
                layout.padding_right = Some(LengthSpec::Px(px));
                layout.padding_bottom = Some(LengthSpec::Px(px));
            }
            (Self::Uniform(px), _) => layout.padding = Some(LengthSpec::Px(px)),
        }
    }
}

/// Where layout put `id`'s padding, parts and first child, against what it
/// declares: the content box is the padding box inset by the declared
/// padding (the parts' room is not padding), the band at its start is
/// `extent` long, and the first child starts past it.
fn assert_laid_out_before_children(
    cx: &AppContext,
    id: StableNodeId,
    padding: Padding,
    first_part: Option<LayoutPartKind>,
    child: Option<StableNodeId>,
    extent: f32,
    axis: FlexDirection,
) {
    let world = cx.world();
    let result = world.layout_result(id).unwrap();
    let declared = world.record(id).resolved_layout.resolved_padding();
    let (padding_box, content) = (result.padding_box, result.content_box);
    let used = (content.x - padding_box.x, content.y - padding_box.y);
    assert!((used.0 - declared.left).abs() < 1e-3, "{id:?}: {used:?}");
    assert!((used.1 - declared.top).abs() < 1e-3, "{id:?}: {used:?}");
    if let Padding::Lead(px) = padding {
        let used = match axis {
            FlexDirection::Column => used.1,
            FlexDirection::Row => used.0,
        };
        assert!((used - px).abs() < 1e-3, "{id:?}: {used} for {px}");
    }
    if let Padding::Uniform(px) = padding {
        let used = [
            content.y - padding_box.y,
            padding_box.x + padding_box.width - content.x - content.width,
            padding_box.y + padding_box.height - content.y - content.height,
            content.x - padding_box.x,
        ];
        assert!(
            used.iter().all(|edge| (edge - px).abs() < 1e-3),
            "{id:?}: {used:?} for {px}"
        );
    }
    let lead = world.leading_parts(id).unwrap_or_default();
    assert_eq!(lead.extent, extent, "{id:?}");
    let start = |bounds: crate::LayoutBox| match axis {
        FlexDirection::Column => (bounds.y, content.y),
        FlexDirection::Row => (bounds.x, content.x),
    };
    if let Some(kind) = first_part {
        let part = result
            .parts
            .iter()
            .find(|part| part.kind == kind)
            .unwrap_or_else(|| panic!("{kind:?} of {id:?}"))
            .bounds;
        let (at, from) = start(part);
        assert!(
            (at - from).abs() < 1e-3,
            "{kind:?} of {id:?}: {at} from {from}"
        );
    }
    if let Some(child) = child {
        let (at, from) = start(world.layout_box(child).unwrap());
        assert!(
            (at - from - extent).abs() < 1e-3,
            "child of {id:?}: {at} from {from} + {extent}"
        );
    }
}

#[derive(Clone, Copy, Debug)]
struct EmptyCase {
    padding: Padding,
    icon: bool,
    message: bool,
    compact: bool,
}

fn empty_state_view(case: EmptyCase) -> crate::EmptyState {
    let mut view = crate::EmptyState::new("No files").compact(case.compact);
    view.message = case.message.then(|| "Drop files here to share them".into());
    view.icon = case.icon.then_some(Icon::File);
    case.padding
        .apply(Arc::make_mut(&mut view.style.layout), FlexDirection::Column);
    view
}

/// An empty state with an action, in a 320px column above a button, under
/// `theme` when given, laid out once.
fn empty_state_page(
    case: EmptyCase,
    theme: Option<&ThemeDefinition>,
) -> (AppContext, Entity<crate::EmptyState>, StableNodeId) {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    if let Some(theme) = theme {
        cx.set_theme_definition(theme).unwrap();
    }
    let root = cx
        .create_component(document, Stack::column(4.0).width(LengthSpec::Px(320.0)))
        .unwrap();
    let empty = cx
        .create_detached_component(document, empty_state_view(case))
        .unwrap();
    cx.append_child(root, empty).unwrap();
    let action = cx
        .create_detached_component(document, Button::new("Upload"))
        .unwrap();
    cx.set_empty_state_action(empty, Some(action.stable_id()))
        .unwrap();
    let below = cx
        .create_detached_component(document, Button::new("Below"))
        .unwrap();
    cx.append_child(root, below).unwrap();
    product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
    (cx, empty, action.stable_id())
}

fn assert_empty_state(cx: &AppContext, id: StableNodeId, action: StableNodeId, case: EmptyCase) {
    let world = cx.world();
    let presentation = *world.nodes.empty_state_text(id).unwrap();
    let inputs = world.empty_state_inputs(id).unwrap();
    let size = |metrics: crate::TextMetrics| {
        crate::layout_engine::Size::new(metrics.width, metrics.height)
    };
    let block = inputs.block_height(size(presentation.title), presentation.message.map(size));
    let first = if case.icon {
        LayoutPartKind::LeadingContent
    } else {
        LayoutPartKind::Header
    };
    assert_laid_out_before_children(
        cx,
        id,
        case.padding,
        Some(first),
        Some(action),
        block,
        FlexDirection::Column,
    );
    // The default padding is the theme's spacing on every side, the edge
    // before the block too.
    if case.padding == Padding::Default {
        let spacing = world.theme().spacing();
        let (inline, block) = if case.compact {
            (spacing.sm, spacing.md)
        } else {
            (spacing.xxxl, spacing.page)
        };
        let result = world.layout_result(id).unwrap();
        let used = (
            result.content_box.x - result.padding_box.x,
            result.content_box.y - result.padding_box.y,
        );
        assert_eq!(used, (inline, block), "{case:?}");
    }
}

/// An empty state under author padding on each side of its block or the
/// `padding` shorthand alone, a theme that moves its default padding and
/// gaps, and its icon and message coming and going: its block starts where
/// the padding ends, the action starts past it, and each step lays out what
/// a cold layout does.
#[test]
fn issue385_empty_state_block_lays_out_before_its_action_like_a_cold_layout() {
    let document = DocumentId::new(1).unwrap();
    let moved = moved_theme();
    for compact in [false, true] {
        let first = EmptyCase {
            padding: Padding::Default,
            icon: false,
            message: false,
            compact,
        };
        let mut shaper = bundled_face_shaper();
        let (mut cx, empty, action) = empty_state_page(first, None);
        assert_empty_state(&cx, empty.stable_id(), action, first);
        let mut themed_now = false;
        let steps = [
            (Padding::Lead(6.0), true, false, false),
            (Padding::Lead(6.0), true, true, false),
            (Padding::Rest(40.0), false, true, false),
            (Padding::Lead(0.0), true, true, true),
            (Padding::Default, true, true, true),
            (Padding::Lead(18.0), false, false, true),
            (Padding::Default, true, false, true),
            (Padding::Uniform(20.0), true, true, true),
            (Padding::Uniform(7.0), false, false, true),
        ];
        for (padding, icon, message, themed) in steps {
            let case = EmptyCase {
                padding,
                icon,
                message,
                compact,
            };
            cx.update_component(empty, |view, _| {
                let action = view.action;
                *view = empty_state_view(case);
                view.action = action;
            })
            .unwrap();
            if themed && !themed_now {
                cx.set_theme_definition(&moved).unwrap();
                themed_now = true;
            }
            product_frame(&mut cx, document, viewport(), &mut shaper);
            let (mut cold, cold_empty, _) = empty_state_page(case, themed.then_some(&moved));
            assert_eq!(cold_empty.stable_id(), empty.stable_id());
            assert_matches_cold(&mut cx, &mut cold, document);
            assert_empty_state(&cx, empty.stable_id(), action, case);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct CardCase {
    padding: Padding,
    title: bool,
    loading: bool,
}

fn card_view(case: CardCase) -> crate::Card {
    let mut card = crate::Card::new().loading(case.loading);
    if case.title {
        card = card.title("Activity");
    }
    let layout = Arc::make_mut(&mut card.style.layout);
    layout.width = Some(LengthSpec::Px(240.0));
    case.padding.apply(layout, FlexDirection::Column);
    card
}

/// A 240px card over a line of body text, in a row, under `theme` when
/// given, laid out once.
fn card_page(
    case: CardCase,
    theme: Option<&ThemeDefinition>,
) -> (AppContext, Entity<crate::Card>, StableNodeId) {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    if let Some(theme) = theme {
        cx.set_theme_definition(theme).unwrap();
    }
    let root = cx
        .create_component(
            document,
            Stack::row(8.0)
                .width(LengthSpec::Px(600.0))
                .with_layout(|style| style.align_items = nana_ui_core::AlignSpec::Start),
        )
        .unwrap();
    let card = cx
        .create_detached_component(document, card_view(case))
        .unwrap();
    cx.append_child(root, card).unwrap();
    let body = cx
        .create_detached_component(document, Text::new("Body"))
        .unwrap();
    cx.append_child(card, body).unwrap();
    product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
    (cx, card, body.stable_id())
}

/// A card under author padding above and beside its header, a theme
/// switch, and its title and spinner coming and going: the header starts
/// where the padding ends, the body starts past its band (and where the
/// padding ends without a title), the hugging height counts the band once,
/// and each step lays out what a cold layout does.
#[test]
fn issue385_card_header_lays_out_before_its_body_like_a_cold_layout() {
    let document = DocumentId::new(1).unwrap();
    let moved = moved_theme();
    let first = CardCase {
        padding: Padding::Default,
        title: true,
        loading: false,
    };
    let mut shaper = bundled_face_shaper();
    let (mut cx, card, body) = card_page(first, None);
    let mut themed_now = false;
    let steps = [
        (first, false),
        (
            CardCase {
                padding: Padding::Lead(4.0),
                ..first
            },
            false,
        ),
        (
            CardCase {
                padding: Padding::Lead(4.0),
                title: false,
                loading: false,
            },
            false,
        ),
        (
            CardCase {
                padding: Padding::Rest(20.0),
                title: true,
                loading: true,
            },
            false,
        ),
        (
            CardCase {
                padding: Padding::Lead(0.0),
                title: true,
                loading: true,
            },
            true,
        ),
        (
            CardCase {
                padding: Padding::Default,
                title: false,
                loading: true,
            },
            true,
        ),
        (first, true),
    ];
    for (case, themed) in steps {
        cx.update_component(card, |view, _| *view = card_view(case))
            .unwrap();
        if themed && !themed_now {
            cx.set_theme_definition(&moved).unwrap();
            themed_now = true;
        }
        product_frame(&mut cx, document, viewport(), &mut shaper);
        let (mut cold, cold_card, _) = card_page(case, themed.then_some(&moved));
        assert_eq!(cold_card.stable_id(), card.stable_id());
        assert_matches_cold(&mut cx, &mut cold, document);
        let band = if case.title {
            crate::view_components::CARD_TITLE_BAND
        } else {
            0.0
        };
        let id = card.stable_id();
        assert_laid_out_before_children(
            &cx,
            id,
            case.padding,
            case.title.then_some(LayoutPartKind::Header),
            Some(body),
            band,
            FlexDirection::Column,
        );
        // The body's line and the band, once, between the paddings.
        let world = cx.world();
        let result = world.layout_result(id).unwrap();
        let body_box = world.layout_box(body).unwrap();
        assert!(
            (result.content_box.height - band - body_box.height).abs() < 1e-3,
            "{case:?}: {result:?}"
        );
    }
}

fn toast_view(description: bool) -> crate::Toast {
    let mut view = crate::Toast::new("Export complete", crate::ToastTone::Success);
    if description {
        view = view.description("Master sent to disk");
    }
    view
}

/// An assembled toast in a 420px column above a button, under `theme`
/// when given, laid out once.
fn toast_page(
    description: bool,
    theme: Option<&ThemeDefinition>,
) -> (AppContext, Entity<crate::Toast>) {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    if let Some(theme) = theme {
        cx.set_theme_definition(theme).unwrap();
    }
    let root = cx
        .create_component(document, Stack::column(4.0).width(LengthSpec::Px(420.0)))
        .unwrap();
    let toast = cx
        .create_detached_component(document, toast_view(description))
        .unwrap();
    cx.append_child(root, toast).unwrap();
    cx.assemble_toast(toast).unwrap();
    let below = cx
        .create_detached_component(document, Button::new("Below"))
        .unwrap();
    cx.append_child(root, below).unwrap();
    product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
    (cx, toast)
}

/// An assembled toast through recipes that move its insets, its gap and its
/// mark, a mark taller than its copy and no mark at all, each reached by a
/// theme switch on its own, then with its description coming and going: its
/// padding is the recipe's alone, the mark starts where it ends and the copy
/// past the mark and its gap, the content box is at least as tall as the
/// mark, and each step lays out what a cold layout does.
#[test]
fn issue385_assembled_toast_mark_lays_out_before_its_copy_like_a_cold_layout() {
    let document = DocumentId::new(1).unwrap();
    let mut shaper = bundled_face_shaper();
    let (mut cx, toast) = toast_page(false, None);
    let themes = [
        moved_theme(),
        toast_theme(tall_mark()),
        toast_theme(ToastIndicator::None),
        toast_theme(ToastIndicator::Dot { size: 8.0 }),
        toast_theme(ToastIndicator::Dot { size: 30.0 }),
    ];
    let mut description = false;
    for theme in &themes {
        // Consecutive recipes differ in what the parts read alone, so the
        // switch is the only thing that can move them.
        for switch in [true, false, false] {
            if switch {
                cx.set_theme_definition(theme).unwrap();
            } else {
                description = !description;
                cx.update_component(toast, |view, _| {
                    view.description = toast_view(description).description;
                })
                .unwrap();
            }
            product_frame(&mut cx, document, viewport(), &mut shaper);
            let (mut cold, cold_toast) = toast_page(description, Some(theme));
            assert_eq!(cold_toast.stable_id(), toast.stable_id());
            assert_matches_cold(&mut cx, &mut cold, document);
            let id = toast.stable_id();
            let world = cx.world();
            let recipe = *world.theme().recipes().toast();
            let mark = recipe.indicator.size();
            let lead = if matches!(recipe.indicator, ToastIndicator::None) {
                0.0
            } else {
                mark + recipe.gap
            };
            let result = world.layout_result(id).unwrap();
            let copy = world.child_ids(id)[0];
            assert_laid_out_before_children(
                &cx,
                id,
                Padding::Lead(recipe.padding.inline),
                (mark > 0.0).then_some(LayoutPartKind::LeadingContent),
                Some(copy),
                lead,
                FlexDirection::Row,
            );
            assert!(result.content_box.height >= mark - 1e-3, "{result:?}");
        }
    }
}
