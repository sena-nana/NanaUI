//! Content declared in one place and shown under another node (Vue
//! `<Teleport>`): a dialog written next to the button that opens it, laid
//! out in an overlay layer.
//!
//! ```ignore
//! let layer = node_ref();
//! column(0.0, (
//!     widget(Stack::column(0.0)).node_ref(layer),
//!     teleport(layer, text("在浮层里")),
//! ))
//! ```
//!
//! The content belongs to where it is declared: it is built there, its
//! scope and bindings are that place's, and it is despawned with it. Only
//! its parent in the tree is the target (`place_assembled`), so layout,
//! paint, hit testing, focus order and accessibility follow the target.
//! While the target is `None` (or gone) the content stays in place.

use std::borrow::Cow;
use std::panic::Location;

use super::node::{IntoView, StructuralBinding, UNBUILT, ViewBuilder};
use super::prop::{IntoProp, PropSource};
use super::reactive::{self, EffectKey, EffectTarget, on_mount};
use crate::{AppContext, FrameworkError, StableNodeId, Stack};

/// See [`teleport`].
pub struct Teleport<V> {
    to: PropSource<Option<StableNodeId>>,
    content: V,
    key: Option<Cow<'static, str>>,
    site: &'static Location<'static>,
}

/// Show `content` under `to` (a [`super::NodeRef`], a node id, or a
/// closure choosing one), following it when it changes.
#[track_caller]
pub fn teleport<V: IntoView>(to: impl IntoProp<Option<StableNodeId>>, content: V) -> Teleport<V> {
    Teleport {
        to: to.into_source(),
        content,
        key: None,
        site: Location::caller(),
    }
}

impl<V> Teleport<V> {
    pub fn key(mut self, key: impl Into<Cow<'static, str>>) -> Self {
        self.key = Some(key.into());
        self
    }
}

/// Put `roots` under `to` when it exists, else back under `anchor`.
fn place(
    cx: &mut AppContext,
    anchor: StableNodeId,
    roots: &[StableNodeId],
    to: Option<StableNodeId>,
) -> Result<(), FrameworkError> {
    let parent = to
        .filter(|target| cx.world().contains(*target))
        .unwrap_or(anchor);
    for &root in roots {
        if cx.world().contains(root) && cx.world().parent_id(root) != Some(parent) {
            cx.place_assembled(root, parent)?;
        }
    }
    Ok(())
}

struct TeleportBinding {
    to: PropSource<Option<StableNodeId>>,
    roots: Vec<StableNodeId>,
}

impl StructuralBinding for TeleportBinding {
    fn update(
        &mut self,
        cx: &mut AppContext,
        anchor: StableNodeId,
        effect: EffectKey,
    ) -> Result<(), FrameworkError> {
        let to = reactive::run_tracked(effect, || self.to.get());
        place(cx, anchor, &self.roots, to)
    }
}

impl<V: IntoView> IntoView for Teleport<V> {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let anchor = vb.place(self.key, Stack::column(0.0));
        let id = anchor.stable_id();
        if id == UNBUILT {
            return;
        }
        let mut roots = Vec::new();
        let content = self.content;
        vb.nest(anchor, |vb| roots = vb.build_collect(content));
        let effect =
            reactive::create_effect(vb.st.tag, EffectTarget::Structural(id), None, self.site);
        let to = self.to;
        let first = reactive::run_tracked(effect, || to.get());
        let placed = roots.clone();
        // The target may be declared after this in the same view: place
        // once everything is in the tree.
        on_mount(move |cx| {
            let _ = place(cx, id, &placed, first);
        });
        vb.st
            .parts
            .structural
            .push((id, effect, Box::new(TeleportBinding { to, roots })));
    }
}
