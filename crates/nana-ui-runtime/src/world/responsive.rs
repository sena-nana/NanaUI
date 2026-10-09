//! Responsive rules (Issue #265): which node follows which container, and
//! the bucket each rule is in.
//!
//! Rules are indexed by their container when they are set. A layout
//! writeback that resized a container, or a padding that moved its content
//! box, queues it; at the end of that commit the queued containers evaluate
//! their rules and only theirs, on the axis that moved. A rule that changes
//! bucket re-resolves its node's layout and seeds what that moved; one that
//! stays changes nothing.
//!
//! Variants can resize the container they read. Within one frame a rule that
//! would return to a bucket it already left, or a frame past
//! [`MAX_ROUNDS`] rounds of changes, keeps the bucket it has: the frame
//! settles, and the next resize evaluates afresh.

use super::*;
use crate::responsive::{LayoutVariant, ResponsiveAxis, ResponsiveContainer, ResponsiveRule};

/// Rounds of bucket changes one frame converges in before its rules hold.
const MAX_ROUNDS: usize = 4;

#[derive(Debug)]
struct Dependent {
    rule: Arc<ResponsiveRule>,
    /// The box the rule reads, while there is one.
    container: Option<StableNodeId>,
    /// The bucket the node is in; `None` until its container was measured.
    bucket: Option<usize>,
}

/// A container's content box, as its rules last read it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Extents {
    inline: f32,
    block: f32,
}

impl Extents {
    fn on(self, axis: ResponsiveAxis) -> f32 {
        match axis {
            ResponsiveAxis::Inline => self.inline,
            ResponsiveAxis::Block => self.block,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ResponsiveIndex {
    dependents: HashMap<StableNodeId, Dependent, BuildIdHasher>,
    /// Container to the nodes whose rules read it.
    containers: HashMap<StableNodeId, Vec<StableNodeId>, BuildIdHasher>,
    /// What each container's rules last read.
    read: HashMap<StableNodeId, Extents, BuildIdHasher>,
    /// Containers to evaluate at the end of the commit.
    pending: Vec<StableNodeId>,
    /// This frame's rounds of changes, and the buckets each node left.
    rounds: usize,
    left: HashMap<StableNodeId, Vec<usize>, BuildIdHasher>,
}

impl ResponsiveIndex {
    pub(super) fn follows(&self, id: StableNodeId) -> bool {
        self.dependents.contains_key(&id)
    }

    fn queue(&mut self, container: StableNodeId) {
        if !self.pending.contains(&container) {
            self.pending.push(container);
        }
    }

    fn link(&mut self, id: StableNodeId, container: Option<StableNodeId>) {
        if let Some(container) = container {
            let dependents = self.containers.entry(container).or_default();
            if !dependents.contains(&id) {
                dependents.push(id);
            }
        }
    }

    fn unlink(&mut self, id: StableNodeId, container: Option<StableNodeId>) {
        let Some(container) = container else {
            return;
        };
        if let Some(dependents) = self.containers.get_mut(&container) {
            dependents.retain(|dependent| *dependent != id);
            if dependents.is_empty() {
                self.containers.remove(&container);
                self.read.remove(&container);
            }
        }
    }

    /// Entries held: rules, container links, read extents and this frame's
    /// history. It follows the rules registered, not frames.
    #[cfg(test)]
    pub(super) fn footprint(&self) -> (usize, usize, usize, usize) {
        (
            self.dependents.len(),
            self.containers.values().map(Vec::len).sum(),
            self.read.len(),
            self.left.values().map(Vec::len).sum(),
        )
    }
}

/// What one evaluation cost.
#[derive(Debug, Default, Clone, Copy)]
struct Evaluation {
    size_changes: usize,
    evaluated: usize,
    changed: usize,
    unchanged: usize,
    downstream: usize,
    fallbacks: usize,
}

impl UiWorld {
    /// The rule `id` follows, if it follows one.
    pub fn responsive_rule(&self, id: StableNodeId) -> Option<&Arc<ResponsiveRule>> {
        self.responsive
            .dependents
            .get(&id)
            .map(|dependent| &dependent.rule)
    }

    /// The bucket `id`'s rule is in, once its container was measured.
    pub fn responsive_bucket(&self, id: StableNodeId) -> Option<usize> {
        self.responsive
            .dependents
            .get(&id)
            .and_then(|dependent| dependent.bucket)
    }

    /// The variant `id`'s resolved layout takes, if its bucket has one.
    pub(super) fn responsive_variant(&self, id: StableNodeId) -> Option<&LayoutVariant> {
        let dependent = self.responsive.dependents.get(&id)?;
        dependent.rule.variant(dependent.bucket?)
    }

    /// The box `rule` on `id` reads.
    fn responsive_container_of(
        &self,
        id: StableNodeId,
        rule: &ResponsiveRule,
    ) -> Option<StableNodeId> {
        match rule.container() {
            ResponsiveContainer::Parent => self.parent_id(id),
            ResponsiveContainer::Node(container) => self.contains(container).then_some(container),
        }
    }

    /// Set or clear the rule `id` follows. A container already measured is
    /// read now, so the node's first layout is in its bucket; one not yet
    /// measured is read after its first layout.
    pub(super) fn set_responsive_rule(
        &mut self,
        id: StableNodeId,
        rule: Option<Arc<ResponsiveRule>>,
    ) {
        let before = self.responsive_variant(id).cloned();
        if let Some(previous) = self.responsive.dependents.remove(&id) {
            self.responsive.unlink(id, previous.container);
        }
        if let Some(rule) = rule {
            let container = self.responsive_container_of(id, &rule);
            self.responsive.link(id, container);
            let extents = container.and_then(|container| self.responsive_extents(container));
            if let (Some(container), None) = (container, extents) {
                self.responsive.queue(container);
            }
            let bucket = extents.map(|extents| rule.bucket_for(extents.on(rule.axis())));
            self.responsive.dependents.insert(
                id,
                Dependent {
                    rule,
                    container,
                    bucket,
                },
            );
        }
        if before != self.responsive_variant(id).cloned() {
            self.apply_responsive_variant(id);
        }
    }

    /// `id` left the tree or moved under another parent: a rule that reads
    /// its parent reads the new one.
    pub(super) fn responsive_reparented(&mut self, id: StableNodeId) {
        let Some(dependent) = self.responsive.dependents.get(&id) else {
            return;
        };
        if dependent.rule.container() != ResponsiveContainer::Parent {
            return;
        }
        let rule = Arc::clone(&dependent.rule);
        self.set_responsive_rule(id, Some(rule));
    }

    /// Forget a node that is gone: as a follower and as a container. The
    /// nodes that read it keep the bucket they were in.
    pub(super) fn forget_responsive(&mut self, id: StableNodeId) {
        if let Some(dependent) = self.responsive.dependents.remove(&id) {
            self.responsive.unlink(id, dependent.container);
            self.responsive.left.remove(&id);
        }
        if let Some(followers) = self.responsive.containers.remove(&id) {
            for follower in followers {
                if let Some(dependent) = self.responsive.dependents.get_mut(&follower) {
                    dependent.container = None;
                }
            }
        }
        self.responsive.read.remove(&id);
        self.responsive.pending.retain(|container| *container != id);
    }

    /// A container's box or padding moved: read it at the end of the commit.
    pub(super) fn note_responsive_resize(&mut self, id: StableNodeId) {
        if self.responsive.containers.contains_key(&id) {
            self.responsive.queue(id);
        }
    }

    /// The frame starts over: what last frame's rules left is not this one's.
    pub(super) fn begin_responsive_frame(&mut self) {
        self.responsive.rounds = 0;
        self.responsive.left.clear();
    }

    /// The content box of `container` as its rules read it: its border box
    /// less padding and border, inline and block in its writing mode.
    fn responsive_extents(&self, container: StableNodeId) -> Option<Extents> {
        let record = self.nodes.get(container)?;
        let bounds = record.layout;
        if bounds.width <= 0.0 && bounds.height <= 0.0 {
            return None;
        }
        let padding = self.used_layout_padding(container);
        let border = record.resolved_layout.resolved_border_edges();
        let width =
            (bounds.width - padding.left - padding.right - border.left - border.right).max(0.0);
        let height =
            (bounds.height - padding.top - padding.bottom - border.top - border.bottom).max(0.0);
        let (inline, block) = if record.resolved.0.writing_mode.is_vertical() {
            (height, width)
        } else {
            (width, height)
        };
        Some(Extents { inline, block })
    }

    /// Evaluate the rules of the containers this commit resized. Each rule
    /// reads its own axis; a rule whose axis held is not evaluated.
    pub(super) fn evaluate_responsive(&mut self) {
        if self.responsive.pending.is_empty() {
            return;
        }
        let mut containers = std::mem::take(&mut self.responsive.pending);
        containers.sort_unstable();
        let mut cost = Evaluation::default();
        for container in containers {
            let Some(extents) = self.responsive_extents(container) else {
                continue;
            };
            let read = self.responsive.read.insert(container, extents);
            let inline_moved = read.is_none_or(|read| read.inline != extents.inline);
            let block_moved = read.is_none_or(|read| read.block != extents.block);
            cost.size_changes += usize::from(inline_moved) + usize::from(block_moved);
            if !inline_moved && !block_moved {
                continue;
            }
            let mut followers = self
                .responsive
                .containers
                .get(&container)
                .cloned()
                .unwrap_or_default();
            followers.sort_unstable();
            for id in followers {
                let Some(dependent) = self.responsive.dependents.get(&id) else {
                    continue;
                };
                let axis = dependent.rule.axis();
                if read.is_some_and(|read| read.on(axis) == extents.on(axis)) {
                    continue;
                }
                cost.evaluated += 1;
                let bucket = dependent.rule.bucket_for(extents.on(axis));
                let current = dependent.bucket;
                if current == Some(bucket) {
                    cost.unchanged += 1;
                    continue;
                }
                // A bucket the node already left this frame, or a frame
                // that has run out of rounds: hold, and let it settle.
                let returning = self
                    .responsive
                    .left
                    .get(&id)
                    .is_some_and(|left| left.contains(&bucket));
                if returning || self.responsive.rounds >= MAX_ROUNDS {
                    cost.unchanged += 1;
                    cost.fallbacks += 1;
                    continue;
                }
                if let Some(current) = current {
                    self.responsive.left.entry(id).or_default().push(current);
                }
                let before = self.responsive_variant(id).cloned();
                if let Some(dependent) = self.responsive.dependents.get_mut(&id) {
                    dependent.bucket = Some(bucket);
                }
                cost.changed += 1;
                if before != self.responsive_variant(id).cloned() {
                    cost.downstream += self.apply_responsive_variant(id);
                }
            }
        }
        let round = usize::from(cost.changed > 0);
        self.responsive.rounds += round;
        self.bump_last_counters(|counters| {
            counters.record_container_query_evaluation(
                cost.size_changes,
                cost.evaluated,
                cost.changed,
                cost.unchanged,
            );
            counters.record_container_query_results(cost.downstream, round, cost.fallbacks);
        });
    }

    /// `id`'s variant changed: restyle it as a style write that moved its
    /// layout the same way would. Returns the layout seeds queued.
    fn apply_responsive_variant(&mut self, id: StableNodeId) -> usize {
        let seeds_before = self.layout_seeds_created;
        self.restyle(id, None);
        (self.layout_seeds_created - seeds_before) as usize
    }
}
