// Copyright 2022 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

use accesskit::{
    ActionHandler, ActivationHandler, Live, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate,
};
use accesskit_consumer::{FilterResult, FullNodeId, NodeRef, Tree, TreeChangeHandler, TreeState};
use hashbrown::{HashMap, HashSet};
use std::fmt::{Debug, Formatter};
use std::sync::{Arc, atomic::Ordering};
use windows::{
    Win32::{
        Foundation::*,
        UI::{Accessibility::*, WindowsAndMessaging::*},
    },
    core::ComObject,
};

use crate::{
    context::{ActionHandlerNoMut, ActionHandlerWrapper, Context},
    filters::filter,
    node::{NodeWrapper, PlatformNode},
    util::{QueuedEvent, StringBuffer},
    window_handle::WindowHandle,
};

fn focus_event(context: &Arc<Context>, node_id: FullNodeId) -> QueuedEvent {
    let platform_node = context.get_or_create_platform_node(node_id);
    let element: IRawElementProviderSimple = platform_node.into_interface();
    QueuedEvent::Simple {
        element,
        event_id: UIA_AutomationFocusChangedEventId,
    }
}

fn clients_are_listening() -> bool {
    unsafe { UiaClientsAreListening() }.as_bool()
}

fn opened_closed_events(node: &NodeRef) -> Option<(UIA_EVENT_ID, UIA_EVENT_ID)> {
    if node.is_dialog() {
        Some((
            UIA_Window_WindowOpenedEventId,
            UIA_Window_WindowClosedEventId,
        ))
    } else {
        match node.role() {
            Role::Menu | Role::MenuListPopup => {
                Some((UIA_MenuOpenedEventId, UIA_MenuClosedEventId))
            }
            Role::Tooltip => Some((UIA_ToolTipOpenedEventId, UIA_ToolTipClosedEventId)),
            _ => None,
        }
    }
}

struct AdapterChangeHandler<'a> {
    context: &'a Arc<Context>,
    emit_events: bool,
    queue: Vec<QueuedEvent>,
    text_changed_events: Vec<QueuedEvent>,
    text_changed: HashSet<FullNodeId>,
    selection_changed: HashMap<FullNodeId, SelectionChanges>,
}

impl<'a> AdapterChangeHandler<'a> {
    fn new(context: &'a Arc<Context>, emit_events: bool) -> Self {
        Self {
            context,
            emit_events,
            queue: Vec::new(),
            text_changed_events: Vec::new(),
            text_changed: HashSet::new(),
            selection_changed: HashMap::new(),
        }
    }

    fn finish(mut self) -> Vec<QueuedEvent> {
        if self.text_changed_events.is_empty() {
            self.queue
        } else {
            self.text_changed_events.reserve(self.queue.len());
            self.text_changed_events.append(&mut self.queue);
            self.text_changed_events
        }
    }
}

impl AdapterChangeHandler<'_> {
    fn insert_text_change_if_needed_parent(&mut self, node: NodeRef) {
        if !node.supports_text_ranges() {
            return;
        }
        let id = node.id();
        if !self.text_changed.insert(id) {
            return;
        }
        let platform_node = self.context.get_or_create_platform_node(id);
        let element: IRawElementProviderSimple = platform_node.into_interface();
        // Text change events must come before selection change
        // events. It doesn't matter if text change events come
        // before other events.
        self.text_changed_events.push(QueuedEvent::Simple {
            element,
            event_id: UIA_Text_TextChangedEventId,
        });
    }

    fn insert_text_change_if_needed(&mut self, node: &NodeRef) {
        if node.role() != Role::TextRun {
            return;
        }
        if let Some(node) = node.filtered_parent(&filter) {
            self.insert_text_change_if_needed_parent(node);
        }
    }

    fn element_for_possibly_removed_node(
        &self,
        tree_state: &TreeState,
        id: FullNodeId,
    ) -> IRawElementProviderSimple {
        if tree_state.node_by_id(id).is_some() {
            self.context
                .get_or_create_platform_node(id)
                .into_interface()
        } else {
            PlatformNode::new(self.context, id).into_interface()
        }
    }

    fn handle_selection_state_change(&mut self, node: &NodeRef, is_selected: bool) {
        // If `node` belongs to a selection container, then map the events with the
        // selection container as the key because |FinalizeSelectionEvents| needs to
        // determine whether or not there is only one element selected in order to
        // optimize what platform events are sent.
        let key = if let Some(container) = node.selection_container(&filter) {
            container.id()
        } else {
            node.id()
        };

        let changes = self
            .selection_changed
            .entry(key)
            .or_insert_with(|| SelectionChanges {
                added_items: HashSet::new(),
                removed_items: HashSet::new(),
            });
        if is_selected {
            changes.added_items.insert(node.id());
        } else {
            changes.removed_items.insert(node.id());
        }
    }

    fn enqueue_selection_changes(&mut self, tree: &Tree) {
        if !self.emit_events || self.selection_changed.is_empty() {
            return;
        }
        let tree_state = tree.state();
        for (id, changes) in self.selection_changed.iter() {
            let Some(node) = tree_state.node_by_id(*id) else {
                continue;
            };
            // Determine if `node` is a selection container with one selected child in
            // order to optimize what platform events are sent.
            let mut container = None;
            let mut only_selected_child = None;
            if node.is_container_with_selectable_children() {
                container = Some(node);
                for child in node.filtered_children(filter) {
                    if let Some(true) = child.is_selected() {
                        if only_selected_child.is_none() {
                            only_selected_child = Some(child);
                        } else {
                            only_selected_child = None;
                            break;
                        }
                    }
                }
            }

            if let Some(only_selected_child) = only_selected_child {
                self.queue.push(QueuedEvent::Simple {
                    element: self
                        .context
                        .get_or_create_platform_node(only_selected_child.id())
                        .into_interface(),
                    event_id: UIA_SelectionItem_ElementSelectedEventId,
                });
                self.queue.push(QueuedEvent::PropertyChanged {
                    element: self
                        .context
                        .get_or_create_platform_node(only_selected_child.id())
                        .into_interface(),
                    property_id: UIA_SelectionItemIsSelectedPropertyId,
                    old_value: false.into(),
                    new_value: true.into(),
                });
                for child_id in changes.removed_items.iter() {
                    let element = self.element_for_possibly_removed_node(tree_state, *child_id);
                    self.queue.push(QueuedEvent::PropertyChanged {
                        element,
                        property_id: UIA_SelectionItemIsSelectedPropertyId,
                        old_value: true.into(),
                        new_value: false.into(),
                    });
                }
            } else {
                // Per UIA documentation, beyond the "invalidate limit" we're supposed to
                // fire a 'SelectionInvalidated' event.  The exact value isn't specified,
                // but System.Windows.Automation.Provider uses a value of 20.
                const INVALIDATE_LIMIT: usize = 20;
                if let Some(container) = container.filter(|_| {
                    changes.added_items.len() + changes.removed_items.len() > INVALIDATE_LIMIT
                }) {
                    let platform_node = self.context.get_or_create_platform_node(container.id());
                    self.queue.push(QueuedEvent::Simple {
                        element: platform_node.into_interface(),
                        event_id: UIA_Selection_InvalidatedEventId,
                    });
                } else {
                    let container_is_multiselectable =
                        container.is_some_and(|c| c.is_multiselectable());
                    for added_id in changes.added_items.iter() {
                        self.queue.push(QueuedEvent::Simple {
                            element: self
                                .context
                                .get_or_create_platform_node(*added_id)
                                .into_interface(),
                            event_id: match container_is_multiselectable {
                                true => UIA_SelectionItem_ElementAddedToSelectionEventId,
                                false => UIA_SelectionItem_ElementSelectedEventId,
                            },
                        });
                        self.queue.push(QueuedEvent::PropertyChanged {
                            element: self
                                .context
                                .get_or_create_platform_node(*added_id)
                                .into_interface(),
                            property_id: UIA_SelectionItemIsSelectedPropertyId,
                            old_value: false.into(),
                            new_value: true.into(),
                        });
                    }
                    for removed_id in changes.removed_items.iter() {
                        let element =
                            self.element_for_possibly_removed_node(tree_state, *removed_id);
                        self.queue.push(QueuedEvent::Simple {
                            element: element.clone(),
                            event_id: UIA_SelectionItem_ElementRemovedFromSelectionEventId,
                        });
                        self.queue.push(QueuedEvent::PropertyChanged {
                            element,
                            property_id: UIA_SelectionItemIsSelectedPropertyId,
                            old_value: true.into(),
                            new_value: false.into(),
                        });
                    }
                }
            }
        }
    }
}

struct SelectionChanges {
    added_items: HashSet<FullNodeId>,
    removed_items: HashSet<FullNodeId>,
}

impl TreeChangeHandler for AdapterChangeHandler<'_> {
    fn node_added(&mut self, node: &NodeRef) {
        if !self.emit_events {
            return;
        }
        self.insert_text_change_if_needed(node);
        if filter(node) != FilterResult::Include {
            return;
        }
        let mut buffer = StringBuffer::acquire();
        let mut wrapper = NodeWrapper {
            node,
            string_buffer: &mut buffer,
        };
        if let Some((opened_event_id, _)) = opened_closed_events(node) {
            let platform_node = self.context.get_or_create_platform_node(node.id());
            let element: IRawElementProviderSimple = platform_node.into_interface();
            self.queue.push(QueuedEvent::Simple {
                element,
                event_id: opened_event_id,
            });
        }
        if node.live() != Live::Off && wrapper.name().is_some() {
            let platform_node = self.context.get_or_create_platform_node(node.id());
            let element: IRawElementProviderSimple = platform_node.into_interface();
            self.queue.push(QueuedEvent::Simple {
                element,
                event_id: UIA_LiveRegionChangedEventId,
            });
        }
        if wrapper.is_selection_item_pattern_supported() && wrapper.is_selected() {
            self.handle_selection_state_change(node, true);
        }
    }

    fn node_updated(&mut self, old_node: &NodeRef, new_node: &NodeRef) {
        if !self.emit_events {
            return;
        }
        if old_node.raw_value() != new_node.raw_value() {
            self.insert_text_change_if_needed(new_node);
        }
        let old_node_was_filtered_out = filter(old_node) != FilterResult::Include;
        if filter(new_node) != FilterResult::Include {
            if !old_node_was_filtered_out {
                if let Some((_, closed_event_id)) = opened_closed_events(old_node) {
                    let platform_node = self.context.get_or_create_platform_node(old_node.id());
                    let element: IRawElementProviderSimple = platform_node.into_interface();
                    self.queue.push(QueuedEvent::Simple {
                        element,
                        event_id: closed_event_id,
                    });
                }
                let mut old_buffer = StringBuffer::acquire();
                let old_wrapper = NodeWrapper {
                    node: old_node,
                    string_buffer: &mut old_buffer,
                };
                if old_wrapper.is_selection_item_pattern_supported() && old_wrapper.is_selected() {
                    self.handle_selection_state_change(old_node, false);
                }
            }
            return;
        }
        let platform_node = self.context.get_or_create_platform_node(new_node.id());
        let element: IRawElementProviderSimple = platform_node.into_interface();
        let mut old_buffer = StringBuffer::acquire();
        let mut old_wrapper = NodeWrapper {
            node: old_node,
            string_buffer: &mut old_buffer,
        };
        let mut new_buffer = StringBuffer::acquire();
        let mut new_wrapper = NodeWrapper {
            node: new_node,
            string_buffer: &mut new_buffer,
        };
        new_wrapper.enqueue_property_changes(
            &mut self.queue,
            self.context,
            &element,
            &mut old_wrapper,
        );
        if new_node.live() != Live::Off {
            let new_name = new_wrapper.name();
            if new_name.is_some()
                && (new_node.live() != old_node.live()
                    || old_node_was_filtered_out
                    || new_name != old_wrapper.name())
            {
                self.queue.push(QueuedEvent::Simple {
                    element: element.clone(),
                    event_id: UIA_LiveRegionChangedEventId,
                });
            }
        }
        let old_events = (!old_node_was_filtered_out)
            .then(|| opened_closed_events(old_node))
            .flatten();
        let new_events = opened_closed_events(new_node);
        if old_events != new_events {
            if let Some((_, closed_event_id)) = old_events {
                self.queue.push(QueuedEvent::Simple {
                    element: element.clone(),
                    event_id: closed_event_id,
                });
            }
            if let Some((opened_event_id, _)) = new_events {
                self.queue.push(QueuedEvent::Simple {
                    element: element.clone(),
                    event_id: opened_event_id,
                });
            }
        }
        if new_wrapper.is_selection_item_pattern_supported()
            && (new_wrapper.is_selected() != old_wrapper.is_selected()
                || (old_node_was_filtered_out && new_wrapper.is_selected()))
        {
            self.handle_selection_state_change(new_node, new_wrapper.is_selected());
        }
    }

    fn focus_moved(&mut self, _old_node: Option<&NodeRef>, new_node: Option<&NodeRef>) {
        if !self.emit_events {
            return;
        }
        if let Some(new_node) = new_node {
            self.queue.push(focus_event(self.context, new_node.id()));
        }
    }

    fn node_removed(&mut self, node: &NodeRef) {
        self.context.remove_platform_node(node.id());
        if !self.emit_events {
            return;
        }
        self.insert_text_change_if_needed(node);
        if filter(node) != FilterResult::Include {
            return;
        }
        if let Some((_, closed_event_id)) = opened_closed_events(node) {
            let platform_node = PlatformNode::new(self.context, node.id());
            let element: IRawElementProviderSimple = platform_node.into_interface();
            self.queue.push(QueuedEvent::Simple {
                element,
                event_id: closed_event_id,
            });
        }
        let mut buffer = StringBuffer::acquire();
        let wrapper = NodeWrapper {
            node,
            string_buffer: &mut buffer,
        };
        if wrapper.is_selection_item_pattern_supported() {
            self.handle_selection_state_change(node, false);
        }
    }

    // TODO: handle other events (#20)
}

const PLACEHOLDER_ROOT_ID: NodeId = NodeId(0);

enum State {
    Inactive {
        hwnd: WindowHandle,
        is_window_focused: bool,
        action_handler: Arc<dyn ActionHandlerNoMut + Send + Sync>,
    },
    Placeholder(Arc<Context>),
    Active(Arc<Context>),
}

impl Debug for State {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            State::Inactive {
                hwnd,
                is_window_focused,
                action_handler: _,
            } => f
                .debug_struct("Inactive")
                .field("hwnd", hwnd)
                .field("is_window_focused", is_window_focused)
                .field("action_handler", &"ActionHandler")
                .finish(),
            State::Placeholder(context) => f.debug_tuple("Placeholder").field(context).finish(),
            State::Active(context) => f.debug_tuple("Active").field(context).finish(),
        }
    }
}

#[derive(Debug)]
pub struct Adapter {
    state: State,
}

impl Adapter {
    /// Creates a new Windows platform adapter.
    ///
    /// The action handler may or may not be called on the thread that owns
    /// the window.
    ///
    /// This must not be called while handling the `WM_GETOBJECT` message,
    /// because this function must initialize UI Automation before
    /// that message is handled. This is necessary to prevent a race condition
    /// that leads to nested `WM_GETOBJECT` messages and, in some cases,
    /// assistive technologies not realizing that the window natively implements.
    /// UIA. See [AccessKit issue #37](https://github.com/AccessKit/accesskit/issues/37)
    /// for more details.
    pub fn new(
        hwnd: HWND,
        is_window_focused: bool,
        action_handler: impl 'static + ActionHandler + Send,
    ) -> Self {
        Self::with_wrapped_action_handler(
            hwnd,
            is_window_focused,
            Arc::new(ActionHandlerWrapper::new(action_handler)),
        )
    }

    // Currently required by the test infrastructure
    pub(crate) fn with_wrapped_action_handler(
        hwnd: HWND,
        is_window_focused: bool,
        action_handler: Arc<dyn ActionHandlerNoMut + Send + Sync>,
    ) -> Self {
        init_uia();

        let state = State::Inactive {
            hwnd: hwnd.into(),
            is_window_focused,
            action_handler,
        };
        Self { state }
    }

    /// If and only if the tree has been initialized, call the provided function
    /// and apply the resulting update. Note: If the caller's implementation of
    /// [`ActivationHandler::request_initial_tree`] initially returned `None`,
    /// the [`TreeUpdate`] returned by the provided function must contain
    /// a full tree.
    ///
    /// If a [`QueuedEvents`] instance is returned, the caller must call
    /// [`QueuedEvents::raise`] on it.
    ///
    /// This method may be safely called on any thread, but refer to
    /// [`QueuedEvents::raise`] for restrictions on the context in which
    /// it should be called.
    pub fn update_if_active(
        &mut self,
        update_factory: impl FnOnce() -> TreeUpdate,
    ) -> Option<QueuedEvents> {
        if matches!(self.state, State::Inactive { .. }) {
            return None;
        }
        self.update_if_active_with_listening(update_factory, clients_are_listening())
    }

    fn update_if_active_with_listening(
        &mut self,
        update_factory: impl FnOnce() -> TreeUpdate,
        emit_events: bool,
    ) -> Option<QueuedEvents> {
        match &self.state {
            State::Inactive { .. } => None,
            State::Placeholder(context) => {
                let is_window_focused = context.read_tree().state().is_host_focused();
                let tree = Tree::new(update_factory(), is_window_focused);
                *context.tree.write().unwrap() = tree;
                context.clear_platform_nodes();
                context.is_placeholder.store(false, Ordering::SeqCst);
                let result = if emit_events {
                    context
                        .read_tree()
                        .state()
                        .focus()
                        .map(|node| QueuedEvents(vec![focus_event(context, node.id())]))
                } else {
                    None
                };
                self.state = State::Active(Arc::clone(context));
                result
            }
            State::Active(context) => {
                let mut handler = AdapterChangeHandler::new(context, emit_events);
                let mut tree = context.tree.write().unwrap();
                tree.update_and_process_changes(update_factory(), &mut handler);
                handler.enqueue_selection_changes(&tree);
                Some(QueuedEvents(handler.finish()))
            }
        }
    }

    /// Update the tree state based on whether the window is focused.
    ///
    /// If a [`QueuedEvents`] instance is returned, the caller must call
    /// [`QueuedEvents::raise`] on it.
    ///
    /// This method may be safely called on any thread, but refer to
    /// [`QueuedEvents::raise`] for restrictions on the context in which
    /// it should be called.
    pub fn update_window_focus_state(&mut self, is_focused: bool) -> Option<QueuedEvents> {
        match &mut self.state {
            State::Inactive {
                is_window_focused, ..
            } => {
                *is_window_focused = is_focused;
                None
            }
            State::Placeholder(context) | State::Active(context) => {
                let mut handler = AdapterChangeHandler::new(context, clients_are_listening());
                let mut tree = context.tree.write().unwrap();
                tree.update_host_focus_state_and_process_changes(is_focused, &mut handler);
                Some(QueuedEvents(handler.finish()))
            }
        }
    }

    pub(crate) fn disconnect_providers(&mut self) -> Option<Vec<ComObject<PlatformNode>>> {
        match &self.state {
            State::Inactive { .. } => None,
            State::Placeholder(context) | State::Active(context) => {
                Some(context.take_platform_nodes())
            }
        }
    }

    /// Handle the `WM_GETOBJECT` window message. The accessibility tree
    /// is lazily initialized if necessary using the provided
    /// [`ActivationHandler`] implementation.
    ///
    /// This returns an `Option` so the caller can pass the message
    /// to `DefWindowProc` if AccessKit decides not to handle it.
    /// The optional value is an `Into<LRESULT>` rather than simply an `LRESULT`
    /// so the necessary call to UIA, which may lead to a nested `WM_GETOBJECT`
    /// message, can be done outside of any lock that the caller might hold
    /// on the `Adapter` or window state, while still abstracting away
    /// the details of that call to UIA.
    pub fn handle_wm_getobject<H: ActivationHandler + ?Sized>(
        &mut self,
        wparam: WPARAM,
        lparam: LPARAM,
        activation_handler: &mut H,
    ) -> Option<impl Into<LRESULT> + use<H>> {
        // Don't bother with MSAA object IDs that are asking for something other
        // than the client area of the window. DefWindowProc can handle those.
        // First, cast the lparam to i32, to handle inconsistent conversion
        // behavior in senders.
        let objid = normalize_objid(lparam);
        if objid < 0 && objid != UiaRootObjectId && objid != OBJID_CLIENT.0 {
            return None;
        }

        let (hwnd, platform_node) = match &self.state {
            State::Inactive {
                hwnd,
                is_window_focused,
                action_handler,
            } => match activation_handler.request_initial_tree() {
                Some(initial_state) => {
                    let hwnd = *hwnd;
                    let tree = Tree::new(initial_state, *is_window_focused);
                    let context = Context::new(hwnd, tree, Arc::clone(action_handler), false);
                    let node_id = context.read_tree().state().root_id();
                    let platform_node = context.get_or_create_platform_node(node_id);
                    self.state = State::Active(context);
                    (hwnd, platform_node)
                }
                None => {
                    let hwnd = *hwnd;
                    let placeholder_update = TreeUpdate {
                        nodes: vec![(PLACEHOLDER_ROOT_ID, Node::new(Role::Window))],
                        tree: Some(TreeInfo::new(PLACEHOLDER_ROOT_ID)),
                        tree_id: TreeId::ROOT,
                        focus: PLACEHOLDER_ROOT_ID,
                    };
                    let placeholder_tree = Tree::new(placeholder_update, *is_window_focused);
                    let context =
                        Context::new(hwnd, placeholder_tree, Arc::clone(action_handler), true);
                    let platform_node = PlatformNode::unspecified_root(&context);
                    self.state = State::Placeholder(context);
                    (hwnd, platform_node)
                }
            },
            State::Placeholder(context) => (context.hwnd, PlatformNode::unspecified_root(context)),
            State::Active(context) => {
                let node_id = context.read_tree().state().root_id();
                (context.hwnd, context.get_or_create_platform_node(node_id))
            }
        };
        let el: IRawElementProviderSimple = platform_node.into_interface();
        Some(WmGetObjectResult {
            hwnd,
            wparam,
            lparam,
            el,
        })
    }
}

fn init_uia() {
    // `UiaLookupId` is a cheap way of forcing UIA to initialize itself.
    unsafe {
        UiaLookupId(
            AutomationIdentifierType_Property,
            &ControlType_Property_GUID,
        )
    };
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn normalize_objid(lparam: LPARAM) -> i32 {
    (lparam.0 & 0xFFFFFFFF) as _
}
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn normalize_objid(lparam: LPARAM) -> i32 {
    lparam.0 as _
}

struct WmGetObjectResult {
    hwnd: WindowHandle,
    wparam: WPARAM,
    lparam: LPARAM,
    el: IRawElementProviderSimple,
}

impl From<WmGetObjectResult> for LRESULT {
    fn from(this: WmGetObjectResult) -> Self {
        unsafe { UiaReturnRawElementProvider(this.hwnd.0, this.wparam, this.lparam, &this.el) }
    }
}

/// Events generated by a tree update.
#[must_use = "events must be explicitly raised"]
pub struct QueuedEvents(Vec<QueuedEvent>);

impl QueuedEvents {
    /// Raise all queued events synchronously.
    ///
    /// The window may receive `WM_GETOBJECT` messages during this call.
    /// This means that any locks required by the `WM_GETOBJECT` handler
    /// must not be held when this method is called.
    ///
    /// This method should be called on the thread that owns the window.
    /// It's not clear whether this is a strict requirement of UIA itself,
    /// but based on the known behavior of UIA, MSAA, and some ATs,
    /// it's strongly recommended.
    pub fn raise(self) {
        for event in self.0 {
            match event {
                QueuedEvent::Simple { element, event_id } => {
                    let _ = unsafe { UiaRaiseAutomationEvent(&element, event_id) };
                }
                QueuedEvent::PropertyChanged {
                    element,
                    property_id,
                    old_value,
                    new_value,
                } => {
                    let _ = unsafe {
                        UiaRaiseAutomationPropertyChangedEvent(
                            &element,
                            property_id,
                            &old_value,
                            &new_value,
                        )
                    };
                }
            }
        }
    }
}

// We explicitly want to allow the queued events to be sent to the UI thread,
// so implement Send even though windows-rs doesn't implement it for all
// contained types. This is safe because we're not using COM threading.
unsafe impl Send for QueuedEvents {}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use accesskit::{ActionRequest, TextPosition, TextSelection};

    struct NullActionHandler;

    impl ActionHandler for NullActionHandler {
        fn do_action(&mut self, _request: ActionRequest) {}
    }

    fn make_context(initial: TreeUpdate, is_placeholder: bool) -> Arc<Context> {
        let tree = Tree::new(initial, true);
        Context::new(
            WindowHandle(HWND(std::ptr::null_mut())),
            tree,
            Arc::new(ActionHandlerWrapper::new(NullActionHandler)),
            is_placeholder,
        )
    }

    fn apply_update(
        context: &Arc<Context>,
        update: TreeUpdate,
        emit_events: bool,
    ) -> Vec<QueuedEvent> {
        let mut handler = AdapterChangeHandler::new(context, emit_events);
        let mut tree = context.tree.write().unwrap();
        tree.update_and_process_changes(update, &mut handler);
        handler.enqueue_selection_changes(&tree);
        handler.finish()
    }

    fn simple_event_ids(events: &[QueuedEvent]) -> Vec<UIA_EVENT_ID> {
        events
            .iter()
            .filter_map(|event| match event {
                QueuedEvent::Simple { event_id, .. } => Some(*event_id),
                QueuedEvent::PropertyChanged { .. } => None,
            })
            .collect()
    }

    fn root_update(children: Vec<NodeId>, extra_nodes: Vec<(NodeId, Node)>) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_children(children);
        let mut nodes = vec![(NodeId(0), root)];
        nodes.extend(extra_nodes);
        TreeUpdate {
            nodes,
            tree: Some(TreeInfo::new(NodeId(0))),
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }

    #[test]
    fn menu_and_tooltip_opened_closed_events_follow_lifecycle() {
        let context = make_context(root_update(vec![], vec![]), false);

        // Add visible Menu, MenuListPopup, and Tooltip.
        let events = apply_update(
            &context,
            TreeUpdate {
                nodes: vec![
                    {
                        let mut root = Node::new(Role::Window);
                        root.set_children(vec![NodeId(1), NodeId(2), NodeId(3)]);
                        (NodeId(0), root)
                    },
                    (NodeId(1), Node::new(Role::Menu)),
                    (NodeId(2), Node::new(Role::MenuListPopup)),
                    (NodeId(3), Node::new(Role::Tooltip)),
                ],
                tree: None,
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            true,
        );
        let ids = simple_event_ids(&events);
        assert_eq!(
            ids.iter()
                .filter(|id| **id == UIA_MenuOpenedEventId)
                .count(),
            2
        );
        assert_eq!(
            ids.iter()
                .filter(|id| **id == UIA_ToolTipOpenedEventId)
                .count(),
            1
        );

        // Hide Menu and Tooltip -> Closed events.
        let mut hidden_menu = Node::new(Role::Menu);
        hidden_menu.set_hidden();
        let mut hidden_tooltip = Node::new(Role::Tooltip);
        hidden_tooltip.set_hidden();
        let events = apply_update(
            &context,
            TreeUpdate {
                nodes: vec![(NodeId(1), hidden_menu), (NodeId(3), hidden_tooltip)],
                tree: None,
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            true,
        );
        let ids = simple_event_ids(&events);
        assert!(ids.contains(&UIA_MenuClosedEventId));
        assert!(ids.contains(&UIA_ToolTipClosedEventId));

        // Unhide Menu and Tooltip -> Opened events.
        let events = apply_update(
            &context,
            TreeUpdate {
                nodes: vec![
                    (NodeId(1), Node::new(Role::Menu)),
                    (NodeId(3), Node::new(Role::Tooltip)),
                ],
                tree: None,
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            true,
        );
        let ids = simple_event_ids(&events);
        assert!(ids.contains(&UIA_MenuOpenedEventId));
        assert!(ids.contains(&UIA_ToolTipOpenedEventId));

        // Remove all three -> Closed events.
        let events = apply_update(
            &context,
            TreeUpdate {
                nodes: vec![(NodeId(0), Node::new(Role::Window))],
                tree: None,
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            true,
        );
        let ids = simple_event_ids(&events);
        assert_eq!(
            ids.iter()
                .filter(|id| **id == UIA_MenuClosedEventId)
                .count(),
            2
        );
        assert_eq!(
            ids.iter()
                .filter(|id| **id == UIA_ToolTipClosedEventId)
                .count(),
            1
        );
    }

    #[test]
    fn text_changed_events_precede_selection_and_property_events() {
        let mut text_input = Node::new(Role::TextInput);
        text_input.set_children(vec![NodeId(2)]);
        text_input.set_value("hello");
        text_input.set_text_selection(TextSelection {
            anchor: TextPosition {
                node: NodeId(2),
                character_index: 0,
            },
            focus: TextPosition {
                node: NodeId(2),
                character_index: 0,
            },
        });
        let mut text_run = Node::new(Role::TextRun);
        text_run.set_value("hello");
        text_run.set_character_lengths([1, 1, 1, 1, 1]);

        let context = make_context(
            root_update(
                vec![NodeId(1)],
                vec![(NodeId(1), text_input), (NodeId(2), text_run)],
            ),
            false,
        );

        let mut updated_input = Node::new(Role::TextInput);
        updated_input.set_children(vec![NodeId(2)]);
        updated_input.set_value("world!");
        updated_input.set_text_selection(TextSelection {
            anchor: TextPosition {
                node: NodeId(2),
                character_index: 6,
            },
            focus: TextPosition {
                node: NodeId(2),
                character_index: 6,
            },
        });
        let mut updated_run = Node::new(Role::TextRun);
        updated_run.set_value("world!");
        updated_run.set_character_lengths([1, 1, 1, 1, 1, 1]);

        let events = apply_update(
            &context,
            TreeUpdate {
                nodes: vec![(NodeId(1), updated_input), (NodeId(2), updated_run)],
                tree: None,
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            true,
        );

        let text_changed_pos = events
            .iter()
            .position(|e| {
                matches!(
                    e,
                    QueuedEvent::Simple {
                        event_id,
                        ..
                    } if *event_id == UIA_Text_TextChangedEventId
                )
            })
            .expect("expected UIA_Text_TextChangedEventId");
        let selection_changed_pos = events
            .iter()
            .position(|e| {
                matches!(
                    e,
                    QueuedEvent::Simple {
                        event_id,
                        ..
                    } if *event_id == UIA_Text_TextSelectionChangedEventId
                )
            })
            .expect("expected UIA_Text_TextSelectionChangedEventId");
        assert!(text_changed_pos < selection_changed_pos);
    }

    #[test]
    fn no_listeners_skips_events_but_updates_tree_and_prunes_platform_nodes() {
        let mut button = Node::new(Role::Button);
        button.set_label("Initial");
        let context = make_context(
            root_update(vec![NodeId(1)], vec![(NodeId(1), button)]),
            false,
        );

        // Cache a PlatformNode for button NodeId(1).
        let button_full_id = context
            .read_tree()
            .state()
            .node_by_tree_local_id(NodeId(1), TreeId::ROOT)
            .unwrap()
            .id();
        let _cached = context.get_or_create_platform_node(button_full_id);
        assert_eq!(context.platform_node_count(), 1);

        // Update label with emit_events = false: tree updates, no events, no new platform nodes.
        let mut updated_button = Node::new(Role::Button);
        updated_button.set_label("Updated");
        let events = apply_update(
            &context,
            TreeUpdate {
                nodes: vec![(NodeId(1), updated_button)],
                tree: None,
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            false,
        );
        assert!(events.is_empty());
        assert_eq!(
            context
                .read_tree()
                .state()
                .node_by_tree_local_id(NodeId(1), TreeId::ROOT)
                .unwrap()
                .data()
                .label(),
            Some("Updated")
        );
        assert_eq!(context.platform_node_count(), 1);

        // Remove button with emit_events = false: removed node is still evicted from platform_nodes.
        let events = apply_update(
            &context,
            TreeUpdate {
                nodes: vec![(NodeId(0), Node::new(Role::Window))],
                tree: None,
                tree_id: TreeId::ROOT,
                focus: NodeId(0),
            },
            false,
        );
        assert!(events.is_empty());
        assert_eq!(context.platform_node_count(), 0);
    }

    #[test]
    fn placeholder_to_active_clears_stale_platform_nodes() {
        let placeholder_update = TreeUpdate {
            nodes: vec![(PLACEHOLDER_ROOT_ID, Node::new(Role::Window))],
            tree: Some(TreeInfo::new(PLACEHOLDER_ROOT_ID)),
            tree_id: TreeId::ROOT,
            focus: PLACEHOLDER_ROOT_ID,
        };
        let context = make_context(placeholder_update, true);
        let placeholder_root_id = context.read_tree().state().root_id();
        let _stale = context.get_or_create_platform_node(placeholder_root_id);
        assert_eq!(context.platform_node_count(), 1);

        let mut adapter = Adapter {
            state: State::Placeholder(Arc::clone(&context)),
        };
        let real_update = root_update(vec![NodeId(1)], vec![(NodeId(1), Node::new(Role::Button))]);
        let events = adapter.update_if_active_with_listening(|| real_update, false);
        assert!(events.is_none());
        assert!(!context.is_placeholder.load(Ordering::SeqCst));
        assert_eq!(context.platform_node_count(), 0);
        assert!(matches!(adapter.state, State::Active(_)));
    }
}
