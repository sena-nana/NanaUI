//! Tauri IPC compatibility for the Nana host (no WebView / no real Tauri).
//!
//! Generic soft stubs for `@tauri-apps/api` / plugin invokes that appear in
//! bundled frontend graphs. Projects can extend `stub_invoke` or register
//! additional host ops on top of this layer.

use std::collections::BTreeMap;

use nana_js_engine::{HostApiRegistry, HostValue};

/// Prepended to the business bundle so `window.__TAURI_INTERNALS__.invoke` exists
/// before any module body runs.
pub const TAURI_COMPAT_JS: &str = r#"
(function () {
  var g = globalThis;
  if (!g.window) g.window = g;
  function hostInvoke(cmd, args) {
    try {
      if (g.__nanaHost && typeof g.__nanaHost.call === "function") {
        return g.__nanaHost.call("tauriInvoke", [String(cmd || ""), args || {}]);
      }
    } catch (_e) {}
    return null;
  }
  function asPromise(value) {
    return Promise.resolve(value);
  }
  if (!g.__TAURI_INTERNALS__) {
    g.__TAURI_INTERNALS__ = {
      invoke: function (cmd, args) {
        return asPromise(hostInvoke(cmd, args));
      },
      transformCallback: function (callback) {
        return callback;
      },
      metadata: {
        currentWindow: { label: "main" },
        currentWebview: { label: "main" },
      },
    };
  } else if (typeof g.__TAURI_INTERNALS__.invoke !== "function") {
    g.__TAURI_INTERNALS__.invoke = function (cmd, args) {
      return asPromise(hostInvoke(cmd, args));
    };
  }
  if (!g.__TAURI_EVENT_PLUGIN_INTERNALS__) {
    g.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: function () {},
    };
  }
  g.__nanaTauriCompat = {
    invoke: hostInvoke,
    mode: "stub",
  };
})();
"#;

/// Register `tauriInvoke` on the Vue host API registry.
pub fn register_tauri_compat(api: &mut HostApiRegistry) {
    api.register("tauriInvoke", |args| {
        let cmd = args
            .first()
            .and_then(HostValue::as_str)
            .unwrap_or("")
            .to_string();
        let payload = args.get(1).cloned().unwrap_or(HostValue::Null);
        Ok(stub_invoke(&cmd, &payload))
    });
}

fn stub_invoke(cmd: &str, args: &HostValue) -> HostValue {
    match cmd {
        "" => HostValue::Null,
        "plugin:window|is_fullscreen"
        | "plugin:window|is_maximized"
        | "plugin:window|is_minimized"
        | "plugin:window|is_visible"
        | "plugin:window|is_decorated"
        | "plugin:window|is_resizable"
        | "plugin:window|is_focused" => HostValue::Bool(false),
        "plugin:window|theme" | "plugin:window|scale_factor" => HostValue::string("light"),
        "plugin:window|inner_size"
        | "plugin:window|outer_size"
        | "plugin:window|inner_position"
        | "plugin:window|outer_position" => json_object(&[
            ("width", HostValue::Number(960.0)),
            ("height", HostValue::Number(640.0)),
            ("x", HostValue::Number(0.0)),
            ("y", HostValue::Number(0.0)),
        ]),
        "plugin:store|get" | "plugin:store|get_store" => HostValue::Null,
        "plugin:store|set" | "plugin:store|save" | "plugin:store|delete" | "plugin:store|clear" => {
            HostValue::Null
        }
        "plugin:dialog|open"
        | "plugin:dialog|save"
        | "plugin:dialog|message"
        | "plugin:dialog|ask"
        | "plugin:dialog|confirm"
        | "plugin:opener|open_url"
        | "plugin:opener|open_path" => HostValue::Null,
        // Soft empty collections for common list-shaped commands.
        other if other.ends_with("_list") || other.contains("list_") => {
            HostValue::Array(Vec::new())
        }
        // Workspace / github soft stubs — business mockTransport usually handles
        // these first; keep host-side fallbacks for stray Tauri invokes.
        "workspace_get_bootstrap" | "workspace_create" | "workspace_get_settings" => {
            eprintln!("[tauri-compat] stub invoke `{cmd}` (prefer JS mockTransport)");
            HostValue::Null
        }
        "github_get_binding_status" => json_object(&[
            ("state", HostValue::string("bound")),
            ("clientIdConfigured", HostValue::Bool(true)),
            ("clientIdSource", HostValue::string("mock")),
            ("binding", HostValue::Null),
        ]),
        _ => {
            let _ = args;
            eprintln!("[tauri-compat] stub invoke `{cmd}` → null");
            HostValue::Null
        }
    }
}

fn json_object(entries: &[(&str, HostValue)]) -> HostValue {
    let mut map = BTreeMap::new();
    for (k, v) in entries {
        map.insert((*k).to_string(), v.clone());
    }
    HostValue::Object(map)
}
