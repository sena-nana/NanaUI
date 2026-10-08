mod grid;
use grid::*;
mod flow;
use flow::*;
mod measure;
use measure::*;
pub(crate) use measure::{depends_on_used_basis, spec_tracks_containing_block};
mod placement;
use placement::*;
mod inline;
use inline::*;
mod flex;
#[cfg(test)]
mod inline_scope;
use flex::*;
// These caches use internal numeric identities/constraint bits, not external text keys.
use hashbrown::HashMap;
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

use nana_ui_core::box_layout::text_line_box_height_px;
use nana_ui_core::{
    AlignSpec, BoxSizing, ClearSpec, ConstraintClass, DisplaySpec, FlexDirection, FlexWrap,
    FloatSpec, FontSizeContext, FormattingContext, GridAutoFlow, GridLine, GridPlacement,
    GridRepeatAuto, GridTemplateAreas, GridTrack, IntrinsicMetrics, JustifySpec, LayoutBehavior,
    LayoutFoundation, LayoutIntent, LayoutNode, LayoutNodeId, LayoutOwnership, LayoutPlacement,
    LayoutRect, LayoutResult as FoundationResult, LayoutSize, LayoutStyle, LengthSpec,
    Participation, PlacementMode, PositionSpec, ReplacedContent, TextAlignSpec,
    resolve_grid_track_sizes,
};

use crate::layout_frontier::{LayoutFrontier, LayoutFrontierSeed, LayoutFrontierStats};
use crate::{
    DocumentId, LayoutBox, LayoutInput, MutationQueue, NodeKind, NodeStyle, StableNodeId, UiWorld,
    UiWorldError,
};

/// Logical viewport supplied by the platform host to the retained layout system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutViewport {
    pub width: f32,
    pub height: f32,
}

impl LayoutViewport {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: finite_extent(width),
            height: finite_extent(height),
        }
    }
}

fn inset_foundation_rect(rect: LayoutRect, edges: nana_ui_core::PaddingSpec) -> LayoutRect {
    let left = edges.left.max(0.0);
    let right = edges.right.max(0.0);
    let top = edges.top.max(0.0);
    let bottom = edges.bottom.max(0.0);
    LayoutRect::new(
        rect.inline + left,
        rect.block + top,
        LayoutSize::new(
            (rect.size.inline - left - right).max(0.0),
            (rect.size.block - top - bottom).max(0.0),
        ),
    )
}

/// Backend-neutral layout owner used by canonical Runtime applications.
///
/// Consumes the same `LayoutStyle` and shaped text metrics stored in `UiWorld`
/// (flex wrap / `display:grid` 2D tracks·repeat·areas·placement via
/// `uses_2d_grid` / percent / calc / absolute / fixed / float / IFC subset
/// including shrink-to-avoid-float line boxes beside sibling floats)
/// and returns atomic layout writeback. Vue `measure_layout` and css-parity
/// call [`Self::layout_style_tree`] so mixed trees and fixtures share this
/// algorithm.
#[derive(Debug, Default, Clone, Copy)]
pub struct RuntimeLayoutEngine;

/// Style-only tree accepted by [`RuntimeLayoutEngine::layout_style_tree`].
///
/// Vue `LayoutNode` and css-parity fixtures adapt onto this type; they do not
/// keep a second layout algorithm.
#[derive(Debug, Clone, Default)]
pub struct StyleLayoutNode {
    pub id: String,
    pub style: LayoutStyle,
    pub children: Vec<StyleLayoutNode>,
    pub text: Option<String>,
}

impl RuntimeLayoutEngine {
    /// Run the existing layout oracle and expose its output through the shared
    /// Foundation contract.  The adapter owns no second tree: stable runtime
    /// IDs are lowered directly to `LayoutNodeId`s and the returned result is
    /// suitable for Scene, hit testing, accessibility, and scroll consumers.
    pub fn layout_document_foundation(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
    ) -> Result<LayoutFoundation, UiWorldError> {
        let mut foundation = LayoutFoundation::new();
        self.layout_document_foundation_into(world, document, viewport, &mut foundation)?;
        Ok(foundation)
    }

    /// Refresh an existing Foundation projection without rebuilding component
    /// identities.  The mature Runtime algorithm remains the geometry oracle;
    /// this method only lowers its node/result stream into shared contracts.
    pub fn layout_document_foundation_into(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
        foundation: &mut LayoutFoundation,
    ) -> Result<(), UiWorldError> {
        let boxes = self.layout_document(world, document, viewport)?;
        let order = world.document_order(document);
        foundation.retain_nodes(order.iter().filter_map(|id| LayoutNodeId::new(id.get())));
        let mut by_id = HashMap::with_capacity(boxes.len());
        for (id, bounds) in boxes.iter().copied() {
            by_id.insert(id, bounds);
        }
        // Keep the resolved style/parent projection local to this lowering
        // pass.  Foundation results must carry the same clip and containing
        // block dependencies as the runtime result, without consulting a
        // potentially stale retained snapshot from the previous frame.
        let mut styles = HashMap::with_capacity(order.len());
        let mut parents = HashMap::with_capacity(order.len());
        for id in order {
            let Some(layout_id) = LayoutNodeId::new(id.get()) else {
                continue;
            };
            let input = world.layout_input(id)?;
            styles.insert(id, Arc::clone(&input.style));
            parents.insert(id, input.parent);
            let context = if input.style.omits_box() {
                FormattingContext::None
            } else if input.style.display.is_none() && input.style.direction.is_some() {
                // Stack::row/column lower their parent-owned flow axis into
                // `direction` while leaving CSS display unspecified. Preserve
                // that facade contract in Foundation instead of silently
                // classifying the node as an ordinary block.
                FormattingContext::Flex
            } else {
                FormattingContext::from_display(input.style.display.unwrap_or(DisplaySpec::Block))
            };
            let placement = match input.style.position {
                PositionSpec::Absolute => PlacementMode::Absolute,
                PositionSpec::Fixed => PlacementMode::Fixed,
                PositionSpec::Sticky => PlacementMode::Sticky,
                PositionSpec::Static | PositionSpec::Relative => PlacementMode::NormalFlow,
            };
            let mut node = LayoutNode::new(
                layout_id,
                LayoutIntent {
                    default_direction: input.style.direction,
                    ..LayoutIntent::component_default(LayoutOwnership::default())
                },
                context,
            );
            node.children = input
                .children
                .iter()
                .filter_map(|child| LayoutNodeId::new(child.get()))
                .collect();
            node.placement = placement;
            let custom_render = world.custom_render(id);
            let replaced = custom_render.is_some()
                || input.style.paint.content_image.is_some()
                || input.style.paint.skipped_replaced.is_some();
            let intrinsic_size = by_id
                .get(&id)
                .map(|bounds| LayoutSize::new(bounds.width, bounds.height));
            let fit = custom_render
                .map(|render| match render.fit {
                    nana_ui_core::ContentFit::Cover => nana_ui_core::ObjectFit::Cover,
                    nana_ui_core::ContentFit::Fill => nana_ui_core::ObjectFit::Fill,
                    nana_ui_core::ContentFit::ScaleDown => nana_ui_core::ObjectFit::ScaleDown,
                    nana_ui_core::ContentFit::None => nana_ui_core::ObjectFit::None,
                    nana_ui_core::ContentFit::Contain => nana_ui_core::ObjectFit::Contain,
                })
                .unwrap_or_else(|| match input.style.paint.object_fit {
                    Some(nana_ui_core::BackgroundImageFit::Cover) => nana_ui_core::ObjectFit::Cover,
                    Some(nana_ui_core::BackgroundImageFit::Stretch) => {
                        nana_ui_core::ObjectFit::Fill
                    }
                    Some(nana_ui_core::BackgroundImageFit::ScaleDown) => {
                        nana_ui_core::ObjectFit::ScaleDown
                    }
                    Some(nana_ui_core::BackgroundImageFit::Auto) => nana_ui_core::ObjectFit::None,
                    Some(nana_ui_core::BackgroundImageFit::Contain) | None => {
                        nana_ui_core::ObjectFit::Contain
                    }
                    Some(nana_ui_core::BackgroundImageFit::Length) => nana_ui_core::ObjectFit::Fill,
                });
            let replaced_content = replaced.then(|| ReplacedContent {
                intrinsic_size,
                aspect_ratio: input.style.aspect_ratio,
                fit,
                resource_generation: custom_render.map_or(0, |render| render.revision),
                ..ReplacedContent::default()
            });
            node.participation = if let Some(content) = replaced_content {
                Participation::Replaced(content)
            } else if placement.is_out_of_flow() {
                Participation::OutOfFlow(placement)
            } else if input.text_metrics.is_some() {
                Participation::NativeTextInline
            } else {
                Participation::NormalFlow
            };
            node.behavior = LayoutBehavior {
                scroll_x: input.style.overflow_x.scrolls(),
                scroll_y: input.style.overflow_y.scrolls(),
                clip: input.style.overflow_x.clips() || input.style.overflow_y.clips(),
                viewport: input.style.overflow_x.scrolls() || input.style.overflow_y.scrolls(),
                scroll_offset: world
                    .scroll_request(id)
                    .map(|offset| LayoutSize::new(offset.x, offset.y))
                    .unwrap_or(LayoutSize::ZERO),
            };
            // A custom render revision is a presentation frame/resource
            // update. Keep the retained participation while upserting the
            // structural node, then use Foundation's replaced-content path so
            // frame churn cannot masquerade as a layout participation remap.
            let previous_replaced = foundation
                .node(layout_id)
                .and_then(|existing| match existing.participation {
                    Participation::Replaced(content) => Some(content),
                    _ => None,
                });
            if previous_replaced.is_some() && replaced_content.is_some() {
                node.participation = Participation::Replaced(previous_replaced.unwrap());
            }
            foundation.upsert(node);
            if let (Some(previous), Some(content)) = (previous_replaced, replaced_content) {
                let intrinsic_metadata_changed = previous.intrinsic_size != content.intrinsic_size
                    || previous.aspect_ratio != content.aspect_ratio
                    || previous.baseline != content.baseline;
                foundation.set_replaced_content(layout_id, content, intrinsic_metadata_changed);
            }
            if let Some(metrics) = input.text_metrics {
                let mut intrinsic =
                    IntrinsicMetrics::new(LayoutSize::new(metrics.width, metrics.height));
                intrinsic.first_baseline = metrics
                    .ascent
                    .filter(|value| value.is_finite())
                    .map(|value| value.max(0.0));
                foundation.set_metrics(layout_id, intrinsic);
            } else if replaced {
                if let Some(size) = intrinsic_size {
                    let mut intrinsic = IntrinsicMetrics::new(size);
                    intrinsic.aspect_ratio = input.style.aspect_ratio;
                    foundation.set_metrics(layout_id, intrinsic);
                }
            }
        }
        let retained_results = boxes
            .iter()
            .filter_map(|(id, _)| LayoutNodeId::new(id.get()))
            .filter(|id| {
                foundation
                    .node(*id)
                    .is_some_and(|node| node.formatting_context.generates_box())
            })
            .collect::<Vec<_>>();
        foundation.retain_results(retained_results);
        fn generated_children(
            foundation: &LayoutFoundation,
            parent: LayoutNodeId,
            output: &mut Vec<LayoutNodeId>,
        ) {
            let Some(node) = foundation.node(parent) else {
                return;
            };
            for child in node.children.iter().copied() {
                let Some(child_node) = foundation.node(child) else {
                    continue;
                };
                if child_node.formatting_context.is_transparent() {
                    generated_children(foundation, child, output);
                } else if child_node.formatting_context.generates_box() {
                    output.push(child);
                }
            }
        }
        for (id, box_) in boxes.into_iter().rev() {
            let Some(layout_id) = LayoutNodeId::new(id.get()) else {
                continue;
            };
            let Some(node) = foundation.node(layout_id) else {
                continue;
            };
            if !node.formatting_context.generates_box() {
                continue;
            }
            let bounds = LayoutRect::new(box_.x, box_.y, LayoutSize::new(box_.width, box_.height));
            let mut result = FoundationResult::new(layout_id, bounds, 0);
            if let Some(style) =
                styles.get(&StableNodeId::new(layout_id.get()).expect("nonzero id"))
            {
                let border = style.resolved_border_edges();
                let percent_base = parents
                    .get(&StableNodeId::new(layout_id.get()).expect("nonzero id"))
                    .copied()
                    .flatten()
                    .and_then(|parent| by_id.get(&parent).map(|bounds| bounds.width));
                let padding = style.resolved_padding_against(percent_base);
                result.padding_box = inset_foundation_rect(bounds, border);
                result.content_box = inset_foundation_rect(result.padding_box, padding);
                result.border_box = bounds;
                result.used_size = result.content_box.size.into();
            }
            let mut ancestor = parents
                .get(&StableNodeId::new(layout_id.get()).expect("nonzero id"))
                .copied()
                .flatten();
            let mut clip = None;
            let mut containing_block = None;
            while let Some(candidate) = ancestor {
                if let Some(style) = styles.get(&candidate) {
                    if clip.is_none() && style.clips_overflow() {
                        clip = LayoutNodeId::new(candidate.get());
                    }
                    if containing_block.is_none() && style.position.establishes_containing_block() {
                        containing_block = LayoutNodeId::new(candidate.get());
                    }
                }
                ancestor = parents.get(&candidate).copied().flatten();
            }
            if node.placement == PlacementMode::Fixed {
                containing_block = None;
            }
            let clip_dependency = clip;
            result.clip = clip_dependency
                .map(|clip| {
                    styles
                        .get(&StableNodeId::new(clip.get()).expect("nonzero id"))
                        .and_then(|_| {
                            by_id.get(&StableNodeId::new(clip.get()).expect("nonzero id"))
                        })
                        .map(|clip_box| {
                            LayoutRect::new(
                                clip_box.x,
                                clip_box.y,
                                LayoutSize::new(clip_box.width, clip_box.height),
                            )
                        })
                })
                .flatten();
            result.containing_block = containing_block;
            result.dependency_footprint.extend(
                clip_dependency
                    .into_iter()
                    .chain(containing_block)
                    .filter_map(|id| LayoutNodeId::new(id.get())),
            );
            let mut children = Vec::new();
            generated_children(foundation, layout_id, &mut children);
            for child in children {
                let Some(stable_child) = StableNodeId::new(child.get()) else {
                    continue;
                };
                let Some(child_box) = by_id.get(&stable_child) else {
                    continue;
                };
                if foundation
                    .node(child)
                    .is_some_and(|child| child.formatting_context.generates_box())
                {
                    result.children.push(LayoutPlacement {
                        node: child,
                        bounds: LayoutRect::new(
                            child_box.x,
                            child_box.y,
                            LayoutSize::new(child_box.width, child_box.height),
                        ),
                        participation: foundation
                            .node(child)
                            .map(|child| child.participate(node.formatting_context))
                            .unwrap_or(Participation::Unsupported),
                    });
                }
            }
            result
                .dependency_footprint
                .extend(result.children.iter().map(|child| child.node));
            result.fragments.push(nana_ui_core::LayoutFragment {
                node: layout_id,
                bounds,
                kind: if node.participation == Participation::NativeTextInline {
                    nana_ui_core::FragmentKind::Text
                } else {
                    nana_ui_core::FragmentKind::Box
                },
            });
            result.overflow = result.children.iter().fold(bounds, |current, child| {
                let child_overflow = foundation
                    .result(child.node)
                    .map(|result| result.overflow)
                    .unwrap_or(child.bounds);
                let right = (current.inline + current.size.inline)
                    .max(child_overflow.inline + child_overflow.size.inline);
                let bottom = (current.block + current.size.block)
                    .max(child_overflow.block + child_overflow.size.block);
                LayoutRect::new(
                    current.inline.min(child_overflow.inline),
                    current.block.min(child_overflow.block),
                    LayoutSize::new(
                        (right - current.inline.min(child_overflow.inline)).max(0.0),
                        (bottom - current.block.min(child_overflow.block)).max(0.0),
                    ),
                )
            });
            result.scroll_extent = nana_ui_core::UsedSize::new(
                result.overflow.size.inline,
                result.overflow.size.block,
            );
            if let Some(metrics) = foundation.metrics(layout_id, ConstraintClass::Unconstrained) {
                result.first_baseline = metrics
                    .first_baseline
                    .map(|baseline| result.content_box.block + baseline);
                result.last_baseline = metrics
                    .last_baseline
                    .map(|baseline| result.content_box.block + baseline);
            }
            if let Participation::Replaced(content) = node.participation {
                let baseline = match content.baseline {
                    nana_ui_core::BaselinePolicy::Bottom => {
                        Some(result.content_box.block + result.content_box.size.block)
                    }
                    nana_ui_core::BaselinePolicy::Center => {
                        Some(result.content_box.block + result.content_box.size.block * 0.5)
                    }
                    nana_ui_core::BaselinePolicy::FirstBaseline => {
                        result.first_baseline.or(Some(result.content_box.block))
                    }
                };
                result.first_baseline = result.first_baseline.or(baseline);
                result.last_baseline = result.last_baseline.or(baseline);
            }
            foundation.publish_result(result);
        }
        Ok(())
    }

    pub fn layout_document(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
    ) -> Result<Vec<(StableNodeId, LayoutBox)>, UiWorldError> {
        let order = world.document_order(document);
        let mut nodes = LayoutInputMap::new(world);
        nodes.prefetch(&order)?;
        let roots = world.document_roots(document);
        let mut output = HashMap::with_capacity(nodes.len());
        let mut intrinsic = PassIntrinsicCache::with_capacity(nodes.len());
        let available = Size::new(viewport.width, viewport.height);
        for root in roots {
            let root_size = intrinsic_size(
                root,
                available,
                None,
                viewport,
                ROOT_FONT_PX,
                &mut nodes,
                &mut intrinsic,
            )?;
            place_node(
                root,
                Point::ZERO,
                root_size,
                available,
                viewport,
                ROOT_FONT_PX,
                &mut nodes,
                &mut intrinsic,
                &mut output,
            )?;
        }
        Ok(order
            .into_iter()
            .map(|id| (id, output.remove(&id).unwrap_or_default()))
            .collect())
    }

    /// Incremental retained layout driven by typed invalidation seeds.
    ///
    /// `force_full` disables pruning (viewport semantics changed) and
    /// rebuilds the retained cache. Otherwise the dependency graph expands
    /// each typed seed only across edges whose footprint consumes its metric.
    pub fn layout_document_with_frontier(
        self,
        world: &UiWorld,
        document: DocumentId,
        viewport: LayoutViewport,
        typed_seeds: &[LayoutFrontierSeed],
        retained: &mut RetainedLayoutCache,
        force_full: bool,
    ) -> Result<Vec<(StableNodeId, LayoutBox)>, UiWorldError> {
        let roots = world.document_roots(document);
        if roots.is_empty() {
            retained.remove_document(document);
            return Ok(Vec::new());
        }
        let retained = retained.documents.entry(document).or_default();
        // Reset before the full-layout reuse return, which runs no measure or placement.
        retained.execution_stats = LayoutExecutionStats::default();
        if force_full && world.layout_source_reusable() {
            let viewport_width = viewport.width.to_bits();
            let viewport_height = viewport.height.to_bits();
            let epoch = world.layout_source_epoch();
            let reused = retained.full_snapshots.iter().find_map(|snapshot| {
                let snapshot = snapshot.as_ref()?;
                (snapshot.epoch == epoch
                    && snapshot.viewport_width == viewport_width
                    && snapshot.viewport_height == viewport_height)
                    .then(|| {
                        (
                            snapshot.emitted.clone(),
                            snapshot.used_padding.clone(),
                            snapshot.far_start.clone(),
                        )
                    })
            });
            if let Some((emitted, used_padding, far_start)) = reused {
                retained.boxes.clear();
                retained.boxes.extend(emitted.iter().copied());
                retained.used_padding = used_padding;
                retained.far_start = far_start;
                retained.intrinsics.clear();
                retained.intrinsic_metrics.clear();
                retained.placements.clear();
                retained.container_plans.clear();
                retained.measure_plans.clear();
                retained.materialized_inputs = emitted.len();
                return Ok(emitted);
            }
        }
        if force_full {
            retained.clear();
        }
        let mut nodes = LayoutInputMap::new(world);
        #[cfg(feature = "benchmark")]
        let mut phase = plan_stats::PhaseClock::start();
        if force_full {
            let order = world.document_order(document);
            nodes.prefetch(&order)?;
        }
        #[cfg(feature = "benchmark")]
        phase.lap(0);
        let frontier = if force_full {
            LayoutFrontier::default()
        } else {
            // Typed mutation authority path: use the retained dependency index
            // so parent constraints, containing blocks and local formatting
            // contexts participate in one deduplicated closure.
            let graph = world.layout_dependency_graph_for_seeds(document, typed_seeds);
            LayoutFrontier::from_dependency_graph(
                typed_seeds
                    .iter()
                    .copied()
                    .filter(|seed| world.document_of(seed.node) == Some(document)),
                &graph,
            )
        };
        let affected = if force_full {
            HashSet::new()
        } else {
            frontier.nodes().clone()
        };
        retained.frontier_stats = LayoutFrontierStats::from_frontier(&frontier);
        // Used-size memos on the measure frontier are stale. Intrinsic facts
        // stay until publication so an identical recompute does not bump generation.
        for id in frontier.measure_nodes() {
            retained.intrinsics.remove(id);
        }
        #[cfg(any(test, feature = "benchmark"))]
        plan_stats::note_scope(typed_seeds.len(), affected.len());
        #[cfg(any(test, feature = "benchmark"))]
        plan_stats::note_frontier(&frontier, force_full);
        let scope = ScopeContext {
            affected: &affected,
            measure: frontier.measure_nodes(),
            retained: &*retained,
        };
        let scope_ref = (!force_full).then_some(&scope);
        let mut output = HashMap::with_capacity(nodes.len());
        let mut intrinsic = PassIntrinsicCache::with_capacity(nodes.len());
        let available = Size::new(viewport.width, viewport.height);
        let islands = if force_full {
            Vec::new()
        } else {
            let mut islands = Vec::new();
            for id in affected.iter().copied() {
                // A dependency boundary may be a fixed-size ordinary ancestor
                // as well as an explicit isolation context. Use the retained
                // placement as the local layout root so a child can be
                // recomputed without pulling that stable ancestor into the
                // measure frontier. A fixed inline-block has the same role
                // for an inline formatting context: its border box is the
                // island, and the outer line is not packed again.
                let Some(parent) = world.parent_id(id) else {
                    continue;
                };
                if affected.contains(&parent) {
                    continue;
                }
                if retained.placements.contains_key(&id) && retained.boxes.contains_key(&id) {
                    let (origin, containing, font) = retained.placements[&id];
                    islands.push((id, origin, containing, font));
                    continue;
                }
                let (Some(border), Some(parent_box)) = (
                    retained.boxes.get(&id).copied(),
                    retained.boxes.get(&parent).copied(),
                ) else {
                    continue;
                };
                if let Some((origin, containing, font)) =
                    fixed_inline_block_island(world, id, border, parent_box)
                {
                    islands.push((id, origin, containing, font));
                }
            }
            islands
        };
        // An island whose own border box changed moves its siblings, which only
        // its parent places. Such a pass also walks from the document roots,
        // where the parent's retained plan replays the shift.
        let mut island_resized = false;
        for &(root, origin, containing, font) in &islands {
            // An island boundary normally has a stable used size, but an
            // affected content-sized child can grow inside a fixed ancestor.
            // Re-measure the island itself so its retained box does not freeze
            // the new intrinsic size before placement reaches its descendants.
            let root_size = intrinsic_size_scoped(
                root,
                containing,
                None,
                viewport,
                font,
                &mut nodes,
                &mut intrinsic,
                scope_ref,
            )?;
            island_resized |= retained.boxes.get(&root).is_none_or(|previous| {
                previous.width.to_bits() != root_size.width.to_bits()
                    || previous.height.to_bits() != root_size.height.to_bits()
            });
            place_node_scoped(
                root,
                origin,
                root_size,
                containing,
                viewport,
                font,
                &mut nodes,
                &mut intrinsic,
                &mut output,
                scope_ref,
                None,
            )?;
        }
        for root in roots {
            if !force_full && !islands.is_empty() && !island_resized && !affected.contains(&root) {
                continue;
            }
            let root_size = intrinsic_size_scoped(
                root,
                available,
                None,
                viewport,
                ROOT_FONT_PX,
                &mut nodes,
                &mut intrinsic,
                scope_ref,
            )?;
            #[cfg(feature = "benchmark")]
            phase.lap(1);
            place_node_scoped(
                root,
                Point::ZERO,
                root_size,
                available,
                viewport,
                ROOT_FONT_PX,
                &mut nodes,
                &mut intrinsic,
                &mut output,
                scope_ref,
                None,
            )?;
            #[cfg(feature = "benchmark")]
            phase.lap(2);
        }
        // Publish recomputed boxes from the placed set; no document_order walk.
        let mut emitted = output.into_iter().collect::<Vec<_>>();
        emitted.sort_unstable_by_key(|(id, _)| *id);
        for (id, box_) in &emitted {
            retained.boxes.insert(*id, *box_);
        }
        retained.used_padding.extend(nodes.used_padding.drain());
        retained.far_start.extend(nodes.far_start.drain());
        retained.placements.extend(nodes.placements.drain());
        for (id, plan) in nodes.container_plans.drain() {
            match plan {
                Some(mut plan) => {
                    // Gate E: one plan per container, entries are the direct
                    // participants recorded this pass. A repeat edit replaces
                    // the previous plan instead of appending another.
                    bound_container_plan(&mut plan);
                    retained.container_plans.insert(id, plan);
                }
                None => {
                    retained.container_plans.remove(&id);
                }
            }
        }
        for (id, plans) in nodes.measure_plans.drain() {
            if plans.is_empty() {
                retained.measure_plans.remove(&id);
                continue;
            }
            // Merge rather than replace. A container is commonly measured under
            // two constraints per pass but only RE-measured under one of them:
            // the other is answered from its cached plan, which records nothing.
            // Replacing would drop that constraint's plan, so the two slots
            // would alternate instead of holding both. The slot array evicts
            // any older constraint for this same container.
            let slots = retained.measure_plans.entry(id).or_default();
            for mut plan in plans.into_plans().collect::<Vec<_>>().into_iter().rev() {
                bound_measure_plan(&mut plan);
                slots.insert(plan);
            }
        }
        let intrinsic_counters = intrinsic.counters();
        retained.execution_stats = intrinsic.execution_stats;
        let universe = if force_full { nodes.len() } else { world.len() };
        for (key, size) in intrinsic.used {
            retained
                .intrinsics
                .entry(key.id)
                .or_default()
                .insert(key, size);
        }
        let retained_metric_budget = universe
            .saturating_mul(4)
            .max(1)
            .min(crate::IntrinsicCacheBudget::default().max_entries);
        retained.retain_intrinsic_metrics(intrinsic.new_metrics, retained_metric_budget);
        retained.materialized_inputs = nodes.materialized;
        retained.record_intrinsic_counters(intrinsic_counters);
        // Despawned ids linger in the retained maps; keep them bounded.
        // Scoped passes only materialize a subset, so membership is the live
        // world, not the partial input map.
        if retained.boxes.len() > universe.saturating_mul(2) {
            #[cfg(any(test, feature = "benchmark"))]
            plan_stats::note_retain_sweep();
            retained.boxes.retain(|id, _| world.contains(*id));
            retained.placements.retain(|id, _| world.contains(*id));
            retained.used_padding.retain(|id, _| world.contains(*id));
            retained.far_start.retain(|id, _| world.contains(*id));
            retained.container_plans.retain(|id, _| world.contains(*id));
            retained.measure_plans.retain(|id, _| world.contains(*id));
        }
        if retained.intrinsics.len() > universe.saturating_mul(2) {
            retained.intrinsics.retain(|id, _| world.contains(*id));
        }
        if retained.intrinsic_metrics.len() > universe.saturating_mul(2) {
            retained.intrinsic_metrics.retain(|key, _| {
                StableNodeId::new(key.content).is_some_and(|id| world.contains(id))
            });
        }
        #[cfg(feature = "benchmark")]
        phase.lap(3);
        if force_full && world.layout_source_reusable() {
            let snapshot = FullLayoutSnapshot {
                viewport_width: viewport.width.to_bits(),
                viewport_height: viewport.height.to_bits(),
                epoch: world.layout_source_epoch(),
                emitted: emitted.clone(),
                used_padding: retained.used_padding.clone(),
                far_start: retained.far_start.clone(),
            };
            retained.full_snapshots[1] = retained.full_snapshots[0].take();
            retained.full_snapshots[0] = Some(snapshot);
        }
        Ok(emitted)
    }

    /// Layout a style tree with the same algorithm as [`Self::layout_document`].
    ///
    /// Hidden / `display:none` nodes are omitted from the result (css-parity /
    /// Vue measure contract). Product `UiWorld` still records a zero box.
    ///
    /// Text leaves are measured by `shaper`, so a host passes the same one its
    /// frames flush with: a style tree measured by a second shaper would give
    /// boxes the painted text does not fit.
    pub fn layout_style_tree(
        self,
        root: &StyleLayoutNode,
        viewport: LayoutViewport,
        shaper: &mut impl crate::TextShaper,
    ) -> Vec<(String, LayoutBox)> {
        let document = DocumentId::new(1).expect("document 1 is nonzero");
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        let mut names = HashMap::new();
        let mut omitted = HashSet::new();
        let mut next = 1u64;
        fn add(
            node: &StyleLayoutNode,
            parent: Option<StableNodeId>,
            parent_omitted: bool,
            document: DocumentId,
            queue: &mut MutationQueue,
            names: &mut HashMap<StableNodeId, String>,
            omitted: &mut HashSet<StableNodeId>,
            next: &mut u64,
        ) -> StableNodeId {
            let id = StableNodeId::new(*next).expect("style-tree ids start at 1");
            *next += 1;
            queue.create(id, document, NodeKind::Element { tag: "div".into() });
            if let Some(parent) = parent {
                queue.insert(parent, id, None);
            }
            // `display:none` / hidden omit self and descendants. `display:contents`
            // omits only self from the name→box map; descendants still layout.
            let omit_descendants = parent_omitted || node.style.omits_box();
            if omit_descendants || !node.style.generates_box() {
                omitted.insert(id);
            }
            queue.set_style(
                id,
                NodeStyle {
                    layout: Arc::new(node.style.clone()),
                    ..NodeStyle::default()
                },
            );
            names.insert(id, node.id.clone());
            if let Some(text) = node.text.as_deref() {
                queue.set_text(id, crate::TextContent { value: text.into() });
            }
            for child in &node.children {
                add(
                    child,
                    Some(id),
                    omit_descendants,
                    document,
                    queue,
                    names,
                    omitted,
                    next,
                );
            }
            id
        }
        add(
            root,
            None,
            false,
            document,
            &mut queue,
            &mut names,
            &mut omitted,
            &mut next,
        );
        world
            .commit(queue)
            .expect("style-tree mutations are well-formed");
        let order = world.document_order(document);
        world
            .resolve_styles(&order)
            .expect("style-tree style resolve is infallible");
        world
            .shape_text(&order, shaper)
            .expect("style-tree text shaping is infallible");
        let layouts = self
            .layout_document(&world, document, viewport)
            .expect("style-tree layout is infallible");
        layouts
            .into_iter()
            .filter_map(|(id, box_)| {
                if omitted.contains(&id) {
                    return None;
                }
                names.get(&id).cloned().map(|name| (name, box_))
            })
            .collect()
    }
}

/// Cross-frame layout memo for scoped relayout: last published boxes, used-size
/// resolutions, and content-derived intrinsic facts. Each document owns its
/// entries, so a full pass cannot invalidate another window's layout.
#[derive(Default)]
pub struct RetainedLayoutCache {
    documents: HashMap<DocumentId, DocumentLayoutCache>,
}

impl RetainedLayoutCache {
    /// Cumulative intrinsic measurement work observed by scoped layout.  The
    /// snapshot is cumulative across documents and remains available to the
    /// frame driver until the next read/reset.
    pub fn intrinsic_cache_counters(&self) -> crate::IntrinsicCacheCounters {
        let mut counters = crate::IntrinsicCacheCounters::default();
        for document in self.documents.values() {
            let snapshot = document.intrinsic_counters;
            let entries = counters.entries.saturating_add(snapshot.entries);
            let bytes = counters.bytes.saturating_add(snapshot.bytes);
            counters.accumulate(snapshot);
            counters.entries = entries;
            counters.bytes = bytes;
        }
        counters
    }

    pub(crate) fn take_intrinsic_counters(&mut self) -> crate::IntrinsicCacheCounters {
        let mut counters = crate::IntrinsicCacheCounters::default();
        for document in self.documents.values_mut() {
            let snapshot = std::mem::take(&mut document.intrinsic_counters);
            let entries = counters.entries.saturating_add(snapshot.entries);
            let bytes = counters.bytes.saturating_add(snapshot.bytes);
            counters.accumulate(snapshot);
            counters.entries = entries;
            counters.bytes = bytes;
        }
        counters
    }

    /// Release all layout state owned by a closed document.
    pub fn remove_document(&mut self, document: DocumentId) {
        self.documents.remove(&document);
    }

    /// Release one deleted node without scanning cached nodes or constraints.
    pub fn remove_node(&mut self, document: DocumentId, id: StableNodeId) {
        if let Some(cache) = self.documents.get_mut(&document) {
            cache.intrinsics.remove(&id);
            cache
                .intrinsic_metrics
                .retain(|key, _| key.content != id.get());
            cache.boxes.remove(&id);
            cache.placements.remove(&id);
            cache.used_padding.remove(&id);
            cache.far_start.remove(&id);
            cache.container_plans.remove(&id);
            cache.measure_plans.remove(&id);
        }
    }

    pub(crate) fn execution_stats(&self, document: DocumentId) -> LayoutExecutionStats {
        self.documents
            .get(&document)
            .map(|cache| cache.execution_stats)
            .unwrap_or_default()
    }

    /// Structural counters from the most recent pass for `document`.
    pub(crate) fn frontier_stats(&self, document: DocumentId) -> LayoutFrontierStats {
        self.documents
            .get(&document)
            .map(|cache| cache.frontier_stats)
            .unwrap_or_default()
    }

    /// Which page axes the last placement of `id` laid its children out
    /// from the far (right / bottom) end: `[horizontal, vertical]`.
    pub(crate) fn far_start(&self, document: DocumentId, id: StableNodeId) -> Option<[bool; 2]> {
        self.documents.get(&document)?.far_start.get(&id).copied()
    }

    pub(crate) fn used_padding(
        &self,
        document: DocumentId,
        id: StableNodeId,
    ) -> Option<nana_ui_core::PaddingSpec> {
        self.documents
            .get(&document)?
            .used_padding
            .get(&id)
            .copied()
    }
}

/// Two used-size variants per node allow measure/place reuse without
/// accumulating a new entry for every pixel of an interactive resize. The
/// content-derived facts live in the generation-aware authority beside it.
#[derive(Default)]
struct RetainedIntrinsic {
    measurements: [Option<(MeasurementKey, Size)>; 2],
}

impl RetainedIntrinsic {
    fn get(&self, key: MeasurementKey) -> Option<Size> {
        self.measurements
            .iter()
            .flatten()
            .find(|(held, _)| *held == key)
            .map(|(_, size)| *size)
    }

    fn insert(&mut self, key: MeasurementKey, size: Size) {
        let next = Some((key, size));
        if self.measurements[0].is_some_and(|(held, _)| held == key) {
            self.measurements[0] = next;
            return;
        }
        self.measurements[1] = self.measurements[0];
        self.measurements[0] = next;
    }
}

#[derive(Default)]
struct DocumentLayoutCache {
    intrinsics: HashMap<StableNodeId, RetainedIntrinsic>,
    /// Content-derived intrinsic facts, independent from retained used sizes.
    intrinsic_metrics: HashMap<crate::IntrinsicCacheKey, crate::IntrinsicMetrics>,
    intrinsic_counters: crate::IntrinsicCacheCounters,
    boxes: HashMap<StableNodeId, LayoutBox>,
    materialized_inputs: usize,
    placements: HashMap<StableNodeId, (Point, Size, f32)>,
    pub(crate) used_padding: HashMap<StableNodeId, nana_ui_core::PaddingSpec>,
    /// Per container, the page axes placement starts at the far end.
    far_start: HashMap<StableNodeId, [bool; 2]>,
    /// Cached in-flow child placement per container. See [`ContainerPlan`].
    container_plans: HashMap<StableNodeId, ContainerPlan>,
    /// Cached intrinsic measurement per content-sized container. See
    /// [`MeasurePlan`].
    measure_plans: HashMap<StableNodeId, MeasurePlanSlots>,
    frontier_stats: LayoutFrontierStats,
    execution_stats: LayoutExecutionStats,
    /// Last two full-layout results for this document, keyed by viewport and
    /// layout-input epoch. `clear` keeps them: a full pass is what consults
    /// them, and clearing first would drop the hit.
    full_snapshots: [Option<FullLayoutSnapshot>; 2],
}

struct FullLayoutSnapshot {
    viewport_width: u32,
    viewport_height: u32,
    epoch: u64,
    emitted: Vec<(StableNodeId, LayoutBox)>,
    used_padding: HashMap<StableNodeId, nana_ui_core::PaddingSpec>,
    far_start: HashMap<StableNodeId, [bool; 2]>,
}

fn intrinsic_facts_changed(
    mut existing: crate::IntrinsicMetrics,
    mut incoming: crate::IntrinsicMetrics,
) -> bool {
    existing.generation = 0;
    incoming.generation = 0;
    existing != incoming
}

impl DocumentLayoutCache {
    fn record_intrinsic_counters(&mut self, counters: crate::IntrinsicCacheCounters) {
        self.intrinsic_counters.accumulate(counters);
    }

    fn retain_intrinsic_metrics(
        &mut self,
        metrics: HashMap<crate::IntrinsicCacheKey, crate::IntrinsicMetrics>,
        max_entries: usize,
    ) {
        let mut changed = HashSet::new();
        for (key, incoming) in &metrics {
            let differs = self
                .intrinsic_metrics
                .get(key)
                .is_none_or(|existing| intrinsic_facts_changed(*existing, *incoming));
            if differs {
                changed.insert(key.content);
            }
        }
        let mut next_generation = HashMap::new();
        if !changed.is_empty() {
            for (key, existing) in &self.intrinsic_metrics {
                if changed.contains(&key.content) {
                    let slot = next_generation.entry(key.content).or_insert(0);
                    *slot = (*slot).max(existing.generation);
                }
            }
            self.intrinsic_metrics
                .retain(|key, _| !changed.contains(&key.content));
        }
        for (key, mut incoming) in metrics {
            if !changed.contains(&key.content) {
                continue;
            }
            let previous = next_generation.get(&key.content).copied().unwrap_or(0);
            incoming.generation = previous.saturating_add(1).max(1);
            self.intrinsic_metrics.insert(key, incoming);
        }
        self.intrinsic_counters.generation_bumps = self
            .intrinsic_counters
            .generation_bumps
            .saturating_add(changed.len());
        // The per-pass authority enforces the full byte budget. The retained
        // mirror has a fixed-size value, so apply the same budget class here
        // instead of allowing every viewport/constraint variant to accumulate
        // forever across frames.
        let bytes_per_entry = std::mem::size_of::<crate::IntrinsicCacheKey>()
            .saturating_add(std::mem::size_of::<crate::IntrinsicMetrics>())
            .max(1);
        let byte_limited = crate::IntrinsicCacheBudget::default()
            .max_bytes
            .checked_div(bytes_per_entry)
            .unwrap_or(1)
            .max(1);
        let limit = max_entries.min(byte_limited).max(1);
        let mut evicted = 0usize;
        while self.intrinsic_metrics.len() > limit {
            let Some(key) = self.intrinsic_metrics.keys().next().copied() else {
                break;
            };
            self.intrinsic_metrics.remove(&key);
            evicted = evicted.saturating_add(1);
        }
        self.intrinsic_counters.evictions =
            self.intrinsic_counters.evictions.saturating_add(evicted);
    }

    /// Direct participants and tracks stored on this document's context plans.
    ///
    /// Mutation history does not add entries: a later pass replaces the plan
    /// for the same container. See [`bound_container_plan`].
    fn retained_plan_entries(&self) -> usize {
        let mut count = 0usize;
        for plan in self.container_plans.values() {
            count = count.saturating_add(plan.entries.borrow().len());
            count = count.saturating_add(plan.overlay.len());
            if let Some(grid) = &plan.grid {
                count = count.saturating_add(grid.items.len());
                count = count.saturating_add(grid.col_sizes.len());
                count = count.saturating_add(grid.row_sizes.len());
            }
        }
        for slots in self.measure_plans.values() {
            for plan in slots.slots.iter().flatten() {
                count = count.saturating_add(plan.entries.len());
                if let Some(grid) = &plan.grid {
                    count = count.saturating_add(grid.items.len());
                    count = count.saturating_add(grid.col_sizes.len());
                    count = count.saturating_add(grid.row_sizes.len());
                }
            }
        }
        count
    }

    fn clear(&mut self) {
        self.intrinsics.clear();
        self.intrinsic_metrics.clear();
        self.intrinsic_counters = crate::IntrinsicCacheCounters::default();
        self.placements.clear();
        self.boxes.clear();
        self.used_padding.clear();
        self.far_start.clear();
        self.container_plans.clear();
        self.measure_plans.clear();
        self.frontier_stats = LayoutFrontierStats::default();
        self.execution_stats = LayoutExecutionStats::default();
        self.materialized_inputs = 0;
    }
}

/// Test-only visibility into whether scoped layout is actually incremental.
///
/// The differential harness proves the result is CORRECT; these counters prove
/// it is cheap. Without them a "fix" that quietly relayouts every sibling still
/// passes every equivalence test.
#[cfg(any(test, feature = "benchmark"))]
#[allow(dead_code)]
pub mod plan_stats {
    use super::LayoutFrontier;
    use std::cell::Cell;

    thread_local! {
        static PLANS_REUSED: Cell<usize> = const { Cell::new(0) };
        static SUFFIXES_REPLAYED: Cell<usize> = const { Cell::new(0) };
        static MEASURE_PLANS_REUSED: Cell<usize> = const { Cell::new(0) };
        static CHILDREN_MEASURED: Cell<usize> = const { Cell::new(0) };
        static CONTAINERS_UNCACHEABLE: Cell<usize> = const { Cell::new(0) };
        static DIRTY_SEEDS: Cell<usize> = const { Cell::new(0) };
        static AFFECTED: Cell<usize> = const { Cell::new(0) };
        static RETAIN_SWEEPS: Cell<usize> = const { Cell::new(0) };
        static FRONTIER_SEEDS: Cell<usize> = const { Cell::new(0) };
        static FRONTIER_SEED_MERGES: Cell<usize> = const { Cell::new(0) };
        static FRONTIER_NODES_MEASURE: Cell<usize> = const { Cell::new(0) };
        static FRONTIER_NODES_PLACEMENT: Cell<usize> = const { Cell::new(0) };
        static FRONTIER_CONTEXTS: Cell<usize> = const { Cell::new(0) };
        static FRONTIER_EDGES: Cell<usize> = const { Cell::new(0) };
        static FRONTIER_STOPPED: Cell<usize> = const { Cell::new(0) };
        static FRONTIER_LOCAL_FALLBACKS: Cell<usize> = const { Cell::new(0) };
        static FRONTIER_FULL_FALLBACKS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn note_scope(dirty: usize, affected: usize) {
        DIRTY_SEEDS.with(|cell| cell.set(cell.get() + dirty));
        AFFECTED.with(|cell| cell.set(cell.get() + affected));
    }

    pub(crate) fn note_frontier(frontier: &LayoutFrontier, force_full: bool) {
        FRONTIER_SEEDS.with(|cell| cell.set(cell.get() + frontier.seeds()));
        FRONTIER_SEED_MERGES.with(|cell| cell.set(cell.get() + frontier.seed_merges()));
        FRONTIER_NODES_MEASURE.with(|cell| cell.set(cell.get() + frontier.measure_nodes().len()));
        FRONTIER_NODES_PLACEMENT
            .with(|cell| cell.set(cell.get() + frontier.placement_nodes().len()));
        FRONTIER_CONTEXTS.with(|cell| cell.set(cell.get() + frontier.context_nodes().len()));
        FRONTIER_EDGES.with(|cell| cell.set(cell.get() + frontier.dependency_edges_visited()));
        FRONTIER_STOPPED.with(|cell| cell.set(cell.get() + frontier.propagations_stopped()));
        FRONTIER_LOCAL_FALLBACKS
            .with(|cell| cell.set(cell.get() + frontier.local_subtree_fallbacks()));
        if force_full {
            // A deliberate viewport/full rebuild is not a correctness
            // fallback. Keep the fallback counter reserved for unsupported
            // contexts that escape the bounded frontier.
            return;
        }
        FRONTIER_FULL_FALLBACKS
            .with(|cell| cell.set(cell.get() + frontier.full_document_fallbacks()));
    }

    pub(crate) fn note_retain_sweep() {
        RETAIN_SWEEPS.with(|cell| cell.set(cell.get() + 1));
    }

    /// Nodes handed to `layout_document_with_frontier` as the change closure seed.
    #[cfg(feature = "benchmark")]
    pub fn dirty_seeds() -> usize {
        DIRTY_SEEDS.with(Cell::get)
    }

    /// Seeds plus their ancestors: what the pass actually walks.
    #[cfg(feature = "benchmark")]
    pub fn affected() -> usize {
        AFFECTED.with(Cell::get)
    }

    /// Times the retained caches were swept for despawned ids.
    #[cfg(feature = "benchmark")]
    pub fn retain_sweeps() -> usize {
        RETAIN_SWEEPS.with(Cell::get)
    }

    pub fn reset() {
        PLANS_REUSED.with(|cell| cell.set(0));
        SUFFIXES_REPLAYED.with(|cell| cell.set(0));
        MEASURE_PLANS_REUSED.with(|cell| cell.set(0));
        CHILDREN_MEASURED.with(|cell| cell.set(0));
        CONTAINERS_UNCACHEABLE.with(|cell| cell.set(0));
        DIRTY_SEEDS.with(|cell| cell.set(0));
        AFFECTED.with(|cell| cell.set(0));
        RETAIN_SWEEPS.with(|cell| cell.set(0));
        FRONTIER_SEEDS.with(|cell| cell.set(0));
        FRONTIER_SEED_MERGES.with(|cell| cell.set(0));
        FRONTIER_NODES_MEASURE.with(|cell| cell.set(0));
        FRONTIER_NODES_PLACEMENT.with(|cell| cell.set(0));
        FRONTIER_CONTEXTS.with(|cell| cell.set(0));
        FRONTIER_EDGES.with(|cell| cell.set(0));
        FRONTIER_STOPPED.with(|cell| cell.set(0));
        FRONTIER_LOCAL_FALLBACKS.with(|cell| cell.set(0));
        FRONTIER_FULL_FALLBACKS.with(|cell| cell.set(0));
    }

    pub(crate) fn note_plan_reused() {
        PLANS_REUSED.with(|cell| cell.set(cell.get() + 1));
    }

    /// A reuse that replayed a sequential container's suffix, rather than
    /// finding nothing changed.
    pub(crate) fn note_suffix_replayed() {
        SUFFIXES_REPLAYED.with(|cell| cell.set(cell.get() + 1));
    }

    /// Containers that kept their prefix and replayed only the children from
    /// the first changed one on.
    pub fn suffixes_replayed() -> usize {
        SUFFIXES_REPLAYED.with(Cell::get)
    }

    pub(crate) fn note_measure_plan_reused() {
        MEASURE_PLANS_REUSED.with(|cell| cell.set(cell.get() + 1));
    }

    /// Containers that returned a cached intrinsic size instead of re-measuring
    /// their children. See [`super::MeasurePlan`].
    pub fn measure_plans_reused() -> usize {
        MEASURE_PLANS_REUSED.with(Cell::get)
    }

    pub(crate) fn note_child_measured() {
        CHILDREN_MEASURED.with(|cell| cell.set(cell.get() + 1));
    }

    /// A container that took the placement path but could not be cached, so it
    /// will rescan its children on every future frame.
    pub(crate) fn note_container_uncacheable() {
        CONTAINERS_UNCACHEABLE.with(|cell| cell.set(cell.get() + 1));
    }

    /// The positioned formatting context could not be applied incrementally,
    /// so this pass laid out that context's positioned children and stopped
    /// there. This is not a document fallback.
    pub(crate) fn note_local_context_fallback() {
        FRONTIER_LOCAL_FALLBACKS.with(|cell| cell.set(cell.get() + 1));
    }

    #[cfg(feature = "benchmark")]
    pub fn containers_uncacheable() -> usize {
        CONTAINERS_UNCACHEABLE.with(Cell::get)
    }

    pub fn plans_reused() -> usize {
        PLANS_REUSED.with(Cell::get)
    }

    /// Children a container had to intrinsic-measure, counting BOTH sibling
    /// scans: the one in its placement loop and the one in its own intrinsic
    /// measurement. This is the scan that used to make every dirty frame O(N),
    /// and the measure-side half of it is invisible unless both are counted.
    pub fn children_measured() -> usize {
        CHILDREN_MEASURED.with(Cell::get)
    }

    pub fn frontier_seeds() -> usize {
        FRONTIER_SEEDS.with(Cell::get)
    }

    pub fn frontier_seed_merges() -> usize {
        FRONTIER_SEED_MERGES.with(Cell::get)
    }

    pub fn frontier_nodes_measure() -> usize {
        FRONTIER_NODES_MEASURE.with(Cell::get)
    }

    pub fn frontier_nodes_placement() -> usize {
        FRONTIER_NODES_PLACEMENT.with(Cell::get)
    }

    pub fn frontier_contexts() -> usize {
        FRONTIER_CONTEXTS.with(Cell::get)
    }

    pub fn dependency_edges_visited() -> usize {
        FRONTIER_EDGES.with(Cell::get)
    }

    pub fn propagations_stopped() -> usize {
        FRONTIER_STOPPED.with(Cell::get)
    }

    pub fn local_subtree_fallbacks() -> usize {
        FRONTIER_LOCAL_FALLBACKS.with(Cell::get)
    }

    pub fn full_document_fallbacks() -> usize {
        FRONTIER_FULL_FALLBACKS.with(Cell::get)
    }

    /// Coarse clocks for `--profile-layout`. Slots:
    /// prefetch, root measure, root place, engine tail,
    /// large-container child measure, large-container measure fold,
    /// large-container place remeasure, large-container place pack,
    /// result build, result store,
    /// writeback compare, view scan, commit, publish,
    /// plain-leaf content, plain-leaf baseline, plain-leaf metric record.
    const PHASES: usize = 17;

    thread_local! {
        static PHASE_NS: Cell<[u64; 17]> = const { Cell::new([0; 17]) };
    }

    pub(crate) fn add_phase(slot: usize, elapsed: std::time::Duration) {
        PHASE_NS.with(|cell| {
            let mut slots = cell.get();
            slots[slot] = slots[slot].saturating_add(elapsed.as_nanos() as u64);
            cell.set(slots);
        });
    }

    pub fn take_phases_ns() -> [u64; PHASES] {
        PHASE_NS.with(|cell| cell.replace([0; PHASES]))
    }

    pub(crate) struct PhaseClock {
        start: std::time::Instant,
    }

    impl PhaseClock {
        pub(crate) fn start() -> Self {
            Self {
                start: std::time::Instant::now(),
            }
        }

        pub(crate) fn lap(&mut self, slot: usize) {
            let now = std::time::Instant::now();
            add_phase(slot, now.saturating_duration_since(self.start));
            self.start = now;
        }
    }
}

/// One positioned child of a cached container. The list is the direct
/// participants of that positioned formatting context, replaced when the
/// container is recorded again.
#[derive(Clone)]
struct PlannedOverlay {
    child: StableNodeId,
    /// Retained style and used box for this participant. A later pass replaces
    /// the record; the fields are the cached placement, not a second authority.
    #[allow(dead_code)]
    style: Arc<nana_ui_core::LayoutStyle>,
    fixed: bool,
    /// The used box reads the containing block's size (percentage, fill,
    /// or both insets). A fixed child reads the viewport instead.
    tracks_containing_block: bool,
    #[allow(dead_code)]
    base: Size,
    #[allow(dead_code)]
    base_origin: Point,
    #[allow(dead_code)]
    origin: Point,
    #[allow(dead_code)]
    size: Size,
}

/// One child's contribution to a cached container placement.
#[derive(Clone)]
struct PlannedChild {
    child: StableNodeId,
    /// The child's own layout style at plan time, compared by pointer. This is
    /// what catches a style edit that moves a child without resizing it --
    /// `margin`, `align_self`, `order`, `flex_grow`.
    style: Arc<nana_ui_core::LayoutStyle>,
    /// Intrinsic size measured BEFORE flex distribution: the pure input the
    /// rest of the container's placement is a function of.
    intrinsic: Size,
    /// The main size the line gave this child when it differs from the
    /// intrinsic one, and the child measured at it: its cross size is then
    /// the line's input, and an edit can change it while the intrinsic stays.
    at_main: Option<(f32, Size)>,
    origin: Point,
    size: Size,
    /// Main-axis cursor before this child, i.e. the prefix sum of every
    /// preceding child's outer main extent plus gaps. Lets a suffix replay
    /// start at any index in O(1) instead of re-accumulating from zero.
    cursor_before: f32,
}

/// One grid item's occupied tracks and the intrinsic contribution those
/// tracks were solved from. Spans are the placement result, not a second
/// copy of the child's style.
#[derive(Clone)]
struct GridItemPlan {
    child: StableNodeId,
    col: u32,
    row: u32,
    col_span: u32,
    row_span: u32,
    contribution: Size,
}

/// Resolved grid tracks plus the contribution that produced them.
///
/// A later pass re-solves every track from these contributions. Items whose
/// contribution and cell constraint are unchanged are not measured again.
/// Subgrid, a child entering or leaving flow, and a container whose own
/// content-sized keywords this record cannot finish fall back to measuring
/// this grid, not the document.
#[derive(Clone)]
struct GridTrackPlan {
    items: Vec<GridItemPlan>,
    col_sizes: Vec<f32>,
    row_sizes: Vec<f32>,
    col_gap: f32,
    row_gap: f32,
}

impl GridTrackPlan {
    fn from_layout(grid: &Grid2DLayout) -> Self {
        Self {
            items: grid
                .items
                .iter()
                .map(|item| GridItemPlan {
                    child: item.id,
                    col: item.col as u32,
                    row: item.row as u32,
                    col_span: item.col_span as u32,
                    row_span: item.row_span as u32,
                    contribution: item.intrinsic,
                })
                .collect(),
            col_sizes: grid.col_sizes.clone(),
            row_sizes: grid.row_sizes.clone(),
            col_gap: grid.col_gap,
            row_gap: grid.row_gap,
        }
    }

    fn item(&self, child: StableNodeId) -> Option<&GridItemPlan> {
        self.items.iter().find(|item| item.child == child)
    }

    fn cell(&self, item: &GridItemPlan) -> (f32, f32) {
        (
            grid_span_extent(
                &self.col_sizes,
                item.col as usize,
                item.col_span as usize,
                self.col_gap,
            ),
            grid_span_extent(
                &self.row_sizes,
                item.row as usize,
                item.row_span as usize,
                self.row_gap,
            ),
        )
    }
}

fn grid_child_in_flow(style: &LayoutStyle) -> bool {
    !style.omits_box()
        && !style.position.is_out_of_flow()
        && !style.display.is_some_and(DisplaySpec::is_contents)
}

/// Both axes are a length that does not read the grid content box, so a
/// sibling-driven change of that box does not change this item's contribution.
fn grid_contribution_ignores_content_box(style: &LayoutStyle) -> bool {
    fn fixed(spec: Option<LengthSpec>) -> bool {
        matches!(
            spec,
            Some(
                LengthSpec::Px(_)
                    | LengthSpec::Em(_)
                    | LengthSpec::Rem(_)
                    | LengthSpec::CalcEmOffset { .. }
                    | LengthSpec::CalcRemOffset { .. }
            )
        )
    }
    style.aspect_ratio.is_none()
        && fixed(style.width)
        && fixed(style.height)
        && style.min_width.is_none_or(|spec| fixed(Some(spec)))
        && style.min_height.is_none_or(|spec| fixed(Some(spec)))
        && style.max_width.is_none_or(|spec| fixed(Some(spec)))
        && style.max_height.is_none_or(|spec| fixed(Some(spec)))
}

fn sort_ids_with_sizes(ids: &mut [StableNodeId], sizes: &mut [Size], nodes: &LayoutInputMap<'_>) {
    if ids.len() != sizes.len() {
        return;
    }
    let order_of = |id: StableNodeId| nodes.style(id).map(|style| style.order).unwrap_or(0);
    if ids.iter().copied().all(|id| order_of(id) == 0) {
        return;
    }
    let mut order: Vec<usize> = (0..ids.len()).collect();
    order.sort_by_key(|&index| order_of(ids[index]));
    let old_ids = ids.to_vec();
    let old_sizes = sizes.to_vec();
    for (slot, index) in order.into_iter().enumerate() {
        ids[slot] = old_ids[index];
        sizes[slot] = old_sizes[index];
    }
}

/// A container's placement of its in-flow children, cached across passes.
///
/// The whole point of scoped layout is to charge by the change, but a flex
/// container still had to walk every child to discover that none of them
/// moved: `subtree_unchanged` prunes a child's SUBTREE, not the parent's scan
/// of its siblings. So a one-row edit in an N-row list cost O(N).
///
/// The placement of in-flow children is a pure function of the container's own
/// inputs plus, in order, each child's layout style and intrinsic size. When
/// all of those are unchanged the previous result still holds, so the pass can
/// skip straight to the children the change closure actually reaches.
///
/// Only children in that closure need re-checking: a layout-affecting
/// `set_style` marks the node LAYOUT-dirty (`mark_subtree`), which is what puts
/// it in the closure. `UiWorld::children_layout_style_is_local` guards the
/// cases where an ancestor could move a child's style without touching it.
#[derive(Clone)]
struct ContainerPlan {
    origin: Point,
    size: Size,
    containing: Size,
    parent_font_px: f32,
    viewport: LayoutViewport,
    /// The container's effective style, compared by pointer.
    style: Arc<nana_ui_core::LayoutStyle>,
    /// The container's child list, compared by pointer. A structural edit
    /// copy-on-writes this `Arc` (the cache holds a reference, so the world's
    /// `Arc::make_mut` cannot mutate it in place), so a different pointer is a
    /// different list.
    children: Arc<Vec<StableNodeId>>,
    /// Containing block handed to each child.
    content: Size,
    child_font_px: f32,
    /// Available size each child's intrinsic measurement was taken against.
    child_available: Size,
    main_direction: FlexDirection,
    /// The writing mode and direction the container laid out in, inherited.
    /// An ancestor can change it without touching this container's own style,
    /// so the plan compares it with the other inputs.
    writing: nana_ui_core::WritingContext,
    /// The main and cross axes run from their far page edge — the right or
    /// the bottom. Placement is flow-relative (every cursor and margin is read
    /// from the start edge) and only turned onto the page where an origin is
    /// written; the replay turns it the same way.
    main_reversed: bool,
    cross_reversed: bool,
    /// Origin of the container's content box.
    content_origin: Point,
    /// Main-axis gap between children.
    gap: f32,
    /// True when this container placed its children as a plain accumulation
    /// from the main-start edge, so child `i`'s position depends only on the
    /// children before it — on a reversed axis too, where it is measured back
    /// from the far edge. Everything that would couple siblings is excluded:
    /// wrapping, a `justify-content` that distributes free space, grid tracks,
    /// auto main margins, baseline or center/end cross alignment, and any flex
    /// grow/shrink redistribution (detected from the data -- every child's
    /// used main size equalled its intrinsic).
    ///
    /// Under that shape a resized child shifts exactly the children after it,
    /// so the pass can keep the prefix and replay only the suffix.
    sequential: bool,
    /// In placement order.
    entries: RefCell<Vec<PlannedChild>>,
    /// `(child, index into entries)`, sorted by child, so the pass can ask
    /// "which of my children are in the change closure?" without walking every
    /// entry. Scanning the entries instead would leave the fast path O(number
    /// of children), which is the cost it exists to remove.
    by_child: Vec<(StableNodeId, u32)>,
    /// Occupied tracks, contributions, and the resolved track sizes. `None`
    /// on every flex and block plan.
    grid: Option<GridTrackPlan>,
    /// No in-flow child reads this container's content box on either axis,
    /// so a change of this container's own size refreshes positioned children
    /// only.
    cross_independent: bool,
    /// Positioned participants of this container. Empty on a flow-only plan.
    overlay: Vec<PlannedOverlay>,
}

impl ContainerPlan {
    /// Everything the container's own placement depends on, other than its
    /// children and its used border-box size. A mismatch here means the plan
    /// is about a different layout.
    #[allow(clippy::too_many_arguments)]
    fn flow_identity_holds(
        &self,
        origin: Point,
        containing: Size,
        parent_font_px: f32,
        viewport: LayoutViewport,
        style: &Arc<nana_ui_core::LayoutStyle>,
        children: &Arc<Vec<StableNodeId>>,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        self.writing == writing
            && self.origin == origin
            && self.containing_compatible(containing)
            && self.parent_font_px == parent_font_px
            && self.viewport == viewport
            && Arc::ptr_eq(&self.style, style)
            && Arc::ptr_eq(&self.children, children)
    }

    /// Everything the container's own placement depends on, other than its
    /// children. A mismatch here means the plan is about a different layout.
    #[allow(clippy::too_many_arguments)]
    fn inputs_match(
        &self,
        origin: Point,
        size: Size,
        containing: Size,
        parent_font_px: f32,
        viewport: LayoutViewport,
        style: &Arc<nana_ui_core::LayoutStyle>,
        children: &Arc<Vec<StableNodeId>>,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        self.flow_identity_holds(
            origin,
            containing,
            parent_font_px,
            viewport,
            style,
            children,
            writing,
        ) && self.size_compatible(size)
    }

    /// In-flow start edges stay put when this container's cross size changes
    /// and no child stretches to it. Positioned children that read the
    /// containing block are refreshed separately.
    fn flow_stable_under_own_size(&self, content_origin: Point) -> bool {
        self.cross_independent
            && self.sequential
            && self.grid.is_none()
            && !self.main_reversed
            && !self.cross_reversed
            && self.content_origin == content_origin
    }

    /// Flow reuse, including a containing-block size change that does not
    /// move in-flow children.
    #[allow(clippy::too_many_arguments)]
    fn can_reuse_flow(
        &self,
        origin: Point,
        size: Size,
        containing: Size,
        content_origin: Point,
        parent_font_px: f32,
        viewport: LayoutViewport,
        style: &Arc<nana_ui_core::LayoutStyle>,
        children: &Arc<Vec<StableNodeId>>,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        self.inputs_match(
            origin,
            size,
            containing,
            parent_font_px,
            viewport,
            style,
            children,
            writing,
        ) || (self.flow_identity_holds(
            origin,
            containing,
            parent_font_px,
            viewport,
            style,
            children,
            writing,
        ) && self.flow_stable_under_own_size(content_origin))
    }

    /// A sequential plan places from the start edge. Its main size can grow
    /// with a child without moving the cross axis or the prefix.
    fn size_compatible(&self, size: Size) -> bool {
        if self.size == size {
            return true;
        }
        // Track sizes are solved again from the recorded contributions, so
        // the used border box may grow with a row or a column. A pass whose
        // children did not change still re-solves when this size moved.
        if self.grid.is_some() {
            return true;
        }
        if self.sequential && !self.main_reversed {
            return match self.main_direction {
                FlexDirection::Column => self.size.width.to_bits() == size.width.to_bits(),
                FlexDirection::Row => self.size.height.to_bits() == size.height.to_bits(),
            };
        }
        // A wrap container's cross size is the sum of its line cross sizes.
        // The main size is the line budget; if that moved, line membership
        // has to be solved again by the formatting context.
        if !flex_line_local_style(self.style.as_ref()) || self.main_reversed || self.cross_reversed
        {
            return false;
        }
        match self.main_direction {
            FlexDirection::Row => {
                self.style.height.is_none() && self.size.width.to_bits() == size.width.to_bits()
            }
            FlexDirection::Column => {
                self.style.width.is_none() && self.size.height.to_bits() == size.height.to_bits()
            }
        }
    }

    /// The line budget is this container's main size. The parent's cross size
    /// can grow with this container without changing that budget.
    fn containing_compatible(&self, containing: Size) -> bool {
        if self.containing == containing {
            return true;
        }
        if self.grid.is_some() {
            return true;
        }
        if !flex_line_local_style(self.style.as_ref()) || self.main_reversed || self.cross_reversed
        {
            return false;
        }
        match self.main_direction {
            FlexDirection::Row => self.containing.width.to_bits() == containing.width.to_bits(),
            FlexDirection::Column => {
                self.containing.height.to_bits() == containing.height.to_bits()
            }
        }
    }

    fn child_count(&self) -> usize {
        self.by_child.len()
    }

    /// Every affected direct child still has the role this plan recorded:
    /// in flow, positioned, or omitted. A child that enters or leaves flow, or
    /// becomes positioned, changes the participant lists themselves, which no
    /// replay can patch. Driven from the closure, like [`Self::affected_entries`].
    fn flow_membership_holds(
        &self,
        container: StableNodeId,
        scope: &ScopeContext<'_>,
        nodes: &LayoutInputMap<'_>,
    ) -> bool {
        scope.affected.iter().all(|&child| {
            if nodes.world.parent_id(child) != Some(container) {
                return true;
            }
            let Some(style) = nodes.style(child) else {
                return false;
            };
            let was_in_flow = self
                .by_child
                .binary_search_by_key(&child, |(entry, _)| *entry)
                .is_ok();
            let was_positioned = self.overlay.iter().any(|entry| entry.child == child);
            if style.omits_box() {
                !was_in_flow && !was_positioned
            } else if style.position.is_out_of_flow() {
                was_positioned
            } else {
                was_in_flow
            }
        })
    }

    /// Entry indices for the children the change closure reaches, in placement
    /// order. Driven from the closure (small) rather than the child list.
    fn affected_entries(&self, scope: &ScopeContext<'_>) -> Vec<u32> {
        let mut indices: Vec<u32> = scope
            .affected
            .iter()
            .filter_map(|id| {
                self.by_child
                    .binary_search_by_key(id, |(child, _)| *child)
                    .ok()
                    .map(|slot| self.by_child[slot].1)
            })
            .collect();
        indices.sort_unstable();
        indices
    }
}

/// Whether two layout styles are the same INPUT to layout, as opposed to the
/// same spelling of one.
///
/// `direction` is the one field the engine never reads directly: every use goes
/// through [`used_flow_direction`], which is `unwrap_or(Column)`. So `None` and
/// `Some(Column)` are the same layout, and `Some(Row)` is not.
///
/// That distinction is not academic. Three writers disagree about how to spell
/// a default column: `MessageBridge::register` seeds `direction` from the
/// widget kind, the CSS cascade republishes a style that leaves it `None`, and
/// the Runtime's own `Stack` projection writes `Some(Column)` back. On a
/// 2,000-row Vue list all three run every pointer event, so a plain `==`
/// retires the cached plan on every frame while nothing about the layout has
/// changed. That was the whole of the measure plan's benefit: on that
/// benchmark, `==` left settle at 0.786 ms and the container re-measuring
/// 2,000 children per event; this comparison takes it to 0.630 ms and 5.4.
///
/// The equality is exact, not a tolerance: it accepts exactly the pairs that
/// `used_flow_direction` maps to the same axis, and every other field still has
/// to match outright.
fn layout_inputs_equal(a: &nana_ui_core::LayoutStyle, b: &nana_ui_core::LayoutStyle) -> bool {
    if a == b {
        return true;
    }
    if a.direction.unwrap_or(FlexDirection::Column) != b.direction.unwrap_or(FlexDirection::Column)
    {
        return false;
    }
    // Only reached when the styles differ, so the clone is off the hot path:
    // once per closure child that failed the cheap compare.
    let mut probe = a.clone();
    probe.direction = b.direction;
    probe == *b
}

/// Wrap flex whose line breaks depend only on each item's main size.
fn flex_line_local_style(style: &LayoutStyle) -> bool {
    style
        .display
        .is_some_and(|display| display.is_flex_container())
        && matches!(style.flex_wrap, FlexWrap::Wrap)
        && style.justify_content == JustifySpec::Start
        && style.align_items == AlignSpec::Start
        && style.align_content == JustifySpec::Start
        && style.aspect_ratio.is_none()
        && !style.flex_reverse
}

fn child_blocks_flex_line_local(style: &LayoutStyle) -> bool {
    style.flex_grow.unwrap_or(0.0) > 0.0
        || style.flex_shrink.unwrap_or(0.0) > 0.0
        || style.order != 0
        || style.aspect_ratio.is_some()
        || style.clear != ClearSpec::None
        || style
            .align_self
            .is_some_and(|align| align != AlignSpec::Start)
        || matches!(
            style.margin_left,
            Some(LengthSpec::Auto) | Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)
        )
        || matches!(
            style.margin_right,
            Some(LengthSpec::Auto) | Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)
        )
        || matches!(
            style.margin_top,
            Some(LengthSpec::Auto) | Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)
        )
        || matches!(
            style.margin_bottom,
            Some(LengthSpec::Auto) | Some(LengthSpec::Percent(_)) | Some(LengthSpec::Fill)
        )
}

/// One child's contribution to a cached container measurement.
#[derive(Clone)]
struct MeasuredChild {
    child: StableNodeId,
    /// The child's effective layout style at plan time, or `None` when the node
    /// was missing. Compared by pointer with a value fallback, for the same
    /// reason as in [`ContainerPlan`]: a host that rebuilds its style objects
    /// every frame hands back a fresh `Arc` holding an identical style.
    style: Option<Arc<nana_ui_core::LayoutStyle>>,
    /// The intrinsic size measured for this child, or `None` for a child the
    /// flow collection dropped (`display:none`, out of flow). A dropped child
    /// contributes nothing to the container's measurement, and it cannot start
    /// contributing without its own style changing -- which the style compare
    /// above catches.
    intrinsic: Option<Size>,
    /// The main size the line gave this child when it differs from the
    /// measured one, and the child measured at it (its cross size is the
    /// container's input then).
    at_main: Option<(f32, Size)>,
}

/// A container's own intrinsic measurement, cached across passes.
///
/// The measure-side twin of [`ContainerPlan`], and the same shape of hole.
/// [`measure::intrinsic_size_scoped`] short-circuits a node whose width and
/// height both resolve from its own style, which is what stops a dirty frame
/// from re-measuring the whole document. A CONTENT-SIZED container has no such
/// short circuit: its own size is a function of its children, so every affected
/// container re-measured every child -- each one only to hit the retained memo
/// and return the value it already had. Layout invalidation propagates to
/// ancestors, so a single edit puts every content-sized container above it on
/// that path, and the frame is O(number of children) again.
///
/// The measurement is a pure function of the container's own inputs (style,
/// child list, text metrics, available size, viewport, inherited font size,
/// parent flow direction) plus, per child, that child's layout style and its
/// intrinsic size under the recorded available size. When all of those are
/// unchanged the previous result still holds.
///
/// Only children the change closure reaches need re-checking, and the check is
/// driven FROM the closure: a container looks each affected id up in its own
/// sorted entries, rather than walking its children looking for affected ones.
/// Walking the children would leave the fast path O(number of children), which
/// is the cost this exists to remove.
///
/// Entries are sorted by child id rather than kept in flow order: the container
/// is either reusing the whole cached measurement or recomputing it from
/// scratch, and neither needs the order.
///
/// This is a cache of its own, NOT a relaxation of the
/// `retained.intrinsics.remove` in `layout_document_with_frontier`. That removal is
/// still right and still happens: an affected node's memo holds entries for
/// constraint combinations this frame will not measure, and those really are
/// stale. What a plan caches is the container's result under ONE recorded
/// constraint, re-validated against the closure before it is used, so the two
/// do not overlap.
#[derive(Clone)]
struct MeasurePlan {
    /// Constraint the container was measured against. This is the part of the
    /// per-pass cache key that the plan, keyed by id alone, has to carry.
    available: Size,
    parent_direction: Option<FlexDirection>,
    viewport: LayoutViewport,
    parent_font_px: f32,
    /// The container's effective style, compared by pointer with a value
    /// fallback.
    style: Arc<nana_ui_core::LayoutStyle>,
    /// The writing mode and direction the container measured in, inherited;
    /// see [`ContainerPlan::writing`].
    writing: nana_ui_core::WritingContext,
    /// The container's child list, compared by pointer. A structural edit
    /// copy-on-writes this `Arc`, so a different pointer is a different list.
    children: Arc<Vec<StableNodeId>>,
    /// The container's own shaped text, which competes with the children for
    /// the content size.
    text_metrics: Option<crate::TextMetrics>,
    /// That text's unwrapped width, when it wrapped narrower.
    text_natural_width: Option<f32>,
    /// Available size in-flow children were measured against. Flex uses one
    /// value for every child. A grid stores the content box here; each item's
    /// contribution lives on [`GridTrackPlan`], and a fill axis is applied
    /// only when that contribution is remeasured.
    child_available: Size,
    /// Flow direction handed to each child as its `parent_direction`.
    child_direction: FlexDirection,
    /// Sorted by child id.
    entries: Vec<MeasuredChild>,
    /// What the measurement produced.
    size: Size,
    /// Main size is the sum of child border boxes, margins, and gaps.
    sequential: bool,
    /// Present when this measurement is a grid track solution.
    grid: Option<GridTrackPlan>,
}

/// The measure plans retained for one container.
///
/// A container is commonly measured TWICE per pass under two different
/// constraints: once from its parent's own intrinsic measurement, against the
/// parent's available content box, and once from its parent's placement,
/// against the parent's USED content box. Under an auto-height ancestor those
/// two differ, so a single slot is written by one call and missed by the other,
/// every pass, forever. Two slots is the same answer -- and the same reason --
/// as [`RetainedIntrinsic`].
#[derive(Default)]
struct MeasurePlanSlots {
    slots: [Option<MeasurePlan>; 2],
}

impl MeasurePlanSlots {
    fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    /// Picks the slot recorded under this constraint. Selection only, not a
    /// correctness check: `MeasurePlan::inputs_match` compares `available`
    /// again, so handing back the wrong slot costs a recompute, never a stale
    /// answer.
    fn get(&self, available: Size) -> Option<&MeasurePlan> {
        self.slots
            .iter()
            .flatten()
            .find(|plan| plan.available == available)
    }

    fn insert(&mut self, plan: MeasurePlan) {
        if self.slots[0]
            .as_ref()
            .is_some_and(|held| held.available == plan.available)
        {
            self.slots[0] = Some(plan);
            return;
        }
        self.slots.swap(0, 1);
        self.slots[0] = Some(plan);
    }

    fn clear(&mut self) {
        self.slots = [None, None];
    }

    /// The plans this holder carries, most recent first.
    fn into_plans(self) -> impl Iterator<Item = MeasurePlan> {
        self.slots.into_iter().flatten()
    }
}

impl MeasurePlan {
    /// Everything the measurement depends on other than the children.
    #[allow(clippy::too_many_arguments)]
    fn inputs_match(
        &self,
        available: Size,
        parent_direction: Option<FlexDirection>,
        viewport: LayoutViewport,
        parent_font_px: f32,
        style: &Arc<nana_ui_core::LayoutStyle>,
        children: &Arc<Vec<StableNodeId>>,
        text_metrics: Option<crate::TextMetrics>,
        text_natural_width: Option<f32>,
        writing: nana_ui_core::WritingContext,
    ) -> bool {
        self.writing == writing
            && self.available == available
            && self.parent_direction == parent_direction
            && self.viewport == viewport
            && self.parent_font_px == parent_font_px
            && self.text_metrics == text_metrics
            && self.text_natural_width == text_natural_width
            && Arc::ptr_eq(&self.children, children)
            && (Arc::ptr_eq(&self.style, style) || layout_inputs_equal(&self.style, style))
    }

    fn entry(&self, child: StableNodeId) -> Option<&MeasuredChild> {
        self.entries
            .binary_search_by_key(&child, |entry| entry.child)
            .ok()
            .map(|slot| &self.entries[slot])
    }
}

/// On-demand `LayoutInput` cache. A miss loads exactly that id from `UiWorld`.
struct LayoutInputMap<'a> {
    world: &'a UiWorld,
    nodes: HashMap<StableNodeId, LayoutInput>,
    /// Effective styles for nodes this pass never materialized into `nodes`.
    ///
    /// A scoped pass prefetches nothing, so an unchanged sibling is reached
    /// only through [`Self::style`] -- and reached repeatedly: flex main-axis
    /// distribution, the baseline fold, and the cross-axis fold each ask for
    /// the same child's style. `UiWorld::effective_layout_style` is not a
    /// field read; it is several hash lookups plus an `Arc` clone, and a
    /// hidden or overlay-hosted node also pays an `Arc::make_mut` clone of the
    /// whole `LayoutStyle`. Resolving that once per node per pass is exact,
    /// not an approximation: `layout_document_with_frontier` borrows the world
    /// immutably for the entire pass, so no resolution can change underneath
    /// this map.
    styles: RefCell<HashMap<StableNodeId, Option<Arc<nana_ui_core::LayoutStyle>>>>,
    materialized: usize,
    placements: HashMap<StableNodeId, (Point, Size, f32)>,
    used_padding: HashMap<StableNodeId, nana_ui_core::PaddingSpec>,
    far_start: HashMap<StableNodeId, [bool; 2]>,
    /// Container plans rebuilt this pass. Merged into the retained cache at the
    /// end; containers that took the fast path record nothing, so their
    /// existing plan simply stays. `None` retires a plan recorded when the
    /// container was still on the cacheable path.
    container_plans: HashMap<StableNodeId, Option<ContainerPlan>>,
    /// Measure plans rebuilt this pass, merged the same way. An entry that
    /// ends the pass empty retires the container's retained plans.
    measure_plans: HashMap<StableNodeId, MeasurePlanSlots>,
}

#[derive(Debug, Clone, Copy, Default)]
struct BaselineMetrics {
    first: Option<f32>,
    last: Option<f32>,
}

impl<'a> LayoutInputMap<'a> {
    fn new(world: &'a UiWorld) -> Self {
        Self {
            world,
            nodes: HashMap::new(),
            styles: RefCell::new(HashMap::new()),
            materialized: 0,
            placements: HashMap::new(),
            used_padding: HashMap::new(),
            far_start: HashMap::new(),
            container_plans: HashMap::new(),
            measure_plans: HashMap::new(),
        }
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }

    fn prefetch(&mut self, ids: &[StableNodeId]) -> Result<(), UiWorldError> {
        // Full passes prefetch once, immediately after constructing this map.
        // Fill the final cache directly: materializing a temporary Vec and
        // then moving every input into a HashMap doubles the container work on
        // the cold path that large documents pay most often.
        debug_assert!(self.nodes.is_empty());
        if !ids.is_empty() {
            self.world.record_hot_path_allocation(
                1,
                ids.len().saturating_mul(std::mem::size_of::<LayoutInput>()),
            );
        }
        let mut nodes = HashMap::with_capacity(ids.len());
        for &id in ids {
            let input = self.world.layout_input(id)?;
            nodes.insert(id, input);
        }
        self.materialized = nodes.len();
        self.nodes = nodes;
        Ok(())
    }

    fn get(&mut self, id: StableNodeId) -> Result<Option<&LayoutInput>, UiWorldError> {
        match self.nodes.entry(id) {
            hashbrown::hash_map::Entry::Occupied(entry) => Ok(Some(entry.into_mut())),
            hashbrown::hash_map::Entry::Vacant(entry) => {
                let input = match self.world.layout_input(id) {
                    Ok(input) => input,
                    Err(UiWorldError::MissingNode(_)) => return Ok(None),
                    Err(error) => return Err(error),
                };
                self.materialized = self.materialized.saturating_add(1);
                Ok(Some(entry.insert(input)))
            }
        }
    }

    /// Style for classifying / measuring siblings without assembling `LayoutInput`.
    fn style(&self, id: StableNodeId) -> Option<Arc<nana_ui_core::LayoutStyle>> {
        if let Some(node) = self.nodes.get(&id) {
            return Some(Arc::clone(&node.style));
        }
        if let Some(cached) = self.styles.borrow().get(&id) {
            return cached.clone();
        }
        let resolved = self.world.layout_style(id);
        self.styles.borrow_mut().insert(id, resolved.clone());
        resolved
    }

    /// Shared baseline authority for all formatting contexts. Retained
    /// `nana-text` layouts provide first/last line baselines; host-shaped text
    /// falls back to its first-line ascent. Replaced/custom content has an
    /// explicit bottom-edge fallback so it never accidentally inherits text's
    /// approximate ascent.
    fn baseline_metrics(
        &self,
        id: StableNodeId,
        fallback_font_px: f32,
        inline_base: Option<f32>,
        used: Option<Size>,
    ) -> BaselineMetrics {
        let Some(style) = self.style(id) else {
            return BaselineMetrics::default();
        };
        let font = fonts_of(&style, fallback_font_px).element_px;
        let chrome_top =
            style.resolved_padding_against(inline_base).top + style.resolved_border_width();
        let replaced = self.world.custom_render(id).is_some()
            || style.paint.content_image.is_some()
            || style.paint.skipped_replaced.is_some()
            || {
                #[cfg(feature = "image-viewer")]
                {
                    matches!(
                        self.world.standard_visual_ref(id),
                        Some(crate::StandardVisual::ImageViewer { .. })
                    )
                }
                #[cfg(not(feature = "image-viewer"))]
                {
                    false
                }
            };
        if replaced {
            // Replaced content's baseline is its border-box bottom. The
            // measure caller supplies the used extent when it is available;
            // an absent extent remains an explicit fallback for a later query.
            let block = used.and_then(|used| {
                let (_, block) = self
                    .nodes
                    .get(&id)
                    .map(|node| node.writing.logical_size(used.width, used.height))?;
                Some(block.max(0.0))
            });
            return BaselineMetrics {
                first: block,
                last: block,
            };
        }
        if let Some((_, layout)) = self.world.text_layout(id)
            && !layout.is_vertical()
            && !layout.lines.is_empty()
        {
            let first = layout
                .lines
                .first()
                .map(|line| chrome_top + line.metrics.baseline_y_px);
            let last = layout
                .lines
                .last()
                .map(|line| chrome_top + line.metrics.baseline_y_px);
            return BaselineMetrics { first, last };
        }
        if matches!(
            self.world.standard_visual_ref(id),
            Some(crate::StandardVisual::Button { label, .. }) if label.is_empty()
        ) {
            let block = used.map(|used| {
                let writing = self
                    .nodes
                    .get(&id)
                    .map_or_else(Default::default, |node| node.writing);
                writing.logical_size(used.width, used.height).1.max(0.0)
            });
            return BaselineMetrics {
                first: block,
                last: block,
            };
        }
        let button_with_label = matches!(
            self.world.standard_visual_ref(id),
            Some(crate::StandardVisual::Button { label, .. }) if !label.is_empty()
        );
        let ascent = self
            .nodes
            .get(&id)
            .and_then(|node| node.text_metrics)
            .and_then(|metrics| metrics.ascent)
            .filter(|value| value.is_finite() && *value >= 0.0);
        let ascent = ascent.unwrap_or(font * nana_ui_core::TEXT_APPROX_ASCENT_EM);
        let first = Some(chrome_top + ascent);
        if button_with_label {
            // A labelled button's baseline belongs to its internal label
            // content. The shaped text metrics above are authoritative; the
            // approximation is only the same explicit host fallback used
            // when shaping has not produced a line yet.
            return BaselineMetrics { first, last: first };
        }
        BaselineMetrics { first, last: first }
    }

    fn baseline(&self, id: StableNodeId, fallback_font_px: f32, used: Size) -> f32 {
        let writing = self
            .nodes
            .get(&id)
            .map_or_else(Default::default, |node| node.writing);
        let (_, block_extent) = writing.logical_size(used.width, used.height);
        let replaced = self.world.custom_render(id).is_some()
            || self.style(id).is_some_and(|style| {
                style.paint.content_image.is_some() || style.paint.skipped_replaced.is_some()
            })
            || {
                #[cfg(feature = "image-viewer")]
                {
                    matches!(
                        self.world.standard_visual_ref(id),
                        Some(crate::StandardVisual::ImageViewer { .. })
                    )
                }
                #[cfg(not(feature = "image-viewer"))]
                {
                    false
                }
            };
        if replaced {
            // Replaced/custom nodes align to their bottom edge by default.
            return block_extent.max(0.0);
        }
        let metrics = self.baseline_metrics(
            id,
            fallback_font_px,
            Some(writing.inline_size(used.width, used.height)),
            Some(used),
        );
        // Read both ends here so callers share one retained result even when
        // the current parent only asks for first baseline alignment.
        let _last = metrics.last;
        metrics.first.unwrap_or(block_extent.max(0.0))
    }
}

struct ScopeContext<'a> {
    affected: &'a HashSet<StableNodeId>,
    measure: &'a HashSet<StableNodeId>,
    retained: &'a DocumentLayoutCache,
}

/// Prune a child recursion when the child is outside the affected closure and
/// its recomputed entry box is bit-identical to the retained one.
fn subtree_unchanged(
    child: StableNodeId,
    origin: Point,
    size: Size,
    containing: Size,
    child_style: &nana_ui_core::LayoutStyle,
    child_fonts: FontSizeContext,
    // The parent's: it is the child's containing block.
    writing: nana_ui_core::WritingContext,
    scope: Option<&ScopeContext<'_>>,
) -> bool {
    let Some(scope) = scope else {
        return false;
    };
    if scope.affected.contains(&child) {
        return false;
    }
    let Some(cached) = scope.retained.boxes.get(&child) else {
        return false;
    };
    let (relative_x, relative_y) = child_style.relative_offset_against_fonts(
        Some(containing.width),
        Some(containing.height),
        child_fonts,
    );
    scope.retained.used_padding.get(&child).copied()
        == Some(child_style.resolved_padding_against_fonts(
            Some(writing.inline_size(containing.width, containing.height)),
            child_fonts,
        ))
        && cached.x == origin.x + relative_x
        && cached.y == origin.y + relative_y
        && cached.width == size.width
        && cached.height == size.height
}

fn sort_by_order(ids: &mut [StableNodeId], nodes: &LayoutInputMap<'_>) {
    let order_of = |id: StableNodeId| nodes.style(id).map(|style| style.order).unwrap_or(0);
    // Every key costs a map lookup plus an `Arc` clone, so resolve each one at
    // most once. Siblings almost always keep the default order, and a stable
    // sort on all-equal keys is a no-op, so scan for that case and skip.
    if ids.iter().all(|id| order_of(*id) == 0) {
        return;
    }
    // `sort_by_key` re-evaluates the key on every comparison; `sort_by_cached_key`
    // is likewise stable but evaluates it once per element.
    ids.sort_by_cached_key(|id| order_of(*id));
}

fn uses_2d_grid(style: &LayoutStyle, flow: &[StableNodeId], nodes: &LayoutInputMap<'_>) -> bool {
    if !style.display.is_some_and(DisplaySpec::is_grid_container) {
        return false;
    }
    if style.active_grid_columns().is_some()
        || style.active_grid_rows().is_some()
        || style.grid_columns_repeat.is_some()
        || style.grid_rows_repeat.is_some()
        || style.is_subgrid_columns()
        || style.is_subgrid_rows()
        || style.grid_auto_flow.is_some()
        || style
            .grid_auto_columns
            .as_ref()
            .is_some_and(|tracks| !tracks.is_empty())
        || style
            .grid_auto_rows
            .as_ref()
            .is_some_and(|tracks| !tracks.is_empty())
        || style
            .grid_template_areas
            .as_ref()
            .is_some_and(|areas| !areas.cells.is_empty())
    {
        return true;
    }
    flow.iter().any(|id| {
        nodes
            .style(*id)
            .is_some_and(|child| !child.grid_placement.is_auto())
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Point {
    x: f32,
    y: f32,
}

impl Point {
    const ZERO: Self = Self { x: 0.0, y: 0.0 };
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Size {
    width: f32,
    height: f32,
}

/// All non-content inputs that can change a used measurement.  The containing
/// block size remains in the key, while font, viewport, writing mode, and
/// parent flow are carried alongside it so a compatible constraint cannot
/// accidentally reuse a result from another layout environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct MeasurementKey {
    id: StableNodeId,
    width: u32,
    height: u32,
    parent_direction: u8,
    viewport_width: u32,
    viewport_height: u32,
    parent_font: u32,
    writing: nana_ui_core::WritingContext,
    containing_writing: nana_ui_core::WritingContext,
    constraint: crate::ConstraintClass,
    direction_sensitive: bool,
}

impl MeasurementKey {
    fn new(
        id: StableNodeId,
        available: Size,
        parent_direction: Option<FlexDirection>,
        viewport: LayoutViewport,
        parent_font_px: f32,
        writing: nana_ui_core::WritingContext,
        containing_writing: nana_ui_core::WritingContext,
        constraint: crate::ConstraintClass,
        direction_sensitive: bool,
    ) -> Self {
        Self {
            id,
            width: available.width.to_bits(),
            height: available.height.to_bits(),
            parent_direction: match parent_direction {
                None => 0,
                Some(FlexDirection::Column) => 1,
                Some(FlexDirection::Row) => 2,
            },
            viewport_width: viewport.width.to_bits(),
            viewport_height: viewport.height.to_bits(),
            parent_font: parent_font_px.to_bits(),
            writing,
            containing_writing,
            constraint: constraint.normalized(),
            direction_sensitive,
        }
    }
}

fn measurement_constraint_class(
    style: &nana_ui_core::LayoutStyle,
    available: Size,
) -> crate::ConstraintClass {
    let extents = (available.width.max(0.0), available.height.max(0.0));
    if style
        .aspect_ratio
        .is_some_and(|ratio| ratio.is_finite() && ratio > 0.0)
    {
        crate::ConstraintClass::aspect_ratio(extents.0, extents.1)
    } else if style
        .width
        .is_some_and(nana_ui_core::LengthSpec::is_full_percent_fill)
        || style
            .height
            .is_some_and(nana_ui_core::LengthSpec::is_full_percent_fill)
    {
        crate::ConstraintClass::fill(extents.0, extents.1)
    } else {
        crate::ConstraintClass::percentage_cb(extents.0, extents.1)
    }
}

/// Work a layout pass ran. Frontier membership is recorded separately.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LayoutExecutionStats {
    pub measure_nodes: usize,
    pub measure_cache_hits: usize,
    pub measure_cache_misses: usize,
    pub placement_nodes: usize,
    pub origin_only_updates: usize,
}

/// Per-pass used-size memo.  The public [`crate::IntrinsicCache`] owns the
/// generation-aware intrinsic contract; this tiny adapter keeps the existing
/// layout algorithm's physical-size representation while exposing the same
/// hit/miss accounting.  Its key deliberately has no formatting-context id,
/// so a child measured by flex/grid/inline can share the result in a pass.
#[derive(Default)]
struct PassIntrinsicCache {
    /// Intrinsic facts are kept separately from used sizes.  A percentage,
    /// fill, or stretch result is a resolution against one containing block;
    /// it must never become the preferred value in the shared authority.
    cache: crate::IntrinsicCache,
    /// Used-size memo for this pass, and the writeback list merged into the
    /// retained cache at the end. Its key contains the concrete available
    /// size because that is exactly what the placement algorithm resolves.
    used: HashMap<MeasurementKey, Size>,
    new_metrics: HashMap<crate::IntrinsicCacheKey, crate::IntrinsicMetrics>,
    latest_intrinsic_keys: HashMap<StableNodeId, crate::IntrinsicCacheKey>,
    seeded_intrinsic: HashSet<crate::IntrinsicCacheKey>,
    extra_counters: crate::IntrinsicCacheCounters,
    execution_stats: LayoutExecutionStats,
}

impl PassIntrinsicCache {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            cache: crate::IntrinsicCache::new(crate::IntrinsicCacheBudget {
                // A node may be measured at its parent width and again at a
                // flex/grid used width. Keep a few constraint classes alive
                // for baseline and cross-context consumers in this pass.
                max_entries: capacity.saturating_mul(4).max(1),
                max_bytes: usize::MAX,
            }),
            used: HashMap::with_capacity(capacity),
            new_metrics: HashMap::with_capacity(capacity),
            latest_intrinsic_keys: HashMap::with_capacity(capacity),
            seeded_intrinsic: HashSet::with_capacity(capacity),
            extra_counters: crate::IntrinsicCacheCounters::default(),
            execution_stats: LayoutExecutionStats::default(),
        }
    }

    fn note_measure_cache_hit(&mut self) {
        self.execution_stats.measure_cache_hits =
            self.execution_stats.measure_cache_hits.saturating_add(1);
    }

    fn note_measure_cache_miss(&mut self) {
        self.execution_stats.measure_cache_misses =
            self.execution_stats.measure_cache_misses.saturating_add(1);
    }

    fn note_measure_node(&mut self) {
        self.execution_stats.measure_nodes = self.execution_stats.measure_nodes.saturating_add(1);
    }

    fn note_placement_node(&mut self) {
        self.execution_stats.placement_nodes =
            self.execution_stats.placement_nodes.saturating_add(1);
    }

    fn note_origin_only(&mut self) {
        self.execution_stats.origin_only_updates =
            self.execution_stats.origin_only_updates.saturating_add(1);
    }

    fn get(&mut self, key: &MeasurementKey) -> Option<Size> {
        // A hit answers the query. Intrinsic accounting stays on the miss
        // path; this memo must not increment those counters.
        self.used.get(key).copied()
    }

    fn insert(&mut self, key: MeasurementKey, size: Size) {
        self.used.insert(key, size);
    }

    /// Publish content-derived facts. `preferred` is the natural border-box
    /// result before resolving the current containing block. Callers pass the
    /// final used value separately to [`Self::insert`].
    fn insert_intrinsic_bounds(
        &mut self,
        key: MeasurementKey,
        min_inline: f32,
        max_inline: f32,
        min_block: f32,
        max_block: Option<f32>,
        preferred: Size,
        first_baseline: Option<f32>,
        last_baseline: Option<f32>,
        aspect_ratio: Option<f32>,
    ) {
        let cache_key = Self::intrinsic_key(key);
        let metrics = crate::IntrinsicMetrics::new(
            min_inline,
            max_inline,
            min_block,
            max_block,
            crate::UsedSize::new(preferred.width, preferred.height),
        )
        .with_baselines(first_baseline, last_baseline)
        .with_aspect_ratio(aspect_ratio);
        let context = match key.parent_direction {
            1 => Some(crate::FormattingContextId::new(1)),
            2 => Some(crate::FormattingContextId::new(2)),
            _ => None,
        };
        self.seeded_intrinsic.insert(cache_key);
        self.cache.insert(cache_key, metrics, context);
        self.new_metrics.insert(cache_key, metrics);
        self.latest_intrinsic_keys.insert(key.id, cache_key);
    }

    /// Record facts for the retained cache without the in-pass LRU.
    ///
    /// Placement reads a baseline from that LRU only for `align-items:
    /// baseline`; every other alignment recomputes from the node. A plain
    /// leaf still publishes the same metrics the general insert would have
    /// written to `new_metrics`.
    fn remember_intrinsic_bounds(
        &mut self,
        key: MeasurementKey,
        min_inline: f32,
        max_inline: f32,
        min_block: f32,
        max_block: Option<f32>,
        preferred: Size,
        first_baseline: Option<f32>,
        last_baseline: Option<f32>,
        aspect_ratio: Option<f32>,
    ) {
        let cache_key = Self::intrinsic_key(key);
        let metrics = crate::IntrinsicMetrics::new(
            min_inline,
            max_inline,
            min_block,
            max_block,
            crate::UsedSize::new(preferred.width, preferred.height),
        )
        .with_baselines(first_baseline, last_baseline)
        .with_aspect_ratio(aspect_ratio);
        self.new_metrics.insert(cache_key, metrics);
    }

    fn seed_intrinsic(
        &mut self,
        key: crate::IntrinsicCacheKey,
        metrics: crate::IntrinsicMetrics,
        context: Option<crate::FormattingContextId>,
    ) {
        if !self.seeded_intrinsic.insert(key) {
            if let Some(id) = StableNodeId::new(key.content) {
                self.latest_intrinsic_keys.insert(id, key);
            }
            return;
        }
        self.cache.insert(key, metrics, context);
        if let Some(id) = StableNodeId::new(key.content) {
            self.latest_intrinsic_keys.insert(id, key);
        }
    }

    fn get_intrinsic(
        &mut self,
        key: MeasurementKey,
        context: Option<crate::FormattingContextId>,
    ) -> Option<crate::IntrinsicMetrics> {
        let key = Self::intrinsic_key(key);
        self.cache.get(&key, context)
    }

    fn record_full_subtree(&mut self) {
        self.extra_counters.full_subtrees = self.extra_counters.full_subtrees.saturating_add(1);
        self.extra_counters.intrinsic_measure_full_subtrees = self
            .extra_counters
            .intrinsic_measure_full_subtrees
            .saturating_add(1);
    }

    fn baseline(&mut self, id: StableNodeId, which: crate::Baseline) -> Option<f32> {
        let Some(key) = self.latest_intrinsic_keys.get(&id).copied() else {
            self.extra_counters.baseline_queries =
                self.extra_counters.baseline_queries.saturating_add(1);
            return None;
        };
        self.cache.baseline(&key, None, which)
    }

    fn counters(&self) -> crate::IntrinsicCacheCounters {
        let mut counters = self.cache.counters();
        let mut extra = self.extra_counters;
        extra.entries = counters.entries;
        extra.bytes = counters.bytes;
        counters.accumulate(extra);
        counters
    }

    fn intrinsic_key(key: MeasurementKey) -> crate::IntrinsicCacheKey {
        // Natural contributions for wrapping and percentage descendants can
        // depend on the containing block. Keep that relevant basis in the
        // constraint class and carry the remaining environment in the style
        // identity; the formatting-context name itself is still absent.
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        // Parent flow direction selects the *used-size* resolution path, but
        // it is not part of ordinary intrinsic facts. Aspect-ratio stretch
        // transfer is the deliberate exception: its used width depends on
        // whether the parent is a row, so keep that relevant dependency in
        // the identity while all other content still crosses contexts.
        if key.direction_sensitive
            || matches!(key.constraint, crate::ConstraintClass::AspectRatio { .. })
        {
            key.parent_direction.hash(&mut hasher);
        }
        key.viewport_width.hash(&mut hasher);
        key.viewport_height.hash(&mut hasher);
        key.parent_font.hash(&mut hasher);
        key.writing.hash(&mut hasher);
        key.containing_writing.hash(&mut hasher);
        crate::IntrinsicCacheKey::new(key.id.get(), hasher.finish(), key.constraint)
    }
}

impl Size {
    fn new(width: f32, height: f32) -> Self {
        Self {
            width: finite_extent(width),
            height: finite_extent(height),
        }
    }
}

fn gap_containing_block(style: &LayoutStyle, content: Size) -> nana_ui_core::ParentBox {
    // Auto-height wrap: row-gap % falls back to width (T-W05/W06). A Fill/px
    // height is a definite CB and must not use the parent/viewport leftover.
    let height = match style.height {
        None
        | Some(LengthSpec::Auto)
        | Some(LengthSpec::Shrink)
        | Some(LengthSpec::MinContent)
        | Some(LengthSpec::MaxContent)
        | Some(LengthSpec::FitContent) => None,
        Some(LengthSpec::Fill) => Some(content.height).filter(|value| *value > 0.0),
        Some(_) => Some(content.height).filter(|value| *value > 0.0),
    };
    nana_ui_core::ParentBox::new(Some(content.width).filter(|value| *value > 0.0), height)
}

/// Whether placement lays the container's main and cross axes out from their
/// far page end — the right or the bottom: an RTL inline axis (the right, or
/// the bottom of a vertical one), `vertical-rl`'s block axis from the right,
/// or `flex-direction: *-reverse`. A 2D grid places on the page as authored.
fn flow_axes_reversed(
    style: &LayoutStyle,
    writing: nana_ui_core::WritingContext,
    direction: FlexDirection,
    ifc: bool,
    grid_2d: bool,
) -> (bool, bool) {
    if grid_2d {
        return (false, false);
    }
    let cross = if direction.is_row() {
        FlexDirection::Column
    } else {
        FlexDirection::Row
    };
    let main = if ifc {
        writing.physical_axis_reversed(direction)
    } else {
        style.flex_reverse != writing.physical_axis_reversed(direction)
    };
    (main, writing.physical_axis_reversed(cross))
}

/// Which page axes of container `id` start at their far end: `[horizontal,
/// vertical]`, true where the content starts at the right / bottom and
/// overflows toward the left / top. The scroll origin sits on that start edge.
///
/// Placement records it as it lays the children out, so this is what the
/// boxes actually did. Before a first layout it is read off the container's
/// own style, which differs only for a reversed non-flex container that turns
/// out to be an inline formatting context (whose lines ignore `*-reverse`).
pub(crate) fn far_start_axes(world: &UiWorld, id: StableNodeId) -> [bool; 2] {
    if let Some(placed) = world.layout_far_start(id) {
        return placed;
    }
    let Some(style) = world.layout_style(id) else {
        return [false; 2];
    };
    let writing = world.layout_writing(id);
    let grid = style.display.is_some_and(DisplaySpec::is_grid_container);
    let direction = used_flow_direction(&style, writing, false);
    let reversed = flow_axes_reversed(&style, writing, direction, false, grid);
    page_far_start(writing, direction, grid, reversed)
}

/// The page axes `[horizontal, vertical]` a placement starting its main /
/// cross axes at their far ends (`reversed`) turns into. A grid turns its
/// tracks onto the page by the writing context alone.
fn page_far_start(
    writing: nana_ui_core::WritingContext,
    direction: FlexDirection,
    grid_2d: bool,
    (main, cross): (bool, bool),
) -> [bool; 2] {
    if grid_2d {
        [
            writing.physical_axis_reversed(FlexDirection::Row),
            writing.physical_axis_reversed(FlexDirection::Column),
        ]
    } else if direction.is_row() {
        [main, cross]
    } else {
        [cross, main]
    }
}

/// Physical main axis for this formatting context.
///
/// IFC always follows the writing-mode inline axis. Flex `row`/`column` are
/// remapped through writing-mode; block containers without an explicit
/// `flex-direction` stack along the block axis.
fn used_flow_direction(
    style: &LayoutStyle,
    context: nana_ui_core::WritingContext,
    ifc: bool,
) -> FlexDirection {
    if ifc {
        return context.inline_flex_direction();
    }
    let css = style.direction.unwrap_or(FlexDirection::Column);
    context.physical_flex_direction(css)
}

/// `text-align` as the flow-relative justification of an inline formatting
/// context's line: `start` / `end` are the inline axis's own, and the physical
/// `left` / `right` land on whichever end of it the page puts there — the end
/// of an RTL line for `left`, its start for `right`.
fn ifc_justify(align: TextAlignSpec, context: nana_ui_core::WritingContext) -> JustifySpec {
    let reversed = context.inline_reversed();
    let physical = align.to_justify(reversed);
    if reversed {
        flip_justify_for_reverse(physical)
    } else {
        physical
    }
}

fn flip_justify_for_reverse(justify: JustifySpec) -> JustifySpec {
    match justify {
        JustifySpec::Start => JustifySpec::End,
        JustifySpec::End => JustifySpec::Start,
        other => other,
    }
}

fn demote_fill_spec(spec: Option<LengthSpec>) -> Option<LengthSpec> {
    match spec {
        Some(s) if s.is_full_percent_fill() => None,
        other => other,
    }
}

/// `100%` / `Fill` against a definite grid CB must not become the auto-track
/// contribution. Measure that axis as indefinite (same as auto tracks).
fn grid_item_measure_available(style: &LayoutStyle, content: Size) -> Size {
    let width = if style.width.is_some() && demote_fill_spec(style.width).is_none() {
        0.0
    } else {
        content.width
    };
    let height = if style.height.is_some() && demote_fill_spec(style.height).is_none() {
        0.0
    } else {
        content.height
    };
    Size::new(width, height)
}

#[allow(clippy::too_many_arguments)]
fn packing_main_size(
    style: &LayoutStyle,
    intrinsic: Size,
    direction: FlexDirection,
    content_main: f32,
    // What percentage paddings resolve against: the containing block's inline
    // size, which is `content_main` only for a row in `horizontal-tb`.
    edge_percent_base: f32,
    viewport: LayoutViewport,
    parent_font_px: f32,
    track: Option<GridTrack>,
) -> f32 {
    // A line takes an item by the size it has before it grows (CSS Flexbox
    // §9.3, the hypothetical main size): its basis, else its main size, else
    // its content, within its min and max. Growth is shared out once the
    // line is known (`distribute_flex_main`), so a growing item no longer
    // claims a whole line and pushes siblings that fit beside it onto the
    // next.
    let spec = if style.grows() {
        style
            .flex_basis
            .filter(|basis| !matches!(basis, LengthSpec::Auto))
            .or(match direction {
                FlexDirection::Row => style.width,
                FlexDirection::Column => style.height,
            })
    } else {
        style.child_main_length(direction)
    }
    .or_else(|| track.map(GridTrack::as_row_main_length));
    let fonts = fonts_of(style, parent_font_px);
    let vp = Some((viewport.width, viewport.height));
    let (min, max) = match direction {
        FlexDirection::Row => (
            style.resolved_min_width_fonts(Some(content_main), vp, fonts),
            style.resolved_max_width_fonts(Some(content_main), vp, fonts),
        ),
        FlexDirection::Column => (
            style.resolved_min_height_fonts(Some(content_main), vp, fonts),
            style.resolved_max_height_fonts(Some(content_main), vp, fonts),
        ),
    };
    let clamp = |value: f32| {
        let value = value.max(min);
        max.map_or(value, |max| value.min(max))
    };
    match resolve_child_main(spec, content_main, viewport, fonts) {
        Some(value) => content_box_main_border_size(
            style,
            direction,
            Some(edge_percent_base),
            clamp(value),
            fonts,
        ),
        None if matches!(spec, Some(LengthSpec::Fill)) => content_main,
        None => clamp(main_extent(intrinsic, direction)),
    }
}

/// CSS initial `medium` ≈ 16px. Root `rem` and the em base when no ancestor
/// set `font-size`.
const ROOT_FONT_PX: f32 = nana_ui_core::type_scale::LINE;

fn fonts_of(style: &LayoutStyle, parent_font_px: f32) -> FontSizeContext {
    FontSizeContext::new(ROOT_FONT_PX, style.font_size.unwrap_or(parent_font_px))
}

fn resolve_child_main(
    spec: Option<LengthSpec>,
    percent_base: f32,
    viewport: LayoutViewport,
    fonts: FontSizeContext,
) -> Option<f32> {
    match spec {
        None
        | Some(LengthSpec::Fill)
        | Some(LengthSpec::Shrink)
        | Some(LengthSpec::Auto)
        | Some(LengthSpec::MinContent)
        | Some(LengthSpec::MaxContent)
        | Some(LengthSpec::FitContent) => None,
        Some(other) => other
            .resolve_with_fonts(
                Some(percent_base),
                Some((viewport.width, viewport.height)),
                fonts,
            )
            .map(|value| value.max(0.0)),
    }
}

fn content_box_main_border_size(
    style: &LayoutStyle,
    direction: FlexDirection,
    margin_percent_base: Option<f32>,
    content_main: f32,
    fonts: FontSizeContext,
) -> f32 {
    if !matches!(style.box_sizing, BoxSizing::ContentBox) {
        return content_main;
    }
    let pad = style.resolved_padding_against_fonts(margin_percent_base, fonts);
    let border = style.resolved_border_edges();
    content_main
        + match direction {
            FlexDirection::Row => pad.left + pad.right + border.left + border.right,
            FlexDirection::Column => pad.top + pad.bottom + border.top + border.bottom,
        }
}

fn resolve_axis(
    spec: Option<LengthSpec>,
    percent_base: f32,
    fill_base: f32,
    viewport: LayoutViewport,
    fonts: FontSizeContext,
) -> Option<f32> {
    spec.and_then(|value| {
        if value == LengthSpec::Fill {
            Some(fill_base)
        } else {
            value
                .resolve_with_fonts(
                    Some(percent_base),
                    Some((viewport.width, viewport.height)),
                    fonts,
                )
                .map(|value| value.max(0.0))
        }
    })
}

fn demote_fill_spec_if_indefinite(spec: Option<LengthSpec>, base: f32) -> Option<LengthSpec> {
    if base > 0.5 {
        spec
    } else {
        demote_fill_spec(spec)
    }
}

fn aspect_ratio_is_usable(style: &nana_ui_core::LayoutStyle) -> bool {
    style.aspect_ratio.is_some_and(|r| r.is_finite() && r > 0.0)
}

/// After stretch (or a flexed used width), fill `height:auto` from the used width.
fn fill_auto_height_from_aspect_ratio(
    style: &nana_ui_core::LayoutStyle,
    size: &mut Size,
    percent_base: Option<f32>,
    fonts: FontSizeContext,
) {
    if !aspect_ratio_is_usable(style) || style.height.is_some() {
        return;
    }
    let padding = style.resolved_padding_against_fonts(percent_base, fonts);
    let border = style.resolved_border_edges();
    let chrome_w = padding.left + padding.right + border.left + border.right;
    let chrome_h = padding.top + padding.bottom + border.top + border.bottom;
    let mut content_w = Some((size.width - chrome_w).max(0.0));
    let mut content_h = None;
    style.apply_aspect_ratio_used(&mut content_w, &mut content_h);
    if let Some(h) = content_h {
        size.height = h + chrome_h;
    }
}

/// Whether the item's cross size is its own, so `align-items: stretch`
/// leaves it: any size it declares but `auto`, which stretches as unset
/// does (CSS). A content-sized keyword (`Shrink`, `fit-content`) keeps the
/// content's size.
fn cross_axis_is_definite(style: &nana_ui_core::LayoutStyle, direction: FlexDirection) -> bool {
    let declared = |spec: Option<LengthSpec>| spec.is_some_and(|spec| spec != LengthSpec::Auto);
    match direction {
        // Transferred block size from a definite used width + `aspect-ratio`.
        FlexDirection::Row => declared(style.height) || aspect_ratio_is_usable(style),
        FlexDirection::Column => declared(style.width),
    }
}

fn main_extent(size: Size, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => size.width,
        FlexDirection::Column => size.height,
    }
}

fn cross_extent(size: Size, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => size.height,
        FlexDirection::Column => size.width,
    }
}

fn set_main_extent(size: &mut Size, direction: FlexDirection, value: f32) {
    match direction {
        FlexDirection::Row => size.width = finite_extent(value),
        FlexDirection::Column => size.height = finite_extent(value),
    }
}

fn set_cross_extent(size: &mut Size, direction: FlexDirection, value: f32) {
    match direction {
        FlexDirection::Row => size.height = finite_extent(value),
        FlexDirection::Column => size.width = finite_extent(value),
    }
}

fn main_start_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => margin.left,
        FlexDirection::Column => margin.top,
    }
}

fn main_end_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => margin.right,
        FlexDirection::Column => margin.bottom,
    }
}

fn cross_start_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => margin.top,
        FlexDirection::Column => margin.left,
    }
}

fn cross_end_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    match direction {
        FlexDirection::Row => margin.bottom,
        FlexDirection::Column => margin.right,
    }
}

fn cross_margin(margin: nana_ui_core::PaddingSpec, direction: FlexDirection) -> f32 {
    cross_start_margin(margin, direction) + cross_end_margin(margin, direction)
}

/// Drop a repeated child row. Plans store one entry per direct participant;
/// a later record for the same child replaces the stale one.
fn dedupe_plan_rows<T: Clone>(rows: &mut Vec<T>, child_of: impl Fn(&T) -> StableNodeId) {
    let mut seen = HashSet::with_capacity(rows.len());
    if rows.iter().all(|row| seen.insert(child_of(row))) {
        return;
    }
    seen.clear();
    let mut kept = Vec::with_capacity(rows.len());
    for row in rows.drain(..) {
        if seen.insert(child_of(&row)) {
            kept.push(row);
        }
    }
    *rows = kept;
}

fn bound_container_plan(plan: &mut ContainerPlan) {
    dedupe_plan_rows(&mut plan.entries.borrow_mut(), |entry| entry.child);
    dedupe_plan_rows(&mut plan.overlay, |entry| entry.child);
    if let Some(grid) = plan.grid.as_mut() {
        dedupe_plan_rows(&mut grid.items, |item| item.child);
    }
}

fn bound_measure_plan(plan: &mut MeasurePlan) {
    dedupe_plan_rows(&mut plan.entries, |entry| entry.child);
    if let Some(grid) = plan.grid.as_mut() {
        dedupe_plan_rows(&mut grid.items, |item| item.child);
    }
}

fn finite_extent(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod issue258;
#[cfg(test)]
mod tests;
