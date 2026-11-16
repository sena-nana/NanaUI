//! Layout and style struct size contracts across hot/cold splits.
//!
//! `docs/reference/theme.md` §1.5 originally chose to resolve sizing intent on
//! write rather than read when `LayoutStyle` was 4808 bytes; even after cold-group
//! sharing (`<= 464 B`), write-time resolution avoids cloning/updating `LayoutStyle`
//! and its cold shared groups per control per frame while `ComputedStyle` stays `<= 80 B`.

#[test]
fn length_spec_and_optional_length_fit_in_8_bytes() {
    assert_eq!(size_of::<nana_ui_core::LengthSpec>(), 8);
    assert_eq!(size_of::<Option<nana_ui_core::LengthSpec>>(), 8);
}

#[test]
fn layout_style_fits_within_subbatch_2b3_budget() {
    let layout = size_of::<nana_ui_core::LayoutStyle>();
    let computed = size_of::<nana_ui_runtime::ComputedStyle>();
    assert!(
        layout <= 464,
        "LayoutStyle regressed to {layout} bytes (expected <= 464)"
    );
    assert!(
        computed < layout,
        "colour resolution rides on ComputedStyle ({computed} B) and stays smaller than LayoutStyle ({layout} B)"
    );
}

#[test]
fn resolve_logical_box_edges_does_not_allocate_default_cold_edges() {
    let mut layout = nana_ui_core::LayoutStyle {
        padding_left: Some(nana_ui_core::LengthSpec::Px(4.0)),
        ..Default::default()
    };
    layout.resolve_logical_box_edges();
    assert!(
        layout.logical_edges.is_shared_default(),
        "resolving a plain physical style must keep Shared::empty() for logical_edges"
    );
}

#[test]
fn computed_and_node_style_sizes_stay_within_subbatch_2b2_budget() {
    let computed = size_of::<nana_ui_runtime::ComputedStyle>();
    let inherited = size_of::<nana_ui_runtime::InheritedStyle>();
    let node_style = size_of::<nana_ui_runtime::NodeStyle>();
    assert!(
        computed <= 80,
        "ComputedStyle regressed to {computed} bytes (expected <= 80)"
    );
    assert!(
        inherited <= 112,
        "InheritedStyle regressed to {inherited} bytes (expected <= 112)"
    );
    assert!(
        node_style <= 80,
        "NodeStyle regressed to {node_style} bytes (expected <= 80)"
    );
}
