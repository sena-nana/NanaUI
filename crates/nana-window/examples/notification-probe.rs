//! Presents a desktop notification and reads back what the platform holds.
//!
//! On Windows the toast history (Action Center) is the read-back: the probe
//! presents a tagged toast, checks the history holds that tag, replaces it in
//! place, withdraws it and checks the history lets it go. Other platforms only
//! report support and the present outcome. Pass `--hold <seconds>` (at most 10)
//! to keep the toast up for a look. Exits non-zero on a mismatch.

use std::time::Duration;

use nana_window::{NotificationHost, NotificationRequest};

const TAG: &str = "nana-notification-probe";

#[cfg(target_os = "windows")]
fn history_tags() -> Result<Vec<String>, String> {
    use windows::UI::Notifications::ToastNotificationManager;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows_core::HSTRING;

    // SAFETY: a repeated initialisation on this thread is harmless.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    // The host's default AppUserModelID when the process sets none.
    let history = ToastNotificationManager::History().map_err(|e| e.to_string())?;
    let toasts = history
        .GetHistoryWithId(&HSTRING::from("NanaUI.DesktopApp"))
        .map_err(|e| e.to_string())?;
    let mut tags = Vec::new();
    for toast in toasts {
        tags.push(toast.Tag().map_err(|e| e.to_string())?.to_string_lossy());
    }
    Ok(tags)
}

#[cfg(not(target_os = "windows"))]
fn history_tags() -> Result<Vec<String>, String> {
    Err("no notification history read-back on this platform".into())
}

fn hold_seconds() -> u64 {
    let mut arguments = std::env::args().skip(1);
    match (arguments.next().as_deref(), arguments.next()) {
        (Some("--hold"), Some(value)) => value.parse::<u64>().unwrap_or(0).min(10),
        _ => 0,
    }
}

fn main() -> std::process::ExitCode {
    let host = NotificationHost::new();
    let support = host.support();
    println!("support: {support:?}");
    if !support.is_available() {
        println!("notifications unavailable here; nothing to verify");
        return std::process::ExitCode::SUCCESS;
    }
    let request = NotificationRequest::new("NanaUI notification probe", "First delivery")
        .tag(TAG)
        .action("open", "Open");
    let outcome = host.present(request);
    println!("present: {outcome:?}");
    if outcome.is_err() {
        eprintln!("FAIL: present was refused");
        return std::process::ExitCode::FAILURE;
    }
    // The shell records the toast asynchronously; give it a bounded moment.
    let mut held = Err("not read".to_owned());
    for _ in 0..20 {
        held = history_tags();
        if matches!(&held, Ok(tags) if tags.iter().any(|t| t == TAG)) || held.is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("history after present: {held:?}");
    let Ok(tags) = held else {
        println!("no read-back on this platform; present outcome only");
        let _ = host.withdraw(TAG);
        return std::process::ExitCode::SUCCESS;
    };
    if tags.iter().filter(|t| *t == TAG).count() != 1 {
        eprintln!("FAIL: the platform does not hold exactly one toast with the probe tag");
        let _ = host.withdraw(TAG);
        return std::process::ExitCode::FAILURE;
    }
    let updated = host.update(
        TAG,
        NotificationRequest::new("NanaUI notification probe", "Replaced in place").tag(TAG),
    );
    println!("update: {updated:?}");
    std::thread::sleep(Duration::from_millis(300));
    let after_update = history_tags().unwrap_or_default();
    println!("history after update: {after_update:?}");
    if after_update.iter().filter(|t| *t == TAG).count() != 1 {
        eprintln!("FAIL: replacing the toast did not keep exactly one entry");
        let _ = host.withdraw(TAG);
        return std::process::ExitCode::FAILURE;
    }
    std::thread::sleep(Duration::from_secs(hold_seconds()));
    let withdrawn = host.withdraw(TAG);
    println!("withdraw: {withdrawn:?}");
    std::thread::sleep(Duration::from_millis(300));
    let after_withdraw = history_tags().unwrap_or_default();
    println!("history after withdraw: {after_withdraw:?}");
    println!("events: {:?}", host.take_events());
    if after_withdraw.iter().any(|t| t == TAG) {
        eprintln!("FAIL: the toast is still held after withdrawal");
        return std::process::ExitCode::FAILURE;
    }
    println!("OK: the platform held exactly the presented toast, replaced it, and let it go");
    std::process::ExitCode::SUCCESS
}
