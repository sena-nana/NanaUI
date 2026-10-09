//! What a view keeps of an element in the world beside its component: the
//! node whose text names it (`El::labelled_by`, see `labelled.rs`) and the
//! locale its subtree resolves its localized text in (`El::locale`):
//!
//! ```ignore
//! column().locale("ar").children(t(LocalizedText::new("greeting")))
//! column().locale(chosen).children(…) // chosen: Signal<Option<Locale>>
//! ```
//!
//! A constant locale goes into the commit that creates the node, before the
//! children it holds are inserted, and nothing follows it. A signal or a
//! closure gives its first value the same way and moves the scope with each
//! later one; a value equal to the scope's locale commits nothing. A move
//! resolves the localized text in the subtree again in the world: no binding
//! of that text runs.
//!
//! The context keeps one structural binding per node, so one binding follows
//! both relations: a change to either reads the other again, which costs a
//! comparison.

use std::panic::Location;

use super::labelled::relate;
use super::node::{StructuralBinding, ViewBuilder};
use super::prop::PropSource;
use super::reactive::{self, EffectKey, EffectTarget, on_mount};
use crate::{AppContext, FrameworkError, Locale, LocaleScope, MutationQueue, StableNodeId};

/// The relations of one node that follow a signal or a closure.
struct Relations {
    labelled_by: Option<PropSource<Option<StableNodeId>>>,
    locale: Option<PropSource<Option<Locale>>>,
}

impl Relations {
    /// What each relation the node has says now; read under the binding's
    /// effect, it is what the binding follows.
    fn read(&self) -> (Option<Option<StableNodeId>>, Option<Option<Locale>>) {
        (
            self.labelled_by.as_ref().map(|label| label.get()),
            self.locale.as_ref().map(|locale| locale.get()),
        )
    }
}

impl StructuralBinding for Relations {
    fn update(
        &mut self,
        cx: &mut AppContext,
        node: StableNodeId,
        effect: EffectKey,
    ) -> Result<(), FrameworkError> {
        let (label, locale) = reactive::run_tracked(effect, || self.read());
        let named = label.map_or(Ok(()), |label| relate(cx, node, label));
        let scoped = locale.map_or(Ok(()), |locale| scope(cx, node, locale));
        named.and(scoped)
    }
}

/// Make `node` a scope of `locale`, or no scope while it is `None`.
fn scope(
    cx: &mut AppContext,
    node: StableNodeId,
    locale: Option<Locale>,
) -> Result<(), FrameworkError> {
    let world = cx.world();
    if !world.contains(node) || world.scope_locale(LocaleScope::Node(node)) == locale.as_ref() {
        return Ok(());
    }
    let mut mutations = MutationQueue::new();
    mutations.set_locale(node, locale);
    cx.commit_mutations(mutations).map(|_| ())
}

/// Keep `node` related as its element declared: named by what `labelled_by`
/// names, once the view it is in is in the tree and whenever that moves on;
/// a scope of `locale` from the commit that creates it.
pub(super) fn bind(
    vb: &mut ViewBuilder<'_, '_, '_>,
    node: StableNodeId,
    labelled_by: Option<PropSource<Option<StableNodeId>>>,
    locale: Option<PropSource<Option<Locale>>>,
    site: &'static Location<'static>,
) {
    // A constant follows nothing: it is the node's from the start.
    let locale = match locale {
        Some(PropSource::Const(locale)) => {
            vb.ui.mutations().set_locale(node, locale);
            None
        }
        locale => locale,
    };
    let relations = Relations {
        labelled_by,
        locale,
    };
    if relations.labelled_by.is_none() && relations.locale.is_none() {
        return;
    }
    let effect = reactive::create_effect(vb.st.tag, EffectTarget::Structural(node), None, site);
    // What it reads is what it follows.
    let (_, locale) = reactive::run_tracked(effect, || relations.read());
    if let Some(locale) = locale {
        vb.ui.mutations().set_locale(node, locale);
    }
    if relations.labelled_by.is_some() {
        // A caption declared after the control is not built yet, so the
        // relation is made on mount, when it is: reading `label` again there
        // sees it.
        on_mount(move |cx| {
            let _ = cx.run_structural_now(node);
        });
    }
    vb.st
        .parts
        .structural
        .push((node, effect, Box::new(relations)));
}
