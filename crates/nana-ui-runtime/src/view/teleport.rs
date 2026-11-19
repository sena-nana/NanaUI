//! Content declared in one place and shown under another node (Vue
//! `<Teleport>`): a dialog written next to the button that opens it, laid
//! out in an overlay layer.
//!
//! ```ignore
//! let layer = node_ref();
//! column().children((
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
//!
//! The anchor — the node the content is built in, and stays in while it has
//! no target — takes `.class`, `.class_when` and `.css` like the container of
//! any structural view, so it can fill its place (`height: 100%`) or keep out
//! of it.

use std::borrow::Cow;
use std::panic::Location;

use super::node::{FragmentBinding, FragmentDecl, IntoView, SlotItem, ViewBuilder};
use super::prop::{IntoProp, PropSource};
use super::reactive::{self, EffectKey, EffectTarget, FragmentId, on_mount};
use super::structural::container;
use super::style::{ContainerStyle, container_styles};
use crate::{AppContext, FrameworkError, StableNodeId, Stack};

/// See [`teleport`].
pub struct Teleport<V> {
    to: PropSource<Option<StableNodeId>>,
    content: V,
    key: Option<Cow<'static, str>>,
    style: ContainerStyle,
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
        style: ContainerStyle::default(),
        site: Location::caller(),
    }
}

impl<V> Teleport<V> {
    pub fn key(mut self, key: impl Into<Cow<'static, str>>) -> Self {
        self.key = Some(key.into());
        self
    }

    fn container_style_mut(&mut self) -> &mut ContainerStyle {
        &mut self.style
    }
}

container_styles!([V] Teleport<V>);

struct TeleportBinding {
    to: PropSource<Option<StableNodeId>>,
    items: Vec<SlotItem>,
    placed_target: Option<StableNodeId>,
}

impl FragmentBinding for TeleportBinding {
    fn update(
        &mut self,
        cx: &mut AppContext,
        frag: FragmentId,
        effect: EffectKey,
    ) -> Result<(), FrameworkError> {
        let Some(host) = cx.fragment_host(frag) else {
            return Ok(());
        };
        let to = reactive::run_tracked(effect, || self.to.get());
        let prev_container = self.placed_target.unwrap_or(host);
        let mut nodes = Vec::new();
        cx.flatten_slot_items(&self.items, prev_container, &mut nodes);
        let actual_target = if self.placed_target.is_none()
            && to
                .filter(|t| *t != host && cx.world().contains(*t))
                .is_none()
        {
            // Already at home inside `host`: only verify when valid target exists.
            None
        } else {
            cx.teleport_place_nodes(frag, &nodes, to)?
        };
        let new_container = actual_target.unwrap_or(host);
        if new_container != prev_container {
            cx.retarget_slot_items(&self.items, prev_container, new_container);
        }
        self.placed_target = actual_target;
        cx.mark_out_of_host_fragment(frag, actual_target.is_some());
        Ok(())
    }

    fn collect_external_nodes(&self, cx: &AppContext, out: &mut Vec<StableNodeId>) {
        if let Some(target) = self.placed_target {
            cx.flatten_slot_items(&self.items, target, out);
        }
    }

    fn placed_target(&self) -> Option<StableNodeId> {
        self.placed_target
    }

    fn clear_placed_target(&mut self) {
        self.placed_target = None;
    }

    fn teardown(
        &mut self,
        cx: &mut AppContext,
        visible_nodes: &mut Vec<StableNodeId>,
        immediate_despawn: &mut Vec<StableNodeId>,
    ) {
        if self.placed_target.is_some() {
            let mut placed = Vec::new();
            cx.teardown_slot_items(&self.items, &mut placed, immediate_despawn);
            immediate_despawn.extend(placed);
        } else {
            cx.teardown_slot_items(&self.items, visible_nodes, immediate_despawn);
        }
    }

    fn forget(&mut self, cx: &mut AppContext) {
        for item in &self.items {
            if let SlotItem::Fragment(frag) = *item {
                cx.forget_fragment(frag);
            }
        }
    }
}

impl<V: IntoView> IntoView for Teleport<V> {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let boxed = self.key.is_some() || !self.style.is_empty() || vb.fragment_host().is_none();
        let (host, parent_fragment) = if boxed {
            let Some(anchor) = container(
                Stack::column(0.0).structural(),
                self.key,
                self.style,
                self.site,
            )
            .place(vb) else {
                return;
            };
            (anchor.stable_id(), None)
        } else if let Some(host) = vb.fragment_host() {
            (host, vb.parent_fragment())
        } else {
            return;
        };
        let frag = FragmentId::next();
        let effect =
            reactive::create_effect(vb.st.tag, EffectTarget::Fragment(frag), None, self.site);
        let mut items = Vec::new();
        let mut content = Some(self.content);
        if boxed {
            vb.nest_dyn(host, &mut |vb| {
                vb.record_fragment(frag);
                items = vb.build_fragment_slots(host, frag, &mut |vb| {
                    if let Some(content) = content.take() {
                        vb.build_collect_slots(content);
                    }
                });
            });
        } else {
            vb.record_fragment(frag);
            items = vb.build_fragment_slots(host, frag, &mut |vb| {
                if let Some(content) = content.take() {
                    vb.build_collect_slots(content);
                }
            });
        }
        let binding = TeleportBinding {
            to: self.to,
            items: items.clone(),
            placed_target: None,
        };
        // The target may be declared after this in the same view: place once
        // everything is in the tree.
        on_mount(move |cx| {
            let _ = cx.run_fragment_now(frag);
        });
        vb.st.parts.fragments.push(FragmentDecl {
            id: frag,
            host,
            parent: parent_fragment,
            boxed,
            effect,
            items,
            binding: Box::new(binding),
        });
    }
}
