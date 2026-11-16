//! Overlay formatting-context slot placement. Uses the shared intrinsic and
//! placement authority; slots remain ordinary retained child identities.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn place_modal_children(
    _id: StableNodeId,
    origin: Point,
    size: Size,
    modal: &crate::ModalLayoutInput,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    let world = nodes.world;
    let recipe = *world.theme().recipes().dialog();
    let measure = |title: crate::TextMetrics| {
        crate::overlay_surfaces::ModalChrome::measure_in(
            modal.kind,
            title,
            modal.description,
            &modal.slots,
            &recipe,
            world.theme().border().hairline,
            world.theme(),
        )
    };
    let root = LayoutBox {
        x: origin.x,
        y: origin.y,
        width: size.width,
        height: size.height,
    };
    // A title content slot stands in the title's place: the row measures
    // it as it would the title text, as wide as the title may run. How
    // wide that is reads only the card's width, which its height does not
    // move.
    let title_content = match modal.slots.title_content {
        Some(id) if nodes.get(id)?.is_some_and(|node| !node.style.omits_box()) => {
            let provisional =
                crate::overlay_surfaces::modal_surface_bounds(root, modal.kind, None, &recipe);
            let width = measure(modal.title).text_width(provisional, root);
            let measured = intrinsic_size_scoped(
                id,
                Size::new(width, provisional.height),
                Some(FlexDirection::Row),
                viewport,
                parent_font_px,
                nodes,
                intrinsic,
                scope,
            )?;
            Some((id, measured.height.min(provisional.height)))
        }
        _ => None,
    };
    let chrome = measure(
        title_content.map_or(modal.title, |(_, height)| crate::TextMetrics {
            width: 0.0,
            height,
            ascent: None,
        }),
    );
    let body_copy = modal.body_text.map_or(0.0, |metrics| metrics.height);
    let body_gap = if body_copy > 0.0 && modal.slots.body.is_some() {
        nodes.world.theme().spacing().md
    } else {
        0.0
    };
    let surface = match modal.kind {
        crate::ModalSurfaceKind::Dialog(_) | crate::ModalSurfaceKind::Confirm(_) => {
            let provisional =
                crate::overlay_surfaces::modal_surface_bounds(root, modal.kind, None, &recipe);
            let body_available = Size::new(
                (provisional.width - chrome.body_x * 2.0).max(0.0),
                (provisional.height
                    - chrome.header_height
                    - chrome.body_pad_top
                    - chrome.body_pad_bottom
                    - chrome.footer_height
                    - body_copy
                    - body_gap)
                    .max(0.0),
            );
            let body_slot = if let Some(id) = modal.slots.body {
                if nodes.get(id)?.is_some() {
                    intrinsic_size_scoped(
                        id,
                        body_available,
                        Some(FlexDirection::Column),
                        viewport,
                        parent_font_px,
                        nodes,
                        intrinsic,
                        scope,
                    )?
                    .height
                    .min(body_available.height)
                } else {
                    0.0
                }
            } else {
                0.0
            };
            crate::overlay_surfaces::modal_surface_bounds(
                root,
                modal.kind,
                Some(chrome.chrome_height(body_copy + body_gap + body_slot)),
                &recipe,
            )
        }
        _ => crate::overlay_surfaces::modal_surface_bounds(root, modal.kind, None, &recipe),
    };
    let body = chrome.body_box(surface);
    let slot_y = body.y
        + if body_copy > 0.0 {
            body_copy + body_gap
        } else {
            0.0
        };
    let slot_height = (body.y + body.height - slot_y).max(0.0);
    if let Some(id) = modal.slots.body
        && nodes.get(id)?.is_some()
    {
        place_modal_slot(
            id,
            Point {
                x: body.x,
                y: slot_y,
            },
            Size::new(body.width, slot_height),
            Size::new(body.width, slot_height),
            viewport,
            parent_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
    }
    if let Some((id, height)) = title_content {
        let (x, y) = chrome.title_origin(surface);
        let width = chrome.text_width(surface, root);
        place_modal_slot(
            id,
            Point { x, y },
            Size::new(width, height),
            Size::new(width, height),
            viewport,
            parent_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
    }
    for (id, square) in [
        (modal.slots.title_icon, chrome.icon_box(surface)),
        (modal.slots.close_action, chrome.close_box(surface)),
    ] {
        let (Some(id), Some(square)) = (id, square) else {
            continue;
        };
        if nodes.get(id)?.is_none() {
            continue;
        }
        place_modal_slot(
            id,
            Point {
                x: square.x,
                y: square.y,
            },
            Size::new(square.width, square.height),
            Size::new(square.width, square.height),
            viewport,
            parent_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
    }
    let footer = chrome.footer_box(surface);
    let action_y = chrome.action_y(surface);
    let mut action_right = footer.x + footer.width;
    let mut actions = Vec::new();
    for id in modal.slots.actions.iter().rev().copied() {
        if nodes.get(id)?.is_some() {
            actions.push(id);
        }
    }
    for id in actions {
        let measured = intrinsic_size_scoped(
            id,
            Size::new(
                footer.width,
                nana_ui_core::ControlSize::Medium.height_in(nodes.world.theme_metrics()),
            ),
            Some(FlexDirection::Row),
            viewport,
            parent_font_px,
            nodes,
            intrinsic,
            scope,
        )?;
        let action_size = Size::new(
            measured.width.min(footer.width),
            measured
                .height
                .min(nana_ui_core::ControlSize::Medium.height_in(nodes.world.theme_metrics())),
        );
        action_right -= action_size.width;
        place_modal_slot(
            id,
            Point {
                x: action_right,
                y: action_y,
            },
            action_size,
            Size::new(footer.width, chrome.footer_height),
            viewport,
            parent_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
        action_right -= chrome.action_gap;
    }
    if let Some(id) = modal.slots.footer
        && nodes.get(id)?.is_some()
    {
        let width = (action_right - footer.x).max(0.0);
        place_modal_slot(
            id,
            Point {
                x: footer.x,
                y: footer.y,
            },
            Size::new(width, footer.height),
            Size::new(width, footer.height),
            viewport,
            parent_font_px,
            nodes,
            intrinsic,
            output,
            scope,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn place_modal_slot(
    id: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    viewport: LayoutViewport,
    parent_font_px: f32,
    nodes: &mut LayoutInputMap<'_>,
    intrinsic: &mut PassIntrinsicCache,
    output: &mut PassOutput,
    scope: Option<&ScopeContext<'_>>,
) -> Result<(), UiWorldError> {
    let Some(child_style) = nodes.get(id)?.map(|node| node.style.to_arc()) else {
        return Ok(());
    };
    if subtree_unchanged(
        id,
        origin,
        size,
        containing,
        child_style.as_ref(),
        fonts_of(child_style.as_ref(), parent_font_px),
        nodes.world.containing_writing(id),
        scope,
    ) {
        return Ok(());
    }
    place_node_scoped(
        id,
        origin,
        size,
        containing,
        viewport,
        parent_font_px,
        nodes,
        intrinsic,
        output,
        scope,
    )
}
