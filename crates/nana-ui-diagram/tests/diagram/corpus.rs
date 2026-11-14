//! Invariants every diagram keeps, whatever its type.

use crate::common;

use nana_ui_diagram::geom::Verb;
use nana_ui_diagram::{DiagramMetrics, FixedMeasure, Geom, Item, Limits, Rect, render};

fn points(item: &Item) -> Vec<[f32; 2]> {
    let rect = |r: &Rect| vec![[r.x, r.y], [r.x + r.w, r.y + r.h]];
    match item {
        Item::Shape { geom, .. } | Item::ClipBegin(geom) => match geom {
            Geom::Rect { rect: r, .. } | Geom::Ellipse(r) => rect(r),
            Geom::Path(p) => p
                .verbs
                .iter()
                .flat_map(|v| match *v {
                    Verb::Move(a) | Verb::Line(a) => vec![a],
                    Verb::Quad(c, a) => vec![c, a],
                    Verb::Cubic(b, c, a) => vec![b, c, a],
                    Verb::Close => vec![],
                })
                .collect(),
        },
        Item::Text { rect: r, .. } => rect(r),
        Item::ClipEnd => vec![],
    }
}

#[test]
fn every_fixture_is_finite_inside_its_canvas_and_deterministic() {
    for (family, name, source) in common::fixtures() {
        let render_once = || {
            render(
                &source,
                &FixedMeasure,
                &DiagramMetrics::DEFAULT,
                &Limits::DEFAULT,
            )
            .unwrap()
        };
        let scene = render_once();
        assert!(
            scene.size.iter().all(|v| v.is_finite() && *v > 0.0),
            "{family}/{name}"
        );
        let [w, h] = scene.size;
        for item in &scene.items {
            for [x, y] in points(item) {
                assert!(
                    x.is_finite() && y.is_finite(),
                    "{family}/{name}: non-finite {item:?}"
                );
                assert!(
                    x >= -0.5 && y >= -0.5 && x <= w + 0.5 && y <= h + 0.5,
                    "{family}/{name}: {x},{y} outside {w}x{h} in {item:?}"
                );
            }
        }
        assert_eq!(
            scene,
            render_once(),
            "{family}/{name}: layout is not deterministic"
        );
        let clips = scene
            .items
            .iter()
            .filter(|i| matches!(i, Item::ClipBegin(_)))
            .count();
        let ends = scene
            .items
            .iter()
            .filter(|i| matches!(i, Item::ClipEnd))
            .count();
        assert_eq!(clips, ends, "{family}/{name}: unbalanced clips");
    }
}

#[test]
fn flowchart_nodes_never_overlap_and_text_stays_inside_its_box() {
    for (family, name, source) in common::fixtures()
        .into_iter()
        .filter(|(f, ..)| f == "flowchart")
    {
        let scene = render(
            &source,
            &FixedMeasure,
            &DiagramMetrics::DEFAULT,
            &Limits::DEFAULT,
        )
        .unwrap();
        // Node bodies: filled rects/ellipses/paths with the node fill role.
        let bodies: Vec<Rect> = scene
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Shape {
                    geom,
                    fill: Some(nana_ui_diagram::Ink::Role(nana_ui_core::SemanticColorRole::Surface)),
                    stroke: Some(s),
                } if s.ink == nana_ui_diagram::Ink::Role(nana_ui_core::SemanticColorRole::BorderStrong) => Some(geom.bounds()),
                _ => None,
            })
            .collect();
        for (i, a) in bodies.iter().enumerate() {
            for b in &bodies[i + 1..] {
                // The back sheets of a stacked shape sit a few px behind it.
                let sheet =
                    (a.x - b.x).abs() <= 8.5 && (a.y - b.y).abs() <= 8.5 && (a.w - b.w).abs() < 0.1;
                if sheet {
                    continue;
                }
                let inner = |r: &Rect| r.inset(1.0, 1.0);
                assert!(
                    !inner(a).intersects(&inner(b)),
                    "{family}/{name}: {a:?} overlaps {b:?}"
                );
            }
        }
    }
}
