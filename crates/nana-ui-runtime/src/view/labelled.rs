//! A node named for assistive technology by another node's text (ARIA
//! `aria-labelledby`), declared on the element: a select under the caption
//! written beside it, a bare switch at the end of a row.
//!
//! ```ignore
//! let caption = node_ref();
//! column().children((
//!     text("排序方向").node_ref(caption),
//!     select().options(directions).labelled_by(caption),
//! ))
//! ```
//!
//! It is the relation a settings row keeps between its label and its
//! control ([`crate::MutationQueue::set_labelled_by`]). It lives in the
//! world, so the control projecting its own state again keeps it; the
//! control is projected again when the caption's text changes; it goes with
//! either end; and a control with a non-empty name of its own keeps that.
//!
//! The caption may be declared before the control, after it, or in another
//! view. The relation follows what `label` names now, so a caption rebuilt
//! under a `when` names the control again.

use std::panic::Location;

use super::node::{StructuralBinding, ViewBuilder};
use super::prop::PropSource;
use super::reactive::{self, EffectKey, EffectTarget, on_mount};
use crate::{AppContext, FrameworkError, MutationQueue, StableNodeId};

struct LabelledBy {
    label: PropSource<Option<StableNodeId>>,
}

impl StructuralBinding for LabelledBy {
    fn update(
        &mut self,
        cx: &mut AppContext,
        control: StableNodeId,
        effect: EffectKey,
    ) -> Result<(), FrameworkError> {
        let label = reactive::run_tracked(effect, || self.label.get());
        relate(cx, control, label)
    }
}

/// Name `control` by `label`, or by nothing while `label` names no node.
fn relate(
    cx: &mut AppContext,
    control: StableNodeId,
    label: Option<StableNodeId>,
) -> Result<(), FrameworkError> {
    let world = cx.world();
    let label = label.filter(|label| *label != control && world.contains(*label));
    if !world.contains(control) || world.labelled_by(control) == label {
        return Ok(());
    }
    let mut mutations = MutationQueue::new();
    mutations.set_labelled_by(control, label);
    cx.commit_mutations(mutations).map(|_| ())
}

/// Keep `control` named by what `label` names: once the view it is in is
/// in the tree, and whenever `label` moves on.
pub(super) fn bind(
    vb: &mut ViewBuilder<'_, '_, '_>,
    control: StableNodeId,
    label: PropSource<Option<StableNodeId>>,
    site: &'static Location<'static>,
) {
    let effect = reactive::create_effect(vb.st.tag, EffectTarget::Structural(control), None, site);
    // What it reads is what it follows. A caption declared after the
    // control is not built yet, so the relation is made on mount, when it
    // is: reading `label` again there sees it.
    reactive::run_tracked(effect, || label.get());
    on_mount(move |cx| {
        let _ = cx.run_structural_now(control);
    });
    vb.st
        .parts
        .structural
        .push((control, effect, Box::new(LabelledBy { label })));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::view::{
        IntoView, NodeRef, column, node_ref, select, settings_row, signal, slider, switch, text,
        when,
    };
    use crate::{AppContext, DocumentId, LayoutViewport, SelectOption, StableNodeId};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    /// What assistive technology hears `id` called, and its value.
    fn spoken(cx: &mut AppContext, id: StableNodeId) -> (Option<String>, Option<String>) {
        cx.layout_document(document(), LayoutViewport::new(800.0, 600.0))
            .unwrap();
        cx.world()
            .project_accessibility(document())
            .into_iter()
            .find(|node| node.id == id)
            .map(|node| {
                (
                    node.label.map(|label| label.to_string()),
                    node.value.map(|value| value.to_string()),
                )
            })
            .expect("the node is in the accessibility tree")
    }

    fn directions() -> Vec<SelectOption> {
        vec![
            SelectOption::new("asc", "升序"),
            SelectOption::new("desc", "降序"),
        ]
    }

    /// The case MomoBako hit: a sort-direction select read out as its
    /// value. Named by the caption beside it, it says what it is for and
    /// keeps the option it shows as its value; a slider and a bare switch
    /// are named the same way, the caption before them or after.
    #[test]
    fn a_control_is_named_by_the_caption_it_is_labelled_by() {
        let mut cx = AppContext::new();
        let refs = std::cell::Cell::new(None);
        cx.mount_view_root(document(), || {
            let (caption, direction, volume, mute) =
                (node_ref(), node_ref(), node_ref(), node_ref());
            let late = node_ref();
            refs.set(Some((direction, volume, mute)));
            column()
                .children((
                    text("排序方向").node_ref(caption),
                    select()
                        .options(directions())
                        .value(Some(Arc::from("asc")))
                        .labelled_by(caption)
                        .node_ref(direction),
                    slider(0.0, 1.0, 0.1).labelled_by(late).node_ref(volume),
                    switch("").labelled_by(late).node_ref(mute),
                    // Declared after the controls it names.
                    text("音量").node_ref(late),
                ))
                .into_any()
        })
        .unwrap();
        let (direction, volume, mute) = refs.get().unwrap();
        let id = |node: NodeRef| node.get_untracked().unwrap();
        assert_eq!(
            spoken(&mut cx, id(direction)),
            (Some("排序方向".into()), Some("升序".into()))
        );
        assert_eq!(spoken(&mut cx, id(volume)).0.as_deref(), Some("音量"));
        assert_eq!(spoken(&mut cx, id(mute)).0.as_deref(), Some("音量"));
    }

    /// The relation follows the caption: its text, and the node a rebuilt
    /// caption is now. A control's own name still wins over it.
    #[test]
    fn the_name_follows_the_caption_and_yields_to_a_name_of_its_own() {
        let mut cx = AppContext::new();
        let handles = std::cell::Cell::new(None);
        cx.mount_view_root(document(), || {
            let (caption, field, own) = (node_ref(), node_ref(), node_ref());
            let words = signal(String::from("排序方向"));
            let shown = signal(true);
            handles.set(Some((caption, field, own, words, shown)));
            column()
                .children((
                    when(shown, move || text(words).node_ref(caption)),
                    select()
                        .options(directions())
                        .labelled_by(caption)
                        .node_ref(field),
                    select()
                        .options(directions())
                        .label("排序")
                        .labelled_by(caption)
                        .node_ref(own),
                ))
                .into_any()
        })
        .unwrap();
        let (caption, field, own, words, shown) = handles.get().unwrap();
        let field = field.get_untracked().unwrap();
        let own = own.get_untracked().unwrap();
        assert_eq!(spoken(&mut cx, field).0.as_deref(), Some("排序方向"));
        assert_eq!(spoken(&mut cx, own).0.as_deref(), Some("排序"));

        words.set(String::from("顺序"));
        cx.flush_reactive().unwrap();
        assert_eq!(spoken(&mut cx, field).0.as_deref(), Some("顺序"));

        let first = caption.get_untracked().unwrap();
        shown.set(false);
        cx.flush_reactive().unwrap();
        assert!(!cx.world().contains(first));
        assert_eq!(cx.world().labelled_by(field), None, "gone with its caption");
        shown.set(true);
        cx.flush_reactive().unwrap();
        let second = caption.get_untracked().unwrap();
        assert_ne!(first, second);
        assert_eq!(cx.world().labelled_by(field), Some(second));
        assert_eq!(spoken(&mut cx, field).0.as_deref(), Some("顺序"));
    }

    /// A select names itself with `label`, which is never drawn: the field
    /// still shows its option, and a settings row's caption does not
    /// replace the name it has.
    #[test]
    fn a_select_says_its_own_label_and_shows_its_value() {
        let mut cx = AppContext::new();
        let refs = std::cell::Cell::new(None);
        cx.mount_view_root(document(), || {
            let (bare, row_named) = (node_ref(), node_ref());
            refs.set(Some((bare, row_named)));
            column()
                .children((
                    select()
                        .options(directions())
                        .value(Some(Arc::from("desc")))
                        .label("排序方向")
                        .node_ref(bare),
                    settings_row("字号").control(
                        select()
                            .options(directions())
                            .label("排序方向")
                            .node_ref(row_named),
                    ),
                ))
                .into_any()
        })
        .unwrap();
        let (bare, row_named) = refs.get().unwrap();
        let bare = bare.get_untracked().unwrap();
        assert_eq!(
            spoken(&mut cx, bare),
            (Some("排序方向".into()), Some("降序".into()))
        );
        assert_eq!(
            cx.world().text(bare),
            Some("降序"),
            "the field shows its value"
        );
        assert_eq!(
            spoken(&mut cx, row_named.get_untracked().unwrap())
                .0
                .as_deref(),
            Some("排序方向")
        );
    }
}
