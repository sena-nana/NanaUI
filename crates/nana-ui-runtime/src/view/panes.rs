//! Named slots of the pane composites, as views:
//!
//! ```ignore
//! widget(SplitPane::new(model)).first(list()).second(detail())
//! widget(PaneSection::new()).header(title()).tabs(tabs()).body(rows())
//! ```
//!
//! Each composite places its slots in shells of its own when the tree
//! commits (`TypeBehavior::slot_assembler`).

use super::{El, IntoView};
use crate::{PaneSection, SplitPane};

impl<K> El<SplitPane, K> {
    /// The first pane (left, or top).
    pub fn first(self, view: impl IntoView) -> Self {
        self.slot(view, |mut pane: SplitPane, content| {
            pane.first = Some(content);
            pane
        })
    }

    /// The second pane (right, or bottom).
    pub fn second(self, view: impl IntoView) -> Self {
        self.slot(view, |mut pane: SplitPane, content| {
            pane.second = Some(content);
            pane
        })
    }
}

impl<K> El<PaneSection, K> {
    pub fn header(self, view: impl IntoView) -> Self {
        self.slot(view, PaneSection::header)
    }

    pub fn tabs(self, view: impl IntoView) -> Self {
        self.slot(view, PaneSection::tabs)
    }

    pub fn body(self, view: impl IntoView) -> Self {
        self.slot(view, PaneSection::body)
    }
}
