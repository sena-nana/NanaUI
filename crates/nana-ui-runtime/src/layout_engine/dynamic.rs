//! Dynamic Layout in the engine (Issues #209, #212, #213).
//!
//! A container whose [`AdaptationProfile::solve_overflow`] is set, laying out
//! a single line that does not fit, asks its auto-sized children what they
//! can give up ([`DynamicPass::envelope`]) and takes the cheapest first
//! ([`solve_deficit`]). A container that does not solve never reads a
//! child's declaration, so declaring elasticity costs nothing until some
//! container asks. Each child the line shrank gets an assignment; when it is
//! placed it resolves that inside itself ([`DynamicPass::resolve_padding`]):
//! its padding closes up, and whatever the assignment leaves for its content
//! reaches its own line as overflow, which that line solves the same way.
//!
//! Envelopes and line solves are kept across passes. An envelope is a
//! function of the box's style and its children's envelopes, never of the
//! size the box is offered, so a resize finds every envelope it needs. A
//! line solve keeps where it stopped ([`SolveFrontier`]); a resize whose
//! deficit stays in that level only re-shares it, which is the last step of
//! the cold solve, so incremental and cold answers are the same.

use nana_ui_core::DynamicLayoutCounters;
use nana_ui_core::dynamic_layout::{
    AdaptationProfile, EnvelopeShape, ExecutionClass, LayoutUnits, Participant, SolveFrontier,
    SolverBudget, SolverPolicy, SolverScratch, aggregate_sequential, lower_box,
    reassign_within_frontier, solve_deficit,
};

use super::*;

/// What a box's envelope was built from: equal inputs give an equal shape.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EnvelopeInputs {
    style: usize,
    padding: [u32; 2],
    gap: u32,
    gaps: u32,
    children: Vec<(StableNodeId, u64)>,
}

/// An envelope kept across passes.
#[derive(Debug, Clone)]
pub(crate) struct RetainedEnvelope {
    inputs: EnvelopeInputs,
    pub(crate) shape: EnvelopeShape,
}

/// A line solve kept across passes: who took part (and their envelopes'
/// generations), what the container's own gap could give, and where the
/// solve stopped.
#[derive(Debug, Clone)]
pub(crate) struct RetainedLineSolve {
    participants: Vec<(StableNodeId, u64)>,
    own_gap: EnvelopeShape,
    frontier: SolveFrontier,
}

impl RetainedLineSolve {
    /// How many children the solve holds.
    #[cfg(test)]
    pub(crate) fn participants(&self) -> usize {
        self.participants.len()
    }
}

/// What a line assigned a child, and what the child resolved of it inside
/// itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppliedAdjustment {
    pub(crate) inline: bool,
    pub(crate) amount: LayoutUnits,
    /// The part its padding closed up.
    pub(crate) padding: LayoutUnits,
}

/// A box's envelope in its two parts, and what it was built from.
struct EnvelopeParts {
    padding: EnvelopeShape,
    line: EnvelopeShape,
    inputs: EnvelopeInputs,
}

/// What a line assigned one child: how much smaller than its preferred size
/// it is, along which axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Assignment {
    pub(super) inline: bool,
    pub(super) amount: LayoutUnits,
}

/// A line solve's answer, memoized for the pass so measure and placement of
/// one container share it.
#[derive(Debug, Clone)]
struct LineAnswer {
    amounts: Vec<LayoutUnits>,
    own_gap: LayoutUnits,
}

/// Dynamic Layout state for one pass.
#[derive(Default)]
pub(super) struct DynamicPass {
    pub(super) counters: DynamicLayoutCounters,
    scratch: SolverScratch,
    envelopes: HashMap<StableNodeId, RetainedEnvelope>,
    lines: HashMap<(StableNodeId, i32), LineAnswer>,
    /// Assignments placement committed, by child.
    pub(super) assignments: HashMap<StableNodeId, Assignment>,
    /// Children whose line was solved this pass, assigned or not. A child a
    /// container placed from its kept plan, without solving, keeps the
    /// assignment it had: the plan placed it at the size that assignment
    /// gave it.
    decided: HashSet<StableNodeId>,
    /// Line solves to keep (`Some`) or drop (`None`) after the pass.
    pub(super) line_solves: HashMap<StableNodeId, Option<RetainedLineSolve>>,
    /// Adjustments children resolved, to keep (`Some`) or drop (`None`).
    pub(super) applied: HashMap<StableNodeId, Option<AppliedAdjustment>>,
}

/// One line about to be solved: its container and what the pass can read.
pub(super) struct DynamicLine<'p> {
    pub(super) container: StableNodeId,
    pub(super) profile: AdaptationProfile,
    pub(super) pass: &'p mut DynamicPass,
    pub(super) retained: Option<&'p DocumentLayoutCache>,
    /// Placement commits assignments; measure only reads sizes.
    pub(super) commit: bool,
}

impl DynamicLine<'_> {
    /// The line a container solves, if it solves one: its profile says so,
    /// its children sit on a single line, and its page is horizontal.
    pub(super) fn of<'p>(
        container: StableNodeId,
        style: &LayoutStyle,
        writing: nana_ui_core::WritingContext,
        single_line: bool,
        pass: &'p mut DynamicPass,
        retained: Option<&'p DocumentLayoutCache>,
        commit: bool,
    ) -> Option<DynamicLine<'p>> {
        let profile = style.adaptation?;
        (solves(&profile) && single_line && !writing.is_vertical()).then_some(DynamicLine {
            container,
            profile,
            pass,
            retained,
            commit,
        })
    }
}

/// What a line's solve may do: local-box work only (padding and gaps close
/// up, content keeps its size), and a budget wide enough for a toolbar of a
/// few thousand items before a class is shared by capacity.
fn line_policy() -> SolverPolicy {
    SolverPolicy {
        allowed: ExecutionClass::LocalBox,
        budget: SolverBudget {
            max_states: LINE_MAX_STATES,
            ..SolverBudget::default()
        },
        ..SolverPolicy::default()
    }
}

/// Segments one line's cost class may share before it is shared by class.
pub(super) const LINE_MAX_STATES: usize = 4096;

/// Whether a box with `profile` solves its own line's overflow: asked to, or
/// passing its children's elasticity up, which only holds if its line then
/// spends what the box was assigned.
pub(super) fn solves(profile: &AdaptationProfile) -> bool {
    profile.solve_overflow || profile.aggregate_children
}

/// One participant of a line: a child's index in the line, its envelope and
/// the most it may give up.
pub(super) struct LineChild {
    pub(super) index: usize,
    pub(super) id: StableNodeId,
    pub(super) shape: EnvelopeShape,
    pub(super) limit: LayoutUnits,
}

impl DynamicPass {
    /// `id`'s envelope along `inline` (else block), its padding resolved
    /// against `containing_inline`: from this pass, else kept, else built.
    pub(super) fn envelope(
        &mut self,
        id: StableNodeId,
        inline: bool,
        containing_inline: f32,
        parent_font_px: f32,
        nodes: &LayoutInputMap<'_>,
        retained: Option<&DocumentLayoutCache>,
    ) -> EnvelopeShape {
        let Some(style) = nodes.style(id) else {
            return EnvelopeShape::RIGID;
        };
        if style.adaptation.is_none() {
            return EnvelopeShape::RIGID;
        }
        self.counters.envelope_queries += 1;
        if let Some(held) = self.envelopes.get(&id) {
            self.counters.envelope_hits += 1;
            return held.shape;
        }
        let parts = self.parts(
            id,
            inline,
            containing_inline,
            parent_font_px,
            nodes,
            retained,
        );
        let Some(parts) = parts else {
            return EnvelopeShape::RIGID;
        };
        let kept = retained.and_then(|retained| retained.envelopes.get(&id));
        if let Some(kept) = kept
            && kept.inputs == parts.inputs
        {
            self.counters.envelope_hits += 1;
            self.envelopes.insert(id, kept.clone());
            return kept.shape;
        }
        if kept.is_some() {
            self.counters.envelope_rebuilds += 1;
        } else {
            self.counters.envelope_misses += 1;
        }
        let mut shape = aggregate_sequential(&parts.padding, [&parts.line]);
        shape.generation = match kept {
            Some(kept) if kept.shape.same_facts(&shape) => kept.shape.generation,
            Some(kept) => kept.shape.generation + 1,
            None => 1,
        };
        self.envelopes.insert(
            id,
            RetainedEnvelope {
                inputs: parts.inputs,
                shape,
            },
        );
        shape
    }

    /// What `id` gives up, in two parts: its own padding, and its line -- its
    /// own gaps and, when it passes them up, its children's envelopes.
    fn parts(
        &mut self,
        id: StableNodeId,
        inline: bool,
        containing_inline: f32,
        parent_font_px: f32,
        nodes: &LayoutInputMap<'_>,
        retained: Option<&DocumentLayoutCache>,
    ) -> Option<EnvelopeParts> {
        let style = nodes.style(id)?;
        let profile = style.adaptation?;
        let fonts = fonts_of(style.as_ref(), parent_font_px);
        let padding = style.resolved_padding_against_fonts(Some(containing_inline), fonts);
        let (start, end) = if inline {
            (padding.left, padding.right)
        } else {
            (padding.top, padding.bottom)
        };
        let direction = style.direction.unwrap_or(FlexDirection::Column);
        let along = (direction == FlexDirection::Row) == inline;
        let children: Vec<StableNodeId> = if profile.aggregate_children && along {
            nodes.world.child_ids(id).to_vec()
        } else {
            Vec::new()
        };
        let gap = if along {
            style.main_gap_against_fonts(
                direction,
                gap_containing_block(style.as_ref(), Size::new(containing_inline, 0.0)),
                fonts,
            )
        } else {
            0.0
        };
        let gaps = children.len().saturating_sub(1);
        let content_inline = (containing_inline - start - end).max(0.0);
        let child_shapes: Vec<(StableNodeId, EnvelopeShape)> = children
            .iter()
            .map(|child| {
                (
                    *child,
                    self.envelope(
                        *child,
                        inline,
                        content_inline,
                        fonts.element_px,
                        nodes,
                        retained,
                    ),
                )
            })
            .collect();
        let own_gap = lower_box(profile.axis(inline), (0.0, 0.0), gap, gaps, 0.0, 0);
        let line = aggregate_sequential(&own_gap, child_shapes.iter().map(|(_, shape)| shape));
        Some(EnvelopeParts {
            padding: lower_box(profile.axis(inline), (start, end), 0.0, 0, 0.0, 0),
            line,
            inputs: EnvelopeInputs {
                style: Arc::as_ptr(&style) as usize,
                padding: [start.to_bits(), end.to_bits()],
                gap: gap.to_bits(),
                gaps: gaps as u32,
                children: child_shapes
                    .iter()
                    .map(|(child, shape)| (*child, shape.generation))
                    .collect(),
            },
        })
    }

    /// Solve one overflowing line: what each child gives up, and how much
    /// the container's own gaps close, in total. Kept solves whose
    /// participants still match and whose stopping level still holds the
    /// deficit are only re-shared.
    pub(super) fn solve_line(
        &mut self,
        container: StableNodeId,
        own_gap: EnvelopeShape,
        children: &[LineChild],
        deficit: LayoutUnits,
        retained: Option<&DocumentLayoutCache>,
        commit: bool,
    ) -> (Vec<LayoutUnits>, LayoutUnits) {
        let key = (container, deficit.0);
        if let Some(answer) = self.lines.get(&key) {
            self.counters.solver_reuses += 1;
            return (answer.amounts.clone(), answer.own_gap);
        }
        let ids: Vec<(StableNodeId, u64)> = children
            .iter()
            .map(|child| (child.id, child.shape.generation))
            .collect();
        let held = retained
            .and_then(|retained| retained.line_solves.get(&container))
            .filter(|held| held.participants == ids && held.own_gap.same_facts(&own_gap));
        let mut amounts = Vec::new();
        let frontier = match held {
            Some(held)
                if reassign_within_frontier(
                    &held.frontier,
                    deficit,
                    &mut amounts,
                    &mut self.counters,
                )
                .is_some() =>
            {
                self.counters.previous_result_hits += 1;
                self.counters.solver_reuses += 1;
                #[cfg(any(test, feature = "layout-verify"))]
                {
                    let cold = self.cold_line(&own_gap, children, deficit);
                    assert_eq!(
                        cold, amounts,
                        "{container:?}: a re-shared line solve differs from a cold one"
                    );
                }
                held.frontier.clone()
            }
            _ => {
                let mut counters = std::mem::take(&mut self.counters);
                let outcome = Self::solve_cold(
                    &own_gap,
                    children,
                    deficit,
                    &mut self.scratch,
                    &mut counters,
                );
                self.counters = counters;
                amounts.clone_from(&self.scratch.amounts);
                outcome.frontier
            }
        };
        let own = amounts.first().copied().unwrap_or_default();
        let per_child: Vec<LayoutUnits> = amounts.iter().skip(1).copied().collect();
        if commit {
            self.line_solves.insert(
                container,
                Some(RetainedLineSolve {
                    participants: ids,
                    own_gap,
                    frontier,
                }),
            );
        }
        self.lines.insert(
            key,
            LineAnswer {
                amounts: per_child.clone(),
                own_gap: own,
            },
        );
        (per_child, own)
    }

    /// A line's solve from nothing: the container's own gaps first in
    /// order, then each child.
    fn solve_cold(
        own_gap: &EnvelopeShape,
        children: &[LineChild],
        deficit: LayoutUnits,
        scratch: &mut SolverScratch,
        counters: &mut DynamicLayoutCounters,
    ) -> nana_ui_core::dynamic_layout::SolveOutcome {
        let mut participants = Vec::with_capacity(children.len() + 1);
        participants.push(Participant::new(own_gap, LayoutUnits(i32::MAX)));
        participants.extend(
            children
                .iter()
                .map(|child| Participant::new(&child.shape, child.limit)),
        );
        solve_deficit(
            &participants,
            deficit,
            &line_policy(),
            scratch,
            None,
            counters,
        )
    }

    /// What a cold solve gives, for the checks that a re-shared one matches.
    #[cfg(any(test, feature = "layout-verify"))]
    fn cold_line(
        &self,
        own_gap: &EnvelopeShape,
        children: &[LineChild],
        deficit: LayoutUnits,
    ) -> Vec<LayoutUnits> {
        let mut scratch = SolverScratch::default();
        Self::solve_cold(
            own_gap,
            children,
            deficit,
            &mut scratch,
            &mut DynamicLayoutCounters::default(),
        );
        scratch.amounts
    }

    /// The padding `id` closes up for the assignment its line gave it: the
    /// part of the assignment its padding takes when its own padding and its
    /// line compete by cost. The rest its line spends as overflow.
    pub(super) fn resolve_padding(
        &mut self,
        id: StableNodeId,
        padding: nana_ui_core::PaddingSpec,
        containing_inline: f32,
        parent_font_px: f32,
        nodes: &LayoutInputMap<'_>,
        retained: Option<&DocumentLayoutCache>,
    ) -> nana_ui_core::PaddingSpec {
        let kept = retained.and_then(|retained| retained.applied_adjustments.get(&id));
        let assignment = if self.decided.contains(&id) {
            self.assignments.get(&id).copied()
        } else {
            kept.map(|kept| Assignment {
                inline: kept.inline,
                amount: kept.amount,
            })
        };
        let Some(assignment) = assignment else {
            if kept.is_some() {
                self.applied.insert(id, None);
            }
            return padding;
        };
        if nodes
            .style(id)
            .is_none_or(|style| style.adaptation.is_none())
        {
            return padding;
        }
        self.counters.child_resolves += 1;
        let inline = assignment.inline;
        let (start, end) = if inline {
            (padding.left, padding.right)
        } else {
            (padding.top, padding.bottom)
        };
        let Some(parts) = self.parts(
            id,
            inline,
            containing_inline,
            parent_font_px,
            nodes,
            retained,
        ) else {
            return padding;
        };
        let (own, line) = (parts.padding, parts.line);
        let participants = [
            Participant::new(&own, LayoutUnits(i32::MAX)),
            Participant::new(&line, LayoutUnits(i32::MAX)),
        ];
        let policy = line_policy();
        let mut stats = DynamicLayoutCounters::default();
        solve_deficit(
            &participants,
            assignment.amount,
            &policy,
            &mut self.scratch,
            None,
            &mut stats,
        );
        let closed = self.scratch.amounts[0].min(own.total_capacity());
        // Both edges close up evenly; an odd unit goes to the end edge.
        let first = LayoutUnits(closed.0 / 2);
        let second = closed - first;
        let mut resolved = padding;
        if inline {
            resolved.left = (start - first.to_px()).max(0.0);
            resolved.right = (end - second.to_px()).max(0.0);
        } else {
            resolved.top = (start - first.to_px()).max(0.0);
            resolved.bottom = (end - second.to_px()).max(0.0);
        }
        self.applied.insert(
            id,
            Some(AppliedAdjustment {
                inline,
                amount: assignment.amount,
                padding: closed,
            }),
        );
        resolved
    }

    /// Record what a line assigned its children, and drop a kept solve the
    /// line no longer needs.
    pub(super) fn commit_line(
        &mut self,
        container: StableNodeId,
        children: &[StableNodeId],
        amounts: &[LayoutUnits],
        inline: bool,
        retained: Option<&DocumentLayoutCache>,
    ) {
        for (index, child) in children.iter().enumerate() {
            self.decided.insert(*child);
            let amount = amounts.get(index).copied().unwrap_or_default();
            if amount.is_positive() {
                self.assignments
                    .insert(*child, Assignment { inline, amount });
            } else {
                self.assignments.remove(child);
            }
        }
        if amounts.iter().all(|amount| !amount.is_positive())
            && !self.line_solves.contains_key(&container)
            && retained.is_some_and(|retained| retained.line_solves.contains_key(&container))
        {
            self.line_solves.insert(container, None);
        }
    }

    /// Envelopes built this pass, to keep.
    pub(super) fn take_envelopes(&mut self) -> HashMap<StableNodeId, RetainedEnvelope> {
        std::mem::take(&mut self.envelopes)
    }
}
