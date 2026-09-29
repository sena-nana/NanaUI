fn main() {
    // `.vue` files in `views/` become `$OUT_DIR/nana_views.rs`; a mistake in
    // one fails the build with its file, line and column.
    if let Err(error) = nana_ui_sfc::Compiler::new("::nana_ui::runtime").build("views") {
        panic!("{error}");
    }
}
