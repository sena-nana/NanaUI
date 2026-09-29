//! Named slots of modal surfaces and the media transport bar, as views:
//!
//! ```ignore
//! widget(Dialog::new("投币"))
//!     .body(column().children((summary, choices)))
//!     .footer(row().children((cancel, confirm)))
//! ```
//!
//! Each slot is built before its composite and placed by the composite's
//! assembler once the tree commits, so a view never calls
//! `set_modal_slots` or appends into the transport bar's groups. A slot's
//! node stays for the view's life; what changes inside it is a `when` /
//! `each` there.

use super::{El, IntoView};
use crate::{Dialog, Drawer, MediaTransportBar, ModalSurface};

macro_rules! modal_slots {
    ($($surface:ty),*) => {$(
        impl<K> El<$surface, K> {
            /// The surface's content, under its title.
            pub fn body(self, view: impl IntoView) -> Self {
                self.slot(view, |mut modal: $surface, id| {
                    modal.slots_mut().body = Some(id);
                    modal
                })
            }

            /// The row of actions at the surface's foot.
            pub fn footer(self, view: impl IntoView) -> Self {
                self.slot(view, |mut modal: $surface, id| {
                    modal.slots_mut().footer = Some(id);
                    modal
                })
            }
        }
    )*};
}

modal_slots!(Dialog, Drawer);

impl<K> El<MediaTransportBar, K> {
    /// Controls after play: next, skip.
    pub fn leading(self, view: impl IntoView) -> Self {
        self.slot(view, MediaTransportBar::leading_content)
    }

    /// Controls before volume: speed, quality, subtitles.
    pub fn trailing(self, view: impl IntoView) -> Self {
        self.slot(view, MediaTransportBar::trailing_content)
    }

    /// The second row under the seek bar; it collapses while empty.
    pub fn secondary(self, view: impl IntoView) -> Self {
        self.slot(view, MediaTransportBar::secondary_content)
    }
}
