// Button / IconButton mapping onto nana_ui controls (L2 → L3).

fn resolve_icon_from_props(props: &WidgetProps) -> Option<Icon> {
    if !props.value.is_empty() {
        if let Some(icon) = Icon::parse_name(&props.value) {
            return Some(icon);
        }
    }
    Icon::parse_name(props.display_label())
        .or_else(|| Icon::parse_name(props.role.as_str()))
        .or_else(|| props.class_names.iter().find_map(|c| Icon::parse_name(c)))
}

/// Drawable for a toolbar / Lucide button: prefer true SVG, else shell glyph.
#[derive(Debug, Clone)]
enum ResolvedButtonIcon {
    Svg(SvgHandle),
    Glyph(Icon),
    /// Lucide present but no serializable geometry yet — keep icon slot, avoid caption bleed.
    Placeholder,
}

fn find_descendant_glyph(snap: &SemanticSnapshot, id: WidgetId) -> Option<Icon> {
    let w = snap.get(id)?;
    if matches!(w.kind, WidgetKind::Icon) {
        if let Some(glyph) = resolve_icon_from_props(&w.props) {
            return Some(glyph);
        }
    }
    if let Some(glyph) = w.props.class_names.iter().find_map(|c| Icon::parse_name(c)) {
        return Some(glyph);
    }
    for &child in &w.children {
        if let Some(glyph) = find_descendant_glyph(snap, child) {
            return Some(glyph);
        }
    }
    None
}

fn has_lucide_descendant(snap: &SemanticSnapshot, id: WidgetId) -> bool {
    let Some(w) = snap.get(id) else {
        return false;
    };
    if crate::svg_icon::is_svg_icon_root(w)
        || w.props
            .class_names
            .iter()
            .any(|c| c == "lucide" || c.starts_with("lucide-"))
    {
        return true;
    }
    w.children
        .iter()
        .copied()
        .any(|child| has_lucide_descendant(snap, child))
}

/// Toolbar / Lucide buttons: SVG geometry first, then shell glyph, then caption text.
/// Icon-only overview buttons otherwise paint as empty `Button::label("")` and vanish.
fn resolve_button_icon_and_label(
    snap: &SemanticSnapshot,
    props: &WidgetProps,
    children: &[WidgetId],
) -> (Option<ResolvedButtonIcon>, String) {
    let mut resolved = children.iter().copied().find_map(|id| {
        crate::svg_icon::try_svg_handle(snap, id).map(ResolvedButtonIcon::Svg)
    });
    if resolved.is_none() {
        resolved = children
            .iter()
            .copied()
            .find_map(|id| find_descendant_glyph(snap, id))
            .or_else(|| resolve_icon_from_props(props))
            .map(ResolvedButtonIcon::Glyph);
    }
    if resolved.is_none()
        && children
            .iter()
            .copied()
            .any(|id| has_lucide_descendant(snap, id))
    {
        resolved = Some(ResolvedButtonIcon::Placeholder);
    }
    let mut label = String::new();
    for &child in children {
        // Only real text nodes — never SVG `d` path data / Lucide glyph names.
        label = collect_button_caption_text(snap, child);
        if !label.is_empty() {
            break;
        }
    }
    if label.is_empty() {
        let own = props.display_label().trim();
        // Prefer short visible captions; long aria/title stay as tooltip via hint.
        if !own.is_empty()
            && Icon::parse_name(own).is_none()
            && !looks_like_svg_path(own)
            && !own.starts_with("lucide")
            && own.chars().count() <= 8
        {
            label = own.to_string();
        }
    }
    (resolved, label)
}

/// SidebarRow / ListItem: leading icon (SVG/glyph) + caption from children or props.
fn resolve_row_leading_and_label<'a, Message: 'a>(
    snap: &'a SemanticSnapshot,
    props: &WidgetProps,
    children: &[WidgetId],
    tokens: ThemeTokens,
) -> (Option<Element<'a, Message>>, String) {
    let (resolved, mut label) = resolve_button_icon_and_label(snap, props, children);
    if label.is_empty() {
        label = props.display_label().to_string();
    }
    let fg = props
        .layout
        .color
        .map(rgba_color)
        .unwrap_or(tokens.colors.text);
    let size = children
        .iter()
        .copied()
        .find_map(|id| snap.get(id).map(|w| crate::svg_icon::resolve_icon_size(&w.props)))
        .unwrap_or_else(|| props.size.icon_size());
    let leading = resolved
        .as_ref()
        .map(|glyph| button_icon_element(glyph, size, fg));
    (leading, label)
}

fn resolve_row_leading_and_label_owned<Message: 'static>(
    snap: &SemanticSnapshot,
    props: &WidgetProps,
    children: &[WidgetId],
    tokens: ThemeTokens,
) -> (Option<Element<'static, Message>>, String) {
    let (resolved, mut label) = resolve_button_icon_and_label(snap, props, children);
    if label.is_empty() {
        label = owned_display(props);
    }
    let fg = props
        .layout
        .color
        .map(rgba_color)
        .unwrap_or(tokens.colors.text);
    let size = children
        .iter()
        .copied()
        .find_map(|id| snap.get(id).map(|w| crate::svg_icon::resolve_icon_size(&w.props)))
        .unwrap_or_else(|| props.size.icon_size());
    let leading = resolved.map(|glyph| match glyph {
        ResolvedButtonIcon::Svg(handle) => {
            crate::svg_icon::svg_icon_element::<Message>(handle, size, fg)
        }
        ResolvedButtonIcon::Glyph(kind) => icon(kind, size, fg),
        ResolvedButtonIcon::Placeholder => crate::svg_icon::empty_icon_placeholder(size),
    });
    (leading, label)
}

fn button_icon_element<'a, Message: 'a>(
    resolved: &ResolvedButtonIcon,
    size: f32,
    color: Color,
) -> Element<'a, Message> {
    match resolved {
        ResolvedButtonIcon::Svg(handle) => {
            crate::svg_icon::svg_icon_element(handle.clone(), size, color)
        }
        ResolvedButtonIcon::Glyph(kind) => icon(*kind, size, color),
        ResolvedButtonIcon::Placeholder => crate::svg_icon::empty_icon_placeholder(size),
    }
}

fn collect_button_caption_text(snap: &SemanticSnapshot, id: WidgetId) -> String {
    let Some(w) = snap.get(id) else {
        return String::new();
    };
    match w.kind {
        WidgetKind::Text => {
            let t = w.props.display_label().trim();
            if t.is_empty() || Icon::parse_name(t).is_some() || looks_like_svg_path(t) {
                String::new()
            } else {
                t.to_string()
            }
        }
        WidgetKind::Icon | WidgetKind::Box => String::new(),
        _ => {
            for &child in &w.children {
                let t = collect_button_caption_text(snap, child);
                if !t.is_empty() {
                    return t;
                }
            }
            String::new()
        }
    }
}

fn is_square_icon_button(props: &WidgetProps) -> bool {
    match (props.layout.width, props.layout.height) {
        (Some(LengthSpec::Px(w)), Some(LengthSpec::Px(h))) => {
            (w - h).abs() < 0.5 && (24.0..=48.0).contains(&w)
        }
        _ => false,
    }
}

fn is_compact_chip_button(props: &WidgetProps) -> bool {
    props.layout.white_space_nowrap
        && matches!(props.layout.height, Some(LengthSpec::Px(h)) if (1.0..26.0).contains(&h))
        && matches!(
            props.layout.width,
            Some(LengthSpec::Auto) | None
        )
}

fn resolved_button_kind(props: &WidgetProps) -> ButtonKind {
    if matches!(props.button_kind, ButtonKind::Primary | ButtonKind::Danger) {
        props.button_kind
    } else if is_square_icon_button(props) {
        // Align with IconButton::selected: active square toolbar buttons use Selected.
        if props.active {
            ButtonKind::Selected
        } else {
            ButtonKind::Ghost
        }
    } else {
        props.button_kind
    }
}

/// Which outer box-model slices Button/IconButton already applied internally.
#[derive(Clone, Copy, Default)]
struct WidgetBoxConsume {
    padding: bool,
    paint: bool,
}

fn layout_has_explicit_padding(layout: &crate::css_map::LayoutStyle) -> bool {
    layout.padding.is_some()
        || layout.padding_top.is_some()
        || layout.padding_right.is_some()
        || layout.padding_bottom.is_some()
        || layout.padding_left.is_some()
}

fn button_box_consume(kind: &WidgetKind, layout: &crate::css_map::LayoutStyle) -> WidgetBoxConsume {
    if !matches!(kind, WidgetKind::Button | WidgetKind::Chip) {
        return WidgetBoxConsume::default();
    }
    WidgetBoxConsume {
        // Explicit CSS padding (including `padding: 0`) is applied on the
        // iced Button itself — skip the outer container to avoid double pad.
        padding: layout_has_explicit_padding(layout),
        // Surface paint (bg / radius / border) is applied via ButtonPaintOverride.
        paint: layout.has_surface_paint() || layout.color.is_some(),
    }
}

fn button_paint_from_layout(layout: &crate::css_map::LayoutStyle) -> ButtonPaintOverride {
    ButtonPaintOverride {
        background: layout.background.map(rgba_color),
        text_color: layout.color.map(rgba_color),
        border_radius: layout.border_radius,
        border_width: layout.border_width,
        border_color: layout.border_color.map(rgba_color),
    }
}

fn button_padding_from_layout(
    layout: &crate::css_map::LayoutStyle,
    parent_width: Option<f32>,
) -> Option<Padding> {
    if !layout_has_explicit_padding(layout) {
        return None;
    }
    let pad = layout.resolved_padding_against(parent_width);
    Some(Padding {
        top: pad.top,
        right: pad.right,
        bottom: pad.bottom,
        left: pad.left,
    })
}

fn button_fixed_axis(spec: Option<LengthSpec>) -> Option<f32> {
    match spec {
        Some(LengthSpec::Px(px)) if px.is_finite() && px > 0.0 => Some(px),
        _ => None,
    }
}

fn button_control_size(props: &WidgetProps) -> ControlSize {
    if is_square_icon_button(props) {
        return ControlSize::Medium;
    }
    if let Some(h) = button_fixed_axis(props.layout.height) {
        return ControlSize::nearest(h);
    }
    if matches!(props.button_kind, ButtonKind::Primary) {
        ControlSize::Medium
    } else {
        props.size
    }
}

fn button_fg_color(props: &WidgetProps, tokens: ThemeTokens, kind: ButtonKind) -> Color {
    props
        .layout
        .color
        .map(rgba_color)
        .unwrap_or_else(|| match kind {
            ButtonKind::Primary => tokens.colors.accent_on_soft,
            ButtonKind::Warning => tokens.colors.warning,
            ButtonKind::Danger => tokens.colors.danger,
            ButtonKind::Text => tokens.colors.accent,
            ButtonKind::Ghost | ButtonKind::Subtle | ButtonKind::Selected => tokens.colors.text,
        })
}

fn apply_button_layout_chrome<'a, Message: Clone + 'a>(
    mut btn: Button<'a, Message>,
    props: &WidgetProps,
    parent_width: Option<f32>,
) -> Button<'a, Message> {
    let paint = button_paint_from_layout(&props.layout);
    if !paint.is_empty() {
        btn = btn.paint(paint);
    }
    if let Some(pad) = button_padding_from_layout(&props.layout, parent_width) {
        btn = btn.padding(Some(pad));
    }
    if let Some(h) = button_fixed_axis(props.layout.height) {
        btn = btn.height(Some(h));
    }
    if let Some(w) = button_fixed_axis(props.layout.width) {
        btn = btn.width(Length::Fixed(w));
    } else if matches!(props.button_kind, ButtonKind::Primary) || is_compact_chip_button(props) {
        btn = btn.width(Length::Shrink);
    }
    btn
}

fn apply_icon_button_layout_chrome<'a, Message: Clone + 'a>(
    mut btn: IconButton<'a, Message>,
    props: &WidgetProps,
    parent_width: Option<f32>,
) -> IconButton<'a, Message> {
    let paint = button_paint_from_layout(&props.layout);
    if !paint.is_empty() {
        btn = btn.paint(paint);
    }
    if let Some(pad) = button_padding_from_layout(&props.layout, parent_width) {
        btn = btn.padding(Some(pad));
    }
    if let Some(w) = button_fixed_axis(props.layout.width) {
        btn = btn.width(Some(w));
    }
    if let Some(h) = button_fixed_axis(props.layout.height) {
        btn = btn.height(Some(h));
    }
    btn
}

fn button_view<'a, Message: Clone + 'a>(
    props: &'a WidgetProps,
    children: &[WidgetId],
    snap: &'a SemanticSnapshot,
    tokens: ThemeTokens,
    parent_width: Option<f32>,
    on_press: Message,
) -> Element<'a, Message> {
    let (resolved_icon, label) = resolve_button_icon_and_label(snap, props, children);
    let icon_only = is_square_icon_button(props)
        || (resolved_icon.is_some() && label.is_empty());
    let btn_kind = resolved_button_kind(props);
    let size = button_control_size(props);
    let gap = props.layout.gap_or(6.0);
    let fg = button_fg_color(props, tokens, btn_kind);
    if let (Some(glyph), true) = (resolved_icon.as_ref(), icon_only) {
        let caption = if !props.hint.is_empty() {
            props.hint.clone()
        } else if !label.is_empty() {
            label.clone()
        } else {
            props.display_label().to_string()
        };
        // Prefer SVG / placeholder in a square button + tooltip (IconButton is enum-only).
        if matches!(
            glyph,
            ResolvedButtonIcon::Svg(_) | ResolvedButtonIcon::Placeholder
        ) {
            let icon_el = button_icon_element(glyph, size.icon_size(), fg);
            let mut btn = Button::new(icon_el)
                .kind(btn_kind)
                .size(size)
                .disabled(props.disabled)
                .on_press(on_press);
            btn = apply_button_layout_chrome(btn, props, parent_width);
            let trigger = btn.view(tokens);
            if caption.is_empty() {
                return trigger;
            }
            return Tooltip::new(trigger, text(caption).size(11))
                .config(TooltipConfig {
                    placement: TooltipPlacement::Bottom,
                    ..TooltipConfig::default()
                })
                .view(tokens);
        }
        if let ResolvedButtonIcon::Glyph(kind) = glyph {
            let btn = IconButton::new(caption, *kind)
                .kind(btn_kind)
                .size(size)
                .disabled(props.disabled)
                .selected(props.active)
                .on_press(on_press);
            return apply_icon_button_layout_chrome(btn, props, parent_width).view(tokens);
        }
    }
    if let Some(glyph) = resolved_icon.as_ref() {
        let text_size = props
            .layout
            .font_size
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or_else(|| size.text_size());
        let content: Element<'a, Message> = if label.is_empty() {
            button_icon_element(glyph, size.icon_size(), fg)
        } else {
            let weight = css_font_weight_to_iced(props.layout.font_weight);
            row![
                button_icon_element(glyph, size.icon_size(), fg),
                text(label.clone())
                    .size(text_size)
                    .font(ui_font(weight))
                    .color(fg),
            ]
            .spacing(gap)
            .align_y(Alignment::Center)
            .into()
        };
        let btn = Button::new(content)
            .kind(btn_kind)
            .size(size)
            .disabled(props.disabled)
            .loading(props.loading, 0)
            .on_press(on_press);
        return apply_button_layout_chrome(btn, props, parent_width).view(tokens);
    }
    let caption = if label.is_empty() {
        props.display_label().to_string()
    } else {
        label
    };
    let kind = if is_compact_chip_button(props) {
        ButtonKind::Ghost
    } else {
        btn_kind
    };
    // CSS typography / color: build labelled content so LayoutStyle wins over
    // Button::label Medium defaults.
    let btn = if props.layout.font_weight.is_some()
        || props.layout.font_size.is_some()
        || props.layout.color.is_some()
    {
        let text_size = props
            .layout
            .font_size
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or_else(|| size.text_size());
        let weight = css_font_weight_to_iced(props.layout.font_weight);
        let content = text(caption)
            .size(text_size)
            .font(ui_font(weight))
            .color(button_fg_color(props, tokens, kind));
        Button::new(content)
            .kind(kind)
            .size(size)
            .disabled(props.disabled)
            .loading(props.loading, 0)
            .on_press(on_press)
    } else {
        Button::label(caption)
            .kind(kind)
            .size(size)
            .disabled(props.disabled)
            .loading(props.loading, 0)
            .on_press(on_press)
    };
    apply_button_layout_chrome(btn, props, parent_width).view(tokens)
}

fn button_view_owned<Message: Clone + 'static>(
    props: &WidgetProps,
    children: &[WidgetId],
    snap: &SemanticSnapshot,
    tokens: ThemeTokens,
    parent_width: Option<f32>,
    on_press: Message,
) -> Element<'static, Message> {
    let (resolved_icon, label) = resolve_button_icon_and_label(snap, props, children);
    let icon_only = is_square_icon_button(props)
        || (resolved_icon.is_some() && label.is_empty());
    let btn_kind = resolved_button_kind(props);
    let size = button_control_size(props);
    let gap = props.layout.gap_or(6.0);
    let fg = button_fg_color(props, tokens, btn_kind);
    if let (Some(glyph), true) = (resolved_icon.as_ref(), icon_only) {
        let caption = if !props.hint.is_empty() {
            props.hint.clone()
        } else if !label.is_empty() {
            label.clone()
        } else {
            owned_display(props)
        };
        if matches!(
            glyph,
            ResolvedButtonIcon::Svg(_) | ResolvedButtonIcon::Placeholder
        ) {
            let icon_el = button_icon_element(glyph, size.icon_size(), fg);
            let mut btn = Button::new(icon_el)
                .kind(btn_kind)
                .size(size)
                .disabled(props.disabled)
                .on_press(on_press);
            btn = apply_button_layout_chrome(btn, props, parent_width);
            let trigger = btn.view(tokens);
            if caption.is_empty() {
                return trigger;
            }
            return Tooltip::new(trigger, text(caption).size(11))
                .config(TooltipConfig {
                    placement: TooltipPlacement::Bottom,
                    ..TooltipConfig::default()
                })
                .view(tokens);
        }
        if let ResolvedButtonIcon::Glyph(kind) = glyph {
            let btn = IconButton::new(caption, *kind)
                .kind(btn_kind)
                .size(size)
                .disabled(props.disabled)
                .selected(props.active)
                .on_press(on_press);
            return apply_icon_button_layout_chrome(btn, props, parent_width).view(tokens);
        }
    }
    if let Some(glyph) = resolved_icon.as_ref() {
        let text_size = props
            .layout
            .font_size
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or_else(|| size.text_size());
        let content: Element<'static, Message> = if label.is_empty() {
            button_icon_element(glyph, size.icon_size(), fg)
        } else {
            let weight = css_font_weight_to_iced(props.layout.font_weight);
            row![
                button_icon_element(glyph, size.icon_size(), fg),
                text(label)
                    .size(text_size)
                    .font(ui_font(weight))
                    .color(fg),
            ]
            .spacing(gap)
            .align_y(Alignment::Center)
            .into()
        };
        let btn = Button::new(content)
            .kind(btn_kind)
            .size(size)
            .disabled(props.disabled)
            .loading(props.loading, 0)
            .on_press(on_press);
        return apply_button_layout_chrome(btn, props, parent_width).view(tokens);
    }
    let caption = if label.is_empty() {
        owned_display(props)
    } else {
        label
    };
    let kind = if is_compact_chip_button(props) {
        ButtonKind::Ghost
    } else {
        btn_kind
    };
    let btn = if props.layout.font_weight.is_some()
        || props.layout.font_size.is_some()
        || props.layout.color.is_some()
    {
        let text_size = props
            .layout
            .font_size
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or_else(|| size.text_size());
        let weight = css_font_weight_to_iced(props.layout.font_weight);
        let content = text(caption)
            .size(text_size)
            .font(ui_font(weight))
            .color(button_fg_color(props, tokens, kind));
        Button::new(content)
            .kind(kind)
            .size(size)
            .disabled(props.disabled)
            .loading(props.loading, 0)
            .on_press(on_press)
    } else {
        Button::label(caption)
            .kind(kind)
            .size(size)
            .disabled(props.disabled)
            .loading(props.loading, 0)
            .on_press(on_press)
    };
    apply_button_layout_chrome(btn, props, parent_width).view(tokens)
}
