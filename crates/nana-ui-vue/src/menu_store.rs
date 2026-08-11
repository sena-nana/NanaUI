//! Host-owned [`nana_ui::ContextMenuItem`] store for L2 `ContextMenu`.
//!
//! [`ContextMenuHost`] borrows `&[ContextMenuItem]`, so the hosted
//! `Element<'static>` path keeps items in this store (same contract as
//! [`crate::EditorStore`]).

use nana_ui::ContextMenuItem;

use crate::bridge::{SemanticSnapshot, WidgetId, WidgetKind, WidgetProps};

#[derive(Debug, Default)]
pub struct MenuSlot {
    pub items: Vec<ContextMenuItem<'static, String>>,
    pub query: String,
    pub active_path: Vec<usize>,
    pub searchable: bool,
    /// Two-step confirm target for danger items (`ContextMenuHost::pending`).
    pub pending: Option<String>,
    /// Fingerprint of the last synced `props.options` set.
    options_key: String,
}

/// Caller-owned context-menu item trees keyed by semantic widget id.
#[derive(Debug, Default)]
pub struct MenuStore {
    menus: std::collections::HashMap<WidgetId, MenuSlot>,
}

impl MenuStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sync flat `props.options` into host-owned [`ContextMenuItem`] trees.
    ///
    /// Nested menus: option values shaped as `a/b` or `a/b/c` build a tree
    /// (intermediate segments are created as needed); otherwise items stay flat.
    ///
    /// When the option set changes, clears stale `query` / `active_path` and
    /// drops `pending` unless the armed value still exists in the new tree.
    pub fn sync_from_snapshot(&mut self, snap: &SemanticSnapshot) {
        let ids: Vec<WidgetId> = snap
            .widgets
            .iter()
            .filter(|w| w.kind == WidgetKind::ContextMenu)
            .map(|w| w.id)
            .collect();
        self.menus.retain(|id, _| ids.contains(id));
        for widget in &snap.widgets {
            if widget.kind != WidgetKind::ContextMenu {
                continue;
            }
            let items = items_from_props(&widget.props);
            let options_key = options_key(&widget.props);
            let searchable = widget.props.options.len() >= 6
                || widget
                    .props
                    .class_names
                    .iter()
                    .any(|c| c.contains("search"));
            let slot = self.menus.entry(widget.id).or_default();
            let options_changed = slot.options_key != options_key;
            slot.items = items;
            slot.searchable = searchable;
            slot.options_key = options_key;
            if options_changed {
                slot.query.clear();
                slot.active_path.clear();
                if slot
                    .pending
                    .as_ref()
                    .is_some_and(|pending| !items_contain_value(&slot.items, pending))
                {
                    slot.pending = None;
                }
            }
        }
    }

    pub fn set_query(&mut self, id: WidgetId, query: String) {
        self.menus.entry(id).or_default().query = query;
    }

    pub fn set_active_path(&mut self, id: WidgetId, path: Vec<usize>) {
        self.menus.entry(id).or_default().active_path = path;
    }

    pub fn set_pending(&mut self, id: WidgetId, value: Option<String>) {
        self.menus.entry(id).or_default().pending = value;
    }

    pub fn pending(&self, id: WidgetId) -> Option<&str> {
        self.menus.get(&id).and_then(|s| s.pending.as_deref())
    }

    /// Returns `true` when the select should be held for a confirm click.
    pub fn arm_danger_confirm(&mut self, id: WidgetId, value: &str) -> bool {
        let Some(slot) = self.menus.get_mut(&id) else {
            return false;
        };
        let needs = slot
            .items
            .iter()
            .any(|item| item_needs_confirm(item, value));
        if !needs {
            slot.pending = None;
            return false;
        }
        if slot.pending.as_deref() == Some(value) {
            slot.pending = None;
            false
        } else {
            slot.pending = Some(value.to_string());
            true
        }
    }

    pub fn get(&self, id: WidgetId) -> Option<&MenuSlot> {
        self.menus.get(&id)
    }

    /// `'static` borrow for HostedUi `Element<'static>` construction.
    ///
    /// # Safety contract (caller)
    /// Drop the Element before mutating this store (`sync_from_snapshot`,
    /// `set_query`, `set_active_path`).
    pub fn items_static(
        &self,
        id: WidgetId,
    ) -> Option<&'static [ContextMenuItem<'static, String>]> {
        self.menus.get(&id).map(|slot| {
            // SAFETY: HostedUiRenderer drops the previous Element before the
            // next program update mutates this store.
            unsafe { std::slice::from_raw_parts(slot.items.as_ptr(), slot.items.len()) }
        })
    }

    pub fn query_static(&self, id: WidgetId) -> Option<&'static str> {
        self.menus.get(&id).map(|slot| {
            // SAFETY: same frame contract as [`Self::items_static`].
            unsafe { &*(slot.query.as_str() as *const str) }
        })
    }

    pub fn pending_static(&self, id: WidgetId) -> Option<&'static String> {
        self.menus.get(&id).and_then(|slot| {
            slot.pending.as_ref().map(|pending| {
                // SAFETY: same frame contract as [`Self::items_static`].
                unsafe { &*(pending as *const String) }
            })
        })
    }

    pub fn path_static(&self, id: WidgetId) -> Option<&'static [usize]> {
        self.menus.get(&id).map(|slot| {
            // SAFETY: same frame contract as [`Self::items_static`].
            unsafe { std::slice::from_raw_parts(slot.active_path.as_ptr(), slot.active_path.len()) }
        })
    }
}

fn items_from_props(props: &WidgetProps) -> Vec<ContextMenuItem<'static, String>> {
    if props.options.is_empty() {
        let label = if props.label.is_empty() {
            "菜单".to_string()
        } else {
            props.label.clone()
        };
        return vec![ContextMenuItem::new(label.clone(), label)];
    }

    let mut roots: Vec<ContextMenuItem<'static, String>> = Vec::new();

    for opt in &props.options {
        let value = opt.value.clone();
        let label = opt.label.clone();
        let danger = option_is_danger(&value, &label);
        let segments: Vec<&str> = value.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            continue;
        }
        if segments.len() == 1 {
            push_or_replace_child(&mut roots, make_item(value, label, opt.disabled, danger));
            continue;
        }

        // Walk / create ancestors, then attach the leaf.
        let mut parent_path = String::new();
        for (i, seg) in segments.iter().enumerate() {
            let is_leaf = i + 1 == segments.len();
            let current = if parent_path.is_empty() {
                (*seg).to_string()
            } else {
                format!("{parent_path}/{seg}")
            };
            if is_leaf {
                let leaf = make_item(value.clone(), label.clone(), opt.disabled, danger);
                if parent_path.is_empty() {
                    push_or_replace_child(&mut roots, leaf);
                } else {
                    let path = parent_path.clone();
                    if let Some(parent) = find_mut(&mut roots, &path) {
                        push_or_replace_child(&mut parent.children, leaf);
                    } else {
                        roots.push(leaf);
                    }
                }
            } else if find_mut(&mut roots, &current).is_none() {
                let stub = ContextMenuItem::new(current.clone(), (*seg).to_string());
                if parent_path.is_empty() {
                    roots.push(stub);
                } else {
                    let path = parent_path.clone();
                    if let Some(parent) = find_mut(&mut roots, &path) {
                        parent.children.push(stub);
                    } else {
                        roots.push(stub);
                    }
                }
            }
            parent_path = current;
        }
    }
    roots
}

fn make_item(
    value: String,
    label: String,
    disabled: bool,
    danger: bool,
) -> ContextMenuItem<'static, String> {
    let mut item = ContextMenuItem::new(value, label.clone());
    if disabled {
        item = item.disabled(true);
    }
    if danger {
        // Host owns confirm copy; reuse the item label (no invented locale string).
        item = item.danger(true).confirm_label(label);
    }
    item
}

fn push_or_replace_child(
    siblings: &mut Vec<ContextMenuItem<'static, String>>,
    mut item: ContextMenuItem<'static, String>,
) {
    if let Some(idx) = siblings.iter().position(|c| c.value == item.value) {
        let children = std::mem::take(&mut siblings[idx].children);
        item.children = children;
        siblings[idx] = item;
    } else {
        siblings.push(item);
    }
}

fn find_mut<'a>(
    items: &'a mut [ContextMenuItem<'static, String>],
    value: &str,
) -> Option<&'a mut ContextMenuItem<'static, String>> {
    let mut i = 0;
    while i < items.len() {
        if items[i].value == value {
            return Some(&mut items[i]);
        }
        // Search children without holding a conflicting borrow on the parent slice.
        if items[i]
            .children
            .iter()
            .any(|c| item_contains_value(c, value))
        {
            return find_mut(&mut items[i].children, value);
        }
        i += 1;
    }
    None
}

fn item_contains_value(item: &ContextMenuItem<'static, String>, value: &str) -> bool {
    item.value == value || item.children.iter().any(|c| item_contains_value(c, value))
}

fn option_is_danger(value: &str, label: &str) -> bool {
    let hay = format!("{value} {label}").to_ascii_lowercase();
    [
        "delete", "remove", "destroy", "trash", "危险", "删除", "移除", "清空", "注销",
    ]
    .iter()
    .any(|k| hay.contains(k))
}

fn item_needs_confirm(item: &ContextMenuItem<'static, String>, value: &str) -> bool {
    if item.value == value {
        return item.danger && item.confirm_label.is_some();
    }
    item.children
        .iter()
        .any(|child| item_needs_confirm(child, value))
}

fn options_key(props: &WidgetProps) -> String {
    if props.options.is_empty() {
        return format!("label:{}", props.label);
    }
    props
        .options
        .iter()
        .map(|o| format!("{}|{}\t{}", o.value, o.label, o.disabled as u8))
        .collect::<Vec<_>>()
        .join("\n")
}

fn items_contain_value(items: &[ContextMenuItem<'static, String>], value: &str) -> bool {
    items
        .iter()
        .any(|item| item.value == value || items_contain_value(&item.children, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{MessageBridge, SelectOptionProp, WidgetKind, WidgetProps};

    fn menu_snap(options: Vec<SelectOptionProp>) -> crate::bridge::SemanticSnapshot {
        let mut bridge = MessageBridge::new();
        bridge.register(1, WidgetKind::Column, WidgetProps::default());
        bridge.register(
            2,
            WidgetKind::ContextMenu,
            WidgetProps {
                active: true,
                options,
                ..WidgetProps::default()
            },
        );
        bridge.insert_child(2, 1, None);
        bridge.snapshot()
    }

    #[test]
    fn options_change_resets_query_path_and_stale_pending() {
        let snap = menu_snap(vec![
            SelectOptionProp {
                value: "a".into(),
                label: "A".into(),
                disabled: false,
            },
            SelectOptionProp {
                value: "delete".into(),
                label: "删除".into(),
                disabled: false,
            },
        ]);
        let mut menus = MenuStore::new();
        menus.sync_from_snapshot(&snap);
        menus.set_query(2, "del".into());
        menus.set_active_path(2, vec![1]);
        menus.set_pending(2, Some("delete".into()));

        let snap2 = menu_snap(vec![SelectOptionProp {
            value: "b".into(),
            label: "B".into(),
            disabled: false,
        }]);
        menus.sync_from_snapshot(&snap2);
        let slot = menus.get(2).unwrap();
        assert!(slot.query.is_empty());
        assert!(slot.active_path.is_empty());
        assert!(slot.pending.is_none());
    }

    #[test]
    fn options_change_keeps_pending_when_value_survives() {
        let snap = menu_snap(vec![
            SelectOptionProp {
                value: "keep".into(),
                label: "保留".into(),
                disabled: false,
            },
            SelectOptionProp {
                value: "delete".into(),
                label: "删除".into(),
                disabled: false,
            },
        ]);
        let mut menus = MenuStore::new();
        menus.sync_from_snapshot(&snap);
        menus.set_pending(2, Some("delete".into()));

        let snap2 = menu_snap(vec![
            SelectOptionProp {
                value: "delete".into(),
                label: "删除".into(),
                disabled: false,
            },
            SelectOptionProp {
                value: "extra".into(),
                label: "额外".into(),
                disabled: false,
            },
        ]);
        menus.sync_from_snapshot(&snap2);
        assert_eq!(menus.pending(2), Some("delete"));
        assert!(menus.get(2).unwrap().query.is_empty());
        assert!(menus.get(2).unwrap().active_path.is_empty());
    }

    #[test]
    fn nested_slash_path_builds_three_levels() {
        let snap = menu_snap(vec![
            SelectOptionProp {
                value: "file".into(),
                label: "文件".into(),
                disabled: false,
            },
            SelectOptionProp {
                value: "file/edit/rename".into(),
                label: "重命名".into(),
                disabled: false,
            },
            SelectOptionProp {
                value: "file/edit/delete".into(),
                label: "删除".into(),
                disabled: false,
            },
        ]);
        let mut menus = MenuStore::new();
        menus.sync_from_snapshot(&snap);
        let roots = &menus.get(2).unwrap().items;
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].value, "file");
        assert_eq!(roots[0].children.len(), 1);
        assert_eq!(roots[0].children[0].value, "file/edit");
        assert_eq!(roots[0].children[0].children.len(), 2);
        assert_eq!(roots[0].children[0].children[1].value, "file/edit/delete");
        assert!(roots[0].children[0].children[1].danger);
        assert!(menus.arm_danger_confirm(2, "file/edit/delete"));
        assert_eq!(menus.pending(2), Some("file/edit/delete"));
    }
}
