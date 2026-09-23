//! macOS bundle metadata: `Info.plist` generated from the application
//! identity and `[platform.macos]`, with the application's own plist
//! fragment merged last so a platform-specific override always wins.

use std::path::{Path, PathBuf};

use crate::config::{ApplicationConfig, MacosConfig};

/// The former `nana-package-app`: a bundle around one executable, without
/// resource packs or a package manifest. The version comes from the
/// executable's identity marker when it has one.
#[derive(Debug, Clone)]
pub struct BareApp {
    pub exe: PathBuf,
    pub name: String,
    pub identifier: String,
    pub out: PathBuf,
    pub icon: Option<PathBuf>,
    pub strip: bool,
}

pub fn bare_app(spec: &BareApp) -> Result<PathBuf, String> {
    if spec.name.trim().is_empty() || spec.identifier.trim().is_empty() {
        return Err("--name and --identifier are required".into());
    }
    let exe_name = spec
        .exe
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("executable name is not valid UTF-8")?
        .to_owned();
    let bytes = std::fs::read(&spec.exe)
        .map_err(|error| format!("cannot read {}: {error}", spec.exe.display()))?;
    let version = nana_package::find_marker(&bytes)
        .map(|identity| identity.version)
        .unwrap_or_else(|_| "0.0.0".into());
    let app_dir = if spec.out.extension().is_some_and(|ext| ext == "app") {
        spec.out.clone()
    } else {
        spec.out.join(format!("{}.app", spec.name))
    };
    let io = |what: &'static str| move |error: std::io::Error| format!("{what}: {error}");
    let macos = app_dir.join("Contents/MacOS");
    let resources = app_dir.join("Contents/Resources");
    std::fs::create_dir_all(&macos).map_err(io("create Contents/MacOS"))?;
    std::fs::create_dir_all(&resources).map_err(io("create Contents/Resources"))?;
    let dest = macos.join(&exe_name);
    std::fs::write(&dest, &bytes).map_err(io("copy executable"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
            .map_err(io("chmod executable"))?;
    }
    if spec.strip && cfg!(target_os = "macos") {
        strip(&dest)?;
    }
    let icon = match &spec.icon {
        Some(path) => std::fs::read(path).map_err(io("read icon"))?,
        None => nana_app_icon::encode_icns()?,
    };
    std::fs::write(resources.join("AppIcon.icns"), icon).map_err(io("write AppIcon.icns"))?;
    let application = ApplicationConfig {
        id: spec.identifier.clone(),
        name: spec.name.clone(),
        version,
        vendor: None,
    };
    let plist = info_plist(&PlistInput {
        application: &application,
        macos: &MacosConfig::default(),
        executable: &exe_name,
        build_id: None,
        extra: None,
    })?;
    std::fs::write(app_dir.join("Contents/Info.plist"), plist).map_err(io("write Info.plist"))?;
    Ok(app_dir)
}

fn strip(exe: &Path) -> Result<(), String> {
    let status = std::process::Command::new("strip")
        .arg("-x")
        .arg(exe)
        .status()
        .map_err(|error| format!("cannot run strip: {error}"))?;
    if !status.success() {
        return Err(format!("strip failed on {}: {status}", exe.display()));
    }
    Ok(())
}

pub struct PlistInput<'a> {
    pub application: &'a ApplicationConfig,
    pub macos: &'a MacosConfig,
    pub executable: &'a str,
    pub build_id: Option<&'a str>,
    /// Raw `<key>…</key><value/>` pairs from `info_plist_extra`.
    pub extra: Option<&'a str>,
}

enum Value {
    String(String),
    Bool(bool),
    Raw(String),
}

pub fn info_plist(input: &PlistInput<'_>) -> Result<String, String> {
    let app = input.application;
    let macos = input.macos;
    let identifier = macos.bundle_identifier.as_deref().unwrap_or(&app.id);
    let mut entries: Vec<(&str, Value)> = vec![
        ("CFBundleDevelopmentRegion", Value::String("en".into())),
        ("CFBundleDisplayName", Value::String(app.name.clone())),
        ("CFBundleExecutable", Value::String(input.executable.into())),
        ("CFBundleIconFile", Value::String("AppIcon".into())),
        ("CFBundleIdentifier", Value::String(identifier.into())),
        ("CFBundleInfoDictionaryVersion", Value::String("6.0".into())),
        ("CFBundleName", Value::String(app.name.clone())),
        ("CFBundlePackageType", Value::String("APPL".into())),
        (
            "CFBundleShortVersionString",
            Value::String(app.version.clone()),
        ),
        (
            "CFBundleVersion",
            Value::String(input.build_id.unwrap_or(&app.version).into()),
        ),
        (
            "LSMinimumSystemVersion",
            Value::String(macos.minimum_system_version.clone()),
        ),
        ("NSHighResolutionCapable", Value::Bool(true)),
    ];
    if let Some(category) = &macos.category {
        entries.push(("LSApplicationCategoryType", Value::String(category.clone())));
    }
    if !macos.url_schemes.is_empty() {
        let schemes: String = macos
            .url_schemes
            .iter()
            .map(|scheme| format!("        <string>{}</string>\n", escape(scheme)))
            .collect();
        entries.push((
            "CFBundleURLTypes",
            Value::Raw(format!(
                "<array>\n    <dict>\n      <key>CFBundleURLName</key>\n      <string>{}</string>\n      <key>CFBundleURLSchemes</key>\n      <array>\n{schemes}      </array>\n    </dict>\n  </array>",
                escape(identifier)
            )),
        ));
    }
    if !macos.document_types.is_empty() {
        let mut raw = String::from("<array>\n");
        for doc in &macos.document_types {
            let extensions: String = doc
                .extensions
                .iter()
                .map(|ext| format!("        <string>{}</string>\n", escape(ext)))
                .collect();
            raw.push_str(&format!(
                "    <dict>\n      <key>CFBundleTypeName</key>\n      <string>{}</string>\n      <key>CFBundleTypeRole</key>\n      <string>{}</string>\n      <key>CFBundleTypeExtensions</key>\n      <array>\n{extensions}      </array>\n    </dict>\n",
                escape(&doc.name),
                escape(&doc.role)
            ));
        }
        raw.push_str("  </array>");
        entries.push(("CFBundleDocumentTypes", Value::Raw(raw)));
    }

    let overridden = match input.extra {
        Some(extra) => plist_keys(extra)?,
        None => Vec::new(),
    };
    for protected in [
        "CFBundleExecutable",
        "CFBundleIdentifier",
        "CFBundleShortVersionString",
    ] {
        if overridden.iter().any(|key| key == protected) {
            return Err(format!(
                "info_plist_extra may not override {protected}; it is derived from the \
                 application identity and checked against the binary"
            ));
        }
    }

    let mut body = String::new();
    for (key, value) in entries {
        if overridden.iter().any(|o| o == key) {
            continue;
        }
        body.push_str(&format!("  <key>{key}</key>\n  "));
        match value {
            Value::String(text) => body.push_str(&format!("<string>{}</string>\n", escape(&text))),
            Value::Bool(true) => body.push_str("<true/>\n"),
            Value::Bool(false) => body.push_str("<false/>\n"),
            Value::Raw(raw) => {
                body.push_str(&raw);
                body.push('\n');
            }
        }
    }
    if let Some(extra) = input.extra {
        for line in extra.trim().lines() {
            body.push_str("  ");
            body.push_str(line.trim_end());
            body.push('\n');
        }
    }
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n{body}</dict>\n</plist>\n"
    ))
}

/// Top-level `<key>` names in a plist fragment.
fn plist_keys(fragment: &str) -> Result<Vec<String>, String> {
    let mut keys = Vec::new();
    let mut depth = 0i32;
    let mut rest = fragment;
    while let Some(start) = rest.find('<') {
        rest = &rest[start..];
        let end = rest.find('>').ok_or("info_plist_extra: unterminated tag")?;
        let tag = &rest[1..end];
        if let Some(inner) = rest.strip_prefix("<key>") {
            if depth == 0 {
                let close = inner
                    .find("</key>")
                    .ok_or("info_plist_extra: unterminated <key>")?;
                keys.push(inner[..close].trim().to_owned());
            }
        } else if tag.starts_with("dict") || tag.starts_with("array") {
            if !tag.ends_with('/') {
                depth += 1;
            }
        } else if tag == "/dict" || tag == "/array" {
            depth -= 1;
        }
        rest = &rest[end + 1..];
    }
    if depth != 0 {
        return Err("info_plist_extra: unbalanced <dict>/<array>".into());
    }
    Ok(keys)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DocumentType;

    fn app() -> ApplicationConfig {
        ApplicationConfig {
            id: "dev.nana.fixture".into(),
            name: "Nana & Fixture".into(),
            version: "1.2.3".into(),
            vendor: None,
        }
    }

    #[test]
    fn plist_carries_identity_and_capabilities() {
        let macos = MacosConfig {
            url_schemes: vec!["nanafixture".into()],
            document_types: vec![DocumentType {
                name: "Fixture Doc".into(),
                extensions: vec!["nfx".into()],
                role: "Editor".into(),
            }],
            category: Some("public.app-category.games".into()),
            ..MacosConfig::default()
        };
        let app = app();
        let plist = info_plist(&PlistInput {
            application: &app,
            macos: &macos,
            executable: "NanaFixture",
            build_id: Some("42"),
            extra: None,
        })
        .unwrap();
        assert!(plist.contains("<string>1.2.3</string>"));
        assert!(plist.contains("<key>CFBundleVersion</key>\n  <string>42</string>"));
        assert!(plist.contains("<string>Nana &amp; Fixture</string>"));
        assert!(plist.contains("<string>nanafixture</string>"));
        assert!(plist.contains("<string>nfx</string>"));
        assert!(plist.contains("<string>11.0</string>"));
    }

    #[test]
    fn bare_app_layout_includes_plist_and_icns() {
        let dir = std::env::temp_dir().join(format!("nana-packager-bare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("dummy-bin");
        std::fs::write(&exe, b"not-a-real-binary").unwrap();
        let out = dir.join("Dummy.app");
        let packed = bare_app(&BareApp {
            exe,
            name: "Dummy".into(),
            identifier: "dev.nanaui.dummy".into(),
            out: out.clone(),
            icon: None,
            // The fixture is not a Mach-O file; `strip` would reject it.
            strip: false,
        })
        .unwrap();
        assert_eq!(packed, out);
        let plist = std::fs::read_to_string(out.join("Contents/Info.plist")).unwrap();
        assert!(plist.contains("dev.nanaui.dummy"));
        assert!(out.join("Contents/Resources/AppIcon.icns").is_file());
        assert!(out.join("Contents/MacOS/dummy-bin").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extra_overrides_generated_keys_but_not_identity() {
        let app = app();
        let macos = MacosConfig::default();
        let extra = "<key>LSMinimumSystemVersion</key>\n<string>13.0</string>\n<key>NSCameraUsageDescription</key>\n<string>Scan</string>";
        let plist = info_plist(&PlistInput {
            application: &app,
            macos: &macos,
            executable: "X",
            build_id: None,
            extra: Some(extra),
        })
        .unwrap();
        assert!(plist.contains("<string>13.0</string>"));
        assert!(!plist.contains("<string>11.0</string>"));
        assert!(plist.contains("NSCameraUsageDescription"));

        let bad = "<key>CFBundleIdentifier</key><string>evil</string>";
        assert!(
            info_plist(&PlistInput {
                application: &app,
                macos: &macos,
                executable: "X",
                build_id: None,
                extra: Some(bad),
            })
            .is_err()
        );
    }
}
