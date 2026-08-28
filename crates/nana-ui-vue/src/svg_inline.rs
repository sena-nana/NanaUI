//! Inline SVG → the shared image URL / GPU texture cache.
//!
//! Structural `<svg>` trees are serialized to `data:image/svg+xml;base64,...`
//! and bound as [`PaintStyle::content_image`] so `image_url` / resvg rasterizes
//! them with the same decode and URL-keyed texture cache as `background-image`
//! and `<img src>`. Lucide roots stay [`WidgetKind::Icon`]. Descendants are
//! omitted from CSS layout so `path` / `rect` / `circle` are not empty boxes.

use base64::Engine as _;
use nana_ui_core::BackgroundImageFit;

use crate::bridge::{MessageBridge, WidgetId, WidgetKind, WidgetProps};
use crate::css_paint::apply_img_replaced_content;

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const SKIP_ATTRS: &[&str] = &[
    "class",
    "classname",
    "style",
    "innerhtml",
    "textcontent",
    "data-nana-ns",
    "data-nana-canvas",
    "data-nana-gpu",
    "data-nana-image",
];

pub(crate) fn nearest_svg_root(bridge: &MessageBridge, mut id: WidgetId) -> Option<WidgetId> {
    loop {
        let widget = bridge.get(id)?;
        if widget.props.element_tag.eq_ignore_ascii_case("svg") {
            return Some(id);
        }
        id = widget.parent?;
    }
}

pub(crate) fn is_lucide_svg(kind: WidgetKind, props: &WidgetProps) -> bool {
    kind == WidgetKind::Icon
        || props
            .class_names
            .iter()
            .any(|class| class == "lucide" || class.starts_with("lucide-"))
}

pub(crate) fn apply_inline_svg_replaced(
    bridge: &MessageBridge,
    id: WidgetId,
    layout: &mut nana_ui_core::LayoutStyle,
) {
    let Some(widget) = bridge.get(id) else {
        return;
    };
    let Some(root) = nearest_svg_root(bridge, id) else {
        return;
    };
    if root != id {
        layout.hidden = true;
        return;
    }
    if is_lucide_svg(widget.kind, &widget.props) {
        return;
    }
    let Some(url) = serialize_svg_data_url(bridge, id, layout.color) else {
        return;
    };
    let fit = layout
        .paint
        .object_fit
        .unwrap_or(BackgroundImageFit::Contain);
    apply_img_replaced_content(layout, &url);
    if let Some(nana_ui_core::BackgroundImage::Url { fit: slot, .. }) =
        layout.paint.content_image.as_mut()
    {
        *slot = fit;
    }
}

pub(crate) fn serialize_svg_data_url(
    bridge: &MessageBridge,
    root: WidgetId,
    used_color: Option<[f32; 4]>,
) -> Option<String> {
    let markup = serialize_svg_markup(bridge, root, used_color)?;
    if markup.is_empty() {
        return None;
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(markup.as_bytes());
    Some(format!("data:image/svg+xml;base64,{encoded}"))
}

fn serialize_svg_markup(
    bridge: &MessageBridge,
    root: WidgetId,
    used_color: Option<[f32; 4]>,
) -> Option<String> {
    let widget = bridge.get(root)?;
    if !widget.props.element_tag.eq_ignore_ascii_case("svg") {
        return None;
    }
    let mut out = String::new();
    write_element(bridge, root, true, used_color, &mut out);
    Some(out)
}

fn write_element(
    bridge: &MessageBridge,
    id: WidgetId,
    is_root: bool,
    inherited_color: Option<[f32; 4]>,
    out: &mut String,
) {
    let Some(widget) = bridge.get(id) else {
        return;
    };
    let tag = widget.props.element_tag.trim();
    if tag.is_empty() || tag == "#text" {
        xml_escape_into(&widget.props.label, out);
        return;
    }
    let used_color = widget.props.layout.color.or(inherited_color);
    out.push('<');
    out.push_str(tag);
    if is_root && !has_attr(&widget.props, "xmlns") {
        out.push_str(" xmlns=\"");
        out.push_str(SVG_NS);
        out.push('"');
    }
    write_attrs(&widget.props, used_color, is_root, out);
    if is_root {
        write_root_color(&widget.props, used_color, out);
    }
    let children = widget.children.clone();
    let label = widget.props.label.clone();
    if children.is_empty() && label.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    if children.is_empty() {
        xml_escape_into(&label, out);
    } else {
        for child in children {
            write_element(bridge, child, false, used_color, out);
        }
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

fn write_attrs(props: &WidgetProps, used_color: Option<[f32; 4]>, is_root: bool, out: &mut String) {
    let mut wrote_fill = has_attr(props, "fill");
    let mut wrote_stroke = has_attr(props, "stroke");
    for (key, value) in &props.attrs {
        if skip_attr(key) || value.is_empty() {
            continue;
        }
        let name = svg_attr_name(key);
        let value = paint_attr_value(name, value, used_color);
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        xml_escape_into(&value, out);
        out.push('"');
    }
    if !props.inline_style.trim().is_empty() {
        let style = bake_current_color_token(props.inline_style.trim(), used_color);
        out.push_str(" style=\"");
        xml_escape_into(&style, out);
        out.push('"');
    }
    if !wrote_fill && let Some(color) = props.layout.background {
        out.push_str(" fill=\"");
        out.push_str(&rgba_to_svg(color));
        out.push('"');
        wrote_fill = true;
    }
    if !wrote_fill
        && is_root
        && let Some(color) = used_color
    {
        out.push_str(" fill=\"");
        out.push_str(&rgba_to_svg(color));
        out.push('"');
        wrote_fill = true;
    }
    let _ = wrote_fill;
    if !wrote_stroke && let Some(color) = props.layout.border_color {
        out.push_str(" stroke=\"");
        out.push_str(&rgba_to_svg(color));
        out.push('"');
        wrote_stroke = true;
        if !has_attr(props, "stroke-width")
            && let Some(width) = props.layout.border_width
            && width > 0.0
        {
            out.push_str(" stroke-width=\"");
            out.push_str(&format_float(width));
            out.push('"');
        }
    }
    let _ = wrote_stroke;
}

fn write_root_color(props: &WidgetProps, used_color: Option<[f32; 4]>, out: &mut String) {
    if has_attr(props, "color") {
        return;
    }
    let Some(color) = used_color else {
        return;
    };
    out.push_str(" color=\"");
    out.push_str(&rgba_to_svg(color));
    out.push('"');
}

fn paint_attr_value(name: &str, value: &str, used_color: Option<[f32; 4]>) -> String {
    if is_svg_paint_attr(name) && is_current_color(value) {
        if let Some(color) = used_color {
            return rgba_to_svg(color);
        }
        return value.to_string();
    }
    value.to_string()
}

fn is_svg_paint_attr(name: &str) -> bool {
    name.eq_ignore_ascii_case("fill")
        || name.eq_ignore_ascii_case("stroke")
        || name.eq_ignore_ascii_case("stop-color")
        || name.eq_ignore_ascii_case("flood-color")
        || name.eq_ignore_ascii_case("lighting-color")
        || name.eq_ignore_ascii_case("color")
}

fn is_current_color(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("currentcolor")
}

fn bake_current_color_token(input: &str, used_color: Option<[f32; 4]>) -> String {
    let Some(color) = used_color else {
        return input.to_string();
    };
    let needle = "currentcolor";
    let lower = input.to_ascii_lowercase();
    if !lower.contains(needle) {
        return input.to_string();
    }
    let baked = rgba_to_svg(color);
    let mut out = String::with_capacity(input.len());
    let mut start = 0;
    while let Some(rel) = lower[start..].find(needle) {
        let at = start + rel;
        out.push_str(&input[start..at]);
        out.push_str(&baked);
        start = at + needle.len();
    }
    out.push_str(&input[start..]);
    out
}

fn has_attr(props: &WidgetProps, name: &str) -> bool {
    props
        .attrs
        .keys()
        .any(|key| svg_attr_name(key).eq_ignore_ascii_case(name) || key.eq_ignore_ascii_case(name))
}

fn skip_attr(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SKIP_ATTRS.contains(&lower.as_str()) || lower.starts_with("data-nana-")
}

fn svg_attr_name(key: &str) -> &str {
    match key {
        "viewbox" | "view-box" => "viewBox",
        "preserveaspectratio" | "preserve-aspect-ratio" => "preserveAspectRatio",
        "pathlength" | "path-length" => "pathLength",
        "strokewidth" | "stroke-width" => "stroke-width",
        "strokelinecap" | "stroke-linecap" => "stroke-linecap",
        "strokelinejoin" | "stroke-linejoin" => "stroke-linejoin",
        "strokedasharray" | "stroke-dasharray" => "stroke-dasharray",
        "strokedashoffset" | "stroke-dashoffset" => "stroke-dashoffset",
        "fillopacity" | "fill-opacity" => "fill-opacity",
        "strokeopacity" | "stroke-opacity" => "stroke-opacity",
        "fillrule" | "fill-rule" => "fill-rule",
        "clippath" | "clip-path" => "clip-path",
        "stopcolor" | "stop-color" => "stop-color",
        "stopopacity" | "stop-opacity" => "stop-opacity",
        "gradienttransform" | "gradient-transform" => "gradientTransform",
        "gradientunits" | "gradient-units" => "gradientUnits",
        "spreadmethod" | "spread-method" => "spreadMethod",
        "xlink:href" | "xlinkhref" => "href",
        other => other,
    }
}

fn xml_escape_into(input: &str, out: &mut String) {
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
}

fn rgba_to_svg(color: [f32; 4]) -> String {
    let r = (color[0] * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (color[1] * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (color[2] * 255.0).round().clamp(0.0, 255.0) as u8;
    if color[3] < 0.995 {
        format!("rgba({r},{g},{b},{:.3})", color[3].clamp(0.0, 1.0))
    } else {
        format!("#{r:02x}{g:02x}{b:02x}")
    }
}

fn format_float(value: f32) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i32)
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{MessageBridge, WidgetKind, WidgetProps};
    use nana_js_engine::HostValue;

    #[test]
    fn inline_svg_path_becomes_content_image_data_url() {
        let mut bridge = MessageBridge::new();
        let mut svg = WidgetProps::default();
        svg.element_tag = "svg".into();
        svg.attrs.insert("viewBox".into(), "0 0 10 10".into());
        svg.layout.width = Some(nana_ui_core::LengthSpec::Px(10.0));
        svg.layout.height = Some(nana_ui_core::LengthSpec::Px(10.0));
        bridge.register(1, WidgetKind::Column, svg);
        let mut path = WidgetProps::default();
        path.element_tag = "path".into();
        path.attrs.insert("d".into(), "M0 0 H10 V10 H0 Z".into());
        path.attrs.insert("fill".into(), "#00ff00".into());
        bridge.register(2, WidgetKind::Box, path);
        bridge.insert_child(2, 1, None);

        let root = bridge.get(1).expect("svg");
        match &root.props.layout.paint.content_image {
            Some(nana_ui_core::BackgroundImage::Url { url, .. }) => {
                assert!(
                    url.starts_with("data:image/svg+xml;base64,"),
                    "inline svg must share the image_url data-url cache, got {url}"
                );
                let encoded = url.rsplit_once(',').expect("data url").1;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .expect("base64");
                let markup = String::from_utf8(bytes).expect("utf8 svg");
                assert!(
                    markup.contains("<path"),
                    "serialized svg must keep the path, got {markup}"
                );
                assert!(
                    markup.contains("M0 0 H10 V10 H0 Z"),
                    "path d must round-trip, got {markup}"
                );
                assert!(
                    markup.contains("xmlns="),
                    "root svg must declare xmlns, got {markup}"
                );
            }
            other => panic!("expected inline svg content_image, got {other:?}"),
        }
        assert!(
            bridge.get(2).expect("path").props.layout.hidden,
            "svg descendants must leave CSS layout so they are not empty boxes"
        );
    }

    #[test]
    fn lucide_svg_is_not_replaced_by_content_image() {
        let mut bridge = MessageBridge::new();
        let mut svg = WidgetProps::default();
        svg.element_tag = "svg".into();
        svg.class_names = vec!["lucide".into(), "lucide-search".into()];
        bridge.register(1, WidgetKind::Icon, svg);
        let layout = &bridge.get(1).expect("icon").props.layout;
        assert!(
            layout.paint.content_image.is_none(),
            "Lucide stays on the Icon painter"
        );
    }

    #[test]
    fn patching_path_d_rebuilds_svg_data_url() {
        let mut bridge = MessageBridge::new();
        let mut svg = WidgetProps::default();
        svg.element_tag = "svg".into();
        bridge.register(1, WidgetKind::Column, svg);
        let mut path = WidgetProps::default();
        path.element_tag = "path".into();
        path.attrs.insert("d".into(), "M0 0".into());
        bridge.register(2, WidgetKind::Box, path);
        bridge.insert_child(2, 1, None);
        let before = content_url(&bridge, 1);
        bridge.patch_prop(2, "d", &HostValue::string("M0 0 H4 V4 H0 Z"));
        let after = content_url(&bridge, 1);
        assert_ne!(
            before, after,
            "geometry patches must invalidate the URL key"
        );
        let encoded = after.rsplit_once(',').expect("data url").1;
        let markup = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .expect("base64"),
        )
        .expect("utf8");
        assert!(markup.contains("M0 0 H4 V4 H0 Z"), "got {markup}");
    }

    #[test]
    fn omitted_fill_bakes_inherited_css_color() {
        let mut bridge = MessageBridge::new();
        let mut parent = WidgetProps::default();
        parent.element_tag = "div".into();
        parent.inline_style = "color:#ba7a7a".into();
        bridge.register(1, WidgetKind::Column, parent);

        let mut svg = WidgetProps::default();
        svg.element_tag = "svg".into();
        svg.attrs.insert("viewBox".into(), "0 0 8 8".into());
        svg.attrs.insert("width".into(), "8".into());
        svg.attrs.insert("height".into(), "8".into());
        bridge.register(2, WidgetKind::Column, svg);
        bridge.insert_child(2, 1, None);

        let mut path = WidgetProps::default();
        path.element_tag = "path".into();
        path.attrs.insert("d".into(), "M0 0 H8 V8 H0 Z".into());
        bridge.register(3, WidgetKind::Box, path);
        bridge.insert_child(3, 2, None);

        let markup = decoded_markup(&content_url(&bridge, 2));
        assert!(
            markup.contains("fill=\"#ba7a7a\""),
            "omitted fill must bake inherited CSS color, got {markup}"
        );
        assert!(
            markup.contains("color=\"#ba7a7a\""),
            "root must expose currentColor, got {markup}"
        );

        let rgba = raster_svg_rgba(&markup)
            .expect("canvas imageDecode/resvg must rasterize the baked fill (serializer contract)");
        assert!(
            rgba.len() >= 4,
            "inherited-color svg must rasterize, got {} bytes",
            rgba.len()
        );
        assert_eq!(
            &rgba[..4],
            &[0xba, 0x7a, 0x7a, 0xff],
            "parent color #ba7a7a must ink the raster, got {:02x}{:02x}{:02x}{:02x}",
            rgba[0],
            rgba[1],
            rgba[2],
            rgba[3]
        );
    }

    #[test]
    fn current_color_fill_bakes_inherited_css_color() {
        let mut bridge = MessageBridge::new();
        let mut parent = WidgetProps::default();
        parent.element_tag = "div".into();
        parent.inline_style = "color:#ba7a7a".into();
        bridge.register(1, WidgetKind::Column, parent);

        let mut svg = WidgetProps::default();
        svg.element_tag = "svg".into();
        svg.attrs.insert("viewBox".into(), "0 0 8 8".into());
        svg.attrs.insert("width".into(), "8".into());
        svg.attrs.insert("height".into(), "8".into());
        bridge.register(2, WidgetKind::Column, svg);
        bridge.insert_child(2, 1, None);

        let mut path = WidgetProps::default();
        path.element_tag = "path".into();
        path.attrs.insert("d".into(), "M0 0 H8 V8 H0 Z".into());
        path.attrs.insert("fill".into(), "currentColor".into());
        bridge.register(3, WidgetKind::Box, path);
        bridge.insert_child(3, 2, None);

        let markup = decoded_markup(&content_url(&bridge, 2));
        assert!(
            markup.contains("fill=\"#ba7a7a\""),
            "currentColor fill must bake used text color, got {markup}"
        );
        assert!(
            !markup.to_ascii_lowercase().contains("currentcolor"),
            "serialized svg must not leave currentColor for resvg, got {markup}"
        );
    }

    fn decoded_markup(url: &str) -> String {
        let encoded = url.rsplit_once(',').expect("data url").1;
        String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .expect("base64"),
        )
        .expect("utf8 svg")
    }

    fn raster_svg_rgba(markup: &str) -> Option<Vec<u8>> {
        let canvas = nana_ui_web_api::shared_canvas_runtime();
        let mut api = nana_js_engine::HostApiRegistry::new();
        nana_ui_web_api::register_web_api_host_ops_with_resources(
            &mut api,
            nana_ui_web_api::shared_web_api_state(),
            nana_ui_web_api::default_shared_clipboard(),
            canvas.clone(),
        );
        let desc = api
            .call(
                "imageDecode",
                &[HostValue::Bytes(markup.as_bytes().to_vec())],
            )
            .ok()?;
        let id = desc.as_object()?.get("id")?.as_u64()?;
        let bitmap = canvas
            .lock()
            .ok()?
            .bitmap(nana_ui_web_api::CanvasId(id))
            .ok()?;
        Some(bitmap.rgba)
    }

    fn content_url(bridge: &MessageBridge, id: WidgetId) -> String {
        match &bridge
            .get(id)
            .expect("widget")
            .props
            .layout
            .paint
            .content_image
        {
            Some(nana_ui_core::BackgroundImage::Url { url, .. }) => url.clone(),
            other => panic!("expected content_image url, got {other:?}"),
        }
    }
}
