//! 共享剪贴板合同与桌面富文本来源验证。
use super::*;

#[test]
fn rich_clipboard_html_keeps_escaped_text_and_an_owned_identity() {
    let html = rich_clipboard_html("甲<&>\n乙", "1-a");
    assert_eq!(
        html,
        "<!--nana-ui-rich:1-a--><pre>甲&lt;&amp;&gt;\n乙</pre>"
    );
    assert_eq!(rich_clipboard_token(&html), Some("1-a"));
    assert_eq!(rich_clipboard_token("<pre>甲</pre>"), None);
    assert_eq!(rich_clipboard_token("<!--nana-ui-rich:bad token-->"), None);
    let mut clipboard = MemoryClipboard::new();
    assert!(clipboard.write_text_with_token("甲", "1-a"));
    assert_eq!(clipboard.read_token(), Some("1-a".into()));
    assert!(clipboard.write_text("甲"));
    assert_eq!(clipboard.read_token(), None);
}

#[test]
fn unsupported_clipboard_is_inert() {
    let mut clip = UnsupportedClipboard;
    assert!(clip.read_text().is_none());
    assert!(!clip.write_text("nana"));
}

#[test]
fn memory_clipboard_roundtrip() {
    let mut clip = MemoryClipboard::new();
    assert!(clip.read_text().is_none());
    assert!(clip.write_text("hello"));
    assert_eq!(clip.read_text().as_deref(), Some("hello"));
    assert!(clip.write_text(""));
    assert_eq!(clip.read_text().as_deref(), Some(""));
}

#[test]
#[cfg(target_os = "android")]
fn android_default_clipboard_uses_jni_backend() {
    let host = default_shared_clipboard();
    let mut clip = host.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // Host `cargo test` has no Activity context; JNI must fail closed
    // rather than report a successful pasteboard round-trip.
    assert!(clip.read_text().is_none());
    assert!(!clip.write_text("nana"));
}

#[test]
#[cfg(not(any(target_os = "android", target_env = "ohos")))]
fn os_clipboard_roundtrip_when_available() {
    let mut clip = OsClipboard::new();
    if !clip.is_available() {
        // Headless / locked-down environments may lack a pasteboard.
        return;
    }
    let marker = format!("nana-ui-clipboard-{}-{}", std::process::id(), "probe");
    let previous = clip.read_text();

    // The pasteboard is one machine-wide resource, so anything else on the
    // machine — including a second `cargo test` — can overwrite it between
    // our write and our read. That is interference, not a broken clipboard,
    // so the write/read pair is retried as a unit and only a run that never
    // round-trips is a failure. Asserting on a single attempt made this
    // test fail whenever two test processes overlapped.
    let mut observed = None;
    for _ in 0..8 {
        assert!(
            clip.write_text(&marker),
            "desktop OS clipboard write must succeed when available"
        );
        observed = clip.read_text();
        if observed.as_deref() == Some(marker.as_str()) {
            break;
        }
    }
    assert_eq!(
        observed.as_deref(),
        Some(marker.as_str()),
        "desktop OS clipboard read must return the written text"
    );
    let token = format!("{:x}-ab", std::process::id());
    let mut rich_roundtrip = false;
    for _ in 0..8 {
        assert!(clip.write_text_with_token(&marker, &token));
        rich_roundtrip = clip.read_text().as_deref() == Some(marker.as_str())
            && clip.read_token().as_deref() == Some(token.as_str());
        if rich_roundtrip {
            break;
        }
    }
    assert!(
        rich_roundtrip,
        "desktop HTML clipboard must retain the rich source token"
    );
    assert!(clip.write_text(&marker));
    assert_eq!(clip.read_token(), None);

    if let Some(prev) = previous {
        let _ = clip.write_text(&prev);
    }
}

#[test]
fn a_busy_shared_clipboard_answers_busy_instead_of_waiting() {
    let clipboard = shared_clipboard(MemoryClipboard::new());
    write_shared_clipboard(&clipboard, "hello").unwrap();
    assert_eq!(read_shared_clipboard(&clipboard), Ok(Some("hello".into())));
    let _held = clipboard.lock().unwrap();
    assert_eq!(
        read_shared_clipboard(&clipboard),
        Err(nana_ui_input::HostServiceError::Busy)
    );
}
