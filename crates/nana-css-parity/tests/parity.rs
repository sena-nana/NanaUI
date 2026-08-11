//! CSS layout parity：盘点优先用例 T-F* / T-S* / T-L* / T-B01 / T-V01。

use nana_css_parity::cases::{PRIORITY_CASE_IDS, catalog};
use nana_css_parity::{
    CaseStatus, compare_to_expected, format_report, load_all_fixtures, load_fixture,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn fixture_path(id: &str) -> PathBuf {
    nana_css_parity::fixtures_dir().join(format!("{id}.json"))
}

fn assert_pass_case(id: &str) {
    let case = load_fixture(&fixture_path(id)).unwrap_or_else(|e| panic!("{id}: {e}"));
    assert_eq!(case.status, CaseStatus::Pass, "{id} fixture status");
    assert_eq!(case.id, id);
    let report = compare_to_expected(&case);
    assert!(report.ok, "{}", format_report(&report));
}

// --- Flex ---

#[test]
fn t_f01_row_gap() {
    assert_pass_case("T-F01");
}
#[test]
fn t_f02_column_gap() {
    assert_pass_case("T-F02");
}
#[test]
fn t_f03_justify_center() {
    assert_pass_case("T-F03");
}
#[test]
fn t_f04_space_between() {
    assert_pass_case("T-F04");
}
#[test]
fn t_f05_align_center() {
    assert_pass_case("T-F05");
}
#[test]
fn t_f06_flex1_row() {
    assert_pass_case("T-F06");
}
#[test]
fn t_f07_flex1_column() {
    assert_pass_case("T-F07");
}
#[test]
fn t_f08_fixed_plus_flex1() {
    assert_pass_case("T-F08");
}
#[test]
fn t_f09_space_around() {
    assert_pass_case("T-F09");
}
#[test]
fn t_f10_space_evenly() {
    assert_pass_case("T-F10");
}
#[test]
fn t_f11_gap_two_value_row() {
    assert_pass_case("T-F11");
}
#[test]
fn t_f12_row_column_gap_longhands() {
    assert_pass_case("T-F12");
}
#[test]
fn t_f13_row_gap_percent() {
    assert_pass_case("T-F13");
}
#[test]
fn t_f14_column_gap_percent() {
    assert_pass_case("T-F14");
}
#[test]
fn t_f15_justify_flex_end() {
    assert_pass_case("T-F15");
}
#[test]
fn t_f16_align_items_flex_end() {
    assert_pass_case("T-F16");
}
#[test]
fn t_f17_flex_grow_weighted() {
    assert_pass_case("T-F17");
}
#[test]
fn t_f18_flex_shrink_overflow() {
    assert_pass_case("T-F18");
}
#[test]
fn t_f19_flex_shrink_min_width_freeze() {
    assert_pass_case("T-F19");
}

// --- Size ---

#[test]
fn t_s01_percent_width() {
    assert_pass_case("T-S01");
}
#[test]
fn t_s02_height_percent_chain() {
    assert_pass_case("T-S02");
}
#[test]
fn t_s03_min_height() {
    assert_pass_case("T-S03");
}
#[test]
fn t_s04_min_width_zero() {
    assert_pass_case("T-S04");
}
#[test]
fn t_s05_px_size() {
    assert_pass_case("T-S05");
}

// --- Shell / Lilia ---

#[test]
fn t_l01_app_shell() {
    assert_pass_case("T-L01");
}
#[test]
fn t_l02_sidebar_main() {
    assert_pass_case("T-L02");
}
#[test]
fn t_l03_settings_row() {
    assert_pass_case("T-L03");
}
#[test]
fn t_l04_flex_basis_sidebar_without_width() {
    assert_pass_case("T-L04");
}

// --- Box / visibility ---

#[test]
fn t_b01_padding_four() {
    assert_pass_case("T-B01");
}
#[test]
fn t_b02_margin_four() {
    assert_pass_case("T-B02");
}
#[test]
fn t_b03_margin_two() {
    assert_pass_case("T-B03");
}
#[test]
fn t_b04_padding_three() {
    assert_pass_case("T-B04");
}
#[test]
fn t_b05_margin_three() {
    assert_pass_case("T-B05");
}
#[test]
fn t_b06_percent_padding_margin_sibling_advance() {
    assert_pass_case("T-B06");
}
#[test]
fn t_b07_column_percent_margin_sibling_advance() {
    assert_pass_case("T-B07");
}
#[test]
fn t_b08_content_box_width_plus_padding() {
    assert_pass_case("T-B08");
}
#[test]
fn t_b09_border_width_in_border_box() {
    assert_pass_case("T-B09");
}
#[test]
fn t_s06_max_width() {
    assert_pass_case("T-S06");
}
#[test]
fn t_s07_max_height() {
    assert_pass_case("T-S07");
}
#[test]
fn t_s08_calc_percent_minus_px() {
    assert_pass_case("T-S08");
}
#[test]
fn t_s09_nested_percent_and_calc() {
    assert_pass_case("T-S09");
}
#[test]
fn t_s10_calc_lightweight_forms() {
    assert_pass_case("T-S10");
}
#[test]
fn t_s11_nested_padding_percent_width_chain() {
    assert_pass_case("T-S11");
}
#[test]
fn t_s12_min_width_floors_percent_width() {
    assert_pass_case("T-S12");
}
#[test]
fn t_s13_flex_min_width_redistributes() {
    assert_pass_case("T-S13");
}
#[test]
fn t_s14_flex_max_width_redistributes() {
    assert_pass_case("T-S14");
}
#[test]
fn t_s15_viewport_min_clamp() {
    assert_pass_case("T-S15");
}
#[test]
fn t_s16_em_rem_sizes() {
    assert_pass_case("T-S16");
}
#[test]
fn t_b10_negative_margin_rem_padding() {
    assert_pass_case("T-B10");
}
#[test]
fn t_b11_logical_padding_ltr() {
    assert_pass_case("T-B11");
}
#[test]
fn t_b12_logical_margin_ltr() {
    assert_pass_case("T-B12");
}
#[test]
fn t_g01_grid_template_columns() {
    assert_pass_case("T-G01");
}
#[test]
fn t_g02_grid_var_minmax() {
    assert_pass_case("T-G02");
}
#[test]
fn t_g03_grid_two_equal_fr() {
    assert_pass_case("T-G03");
}
#[test]
fn t_g04_grid_weighted_fr() {
    assert_pass_case("T-G04");
}
#[test]
fn t_g05_grid_minmax_nonzero_min() {
    assert_pass_case("T-G05");
}
#[test]
fn t_g06_grid_fractional_fr() {
    assert_pass_case("T-G06");
}
#[test]
fn t_g07_grid_multi_min_freeze() {
    assert_pass_case("T-G07");
}
#[test]
fn t_g08_grid_minmax_px_max() {
    assert_pass_case("T-G08");
}
#[test]
fn t_g09_grid_multi_max_freeze() {
    assert_pass_case("T-G09");
}
#[test]
fn t_g10_grid_template_rows_gap() {
    assert_pass_case("T-G10");
}
#[test]
fn t_g11_grid_rows_minmax_min_freeze() {
    assert_pass_case("T-G11");
}
#[test]
fn t_g12_grid_rows_minmax_max_clamp() {
    assert_pass_case("T-G12");
}
#[test]
fn t_g13_grid_rows_multi_min_freeze() {
    assert_pass_case("T-G13");
}
#[test]
fn t_g14_grid_rows_multi_max_freeze() {
    assert_pass_case("T-G14");
}
#[test]
fn t_g15_grid_columns_win_over_rows() {
    assert_pass_case("T-G15");
}
#[test]
fn t_g16_columns_none_to_rows_only() {
    assert_pass_case("T-G16");
}
#[test]
fn t_g17_rows_none_to_columns_only() {
    assert_pass_case("T-G17");
}
#[test]
fn t_g18_both_none_falls_back_to_row() {
    assert_pass_case("T-G18");
}
#[test]
fn t_g19_grid_gap_percent() {
    assert_pass_case("T-G19");
}
#[test]
fn t_p01_position_relative_inset() {
    assert_pass_case("T-P01");
}
#[test]
fn t_p02_position_absolute_top_left() {
    assert_pass_case("T-P02");
}
#[test]
fn t_p03_position_absolute_right_bottom() {
    assert_pass_case("T-P03");
}
#[test]
fn t_p04_absolute_left_right_stretch() {
    assert_pass_case("T-P04");
}
#[test]
fn t_p05_absolute_padded_containing_block() {
    assert_pass_case("T-P05");
}
#[test]
fn t_p06_absolute_static_origin() {
    assert_pass_case("T-P06");
}
#[test]
fn t_p07_absolute_percent_inset() {
    assert_pass_case("T-P07");
}
#[test]
fn t_p08_nested_absolute() {
    assert_pass_case("T-P08");
}
#[test]
fn t_p09_inset_two_value_shorthand() {
    assert_pass_case("T-P09");
}
#[test]
fn t_p10_inset_mixed_percent_px() {
    assert_pass_case("T-P10");
}
#[test]
fn t_p11_absolute_mixed_sides_percent_px() {
    assert_pass_case("T-P11");
}
#[test]
fn t_p12_inset_three_value_mixed() {
    assert_pass_case("T-P12");
}
#[test]
fn t_p13_inset_four_value_mixed() {
    assert_pass_case("T-P13");
}
#[test]
fn t_p14_logical_inset_ltr() {
    assert_pass_case("T-P14");
}
#[test]
fn t_v01_display_none() {
    assert_pass_case("T-V01");
}
#[test]
fn t_v02_visibility_hidden_skips_like_display_none() {
    assert_pass_case("T-V02");
}

#[test]
fn t_w01_flex_wrap() {
    assert_pass_case("T-W01");
}
#[test]
fn t_w02_flex_wrap_reverse() {
    assert_pass_case("T-W02");
}
#[test]
fn t_w03_flex_wrap_two_value_gap() {
    assert_pass_case("T-W03");
}
#[test]
fn t_w04_flex_wrap_counts_horizontal_margin() {
    assert_pass_case("T-W04");
}
#[test]
fn t_w05_wrap_cross_gap_percent() {
    assert_pass_case("T-W05");
}
#[test]
fn t_w06_wrap_reverse_cross_gap_percent() {
    assert_pass_case("T-W06");
}
#[test]
fn t_w07_column_flex_wrap() {
    assert_pass_case("T-W07");
}
#[test]
fn t_w08_column_wrap_reverse() {
    assert_pass_case("T-W08");
}
#[test]
fn t_w09_column_wrap_vertical_margin() {
    assert_pass_case("T-W09");
}

#[test]
fn catalog_covers_priority_ids() {
    let ids: BTreeSet<_> = catalog().iter().map(|(id, _, _)| *id).collect();
    for id in PRIORITY_CASE_IDS {
        assert!(ids.contains(id), "catalog missing {id}");
        assert!(fixture_path(id).is_file(), "fixture file missing for {id}");
    }
}

#[test]
fn all_pass_fixtures_succeed() {
    let fixtures = load_all_fixtures().expect("fixtures");
    let mut failures = Vec::new();
    for (_path, case) in fixtures {
        if case.status != CaseStatus::Pass {
            continue;
        }
        let report = compare_to_expected(&case);
        if !report.ok {
            failures.push(format_report(&report));
        }
    }
    assert!(
        failures.is_empty(),
        "pass fixtures failed:\n{}",
        failures.join("\n")
    );
}

#[cfg(feature = "webview-ref")]
mod webview_live {
    use super::*;
    use nana_css_parity::{compare_maps, expected_to_map, measure_nana};

    #[test]
    #[ignore = "需要本机显示环境；CI 无显示时跳过。运行: cargo test -p nana-css-parity --features webview-ref -- --ignored"]
    fn webview_vs_nana_pass_cases() {
        let fixtures = load_all_fixtures().expect("fixtures");
        for (_path, case) in fixtures {
            if case.status != CaseStatus::Pass {
                continue;
            }
            match nana_css_parity::webview::measure_webview(&case) {
                Err(e) if e.starts_with("skip:") => {
                    eprintln!("skip {}: {e}", case.id);
                    continue;
                }
                Err(e) => panic!("{}: {e}", case.id),
                Ok(boxes) => {
                    let nana = measure_nana(&case);
                    let expected = expected_to_map(&boxes);
                    let report = compare_maps(&case.id, &expected, &nana, case.tolerance_px);
                    assert!(report.ok, "{}", format_report(&report));
                }
            }
        }
    }
}
