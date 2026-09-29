//! `@font-face` through the bridge: the CSS family reaches the loaded face.
use crate::bridge::MessageBridge;
#[test]
fn inject_font_face_aliases_css_family_onto_loaded_face() {
    let css = format!(
        r#"
        @font-face {{
            font-family: "Host Sans";
            src: url("{src}") format("truetype");
            font-weight: 400;
            font-display: swap;
        }}
        .title {{ font-family: "Host Sans", sans-serif; }}
        "#,
        src = "NotoSansSC-Regular.ttf"
    );
    let mut bridge = MessageBridge::new();
    bridge.set_stylesheet_base(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../nana-ui/assets/fonts"),
    );
    bridge.inject_stylesheet(&css);
    let used = nana_ui::shaped_face_families("Host Sans", "H");
    assert!(
        used.iter().any(|name| name == "Host Sans"),
        "CSS family must map to the loaded face, used={used:?}"
    );
    // `@font-face` *replaces* the face's own names: the bytes answer to
    // the declared family and to nothing else, so a stylesheet cannot
    // reach them by the name table Noto happens to carry. Before #99 the
    // alias was pushed alongside the original names, which let
    // `font-family: "Noto Sans SC"` resolve to a face the document never
    // declared.
    assert!(
        !used.iter().any(|name| name.contains("Noto")),
        "a declared family must not keep the face's own names, used={used:?}"
    );
}

#[test]
fn inject_font_face_bad_src_is_not_registered() {
    let mut bridge = MessageBridge::new();
    bridge.inject_stylesheet(
        r#"
        @font-face {
            font-family: "Ghost Face";
            src: url("data:font/ttf;base64,AAAA");
        }
        "#,
    );
    let used = nana_ui::shaped_face_families("Ghost Face", "H");
    assert!(
        !used.iter().any(|name| name == "Ghost Face"),
        "bad src must not alias a family, used={used:?}"
    );
}
