//! Installs a tray and reads back what the platform actually holds.
//!
//! Runs on the main thread, which the tray needs, so it verifies the real
//! platform path. Prints the read-back and exits non-zero when it does not
//! match what was asked for. Pass `--hold <seconds>` to keep the tray up for
//! a look.

use nana_window::{
    MenuEntry, Tray, TraySupport, install_tray, installed_tray, remove_tray, tray_support,
};

/// A 2×2 opaque square: enough for the platform to accept an icon.
const ICON: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xb6, 0x0d,
    0x24, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x60, 0x60, 0xf8,
    0x0f, 0xc5, 0x10, 0x06, 0x00, 0x1d, 0xf4, 0x03, 0xfd, 0xc8, 0xe1, 0xf6, 0xa8, 0x00, 0x00, 0x00,
    0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn tray(checked: bool) -> Tray {
    Tray {
        tooltip: "Tray probe".into(),
        icon_png: ICON.to_vec(),
        template_icon: true,
        menu: vec![
            MenuEntry::item(1, "Show"),
            MenuEntry::Separator,
            MenuEntry::item(2, "Click-through").checked(checked),
            MenuEntry::item(3, "Quit"),
        ],
        primary: Some(1),
        ..Default::default()
    }
}

fn main() -> std::process::ExitCode {
    println!("support: {:?}", tray_support());
    let outcome = install_tray(&tray(false), || {});
    println!("install: {outcome:?}");
    if outcome != TraySupport::MenuBarExtra {
        println!("no read-back on this platform; nothing to verify");
        remove_tray();
        return std::process::ExitCode::SUCCESS;
    }
    // Installing again updates the same item in place.
    install_tray(&tray(true), || {});
    let expected = (
        "Tray probe".to_owned(),
        vec![
            "Show".to_owned(),
            "-".to_owned(),
            "Click-through".to_owned(),
            "Quit".to_owned(),
        ],
    );
    let installed = installed_tray();
    println!("installed: {installed:?}");
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() == Some("--hold")
        && let Some(seconds) = arguments.next().and_then(|value| value.parse().ok())
    {
        std::thread::sleep(std::time::Duration::from_secs(seconds));
    }
    remove_tray();
    if installed.as_ref() != Some(&expected) {
        eprintln!("FAIL: expected {expected:?}");
        return std::process::ExitCode::FAILURE;
    }
    if installed_tray().is_some() {
        eprintln!("FAIL: the tray is still held after removal");
        return std::process::ExitCode::FAILURE;
    }
    println!("OK: the platform holds exactly the tray that was installed, and lets it go");
    std::process::ExitCode::SUCCESS
}
