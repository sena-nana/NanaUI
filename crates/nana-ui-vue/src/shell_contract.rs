//! Nana shell / controls **class → [`LayoutStyle`]** contract (non-neutral).
//!
//! Documented `nana-*` host classes and layout utilities (`gap-sm`, `flex-row`,
//! …) live here. Neutral CSS declaration parse stays in [`crate::css_map`].
//! Business / `lilia-*` BEM must not be harvested.
//!
//! [`LayoutStyleCss::apply_class_layout_hints`](crate::css_map::LayoutStyleCss)
//! delegates here so cascade / measure call sites stay stable.

use nana_ui_core::box_layout::{
    AlignSpec, DisplaySpec, FlexDirection, FlexWrap, GridTrack, JustifySpec, LayoutStyle,
    LengthSpec, OverflowSpec,
};

/// Apply documented shell / utility / `nana-*` class tokens onto `layout`.
///
/// This is **not** the neutral CSS parser. Utility (`gap-sm`, `flex-row`) and
/// documented host classes live here; business/`lilia-*` BEM must not be
/// harvested. Neutral property parse stays in [`crate::css_map::LayoutStyleCss`].
pub fn apply_class_layout_hints(layout: &mut LayoutStyle, class_names: &[String]) {
    for name in class_names {
        match name.as_str() {
            "gap-sm" => {
                apply_class_uniform_gap(layout, 4.0);
            }
            "gap-md" => {
                apply_class_uniform_gap(layout, 8.0);
            }
            "gap-lg" => {
                apply_class_uniform_gap(layout, 16.0);
            }
            "p-sm" | "pad-sm" => {
                if layout.padding.is_none() {
                    layout.padding = Some(LengthSpec::Px(4.0));
                }
            }
            "p-md" | "pad-md" => {
                if layout.padding.is_none() {
                    layout.padding = Some(LengthSpec::Px(8.0));
                }
            }
            "p-lg" | "pad-lg" => {
                if layout.padding.is_none() {
                    layout.padding = Some(LengthSpec::Px(16.0));
                }
            }
            "w-full" | "width-full" => {
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Fill);
                }
            }
            "h-full" | "height-full" => {
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Fill);
                }
            }
            "min-w-0" | "min-width-0" => {
                layout.min_width = Some(LengthSpec::Px(0.0));
                layout.allow_shrink = true;
            }
            "flex-1" | "grow" => {
                if layout.flex_grow.is_none() {
                    layout.flex_grow = Some(1.0);
                }
            }
            "items-center" => layout.align_items = AlignSpec::Center,
            "items-start" => layout.align_items = AlignSpec::Start,
            "items-end" => layout.align_items = AlignSpec::End,
            "items-stretch" => layout.align_items = AlignSpec::Stretch,
            "justify-center" => layout.justify_content = JustifySpec::Center,
            "justify-between" | "justify-space-between" => {
                layout.justify_content = JustifySpec::SpaceBetween;
            }
            "justify-end" => layout.justify_content = JustifySpec::End,
            "justify-start" => layout.justify_content = JustifySpec::Start,
            "overflow-auto" | "overflow-y-auto" => {
                layout.overflow_y = OverflowSpec::Auto;
            }
            "overflow-hidden" => {
                layout.overflow_x = OverflowSpec::Hidden;
                layout.overflow_y = OverflowSpec::Hidden;
            }
            "rounded" | "rounded-lg" => {
                if layout.border_radius.is_none() {
                    layout.border_radius = Some(12.0);
                }
            }
            "rounded-md" => {
                if layout.border_radius.is_none() {
                    layout.border_radius = Some(8.0);
                }
            }
            "rounded-full" | "pill" if layout.border_radius.is_none() => {
                layout.border_radius = Some(999.0);
            }
            _ => {}
        }
    }

    // nana-controls.css 布局 class 镜像（公共控件合同；业务页 class 走 stylesheet）
    for name in class_names {
        match name.as_str() {
            "nana-settings-page" | "settings-page" => {
                layout.ensure_direction(FlexDirection::Column);
                if layout.gap.is_none() {
                    layout.gap = Some(LengthSpec::Px(16.0));
                }
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                if layout.padding.is_none()
                    && layout.padding_top.is_none()
                    && layout.padding_left.is_none()
                {
                    layout.padding_top = Some(LengthSpec::Px(20.0));
                    layout.padding_right = Some(LengthSpec::Px(24.0));
                    layout.padding_bottom = Some(LengthSpec::Px(24.0));
                    layout.padding_left = Some(LengthSpec::Px(24.0));
                }
            }
            "nana-settings-card" | "nana-card" | "card" => {
                // Public nana-controls / card contract (not app page classes).
                // Do not invent gap: Lilia `.card` + `.card-heading { margin-bottom }`
                // already owns heading↔body spacing; a default gap:8 double-counts
                // that 8px and pushes charts down.
                layout.ensure_direction(FlexDirection::Column);
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                if layout.padding.is_none()
                    && layout.padding_top.is_none()
                    && layout.padding_left.is_none()
                {
                    layout.padding = Some(LengthSpec::Px(12.0));
                }
                if layout.background.is_none() {
                    layout.background = Some([1.0, 1.0, 1.0, 1.0]);
                }
                if layout.border_radius.is_none() {
                    layout.border_radius = Some(16.0);
                }
                if layout.border_width.is_none() {
                    layout.border_width = Some(1.0);
                }
                if layout.border_color.is_none() {
                    layout.border_color = Some([0.933, 0.941, 0.953, 1.0]);
                }
            }
            "nana-settings-card__body" => {
                layout.ensure_direction(FlexDirection::Column);
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
            }
            "nana-settings-row" | "settings-row" => {
                if !class_names
                    .iter()
                    .any(|c| c == "nana-settings-row--stacked")
                {
                    layout.direction = Some(FlexDirection::Row);
                }
                layout.align_items = AlignSpec::Center;
                layout.justify_content = JustifySpec::SpaceBetween;
                // Force — mirrors nana-controls.css; may override kind default gap.
                layout.gap = Some(LengthSpec::Px(14.0));
                layout.flex_wrap = FlexWrap::Wrap;
                if layout.padding_top.is_none() && layout.padding.is_none() {
                    layout.padding_top = Some(LengthSpec::Px(10.0));
                    layout.padding_bottom = Some(LengthSpec::Px(10.0));
                }
            }
            "nana-settings-row--stacked" => {
                layout.direction = Some(FlexDirection::Column);
                layout.align_items = AlignSpec::Stretch;
            }
            "nana-settings-row__label" | "settings-row__label" => {
                if layout.flex_grow.is_none() {
                    layout.flex_grow = Some(1.0);
                }
                if layout.flex_basis.is_none() {
                    layout.flex_basis = Some(LengthSpec::Px(220.0));
                }
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                // Mirrors nana-controls.css label truncation.
                layout.text_overflow_ellipsis = true;
                layout.white_space_nowrap = true;
                if layout.overflow_x == OverflowSpec::Visible {
                    layout.overflow_x = OverflowSpec::Hidden;
                }
            }
            "nana-sidebar-row__label" => {
                if layout.flex_grow.is_none() {
                    layout.flex_grow = Some(1.0);
                }
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                layout.text_overflow_ellipsis = true;
                layout.white_space_nowrap = true;
                if layout.overflow_x == OverflowSpec::Visible {
                    layout.overflow_x = OverflowSpec::Hidden;
                }
            }
            "nana-settings-row__control" | "settings-row__control" => {
                layout.direction = Some(layout.direction.unwrap_or(FlexDirection::Row));
                layout.align_items = AlignSpec::Center;
                layout.justify_content = JustifySpec::End;
                if layout.gap.is_none() {
                    layout.gap = Some(LengthSpec::Px(8.0));
                }
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                if layout.max_width.is_none() {
                    layout.max_width = Some(LengthSpec::Fill); // marker: no finite clamp
                }
            }
            "nana-appearance-panel" => {
                layout.ensure_direction(FlexDirection::Column);
                if layout.gap.is_none() {
                    layout.gap = Some(LengthSpec::Px(12.0));
                }
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
            }
            // Document scaffold roots: Fill so viewport → mount → % height
            // chains survive cascade rebuild (e.g. unrelated inject_stylesheet).
            "nana-html-root" | "nana-mount-root" => {
                layout.ensure_direction(FlexDirection::Column);
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Fill);
                }
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Fill);
                }
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                layout.min_height = Some(layout.min_height.unwrap_or(LengthSpec::Px(0.0)));
            }
            "nana-workspace-shell" => {
                layout.ensure_direction(FlexDirection::Column);
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Fill);
                }
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                layout.min_height = Some(layout.min_height.unwrap_or(LengthSpec::Px(0.0)));
            }
            "nana-workspace-shell__body" => {
                // grid: 220px 1fr → Row + Fixed + Fill（P0-6）
                layout.direction = Some(FlexDirection::Row);
                if layout.grid_columns.is_none() {
                    layout.grid_columns = Some(vec![
                        GridTrack::Px(220.0),
                        GridTrack::MinMax {
                            min_px: 0.0,
                            fr: 1.0,
                            max_px: None,
                        },
                    ]);
                }
                if layout.flex_grow.is_none() {
                    layout.flex_grow = Some(1.0);
                }
                layout.min_height = Some(layout.min_height.unwrap_or(LengthSpec::Px(0.0)));
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Fill);
                }
            }
            "nana-workspace-shell__sidebar" => {
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Px(220.0));
                }
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Fill);
                }
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                if layout.padding.is_none() && layout.padding_top.is_none() {
                    layout.padding = Some(LengthSpec::Px(8.0));
                }
            }
            "nana-workspace-shell__primary" => {
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Fill);
                }
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Fill);
                }
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                if layout.padding.is_none()
                    && layout.padding_top.is_none()
                    && layout.padding_left.is_none()
                {
                    layout.padding_top = Some(LengthSpec::Px(20.0));
                    layout.padding_right = Some(LengthSpec::Px(24.0));
                    layout.padding_bottom = Some(LengthSpec::Px(20.0));
                    layout.padding_left = Some(LengthSpec::Px(24.0));
                }
            }
            "titlebar" | "nana-workspace-shell__titlebar" => {
                // Public chrome strip contract: horizontal Fixed height.
                layout.direction = Some(FlexDirection::Row);
                layout.align_items = AlignSpec::Center;
                layout.height = Some(LengthSpec::Px(36.0));
                layout.flex_grow = None;
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Fill);
                }
                if layout.min_height.is_none() {
                    layout.min_height = Some(LengthSpec::Px(36.0));
                }
                if layout.padding_left.is_none() && layout.padding.is_none() {
                    layout.padding_left = Some(LengthSpec::Px(12.0));
                    layout.padding_right = Some(LengthSpec::Px(12.0));
                }
            }
            "titlebar__left-controls" | "titlebar__center" | "titlebar__controls" => {
                layout.direction = Some(FlexDirection::Row);
                layout.align_items = AlignSpec::Center;
                layout.height = Some(LengthSpec::Fill);
                layout.flex_grow = None;
            }
            "nana-sidebar-frame" => {
                layout.ensure_direction(FlexDirection::Column);
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Fill);
                }
                // Stretch cross-axis so body rows/labels get the frame content
                // width (default Start leaves ellipsis+grow text at w=0).
                layout.align_items = AlignSpec::Stretch;
                layout.min_height = Some(layout.min_height.unwrap_or(LengthSpec::Px(0.0)));
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                layout.allow_shrink = true;
                if layout.gap.is_none() {
                    layout.gap = Some(LengthSpec::Px(14.0));
                }
                if layout.padding.is_none()
                    && layout.padding_top.is_none()
                    && layout.padding_left.is_none()
                {
                    layout.padding_top = Some(LengthSpec::Px(10.0));
                    layout.padding_bottom = Some(LengthSpec::Px(10.0));
                    layout.padding_left = Some(LengthSpec::Px(12.0));
                    layout.padding_right = Some(LengthSpec::Px(8.0));
                }
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Px(220.0));
                }
                layout.overflow_x = OverflowSpec::Hidden;
                layout.overflow_y = OverflowSpec::Hidden;
            }
            "nana-sidebar-frame__body" => {
                if layout.flex_grow.is_none() {
                    layout.flex_grow = Some(1.0);
                }
                layout.align_items = AlignSpec::Stretch;
                layout.min_height = Some(layout.min_height.unwrap_or(LengthSpec::Px(0.0)));
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                layout.allow_shrink = true;
                layout.overflow_y = OverflowSpec::Auto;
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Fill);
                }
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Fill);
                }
            }
            // Documented host contract only — apps alias DOM to this class
            // (do not recognize product `lilia-*` BEM here).
            "nana-workspace-region__content" => {
                // Fill so % / definite-height chain holds. Stretch matches CSS
                // block layout (width:auto children fill the CB); Start left
                // page grids (`1fr` tracks) at w=0 → hollow Card shells.
                layout.ensure_direction(FlexDirection::Column);
                layout.align_items = AlignSpec::Stretch;
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Fill);
                }
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Fill);
                }
                if layout.flex_grow.is_none() {
                    layout.flex_grow = Some(1.0);
                }
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                layout.min_height = Some(layout.min_height.unwrap_or(LengthSpec::Px(0.0)));
            }
            "nana-sidebar-frame__top" | "nana-sidebar-frame__footer" => {
                if layout.flex_grow.is_none() {
                    layout.flex_grow = Some(0.0);
                }
                layout.flex_shrink = Some(0.0);
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Fill);
                }
            }
            // Body list stack (nana-controls.css `.sidebar-sections`): fill the
            // scrollport cross-axis so remounted frames keep readable row labels.
            // Stretch matches CSS flex initial align-items so header rows with
            // `flex:1` + ellipsis labels receive a definite width.
            "sidebar-sections" | "sidebar-collapse" | "sidebar-collapse__inner" => {
                layout.ensure_direction(FlexDirection::Column);
                layout.align_items = AlignSpec::Stretch;
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Fill);
                }
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                layout.allow_shrink = true;
                if layout.flex_grow.is_none() {
                    layout.flex_grow = Some(1.0);
                }
                layout.min_height = Some(layout.min_height.unwrap_or(LengthSpec::Px(0.0)));
            }
            "nana-sidebar-nav" => {
                layout.ensure_direction(FlexDirection::Column);
                if layout.gap.is_none() {
                    layout.gap = Some(LengthSpec::Px(4.0));
                }
            }
            "nana-sidebar-row" | "nana-sidebar-nav__item" | "sidebar-row" => {
                layout.direction = Some(FlexDirection::Row);
                layout.align_items = AlignSpec::Center;
                if layout.gap.is_none() {
                    layout.gap = Some(LengthSpec::Px(6.0));
                }
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Fill);
                }
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Px(28.0));
                }
                layout.allow_shrink = true;
                layout.min_width = Some(layout.min_width.unwrap_or(LengthSpec::Px(0.0)));
                if layout.padding_left.is_none() && layout.padding.is_none() {
                    layout.padding_left = Some(LengthSpec::Px(10.0));
                    layout.padding_right = Some(LengthSpec::Px(10.0));
                }
                if layout.border_radius.is_none() {
                    layout.border_radius = Some(12.0);
                }
            }
            "nana-switch--control-end" => {
                layout.justify_content = JustifySpec::SpaceBetween;
            }
            "nana-tabs--fill" if layout.width.is_none() => {
                layout.width = Some(LengthSpec::Fill);
            }
            _ => {}
        }
    }

    // Generic direction utilities + documented host contracts only.
    // Do NOT invent layout from app/page class names (home-page, overview-*,
    // sb-*, lilia-*, repo-*, …) — those must come from CSS / Style Model.
    // When stylesheet already authored grid tracks / display:grid, row/col
    // utility classes must not clobber the grid axis (recompute_grid_axis).
    let grid_axis_locked = has_authored_grid_axis(layout);
    for name in class_names {
        match name.as_str() {
            "flex-row" | "hstack" | "nana-row" | "row" => {
                if !grid_axis_locked {
                    layout.direction = Some(FlexDirection::Row);
                }
                // Do not invent gap — CSS flex direction utilities are gapless
                // unless `gap` / `gap-*` is authored (avoids workspace seams).
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Fill);
                }
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Fill);
                }
            }
            "flex-col" | "flex-column" | "vstack" | "nana-column" | "column" => {
                if !grid_axis_locked && layout.direction.is_none() {
                    layout.direction = Some(FlexDirection::Column);
                }
                // Same as flex-row: direction only; gap must be explicit.
            }
            "nana-root-paint" => {
                if layout.direction.is_none() {
                    layout.direction = Some(FlexDirection::Column);
                }
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Fill);
                }
            }
            // Documented `<nana-gpu>` / host preview slot contract.
            "nana-gpu-preview" | "nana-gpu" => {
                if layout.width.is_none() {
                    layout.width = Some(LengthSpec::Fill);
                }
                if layout.height.is_none() {
                    layout.height = Some(LengthSpec::Px(100.0));
                }
                if layout.border_radius.is_none() {
                    layout.border_radius = Some(10.0);
                }
                if layout.border_width.is_none() {
                    layout.border_width = Some(1.0);
                }
                if layout.border_color.is_none() {
                    layout.border_color = Some([0.89, 0.90, 0.91, 1.0]);
                }
                // Drop navy WebView host-marker so the light placeholder paints.
                if layout.background.is_some_and(is_gpu_host_marker_bg) {
                    layout.background = None;
                }
                layout.flex_grow = Some(0.0);
                layout.flex_shrink = Some(0.0);
                layout.allow_shrink = true;
            }
            _ => {}
        }
    }
}

/// Class token (`gap-sm` / `gap-md` / `gap-lg`) → uniform gap.
/// Only fills when `gap` is unset; when it writes, clears axis longhands
/// (same reset as css_map gap shorthand one-value form).
fn apply_class_uniform_gap(layout: &mut LayoutStyle, px: f32) {
    if layout.gap.is_none() {
        layout.gap = Some(LengthSpec::Px(px));
        layout.row_gap = None;
        layout.column_gap = None;
    }
}

/// Stylesheet already set `display:grid` / `inline-grid` or non-empty template tracks.
fn has_authored_grid_axis(layout: &LayoutStyle) -> bool {
    layout.display.is_some_and(DisplaySpec::is_grid_container)
        || layout.grid_columns.as_ref().is_some_and(|t| !t.is_empty())
        || layout.grid_rows.as_ref().is_some_and(|t| !t.is_empty())
}

/// Host WebView may mark `<nana-gpu>` with slate `#1e293b` until a texture
/// composites. On the Scene host that navy void is not a texture — drop it so
/// the light placeholder can paint.
fn is_gpu_host_marker_bg(c: [f32; 4]) -> bool {
    // #1e293b ≈ rgb(30, 41, 59)
    (c[0] - 30.0 / 255.0).abs() < 0.04
        && (c[1] - 41.0 / 255.0).abs() < 0.04
        && (c[2] - 59.0 / 255.0).abs() < 0.05
        && c[3] > 0.9
}
