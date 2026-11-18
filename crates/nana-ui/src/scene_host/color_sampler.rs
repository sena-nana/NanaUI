use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nana_ui_platform::WindowId;
use nana_ui_runtime::StableNodeId;
use nana_window::{
    ColorSamplerError, ColorSamplerHandle, ColorSamplerResult, color_sampler_support,
    pick_screen_color,
};
use winit::event_loop::ActiveEventLoop;

use super::WindowManager;
use crate::RuntimeProgram;
use crate::runtime_host::HostDocumentAccess;

struct WinitWindowHandle<'a>(&'a dyn winit::window::Window);

impl raw_window_handle::HasWindowHandle for WinitWindowHandle<'_> {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        self.0.window_handle()
    }
}

type CompletedSamplers = Arc<Mutex<Vec<(WindowId, u64, ColorSamplerResult)>>>;

pub(super) struct ColorSamplers {
    next_token: u64,
    active: HashMap<WindowId, (u64, StableNodeId)>,
    native: HashMap<WindowId, ColorSamplerHandle>,
    completed: CompletedSamplers,
    shutting_down: bool,
}

impl Default for ColorSamplers {
    fn default() -> Self {
        Self {
            next_token: 1,
            active: HashMap::new(),
            native: HashMap::new(),
            completed: Arc::new(Mutex::new(Vec::new())),
            shutting_down: false,
        }
    }
}

struct BeginSampler {
    token: u64,
    replaced_node: Option<StableNodeId>,
    replaced_handle: Option<ColorSamplerHandle>,
}

impl ColorSamplers {
    fn begin(&mut self, window: WindowId, node: StableNodeId) -> Option<BeginSampler> {
        if self.shutting_down {
            return None;
        }
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1).max(1);
        let replaced_node = self.active.insert(window, (token, node)).map(|(_, n)| n);
        let replaced_handle = self.native.remove(&window);
        Some(BeginSampler {
            token,
            replaced_node,
            replaced_handle,
        })
    }

    fn finish(
        &mut self,
        window: WindowId,
        token: u64,
        result: ColorSamplerResult,
    ) -> Option<(StableNodeId, Option<[f32; 3]>)> {
        if self.active.get(&window).map(|(t, _)| *t) != Some(token) {
            return None;
        }
        let (_, node) = self.active.remove(&window)?;
        self.native.remove(&window);
        let picked = match result {
            ColorSamplerResult::Picked { rgba, .. } => Some([rgba[0], rgba[1], rgba[2]]),
            ColorSamplerResult::Cancelled { .. } | ColorSamplerResult::Failed { .. } => None,
        };
        Some((node, picked))
    }

    fn close(&mut self, window: WindowId) -> Option<StableNodeId> {
        self.native.remove(&window);
        self.active.remove(&window).map(|(_, node)| node)
    }
}

impl<Program: RuntimeProgram> WindowManager<Program> {
    pub(super) fn sync_document_eyedropper(&mut self, id: WindowId) {
        let supported = color_sampler_support().is_supported();
        let _ = self.program.write_document(id, |document| {
            document.context_mut().set_eyedropper_supported(supported);
        });
    }

    pub(super) fn drain_color_pick_requests(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
    ) {
        if !self.color_samplers.shutting_down
            && let Some(&(_, active_node)) = self.color_samplers.active.get(&id)
        {
            let still_active = self
                .program
                .read_document(id, |document| {
                    document.context().color_pick_active(active_node)
                })
                .unwrap_or(false);
            if !still_active {
                self.close_color_sampler(event_loop, id);
            }
        }
        let Some(mut requests) = self.program.write_document(id, |document| {
            document.context_mut().take_color_pick_requests()
        }) else {
            return;
        };
        let Some(node) = requests.pop() else {
            return;
        };
        for superseded in requests {
            self.apply_color_pick_result(event_loop, id, superseded, None);
        }

        if self.color_samplers.shutting_down {
            self.apply_color_pick_result(event_loop, id, node, None);
            return;
        }
        if !color_sampler_support().is_supported() {
            nana_diagnostics::event!(
                nana_diagnostics::framework::window::COLOR_SAMPLER_FAILED,
                window = id.0,
                unsupported = true
            );
            self.apply_color_pick_result_with_support(event_loop, id, node, None, false);
            return;
        }
        let Some(window) = self.window(id).cloned() else {
            self.apply_color_pick_result(event_loop, id, node, None);
            return;
        };
        let Some(begin) = self.color_samplers.begin(id, node) else {
            self.apply_color_pick_result(event_loop, id, node, None);
            return;
        };

        drop(begin.replaced_handle);
        if let Some(previous_node) = begin.replaced_node
            && previous_node != node
        {
            self.apply_color_pick_result(event_loop, id, previous_node, None);
        }

        let token = begin.token;
        let completed = Arc::clone(&self.color_samplers.completed);
        let proxy = self.proxy.clone();
        let owner = WinitWindowHandle(window.as_ref());
        match pick_screen_color(Some(&owner), token, move |result| {
            if let Ok(mut queue) = completed.lock() {
                queue.push((id, token, result));
            }
            proxy.wake_up();
        }) {
            Ok(handle) => {
                self.color_samplers.native.insert(id, handle);
            }
            Err(error) => {
                let unsupported = matches!(error, ColorSamplerError::Unsupported);
                nana_diagnostics::event!(
                    nana_diagnostics::framework::window::COLOR_SAMPLER_FAILED,
                    window = id.0,
                    unsupported = unsupported
                );
                if let Some((failed_node, _)) =
                    self.color_samplers
                        .finish(id, token, ColorSamplerResult::failed(token, error))
                {
                    self.apply_color_pick_result_with_support(
                        event_loop,
                        id,
                        failed_node,
                        None,
                        !unsupported,
                    );
                }
            }
        }
    }

    pub(super) fn drain_all_color_pick_requests(&mut self, event_loop: &dyn ActiveEventLoop) {
        let ids: Vec<_> = self.window_contexts.keys().copied().collect();
        for id in ids {
            if event_loop.exiting() {
                return;
            }
            self.drain_color_pick_requests(event_loop, id);
        }
    }

    pub(super) fn complete_color_samplers(&mut self, event_loop: &dyn ActiveEventLoop) {
        let completed = {
            let Ok(mut guard) = self.color_samplers.completed.lock() else {
                return;
            };
            std::mem::take(&mut *guard)
        };
        for (id, token, result) in completed {
            let failed_unsupported = match &result {
                ColorSamplerResult::Failed { error, .. } => {
                    Some(matches!(error, ColorSamplerError::Unsupported))
                }
                _ => None,
            };
            if let Some((node, picked)) = self.color_samplers.finish(id, token, result) {
                if let Some(unsupported) = failed_unsupported {
                    nana_diagnostics::event!(
                        nana_diagnostics::framework::window::COLOR_SAMPLER_FAILED,
                        window = id.0,
                        unsupported = unsupported
                    );
                    self.apply_color_pick_result_with_support(
                        event_loop,
                        id,
                        node,
                        picked,
                        !unsupported,
                    );
                } else {
                    self.apply_color_pick_result(event_loop, id, node, picked);
                }
            }
        }
    }

    pub(super) fn close_color_sampler(&mut self, event_loop: &dyn ActiveEventLoop, id: WindowId) {
        if let Some(node) = self.color_samplers.close(id) {
            self.apply_color_pick_result(event_loop, id, node, None);
        }
    }

    pub(super) fn close_all_color_samplers(&mut self) {
        self.color_samplers.shutting_down = true;
        let ids: Vec<_> = self.color_samplers.active.keys().copied().collect();
        for id in ids {
            self.color_samplers.close(id);
        }
    }

    fn apply_color_pick_result(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        node: StableNodeId,
        rgb: Option<[f32; 3]>,
    ) {
        self.apply_color_pick_result_with_support(event_loop, id, node, rgb, true);
    }

    fn apply_color_pick_result_with_support(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        id: WindowId,
        node: StableNodeId,
        rgb: Option<[f32; 3]>,
        supported: bool,
    ) {
        let updated = self
            .program
            .write_document(id, |document| {
                let context = document.context_mut();
                let mut changed = context.complete_color_pick(node, rgb);
                if !supported && context.world().eyedropper_supported() {
                    context.set_eyedropper_supported(false);
                    changed = true;
                }
                changed
            })
            .unwrap_or(false);
        if !updated || event_loop.exiting() {
            return;
        }
        self.bind_after_present.insert(id);
        let update = self.drain_program_messages(id);
        self.apply_update(event_loop, update, None);
        self.request_redraw(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u64) -> StableNodeId {
        StableNodeId::new(id).expect("non-zero test node id")
    }

    #[test]
    fn newer_request_replaces_active_sampler_and_rejects_stale_token() {
        let mut samplers = ColorSamplers::default();
        let win = WindowId(1);
        let first = samplers.begin(win, node(10)).unwrap();
        let second = samplers.begin(win, node(20)).unwrap();
        assert_eq!(second.replaced_node, Some(node(10)));

        // Stale completion for first token is ignored.
        assert!(
            samplers
                .finish(
                    win,
                    first.token,
                    ColorSamplerResult::picked(first.token, [1.0, 0.0, 0.0])
                )
                .is_none()
        );
        // Second token completes once and clears active state.
        let done = samplers
            .finish(
                win,
                second.token,
                ColorSamplerResult::picked(second.token, [0.2, 0.4, 0.6]),
            )
            .unwrap();
        assert_eq!(done.0, node(20));
        assert_eq!(done.1, Some([0.2, 0.4, 0.6]));
        assert!(samplers.active.is_empty());
    }

    #[test]
    fn closing_and_reopening_window_rejects_late_callback() {
        let mut samplers = ColorSamplers::default();
        let win = WindowId(1);
        let first = samplers.begin(win, node(10)).unwrap();
        assert_eq!(samplers.close(win), Some(node(10)));
        assert_eq!(samplers.close(win), None);

        let second = samplers.begin(win, node(10)).unwrap();
        assert!(
            samplers
                .finish(
                    win,
                    first.token,
                    ColorSamplerResult::picked(first.token, [1.0, 1.0, 1.0])
                )
                .is_none()
        );
        let cancelled = samplers
            .finish(
                win,
                second.token,
                ColorSamplerResult::cancelled(second.token),
            )
            .unwrap();
        assert_eq!(cancelled, (node(10), None));
    }

    #[test]
    fn shutdown_rejects_new_samplers_and_queues_are_host_local() {
        let mut first = ColorSamplers::default();
        let second = ColorSamplers::default();
        let b = first.begin(WindowId(1), node(5)).unwrap();
        first.completed.lock().unwrap().push((
            WindowId(1),
            b.token,
            ColorSamplerResult::cancelled(b.token),
        ));
        assert!(second.completed.lock().unwrap().is_empty());

        first.shutting_down = true;
        assert!(first.begin(WindowId(1), node(6)).is_none());
    }

    #[test]
    fn failed_completion_clears_active_sampler_and_yields_none_color() {
        let mut samplers = ColorSamplers::default();
        let win = WindowId(2);
        let started = samplers.begin(win, node(42)).unwrap();
        let done = samplers
            .finish(
                win,
                started.token,
                ColorSamplerResult::failed(started.token, ColorSamplerError::Unsupported),
            )
            .unwrap();
        assert_eq!(done, (node(42), None));
        assert!(samplers.active.is_empty());
        assert!(samplers.native.is_empty());
    }
}
