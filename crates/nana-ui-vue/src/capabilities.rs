//! Rust-owned capability / permission gate for sensitive host operations.
//!
//! Vue UI may request workspace switch, secrets, or privileged transport calls only
//! through registered host APIs. Grants live in Rust; changing UI appearance or
//! patching JS cannot enlarge the allow-list.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use nana_js_engine::{HostApiRegistry, HostValue, JsException};

/// Named capability checked before a sensitive host op runs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Capability(pub String);

impl Capability {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub const WORKSPACE_READ: &'static str = "workspace.read";
    pub const WORKSPACE_SWITCH: &'static str = "workspace.switch";
    pub const WORKSPACE_SETTINGS_WRITE: &'static str = "workspace.settings.write";
    pub const SECRET_READ: &'static str = "secret.read";
    pub const GITHUB_TOKEN: &'static str = "github.token";
}

/// Deny-by-default permission policy held on the Rust side.
#[derive(Debug, Clone, Default)]
pub struct PermissionPolicy {
    granted: BTreeSet<String>,
}

impl PermissionPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Dev / evidence helper: grant the common read-only workspace surface.
    pub fn with_workspace_read() -> Self {
        let mut p = Self::new();
        p.grant(Capability::WORKSPACE_READ);
        p
    }

    pub fn grant(&mut self, capability: impl AsRef<str>) {
        self.granted.insert(capability.as_ref().to_string());
    }

    pub fn revoke(&mut self, capability: impl AsRef<str>) {
        self.granted.remove(capability.as_ref());
    }

    pub fn is_granted(&self, capability: impl AsRef<str>) -> bool {
        self.granted.contains(capability.as_ref())
    }

    pub fn require(&self, capability: impl AsRef<str>) -> Result<(), JsException> {
        let name = capability.as_ref();
        if self.is_granted(name) {
            Ok(())
        } else {
            Err(JsException::new(format!(
                "permission denied: capability `{name}` is not granted (Rust host gate)"
            )))
        }
    }
}

/// Shared policy handle installed into [`HostApiRegistry`].
pub type SharedPermissionPolicy = Arc<Mutex<PermissionPolicy>>;

pub fn shared_permission_policy(policy: PermissionPolicy) -> SharedPermissionPolicy {
    Arc::new(Mutex::new(policy))
}

/// In-memory workspace bootstrap owned by Rust (not by JS mock transport).
#[derive(Debug, Clone)]
pub struct WorkspaceBootstrap {
    pub active_workspace_id: String,
    pub workspaces: Vec<WorkspaceRecord>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
}

impl Default for WorkspaceBootstrap {
    fn default() -> Self {
        Self {
            active_workspace_id: "ws-demo".into(),
            workspaces: vec![WorkspaceRecord {
                id: "ws-demo".into(),
                name: "demo-workspace".into(),
            }],
        }
    }
}

impl WorkspaceBootstrap {
    fn to_host_value(&self) -> HostValue {
        let workspaces = self
            .workspaces
            .iter()
            .map(|w| {
                HostValue::Object(
                    [
                        ("id".into(), HostValue::string(&w.id)),
                        ("name".into(), HostValue::string(&w.name)),
                        (
                            "roots".into(),
                            HostValue::Array(vec![HostValue::Object(
                                [
                                    ("id".into(), HostValue::string("root-1")),
                                    ("path".into(), HostValue::string("/tmp/demo")),
                                    ("primary".into(), HostValue::Bool(true)),
                                ]
                                .into_iter()
                                .collect(),
                            )]),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                )
            })
            .collect();
        HostValue::Object(
            [
                (
                    "activeWorkspaceId".into(),
                    HostValue::string(&self.active_workspace_id),
                ),
                ("workspaces".into(), HostValue::Array(workspaces)),
                ("githubBound".into(), HostValue::Bool(false)),
            ]
            .into_iter()
            .collect(),
        )
    }
}

/// Register capability-gated workspace / secret host ops.
///
/// JS must call these instead of mutating privileged state locally. Denied calls
/// raise a JS exception — UI theming cannot bypass the gate.
pub fn register_capability_host_ops(
    api: &mut HostApiRegistry,
    policy: SharedPermissionPolicy,
    workspace: Arc<Mutex<WorkspaceBootstrap>>,
) {
    {
        let policy = Arc::clone(&policy);
        let workspace = Arc::clone(&workspace);
        api.register("workspaceGetBootstrap", move |_args| {
            policy
                .lock()
                .map_err(|_| JsException::new("permission policy poisoned"))?
                .require(Capability::WORKSPACE_READ)?;
            let guard = workspace
                .lock()
                .map_err(|_| JsException::new("workspace state poisoned"))?;
            Ok(guard.to_host_value())
        });
    }
    {
        let policy = Arc::clone(&policy);
        let workspace = Arc::clone(&workspace);
        api.register("workspaceSwitch", move |args| {
            policy
                .lock()
                .map_err(|_| JsException::new("permission policy poisoned"))?
                .require(Capability::WORKSPACE_SWITCH)?;
            let id = args
                .first()
                .and_then(HostValue::as_str)
                .unwrap_or("ws-demo")
                .to_string();
            let mut guard = workspace
                .lock()
                .map_err(|_| JsException::new("workspace state poisoned"))?;
            if !guard.workspaces.iter().any(|w| w.id == id) {
                return Err(JsException::new(format!("unknown workspace `{id}`")));
            }
            guard.active_workspace_id = id;
            Ok(guard.to_host_value())
        });
    }
    {
        let policy = Arc::clone(&policy);
        api.register("secretGet", move |args| {
            policy
                .lock()
                .map_err(|_| JsException::new("permission policy poisoned"))?
                .require(Capability::SECRET_READ)?;
            let key = args.first().and_then(HostValue::as_str).unwrap_or_default();
            // Never return real secrets from the default gate — only an authorized stub.
            Ok(HostValue::Object(
                [
                    ("key".into(), HostValue::string(key)),
                    ("present".into(), HostValue::Bool(false)),
                    ("value".into(), HostValue::Null),
                ]
                .into_iter()
                .collect(),
            ))
        });
    }
    {
        let policy = Arc::clone(&policy);
        api.register("capabilityGranted", move |args| {
            let name = args.first().and_then(HostValue::as_str).unwrap_or_default();
            let granted = policy
                .lock()
                .map_err(|_| JsException::new("permission policy poisoned"))?
                .is_granted(name);
            Ok(HostValue::Bool(granted))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_js_engine::HostApiRegistry;

    #[test]
    fn workspace_switch_denied_without_grant() {
        let policy = shared_permission_policy(PermissionPolicy::with_workspace_read());
        let workspace = Arc::new(Mutex::new(WorkspaceBootstrap::default()));
        let mut api = HostApiRegistry::new();
        register_capability_host_ops(&mut api, policy, Arc::clone(&workspace));

        let boot = api.call("workspaceGetBootstrap", &[]).expect("read ok");
        assert!(boot.as_object().is_some());

        let err = api
            .call("workspaceSwitch", &[HostValue::string("ws-demo")])
            .expect_err("switch must be denied");
        assert!(
            err.message.contains("permission denied"),
            "unexpected: {err:?}"
        );
    }

    #[test]
    fn workspace_switch_allowed_when_granted() {
        let mut policy = PermissionPolicy::with_workspace_read();
        policy.grant(Capability::WORKSPACE_SWITCH);
        let policy = shared_permission_policy(policy);
        let workspace = Arc::new(Mutex::new(WorkspaceBootstrap {
            active_workspace_id: "ws-demo".into(),
            workspaces: vec![
                WorkspaceRecord {
                    id: "ws-demo".into(),
                    name: "demo".into(),
                },
                WorkspaceRecord {
                    id: "ws-other".into(),
                    name: "other".into(),
                },
            ],
        }));
        let mut api = HostApiRegistry::new();
        register_capability_host_ops(&mut api, policy, Arc::clone(&workspace));

        let result = api
            .call("workspaceSwitch", &[HostValue::string("ws-other")])
            .expect("switch ok");
        let active = result
            .as_object()
            .and_then(|o| o.get("activeWorkspaceId"))
            .and_then(HostValue::as_str);
        assert_eq!(active, Some("ws-other"));
        assert_eq!(workspace.lock().unwrap().active_workspace_id, "ws-other");
    }

    #[test]
    fn secret_read_denied_by_default() {
        let policy = shared_permission_policy(PermissionPolicy::new());
        let workspace = Arc::new(Mutex::new(WorkspaceBootstrap::default()));
        let mut api = HostApiRegistry::new();
        register_capability_host_ops(&mut api, policy, workspace);
        let err = api
            .call("secretGet", &[HostValue::string("github.token")])
            .expect_err("secret denied");
        assert!(err.message.contains("permission denied"));
    }
}
