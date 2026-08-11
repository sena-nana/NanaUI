// Surface paint + flex child sizing + length_from_spec / align helpers.

#[derive(Clone, Copy)]
struct SurfacePaint {
    background: Option<[f32; 4]>,
    radius: f32,
    border_width: f32,
    border_color: [f32; 4],
}

fn surface_paint_from(layout: &crate::css_map::LayoutStyle) -> SurfacePaint {
    SurfacePaint {
        background: layout.background,
        radius: layout.border_radius.unwrap_or(0.0),
        border_width: layout.border_width.unwrap_or(0.0),
        border_color: layout.border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]),
    }
}

fn surface_style(paint: SurfacePaint) -> iced::widget::container::Style {
    let mut style = iced::widget::container::Style::default();
    if let Some(bg) = paint.background {
        style.background = Some(Background::Color(rgba_color(bg)));
    }
    if paint.radius > 0.0 || paint.border_width > 0.0 {
        style.border = Border {
            color: rgba_color(paint.border_color),
            width: paint.border_width,
            radius: paint.radius.into(),
        };
    }
    if paint.radius >= 8.0 && paint.background.is_some() {
        style.shadow = Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.06),
            offset: iced::Vector::new(0.0, 1.0),
            blur_radius: 4.0,
        };
    }
    style
}

/// 布局节点已 chromed 后，仅按父主轴补 flex / % 尺寸（避免双重 padding）。
fn apply_flex_child_sizing<'a, Message: 'a>(
    content: Element<'a, Message>,
    layout: &crate::css_map::LayoutStyle,
    parent_box: ParentBox,
    parent_direction: FlexDirection,
    parent_align_items: AlignSpec,
    main_override: Option<f32>,
) -> Element<'a, Message> {
    if let Some(px) = main_override {
        let mut c = container(content);
        match parent_direction {
            FlexDirection::Row => c = c.width(Length::Fixed(px.max(0.0))),
            FlexDirection::Column => c = c.height(Length::Fixed(px.max(0.0))),
        }
        // Grow/override wrapper is the flex item's border box — paint here so
        // workspace/region fill is not left transparent when the inner chrome
        // shrink-wraps below the allocated track.
        if layout.has_surface_paint() {
            let paint = surface_paint_from(layout);
            c = c.style(move |_theme| surface_style(paint));
        }
        return c.into();
    }
    let main = layout.child_main_length(parent_direction);
    let main = main.filter(|m| {
        layout.grows()
            || matches!(
                m,
                LengthSpec::Percent(_)
                    | LengthSpec::CalcPercentOffset { .. }
                    | LengthSpec::Viewport { .. }
                    | LengthSpec::CalcViewportOffset { .. }
                    | LengthSpec::Min2(_, _)
                    | LengthSpec::Max2(_, _)
                    | LengthSpec::Clamp3(_, _, _)
            )
    });
    let cross_stretch = matches!(
        layout.resolved_align_self(parent_align_items),
        AlignSpec::Stretch
    );
    let cross_fill_width = matches!(parent_direction, FlexDirection::Column)
        && (matches!(layout.width, Some(LengthSpec::Fill))
            || (layout.width.is_none() && cross_stretch));
    let cross_fill_height = matches!(parent_direction, FlexDirection::Row)
        && (matches!(layout.height, Some(LengthSpec::Fill))
            || (layout.height.is_none() && cross_stretch));
    if main.is_none() && !cross_fill_width && !cross_fill_height {
        return content;
    }
    let mut c = container(content);
    match parent_direction {
        FlexDirection::Row => {
            if let Some(main) = main {
                c = c.width(length_from_spec(Some(main), parent_box.width, layout, false));
            }
            if cross_fill_height {
                // Stretch ≠ scrollport: under an indefinite cross CB (auto grid
                // tracks) pin to content, not viewport Fixed/Fill — otherwise
                // iced measures the auto track as the full remainder and 1fr
                // collapses.
                c = c.height(cross_axis_stretch_length(
                    true,
                    parent_box.height,
                    layout,
                ));
            }
        }
        FlexDirection::Column => {
            if let Some(main) = main {
                let mut h = length_from_spec(Some(main), parent_box.height, layout, true);
                // Grow/Fill inside an indefinite column CB (typical scroll
                // content) collapses to 0 under iced's infinite max. Keep
                // intrinsic height for non-scrollport flex items; the
                // scrollport itself owns viewport sizing via scrolls_y.
                if parent_box.height.is_none()
                    && !layout.scrolls_y()
                    && matches!(h, Length::Fill | Length::FillPortion(_))
                {
                    h = Length::Fit;
                }
                c = c.height(h);
            }
            if cross_fill_width {
                c = c.width(cross_axis_stretch_length(
                    false,
                    parent_box.width,
                    layout,
                ));
            }
        }
    }
    if layout.has_surface_paint() {
        let paint = surface_paint_from(layout);
        c = c.style(move |_theme| surface_style(paint));
    }
    c.into()
}

/// content-box declared px → outer iced Fixed (border-box).
/// Independent of [`WidgetBoxConsume`]: consume only skips outer draw.
fn content_box_outer_fixed(declared: f32, pad_start: f32, pad_end: f32, border: f32) -> f32 {
    declared + pad_start + pad_end + 2.0 * border
}

/// Outer Fixed axes after content-box chrome expansion (no `main_override`).
/// Used by tests to assert consume does not shrink the flex-allocated border-box.
/// Any [`LengthSpec::is_definite_declared`] that resolves to Fixed expands
/// (Px/%/calc, em/rem, vw, min/clamp, …) — same rule as measure.
#[cfg(test)]
fn content_box_outer_axes(
    layout: &crate::css_map::LayoutStyle,
    parent_width: Option<f32>,
) -> (Option<f32>, Option<f32>) {
    if !matches!(layout.box_sizing, BoxSizing::ContentBox) {
        return (None, None);
    }
    let pad = layout.resolved_padding_against(parent_width);
    let bw = layout.resolved_border_width();
    let width = layout
        .width
        .filter(|s| s.is_definite_declared())
        .and_then(|w| match length_from_spec(Some(w), parent_width, layout, false) {
            Length::Fixed(px) => Some(content_box_outer_fixed(px, pad.left, pad.right, bw)),
            _ => None,
        });
    let height = layout
        .height
        .filter(|s| s.is_definite_declared())
        .and_then(|h| match length_from_spec(Some(h), None, layout, true) {
            Length::Fixed(px) => Some(content_box_outer_fixed(px, pad.top, pad.bottom, bw)),
            _ => None,
        });
    (width, height)
}

/// 控件级盒模型：主轴 flex、% 宽高、min-width:0、margin。
fn apply_widget_box_model<'a, Message: 'a>(
    content: Element<'a, Message>,
    layout: &crate::css_map::LayoutStyle,
    parent_box: ParentBox,
    parent_direction: FlexDirection,
    parent_align_items: AlignSpec,
    main_override: Option<f32>,
    consume: WidgetBoxConsume,
) -> Element<'a, Message> {
    let pad = layout.resolved_padding_against(parent_box.width);
    let mut margin = layout.resolved_margin_against(parent_box.width);
    // Approximate CSS `position: relative` inset by folding into outer margin.
    // Absolute is measure-only in iced flow. Fixed paints via root viewport
    // layer. Sticky stays deferred. Product floats use Nana Overlay.
    let (dx, dy) = layout.relative_offset_against(parent_box.width, parent_box.height);
    if dx != 0.0 || dy != 0.0 {
        margin.left = (margin.left + dx).max(0.0);
        margin.top = (margin.top + dy).max(0.0);
    }
    let main = layout.child_main_length(parent_direction);
    let mut width = match parent_direction {
        FlexDirection::Row => {
            if let Some(px) = main_override {
                Some(Length::Fixed(px.max(0.0)))
            } else {
                main.map(|m| length_from_spec(Some(m), parent_box.width, layout, false))
            }
        }
        FlexDirection::Column => layout
            .width
            .map(|w| length_from_spec(Some(w), parent_box.width, layout, false)),
    };
    let mut height = match parent_direction {
        FlexDirection::Column => {
            if let Some(px) = main_override {
                Some(Length::Fixed(px.max(0.0)))
            } else {
                main.map(|m| length_from_spec(Some(m), parent_box.height, layout, true))
            }
        }
        FlexDirection::Row => layout
            .height
            .map(|h| length_from_spec(Some(h), parent_box.height, layout, true))
            .or_else(|| {
                let mh = layout
                    .resolved_min_height(parent_box.height, crate::css_map::active_viewport());
                (mh > 0.0).then_some(Length::Fixed(mh))
            }),
    };

    // content-box: iced Fixed is outer (border-box); expand declared content by
    // padding + border. Match measure: any `is_definite_declared` axis that
    // already resolved to Fixed (Px/%/calc, em/rem, vw, min/clamp, …).
    // `main_override` is already border-box — do not double-add.
    // `consume` only skips outer pad/paint *drawing*; expansion still uses full
    // chrome so Fixed matches measure's border-box (Button inner paint included).
    let bw = layout.resolved_border_width();
    if matches!(layout.box_sizing, BoxSizing::ContentBox) && main_override.is_none() {
        let width_spec = match parent_direction {
            FlexDirection::Row => main,
            FlexDirection::Column => layout.width,
        };
        if let Some(Length::Fixed(px)) = width {
            if width_spec.is_some_and(LengthSpec::is_definite_declared) {
                width = Some(Length::Fixed(content_box_outer_fixed(
                    px,
                    pad.left,
                    pad.right,
                    bw,
                )));
            }
        }
        let height_spec = match parent_direction {
            FlexDirection::Column => main,
            FlexDirection::Row => layout.height,
        };
        if let Some(Length::Fixed(px)) = height {
            if height_spec.is_some_and(LengthSpec::is_definite_declared) {
                height = Some(Length::Fixed(content_box_outer_fixed(
                    px,
                    pad.top,
                    pad.bottom,
                    bw,
                )));
            }
        }
    }

    // P0-5: min-width:0 → Fill so flex item can shrink below content size
    if layout.allow_shrink || layout.has_zero_min_width() {
        if width.is_none() && matches!(parent_direction, FlexDirection::Row) {
            width = Some(Length::Fill);
        }
        if width.is_none() && layout.width.is_none() {
            // still allow shrink in nested rows
        }
    }
    // Non-zero min-width floors iced Length (matches measure clamp).
    let mw = layout.resolved_min_width(parent_box.width, crate::css_map::active_viewport());
    if mw > 0.0 {
        width = Some(match width {
            Some(Length::Fixed(px)) => Length::Fixed(px.max(mw)),
            Some(w) => w.min(mw),
            None => Length::Fixed(mw),
        });
    }

    // Cross-axis stretch from parent `align-items` / own `align-self`.
    let cross_stretch = matches!(
        layout.resolved_align_self(parent_align_items),
        AlignSpec::Stretch
    );
    if cross_stretch {
        match parent_direction {
            FlexDirection::Row if height.is_none() => {
                height = Some(cross_axis_stretch_length(
                    true,
                    parent_box.height,
                    layout,
                ));
            }
            FlexDirection::Column if width.is_none() => {
                width = Some(cross_axis_stretch_length(
                    false,
                    parent_box.width,
                    layout,
                ));
            }
            _ => {}
        }
    }

    // Fold border into padding so iced content matches CSS content box
    // (border paint stays via surface style; layout chrome includes bw).
    // Button/IconButton consume padding+paint themselves — skip outer draw only.
    let fold_bw = if consume.paint { 0.0 } else { bw };
    let layout_pad = if consume.padding {
        Padding::ZERO
    } else {
        Padding {
            top: pad.top + fold_bw,
            right: pad.right + fold_bw,
            bottom: pad.bottom + fold_bw,
            left: pad.left + fold_bw,
        }
    };
    let needs_pad = layout_pad.top > 0.0
        || layout_pad.right > 0.0
        || layout_pad.bottom > 0.0
        || layout_pad.left > 0.0;
    let needs_margin = !margin.is_zero();
    let needs_paint = layout.has_surface_paint() && !consume.paint;
    if width.is_none()
        && height.is_none()
        && !needs_pad
        && !needs_margin
        && !needs_paint
        && !layout.scrolls_y()
    {
        return content;
    }

    let mut c = container(content);
    if let Some(w) = width {
        c = c.width(w);
    } else if layout.allow_shrink {
        c = c.width(Length::Fill);
    }
    if let Some(h) = height {
        c = c.height(h);
    }
    if needs_pad {
        c = c.padding(layout_pad);
    }
    if needs_paint {
        let paint = surface_paint_from(layout);
        c = c.style(move |_theme| surface_style(paint));
    }
    let mut el: Element<'a, Message> = c.into();
    if needs_margin {
        el = container(el)
            .padding(Padding {
                top: margin.top,
                right: margin.right,
                bottom: margin.bottom,
                left: margin.left,
            })
            .into();
    }
    el
}

fn length_from_spec(
    spec: Option<LengthSpec>,
    percent_base: Option<f32>,
    layout: &crate::css_map::LayoutStyle,
    vertical: bool,
) -> Length {
    // Only demote Fill on the auto-track *main* axis (height for row tracks,
    // width for column tracks) — cross-axis Fill must keep stretching.
    let demote_fill = matches!(
        crate::css_map::intrinsic_auto_track_vertical(),
        Some(v) if v == vertical
    );
    let fill_or_shrink = || {
        if demote_fill {
            // Intrinsic auto-track: Fit = measure contents, do not compress.
            Length::Fit
        } else {
            Length::Fill
        }
    };
    let grow_or_shrink = || {
        if demote_fill {
            Length::Fit
        } else {
            flex_grow_length(layout)
        }
    };
    match spec {
        None => {
            if layout.grows() {
                grow_or_shrink()
            } else if layout.allow_shrink {
                fill_or_shrink()
            } else {
                fill_or_shrink()
            }
        }
        Some(LengthSpec::Px(v)) => Length::Fixed(v.max(0.0)),
        Some(LengthSpec::Percent(p)) => {
            if let Some(base) = percent_base {
                Length::Fixed((base * p / 100.0).max(0.0))
            } else if (p - 100.0).abs() < 0.5 {
                fill_or_shrink()
            } else if demote_fill {
                Length::Fit
            } else {
                // Fallback when parent box unknown: FillPortion heuristic
                let portion = (p / 10.0).round().clamp(1.0, 10.0) as u16;
                Length::FillPortion(portion)
            }
        }
        Some(LengthSpec::CalcPercentOffset { percent, offset_px }) => {
            if let Some(base) = percent_base {
                Length::Fixed((base * percent / 100.0 + offset_px).max(0.0))
            } else if (percent - 100.0).abs() < 0.5 && offset_px <= 0.0 {
                // Parent unknown: approximate `calc(100% - Npx)` as Fill.
                fill_or_shrink()
            } else {
                Length::Fit
            }
        }
        Some(
            spec @ (LengthSpec::Em(_)
            | LengthSpec::Rem(_)
            | LengthSpec::Viewport { .. }
            | LengthSpec::CalcViewportOffset { .. }
            | LengthSpec::CalcEmOffset { .. }
            | LengthSpec::CalcRemOffset { .. }
            | LengthSpec::Min2(_, _)
            | LengthSpec::Max2(_, _)
            | LengthSpec::Clamp3(_, _, _)),
        ) => {
            let fonts = layout.font_size_context(crate::css_map::active_font_sizes().root_px);
            if let Some(px) =
                spec.resolve_with_fonts(percent_base, crate::css_map::active_viewport(), fonts)
            {
                Length::Fixed(px.max(0.0))
            } else {
                Length::Fit
            }
        }
        Some(LengthSpec::Fill) => {
            if layout.grows() {
                grow_or_shrink()
            } else {
                fill_or_shrink()
            }
        }
        Some(LengthSpec::Shrink) => Length::Shrink,
        Some(LengthSpec::Auto) => {
            if layout.grows() {
                grow_or_shrink()
            } else {
                Length::Fit
            }
        }
    }
}

/// Map `flex-grow` to iced `FillPortion` so weighted siblings share free space.
fn flex_grow_length(layout: &crate::css_map::LayoutStyle) -> Length {
    Length::FillPortion(fr_portion(layout.flex_grow.unwrap_or(1.0).max(0.0)))
}

fn align_from_spec(spec: AlignSpec) -> Alignment {
    match spec {
        AlignSpec::Start => Alignment::Start,
        AlignSpec::Center => Alignment::Center,
        AlignSpec::End => Alignment::End,
        AlignSpec::Stretch => Alignment::Start, // cross-axis Fill applied on children
    }
}
