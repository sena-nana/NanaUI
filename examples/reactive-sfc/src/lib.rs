//! Views written as `.vue` files (`views/`), compiled in `build.rs`.

pub mod model {
    #[derive(Clone, Debug, PartialEq)]
    pub struct Todo {
        pub id: u32,
        pub title: String,
    }
}

pub mod views {
    use crate::model::Todo;

    include!(concat!(env!("OUT_DIR"), "/nana_views.rs"));
}

/// Views `sfc-benchmark` compares with the same views written by hand
/// (`bench/`, compiled in `build.rs`).
pub mod bench {
    use std::sync::Arc;

    use nana_ui::runtime::Stack;
    use nana_ui::runtime::view::{FieldWrite, Signal, StyledComponent};

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Item {
        pub id: u32,
        pub title: Signal<String>,
    }

    /// As a release build compiles them.
    pub mod views {
        use super::Item;

        include!(concat!(env!("OUT_DIR"), "/nana_bench_views.rs"));
    }

    /// In hot mode, as a debug build compiles them: static text reads a
    /// replaceable table.
    pub mod hot {
        use super::Item;

        include!(concat!(env!("OUT_DIR"), "/nana_bench_views_hot.rs"));
    }

    /// The same views written as functions by someone who passes static
    /// values as values and binds a lone signal directly.
    pub mod idiomatic {
        use std::sync::Arc;

        use nana_ui::runtime::view::{
            IntoView, Signal, button, column, each, row, text, when, widget,
        };
        use nana_ui::runtime::{Stack, Text};

        use super::{Dimmed, Item};

        pub fn static_row(index: usize) -> impl IntoView {
            let label = "打开";
            row(8.0, (text(format!("{label} 第 {index} 行")), button(label)))
        }

        pub fn styled_row(index: usize, selected: Signal<usize>) -> impl IntoView {
            let mut title = Text::new("");
            Arc::make_mut(&mut title.style.layout).flex_grow = Some(1.0);
            widget(Stack::row(8.0).padding_xy(8.0, 4.0))
                .children((
                    widget(title).value(format!("第 {index} 行")),
                    button("打开"),
                ))
                .prop::<bool, Dimmed>(move || selected.get() == index)
        }

        pub fn list_row(
            title: Signal<String>,
            on_remove: impl Fn() + Send + 'static,
        ) -> impl IntoView {
            row(8.0, (text(title), button("删除").on_activate(on_remove)))
        }

        pub fn row_list(list: Signal<Vec<Item>>) -> impl IntoView {
            column(
                4.0,
                (
                    each(
                        list,
                        |item: &Item| item.id,
                        move |item| {
                            list_row(item.title, move || {
                                list.update(|l| l.retain(|t| t.id != item.id))
                            })
                        },
                    ),
                    when(move || list.with(Vec::is_empty), || text("还没有任务"))
                        .otherwise(move || text(move || format!("共 {} 项", list.with(Vec::len)))),
                ),
            )
        }
    }

    /// The same views written as functions with every signal read in a
    /// closure, as `counter_by_hand` in `tests/views.rs`: nothing folded.
    pub mod naive {
        use nana_ui::runtime::view::{IntoView, button, row, signal, text};

        pub fn static_row(index: usize) -> impl IntoView {
            let label = signal(String::from("打开"));
            row(
                8.0,
                (
                    text(move || format!("{label} 第 {index} 行")),
                    button(move || label.get()),
                ),
            )
        }
    }

    /// `StyledRow.vue` with its declarations in `css!` blocks, compiled by
    /// the same engine at build time.
    pub mod inline_css {
        use nana_ui::runtime::view::{IntoView, Signal, button, css, row, text};

        use super::Dimmed;

        pub fn styled_row(index: usize, selected: Signal<usize>) -> impl IntoView {
            row(
                8.0,
                (
                    text(format!("第 {index} 行")).css(css!("flex-grow: 1")),
                    button("打开"),
                ),
            )
            .css(css!("padding: 4px 8px"))
            .prop::<bool, Dimmed>(move || selected.get() == index)
        }
    }

    /// `class:active` written by hand: the function API has no conditional
    /// class, so a row binds its own opacity field.
    pub struct Dimmed;

    // `Stack::node_style` (inherent) returns a copy; the trait's borrows.
    impl FieldWrite<Stack, bool> for Dimmed {
        const FIELD: &'static str = "style.layout.opacity";

        fn write(target: &mut Stack, dimmed: bool) {
            Arc::make_mut(&mut target.node_style_mut().layout).opacity = dimmed.then_some(0.6);
        }

        fn differs(target: &Stack, dimmed: &bool) -> bool {
            StyledComponent::node_style(target).layout.opacity != dimmed.then_some(0.6)
        }
    }
}
