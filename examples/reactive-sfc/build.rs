fn main() {
    // `.vue` files in `views/` become `$OUT_DIR/nana_views.rs`; a mistake in
    // one fails the build with its file, line and column.
    // Debug builds keep static text replaceable while running (hot reload).
    let debug = std::env::var("PROFILE").as_deref() == Ok("debug");
    if let Err(error) = nana_ui_sfc::Compiler::new("::nana_ui::runtime")
        .hot(debug)
        .build("views")
    {
        panic!("{error}");
    }
}
