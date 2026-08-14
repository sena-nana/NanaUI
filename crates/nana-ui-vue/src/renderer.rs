//! Vue Custom Renderer host ops → Rust tree (`NanaTreeDocument`) + message bridge.
//!
//! ## L2 边界
//! - hostOps 写入树文档 + [`MessageBridge`]；Semantics 解析委托
//!   [`crate::widget_map::resolve_kind_from_hints`]。
//! - 不在此实现 CSS parse / paint；绘制经 `iced_app` → `nana_ui`。

use std::sync::{Arc, Mutex};

use nana_js_engine::{HostApiRegistry, HostValue, JsException};
use nana_ui_web_api::{SharedWebApiState, shared_web_api_state};

use crate::bridge::{MessageBridge, WidgetKind, WidgetProps, resolve_kind_from_hints, widget_id};
#[cfg(feature = "iced-view")]
use crate::native_component::{NativeComponentRegistry, normalize_component_name};
use crate::scroll::{
    ScrollIntoViewOptions, ScrollOffset, scroll_into_view, set_scroll_offset,
    shared_scroll_offset_store,
};
#[cfg(test)]
use crate::tree::get_layout_box;
use crate::tree::{
    ElementNamespace, LayoutBoxStore, NanaTreeDocument, NodeHandle, get_layout_box_from,
    shared_layout_box_store,
};

/// Shared handles used by DOM + semantic bridge host ops.
#[derive(Clone)]
pub struct HostDocs {
    pub document: Arc<Mutex<NanaTreeDocument>>,
    pub bridge: Arc<Mutex<MessageBridge>>,
    pub web_api: SharedWebApiState,
    pub layout_boxes: Arc<LayoutBoxStore>,
    #[cfg(feature = "iced-view")]
    pub components: Option<NativeComponentRegistry>,
}

/// Registers createElement / createWidget / insert / patchProp / … against `api`.
pub fn register_dom_host_ops(api: &mut HostApiRegistry, doc: Arc<Mutex<NanaTreeDocument>>) {
    let bridge = Arc::new(Mutex::new(MessageBridge::new()));
    register_dom_host_ops_with_bridge(api, doc, bridge, shared_web_api_state());
}

/// Registers host ops with an explicit shared [`MessageBridge`] (preferred for VueHost).
pub fn register_dom_host_ops_with_bridge(
    api: &mut HostApiRegistry,
    doc: Arc<Mutex<NanaTreeDocument>>,
    bridge: Arc<Mutex<MessageBridge>>,
    web_api: SharedWebApiState,
) {
    register_dom_host_ops_with_bridge_and_layout(
        api,
        doc,
        bridge,
        web_api,
        shared_layout_box_store(),
    );
}

pub(crate) fn register_dom_host_ops_with_bridge_and_layout(
    api: &mut HostApiRegistry,
    doc: Arc<Mutex<NanaTreeDocument>>,
    bridge: Arc<Mutex<MessageBridge>>,
    web_api: SharedWebApiState,
    layout_boxes: Arc<LayoutBoxStore>,
) {
    let host = HostDocs {
        document: doc,
        bridge,
        web_api,
        layout_boxes,
        #[cfg(feature = "iced-view")]
        components: None,
    };
    register_all(api, host);
}

#[cfg(feature = "iced-view")]
pub fn register_dom_host_ops_with_components(
    api: &mut HostApiRegistry,
    doc: Arc<Mutex<NanaTreeDocument>>,
    bridge: Arc<Mutex<MessageBridge>>,
    web_api: SharedWebApiState,
    components: NativeComponentRegistry,
) {
    register_dom_host_ops_with_components_and_layout(
        api,
        doc,
        bridge,
        web_api,
        components,
        shared_layout_box_store(),
    );
}

#[cfg(feature = "iced-view")]
pub(crate) fn register_dom_host_ops_with_components_and_layout(
    api: &mut HostApiRegistry,
    doc: Arc<Mutex<NanaTreeDocument>>,
    bridge: Arc<Mutex<MessageBridge>>,
    web_api: SharedWebApiState,
    components: NativeComponentRegistry,
    layout_boxes: Arc<LayoutBoxStore>,
) {
    register_all(
        api,
        HostDocs {
            document: doc,
            bridge,
            web_api,
            layout_boxes,
            components: Some(components),
        },
    );
}

fn register_all(api: &mut HostApiRegistry, host: HostDocs) {
    {
        let host = host.clone();
        api.register("createElement", move |args| {
            let tag = arg_str(args, 0).unwrap_or_else(|| "div".into());
            let namespace = ElementNamespace::parse(arg_str(args, 1).as_deref());
            let is = arg_str(args, 2);
            let mut guard = lock_doc(&host.document)?;
            let handle = guard.create_element_ns(&tag, namespace, is.as_deref());
            // runtime-dom: select[multiple] seeded from vnode props.
            if tag.eq_ignore_ascii_case("select")
                && let Some(HostValue::Object(props)) = args.get(3)
                && let Some(multiple) = props.get("multiple")
                && !matches!(multiple, HostValue::Null | HostValue::Undefined)
            {
                guard.set_attribute(handle, "multiple", &host_to_string(multiple));
            }
            drop(guard);
            // Every visible element downlevels onto a Nana foundation kind.
            let kind =
                resolve_kind_from_hints(&tag, None, None, None).unwrap_or(WidgetKind::Column);
            let mut bridge = lock_bridge(&host.bridge)?;
            let mut props = WidgetProps {
                element_tag: tag.to_ascii_lowercase(),
                ..WidgetProps::default()
            };
            if let Some(is) = is.filter(|s| !s.is_empty()) {
                props.attrs.insert("is".into(), is);
            }
            if let Some(ns) = namespace.as_str() {
                props.attrs.insert("data-nana-ns".into(), ns.to_string());
            }
            bridge.register(widget_id(handle), kind, props);
            Ok(HostValue::Number(handle.0 as f64))
        });
    }
    {
        let host = host.clone();
        api.register("createWidget", move |args| {
            let kind_raw = arg_str(args, 0).unwrap_or_else(|| "button".into());
            let native_name = normalize_native_component(&host, &kind_raw);
            let kind = WidgetKind::parse(&kind_raw)
                .or_else(|| native_name.as_ref().map(|_| WidgetKind::Box))
                .ok_or_else(|| JsException::new(format!("unknown widget kind: {kind_raw}")))?;
            let prop_map = match args.get(1) {
                Some(HostValue::Object(map)) => Some(map),
                _ => None,
            };
            let mut props = match prop_map {
                Some(map) => WidgetProps::from_map(map),
                _ => WidgetProps::default(),
            };
            #[cfg(feature = "iced-view")]
            if let Some(name) = native_name {
                let empty = std::collections::BTreeMap::new();
                props.attach_native_component(name.clone(), prop_map.unwrap_or(&empty));
                if let Some(registry) = &host.components {
                    registry.validate_props(&name, &props.native_props)?;
                }
            }
            let mut guard = lock_doc(&host.document)?;
            let element_tag = props
                .native_component
                .as_ref()
                .map(|name| format!("nana-{name}"))
                .unwrap_or_else(|| kind.element_tag().to_owned());
            props.element_tag.clone_from(&element_tag);
            let handle = guard.create_element(&element_tag);
            if !props.label.is_empty() {
                guard.set_attribute(handle, "label", &props.label);
                if matches!(
                    kind,
                    WidgetKind::Text
                        | WidgetKind::Button
                        | WidgetKind::Chip
                        | WidgetKind::SidebarRow
                        | WidgetKind::ListItem
                ) {
                    guard.set_element_text(handle, &props.label);
                }
            }
            if props.disabled {
                guard.set_attribute(handle, "disabled", "");
            }
            // Seed class / data-slot onto the document so slot contracts and
            // querySelector stay aligned with MessageBridge props (Vue may
            // skip a later patchProp when the vnode props were already
            // consumed by createWidget).
            if !props.class_names.is_empty() {
                guard.set_attribute(handle, "class", &props.class_names.join(" "));
            }
            if let Some(slot) = props.attrs.get("data-slot") {
                guard.set_attribute(handle, "data-slot", slot);
            }
            drop(guard);
            let mut bridge = lock_bridge(&host.bridge)?;
            bridge.register(widget_id(handle), kind, props);
            Ok(HostValue::Number(handle.0 as f64))
        });
    }
    {
        let host = host.clone();
        api.register("createText", move |args| {
            let text = arg_str(args, 0).unwrap_or_default();
            // Skip pure whitespace text nodes — they flatten onto body and
            // pollute the iced column without contributing labels.
            if text.trim().is_empty() {
                let mut guard = lock_doc(&host.document)?;
                let handle = guard.create_comment("ws");
                return Ok(HostValue::Number(handle.0 as f64));
            }
            let mut guard = lock_doc(&host.document)?;
            let handle = guard.create_text(&text);
            drop(guard);
            let mut bridge = lock_bridge(&host.bridge)?;
            bridge.register(
                widget_id(handle),
                WidgetKind::Text,
                WidgetProps {
                    label: text,
                    ..WidgetProps::default()
                },
            );
            Ok(HostValue::Number(handle.0 as f64))
        });
    }
    {
        let host = host.clone();
        api.register("createComment", move |args| {
            let text = arg_str(args, 0).unwrap_or_default();
            let mut guard = lock_doc(&host.document)?;
            Ok(HostValue::Number(guard.create_comment(&text).0 as f64))
        });
    }
    {
        let host = host.clone();
        api.register("insert", move |args| {
            let child = arg_handle(args, 0)?;
            let parent = arg_handle(args, 1)?;
            let anchor = arg_handle_opt(args, 2);
            let mut guard = lock_doc(&host.document)?;
            guard.insert(child, parent, anchor);
            let parent_tag = guard.element_tag(parent).unwrap_or_else(|| "div".into());
            drop(guard);
            let mut bridge = lock_bridge(&host.bridge)?;
            let parent_id = widget_id(parent);
            let child_id = widget_id(child);
            // Keep bridge parenting aligned with the document: if the parent
            // exists in the DOM but not yet in the semantic forest, seed it so
            // children are not flattened onto body.
            if !bridge.contains(parent_id) {
                let kind = resolve_kind_from_hints(&parent_tag, None, None, None)
                    .unwrap_or(WidgetKind::Column);
                bridge.register(parent_id, kind, WidgetProps::default());
            }
            if bridge.contains(child_id) {
                bridge.insert_child(child_id, parent_id, anchor.map(|a| a.0));
            }
            Ok(HostValue::Null)
        });
    }
    {
        let host = host.clone();
        api.register("remove", move |args| {
            let child = arg_handle(args, 0)?;
            unmount_native_subtree(&host, widget_id(child))?;
            let mut guard = lock_doc(&host.document)?;
            guard.remove(child);
            drop(guard);
            let mut bridge = lock_bridge(&host.bridge)?;
            if bridge.contains(widget_id(child)) {
                bridge.unregister(widget_id(child));
            }
            Ok(HostValue::Null)
        });
    }
    {
        let host = host.clone();
        api.register("setText", move |args| {
            let node = arg_handle(args, 0)?;
            let text = arg_str(args, 1).unwrap_or_default();
            let mut guard = lock_doc(&host.document)?;
            guard.set_text(node, &text);
            drop(guard);
            let mut bridge = lock_bridge(&host.bridge)?;
            if bridge.contains(widget_id(node)) {
                bridge.set_label(widget_id(node), text);
            }
            Ok(HostValue::Null)
        });
    }
    {
        let host = host.clone();
        api.register("setElementText", move |args| {
            let el = arg_handle(args, 0)?;
            let text = arg_str(args, 1).unwrap_or_default();
            let mut guard = lock_doc(&host.document)?;
            let stale_children = guard.children_of(el);
            guard.set_element_text(el, &text);
            drop(guard);
            let mut bridge = lock_bridge(&host.bridge)?;
            for child in stale_children {
                let cid = widget_id(child);
                drop(bridge);
                unmount_native_subtree(&host, cid)?;
                bridge = lock_bridge(&host.bridge)?;
                if bridge.contains(cid) {
                    bridge.unregister(cid);
                }
            }
            if bridge.contains(widget_id(el)) {
                bridge.set_label(widget_id(el), text);
            }
            Ok(HostValue::Null)
        });
    }
    {
        let host = host.clone();
        api.register("patchProp", move |args| {
            let el = arg_handle(args, 0)?;
            let key = arg_str(args, 1).unwrap_or_default();
            let value = args.get(2).cloned().unwrap_or(HostValue::Null);
            #[cfg(feature = "iced-view")]
            if let Some(registry) = &host.components {
                let bridge = lock_bridge(&host.bridge)?;
                if let Some(widget) = bridge.get(widget_id(el))
                    && let Some(name) = &widget.props.native_component
                {
                    let mut proposed = widget.props.clone();
                    proposed.apply_prop(&key, &value);
                    registry.validate_props(name, &proposed.native_props)?;
                }
            }
            let mut stale_children = Vec::new();
            {
                let mut guard = lock_doc(&host.document)?;
                if key == "innerHTML" || key == "textContent" {
                    stale_children = guard.children_of(el);
                }
                patch_prop(&mut guard, el, &key, value.clone());
            }
            let mut bridge = lock_bridge(&host.bridge)?;
            for child in stale_children {
                let cid = widget_id(child);
                drop(bridge);
                unmount_native_subtree(&host, cid)?;
                bridge = lock_bridge(&host.bridge)?;
                if bridge.contains(cid) {
                    bridge.unregister(cid);
                }
            }
            if bridge.contains(widget_id(el)) {
                bridge.patch_prop(widget_id(el), &key, &value);
                if key == "innerHTML" || key == "textContent" {
                    let label = match &value {
                        HostValue::Null | HostValue::Undefined => String::new(),
                        other => host_to_string(other),
                    };
                    bridge.set_label(widget_id(el), label);
                }
            }
            Ok(HostValue::Null)
        });
    }
    {
        let host = host.clone();
        api.register("getAttribute", move |args| {
            let el = arg_handle(args, 0)?;
            let key = arg_str(args, 1).unwrap_or_default();
            let guard = lock_doc(&host.document)?;
            Ok(match guard.get_attribute(el, &key) {
                Some(v) => HostValue::string(v),
                None => HostValue::Null,
            })
        });
    }
    {
        let host = host.clone();
        api.register("parentNode", move |args| {
            let node = arg_handle(args, 0)?;
            let guard = lock_doc(&host.document)?;
            Ok(match guard.parent_node(node) {
                Some(h) => HostValue::Number(h.0 as f64),
                None => HostValue::Null,
            })
        });
    }
    {
        let host = host.clone();
        api.register("nextSibling", move |args| {
            let node = arg_handle(args, 0)?;
            let guard = lock_doc(&host.document)?;
            Ok(match guard.next_sibling(node) {
                Some(h) => HostValue::Number(h.0 as f64),
                None => HostValue::Null,
            })
        });
    }
    {
        let host = host.clone();
        api.register("querySelector", move |args| {
            let sel = arg_str(args, 0).unwrap_or_default();
            let guard = lock_doc(&host.document)?;
            Ok(match guard.query_selector(&sel) {
                Some(h) => HostValue::Number(h.0 as f64),
                None => HostValue::Null,
            })
        });
    }
    {
        let host = host.clone();
        api.register("querySelectorAll", move |args| {
            let sel = arg_str(args, 0).unwrap_or_default();
            let guard = lock_doc(&host.document)?;
            Ok(HostValue::Array(
                guard
                    .query_selector_all(&sel)
                    .into_iter()
                    .map(|h| HostValue::Number(h.0 as f64))
                    .collect(),
            ))
        });
    }
    {
        let host = host.clone();
        api.register("closest", move |args| {
            let el = arg_handle(args, 0)?;
            let sel = arg_str(args, 1).unwrap_or_default();
            let guard = lock_doc(&host.document)?;
            Ok(match guard.closest(el, &sel) {
                Some(h) => HostValue::Number(h.0 as f64),
                None => HostValue::Null,
            })
        });
    }
    {
        let host = host.clone();
        api.register("contains", move |args| {
            let el = arg_handle(args, 0)?;
            let other = arg_handle(args, 1)?;
            let guard = lock_doc(&host.document)?;
            Ok(HostValue::Bool(guard.contains(el, other)))
        });
    }
    {
        let host = host.clone();
        api.register("firstChild", move |args| {
            let el = arg_handle(args, 0)?;
            let guard = lock_doc(&host.document)?;
            Ok(match guard.first_child(el) {
                Some(h) => HostValue::Number(h.0 as f64),
                None => HostValue::Null,
            })
        });
    }
    {
        let host = host.clone();
        api.register("childNodes", move |args| {
            let el = arg_handle(args, 0)?;
            let guard = lock_doc(&host.document)?;
            Ok(HostValue::Array(
                guard
                    .children_of(el)
                    .into_iter()
                    .map(|h| HostValue::Number(h.0 as f64))
                    .collect(),
            ))
        });
    }
    {
        let host = host.clone();
        api.register("nodeKind", move |args| {
            let el = arg_handle(args, 0)?;
            let guard = lock_doc(&host.document)?;
            Ok(HostValue::string(match guard.node_kind(el) {
                crate::tree::DomNodeKind::Element => "element",
                crate::tree::DomNodeKind::Text => "text",
                crate::tree::DomNodeKind::Comment => "comment",
                crate::tree::DomNodeKind::Document => "document",
                crate::tree::DomNodeKind::Other => "other",
            }))
        });
    }
    {
        let host = host.clone();
        api.register("elementTag", move |args| {
            let el = arg_handle(args, 0)?;
            let guard = lock_doc(&host.document)?;
            Ok(match guard.element_tag(el) {
                Some(tag) => HostValue::string(tag),
                None => HostValue::Null,
            })
        });
    }
    {
        let host = host.clone();
        api.register("mountRoot", move |_args| {
            let guard = lock_doc(&host.document)?;
            let html = guard.html_root();
            let body = guard.mount_root();
            drop(guard);
            let mut bridge = lock_bridge(&host.bridge)?;
            bridge.ensure_document_roots(html.0, body.0);
            Ok(HostValue::Number(body.0 as f64))
        });
    }
    {
        let host = host.clone();
        api.register("clearMount", move |_args| {
            let guard = lock_doc(&host.document)?;
            let body = guard.mount_root();
            let children = guard.children_of(body);
            drop(guard);
            for child in children {
                unmount_native_subtree(&host, widget_id(child))?;
                let mut doc = lock_doc(&host.document)?;
                doc.remove(child);
                drop(doc);
                let mut bridge = lock_bridge(&host.bridge)?;
                if bridge.contains(widget_id(child)) {
                    bridge.unregister(widget_id(child));
                }
            }
            let mut bridge = lock_bridge(&host.bridge)?;
            bridge.clear_mounted();
            Ok(HostValue::Null)
        });
    }
    {
        let host = host.clone();
        api.register("resolveLayout", move |_args| {
            let mut bridge = lock_bridge(&host.bridge)?;
            let mut doc = lock_doc(&host.document)?;
            bridge.resolve_document_layout(&mut doc);
            Ok(HostValue::Null)
        });
    }
    {
        let host = host.clone();
        api.register("layoutSnapshot", move |_args| {
            let guard = lock_doc(&host.document)?;
            let snap = guard.snapshot_boxes();
            let boxes: Vec<HostValue> = snap
                .boxes
                .iter()
                .map(|b| {
                    HostValue::Object(
                        [
                            ("id".into(), HostValue::Number(b.handle.0 as f64)),
                            ("x".into(), HostValue::Number(b.x as f64)),
                            ("y".into(), HostValue::Number(b.y as f64)),
                            ("w".into(), HostValue::Number(b.width as f64)),
                            ("h".into(), HostValue::Number(b.height as f64)),
                        ]
                        .into_iter()
                        .collect(),
                    )
                })
                .collect();
            let texts: Vec<HostValue> = snap
                .texts
                .iter()
                .map(|(h, t)| {
                    HostValue::Object(
                        [
                            ("id".into(), HostValue::Number(h.0 as f64)),
                            ("text".into(), HostValue::String(t.clone())),
                        ]
                        .into_iter()
                        .collect(),
                    )
                })
                .collect();
            let gpu: Vec<HostValue> = snap
                .gpu_slots
                .iter()
                .map(|(h, s)| {
                    HostValue::Object(
                        [
                            ("id".into(), HostValue::Number(h.0 as f64)),
                            ("slot".into(), HostValue::String(s.clone())),
                        ]
                        .into_iter()
                        .collect(),
                    )
                })
                .collect();
            Ok(HostValue::Object(
                [
                    ("boxes".into(), HostValue::Array(boxes)),
                    ("texts".into(), HostValue::Array(texts)),
                    ("gpuSlots".into(), HostValue::Array(gpu)),
                    (
                        "stylesheets".into(),
                        HostValue::Number(guard.stylesheet_count() as f64),
                    ),
                ]
                .into_iter()
                .collect(),
            ))
        });
    }
    {
        let host = host.clone();
        api.register("semanticSnapshot", move |_args| {
            let bridge = lock_bridge(&host.bridge)?;
            let snap = bridge.snapshot();
            let widgets: Vec<HostValue> = snap
                .widgets
                .iter()
                .map(|w| {
                    HostValue::Object(
                        [
                            ("id".into(), HostValue::Number(w.id as f64)),
                            ("kind".into(), HostValue::string(w.kind.as_str())),
                            ("label".into(), HostValue::string(w.props.label.clone())),
                            ("disabled".into(), HostValue::Bool(w.props.disabled)),
                            ("toggled".into(), HostValue::Bool(w.props.toggled)),
                            ("active".into(), HostValue::Bool(w.props.active)),
                            (
                                "children".into(),
                                HostValue::Array(
                                    w.children
                                        .iter()
                                        .map(|c| HostValue::Number(*c as f64))
                                        .collect(),
                                ),
                            ),
                        ]
                        .into_iter()
                        .collect(),
                    )
                })
                .collect();
            Ok(HostValue::Object(
                [
                    ("revision".into(), HostValue::Number(snap.revision as f64)),
                    (
                        "theme".into(),
                        HostValue::string(match snap.theme {
                            nana_ui_core::ThemeMode::Light => "light",
                            nana_ui_core::ThemeMode::Dark => "dark",
                        }),
                    ),
                    (
                        "roots".into(),
                        HostValue::Array(
                            snap.roots
                                .iter()
                                .map(|r| HostValue::Number(*r as f64))
                                .collect(),
                        ),
                    ),
                    ("widgets".into(), HostValue::Array(widgets)),
                ]
                .into_iter()
                .collect(),
            ))
        });
    }
    {
        let host = host.clone();
        api.register("cloneNode", move |args| {
            let node = arg_handle(args, 0)?;
            let deep = args.get(1).and_then(HostValue::as_bool).unwrap_or(true);
            let mut guard = lock_doc(&host.document)?;
            let (copy, pairs) = guard.clone_node_mapped(node, deep);
            drop(guard);
            let mut bridge = lock_bridge(&host.bridge)?;
            sync_clone_pairs_to_bridge(&host.document, &mut bridge, &pairs)?;
            Ok(HostValue::Number(copy.0 as f64))
        });
    }
    {
        let host = host.clone();
        api.register("insertStaticContent", move |args| {
            let content = arg_str(args, 0).unwrap_or_default();
            let parent = arg_handle(args, 1)?;
            let anchor = arg_handle_opt(args, 2);
            let namespace = ElementNamespace::parse(arg_str(args, 3).as_deref());
            let start = arg_handle_opt(args, 4);
            let end = arg_handle_opt(args, 5);
            let mut guard = lock_doc(&host.document)?;
            let (first, last, pairs) =
                guard.insert_static_content(&content, parent, anchor, namespace, start, end);
            drop(guard);
            let mut bridge = lock_bridge(&host.bridge)?;
            sync_clone_pairs_to_bridge(&host.document, &mut bridge, &pairs)?;
            let parent_id = widget_id(parent);
            if !bridge.contains(parent_id) {
                let parent_tag = {
                    let guard = lock_doc(&host.document)?;
                    guard.element_tag(parent).unwrap_or_else(|| "div".into())
                };
                let kind = resolve_kind_from_hints(&parent_tag, None, None, None)
                    .unwrap_or(WidgetKind::Column);
                bridge.register(parent_id, kind, WidgetProps::default());
            }
            let top_ids = {
                let guard = lock_doc(&host.document)?;
                collect_static_range_roots(&guard, parent, first, last)
            };
            for id in top_ids {
                if bridge.contains(id) {
                    bridge.insert_child(id, parent_id, anchor.map(|a| a.0));
                }
            }
            Ok(HostValue::Array(vec![
                HostValue::Number(first.0 as f64),
                HostValue::Number(last.0 as f64),
            ]))
        });
    }
    {
        let host = host.clone();
        api.register("setScopeId", move |args| {
            let el = arg_handle(args, 0)?;
            let scope = arg_str(args, 1).unwrap_or_default();
            let mut guard = lock_doc(&host.document)?;
            guard.set_scope_id(el, &scope);
            drop(guard);
            let mut bridge = lock_bridge(&host.bridge)?;
            if bridge.contains(widget_id(el)) && !scope.is_empty() {
                // Vue scoped CSS: `.foo[data-v-xxxx]` — attribute presence match.
                bridge.set_scope_attr(widget_id(el), &scope);
            }
            Ok(HostValue::Null)
        });
    }
    {
        let host = host.clone();
        api.register("injectStylesheet", move |args| {
            let css = arg_str(args, 0).unwrap_or_default();
            lock_doc(&host.document)?.inject_stylesheet(&css);
            let mut bridge = lock_bridge(&host.bridge)?;
            bridge.inject_stylesheet(&css);
            let mut doc = lock_doc(&host.document)?;
            bridge.resolve_document_layout(&mut doc);
            Ok(HostValue::Null)
        });
    }
    {
        let host = host.clone();
        api.register("layoutBox", move |args| {
            let el = arg_handle(args, 0)?;
            let guard = lock_doc(&host.document)?;
            // Prefer iced paint writeback so getBoundingClientRect matches drawing.
            Ok(match get_layout_box_from(&host.layout_boxes, &guard, el) {
                Some(b) => HostValue::Object(
                    [
                        ("x".into(), HostValue::Number(b.x as f64)),
                        ("y".into(), HostValue::Number(b.y as f64)),
                        ("width".into(), HostValue::Number(b.width as f64)),
                        ("height".into(), HostValue::Number(b.height as f64)),
                        ("top".into(), HostValue::Number(b.y as f64)),
                        ("left".into(), HostValue::Number(b.x as f64)),
                        ("bottom".into(), HostValue::Number((b.y + b.height) as f64)),
                        ("right".into(), HostValue::Number((b.x + b.width) as f64)),
                    ]
                    .into_iter()
                    .collect(),
                ),
                None => HostValue::Object(
                    [
                        ("x".into(), HostValue::Number(0.0)),
                        ("y".into(), HostValue::Number(0.0)),
                        ("width".into(), HostValue::Number(0.0)),
                        ("height".into(), HostValue::Number(0.0)),
                        ("top".into(), HostValue::Number(0.0)),
                        ("left".into(), HostValue::Number(0.0)),
                        ("bottom".into(), HostValue::Number(0.0)),
                        ("right".into(), HostValue::Number(0.0)),
                    ]
                    .into_iter()
                    .collect(),
                ),
            })
        });
    }
    {
        let host = host.clone();
        api.register("getScrollOffset", move |args| {
            let el = arg_handle(args, 0)?;
            let guard = lock_doc(&host.document)?;
            let off = guard.scroll_offset(el);
            Ok(HostValue::Object(
                [
                    ("x".into(), HostValue::Number(off.x as f64)),
                    ("y".into(), HostValue::Number(off.y as f64)),
                    ("scrollLeft".into(), HostValue::Number(off.x as f64)),
                    ("scrollTop".into(), HostValue::Number(off.y as f64)),
                ]
                .into_iter()
                .collect(),
            ))
        });
    }
    {
        let host = host.clone();
        api.register("setScrollOffset", move |args| {
            let el = arg_handle(args, 0)?;
            let x = args.get(1).and_then(HostValue::as_f64).unwrap_or(0.0) as f32;
            let y = args.get(2).and_then(HostValue::as_f64).unwrap_or(0.0) as f32;
            let mut guard = lock_doc(&host.document)?;
            let next = set_scroll_offset(
                &mut guard,
                &host.layout_boxes,
                &shared_scroll_offset_store(),
                widget_id(el),
                ScrollOffset { x, y },
            );
            Ok(HostValue::Object(
                [
                    ("x".into(), HostValue::Number(next.x as f64)),
                    ("y".into(), HostValue::Number(next.y as f64)),
                    ("scrollLeft".into(), HostValue::Number(next.x as f64)),
                    ("scrollTop".into(), HostValue::Number(next.y as f64)),
                ]
                .into_iter()
                .collect(),
            ))
        });
    }
    {
        let host = host.clone();
        api.register("scrollIntoView", move |args| {
            let el = arg_handle(args, 0)?;
            let opts = ScrollIntoViewOptions::from_host_value(args.get(1));
            let mut guard = lock_doc(&host.document)?;
            let bridge = lock_bridge(&host.bridge)?;
            let result = scroll_into_view(
                &mut guard,
                &bridge,
                &host.layout_boxes,
                &shared_scroll_offset_store(),
                el,
                opts,
            );
            Ok(HostValue::Object(
                [(
                    "scrolled".into(),
                    HostValue::Array(
                        result
                            .scrolled
                            .into_iter()
                            .map(|(id, off)| {
                                HostValue::Object(
                                    [
                                        ("id".into(), HostValue::Number(id as f64)),
                                        ("scrollLeft".into(), HostValue::Number(off.x as f64)),
                                        ("scrollTop".into(), HostValue::Number(off.y as f64)),
                                    ]
                                    .into_iter()
                                    .collect(),
                                )
                            })
                            .collect(),
                    ),
                )]
                .into_iter()
                .collect(),
            ))
        });
    }
    {
        let host = host.clone();
        api.register("injectStyleElement", move |args| {
            let css = arg_str(args, 0).unwrap_or_default();
            let mut guard = lock_doc(&host.document)?;
            let handle = guard.inject_style_element(&css);
            drop(guard);
            let mut bridge = lock_bridge(&host.bridge)?;
            bridge.inject_stylesheet(&css);
            let mut doc = lock_doc(&host.document)?;
            bridge.resolve_document_layout(&mut doc);
            Ok(HostValue::Number(handle.0 as f64))
        });
    }
    {
        let host = host.clone();
        api.register("setGpuSlot", move |args| {
            let el = arg_handle(args, 0)?;
            let slot = arg_str(args, 1).unwrap_or_else(|| "default".into());
            let mut guard = lock_doc(&host.document)?;
            guard.set_gpu_slot(el, &slot);
            drop(guard);
            // The Iced adapter renders MessageBridge snapshots, not the raw
            // document side table. Mirror the slot into semantic props so the
            // real HostTexture can be resolved during composition.
            lock_bridge(&host.bridge)?.patch_prop(el.0, "data-nana-gpu", &HostValue::String(slot));
            Ok(HostValue::Null)
        });
    }
    {
        let host = host.clone();
        api.register("setDocumentTheme", move |args| {
            let theme = arg_str(args, 0).unwrap_or_else(|| "light".into());
            // Keep shared web-api `dataset.theme` in lockstep with document/bridge.
            // Otherwise `sync_appearance_shared` (snapshot / appearance) re-applies a
            // stale web-api theme and reverts `var(--*)` after this host op.
            if let Ok(mut web) = host.web_api.lock() {
                web.set_document_dataset("theme", &theme);
            }
            lock_doc(&host.document)?.set_document_theme(&theme);
            let mode = if theme.eq_ignore_ascii_case("dark") {
                nana_ui_core::ThemeMode::Dark
            } else {
                nana_ui_core::ThemeMode::Light
            };
            let mut bridge = lock_bridge(&host.bridge)?;
            bridge.set_theme(mode);
            let mut doc = lock_doc(&host.document)?;
            bridge.resolve_document_layout(&mut doc);
            Ok(HostValue::Null)
        });
    }
    {
        let host = host.clone();
        api.register("getDocumentTheme", move |_args| {
            let bridge = lock_bridge(&host.bridge)?;
            Ok(HostValue::string(bridge.theme_label()))
        });
    }
    {
        let host = host.clone();
        api.register("setFocus", move |args| {
            let el = arg_handle(args, 0)?;
            let mut guard = lock_doc(&host.document)?;
            guard.set_focus(el);
            Ok(HostValue::Null)
        });
    }
    {
        let host = host.clone();
        api.register("clearFocus", move |_args| {
            let mut guard = lock_doc(&host.document)?;
            guard.clear_focus();
            Ok(HostValue::Null)
        });
    }
}

#[cfg(feature = "iced-view")]
fn normalize_native_component(host: &HostDocs, raw: &str) -> Option<String> {
    let name = normalize_component_name(raw);
    host.components
        .as_ref()
        .filter(|registry| registry.contains(&name))
        .map(|_| name)
}

#[cfg(not(feature = "iced-view"))]
fn normalize_native_component(_host: &HostDocs, _raw: &str) -> Option<String> {
    None
}

#[cfg(feature = "iced-view")]
fn unmount_native_subtree(host: &HostDocs, root: u64) -> Result<(), JsException> {
    fn collect(bridge: &MessageBridge, id: u64, mounted: &mut Vec<(String, u64)>) {
        let Some(widget) = bridge.get(id) else {
            return;
        };
        if let Some(name) = &widget.props.native_component {
            mounted.push((name.clone(), id));
        }
        for child in &widget.children {
            collect(bridge, *child, mounted);
        }
    }

    let Some(registry) = &host.components else {
        return Ok(());
    };
    let mut mounted = Vec::new();
    {
        let bridge = lock_bridge(&host.bridge)?;
        collect(&bridge, root, &mut mounted);
    }
    for (component, id) in mounted {
        registry.unmount(&component, id);
    }
    Ok(())
}

#[cfg(not(feature = "iced-view"))]
fn unmount_native_subtree(_host: &HostDocs, _root: u64) -> Result<(), JsException> {
    Ok(())
}

fn patch_prop(doc: &mut NanaTreeDocument, el: NodeHandle, key: &str, value: HostValue) {
    // Vue compiler `.prop` / `^attr` modifiers — strip before applying.
    let key = strip_prop_modifier(key);
    if key.starts_with("on") || key.starts_with("On") {
        let enabled = !matches!(value, HostValue::Null | HostValue::Undefined);
        doc.set_event_flag(el, key, enabled);
        return;
    }

    match key {
        "class" | "className" => match value {
            HostValue::Null | HostValue::Undefined => doc.remove_attribute(el, "class"),
            other => doc.set_attribute(el, "class", &host_to_string(&other)),
        },
        "style" => match value {
            HostValue::Null | HostValue::Undefined => doc.remove_attribute(el, "style"),
            HostValue::Object(map) => {
                let css = serialize_style_object(&map);
                doc.set_attribute(el, "style", &css);
            }
            other => doc.set_attribute(el, "style", &host_to_string(&other)),
        },
        // Vue runtime-dom patchDOMProp: v-html / v-text land here, not as attrs.
        "innerHTML" | "textContent" => match value {
            HostValue::Null | HostValue::Undefined => {
                doc.set_element_text(el, "");
                doc.remove_attribute(el, key);
            }
            other => {
                let text = host_to_string(&other);
                doc.set_element_text(el, &text);
                doc.set_attribute(el, key, &text);
            }
        },
        _ => match value {
            HostValue::Null | HostValue::Undefined => doc.remove_attribute(el, key),
            other => {
                if other.as_bool() == Some(false) {
                    doc.remove_attribute(el, key);
                } else if is_boolean_attr(key) && is_falsey_attr_value(&other) {
                    doc.remove_attribute(el, key);
                } else if is_boolean_attr(key) && include_boolean_attr(&other) {
                    doc.set_attribute(el, key, "");
                } else {
                    // Preserve SVG attr casing (viewBox, xlink:href, …).
                    doc.set_attribute(el, key, &host_to_string(&other));
                }
            }
        },
    }
}

/// Strip Vue `.prop` / `^attr` key modifiers (runtime-dom patchProp).
fn strip_prop_modifier(key: &str) -> &str {
    let key = key.trim();
    if let Some(rest) = key.strip_prefix('.') {
        rest
    } else if let Some(rest) = key.strip_prefix('^') {
        rest
    } else {
        key
    }
}

fn is_boolean_attr(key: &str) -> bool {
    // Align with @vue/shared `isBooleanAttr` (+ Nana extras).
    matches!(
        key,
        "itemscope"
            | "allowfullscreen"
            | "formnovalidate"
            | "ismap"
            | "nomodule"
            | "novalidate"
            | "readonly"
            | "async"
            | "autofocus"
            | "autoplay"
            | "controls"
            | "default"
            | "defer"
            | "disabled"
            | "hidden"
            | "inert"
            | "loop"
            | "open"
            | "required"
            | "reversed"
            | "scoped"
            | "seamless"
            | "checked"
            | "muted"
            | "multiple"
            | "selected"
            | "noresize"
            | "noshade"
            | "nowrap"
    )
}

/// Vue `includeBooleanAttr`: presence is true for `true` or `""`.
fn include_boolean_attr(value: &HostValue) -> bool {
    match value {
        HostValue::Bool(true) => true,
        HostValue::String(s) if s.is_empty() => true,
        HostValue::Bool(false) | HostValue::Null | HostValue::Undefined => false,
        other => !is_falsey_attr_value(other),
    }
}

fn is_falsey_attr_value(value: &HostValue) -> bool {
    match value {
        HostValue::Bool(false) => true,
        HostValue::String(s) => {
            let s = s.trim();
            // Nana: string "false"/"0" must not leave boolean attrs present
            // (HTML presence would still activate hidden/disabled).
            s.eq_ignore_ascii_case("false") || s == "0"
        }
        HostValue::Number(n) if *n == 0.0 => true,
        _ => false,
    }
}

fn lock_doc(
    doc: &Arc<Mutex<NanaTreeDocument>>,
) -> Result<std::sync::MutexGuard<'_, NanaTreeDocument>, JsException> {
    doc.lock()
        .map_err(|_| JsException::new("nana tree document poisoned"))
}

fn lock_bridge(
    bridge: &Arc<Mutex<MessageBridge>>,
) -> Result<std::sync::MutexGuard<'_, MessageBridge>, JsException> {
    bridge
        .lock()
        .map_err(|_| JsException::new("nana message bridge poisoned"))
}

fn arg_str(args: &[HostValue], index: usize) -> Option<String> {
    args.get(index).and_then(|v| match v {
        HostValue::String(s) => Some(s.clone()),
        HostValue::Number(n) => Some(n.to_string()),
        HostValue::Bool(b) => Some(b.to_string()),
        _ => None,
    })
}

fn arg_handle(args: &[HostValue], index: usize) -> Result<NodeHandle, JsException> {
    let n = args
        .get(index)
        .and_then(HostValue::as_f64)
        .ok_or_else(|| JsException::new(format!("expected node handle at arg {index}")))?;
    Ok(NodeHandle(n as u64))
}

fn arg_handle_opt(args: &[HostValue], index: usize) -> Option<NodeHandle> {
    let v = args.get(index)?;
    match v {
        HostValue::Null | HostValue::Undefined => None,
        HostValue::Number(n) => Some(NodeHandle(*n as u64)),
        _ => None,
    }
}

/// Sync tree clone / static materialization into MessageBridge.
///
/// - `(src, dst)` with `src != dst`: clone kind+props from src, then wire children.
/// - `(src, dst)` with `src == dst`: seed a fresh downlevel registration (static parse).
fn sync_clone_pairs_to_bridge(
    doc: &Arc<Mutex<NanaTreeDocument>>,
    bridge: &mut MessageBridge,
    pairs: &[(NodeHandle, NodeHandle)],
) -> Result<(), JsException> {
    let guard = lock_doc(doc)?;
    for &(src, dst) in pairs {
        if src.0 == dst.0 {
            seed_bridge_node(&guard, bridge, dst);
            continue;
        }
        if bridge.clone_register(widget_id(src), widget_id(dst)) {
            continue;
        }
        seed_bridge_node(&guard, bridge, dst);
    }
    // Parenting: for each mapped dst that has a tree parent also in the pair set,
    // insert_child under that parent (document order).
    let dst_set: std::collections::HashSet<u64> = pairs.iter().map(|(_, d)| d.0).collect();
    for &(_, dst) in pairs {
        if !bridge.contains(widget_id(dst)) {
            continue;
        }
        let Some(parent) = guard.parent_node(dst) else {
            continue;
        };
        if !dst_set.contains(&parent.0) {
            continue;
        }
        if bridge.contains(widget_id(parent)) {
            bridge.insert_child(widget_id(dst), widget_id(parent), None);
        }
    }
    Ok(())
}

fn seed_bridge_node(doc: &NanaTreeDocument, bridge: &mut MessageBridge, node: NodeHandle) {
    let id = widget_id(node);
    if bridge.contains(id) {
        return;
    }
    match doc.node_kind(node) {
        crate::tree::DomNodeKind::Element => {
            let tag = doc.element_tag(node).unwrap_or_else(|| "div".into());
            let kind =
                resolve_kind_from_hints(&tag, None, None, None).unwrap_or(WidgetKind::Column);
            let mut props = WidgetProps {
                element_tag: tag,
                ..WidgetProps::default()
            };
            if let Some(ns) = doc.element_namespace(node).and_then(|n| n.as_str()) {
                props.attrs.insert("data-nana-ns".into(), ns.to_string());
            }
            if let Some(is) = doc.get_attribute(node, "is") {
                props.attrs.insert("is".into(), is);
            }
            if let Some(class) = doc.get_attribute(node, "class") {
                props.class_names = class.split_whitespace().map(str::to_string).collect();
            }
            if let Some(label) = doc.text_content(node) {
                if !label.trim().is_empty() {
                    props.label = label;
                }
            }
            bridge.register(id, kind, props);
        }
        crate::tree::DomNodeKind::Text => {
            let label = doc.text_content(node).unwrap_or_default();
            if label.trim().is_empty() {
                return;
            }
            bridge.register(
                id,
                WidgetKind::Text,
                WidgetProps {
                    label,
                    ..WidgetProps::default()
                },
            );
        }
        _ => {}
    }
}

/// Inclusive sibling range under `parent` from `first` through `last`.
fn collect_static_range_roots(
    doc: &NanaTreeDocument,
    parent: NodeHandle,
    first: NodeHandle,
    last: NodeHandle,
) -> Vec<u64> {
    let children = doc.children_of(parent);
    let mut out = Vec::new();
    let mut seen = false;
    for child in children {
        if child.0 == first.0 {
            seen = true;
        }
        if seen {
            out.push(child.0);
        }
        if child.0 == last.0 {
            break;
        }
    }
    if out.is_empty() {
        out.push(first.0);
        if last.0 != first.0 {
            out.push(last.0);
        }
    }
    out
}

fn host_to_string(value: &HostValue) -> String {
    match value {
        HostValue::Null | HostValue::Undefined => String::new(),
        HostValue::Bool(v) => v.to_string(),
        HostValue::Number(v) => {
            if v.fract() == 0.0 && v.is_finite() {
                format!("{}", *v as i64)
            } else {
                v.to_string()
            }
        }
        HostValue::String(v) => {
            if v == "[object Object]" {
                String::new()
            } else {
                v.clone()
            }
        }
        HostValue::Object(map) => {
            for key in ["label", "name", "title", "text", "value", "id"] {
                if let Some(v) = map.get(key) {
                    let s = host_to_string(v);
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
            String::new()
        }
        HostValue::Array(items) => items
            .iter()
            .map(host_to_string)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        other => {
            let s = other.to_json_string();
            if s == "[object Object]" || s.starts_with('{') || s.starts_with('[') {
                String::new()
            } else {
                s
            }
        }
    }
}

/// Serialize a Vue style object into a CSS declaration string.
pub(crate) fn serialize_style_object(
    map: &std::collections::BTreeMap<String, HostValue>,
) -> String {
    let mut out = String::with_capacity(map.len().saturating_mul(24));
    for (key, value) in map {
        let prop = css_style_prop_name(key);
        let val = host_to_string(value);
        if prop.is_empty() || val.is_empty() {
            continue;
        }
        out.push_str(&prop);
        out.push(':');
        out.push_str(&val);
        out.push(';');
    }
    out
}

/// Convert a Vue CSSOM-style property name to a CSS declaration name.
pub(crate) fn css_style_prop_name(key: &str) -> String {
    let key = key.trim();
    if key.is_empty() {
        return String::new();
    }
    if key.starts_with("--") {
        return key.to_string();
    }
    if key.contains('-') {
        return key.to_string();
    }
    camel_to_kebab(key)
}

fn camel_to_kebab(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    for (i, ch) in input.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn css_style_prop_name_converts_camel_case() {
        assert_eq!(
            css_style_prop_name("gridTemplateColumns"),
            "grid-template-columns"
        );
        assert_eq!(css_style_prop_name("--accent"), "--accent");
        assert_eq!(css_style_prop_name("background-color"), "background-color");
    }

    #[test]
    fn camel_case_style_object_is_stored_as_kebab_attr() {
        let mut doc = NanaTreeDocument::new(960, 640, 1.0);
        let root = doc.mount_root();
        let workspace = doc.create_element("div");
        let mut style = BTreeMap::new();
        style.insert(
            "gridTemplateColumns".into(),
            HostValue::string("220px minmax(0, 1fr)"),
        );
        patch_prop(&mut doc, workspace, "style", HostValue::Object(style));
        doc.insert(workspace, root, None);
        let css = doc.get_attribute(workspace, "style").expect("style");
        assert!(
            css.contains("grid-template-columns:"),
            "expected kebab-case style attr, got {css}"
        );
    }

    #[test]
    fn hidden_false_string_does_not_set_hidden_attr() {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let root = doc.mount_root();
        let el = doc.create_element("aside");
        doc.set_element_text(el, "x");
        doc.insert(el, root, None);
        patch_prop(&mut doc, el, "hidden", HostValue::string("false"));
        assert!(doc.get_attribute(el, "hidden").is_none());
    }

    #[test]
    fn create_widget_registers_semantic_button() {
        let doc = Arc::new(Mutex::new(NanaTreeDocument::new(400, 300, 1.0)));
        let bridge = Arc::new(Mutex::new(MessageBridge::new()));
        let mut api = HostApiRegistry::new();
        register_dom_host_ops_with_bridge(
            &mut api,
            Arc::clone(&doc),
            Arc::clone(&bridge),
            shared_web_api_state(),
        );
        let id = api
            .call(
                "createWidget",
                &[
                    HostValue::string("button"),
                    HostValue::Object(
                        [
                            ("label".into(), HostValue::string("Increment")),
                            ("kind".into(), HostValue::string("primary")),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                ],
            )
            .expect("create");
        let nid = id.as_f64().expect("id") as u64;
        let snap = bridge.lock().unwrap().snapshot();
        let w = snap.get(nid).expect("widget");
        assert_eq!(w.kind, WidgetKind::Button);
        assert_eq!(w.props.label, "Increment");
        assert_eq!(w.props.button_kind, nana_ui_core::ButtonKind::Primary);
    }

    #[test]
    fn resolve_layout_uses_cascaded_style_model_geometry() {
        let doc = Arc::new(Mutex::new(NanaTreeDocument::new(400, 200, 1.0)));
        let bridge = Arc::new(Mutex::new(MessageBridge::new()));
        let mut api = HostApiRegistry::new();
        register_dom_host_ops_with_bridge(
            &mut api,
            Arc::clone(&doc),
            Arc::clone(&bridge),
            shared_web_api_state(),
        );
        let body = api.call("mountRoot", &[]).unwrap().as_f64().unwrap();
        let grid = api
            .call("createElement", &[HostValue::string("div")])
            .unwrap()
            .as_f64()
            .unwrap();
        let first = api
            .call("createElement", &[HostValue::string("div")])
            .unwrap()
            .as_f64()
            .unwrap();
        let second = api
            .call("createElement", &[HostValue::string("div")])
            .unwrap()
            .as_f64()
            .unwrap();
        api.call(
            "patchProp",
            &[
                HostValue::Number(grid),
                HostValue::string("class"),
                HostValue::string("measured-grid"),
            ],
        )
        .unwrap();
        for (child, parent) in [(grid, body), (first, grid), (second, grid)] {
            api.call(
                "insert",
                &[
                    HostValue::Number(child),
                    HostValue::Number(parent),
                    HostValue::Null,
                ],
            )
            .unwrap();
        }
        api.call(
            "injectStylesheet",
            &[HostValue::string(
                ".measured-grid{display:grid;grid-template-columns:100px 1fr;width:300px;height:40px}",
            )],
        )
        .unwrap();
        api.call("resolveLayout", &[]).unwrap();

        let doc = doc.lock().unwrap();
        let first = doc
            .layout_box(NodeHandle(first as u64))
            .expect("first cell");
        let second = doc
            .layout_box(NodeHandle(second as u64))
            .expect("second cell");
        assert!((first.width - 100.0).abs() < 0.5, "first={first:?}");
        assert!(
            (second.x - (first.x + first.width)).abs() < 0.5,
            "second={second:?}"
        );
    }

    #[test]
    fn closest_and_query_selector_all_host_ops() {
        let doc = Arc::new(Mutex::new(NanaTreeDocument::new(400, 300, 1.0)));
        let bridge = Arc::new(Mutex::new(MessageBridge::new()));
        let mut api = HostApiRegistry::new();
        register_dom_host_ops_with_bridge(
            &mut api,
            Arc::clone(&doc),
            Arc::clone(&bridge),
            shared_web_api_state(),
        );
        let outer = api
            .call("createElement", &[HostValue::string("div")])
            .unwrap()
            .as_f64()
            .unwrap();
        let inner = api
            .call("createElement", &[HostValue::string("span")])
            .unwrap()
            .as_f64()
            .unwrap();
        let body = {
            let guard = doc.lock().unwrap();
            guard.mount_root().0 as f64
        };
        api.call(
            "insert",
            &[
                HostValue::Number(outer),
                HostValue::Number(body),
                HostValue::Null,
            ],
        )
        .unwrap();
        api.call(
            "insert",
            &[
                HostValue::Number(inner),
                HostValue::Number(outer),
                HostValue::Null,
            ],
        )
        .unwrap();
        api.call(
            "patchProp",
            &[
                HostValue::Number(outer),
                HostValue::string("data-sidebar-repo-id"),
                HostValue::string("repo-1"),
            ],
        )
        .unwrap();
        api.call(
            "patchProp",
            &[
                HostValue::Number(outer),
                HostValue::string("class"),
                HostValue::string("home-pending-action is-confirming"),
            ],
        )
        .unwrap();
        api.call(
            "patchProp",
            &[
                HostValue::Number(inner),
                HostValue::string("id"),
                HostValue::string("sec-a"),
            ],
        )
        .unwrap();

        let closest = api
            .call(
                "closest",
                &[
                    HostValue::Number(inner),
                    HostValue::string("[data-sidebar-repo-id]"),
                ],
            )
            .unwrap();
        assert_eq!(closest.as_f64(), Some(outer));

        let compound = api
            .call(
                "closest",
                &[
                    HostValue::Number(inner),
                    HostValue::string(".home-pending-action.is-confirming"),
                ],
            )
            .unwrap();
        assert_eq!(compound.as_f64(), Some(outer));

        let all = api
            .call(
                "querySelectorAll",
                &[HostValue::string("[data-sidebar-repo-id], #sec-a")],
            )
            .unwrap();
        let HostValue::Array(ids) = all else {
            panic!("expected array");
        };
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn inner_html_patch_sets_element_text() {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let root = doc.mount_root();
        let el = doc.create_element("article");
        doc.insert(el, root, None);
        patch_prop(
            &mut doc,
            el,
            "innerHTML",
            HostValue::string("<strong>hi</strong>"),
        );
        assert_eq!(
            doc.get_attribute(el, "innerHTML").as_deref(),
            Some("<strong>hi</strong>")
        );
        let texts = doc.snapshot_boxes().texts;
        assert!(
            texts.iter().any(|(_, t)| t.contains("<strong>hi</strong>")),
            "expected text content from innerHTML, got {texts:?}"
        );
    }

    #[test]
    fn text_content_null_clears_element_text() {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let root = doc.mount_root();
        let el = doc.create_element("span");
        doc.set_element_text(el, "keep");
        doc.insert(el, root, None);
        patch_prop(&mut doc, el, "textContent", HostValue::Null);
        let texts = doc.snapshot_boxes().texts;
        assert!(
            !texts.iter().any(|(_, t)| t == "keep"),
            "null textContent should clear prior text"
        );
    }

    #[test]
    fn prop_and_attr_modifiers_strip_before_set() {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let root = doc.mount_root();
        let el = doc.create_element("input");
        doc.insert(el, root, None);
        // `.value` → DOM prop path equivalent (stored as attr on Nana tree).
        patch_prop(&mut doc, el, ".value", HostValue::string("hello"));
        assert_eq!(doc.get_attribute(el, "value").as_deref(), Some("hello"));
        // `^href` → force attribute.
        patch_prop(&mut doc, el, "^href", HostValue::string("#x"));
        assert_eq!(doc.get_attribute(el, "href").as_deref(), Some("#x"));
    }

    #[test]
    fn boolean_attr_false_removes_disabled() {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let root = doc.mount_root();
        let el = doc.create_element("button");
        doc.insert(el, root, None);
        patch_prop(&mut doc, el, "disabled", HostValue::Bool(true));
        assert_eq!(doc.get_attribute(el, "disabled").as_deref(), Some(""));
        patch_prop(&mut doc, el, "disabled", HostValue::Bool(false));
        assert!(doc.get_attribute(el, "disabled").is_none());
    }

    #[test]
    fn svg_common_attrs_preserve_casing() {
        let mut doc = NanaTreeDocument::new(400, 300, 1.0);
        let root = doc.mount_root();
        let el = doc.create_element("svg");
        doc.insert(el, root, None);
        patch_prop(&mut doc, el, "viewBox", HostValue::string("0 0 24 24"));
        patch_prop(&mut doc, el, "xlink:href", HostValue::string("#icon"));
        assert_eq!(
            doc.get_attribute(el, "viewBox").as_deref(),
            Some("0 0 24 24")
        );
        assert_eq!(
            doc.get_attribute(el, "xlink:href").as_deref(),
            Some("#icon")
        );
    }

    #[test]
    fn create_element_namespace_and_is_reach_bridge() {
        let doc = Arc::new(Mutex::new(NanaTreeDocument::new(400, 300, 1.0)));
        let bridge = Arc::new(Mutex::new(MessageBridge::new()));
        let mut api = HostApiRegistry::new();
        register_dom_host_ops_with_bridge(
            &mut api,
            Arc::clone(&doc),
            Arc::clone(&bridge),
            shared_web_api_state(),
        );
        let id = api
            .call(
                "createElement",
                &[
                    HostValue::string("path"),
                    HostValue::string("svg"),
                    HostValue::Null,
                    HostValue::Null,
                ],
            )
            .unwrap()
            .as_f64()
            .unwrap() as u64;
        assert_eq!(
            doc.lock().unwrap().element_namespace(NodeHandle(id)),
            Some(ElementNamespace::Svg)
        );
        let snap = bridge.lock().unwrap().snapshot();
        let w = snap.get(id).expect("widget");
        assert_eq!(
            w.props.attrs.get("data-nana-ns").map(String::as_str),
            Some("svg")
        );

        let custom = api
            .call(
                "createElement",
                &[
                    HostValue::string("p"),
                    HostValue::Null,
                    HostValue::string("fancy-p"),
                    HostValue::Null,
                ],
            )
            .unwrap()
            .as_f64()
            .unwrap() as u64;
        assert_eq!(
            doc.lock()
                .unwrap()
                .get_attribute(NodeHandle(custom), "is")
                .as_deref(),
            Some("fancy-p")
        );
    }

    #[test]
    fn clone_node_syncs_message_bridge_props_and_children() {
        let doc = Arc::new(Mutex::new(NanaTreeDocument::new(400, 300, 1.0)));
        let bridge = Arc::new(Mutex::new(MessageBridge::new()));
        let mut api = HostApiRegistry::new();
        register_dom_host_ops_with_bridge(
            &mut api,
            Arc::clone(&doc),
            Arc::clone(&bridge),
            shared_web_api_state(),
        );
        let outer = api
            .call("createElement", &[HostValue::string("div")])
            .unwrap()
            .as_f64()
            .unwrap();
        let inner = api
            .call("createElement", &[HostValue::string("span")])
            .unwrap()
            .as_f64()
            .unwrap();
        let body = doc.lock().unwrap().mount_root().0 as f64;
        api.call(
            "insert",
            &[
                HostValue::Number(outer),
                HostValue::Number(body),
                HostValue::Null,
            ],
        )
        .unwrap();
        api.call(
            "insert",
            &[
                HostValue::Number(inner),
                HostValue::Number(outer),
                HostValue::Null,
            ],
        )
        .unwrap();
        api.call(
            "patchProp",
            &[
                HostValue::Number(outer),
                HostValue::string("class"),
                HostValue::string("card"),
            ],
        )
        .unwrap();
        api.call(
            "patchProp",
            &[
                HostValue::Number(inner),
                HostValue::string("id"),
                HostValue::string("label"),
            ],
        )
        .unwrap();

        let copy = api
            .call(
                "cloneNode",
                &[HostValue::Number(outer), HostValue::Bool(true)],
            )
            .unwrap()
            .as_f64()
            .unwrap() as u64;
        assert_ne!(copy, outer as u64);
        let snap = bridge.lock().unwrap().snapshot();
        let cloned = snap.get(copy).expect("cloned outer in bridge");
        assert!(
            cloned.props.class_names.iter().any(|c| c == "card"),
            "clone must copy class into MessageBridge"
        );
        assert_eq!(cloned.children.len(), 1);
        let child_id = cloned.children[0];
        let child = snap.get(child_id).expect("cloned child");
        assert_eq!(child.props.element_id, "label");
    }

    #[test]
    fn insert_static_content_host_op_registers_and_reuses() {
        let doc = Arc::new(Mutex::new(NanaTreeDocument::new(400, 300, 1.0)));
        let bridge = Arc::new(Mutex::new(MessageBridge::new()));
        let mut api = HostApiRegistry::new();
        register_dom_host_ops_with_bridge(
            &mut api,
            Arc::clone(&doc),
            Arc::clone(&bridge),
            shared_web_api_state(),
        );
        let body = doc.lock().unwrap().mount_root().0 as f64;
        let pair = api
            .call(
                "insertStaticContent",
                &[
                    HostValue::string("<em>a</em><strong>b</strong>"),
                    HostValue::Number(body),
                    HostValue::Null,
                    HostValue::Null,
                    HostValue::Null,
                    HostValue::Null,
                ],
            )
            .unwrap();
        let HostValue::Array(ids) = pair else {
            panic!("expected [start,end]");
        };
        let start = ids[0].as_f64().unwrap() as u64;
        let end = ids[1].as_f64().unwrap() as u64;
        assert!(bridge.lock().unwrap().contains(start));
        assert!(bridge.lock().unwrap().contains(end));

        let reused = api
            .call(
                "insertStaticContent",
                &[
                    HostValue::string(""),
                    HostValue::Number(body),
                    HostValue::Null,
                    HostValue::Null,
                    HostValue::Number(start as f64),
                    HostValue::Number(end as f64),
                ],
            )
            .unwrap();
        let HostValue::Array(ids2) = reused else {
            panic!("expected reuse pair");
        };
        let c0 = ids2[0].as_f64().unwrap() as u64;
        assert_ne!(c0, start);
        assert!(bridge.lock().unwrap().contains(c0));
    }

    #[test]
    fn scroll_into_view_host_op_scrolls_overflow_ancestor() {
        use crate::bridge::WidgetProps;
        use nana_ui_core::{LengthSpec, OverflowSpec};

        shared_scroll_offset_store().clear();
        shared_layout_box_store().begin_frame();

        let doc = Arc::new(Mutex::new(NanaTreeDocument::new(400, 600, 1.0)));
        let bridge = Arc::new(Mutex::new(MessageBridge::new()));
        let mut api = HostApiRegistry::new();
        register_dom_host_ops_with_bridge(
            &mut api,
            Arc::clone(&doc),
            Arc::clone(&bridge),
            shared_web_api_state(),
        );

        let body = {
            let guard = doc.lock().unwrap();
            guard.mount_root().0 as f64
        };
        let scroller = api
            .call("createElement", &[HostValue::string("div")])
            .unwrap()
            .as_f64()
            .unwrap();
        let target = api
            .call("createElement", &[HostValue::string("div")])
            .unwrap()
            .as_f64()
            .unwrap();
        api.call(
            "insert",
            &[
                HostValue::Number(scroller),
                HostValue::Number(body),
                HostValue::Null,
            ],
        )
        .unwrap();
        api.call(
            "insert",
            &[
                HostValue::Number(target),
                HostValue::Number(scroller),
                HostValue::Null,
            ],
        )
        .unwrap();
        api.call(
            "patchProp",
            &[
                HostValue::Number(scroller),
                HostValue::string("style"),
                HostValue::string("overflow-y:auto;height:200px"),
            ],
        )
        .unwrap();

        {
            let mut props = WidgetProps::default();
            props.layout.overflow_y = OverflowSpec::Auto;
            props.layout.height = Some(LengthSpec::Px(200.0));
            let mut b = bridge.lock().unwrap();
            if let Some(w) = b.get_mut(scroller as u64) {
                w.props.layout = props.layout;
            }
        }

        let store = shared_layout_box_store();
        store.record(NodeHandle(scroller as u64), 0.0, 0.0, 300.0, 200.0);
        store.record(NodeHandle(target as u64), 0.0, 480.0, 300.0, 40.0);
        {
            let mut guard = doc.lock().unwrap();
            guard.apply_layout_boxes(&store.snapshot());
        }

        let result = api
            .call(
                "scrollIntoView",
                &[
                    HostValue::Number(target),
                    HostValue::Object(
                        [("block".into(), HostValue::string("start"))]
                            .into_iter()
                            .collect(),
                    ),
                ],
            )
            .expect("scrollIntoView");
        let HostValue::Object(map) = result else {
            panic!("expected object");
        };
        let HostValue::Array(scrolled) = map.get("scrolled").expect("scrolled") else {
            panic!("expected scrolled array");
        };
        assert_eq!(scrolled.len(), 1);

        let box_ =
            get_layout_box(&doc.lock().unwrap(), NodeHandle(target as u64)).expect("target box");
        assert!(
            (box_.y - 0.0).abs() < 1.0,
            "target should be at scrollport top after scrollIntoView, got y={}",
            box_.y
        );
        let off = api
            .call("getScrollOffset", &[HostValue::Number(scroller)])
            .unwrap();
        let HostValue::Object(off_map) = off else {
            panic!("offset object");
        };
        let top = off_map
            .get("scrollTop")
            .and_then(HostValue::as_f64)
            .unwrap_or(0.0);
        assert!((top - 480.0).abs() < 1.0, "scrollTop={top}");

        // Avoid leaking process-wide store state into other tests.
        shared_scroll_offset_store().clear();
        shared_layout_box_store().begin_frame();
    }

    #[test]
    fn teleport_body_overlay_insert_remove_clears_bridge() {
        // X7 / D-03: Teleport `to="body"` → mount_root; L2 Dialog under body
        // coexists with a CSS fixed sibling; remove must not leak bridge widgets.
        let doc = Arc::new(Mutex::new(NanaTreeDocument::new(400, 300, 1.0)));
        let bridge = Arc::new(Mutex::new(MessageBridge::new()));
        {
            let guard = doc.lock().unwrap();
            bridge
                .lock()
                .unwrap()
                .ensure_document_roots(guard.html_root().0, guard.mount_root().0);
        }
        let mut api = HostApiRegistry::new();
        register_dom_host_ops_with_bridge(
            &mut api,
            Arc::clone(&doc),
            Arc::clone(&bridge),
            shared_web_api_state(),
        );

        let body = api
            .call("querySelector", &[HostValue::string("body")])
            .unwrap()
            .as_f64()
            .unwrap();
        let mount = api.call("mountRoot", &[]).unwrap().as_f64().unwrap();
        assert_eq!(body, mount, "querySelector(body) === mountRoot");

        let dialog = api
            .call(
                "createWidget",
                &[
                    HostValue::string("dialog"),
                    HostValue::Object(
                        [
                            ("label".into(), HostValue::string("Confirm")),
                            ("active".into(), HostValue::Bool(true)),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                ],
            )
            .unwrap()
            .as_f64()
            .unwrap();
        let pin = api
            .call("createElement", &[HostValue::string("div")])
            .unwrap()
            .as_f64()
            .unwrap();
        api.call(
            "patchProp",
            &[
                HostValue::Number(pin),
                HostValue::string("style"),
                HostValue::string("position:fixed;top:4px;right:4px;width:20px;height:20px"),
            ],
        )
        .unwrap();
        api.call(
            "insert",
            &[
                HostValue::Number(dialog),
                HostValue::Number(body),
                HostValue::Null,
            ],
        )
        .unwrap();
        api.call(
            "insert",
            &[
                HostValue::Number(pin),
                HostValue::Number(body),
                HostValue::Null,
            ],
        )
        .unwrap();

        {
            let b = bridge.lock().unwrap();
            assert!(b.contains(dialog as u64));
            assert!(b.contains(pin as u64));
            let snap = b.snapshot();
            assert_eq!(snap.get(dialog as u64).unwrap().kind, WidgetKind::Dialog);
            assert!(snap.get(dialog as u64).unwrap().kind.is_overlay());
            assert!(
                !snap.get(dialog as u64).unwrap().props.layout.is_fixed(),
                "Overlay must not keep companion fixed"
            );
            assert!(snap.get(pin as u64).unwrap().props.layout.is_fixed());
        }
        {
            let d = doc.lock().unwrap();
            assert!(d.contains(NodeHandle(body as u64), NodeHandle(dialog as u64)));
            assert!(d.contains(NodeHandle(body as u64), NodeHandle(pin as u64)));
        }

        api.call("remove", &[HostValue::Number(dialog)]).unwrap();
        {
            let b = bridge.lock().unwrap();
            assert!(
                !b.contains(dialog as u64),
                "Dialog must leave bridge on remove"
            );
            assert!(
                b.contains(pin as u64),
                "fixed sibling must survive Dialog remove"
            );
        }
        {
            let d = doc.lock().unwrap();
            assert!(
                !d.contains(NodeHandle(dialog as u64), NodeHandle(dialog as u64)),
                "Dialog node must leave the document on remove"
            );
            assert_eq!(
                d.children_of(NodeHandle(body as u64)),
                vec![NodeHandle(pin as u64)]
            );
            assert_eq!(d.query_selector("body"), Some(NodeHandle(body as u64)));
        }
    }
}
