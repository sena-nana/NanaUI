//! The packaging fixture (Issue #226): a headless application that reads
//! its resources through `nana://res/`. `nana-packager` packages it, and CI
//! launches the package from an unrelated directory with
//! `NANA_PACKAGE_VALIDATE=1`, then tampers with copies and expects each
//! launch to fail.
//!
//! Key policy (the application's, not the framework's): the content key
//! comes from `NANA_FIXTURE_CONTENT_KEY`, and a publisher key compiled in
//! through `NANA_FIXTURE_PUBLISHER_KEY` pins signatures.

use nana_ui::nana_package::{ContentKey, KeyId, PublisherKey, StaticKeys, TrustPolicy};
use nana_ui::{NanaApplication, ResourcePackOptions};

fn main() {
    let identity = nana_ui_platform::application_identity!(
        id: "dev.nana.package-fixture",
        name: "Nana Package Fixture",
        version: env!("CARGO_PKG_VERSION"),
        vendor: "Nana",
    );
    let mut keys = StaticKeys::new();
    if let Some(key) = std::env::var("NANA_FIXTURE_CONTENT_KEY")
        .ok()
        .and_then(|text| ContentKey::from_hex(&text))
    {
        keys.insert(KeyId::from_name("content-main"), 1, key);
    }
    let trust = match option_env!("NANA_FIXTURE_PUBLISHER_KEY").and_then(PublisherKey::from_text) {
        Some(key) => TrustPolicy::RequirePublisher(key),
        None => TrustPolicy::AllowUnsigned,
    };
    let options = ResourcePackOptions::new()
        .keys(keys)
        .trust(trust)
        .loose_root(concat!(env!("CARGO_MANIFEST_DIR"), "/assets"));
    let _session = NanaApplication::builder(identity)
        .resource_packs(options)
        .start();

    for url in ["nana://res/boot/loading.txt", "nana://res/ui/css/app.css"] {
        match nana_ui_core::read_packaged(url, None, 1 << 20) {
            Some(bytes) => println!("{url}: {} bytes", bytes.len()),
            None => {
                eprintln!("{url}: unavailable");
                std::process::exit(1);
            }
        }
    }
}
