//! AccessKit tree publication for the control slot.
//!
//! [`accesskit_android::InjectingAdapter`] injects an accessibility delegate
//! into the Activity's decor view through an embedded dex, so screen readers
//! walk the same Runtime tree as desktop hosts via [`AccessTreeProjector`].
//! The adapter publishes name/role/value and queues reader actions; the host
//! drains them through [`SlotRuntime::apply_accessibility_action`]. Scroll and
//! virtual-list coverage is a later phase.

use std::mem::ManuallyDrop;
use std::sync::{Arc, Mutex};

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, TreeUpdate};
use accesskit_android::InjectingAdapter;
use accesskit_android::jni;
use accesskit_android::jni::objects::JObject;
use android_activity::AndroidApp;
use nana_ui::AccessTreeProjector;

use crate::slot_runtime::SlotRuntime;

struct InitialTree(Option<TreeUpdate>);

impl ActivationHandler for InitialTree {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.0.take()
    }
}

/// Reader actions are queued by the platform adapter and drained by the host
/// before the next accessibility publication.
#[derive(Clone, Default)]
struct SlotActions {
    pending: Arc<Mutex<Vec<ActionRequest>>>,
}

impl ActionHandler for SlotActions {
    fn do_action(&mut self, request: ActionRequest) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.push(request);
        }
    }
}

/// Owns the Android accessibility delegate for the control slot.
pub struct SlotAccessibility {
    adapter: InjectingAdapter,
    projector: AccessTreeProjector,
    actions: SlotActions,
}

impl SlotAccessibility {
    /// Attach the accessibility delegate to the Activity's decor view and
    /// publish the slot tree as the initial (full) update.
    pub fn new(app: &AndroidApp, runtime: &SlotRuntime) -> Result<Self, String> {
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }
            .map_err(|error| format!("a11y jvm: {error}"))?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|error| format!("a11y attach: {error}"))?;
        // `activity_as_ptr` is a global ref; never drop it as a local ref.
        let activity = ManuallyDrop::new(unsafe {
            JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject)
        });
        let window = env
            .call_method(&*activity, "getWindow", "()Landroid/view/Window;", &[])
            .and_then(|value| value.l())
            .map_err(|error| format!("a11y window: {error}"))?;
        let decor = env
            .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])
            .and_then(|value| value.l())
            .map_err(|error| format!("a11y decor view: {error}"))?;

        let projector =
            AccessTreeProjector::new(runtime.accessibility_nodes(), true, runtime.scale());
        let initial = projector.full_update();
        let actions = SlotActions::default();
        let adapter = InjectingAdapter::new(
            &mut env,
            &decor,
            InitialTree(Some(initial)),
            actions.clone(),
        );
        Ok(Self {
            adapter,
            projector,
            actions,
        })
    }

    /// Publish the current slot tree. Cheap no-op while TalkBack has not
    /// initialized the tree.
    pub fn push(&mut self, runtime: &SlotRuntime) {
        let nodes = runtime.accessibility_nodes();
        if let Some(update) = self.projector.synchronize_full(nodes, runtime.scale()) {
            self.adapter.update_if_active(|| update);
        }
    }

    /// Drain TalkBack actions and apply them through the Runtime's typed
    /// accessibility contract. Invalid or unsupported requests are ignored.
    pub fn drain_actions(&mut self, runtime: &mut SlotRuntime) {
        let pending = self
            .actions
            .pending
            .lock()
            .map(|mut actions| std::mem::take(&mut *actions))
            .unwrap_or_default();
        for request in pending {
            let Some(request) = self.projector.project_action(request) else {
                continue;
            };
            let _ = runtime.apply_accessibility_action(request);
        }
    }
}
