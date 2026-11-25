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
    aria_selected: Option<PropSource<Option<bool>>>,
    hit_test: Option<PropSource<nana_ui_core::HitTestBehavior>>,
}

impl Relations {
    /// What each relation the node has says now; read under the binding's
    /// effect, it is what the binding follows.
    fn read(
        &self,
    ) -> (
        Option<Option<StableNodeId>>,
        Option<Option<Locale>>,
        Option<Option<bool>>,
        Option<nana_ui_core::HitTestBehavior>,
    ) {
        (
            self.labelled_by.as_ref().map(|label| label.get()),
            self.locale.as_ref().map(|locale| locale.get()),
            self.aria_selected.as_ref().map(|selected| selected.get()),
            self.hit_test.as_ref().map(|behavior| behavior.get()),
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
        let (label, locale, selected, hit_test) = reactive::run_tracked(effect, || self.read());
        let named = label.map_or(Ok(()), |label| relate(cx, node, label));
        let scoped = locale.map_or(Ok(()), |locale| scope(cx, node, locale));
        let selected = selected.map_or(Ok(()), |selected| select_override(cx, node, selected));
        let hit_test = hit_test.map_or(Ok(()), |behavior| hit_test_override(cx, node, behavior));
        named.and(scoped).and(selected).and(hit_test)
    }
}

/// Apply `El::hit_test` behavior to `node`.
fn hit_test_override(
    cx: &mut AppContext,
    node: StableNodeId,
    behavior: nana_ui_core::HitTestBehavior,
) -> Result<(), FrameworkError> {
    let target = Some(behavior);
    let world = cx.world();
    if !world.contains(node) || world.hit_test_behavior_override(node) == target {
        return Ok(());
    }
    let mut mutations = MutationQueue::new();
    mutations.set_hit_test_behavior(node, target);
    cx.commit_mutations(mutations).map(|_| ())
}

/// Apply `El::aria_selected` override to `node`.
fn select_override(
    cx: &mut AppContext,
    node: StableNodeId,
    selected: Option<bool>,
) -> Result<(), FrameworkError> {
    let world = cx.world();
    if !world.contains(node) || world.selected_override(node) == selected {
        return Ok(());
    }
    let mut mutations = MutationQueue::new();
    mutations.set_selected(node, selected);
    cx.commit_mutations(mutations).map(|_| ())
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
/// a scope of `locale` and/or `aria_selected` from the commit that creates it.
pub(super) fn bind(
    vb: &mut ViewBuilder<'_, '_, '_>,
    node: StableNodeId,
    labelled_by: Option<PropSource<Option<StableNodeId>>>,
    locale: Option<PropSource<Option<Locale>>>,
    aria_selected: Option<PropSource<Option<bool>>>,
    hit_test: Option<PropSource<nana_ui_core::HitTestBehavior>>,
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
    let aria_selected = match aria_selected {
        Some(PropSource::Const(selected)) => {
            if selected.is_some() {
                vb.ui.mutations().set_selected(node, selected);
            }
            None
        }
        aria_selected => aria_selected,
    };
    let hit_test = match hit_test {
        Some(PropSource::Const(behavior)) => {
            vb.ui
                .mutations()
                .set_hit_test_behavior(node, Some(behavior));
            None
        }
        hit_test => hit_test,
    };
    let relations = Relations {
        labelled_by,
        locale,
        aria_selected,
        hit_test,
    };
    if relations.labelled_by.is_none()
        && relations.locale.is_none()
        && relations.aria_selected.is_none()
        && relations.hit_test.is_none()
    {
        return;
    }
    let effect = reactive::create_effect(vb.st.tag, EffectTarget::Structural(node), None, site);
    // What it reads is what it follows.
    let (_, locale, selected, hit_test) = reactive::run_tracked(effect, || relations.read());
    if let Some(locale) = locale {
        vb.ui.mutations().set_locale(node, locale);
    }
    if let Some(selected) = selected
        && selected.is_some()
    {
        vb.ui.mutations().set_selected(node, selected);
    }
    if let Some(behavior) = hit_test {
        vb.ui
            .mutations()
            .set_hit_test_behavior(node, Some(behavior));
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
