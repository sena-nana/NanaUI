//! Replaced content (Issue #263): an image or a host texture takes its
//! resource's natural size once the resource reports one.
//!
//! Three revisions of a replaced node stay apart. Its content -- pixels, a
//! video frame, a texture generation -- is paint. Its fit and sampling are
//! paint. Only its intrinsic metadata, the natural size a resource reports,
//! reaches layout, and only for the nodes that show that resource and read
//! the size: an index from resource to node finds them, so a resource shown
//! by a hundred nodes in a document of a hundred thousand visits a hundred.

use super::*;

/// The resource a replaced node shows: an image by URL, or what a custom
/// renderer draws (a host texture slot, a video).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReplacedResource {
    Url(Arc<str>),
    Render {
        renderer: Arc<str>,
        resource: Arc<str>,
    },
}

/// What a resource says about its own size once it is known: its natural
/// width and height in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplacedMetadata {
    pub width: f32,
    pub height: f32,
}

impl ReplacedMetadata {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub(super) fn is_valid(self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width >= 0.0 && self.height >= 0.0
    }
}

/// Which node shows which resource, both ways, and what each resource
/// reported.
#[derive(Debug, Default)]
pub(super) struct ReplacedIndex {
    by_node: HashMap<StableNodeId, ReplacedResource, BuildIdHasher>,
    dependents: HashMap<ReplacedResource, HashSet<StableNodeId, BuildIdHasher>>,
    metadata: HashMap<ReplacedResource, ReplacedMetadata>,
}

impl ReplacedIndex {
    fn set(&mut self, id: StableNodeId, resource: Option<ReplacedResource>) {
        let previous = match resource.clone() {
            Some(resource) => self.by_node.insert(id, resource),
            None => self.by_node.remove(&id),
        };
        if let Some(previous) = previous
            && let Some(nodes) = self.dependents.get_mut(&previous)
        {
            nodes.remove(&id);
            if nodes.is_empty() {
                self.dependents.remove(&previous);
            }
        }
        if let Some(resource) = resource {
            self.dependents.entry(resource).or_default().insert(id);
        }
    }

    fn natural(&self, id: StableNodeId) -> Option<ReplacedMetadata> {
        self.by_node
            .get(&id)
            .and_then(|resource| self.metadata.get(resource))
            .copied()
    }
}

impl UiWorld {
    /// The resource `id` shows: what its custom renderer draws, else the
    /// image its style replaces it with.
    fn replaced_resource_of(&self, id: StableNodeId) -> Option<ReplacedResource> {
        if let Some(content) = self.nodes.custom_render(id) {
            return Some(ReplacedResource::Render {
                renderer: Arc::clone(&content.renderer),
                resource: Arc::clone(&content.resource),
            });
        }
        match self
            .nodes
            .get(id)?
            .resolved_layout
            .paint
            .content_image
            .as_ref()?
        {
            nana_ui_core::BackgroundImage::Url { url, .. } => {
                Some(ReplacedResource::Url(Arc::from(url.as_str())))
            }
            nana_ui_core::BackgroundImage::Gradient(_) => None,
        }
    }

    /// Index `id` under the resource it shows now. When that moved its
    /// natural size, or whether it is replaced at all, and its box reads
    /// either, seed it.
    pub(super) fn reindex_replaced(&mut self, id: StableNodeId) {
        let before = self.replaced.by_node.get(&id).cloned();
        let natural_before = self.replaced.natural(id);
        let now = self.replaced_resource_of(id);
        if before == now {
            return;
        }
        let replaced_toggled = before.is_some() != now.is_some();
        self.replaced.set(id, now);
        // Replaced or not, the box aligns by a different baseline.
        if replaced_toggled
            || (natural_before != self.replaced.natural(id) && self.reads_natural_size(id))
        {
            self.seed_natural_size(id);
        }
    }

    /// Forget a node that is gone.
    pub(super) fn forget_replaced(&mut self, id: StableNodeId) {
        self.replaced.set(id, None);
    }

    /// What `resource` reported about its size, if it did.
    pub fn replaced_metadata(&self, resource: &ReplacedResource) -> Option<ReplacedMetadata> {
        self.replaced.metadata.get(resource).copied()
    }

    /// The natural size of what `id` shows, once its resource reported one.
    pub(crate) fn replaced_natural_size(&self, id: StableNodeId) -> Option<ReplacedMetadata> {
        self.replaced.natural(id)
    }

    /// `resource` reported what it knows of its size. The nodes that show it
    /// paint again; of them, the ones whose box reads a natural size lay out
    /// again. A size equal to the last one is nothing.
    pub(super) fn apply_replaced_metadata(
        &mut self,
        resource: &ReplacedResource,
        metadata: Option<ReplacedMetadata>,
    ) {
        let previous = match metadata {
            Some(metadata) => self.replaced.metadata.insert(resource.clone(), metadata),
            None => self.replaced.metadata.remove(resource),
        };
        if previous == metadata {
            return;
        }
        self.pending_drain_counts
            .replaced_intrinsic_metadata_updates += 1;
        let mut dependents: Vec<StableNodeId> = self
            .replaced
            .dependents
            .get(resource)
            .map(|nodes| nodes.iter().copied().collect())
            .unwrap_or_default();
        dependents.sort_unstable();
        self.pending_drain_counts
            .resource_intrinsic_dependents_notified += dependents.len();
        for id in dependents {
            let _ = self.mark(id, DirtyMask::RENDER);
            if self.reads_natural_size(id) {
                self.seed_natural_size(id);
            }
        }
    }

    /// Whether `id`'s box takes any of its size from what it shows: unless
    /// both its width and height are its own lengths, a natural size fills
    /// the gap.
    fn reads_natural_size(&self, id: StableNodeId) -> bool {
        let Some(record) = self.nodes.get(id) else {
            return false;
        };
        let style = record.resolved_layout.as_ref();
        let own = |size: Option<LengthSpec>| {
            matches!(
                size,
                Some(
                    LengthSpec::Px(_)
                        | LengthSpec::Em(_)
                        | LengthSpec::Rem(_)
                        | LengthSpec::Percent(_)
                        | LengthSpec::Fill
                )
            )
        };
        !(own(style.width) && own(style.height))
    }

    /// The natural size `id` shows moved: it measures again and exports its
    /// new size and baseline, as text whose metrics moved does.
    fn seed_natural_size(&mut self, id: StableNodeId) {
        let seeds_before = self.layout_seeds_created;
        self.record_layout_invalidation(
            id,
            LayoutInvalidation::new(
                LayoutInvalidationSource::Resource,
                InvalidationReason::RESOURCE,
                InvalidationKind::MEASURE.union(InvalidationKind::PLACEMENT),
                LayoutFieldMask::INTRINSIC,
                METRIC_EXPORTS,
            ),
        );
        self.pending_drain_counts.replaced_layout_seeds +=
            (self.layout_seeds_created - seeds_before) as usize;
    }
}
