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
