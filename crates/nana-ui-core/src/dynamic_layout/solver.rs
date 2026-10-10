//! Choosing what a short line gives up (Issue #210).
//!
//! [`solve_deficit`] covers a deficit from its participants' shapes, cheapest
//! first, in this order:
//!
//! 1. Nothing to cover: done.
//! 2. Feasibility from each class's capacity sum, without reading segments.
//! 3. The cheapest class `k` whose capacity, with every cheaper class, covers
//!    the deficit. Cheaper classes are taken whole from their sums; dearer
//!    ones are never read.
//! 4. Within `k`, cost levels from the cheapest: each level taken whole until
//!    one covers what is left, which is shared by capacity, the remainder one
//!    unit at a time in participant order.
//!
//! A level that more participants share than the budget allows is shared by
//! class capacity instead, the same way every time. Discrete candidates, when
//! the policy allows structural work, are compared by lower bound and pruned.
//! The level a solve stopped in is its [`SolveFrontier`]: a deficit that
//! still falls inside it only re-shares that level
//! ([`reassign_within_frontier`]), which is the last step of the cold solve
//! and so gives the same answer.

use crate::DynamicLayoutCounters;

use super::{DiscreteCandidate, EnvelopeShape, ExecutionClass, LayoutCost, LayoutUnits, TotalCost};

/// One box in a line: what it may give up, and the most it may give up (its
/// preferred size less the least its style lets it be).
#[derive(Debug, Clone, Copy)]
pub struct Participant<'a> {
    pub shape: &'a EnvelopeShape,
    pub limit: LayoutUnits,
    /// Discrete arrangements it may switch to; empty for most.
    pub discrete: &'a [DiscreteCandidate],
}

impl<'a> Participant<'a> {
    pub fn new(shape: &'a EnvelopeShape, limit: LayoutUnits) -> Self {
        Self {
            shape,
            limit: limit.clamp_non_negative(),
            discrete: &[],
        }
    }

    /// Whether `limit` cuts into the shape, so its class sums overstate what
    /// it may give up.
    fn clipped(&self) -> bool {
        self.limit < self.shape.total_capacity()
    }
}

/// Refines a coarse segment on request: a participant whose shape only
/// summarizes its reflow capacity (a paragraph's break opportunities) lays
/// out its detail when, and only when, a solve needs that class.
pub trait ParticipantSource {
    /// The refined shape of participant `index`, with the same capacity per
    /// class; `None` keeps the coarse one.
    fn expand(&mut self, index: usize) -> Option<EnvelopeShape>;
}

/// Hard limits on what one solve may do. Past any of them a solve falls back
/// to a deterministic answer rather than search further.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolverBudget {
    /// Segments one cost class may share before it is shared by class.
    pub max_states: usize,
    /// Coarse segments refined per solve.
    pub max_deep_expansions: usize,
    /// Participants whose reflow capacity is considered for refinement.
    pub max_reflow_candidates: usize,
    /// Segments read per participant.
    pub max_segments_per_child: usize,
}

impl Default for SolverBudget {
    fn default() -> Self {
        Self {
            max_states: 256,
            max_deep_expansions: 8,
            max_reflow_candidates: 8,
            max_segments_per_child: super::MAX_ENVELOPE_SEGMENTS,
        }
    }
}

/// What a solve may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolverPolicy {
    pub budget: SolverBudget,
    /// The dearest class a solve may open.
    pub allowed: ExecutionClass,
    /// Whether discrete arrangements may be chosen.
    pub allow_structural: bool,
}

impl Default for SolverPolicy {
    fn default() -> Self {
        Self {
            budget: SolverBudget::default(),
            allowed: ExecutionClass::LocalReflow,
            allow_structural: false,
        }
    }
}

/// Buffers a solve reuses from one call to the next.
#[derive(Debug, Default)]
pub struct SolverScratch {
    /// What each participant gives up, after a solve.
    pub amounts: Vec<LayoutUnits>,
    entries: Vec<Entry>,
    expanded: Vec<Option<EnvelopeShape>>,
    discrete: Vec<(usize, usize, LayoutUnits, LayoutCost)>,
}

impl SolverScratch {
    fn prepare(&mut self, participants: usize, stats: &mut DynamicLayoutCounters) {
        let grew =
            self.amounts.capacity() < participants || self.expanded.capacity() < participants;
        self.amounts.clear();
        self.amounts.resize(participants, LayoutUnits::ZERO);
        self.expanded.clear();
        self.expanded.resize(participants, None);
        self.entries.clear();
        self.discrete.clear();
        if grew {
            stats.allocations += 1;
        }
        stats.temp_bytes = stats.temp_bytes.max(self.bytes());
    }

    fn bytes(&self) -> usize {
        self.amounts.capacity() * std::mem::size_of::<LayoutUnits>()
            + self.entries.capacity() * std::mem::size_of::<Entry>()
            + self.expanded.capacity() * std::mem::size_of::<Option<EnvelopeShape>>()
            + self.discrete.capacity()
                * std::mem::size_of::<(usize, usize, LayoutUnits, LayoutCost)>()
    }

    fn push_entry(&mut self, entry: Entry, stats: &mut DynamicLayoutCounters) {
        if self.entries.len() == self.entries.capacity() {
            stats.allocations += 1;
        }
        self.entries.push(entry);
        stats.temp_bytes = stats.temp_bytes.max(self.bytes());
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    cost: LayoutCost,
    participant: u32,
    segment: u8,
    capacity: LayoutUnits,
}

/// Where a solve stopped: the cost level whose capacity the deficit ended
/// in, what every participant took before it, and the level's share of each.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SolveFrontier {
    /// Whether the deficit fell inside a level at all: an infeasible solve, a
    /// discrete choice or nothing to cover leaves nothing to re-share.
    pub reusable: bool,
    pub class: ExecutionClass,
    pub cost: LayoutCost,
    /// Capacity taken before this level, and through it.
    pub cum_before: LayoutUnits,
    pub cum_through: LayoutUnits,
    /// What each participant took before this level, and what that cost.
    pub base: Vec<LayoutUnits>,
    pub base_cost: TotalCost,
    /// The level's capacity per participant, in participant order.
    pub tied: Vec<(u32, LayoutUnits)>,
}

impl SolveFrontier {
    /// Whether a deficit of `deficit` falls inside this level.
    pub fn holds(&self, deficit: LayoutUnits) -> bool {
        self.reusable && self.cum_before < deficit && deficit <= self.cum_through
    }
}

/// A solve's answer; what each participant gives up is in
/// [`SolverScratch::amounts`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SolveOutcome {
    /// What no participant could cover.
    pub residual: LayoutUnits,
    pub total: TotalCost,
    pub frontier: SolveFrontier,
    /// Whether a budget made the solve share a class by capacity.
    pub fallback: bool,
    /// The discrete candidate chosen, as (participant, candidate).
    pub discrete: Option<(usize, usize)>,
}

/// Cover `deficit` from `participants`. See the module docs.
pub fn solve_deficit(
    participants: &[Participant<'_>],
    deficit: LayoutUnits,
    policy: &SolverPolicy,
    scratch: &mut SolverScratch,
    source: Option<&mut (dyn ParticipantSource + '_)>,
    stats: &mut DynamicLayoutCounters,
) -> SolveOutcome {
    stats.solver_runs += 1;
    stats.cold_solves += 1;
    scratch.prepare(participants.len(), stats);
    let mut outcome = solve_continuous(participants, deficit, policy, scratch, source, stats);
    if policy.allow_structural && deficit.is_positive() {
        choose_discrete(participants, deficit, policy, scratch, &mut outcome, stats);
    }
    outcome
}

/// Re-share the level `frontier` stopped in for a new `deficit` that falls
/// inside it, writing what each participant gives up to `out`. `None` when
/// the deficit left the level: solve again.
pub fn reassign_within_frontier(
    frontier: &SolveFrontier,
    deficit: LayoutUnits,
    out: &mut Vec<LayoutUnits>,
    stats: &mut DynamicLayoutCounters,
) -> Option<TotalCost> {
    if !frontier.holds(deficit) {
        return None;
    }
    stats.solver_runs += 1;
    stats.incremental_assignments += 1;
    out.clear();
    out.extend_from_slice(&frontier.base);
    let remaining = deficit - frontier.cum_before;
    share(remaining, &frontier.tied, out);
    let mut total = frontier.base_cost;
    total.add(frontier.cost, remaining);
    Some(total)
}

/// Each participant's capacity in `class`, cut by its limit where the limit
/// reaches into the shape (segments are kept cheapest class first).
fn clipped_class_capacity(
    participant: &Participant<'_>,
    shape: &EnvelopeShape,
    class: ExecutionClass,
) -> LayoutUnits {
    if !participant.clipped() {
        return shape.class_capacity(class);
    }
    let mut left = participant.limit;
    let mut capacity = LayoutUnits::ZERO;
    for segment in shape.segments() {
        let take = segment.capacity.min(left);
        left -= take;
        if segment.execution_class == class {
            capacity += take;
        }
    }
    capacity
}

/// What taking all of `class` from a participant costs.
fn clipped_class_cost(
    participant: &Participant<'_>,
    shape: &EnvelopeShape,
    class: ExecutionClass,
) -> TotalCost {
    if !participant.clipped() {
        return shape.class_cost(class);
    }
    let mut left = participant.limit;
    let mut total = TotalCost::ZERO;
    for segment in shape.segments() {
        let take = segment.capacity.min(left);
        left -= take;
        if segment.execution_class == class {
            total.add(segment.marginal_cost, take);
        }
    }
    total
}

fn solve_continuous(
    participants: &[Participant<'_>],
    deficit: LayoutUnits,
    policy: &SolverPolicy,
    scratch: &mut SolverScratch,
    source: Option<&mut (dyn ParticipantSource + '_)>,
    stats: &mut DynamicLayoutCounters,
) -> SolveOutcome {
    let mut outcome = SolveOutcome::default();
    if !deficit.is_positive() {
        return outcome;
    }
    let classes: Vec<ExecutionClass> = ExecutionClass::ALL
        .into_iter()
        .filter(|class| *class <= policy.allowed && *class != ExecutionClass::Structural)
        .collect();
    // Feasibility from class sums: no segment is read here.
    let mut covered = LayoutUnits::ZERO;
    let mut chosen = None;
    for class in &classes {
        let total: LayoutUnits = participants
            .iter()
            .map(|participant| clipped_class_capacity(participant, participant.shape, *class))
            .sum();
        if covered + total >= deficit {
            chosen = Some(*class);
            break;
        }
        covered += total;
    }
    let Some(chosen) = chosen else {
        // Short whatever is taken: take everything allowed.
        for (at, participant) in participants.iter().enumerate() {
            for class in &classes {
                scratch.amounts[at] +=
                    clipped_class_capacity(participant, participant.shape, *class);
                outcome.total =
                    outcome
                        .total
                        .plus(clipped_class_cost(participant, participant.shape, *class));
            }
        }
        let taken: LayoutUnits = scratch.amounts.iter().copied().sum();
        outcome.residual = deficit - taken;
        return outcome;
    };
    // Every cheaper class whole, from its sums.
    for (at, participant) in participants.iter().enumerate() {
        for class in classes.iter().filter(|class| **class < chosen) {
            scratch.amounts[at] += clipped_class_capacity(participant, participant.shape, *class);
            outcome.total =
                outcome
                    .total
                    .plus(clipped_class_cost(participant, participant.shape, *class));
        }
    }
    let mut remaining = deficit - covered;
    // Refine coarse reflow capacity, within budget, only now that a solve
    // needs that class.
    if chosen == ExecutionClass::LocalReflow
        && let Some(source) = source
    {
        let mut considered = 0;
        for (at, participant) in participants.iter().enumerate() {
            let coarse = participant.shape.segments().iter().any(|segment| {
                segment.coarse && segment.execution_class == ExecutionClass::LocalReflow
            });
            if !coarse {
                continue;
            }
            considered += 1;
            if considered > policy.budget.max_reflow_candidates || stats_expansions(stats, policy) {
                break;
            }
            if let Some(shape) = source.expand(at) {
                debug_assert_eq!(
                    shape.class_capacity(ExecutionClass::LocalReflow),
                    participant
                        .shape
                        .class_capacity(ExecutionClass::LocalReflow),
                    "a refined shape keeps its class capacity"
                );
                scratch.expanded[at] = Some(shape);
                stats.deep_expansions += 1;
            }
        }
    }
    // The chosen class's segments, clipped by each participant's limit.
    for (at, participant) in participants.iter().enumerate() {
        let shape = scratch.expanded[at].unwrap_or(*participant.shape);
        if clipped_class_capacity(participant, &shape, chosen) == LayoutUnits::ZERO {
            stats.pruned_candidates += 1;
            continue;
        }
        let mut left = participant.limit;
        for (index, segment) in shape
            .segments()
            .iter()
            .enumerate()
            .take(policy.budget.max_segments_per_child)
        {
            let take = segment.capacity.min(left);
            left -= take;
            if segment.execution_class != chosen || !take.is_positive() {
                continue;
            }
            stats.cost_segments_visited += 1;
            scratch.push_entry(
                Entry {
                    cost: segment.marginal_cost,
                    participant: at as u32,
                    segment: index as u8,
                    capacity: take,
                },
                stats,
            );
        }
    }
    outcome.frontier.class = chosen;
    if scratch.entries.len() > policy.budget.max_states {
        // Shared by class capacity: one level, the same way every time.
        stats.budget_fallbacks += 1;
        outcome.fallback = true;
        let mut per_participant: Vec<(u32, LayoutUnits)> = Vec::new();
        let mut dearest = LayoutCost::ZERO;
        for entry in &scratch.entries {
            dearest = dearest.max(entry.cost);
            match per_participant.last_mut() {
                Some((participant, capacity)) if *participant == entry.participant => {
                    *capacity += entry.capacity;
                }
                _ => per_participant.push((entry.participant, entry.capacity)),
            }
        }
        let through: LayoutUnits = per_participant.iter().map(|(_, capacity)| *capacity).sum();
        outcome.frontier = SolveFrontier {
            reusable: true,
            class: chosen,
            cost: dearest,
            cum_before: covered,
            cum_through: covered + through,
            base: scratch.amounts.clone(),
            base_cost: outcome.total,
            tied: per_participant,
        };
        share(remaining, &outcome.frontier.tied, &mut scratch.amounts);
        outcome.total.add(dearest, remaining);
        return outcome;
    }
    scratch
        .entries
        .sort_by_key(|entry| (entry.cost, entry.participant, entry.segment));
    let mut start = 0;
    let mut levels = 0;
    while start < scratch.entries.len() {
        let cost = scratch.entries[start].cost;
        let end = scratch.entries[start..]
            .iter()
            .position(|entry| entry.cost != cost)
            .map_or(scratch.entries.len(), |offset| start + offset);
        levels += 1;
        let level: LayoutUnits = scratch.entries[start..end]
            .iter()
            .map(|entry| entry.capacity)
            .sum();
        if remaining > level {
            for entry in &scratch.entries[start..end] {
                scratch.amounts[entry.participant as usize] += entry.capacity;
            }
            outcome.total.add(cost, level);
            remaining -= level;
            covered += level;
            start = end;
            continue;
        }
        let tied: Vec<(u32, LayoutUnits)> = scratch.entries[start..end]
            .iter()
            .map(|entry| (entry.participant, entry.capacity))
            .collect();
        outcome.frontier = SolveFrontier {
            reusable: true,
            class: chosen,
            cost,
            cum_before: covered,
            cum_through: covered + level,
            base: scratch.amounts.clone(),
            base_cost: outcome.total,
            tied,
        };
        share(remaining, &outcome.frontier.tied, &mut scratch.amounts);
        outcome.total.add(cost, remaining);
        if levels == 1 && chosen == classes[0] {
            stats.fast_adjust += 1;
        }
        return outcome;
    }
    debug_assert!(false, "a feasible class covers its deficit");
    outcome
}

fn stats_expansions(stats: &DynamicLayoutCounters, policy: &SolverPolicy) -> bool {
    // Counted per solve by the caller's fresh counters; a shared counter only
    // ever makes this stricter.
    stats.deep_expansions >= policy.budget.max_deep_expansions
}

/// Share `amount` over `tied` by capacity: floor of each share, then the
/// remainder one unit at a time in order to those not yet full.
fn share(amount: LayoutUnits, tied: &[(u32, LayoutUnits)], out: &mut [LayoutUnits]) {
    let total: i64 = tied.iter().map(|(_, capacity)| i64::from(capacity.0)).sum();
    if total <= 0 || !amount.is_positive() {
        return;
    }
    let amount = i64::from(amount.0).min(total);
    let mut given = 0i64;
    let mut shares: Vec<i64> = Vec::with_capacity(tied.len());
    for (_, capacity) in tied {
        let part = amount * i64::from(capacity.0) / total;
        shares.push(part);
        given += part;
    }
    let mut left = amount - given;
    for (at, (_, capacity)) in tied.iter().enumerate() {
        if left == 0 {
            break;
        }
        if shares[at] < i64::from(capacity.0) {
            shares[at] += 1;
            left -= 1;
        }
    }
    for ((participant, _), part) in tied.iter().zip(shares) {
        out[*participant as usize] += LayoutUnits(part as i32);
    }
}

/// Compare discrete candidates against the continuous answer: each one's
/// fixed cost is a lower bound, so one that already costs as much as the best
/// so far is pruned without solving what it leaves.
fn choose_discrete(
    participants: &[Participant<'_>],
    deficit: LayoutUnits,
    policy: &SolverPolicy,
    scratch: &mut SolverScratch,
    outcome: &mut SolveOutcome,
    stats: &mut DynamicLayoutCounters,
) {
    scratch.discrete.clear();
    for (at, participant) in participants.iter().enumerate() {
        for (index, candidate) in participant.discrete.iter().enumerate() {
            if candidate.cost.is_forbidden() {
                continue;
            }
            scratch.discrete.push((
                at,
                index,
                LayoutUnits::from_px(candidate.saves_px),
                candidate.cost,
            ));
        }
    }
    if scratch.discrete.is_empty() {
        return;
    }
    // Most saving per unit cost first, then participant order.
    scratch.discrete.sort_by_key(|(at, index, saves, cost)| {
        let cost = u64::from(cost.finite().unwrap_or(u32::MAX));
        let saves = saves.0.max(1) as u64;
        (cost * 1_000_000 / saves, *at, *index)
    });
    let beam =
        (policy.budget.max_states / participants.len().max(1)).clamp(1, scratch.discrete.len());
    stats.beam_states = stats.beam_states.max(beam);
    let mut best: (bool, TotalCost) = (outcome.residual.is_positive(), outcome.total);
    let mut chosen = None;
    let candidates: Vec<_> = scratch.discrete[..beam].to_vec();
    let mut scratch_inner = SolverScratch::default();
    for (at, index, saves, cost) in candidates {
        stats.candidates_created += 1;
        let mut fixed = TotalCost::ZERO;
        fixed.add(cost, LayoutUnits(LayoutUnits::PER_PX));
        if (false, fixed) >= best {
            stats.candidates_pruned += 1;
            continue;
        }
        let left = deficit - saves;
        scratch_inner.prepare(participants.len(), &mut DynamicLayoutCounters::default());
        let rest = solve_continuous(
            participants,
            left,
            policy,
            &mut scratch_inner,
            None,
            &mut DynamicLayoutCounters::default(),
        );
        let candidate = (rest.residual.is_positive(), fixed.plus(rest.total));
        if candidate < best {
            best = candidate;
            chosen = Some((at, index, rest, scratch_inner.amounts.clone()));
        }
    }
    if let Some((at, index, rest, amounts)) = chosen {
        stats.structural_adaptations += 1;
        scratch.amounts = amounts;
        *outcome = SolveOutcome {
            residual: rest.residual,
            total: best.1,
            frontier: SolveFrontier::default(),
            fallback: outcome.fallback,
            discrete: Some((at, index)),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dynamic_layout::{AdjustmentKind, AdjustmentSegment};

    fn px(px: f32) -> LayoutUnits {
        LayoutUnits::from_px(px)
    }

    fn shape(segments: &[(f32, u32, ExecutionClass)]) -> EnvelopeShape {
        EnvelopeShape::from_segments(segments.iter().map(|(capacity, cost, class)| {
            AdjustmentSegment::new(
                px(*capacity),
                LayoutCost::Finite(*cost),
                AdjustmentKind::Padding,
                *class,
            )
        }))
    }

    fn solve(
        shapes: &[EnvelopeShape],
        deficit: f32,
    ) -> (SolveOutcome, Vec<LayoutUnits>, DynamicLayoutCounters) {
        let participants: Vec<_> = shapes
            .iter()
            .map(|shape| Participant::new(shape, px(1e6)))
            .collect();
        let mut scratch = SolverScratch::default();
        let mut stats = DynamicLayoutCounters::default();
        let outcome = solve_deficit(
            &participants,
            px(deficit),
            &SolverPolicy::default(),
            &mut scratch,
            None,
            &mut stats,
        );
        (outcome, scratch.amounts, stats)
    }

    use ExecutionClass::{LocalBox, LocalReflow, PlacementOnly};

    #[test]
    fn a_one_pixel_deficit_takes_the_cheapest_class_and_never_reads_the_rest() {
        let shapes = vec![
            shape(&[
                (4.0, 10, PlacementOnly),
                (8.0, 20, LocalBox),
                (30.0, 50, LocalReflow),
            ]),
            shape(&[(4.0, 10, PlacementOnly), (8.0, 20, LocalBox)]),
        ];
        let (outcome, amounts, stats) = solve(&shapes, 1.0);
        assert_eq!(amounts.iter().copied().sum::<LayoutUnits>(), px(1.0));
        assert_eq!(outcome.frontier.class, PlacementOnly);
        assert_eq!(stats.cost_segments_visited, 2);
        assert_eq!(stats.fast_adjust, 1);
        assert_eq!(outcome.residual, LayoutUnits::ZERO);
    }

    #[test]
    fn a_level_is_shared_by_capacity_and_the_remainder_goes_in_order() {
        let shapes = vec![shape(&[(2.0, 10, LocalBox)]), shape(&[(4.0, 10, LocalBox)])];
        let (_, amounts, _) = solve(&shapes, 3.0);
        assert_eq!(amounts, vec![px(1.0), px(2.0)]);
        // 1/64 px left over goes to the first that is not full.
        let (_, amounts, _) = solve(&shapes, 3.0 + 1.0 / 64.0);
        assert_eq!(amounts[0] + amounts[1], px(3.0) + LayoutUnits(1));
    }

    #[test]
    fn a_saturated_level_passes_what_is_left_to_the_next() {
        let shapes = vec![
            shape(&[(2.0, 10, LocalBox), (10.0, 30, LocalBox)]),
            shape(&[(2.0, 10, LocalBox)]),
        ];
        let (outcome, amounts, _) = solve(&shapes, 6.0);
        assert_eq!(amounts, vec![px(4.0), px(2.0)]);
        assert_eq!(outcome.frontier.cost, LayoutCost::Finite(30));
        assert_eq!(outcome.total.finite, (10 * 4 * 64 + 30 * 2 * 64) as u64);
    }

    #[test]
    fn an_infeasible_deficit_takes_everything_and_reports_the_rest() {
        let shapes = vec![shape(&[(2.0, 10, LocalBox)])];
        let (outcome, amounts, _) = solve(&shapes, 5.0);
        assert_eq!(amounts, vec![px(2.0)]);
        assert_eq!(outcome.residual, px(3.0));
        assert!(!outcome.frontier.reusable);
    }

    #[test]
    fn a_limit_clips_the_dearest_capacity_first() {
        let shape = shape(&[(2.0, 10, PlacementOnly), (10.0, 20, LocalBox)]);
        let participants = [Participant::new(&shape, px(5.0))];
        let mut scratch = SolverScratch::default();
        let mut stats = DynamicLayoutCounters::default();
        let outcome = solve_deficit(
            &participants,
            px(9.0),
            &SolverPolicy::default(),
            &mut scratch,
            None,
            &mut stats,
        );
        assert_eq!(scratch.amounts, vec![px(5.0)]);
        assert_eq!(outcome.residual, px(4.0));
    }

    /// Every deficit inside a held level re-shares to exactly what a cold
    /// solve gives.
    #[test]
    fn re_sharing_a_held_level_equals_a_cold_solve_for_every_deficit() {
        let shapes = vec![
            shape(&[(1.0, 10, PlacementOnly), (3.0, 20, LocalBox)]),
            shape(&[(2.0, 20, LocalBox), (5.0, 40, LocalBox)]),
            shape(&[(0.5, 20, LocalBox)]),
        ];
        let (start, _, _) = solve(&shapes, 2.0);
        assert!(start.frontier.reusable);
        let frontier = start.frontier.clone();
        let mut out = Vec::new();
        for units in (frontier.cum_before.0 + 1)..=frontier.cum_through.0 {
            let deficit = LayoutUnits(units);
            let mut stats = DynamicLayoutCounters::default();
            let total = reassign_within_frontier(&frontier, deficit, &mut out, &mut stats).unwrap();
            let (cold, amounts, _) = solve(&shapes, deficit.to_px());
            assert_eq!(out, amounts, "{units}");
            assert_eq!(total, cold.total, "{units}");
            assert_eq!(stats.incremental_assignments, 1);
        }
        assert!(
            reassign_within_frontier(
                &frontier,
                frontier.cum_through + LayoutUnits(1),
                &mut out,
                &mut DynamicLayoutCounters::default()
            )
            .is_none()
        );
    }

    #[test]
    fn a_budget_shares_a_crowded_class_by_capacity_the_same_way_every_time() {
        let shapes: Vec<_> = (0..40)
            .map(|at| shape(&[(1.0, 10 + at % 7, LocalBox), (1.0, 50 + at % 5, LocalBox)]))
            .collect();
        let participants: Vec<_> = shapes
            .iter()
            .map(|shape| Participant::new(shape, px(1e6)))
            .collect();
        let policy = SolverPolicy {
            budget: SolverBudget {
                max_states: 16,
                ..SolverBudget::default()
            },
            ..SolverPolicy::default()
        };
        let mut runs = Vec::new();
        for _ in 0..3 {
            let mut scratch = SolverScratch::default();
            let mut stats = DynamicLayoutCounters::default();
            let outcome = solve_deficit(
                &participants,
                px(13.0),
                &policy,
                &mut scratch,
                None,
                &mut stats,
            );
            assert!(outcome.fallback);
            assert_eq!(stats.budget_fallbacks, 1);
            assert_eq!(
                scratch.amounts.iter().copied().sum::<LayoutUnits>(),
                px(13.0)
            );
            runs.push(scratch.amounts);
        }
        assert!(runs.windows(2).all(|pair| pair[0] == pair[1]));
    }

    struct Refine(usize);

    impl ParticipantSource for Refine {
        fn expand(&mut self, _: usize) -> Option<EnvelopeShape> {
            self.0 += 1;
            Some(EnvelopeShape::from_segments([
                AdjustmentSegment::new(
                    px(2.0),
                    LayoutCost::Finite(5),
                    AdjustmentKind::TextSpacing,
                    LocalReflow,
                ),
                AdjustmentSegment::new(
                    px(2.0),
                    LayoutCost::Finite(90),
                    AdjustmentKind::TextSpacing,
                    LocalReflow,
                ),
            ]))
        }
    }

    /// Coarse reflow capacity is refined only when a solve needs reflow, and
    /// no more than the budget allows.
    #[test]
    fn coarse_capacity_is_refined_only_when_needed_and_within_budget() {
        let coarse = {
            let mut segment = AdjustmentSegment::new(
                px(4.0),
                LayoutCost::Finite(50),
                AdjustmentKind::TextSpacing,
                LocalReflow,
            );
            segment.coarse = true;
            EnvelopeShape::from_segments([
                AdjustmentSegment::new(
                    px(1.0),
                    LayoutCost::Finite(10),
                    AdjustmentKind::Gap,
                    PlacementOnly,
                ),
                segment,
            ])
        };
        let shapes = vec![coarse; 50];
        let participants: Vec<_> = shapes
            .iter()
            .map(|shape| Participant::new(shape, px(1e6)))
            .collect();
        let policy = SolverPolicy::default();
        let mut source = Refine(0);
        let mut scratch = SolverScratch::default();
        let mut stats = DynamicLayoutCounters::default();
        solve_deficit(
            &participants,
            px(10.0),
            &policy,
            &mut scratch,
            Some(&mut source),
            &mut stats,
        );
        assert_eq!(source.0, 0, "a placement-only deficit refined something");
        solve_deficit(
            &participants,
            px(80.0),
            &policy,
            &mut scratch,
            Some(&mut source),
            &mut stats,
        );
        assert_eq!(source.0, policy.budget.max_deep_expansions);
        assert_eq!(stats.deep_expansions, policy.budget.max_deep_expansions);
    }

    #[test]
    fn a_discrete_candidate_wins_only_by_cost_and_a_lower_bound_prunes() {
        let shape = shape(&[(10.0, 100, LocalBox)]);
        let cheap = [DiscreteCandidate {
            saves_px: 10.0,
            cost: LayoutCost::Finite(5),
        }];
        let dear = [DiscreteCandidate {
            saves_px: 10.0,
            cost: LayoutCost::Finite(1_000_000),
        }];
        let policy = SolverPolicy {
            allow_structural: true,
            ..SolverPolicy::default()
        };
        for (candidates, wins) in [(&cheap[..], true), (&dear[..], false)] {
            let participants = [Participant {
                discrete: candidates,
                ..Participant::new(&shape, px(1e6))
            }];
            let mut scratch = SolverScratch::default();
            let mut stats = DynamicLayoutCounters::default();
            let outcome = solve_deficit(
                &participants,
                px(8.0),
                &policy,
                &mut scratch,
                None,
                &mut stats,
            );
            assert_eq!(outcome.discrete.is_some(), wins);
            assert_eq!(stats.candidates_pruned, usize::from(!wins));
        }
    }

    /// A thousand participants cost a solve a thousand segment reads, not a
    /// million: work follows the direct opportunities.
    #[test]
    fn a_solve_reads_each_opportunity_once() {
        for count in [1_000usize, 2_000] {
            let shapes = vec![shape(&[(1.0, 10, LocalBox), (5.0, 50, LocalReflow)]); count];
            let (_, _, stats) = solve(&shapes, count as f32 / 2.0);
            assert_eq!(stats.cost_segments_visited, count);
        }
    }
}
