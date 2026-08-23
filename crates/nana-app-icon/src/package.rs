use std::fs;
use std::path::PathBuf;

use crate::encode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacAppPackage {
    pub exe: PathBuf,
    pub name: String,
    pub identifier: String,
    pub out: PathBuf,
    pub icon: Option<PathBuf>,
}

pub fn package_macos_app(spec: &MacAppPackage) -> Result<PathBuf, String> {
    if spec.name.trim().is_empty() {
        return Err("app name is required".into());
    }
    if spec.identifier.trim().is_empty() {
        return Err("bundle identifier is required".into());
    }
    let exe_name = spec
        .exe
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "executable name is not valid UTF-8".to_string())?;
    let app_dir = if spec.out.extension().is_some_and(|ext| ext == "app") {
        spec.out.clone()
    } else {
        spec.out.join(format!("{}.app", spec.name))
    };
    let contents = app_dir.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    fs::create_dir_all(&macos).map_err(io_err("MacOS"))?;
    fs::create_dir_all(&resources).map_err(io_err("Resources"))?;

    let dest_exe = macos.join(exe_name);
    fs::copy(&spec.exe, &dest_exe).map_err(|error| {
        format!(
            "failed to copy {} to {}: {error}",
            spec.exe.display(),
            dest_exe.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&dest_exe)
            .map_err(io_err("copied executable metadata"))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&dest_exe, permissions).map_err(io_err("chmod executable"))?;
    }

    let icns_path = resources.join("AppIcon.icns");
    if let Some(icon) = &spec.icon {
        fs::copy(icon, &icns_path)
            .map_err(|error| format!("failed to copy icon {}: {error}", icon.display()))?;
    } else {
        fs::write(&icns_path, encode::icns()?).map_err(io_err("write AppIcon.icns"))?;
    }
    fs::write(
        contents.join("Info.plist"),
        info_plist(&spec.name, &spec.identifier, exe_name),
    )
    .map_err(io_err("write Info.plist"))?;
    Ok(app_dir)
}

fn info_plist(name: &str, identifier: &str, executable: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>{name}</string>
  <key>CFBundleExecutable</key>
  <string>{executable}</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundleIdentifier</key>
  <string>{identifier}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>{name}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
"#
    )
}

fn io_err(context: &'static str) -> impl FnOnce(std::io::Error) -> String {
    move |error| format!("{context}: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_layout_includes_plist_and_icns() {
        let temp =
            std::env::temp_dir().join(format!("nana-app-icon-package-{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();
        let exe = temp.join("dummy-bin");
        fs::write(&exe, b"not-a-real-binary").unwrap();
        let out = temp.join("NanaUI Dummy.app");
        let packed = package_macos_app(&MacAppPackage {
            exe,
            name: "NanaUI Dummy".into(),
            identifier: "dev.nanaui.dummy".into(),
            out: out.clone(),
            icon: None,
        })
        .expect("package");
        assert_eq!(packed, out);
        let plist = fs::read_to_string(out.join("Contents/Info.plist")).unwrap();
        assert!(plist.contains("dev.nanaui.dummy"));
        assert!(out.join("Contents/Resources/AppIcon.icns").is_file());
        assert!(out.join("Contents/MacOS/dummy-bin").is_file());
        let _ = fs::remove_dir_all(&temp);
    }
}
