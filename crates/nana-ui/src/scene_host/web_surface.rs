use super::*;
use crate::web_surface::{WebSurfaceNotice, WebSurfaceRequest};
use nana_window::{BrowserState, WebSurfaceCommand, WebSurfaceEvent};

pub(super) struct HostedWebSurface {
    surface: Option<nana_window::WebSurface>,
    policy: nana_window::BrowserPolicy,
    desc: nana_window::WebSurfaceDesc,
    frames: nana_window::WebFrameSink,
    revision: u64,
    /// Results the engine could not report itself (creation failed).
    events: Vec<nana_window::WebSurfaceCompletion>,
}

/// What the host does with one request against its current instance.
#[derive(Debug, PartialEq, Eq)]
enum Reconcile {
    Create,
    Keep { configure: bool, command: bool },
}

fn reconcile(hosted: Option<&HostedWebSurface>, request: &WebSurfaceRequest) -> Reconcile {
    let Some(hosted) = hosted else {
        return Reconcile::Create;
    };
    if hosted.policy != request.policy || !Arc::ptr_eq(&hosted.frames, &request.frames) {
        return Reconcile::Create;
    }
    Reconcile::Keep {
        configure: hosted.desc != request.desc,
        command: request.revision > hosted.revision,
    }
}

/// A new instance loads the requested page; it never replays window commands.
fn initial_command(request: &WebSurfaceRequest) -> Option<WebSurfaceCommand> {
    match &request.command {
        Some(WebSurfaceCommand::Navigate(url)) => Some(WebSurfaceCommand::Navigate(url.clone())),
        _ if request.restore_url.is_empty() => None,
        _ => Some(WebSurfaceCommand::Navigate(request.restore_url.clone())),
    }
}

impl<Program: RuntimeProgram> WindowManager<Program> {
    pub(super) fn sync_web_surfaces(&mut self) {
        if self.shutting_down {
            return;
        }
        let requests = self.program.web_surface_requests();
        self.web_surfaces
            .retain(|id, _| requests.iter().filter(|request| request.id == *id).count() == 1);
        for request in &requests {
            if request.id.is_empty()
                || requests
                    .iter()
                    .filter(|other| other.id == request.id)
                    .count()
                    != 1
            {
                continue;
            }
            match reconcile(self.web_surfaces.get(&request.id), request) {
                Reconcile::Create => {
                    // Release the old engine before its replacement starts.
                    self.web_surfaces.remove(&request.id);
                    let hosted = self.create_web_surface(request);
                    self.web_surfaces.insert(request.id.clone(), hosted);
                    self.proxy.wake_up();
                }
                Reconcile::Keep { configure, command } => {
                    let hosted = self
                        .web_surfaces
                        .get_mut(&request.id)
                        .expect("reconciled surface exists");
                    hosted.desc = request.desc;
                    let Some(surface) = &mut hosted.surface else {
                        continue;
                    };
                    if configure {
                        surface.configure(request.desc);
                    }
                    if command {
                        hosted.revision = request.revision;
                        if let Err(error) =
                            surface.command(request.revision, request.command.as_ref())
                        {
                            hosted.events.push(nana_window::WebSurfaceCompletion {
                                revision: request.revision,
                                event: WebSurfaceEvent::State(BrowserState {
                                    attached: true,
                                    error: Some(error),
                                    ..Default::default()
                                }),
                            });
                            self.proxy.wake_up();
                        }
                    }
                }
            }
        }
    }

    fn create_web_surface(&self, request: &WebSurfaceRequest) -> HostedWebSurface {
        let proxy = self.proxy.clone();
        let created = nana_window::WebSurface::new(
            request.policy.clone(),
            request.desc,
            request.frames.clone(),
            Arc::new(move || proxy.wake_up()),
        );
        let mut events = Vec::new();
        let surface = match created {
            Ok(mut surface) => {
                if let Err(error) =
                    surface.command(request.revision, initial_command(request).as_ref())
                {
                    events.push(nana_window::WebSurfaceCompletion {
                        revision: request.revision,
                        event: WebSurfaceEvent::State(BrowserState {
                            attached: true,
                            error: Some(error),
                            ..Default::default()
                        }),
                    });
                }
                Some(surface)
            }
            Err(error) => {
                events.push(nana_window::WebSurfaceCompletion {
                    revision: request.revision,
                    event: WebSurfaceEvent::State(BrowserState {
                        error: Some(error),
                        ..Default::default()
                    }),
                });
                None
            }
        };
        HostedWebSurface {
            surface,
            policy: request.policy.clone(),
            desc: request.desc,
            frames: request.frames.clone(),
            revision: request.revision,
            events,
        }
    }

    pub(super) fn drain_web_surface_events(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.sync_web_surfaces();
        let mut notices = Vec::new();
        for (id, hosted) in &mut self.web_surfaces {
            let mut completed = std::mem::take(&mut hosted.events);
            if let Some(surface) = &mut hosted.surface {
                completed.extend(surface.take_events());
            }
            notices.extend(completed.into_iter().map(|completion| WebSurfaceNotice {
                id: id.clone(),
                revision: completion.revision,
                event: completion.event,
            }));
        }
        for notice in notices {
            if self.shutting_down || event_loop.exiting() {
                break;
            }
            // A surface released since its event was queued says nothing more.
            if !self.web_surfaces.contains_key(&notice.id) {
                continue;
            }
            let update = self.program.web_surface_event(notice, &self.context());
            self.apply_update(event_loop, update, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(revision: u64, frames: &nana_window::WebFrameSink) -> WebSurfaceRequest {
        WebSurfaceRequest {
            id: "page".into(),
            policy: nana_window::BrowserPolicy { allow_web: true },
            desc: nana_window::WebSurfaceDesc::default(),
            restore_url: "https://example.com/".into(),
            revision,
            command: None,
            frames: frames.clone(),
        }
    }

    fn hosted(revision: u64, frames: &nana_window::WebFrameSink) -> HostedWebSurface {
        HostedWebSurface {
            surface: None,
            policy: nana_window::BrowserPolicy { allow_web: true },
            desc: nana_window::WebSurfaceDesc::default(),
            frames: frames.clone(),
            revision,
            events: Vec::new(),
        }
    }

    #[test]
    fn commands_run_once_and_settings_apply_in_place() {
        let frames: nana_window::WebFrameSink = Arc::new(|_| {});
        let current = hosted(3, &frames);
        assert_eq!(
            reconcile(Some(&current), &request(3, &frames)),
            Reconcile::Keep {
                configure: false,
                command: false
            }
        );
        let mut resized = request(4, &frames);
        resized.desc.size = [640, 480];
        assert_eq!(
            reconcile(Some(&current), &resized),
            Reconcile::Keep {
                configure: true,
                command: true
            }
        );
        assert_eq!(reconcile(None, &resized), Reconcile::Create);
    }

    #[test]
    fn a_new_sink_or_policy_recreates_the_engine() {
        let frames: nana_window::WebFrameSink = Arc::new(|_| {});
        let current = hosted(1, &frames);
        let other: nana_window::WebFrameSink = Arc::new(|_| {});
        assert_eq!(
            reconcile(Some(&current), &request(1, &other)),
            Reconcile::Create
        );
        let mut closed = request(1, &frames);
        closed.policy.allow_web = false;
        assert_eq!(reconcile(Some(&current), &closed), Reconcile::Create);
    }

    #[test]
    fn new_instances_load_the_page_but_never_replay_window_commands() {
        let frames: nana_window::WebFrameSink = Arc::new(|_| {});
        let mut shown = request(2, &frames);
        shown.command = Some(WebSurfaceCommand::ShowWindow {
            title: "Page".into(),
        });
        assert_eq!(
            initial_command(&shown),
            Some(WebSurfaceCommand::Navigate("https://example.com/".into()))
        );
        shown.command = Some(WebSurfaceCommand::Navigate("https://example.org/".into()));
        assert_eq!(
            initial_command(&shown),
            Some(WebSurfaceCommand::Navigate("https://example.org/".into()))
        );
        shown.command = None;
        shown.restore_url.clear();
        assert_eq!(initial_command(&shown), None);
    }
}
