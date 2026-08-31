//! AccessKit tree publication for the control slot.
//!
//! [`accesskit_android::InjectingAdapter`] injects an accessibility delegate
//! into the Activity's decor view through an embedded dex, so screen readers
//! walk the same Runtime tree as desktop hosts via [`AccessTreeProjector`].
//! Phase one publishes name/role/value only: reader actions are accepted and
//! logged, not driven back into Runtime.

use std::mem::ManuallyDrop;

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, TreeUpdate};
use accesskit_android::InjectingAdapter;
use accesskit_android::jni;
use accesskit_android::jni::objects::JObject;
use android_activity::AndroidApp;
use nana_ui::AccessTreeProjector;
use nana_ui::AccessibilityNode;

use crate::slot_runtime::SlotRuntime;

struct InitialTree(Option<TreeUpdate>);

impl ActivationHandler for InitialTree {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.0.take()
    }
}

/// Phase one: reader actions are queued by the platform adapter but the slot
/// does not map them onto Runtime mutations yet.
#[derive(Default)]
struct SlotActions;

impl ActionHandler for SlotActions {
    fn do_action(&mut self, request: ActionRequest) {
        log::debug!(
            "nana-android-host: slot a11y action not driven yet: {:?}",
            request.action
        );
    }
}

fn slot_accessibility_nodes(runtime: &SlotRuntime) -> Vec<AccessibilityNode> {
    let document = runtime.document();
    document
        .context()
        .world()
        .project_accessibility(document.document())
}

/// Owns the Android accessibility delegate for the control slot.
pub struct SlotAccessibility {
    adapter: InjectingAdapter,
    projector: AccessTreeProjector,
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
            AccessTreeProjector::new(slot_accessibility_nodes(runtime), true, runtime.scale());
        let initial = projector.full_update();
        let adapter =
            InjectingAdapter::new(&mut env, &decor, InitialTree(Some(initial)), SlotActions);
        Ok(Self { adapter, projector })
    }

    /// Publish the current slot tree. Cheap no-op while TalkBack has not
    /// initialized the tree.
    pub fn push(&mut self, runtime: &SlotRuntime) {
        let nodes = slot_accessibility_nodes(runtime);
        if let Some(update) = self.projector.synchronize_full(nodes, runtime.scale()) {
            self.adapter.update_if_active(|| update);
        }
    }
}
