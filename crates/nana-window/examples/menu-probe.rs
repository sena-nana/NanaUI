//! Installs a menu bar and reads back what AppKit actually holds.
//!
//! Runs on the main thread, which the AppKit menu APIs require, so it verifies
//! the real platform path rather than a mock. Prints the read-back and exits
//! non-zero when it does not match what was asked for.

use nana_window::{
    Menu, MenuBar, MenuBarSupport, MenuEntry, MenuShortcut, install_application_menu_bar,
    installed_menu_bar, menu_bar_support,
};

fn main() -> std::process::ExitCode {
    let bar = MenuBar::new([
        Menu::new(
            "Probe",
            [MenuEntry::item(1, "Settings…").shortcut(MenuShortcut::primary(","))],
        ),
        Menu::new(
            "View",
            [
                MenuEntry::item(2, "Toggle Sidebar").shortcut(MenuShortcut::primary("b")),
                MenuEntry::Separator,
                MenuEntry::item(3, "Disabled Item").enabled(false),
                MenuEntry::item(4, "Checked Item").checked(true),
                MenuEntry::Submenu(Menu::new("Nested", [MenuEntry::item(5, "Deep")])),
            ],
        ),
    ]);

    println!("support: {:?}", menu_bar_support());
    let outcome = install_application_menu_bar(&bar);
    println!("install: {outcome:?}");
    if outcome != MenuBarSupport::System {
        println!("not the system menu bar on this platform; nothing to verify");
        return std::process::ExitCode::SUCCESS;
    }

    let Some(installed) = installed_menu_bar() else {
        eprintln!("FAIL: AppKit reported no main menu after install");
        return std::process::ExitCode::FAILURE;
    };
    for (title, entries) in &installed {
        println!("menu {title:?} -> {entries:?}");
    }

    let expected = vec![
        ("Probe".to_owned(), vec!["Settings…".to_owned()]),
        (
            "View".to_owned(),
            vec![
                "Toggle Sidebar".to_owned(),
                "-".to_owned(),
                "Disabled Item".to_owned(),
                "Checked Item".to_owned(),
                "Nested".to_owned(),
            ],
        ),
    ];
    if installed == expected {
        println!("OK: AppKit holds exactly the menu that was installed");
        std::process::ExitCode::SUCCESS
    } else {
        eprintln!("FAIL: expected {expected:?}");
        std::process::ExitCode::FAILURE
    }
}
