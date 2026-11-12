use nana_icons_tabler::{HOME, TRASH, USER};
use nana_ui_core::icon::{IconPathCommand, IconShape};

#[test]
fn catalog_consts_resolve_with_valid_identity() {
    // Named IconData statics keep `Icon` pointer identity stable per process.
    assert_eq!(HOME, nana_icons_tabler::HOME);
    assert_ne!(HOME, TRASH);
}

#[test]
fn catalog_icons_carry_paintable_geometry_without_dead_svg_strings() {
    for icon in [HOME, TRASH, USER] {
        assert!(
            !icon.shapes().is_empty(),
            "{:?} missing shapes",
            icon.name()
        );
        assert!(
            icon.svg().is_empty(),
            "{:?} should not retain dead SVG source once shapes are precompiled",
            icon.name()
        );
        let mut points = Vec::new();
        for shape in icon.shapes() {
            match *shape {
                IconShape::Path(cmds) | IconShape::Fill(cmds) => {
                    for cmd in cmds {
                        match *cmd {
                            IconPathCommand::MoveTo(p)
                            | IconPathCommand::LineTo(p)
                            | IconPathCommand::CubicTo { to: p, .. } => points.push(p),
                            IconPathCommand::Close => {}
                        }
                    }
                }
                IconShape::Circle { center, radius } => {
                    points.push([center[0] - radius, center[1] - radius]);
                    points.push([center[0] + radius, center[1] + radius]);
                }
                IconShape::Rect { origin, size, .. }
                | IconShape::RoundedRect { origin, size, .. } => {
                    points.push(origin);
                    points.push([origin[0] + size[0], origin[1] + size[1]]);
                }
            }
        }
        assert!(
            points.len() >= 2,
            "{:?} geometry must span non-degenerate coordinates",
            icon.name()
        );
        for [x, y] in points {
            assert!(
                (0.0..=24.0).contains(&x) && (0.0..=24.0).contains(&y),
                "{:?} coordinate [{x}, {y}] outside 24x24 grid",
                icon.name()
            );
        }
    }
}
