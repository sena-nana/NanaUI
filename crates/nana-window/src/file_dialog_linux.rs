//! Cancellable portal requests, with the same zenity fallback previously
//! supplied by application rfd consumers. Only the worker touches D-Bus.
use super::*;
use futures_lite::{StreamExt, future};
use std::{collections::HashMap, path::PathBuf};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

pub(super) fn open<W>(
    window: Arc<W>,
    request: FileDialogRequest,
    completion: impl FnOnce(FileDialogResult) + Send + 'static,
) -> Result<FileDialogHandle, FileDialogError>
where
    W: HasWindowHandle + HasDisplayHandle + Send + Sync + ?Sized + 'static,
{
    let (cancel, cancelled) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("nana-file-dialog".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                future::block_on(async {
                    // Export the Wayland parent for this request's lifetime. X11 uses
                    // its native window identifier. Keep the owner alive throughout.
                    let raw = window.window_handle().ok().map(|handle| handle.as_raw());
                    let display = window.display_handle().ok().map(|handle| handle.as_raw());
                    let parent = match raw {
                        Some(raw) => match until_cancel(
                            ashpd::WindowIdentifier::from_raw_handle(&raw, display.as_ref()),
                            &cancelled,
                        )
                        .await
                        {
                            Some(parent) => parent,
                            None => return Ok(Vec::new()),
                        },
                        None => return Err(FileDialogError::WindowClosed),
                    };
                    if cancelled.try_recv().is_ok() {
                        return Ok(Vec::new());
                    }
                    match portal(
                        &request,
                        parent.as_ref().map(ToString::to_string).unwrap_or_default(),
                        &cancelled,
                    )
                    .await
                    {
                        Some(result) => result,
                        None => zenity(&request, &cancelled).await,
                    }
                })
            }))
            .unwrap_or_else(|_| Err(platform("file dialog backend panicked")));
            completion(match result {
                Ok(paths) => FileDialogResult::selected(request.id, paths),
                Err(error) => FileDialogResult::failed(request.id, error),
            });
        })
        .map(|_| {
            FileDialogHandle::new(move || {
                let _ = cancel.try_send(());
            })
        })
        .map_err(|error| FileDialogError::Platform(error.to_string()))
}

async fn until_cancel<F: std::future::Future>(
    work: F,
    cancel: &async_channel::Receiver<()>,
) -> Option<F::Output> {
    future::race(async { Some(work.await) }, async {
        let _ = cancel.recv().await;
        None
    })
    .await
}

fn platform(error: impl std::fmt::Display) -> FileDialogError {
    FileDialogError::Platform(error.to_string())
}

// None means no portal accepted the request; only that permits fallback.
async fn portal(
    request: &FileDialogRequest,
    parent: String,
    cancel: &async_channel::Receiver<()>,
) -> Option<Result<Vec<PathBuf>, FileDialogError>> {
    let Some(connection) = until_cancel(zbus::Connection::session(), cancel).await else {
        return Some(Ok(Vec::new()));
    };
    let connection = connection.ok()?;
    let chooser = match until_cancel(
        zbus::Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.FileChooser",
        ),
        cancel,
    )
    .await
    {
        Some(result) => result.ok()?,
        None => return Some(Ok(Vec::new())),
    };
    let token = format!("nana_{:032x}", rand::random::<u128>());
    // Subscribe before Open/Save, without a predicted object path. Old portal
    // versions may ignore handle_token and return a different request path;
    // their early Response must remain queued until that path is known.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.portal.Desktop")
        .ok()?
        .interface("org.freedesktop.portal.Request")
        .ok()?
        .member("Response")
        .ok()?
        .build();
    let mut responses = match until_cancel(
        zbus::MessageStream::for_match_rule(rule, &connection, Some(8)),
        cancel,
    )
    .await
    {
        Some(result) => result.ok()?,
        None => return Some(Ok(Vec::new())),
    };
    let mut options = HashMap::<&str, Value<'_>>::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("modal", Value::from(true));
    if !request.kind.is_save() {
        options.insert("multiple", Value::from(request.kind.is_multiple()));
        options.insert(
            "directory",
            Value::from(matches!(
                request.kind,
                FileDialogKind::PickFolder | FileDialogKind::PickFolders
            )),
        );
    }
    if let Some(directory) = &request.directory {
        use std::os::unix::ffi::OsStrExt;
        let mut bytes = directory.as_os_str().as_bytes().to_vec();
        bytes.push(0);
        options.insert("current_folder", Value::new(bytes));
    }
    if let Some(name) = &request.file_name {
        options.insert("current_name", Value::from(name.as_ref()));
    }
    if !request.filters.is_empty() {
        let filters: Vec<(String, Vec<(u32, String)>)> = request
            .filters
            .iter()
            .map(|filter| {
                (
                    filter.name.to_string(),
                    filter
                        .extensions
                        .iter()
                        .map(|ext| {
                            (
                                0,
                                if ext.as_ref() == "*" || ext.is_empty() {
                                    "*".into()
                                } else {
                                    format!("*.{ext}")
                                },
                            )
                        })
                        .collect(),
                )
            })
            .collect();
        options.insert("filters", Value::new(filters));
    }
    if cancel.try_recv().is_ok() {
        return Some(Ok(Vec::new()));
    }
    let method = if request.kind.is_save() {
        "SaveFile"
    } else {
        "OpenFile"
    };
    let arguments = (parent, request.title.as_deref().unwrap_or(""), options);
    let open = chooser.call::<_, _, OwnedObjectPath>(method, &arguments);
    let opened = match until_cancel(open, cancel).await {
        Some(result) => result.ok()?,
        None => {
            // The accepted path may not have returned yet. Closing this
            // request-only bus connection revokes its portal ownership even
            // if OpenFile was still being processed. No fallback may open.
            let _ = connection.clone().close().await;
            return Some(Ok(Vec::new()));
        }
    };
    enum Outcome {
        Response(Option<zbus::Result<zbus::Message>>),
        Cancel,
    }
    let outcome = future::race(
        async {
            loop {
                match responses.next().await {
                    Some(Ok(message))
                        if message.header().path().map(|path| path.as_str())
                            == Some(opened.as_str()) =>
                    {
                        break Outcome::Response(Some(Ok(message)));
                    }
                    Some(Ok(_)) => continue,
                    other => break Outcome::Response(other),
                }
            }
        },
        async {
            let _ = cancel.recv().await;
            Outcome::Cancel
        },
    )
    .await;
    match outcome {
        Outcome::Cancel => {
            let result = connection
                .call_method(
                    Some("org.freedesktop.portal.Desktop"),
                    opened.as_str(),
                    Some("org.freedesktop.portal.Request"),
                    "Close",
                    &(),
                )
                .await;
            Some(result.map(|_| Vec::new()).map_err(platform))
        }
        Outcome::Response(None) => Some(Err(platform("portal response stream closed"))),
        Outcome::Response(Some(Err(error))) => Some(Err(platform(error))),
        Outcome::Response(Some(Ok(message))) => Some((|| {
            let (status, mut values): (u32, HashMap<String, OwnedValue>) =
                message.body().deserialize().map_err(platform)?;
            if status == 1 {
                return Ok(Vec::new());
            }
            if status != 0 {
                return Err(platform(format!(
                    "portal rejected file selection ({status})"
                )));
            }
            let uris = Vec::<String>::try_from(
                values
                    .remove("uris")
                    .ok_or_else(|| platform("portal omitted selected paths"))?,
            )
            .map_err(platform)?;
            uris.into_iter()
                .map(|uri| {
                    url::Url::parse(&uri)
                        .map_err(platform)?
                        .to_file_path()
                        .map_err(|_| platform("portal returned a non-file URI"))
                })
                .collect()
        })()),
    }
}

async fn zenity(
    request: &FileDialogRequest,
    cancel: &async_channel::Receiver<()>,
) -> Result<Vec<PathBuf>, FileDialogError> {
    use async_process::Command;
    use std::process::Stdio;
    if cancel.try_recv().is_ok() {
        return Ok(Vec::new());
    }
    let mut command = Command::new("zenity");
    command.args(["--file-selection", "--no-markup"]);
    if request.kind.is_save() {
        command.args(["--save", "--confirm-overwrite"]);
    }
    if matches!(
        request.kind,
        FileDialogKind::PickFolder | FileDialogKind::PickFolders
    ) {
        command.arg("--directory");
    }
    if request.kind.is_multiple() {
        command.args(["--multiple", "--separator=\n"]);
    }
    if let Some(title) = &request.title {
        command.arg("--title").arg(title.as_ref());
    }
    let mut filename = request.directory.clone().unwrap_or_default();
    if let Some(name) = &request.file_name {
        filename.push(name.as_ref());
    } else if request.directory.is_some() {
        filename.push("");
    }
    if !filename.as_os_str().is_empty() {
        command.arg("--filename").arg(filename);
    }
    for filter in &request.filters {
        let globs = filter
            .extensions
            .iter()
            .map(|ext| {
                if ext.as_ref() == "*" {
                    "*".into()
                } else {
                    format!("*.{ext}")
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        command
            .arg("--file-filter")
            .arg(format!("{} | {globs}", filter.name));
    }
    let mut child = command
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(platform)?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| platform("file chooser output unavailable"))?;
    enum Outcome {
        Exit(std::io::Result<(std::process::ExitStatus, String)>),
        Cancel,
    }
    let outcome = future::race(
        async {
            let (status, output) = future::zip(child.status(), async {
                let mut output = String::new();
                futures_lite::io::AsyncReadExt::read_to_string(&mut stdout, &mut output).await?;
                Ok::<_, std::io::Error>(output)
            })
            .await;
            Outcome::Exit(status.and_then(|status| output.map(|output| (status, output))))
        },
        async {
            let _ = cancel.recv().await;
            Outcome::Cancel
        },
    )
    .await;
    match outcome {
        Outcome::Cancel => {
            let _ = child.kill();
            let _ = child.status().await;
            Ok(Vec::new())
        }
        Outcome::Exit(status) => {
            let (status, output) = status.map_err(platform)?;
            if status.code() == Some(1) {
                return Ok(Vec::new());
            }
            if !status.success() {
                return Err(platform(format!("file chooser exited with {status}")));
            }
            let output = output.strip_suffix('\n').unwrap_or(&output);
            Ok(if request.kind.is_multiple() {
                output
                    .split('\n')
                    .filter(|path| !path.is_empty())
                    .map(PathBuf::from)
                    .collect()
            } else if output.is_empty() {
                Vec::new()
            } else {
                vec![output.into()]
            })
        }
    }
}
