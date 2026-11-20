//! Release steps after `package`: signed update manifests (Issue #274 §5)
//! and the Android asset layout (§4).

use std::path::{Path, PathBuf};

use nana_package::PackageManifest;
use nana_package::manifest::ManifestTarget;
use nana_package::update::{
    ArtifactFormat, UPDATE_FILE, UPDATE_SCHEMA, UPDATE_SCHEMA_VERSION, UPDATE_SIGNATURE_FILE,
    UpdateApplication, UpdateArtifact, UpdateManifest,
};

use crate::layout::PackageLayout;
use crate::secrets::{LoadedSecrets, SecretSources};
use crate::util::{content_hex, io};

/// The manifest, layout and executable of a packaged application root.
pub struct LocatedApp {
    pub manifest: PackageManifest,
    pub manifest_bytes: Vec<u8>,
    pub manifest_dir: PathBuf,
    pub layout: PackageLayout,
    pub executable: PathBuf,
}

pub fn locate_app(app_root: &Path) -> Result<LocatedApp, String> {
    let manifest_dir = crate::validate::manifest_dir(app_root)?;
    let manifest_bytes = std::fs::read(manifest_dir.join(nana_package::MANIFEST_FILE))
        .map_err(io("read package manifest"))?;
    let manifest = PackageManifest::from_json(&manifest_bytes).map_err(|e| e.to_string())?;
    let root_name = app_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let layout = PackageLayout::from_manifest(root_name, &manifest.layout);
    let executable = PackageLayout::join(app_root, &layout.executable);
    Ok(LocatedApp {
        manifest,
        manifest_bytes,
        manifest_dir,
        layout,
        executable,
    })
}

/// The artifact format a file name names.
pub fn artifact_format(path: &Path) -> Option<ArtifactFormat> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    Some(if name.ends_with(".dmg") {
        ArtifactFormat::Dmg
    } else if name.ends_with(".msi") {
        ArtifactFormat::Msi
    } else if name.ends_with(".exe") {
        ArtifactFormat::Nsis
    } else if name.ends_with(".deb") {
        ArtifactFormat::Deb
    } else if name.ends_with(".appimage") {
        ArtifactFormat::Appimage
    } else if name.ends_with(".flatpak") {
        ArtifactFormat::Flatpak
    } else if [".zip", ".tar.gz", ".tgz", ".tar.zst", ".tar.xz"]
        .iter()
        .any(|ext| name.ends_with(ext))
    {
        ArtifactFormat::Archive
    } else {
        return None;
    })
}

#[derive(Debug, Clone)]
pub struct UpdateManifestOptions {
    /// The packaged application the update installs.
    pub app_root: PathBuf,
    pub channel: String,
    pub artifacts: Vec<PathBuf>,
    pub minimum_from_version: Option<String>,
    /// Directory for `update.json` and `update.json.sig`.
    pub out: PathBuf,
    pub secrets: SecretSources,
}

/// Write a publisher-signed update manifest for the package at
/// `app_root`. Updates are always signed: without the publisher key this
/// fails.
pub fn update_manifest(options: &UpdateManifestOptions) -> Result<UpdateManifest, String> {
    let app = locate_app(&options.app_root)?;
    let repo_root = crate::secrets::repository_root(&options.app_root);
    let secrets = LoadedSecrets::load(
        &Default::default(),
        true,
        &options.secrets,
        repo_root.as_deref(),
    )?;
    let (signing, public) = secrets.publisher.as_ref().expect("publisher requested");
    if let Some(pinned) = &app.manifest.signing.publisher_key
        && *pinned != public.to_text()
    {
        return Err(format!(
            "the package was signed by {pinned}; the update must be signed by the same key"
        ));
    }
    if options.artifacts.is_empty() {
        return Err("an update needs at least one --artifact".into());
    }
    let mut artifacts = Vec::new();
    for path in &options.artifacts {
        let format = artifact_format(path).ok_or_else(|| {
            format!(
                "{}: not an installer or archive (dmg, msi, exe, deb, AppImage, flatpak, zip, tar.*)",
                path.display()
            )
        })?;
        let bytes = std::fs::read(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        artifacts.push(UpdateArtifact {
            file: path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| format!("{} has no UTF-8 file name", path.display()))?
                .to_owned(),
            format,
            size: bytes.len() as u64,
            blake3: content_hex(&bytes),
        });
    }
    let application = &app.manifest.application;
    let update = UpdateManifest {
        schema: UPDATE_SCHEMA.into(),
        schema_version: UPDATE_SCHEMA_VERSION,
        application: UpdateApplication {
            id: application.id.clone(),
            version: application.version.clone(),
            build_id: application.build_id.clone(),
        },
        channel: options.channel.clone(),
        target: ManifestTarget {
            triple: app.manifest.target.triple.clone(),
            platform: app.manifest.target.platform.clone(),
        },
        minimum_from_version: options.minimum_from_version.clone(),
        package_manifest_blake3: content_hex(&app.manifest_bytes),
        artifacts,
    };
    let json = update.to_json();
    // What an updater will parse: refuse to publish one it would reject.
    UpdateManifest::from_json(json.as_bytes())
        .map_err(|error| format!("the update manifest would be rejected: {error}"))?;
    use ed25519_dalek::Signer;
    let signature = signing.sign(&UpdateManifest::signing_message(json.as_bytes()));
    std::fs::create_dir_all(&options.out).map_err(io("create update output"))?;
    std::fs::write(options.out.join(UPDATE_FILE), &json).map_err(io("write update manifest"))?;
    std::fs::write(
        options.out.join(UPDATE_SIGNATURE_FILE),
        signature.to_bytes(),
    )
    .map_err(io("write update signature"))?;
    Ok(update)
}

/// Copy the package manifest (and signature) and the resource packs of the
/// application at `app_root` into `<assets>/nana/{manifest,resources}/`, the
/// layout the Android runtime reads through the `AssetManager`. The APK
/// must store `.nrpack` uncompressed (`androidResources.noCompress`).
/// Returns the files written.
pub fn android_assets(app_root: &Path, assets: &Path) -> Result<Vec<PathBuf>, String> {
    let app = locate_app(app_root)?;
    let manifest_out = PackageLayout::join(assets, nana_package::ANDROID_ASSET_MANIFEST_DIR);
    let resources_out = PackageLayout::join(assets, nana_package::ANDROID_ASSET_RESOURCES_DIR);
    for dir in [&manifest_out, &resources_out] {
        if dir.exists() {
            std::fs::remove_dir_all(dir).map_err(io("clear previous assets"))?;
        }
        std::fs::create_dir_all(dir).map_err(io("create asset directory"))?;
    }
    let mut written = Vec::new();
    for name in [
        nana_package::MANIFEST_FILE,
        nana_package::MANIFEST_SIGNATURE_FILE,
    ] {
        let from = app.manifest_dir.join(name);
        if from.is_file() {
            let to = manifest_out.join(name);
            std::fs::copy(&from, &to).map_err(io("copy manifest"))?;
            written.push(to);
        }
    }
    let resources = PackageLayout::join(app_root, &app.layout.runtime_resources);
    for pack in &app.manifest.resource_packs {
        let to = resources_out.join(&pack.file);
        std::fs::copy(resources.join(&pack.file), &to)
            .map_err(|error| format!("cannot copy pack {}: {error}", pack.file))?;
        written.push(to);
    }
    Ok(written)
}
