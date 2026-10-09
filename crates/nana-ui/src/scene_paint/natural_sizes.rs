//! Natural sizes of the `url(...)` images replaced boxes show, on their way
//! back from the painter to the document (Issue #263).
//!
//! Only the painter learns what size an image decoded at. While it prepares
//! a target it notes, for each quad it draws with a content image of known
//! size, that size. After presenting, the host takes what is new for the
//! target ([`super::SceneWgpuPainter::take_image_natural_sizes`]) and commits
//! it to the document it painted ([`commit_image_natural_sizes`]); the boxes
//! that read a natural size lay out on the next frame. The painter only
//! writes its own notes and never reaches the document.
//!
//! A size is handed over when a quad starts showing an image, when the image
//! decodes at another size, and on every prepare that draws it into a box
//! with no area: such a box may be waiting for exactly this size, in a
//! document that does not have it. A quad no longer drawn is forgotten.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use nana_ui_core::BackgroundImage;
use nana_ui_runtime::{
    AppContext, FrameworkError, MutationQueue, ReplacedMetadata, ReplacedResource,
};
use nana_ui_scene::{PrimitiveId, QuadSurfacePaint};

use super::url_texture_cache::UrlTextureCache;

/// The size a `url(...)` image decoded at, in image pixels: what a replaced
/// box showing it takes, in CSS pixels, where nothing else sizes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageNaturalSize {
    pub url: Arc<str>,
    pub width: u32,
    pub height: u32,
}

/// Commit natural sizes a painter handed over to the document painted with
/// them, as [`MutationQueue::set_replaced_metadata`]. Only an image a node of
/// the document shows, at a size the document does not have yet, is
/// committed. Returns whether anything was: the boxes that read it lay out on
/// the next frame, so the host paints one.
pub fn commit_image_natural_sizes(
    context: &mut AppContext,
    sizes: &[ImageNaturalSize],
) -> Result<bool, FrameworkError> {
    let world = context.world();
    let mut queue = MutationQueue::new();
    let mut queued = HashSet::new();
    for size in sizes {
        let resource = ReplacedResource::Url(Arc::clone(&size.url));
        let metadata = ReplacedMetadata::new(size.width as f32, size.height as f32);
        if !world.shows_replaced(&resource)
            || world.replaced_metadata(&resource) == Some(metadata)
            || !queued.insert(Arc::clone(&size.url))
        {
            continue;
        }
        queue.set_replaced_metadata(resource, Some(metadata));
    }
    if queue.is_empty() {
        return Ok(false);
    }
    context.commit_mutations(queue)?;
    Ok(true)
}

/// One target's notes: for each quad its last fresh prepare drew with a
/// content image of known size, that image and size, and whether the host
/// took them.
#[derive(Default)]
pub(crate) struct NaturalSizes {
    seen: HashMap<PrimitiveId, Seen>,
    pass: u64,
    /// Entries of `seen` the host has not taken.
    untaken: usize,
}

struct Seen {
    url: Arc<str>,
    size: [u32; 2],
    pass: u64,
    taken: bool,
}

impl NaturalSizes {
    /// A fresh prepare of the target begins.
    pub(super) fn begin_pass(&mut self) {
        self.pass = self.pass.wrapping_add(1);
    }

    /// Quad `id` is drawn into `bounds`: note the natural size of the content
    /// image it shows, once that is known.
    pub(super) fn note_quad(
        &mut self,
        cache: &UrlTextureCache,
        id: PrimitiveId,
        bounds: super::clip::LogicalRect,
        surface: &QuadSurfacePaint,
    ) {
        if let Some(BackgroundImage::Url { url, sampling, .. }) = surface.content_image.as_ref()
            && let Some(size) = cache.natural_size(url, *sampling)
        {
            self.note(id, url, size, bounds.width <= 0.0 || bounds.height <= 0.0);
        }
    }

    fn note(&mut self, id: PrimitiveId, url: &str, size: [u32; 2], waiting: bool) {
        let pass = self.pass;
        let Some(seen) = self.seen.get_mut(&id) else {
            self.seen.insert(
                id,
                Seen {
                    url: Arc::from(url),
                    size,
                    pass,
                    taken: false,
                },
            );
            self.untaken += 1;
            return;
        };
        seen.pass = pass;
        let moved = *seen.url != *url || seen.size != size;
        if moved {
            if *seen.url != *url {
                seen.url = Arc::from(url);
            }
            seen.size = size;
        }
        if (moved || waiting) && seen.taken {
            seen.taken = false;
            self.untaken += 1;
        }
    }

    /// The fresh prepare ended: forget the quads it did not draw.
    pub(super) fn end_pass(&mut self) {
        let pass = self.pass;
        let mut dropped = 0;
        self.seen.retain(|_, seen| {
            let kept = seen.pass == pass;
            if !kept && !seen.taken {
                dropped += 1;
            }
            kept
        });
        self.untaken -= dropped;
    }

    /// What the host has not taken yet. Nothing to take costs nothing.
    pub(super) fn take(&mut self) -> Vec<ImageNaturalSize> {
        if self.untaken == 0 {
            return Vec::new();
        }
        self.untaken = 0;
        self.seen
            .values_mut()
            .filter(|seen| !seen.taken)
            .map(|seen| {
                seen.taken = true;
                ImageNaturalSize {
                    url: Arc::clone(&seen.url),
                    width: seen.size[0],
                    height: seen.size[1],
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(node: u64) -> PrimitiveId {
        PrimitiveId {
            node: nana_ui_runtime::StableNodeId::new(node).unwrap(),
            slot: 0,
        }
    }

    fn pass(sizes: &mut NaturalSizes, notes: &[(u64, &str, [u32; 2], bool)]) {
        sizes.begin_pass();
        for &(node, url, size, waiting) in notes {
            sizes.note(quad(node), url, size, waiting);
        }
        sizes.end_pass();
    }

    fn taken(sizes: &mut NaturalSizes) -> Vec<(String, [u32; 2])> {
        let mut taken: Vec<_> = sizes
            .take()
            .into_iter()
            .map(|size| (size.url.to_string(), [size.width, size.height]))
            .collect();
        taken.sort();
        taken
    }

    #[test]
    fn a_size_is_handed_over_once_until_it_changes() {
        let mut sizes = NaturalSizes::default();
        pass(&mut sizes, &[(1, "a.png", [40, 30], false)]);
        assert_eq!(taken(&mut sizes), [("a.png".into(), [40, 30])]);
        for _ in 0..3 {
            pass(&mut sizes, &[(1, "a.png", [40, 30], false)]);
            assert!(taken(&mut sizes).is_empty(), "drawn again, nothing new");
        }
        pass(&mut sizes, &[(1, "a.png", [80, 60], false)]);
        assert_eq!(taken(&mut sizes), [("a.png".into(), [80, 60])]);
        pass(&mut sizes, &[(1, "b.png", [80, 60], false)]);
        assert_eq!(
            taken(&mut sizes),
            [("b.png".into(), [80, 60])],
            "another image on the same quad"
        );
    }

    #[test]
    fn a_box_with_no_area_hands_its_size_over_on_every_prepare() {
        let mut sizes = NaturalSizes::default();
        for _ in 0..3 {
            pass(&mut sizes, &[(1, "a.png", [40, 30], true)]);
            assert_eq!(taken(&mut sizes), [("a.png".into(), [40, 30])]);
        }
        pass(&mut sizes, &[(1, "a.png", [40, 30], false)]);
        assert!(taken(&mut sizes).is_empty(), "it has its size");
    }

    #[test]
    fn a_quad_no_longer_drawn_is_forgotten() {
        let mut sizes = NaturalSizes::default();
        pass(
            &mut sizes,
            &[(1, "a.png", [40, 30], false), (2, "a.png", [40, 30], false)],
        );
        assert_eq!(taken(&mut sizes).len(), 2);
        pass(&mut sizes, &[(2, "a.png", [40, 30], false)]);
        assert!(taken(&mut sizes).is_empty());
        assert_eq!(sizes.seen.len(), 1);
        // Drawn again, as a quad first drawn: handed over again.
        pass(
            &mut sizes,
            &[(1, "a.png", [40, 30], false), (2, "a.png", [40, 30], false)],
        );
        assert_eq!(taken(&mut sizes), [("a.png".into(), [40, 30])]);
    }

    #[test]
    fn notes_the_host_never_takes_stay_bounded_by_what_is_drawn() {
        let mut sizes = NaturalSizes::default();
        for index in 0..1_000 {
            pass(&mut sizes, &[(1, &format!("{index}.png"), [4, 4], false)]);
        }
        assert_eq!(sizes.seen.len(), 1);
        assert_eq!(sizes.untaken, 1);
        assert_eq!(taken(&mut sizes), [("999.png".into(), [4, 4])]);
        // A quad dropped before the host took it leaves nothing behind.
        pass(&mut sizes, &[(2, "x.png", [4, 4], false)]);
        pass(&mut sizes, &[]);
        assert_eq!(sizes.untaken, 0);
        assert!(taken(&mut sizes).is_empty());
    }
}
