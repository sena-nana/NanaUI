use super::*;

/// What an unmasked node builds.
fn build_ops(ops: &[PaintOp]) -> Vec<BuiltOp> {
    build_masked(ops, false)
}

fn build_masked(ops: &[PaintOp], masked: bool) -> Vec<BuiltOp> {
    super::build_ops(ops, masked, &mut MeshReuse::new(Vec::new()))
}

fn rect_path(x: f32, y: f32, width: f32, height: f32) -> PaintPath {
    // Drawn edge by edge, so it stays a path: `PaintPath::rect` would be
    // drawn as a quad and these tests are about the path geometry.
    let mut path = PaintPath::new();
    path.move_to(x, y)
        .line_to(x + width, y)
        .line_to(x + width, y + height)
        .line_to(x, y + height)
        .close();
    path
}

fn only_mesh(built: &[BuiltOp]) -> &PathMesh {
    match built {
        [BuiltOp::Mesh(mesh)] => mesh,
        other => panic!("expected one mesh, got {other:?}"),
    }
}

#[test]
fn custom_authoring_path_keeps_extended_linear_channels() {
    let color = nana_ui_core::PaintColor::LinearScRgb {
        channels: [1.25, -0.125, 0.25],
        alpha: 0.7,
    };
    let built = build_ops(&[PaintOp::FillPath {
        path: Arc::new(rect_path(0.0, 0.0, 10.0, 10.0)),
        paint: ResolvedPaint::Authoring(color),
    }]);
    assert_eq!(only_mesh(&built).paint_color, Some(color));
}

#[test]
fn custom_authoring_shadow_keeps_linear_channels_without_double_decode() {
    let color = nana_ui_core::PaintColor::LinearScRgb {
        channels: [0.2, 0.1, 0.05],
        alpha: 0.6,
    };
    let built = build_ops(&[PaintOp::Shadow {
        path: Arc::new(rect_path(0.0, 0.0, 10.0, 10.0)),
        shadow: ComponentElevation {
            color,
            offset_x: 0.0,
            offset_y: 0.0,
            blur_radius: 0.0,
            spread_radius: 0.0,
            inset: false,
        },
    }]);
    assert_eq!(only_mesh(&built).paint_color, Some(color));
}

#[test]
fn custom_authoring_icon_keeps_explicit_paint_color() {
    let color = nana_ui_core::PaintColor::LinearScRgb {
        channels: [1.2, -0.1, 0.3],
        alpha: 0.65,
    };
    let built = build_ops(&[PaintOp::Icon {
        rect: LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 16.0,
            height: 16.0,
        },
        icon: Icon::Search,
        paint: ResolvedPaint::Authoring(color),
    }]);
    let [
        BuiltOp::Icon {
            color: fallback,
            paint_color,
            ..
        },
    ] = &built[..]
    else {
        panic!("expected one icon, got {built:?}");
    };
    assert_eq!(*paint_color, Some(color));
    assert_eq!(*fallback, color.to_srgb());
}

/// Coverage the mesh interpolates at `p`, summed over every triangle that
/// holds it (overlap would show up as more than one).
fn coverage_at(mesh: &PathMesh, p: [f32; 2]) -> (f32, usize) {
    let mut total = 0.0;
    let mut hits = 0;
    for tri in mesh.indices.as_chunks::<3>().0.iter() {
        let [a, b, c] = [0, 1, 2].map(|k| mesh.vertices[tri[k] as usize]);
        let area = cross(a.position, b.position, c.position);
        if area.abs() < 1e-9 {
            continue;
        }
        let wa = cross(b.position, c.position, p) / area;
        let wb = cross(c.position, a.position, p) / area;
        let wc = 1.0 - wa - wb;
        if wa >= -1e-5 && wb >= -1e-5 && wc >= -1e-5 {
            total += wa * a.coverage + wb * b.coverage + wc * c.coverage;
            hits += 1;
        }
    }
    (total, hits)
}

#[test]
fn a_shadow_ramps_with_distance_to_the_outline_and_never_overlaps() {
    let built = build_ops(&[PaintOp::Shadow {
        path: Arc::new(rect_path(0.0, 0.0, 100.0, 60.0)),
        shadow: ComponentElevation {
            color: nana_ui_core::PaintColor::Srgb {
                rgba: [0.0, 0.0, 0.0, 0.5],
            },
            offset_x: 0.0,
            offset_y: 6.0,
            blur_radius: 12.0,
            spread_radius: 0.0,
            inset: false,
        },
    }]);
    let mesh = only_mesh(&built);
    // Below the bottom edge (y = 66 after the offset), and left of the left
    // edge (x = 0), the ramp is the same: 0.5 on the edge, 0 at `blur`.
    for (label, at) in [
        ("bottom", |d: f32| [50.0, 66.0 + d]),
        ("left", |d: f32| [-d, 30.0]),
    ] as [(&str, fn(f32) -> [f32; 2]); 2]
    {
        for d in [-9.0, -4.0, 0.0, 4.0, 9.0] {
            let (coverage, hits) = coverage_at(mesh, at(d));
            assert_eq!(hits, 1, "{label} d={d}: one triangle, no overlap");
            let expected = (12.0 - d) / 24.0;
            assert!(
                (coverage - expected).abs() < 0.05,
                "{label} d={d}: coverage {coverage}, expected {expected}"
            );
        }
    }
}

#[test]
fn every_outline_vertex_of_a_fill_moves_in_with_its_inner_ring() {
    // A frame whose points f32 cannot hold exactly, around a hole.
    let shapes = simplify_contours(
        vec![
            vec![[0.1, 0.1], [40.3, 0.7], [41.9, 30.1], [0.3, 29.7]],
            vec![[10.1, 10.3], [20.7, 20.9], [30.3, 10.1]],
        ],
        FillRule::EvenOdd,
    );
    let mut mesh = MeshBuilder::default();
    mesh.fill(&shapes, [1.0; 4]);
    let solid: Vec<_> = mesh
        .vertices
        .iter()
        .filter(|vertex| vertex.coverage == 1.0)
        .collect();
    for vertex in &solid {
        assert_ne!(
            vertex.extrude,
            [0.0, 0.0],
            "{:?} stays put",
            vertex.position
        );
        for other in solid.iter().filter(|o| o.position == vertex.position) {
            assert_eq!(other.extrude, vertex.extrude, "{:?}", vertex.position);
        }
    }
}

#[test]
fn a_solid_rounded_rectangle_fills_and_strokes_as_a_quad() {
    let mut card = PaintPath::new();
    card.rounded_rect(
        LayoutBox {
            x: 10.0,
            y: 10.0,
            width: 80.0,
            height: 40.0,
        },
        [6.0, 6.0, 0.0, 0.0],
    );
    let card = Arc::new(card);
    let solid = ResolvedPaint::Solid([1.0, 0.0, 0.0, 1.0]);
    let stroke = |join| {
        let mut stroke = StrokeStyle::new(2.0);
        stroke.join = join;
        stroke
    };
    let built = build_ops(&[
        PaintOp::FillPath {
            path: Arc::clone(&card),
            paint: solid.clone(),
        },
        PaintOp::StrokePath {
            path: Arc::clone(&card),
            stroke: stroke(LineJoin::Miter),
            paint: solid.clone(),
        },
        PaintOp::StrokePath {
            path: Arc::clone(&card),
            stroke: stroke(LineJoin::Round),
            paint: solid.clone(),
        },
    ]);
    let [
        BuiltOp::Quad {
            rect: fill_rect,
            radii: fill_radii,
            fill: Some(_),
            border: None,
            ..
        },
        BuiltOp::Quad {
            rect: miter_rect,
            radii: miter_radii,
            fill: None,
            border: Some((_, 2.0)),
            ..
        },
        BuiltOp::Quad {
            radii: round_radii, ..
        },
    ] = &built[..]
    else {
        panic!("{built:?}");
    };
    assert_eq!((fill_rect.x, fill_rect.width), (10.0, 80.0));
    assert_eq!(*fill_radii, [6.0, 6.0, 0.0, 0.0]);
    // Half the width either side of the outline; arcs grow by half, a
    // square corner stays square under a miter and rounds under a round
    // join.
    assert_eq!((miter_rect.x, miter_rect.width), (9.0, 82.0));
    assert_eq!(*miter_radii, [7.0, 7.0, 0.0, 0.0]);
    assert_eq!(*round_radii, [7.0, 7.0, 1.0, 1.0]);

    // What a quad border cannot draw stays a path: a dash, a bevelled
    // square corner, a gradient, anything not a rectangle.
    let mut dashed = StrokeStyle::new(2.0).dash(vec![4.0, 2.0], 0.0);
    dashed.join = LineJoin::Miter;
    let mut circle = PaintPath::new();
    circle.ellipse(LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 10.0,
        height: 10.0,
    });
    let built = build_ops(&[
        PaintOp::StrokePath {
            path: Arc::clone(&card),
            stroke: dashed,
            paint: solid.clone(),
        },
        PaintOp::StrokePath {
            path: Arc::clone(&card),
            stroke: stroke(LineJoin::Bevel),
            paint: solid.clone(),
        },
        PaintOp::FillPath {
            path: Arc::new(circle),
            paint: solid,
        },
    ]);
    assert!(
        built.iter().all(|op| matches!(op, BuiltOp::Mesh(_))),
        "{built:?}"
    );
}

#[test]
fn non_finite_or_far_off_input_draws_nothing_instead_of_hanging() {
    let mut far = PaintPath::new();
    far.move_to(0.0, 0.0)
        .cubic_to(1e30, 0.0, 0.0, 1e30, 1e30, 1e30)
        .close();
    let mut nan = PaintPath::new();
    nan.move_to(0.0, 0.0)
        .line_to(f32::NAN, 5.0)
        .line_to(5.0, 5.0)
        .close();
    let square = Arc::new(rect_path(0.0, 0.0, 10.0, 10.0));
    let shadow = |blur_radius: f32| ComponentElevation {
        color: nana_ui_core::PaintColor::Srgb {
            rgba: [0.0, 0.0, 0.0, 1.0],
        },
        offset_x: 0.0,
        offset_y: 0.0,
        blur_radius,
        spread_radius: 0.0,
        inset: false,
    };
    let built = build_ops(&[
        PaintOp::FillPath {
            path: Arc::new(far.clone()),
            paint: ResolvedPaint::Solid([1.0; 4]),
        },
        PaintOp::FillPath {
            path: Arc::new(nan),
            paint: ResolvedPaint::Solid([1.0; 4]),
        },
        PaintOp::StrokePath {
            path: Arc::new(far),
            stroke: StrokeStyle::new(2.0),
            paint: ResolvedPaint::Solid([1.0; 4]),
        },
        PaintOp::StrokePath {
            path: Arc::clone(&square),
            stroke: StrokeStyle::new(f32::INFINITY),
            paint: ResolvedPaint::Solid([1.0; 4]),
        },
        PaintOp::Shadow {
            path: Arc::clone(&square),
            shadow: shadow(f32::INFINITY),
        },
        // A huge but finite transform on a small path.
        PaintOp::SetTransform([1e12, 0.0, 0.0, 1e12, 0.0, 0.0]),
        PaintOp::FillPath {
            path: square,
            paint: ResolvedPaint::Solid([1.0; 4]),
        },
    ]);
    assert!(built.is_empty(), "{} ops", built.len());
}

#[test]
fn an_inset_shadow_covers_every_point_of_a_corner_once() {
    let built = build_ops(&[PaintOp::Shadow {
        path: Arc::new(rect_path(0.0, 0.0, 100.0, 60.0)),
        shadow: ComponentElevation {
            color: nana_ui_core::PaintColor::Srgb {
                rgba: [0.0, 0.0, 0.0, 1.0],
            },
            offset_x: 20.0,
            offset_y: 20.0,
            blur_radius: 8.0,
            spread_radius: 0.0,
            inset: true,
        },
    }]);
    let mesh = only_mesh(&built);
    // The offset puts the band's mitred corner (12, 12) inside the
    // outline, where the solid region past it has to meet it without
    // overlap. Sample at offsets no triangle edge runs through: a point
    // on a shared edge counts for both triangles.
    for y in 0..30 {
        for x in 0..30 {
            let p = [
                x as f32 + std::f32::consts::FRAC_1_SQRT_2,
                y as f32 + std::f32::consts::FRAC_1_PI,
            ];
            let (coverage, hits) = coverage_at(mesh, p);
            assert!(
                hits <= 1 && coverage <= 1.0001,
                "coverage {coverage} at {p:?} ({hits} triangles)"
            );
        }
    }
}

#[test]
fn a_shape_thinner_than_its_blur_casts_a_faint_shadow_without_overlap() {
    let circle = |r: f32| LayoutBox {
        x: 50.0 - r,
        y: 50.0 - r,
        width: r * 2.0,
        height: r * 2.0,
    };
    let mut ring = PaintPath::new();
    ring.ellipse(circle(50.0)).ellipse(circle(48.0));
    let ring = ring.with_fill_rule(FillRule::EvenOdd);
    let shadow = ComponentElevation {
        color: nana_ui_core::PaintColor::Srgb {
            rgba: [0.0, 0.0, 0.0, 1.0],
        },
        offset_x: 0.0,
        offset_y: 0.0,
        blur_radius: 10.0,
        spread_radius: 0.0,
        inset: false,
    };
    let built = build_ops(&[PaintOp::Shadow {
        path: Arc::new(ring),
        shadow,
    }]);
    let mesh = only_mesh(&built);
    // A 2px ring under a 10px blur peaks near erf(4 / (√2·10)) ≈ 0.31
    // (at most twice the true width), not at full strength.
    let alpha = mesh.vertices[0].color[3];
    assert!(alpha < 0.4, "peak alpha {alpha}");
    for r in [40.0, 45.0, 49.0, 53.0, 58.0] {
        let (coverage, hits) = coverage_at(mesh, [50.0 + r, 50.25]);
        assert!(hits <= 1, "r={r}: {hits} triangles overlap");
        assert!(coverage * alpha < 0.4, "r={r}: {}", coverage * alpha);
    }
    // A disc well wider than its blur keeps its full strength.
    let mut disc = PaintPath::new();
    disc.ellipse(circle(50.0));
    let built = build_ops(&[PaintOp::Shadow {
        path: Arc::new(disc),
        shadow,
    }]);
    assert!(only_mesh(&built).vertices[0].color[3] > 0.99);
}

#[test]
fn a_corner_tighter_than_the_blur_does_not_fold_the_band() {
    let mut path = PaintPath::new();
    path.rounded_rect(
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 60.0,
        },
        [6.0; 4],
    );
    let built = build_ops(&[PaintOp::Shadow {
        path: Arc::new(path),
        shadow: ComponentElevation {
            color: nana_ui_core::PaintColor::Srgb {
                rgba: [0.0, 0.0, 0.0, 0.5],
            },
            offset_x: 0.0,
            offset_y: 0.0,
            blur_radius: 16.0,
            spread_radius: 0.0,
            inset: false,
        },
    }]);
    let mesh = only_mesh(&built);
    // Every point in a grid around the top-left corner is covered once at
    // most, and coverage never rises going outwards along the diagonal.
    for y in -14..14 {
        for x in -14..14 {
            let p = [x as f32 + 0.25, y as f32 + 0.25];
            assert!(coverage_at(mesh, p).1 <= 1, "overlap at {p:?}");
        }
    }
    let mut last = f32::INFINITY;
    for step in 0..30 {
        let t = 14.0 - step as f32;
        let (coverage, _) = coverage_at(mesh, [t, t]);
        assert!(coverage <= last + 1e-4, "coverage rose at {t}");
        last = coverage;
    }
}

#[test]
fn a_shadow_band_spans_blur_either_side_of_the_offset_outline() {
    let built = build_ops(&[PaintOp::Shadow {
        path: Arc::new(rect_path(0.0, 0.0, 100.0, 60.0)),
        shadow: ComponentElevation {
            color: nana_ui_core::PaintColor::Srgb {
                rgba: [0.0, 0.0, 0.0, 0.5],
            },
            offset_x: 0.0,
            offset_y: 6.0,
            blur_radius: 12.0,
            spread_radius: 0.0,
            inset: false,
        },
    }]);
    let mesh = only_mesh(&built);
    let b = mesh.bounds;
    assert!((b.x + 12.0).abs() < 0.6, "{b:?}");
    assert!((b.y - (6.0 - 12.0)).abs() < 0.6, "{b:?}");
    assert!((b.width - 124.0).abs() < 1.2, "{b:?}");
    assert!((b.height - 84.0).abs() < 1.2, "{b:?}");
}

#[test]
fn dashes_follow_the_pattern_and_its_offset() {
    let line: Contour = vec![[0.0, 0.0], [40.0, 0.0]];
    let spans = |pieces: Vec<Contour>| -> Vec<(f64, f64)> {
        pieces
            .iter()
            .map(|piece| (piece[0][0], piece[piece.len() - 1][0]))
            .collect()
    };
    assert_eq!(
        spans(dashes(&line, false, &[10.0, 5.0], 0.0)),
        [(0.0, 10.0), (15.0, 25.0), (30.0, 40.0)]
    );
    assert_eq!(
        spans(dashes(&line, false, &[10.0, 5.0], 12.0)),
        [(3.0, 13.0), (18.0, 28.0), (33.0, 40.0)]
    );
    // A dash runs round a corner as one piece.
    let corner: Contour = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 20.0]];
    let pieces = dashes(&corner, false, &[10.0, 10.0], 0.0);
    assert_eq!(pieces[0], vec![[0.0, 0.0], [4.0, 0.0], [4.0, 6.0]]);
    assert_eq!(dash_pattern(&[3.0]), Some(vec![3.0, 3.0]));
    assert_eq!(dash_pattern(&[3.0, -1.0]), None);
    assert_eq!(dash_pattern(&[0.0, 0.0]), None);
}

#[test]
fn a_dashed_stroke_leaves_its_gaps_empty() {
    let mut line = PaintPath::new();
    line.move_to(0.0, 10.0).line_to(40.0, 10.0);
    let built = build_ops(&[PaintOp::StrokePath {
        path: Arc::new(line),
        paint: ResolvedPaint::Solid([1.0; 4]),
        stroke: StrokeStyle::new(4.0).dash(vec![10.0, 5.0], 0.0),
    }]);
    let mesh = only_mesh(&built);
    // Off the triangles' shared diagonals, so a hit counts once.
    assert_eq!(coverage_at(mesh, [5.3, 10.7]).1, 1);
    assert_eq!(coverage_at(mesh, [12.5, 10.7]).1, 0, "the first gap");
    assert_eq!(coverage_at(mesh, [20.3, 10.7]).1, 1);
}

#[test]
fn geometry_is_laid_out_under_the_local_transform() {
    let scale = [2.0, 0.0, 0.0, 2.0, 10.0, 0.0];
    let mut line = PaintPath::new();
    line.move_to(0.0, 10.0).line_to(20.0, 10.0);
    let built = build_ops(&[
        PaintOp::SetTransform(scale),
        PaintOp::FillPath {
            path: Arc::new(rect_path(0.0, 0.0, 5.0, 5.0)),
            paint: ResolvedPaint::Solid([1.0; 4]),
        },
        PaintOp::StrokePath {
            path: Arc::new(line),
            paint: ResolvedPaint::Solid([1.0; 4]),
            stroke: StrokeStyle::new(2.0),
        },
    ]);
    let [BuiltOp::Mesh(fill), BuiltOp::Mesh(stroke)] = &built[..] else {
        panic!("{built:?}");
    };
    let b = fill.bounds;
    assert_eq!((b.x, b.y, b.width, b.height), (10.0, 0.0, 10.0, 10.0));
    // The stroke is laid out first and scaled with the path: 2px becomes 4.
    let b = stroke.bounds;
    assert!(
        (b.height - 4.0).abs() < 1e-3 && (b.y - 18.0).abs() < 1e-3,
        "{b:?}"
    );
    assert!(
        (b.x - 10.0).abs() < 1e-3 && (b.width - 40.0).abs() < 1e-3,
        "{b:?}"
    );
}

#[test]
fn a_gradient_mesh_remembers_where_its_vertices_came_from() {
    let gradient = Arc::new(ResolvedGradient {
        shape: nana_ui_runtime::GradientShape::Linear {
            start: [0.0, 0.0],
            end: [10.0, 0.0],
        },
        stops: vec![(0.0, [1.0, 0.0, 0.0, 1.0]), (1.0, [0.0, 0.0, 1.0, 1.0])],
        linear_stops: None,
        extend: Default::default(),
    });
    let built = build_ops(&[
        PaintOp::SetTransform([1.0, 0.0, 0.0, 1.0, 30.0, 40.0]),
        PaintOp::FillPath {
            path: Arc::new(rect_path(0.0, 0.0, 10.0, 10.0)),
            paint: ResolvedPaint::Gradient(Arc::clone(&gradient)),
        },
    ]);
    let mesh = only_mesh(&built);
    assert!(Arc::ptr_eq(mesh.gradient.as_ref().unwrap(), &gradient));
    for vertex in &mesh.vertices {
        assert_eq!(
            vertex.paint_pos,
            [vertex.position[0] - 30.0, vertex.position[1] - 40.0]
        );
        assert_eq!(vertex.color, [1.0, 1.0, 1.0, 1.0], "only a tint");
    }
}

fn text_op(paint: ResolvedPaint) -> PaintOp {
    PaintOp::Text {
        rect: LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 16.0,
        },
        content: Arc::from("label"),
        paint,
        size: 13.0,
        weight: None,
        family: None,
        italic: false,
        line_height: None,
        wrap: false,
        max_lines: None,
        horizontal: Default::default(),
        vertical: Default::default(),
    }
}

fn circle(cx: f32, cy: f32, r: f32) -> PaintPath {
    let mut path = PaintPath::new();
    path.ellipse(LayoutBox {
        x: cx - r,
        y: cy - r,
        width: r * 2.0,
        height: r * 2.0,
    });
    path
}

fn kinds(built: &[BuiltOp]) -> Vec<&'static str> {
    built
        .iter()
        .map(|op| match op {
            BuiltOp::Mesh(_) => "mesh",
            BuiltOp::Quad { .. } => "quad",
            BuiltOp::QuadRun { .. } => "quad-run",
            BuiltOp::Image { .. } => "image",
            BuiltOp::Text { .. } => "text",
            BuiltOp::Icon { .. } => "icon",
            BuiltOp::Default { .. } => "default",
            BuiltOp::LayerBegin { .. } => "begin",
            BuiltOp::LayerEnd { mask: None } => "end",
            BuiltOp::LayerEnd {
                mask: Some((_, LayerMaskMode::Erase)),
            } => "end-erase",
            BuiltOp::LayerEnd {
                mask: Some((_, LayerMaskMode::Tint)),
            } => "end-tint",
            BuiltOp::LayerEnd {
                mask: Some((_, LayerMaskMode::Keep)),
            } => "end-keep",
        })
        .collect()
}

#[test]
fn opacity_scales_each_op_it_governs() {
    let built = build_ops(&[
        PaintOp::SetOpacity(0.5),
        PaintOp::FillPath {
            path: Arc::new(rect_path(0.0, 0.0, 10.0, 10.0)),
            paint: ResolvedPaint::Solid([1.0, 0.0, 0.0, 0.8]),
        },
        PaintOp::RoundedRect {
            rect: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            radii: [2.0; 4],
            fill: Some(ResolvedPaint::Solid([1.0; 4])),
            border: None,
            border_paint: None,
            shadow: None,
        },
        text_op(ResolvedPaint::Solid([1.0; 4])),
        PaintOp::DrawDefault,
    ]);
    let [
        BuiltOp::Mesh(mesh),
        BuiltOp::Quad { fill, .. },
        BuiltOp::Text { color, .. },
        BuiltOp::Default { opacity, .. },
    ] = &built[..]
    else {
        panic!("{built:?}");
    };
    assert!(
        mesh.vertices
            .iter()
            .all(|v| (v.color[3] - 0.4).abs() < 1e-6)
    );
    assert_eq!(fill.unwrap()[3], 0.5);
    assert_eq!(color[3], 0.5);
    assert_eq!(*opacity, 0.5);
}

#[test]
fn a_gradient_on_glyphs_tints_them_in_a_layer() {
    let gradient = Arc::new(ResolvedGradient {
        shape: nana_ui_runtime::GradientShape::Linear {
            start: [0.0, 0.0],
            end: [40.0, 0.0],
        },
        stops: vec![(0.0, [1.0, 0.0, 0.0, 1.0]), (1.0, [0.0, 0.0, 1.0, 1.0])],
        linear_stops: None,
        extend: Default::default(),
    });
    let built = build_ops(&[text_op(ResolvedPaint::Gradient(gradient))]);
    assert_eq!(kinds(&built), ["begin", "text", "end-tint"]);
    let BuiltOp::Text { color, .. } = &built[1] else {
        unreachable!()
    };
    assert_eq!(*color, [1.0; 4], "the glyphs are only coverage");
}

#[test]
fn a_clip_the_gpu_cannot_express_masks_text_in_a_layer() {
    let clipped = |clip: PaintPath| {
        build_ops(&[
            PaintOp::PushClip {
                path: Arc::new(clip),
            },
            text_op(ResolvedPaint::Solid([1.0; 4])),
            PaintOp::DrawDefault,
            PaintOp::PopClip,
        ])
    };
    // A rectangle is exact on the GPU: no layer.
    let exact = clipped(rect_path(0.0, 0.0, 20.0, 20.0));
    assert_eq!(kinds(&exact), ["text", "default"]);
    let BuiltOp::Text { clips, .. } = &exact[0] else {
        unreachable!()
    };
    assert_eq!(clips.len(), 1);
    // A circle is not: the text is drawn in a layer cut down to it.
    let masked = clipped(circle(20.0, 20.0, 15.0));
    assert_eq!(
        kinds(&masked),
        [
            "begin",
            "text",
            "end-erase",
            "begin",
            "default",
            "end-erase"
        ]
    );
    let BuiltOp::LayerBegin {
        clip: Some(area), ..
    } = &masked[0]
    else {
        unreachable!()
    };
    assert!(
        (area.x - 4.0).abs() < 0.1 && (area.width - 32.0).abs() < 0.2,
        "{area:?}"
    );
    let BuiltOp::LayerEnd {
        mask: Some((erase, _)),
    } = &masked[2]
    else {
        unreachable!()
    };
    // The erase covers the corner of the area and leaves the centre.
    assert_eq!(coverage_at(erase, [5.0, 5.0]).1, 1);
    assert_eq!(coverage_at(erase, [20.3, 20.7]).1, 0);
}

#[test]
fn an_image_fills_a_path_through_a_masked_layer() {
    let built = build_ops(&[PaintOp::FillImage {
        path: Arc::new(circle(20.0, 20.0, 10.0)),
        rect: LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 40.0,
        },
        source: Arc::from("data:image/png;base64,"),
        fit: ImageFit::Cover,
        sampling: nana_ui_core::ImageSampling::Resample,
    }]);
    assert_eq!(kinds(&built), ["begin", "image", "end-erase"]);
}

#[test]
fn an_inset_shadow_stays_inside_and_darkens_toward_the_edge() {
    let built = build_ops(&[PaintOp::Shadow {
        path: Arc::new(rect_path(0.0, 0.0, 100.0, 60.0)),
        shadow: ComponentElevation {
            color: nana_ui_core::PaintColor::Srgb {
                rgba: [0.0, 0.0, 0.0, 0.5],
            },
            offset_x: 0.0,
            offset_y: 0.0,
            blur_radius: 8.0,
            spread_radius: 0.0,
            inset: true,
        },
    }]);
    let mesh = only_mesh(&built);
    let b = mesh.bounds;
    assert!(
        b.x >= -1e-3 && b.y >= -1e-3 && b.x + b.width <= 100.001,
        "{b:?}"
    );
    let at = |p| coverage_at(mesh, p).0;
    assert!(at([1.3, 30.7]) > at([6.3, 30.7]), "darker at the edge");
    assert!(at([50.3, 30.7]) < 0.05, "clear in the middle");
}

#[test]
fn a_masked_node_draws_no_quads_and_keeps_its_mask_on_glyphs_and_images() {
    let card = Arc::new({
        let mut card = PaintPath::new();
        card.rect(LayoutBox {
            x: 10.0,
            y: 10.0,
            width: 80.0,
            height: 40.0,
        });
        card
    });
    let solid = ResolvedPaint::Solid([1.0, 0.0, 0.0, 1.0]);
    let ops = [
        PaintOp::FillPath {
            path: Arc::clone(&card),
            paint: solid.clone(),
        },
        PaintOp::StrokePath {
            path: Arc::clone(&card),
            paint: solid.clone(),
            stroke: StrokeStyle::new(2.0),
        },
        PaintOp::RoundedRect {
            rect: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 20.0,
            },
            radii: [4.0; 4],
            fill: Some(solid.clone()),
            border: None,
            border_paint: None,
            shadow: Some(ComponentElevation {
                color: nana_ui_core::PaintColor::Srgb {
                    rgba: [0.0, 0.0, 0.0, 0.5],
                },
                offset_x: 0.0,
                offset_y: 2.0,
                blur_radius: 6.0,
                spread_radius: 0.0,
                inset: false,
            }),
        },
        text_op(solid.clone()),
        PaintOp::Image {
            rect: LayoutBox {
                x: 50.0,
                y: 0.0,
                width: 20.0,
                height: 20.0,
            },
            source: Arc::from("a.png"),
            fit: ImageFit::Fill,
            sampling: nana_ui_core::ImageSampling::Resample,
            radii: [0.0; 4],
        },
    ];
    assert_eq!(
        kinds(&build_masked(&ops, false)),
        ["quad", "quad", "quad", "text", "image"]
    );
    // The quad shader would evaluate the mask over each quad's own box:
    // every shape is a mesh instead, glyphs and images a kept layer.
    let masked = build_masked(&ops, true);
    assert_eq!(
        kinds(&masked),
        [
            "mesh", "mesh", "mesh", "mesh", "begin", "text", "end-keep", "begin", "image",
            "end-keep"
        ]
    );
    // Each layer holds what it keeps: glyphs reach half their size past
    // the text box.
    let BuiltOp::LayerBegin {
        clip: Some(area), ..
    } = &masked[4]
    else {
        panic!("{:?}", masked[4]);
    };
    assert_eq!(
        (area.x, area.y, area.width, area.height),
        (-6.5, -6.5, 53.0, 29.0)
    );
    let BuiltOp::LayerEnd {
        mask: Some((mesh, LayerMaskMode::Keep)),
    } = &masked[6]
    else {
        panic!("{:?}", masked[6]);
    };
    assert!((mesh.bounds.x - area.x).abs() < 1.0 && (mesh.bounds.width - area.width).abs() < 2.0);
}

fn boxed(x: f32, radius: f32, fill: Option<ResolvedPaint>) -> PaintOp {
    boxed_with(x, radius, fill, None, None)
}

fn boxed_with(
    x: f32,
    radius: f32,
    fill: Option<ResolvedPaint>,
    border: Option<([f32; 4], f32)>,
    shadow: Option<ComponentElevation>,
) -> PaintOp {
    PaintOp::RoundedRect {
        rect: LayoutBox {
            x,
            y: 0.0,
            width: 8.0,
            height: 8.0,
        },
        radii: [radius; 4],
        fill,
        border,
        border_paint: None,
        shadow,
    }
}

#[test]
fn adjacent_solid_boxes_share_one_run_in_order() {
    let red = Some(ResolvedPaint::Solid([1.0, 0.0, 0.0, 1.0]));
    let blue = Some(ResolvedPaint::Solid([0.0, 0.0, 1.0, 0.5]));
    let built = build_ops(&[
        boxed(0.0, 2.0, red.clone()),
        boxed(10.0, 2.0, blue.clone()),
        boxed(20.0, 2.0, red.clone()),
    ]);
    let [BuiltOp::QuadRun { items, radii, .. }] = &built[..] else {
        panic!("expected one run, got {:?}", kinds(&built));
    };
    assert_eq!(*radii, [2.0; 4]);
    let xs: Vec<f32> = items.iter().map(|(rect, _)| rect.x).collect();
    assert_eq!(xs, [0.0, 10.0, 20.0], "paint order is kept");
    assert_eq!(items[1].1, [0.0, 0.0, 1.0, 0.5]);

    // A path between them, or other corners, starts a new stretch; a lone
    // box stays a quad.
    let built = build_ops(&[
        boxed(0.0, 2.0, red.clone()),
        boxed(10.0, 2.0, red.clone()),
        PaintOp::FillPath {
            path: Arc::new(rect_path(0.0, 0.0, 4.0, 4.0)),
            paint: ResolvedPaint::Solid([0.0, 1.0, 0.0, 1.0]),
        },
        boxed(20.0, 2.0, red.clone()),
        boxed(30.0, 3.0, red.clone()),
        boxed(40.0, 3.0, red.clone()),
    ]);
    assert_eq!(kinds(&built), ["quad-run", "mesh", "quad", "quad-run"]);
}

#[test]
fn boxes_with_more_than_a_solid_fill_keep_their_own_quads() {
    let red = Some(ResolvedPaint::Solid([1.0, 0.0, 0.0, 1.0]));
    let bordered = boxed_with(
        10.0,
        2.0,
        red.clone(),
        Some(([0.0, 0.0, 0.0, 1.0], 1.0)),
        None,
    );
    let shadowed = boxed_with(
        20.0,
        2.0,
        red.clone(),
        None,
        Some(ComponentElevation {
            color: nana_ui_core::PaintColor::srgb([0.0, 0.0, 0.0, 0.5]),
            offset_x: 0.0,
            offset_y: 2.0,
            blur_radius: 4.0,
            spread_radius: 0.0,
            inset: false,
        }),
    );
    let authored = boxed(
        30.0,
        2.0,
        Some(ResolvedPaint::Authoring(nana_ui_core::PaintColor::Oklch {
            l: 0.6,
            c: 0.1,
            h: Some(30.0),
            alpha: 1.0,
        })),
    );
    let outline_only = boxed(40.0, 2.0, None);
    for op in [bordered, shadowed, authored, outline_only] {
        let built = build_ops(&[boxed(0.0, 2.0, red.clone()), op.clone()]);
        assert_eq!(kinds(&built), ["quad", "quad"], "{op:?}");
    }
}

/// A small canvas: a fill, a stroke, a shadow, and a fill under a path
/// clip. `moved` shifts the stroke; `clip_r` resizes the clip.
fn canvas(color: [f32; 4], moved: f32, clip_r: f32) -> Vec<PaintOp> {
    let solid = ResolvedPaint::Solid(color);
    vec![
        PaintOp::FillPath {
            path: Arc::new(circle(20.0, 20.0, 12.0)),
            paint: solid.clone(),
        },
        PaintOp::StrokePath {
            path: Arc::new(circle(60.0 + moved, 20.0, 12.0)),
            paint: solid.clone(),
            stroke: StrokeStyle::new(3.0),
        },
        PaintOp::Shadow {
            path: Arc::new(circle(20.0, 60.0, 10.0)),
            shadow: ComponentElevation {
                color: nana_ui_core::PaintColor::srgb([0.0, 0.0, 0.0, 0.4]),
                offset_x: 0.0,
                offset_y: 2.0,
                blur_radius: 4.0,
                spread_radius: 0.0,
                inset: false,
            },
        },
        PaintOp::PushClip {
            path: Arc::new(circle(60.0, 60.0, clip_r)),
        },
        PaintOp::FillPath {
            path: Arc::new(rect_path(40.0, 40.0, 30.0, 30.0)),
            paint: solid,
        },
        PaintOp::PopClip,
    ]
}

fn meshes(built: &[BuiltOp]) -> Vec<Arc<PathMesh>> {
    built
        .iter()
        .filter_map(|op| match op {
            BuiltOp::Mesh(mesh) => Some(Arc::clone(mesh)),
            _ => None,
        })
        .collect()
}

/// Build `ops` starting from `previous`: the meshes, what the next
/// recording would start from, and how many meshes were built.
fn rebuild(
    ops: &[PaintOp],
    previous: Option<&MeshCache>,
) -> (Vec<Arc<PathMesh>>, MeshCache, usize) {
    let mut reuse = MeshReuse::new(previous.into_iter().collect());
    let built = super::build_ops(ops, false, &mut reuse);
    let count = reuse.built;
    (meshes(&built), reuse.finish(), count)
}

#[test]
fn rerecording_unchanged_ops_builds_no_mesh_and_a_changed_op_only_its_own() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let (first, cache, built) = rebuild(&canvas(red, 0.0, 15.0), None);
    assert_eq!(first.len(), 4);
    assert_eq!(built, 4);

    // The same ops, recorded again: every mesh is the one already built.
    let (second, cache, built) = rebuild(&canvas(red, 0.0, 15.0), Some(&cache));
    assert_eq!(built, 0);
    for (a, b) in first.iter().zip(&second) {
        assert!(Arc::ptr_eq(a, b));
    }

    // The stroke moved: only it is built again.
    let (third, cache, built) = rebuild(&canvas(red, 5.0, 15.0), Some(&cache));
    assert_eq!(built, 1);
    let reused: Vec<bool> = second
        .iter()
        .zip(&third)
        .map(|(a, b)| Arc::ptr_eq(a, b))
        .collect();
    assert_eq!(reused, [true, false, true, true]);
    assert_eq!(*third[1], *rebuild(&canvas(red, 5.0, 15.0), None).0[1]);

    // The clip changed: only what it cuts is built again.
    let (fourth, _, built) = rebuild(&canvas(red, 5.0, 18.0), Some(&cache));
    assert_eq!(built, 1);
    let reused: Vec<bool> = third
        .iter()
        .zip(&fourth)
        .map(|(a, b)| Arc::ptr_eq(a, b))
        .collect();
    assert_eq!(reused, [true, true, true, false]);
}

#[test]
fn a_new_colour_recolours_the_built_geometry_instead_of_rebuilding_it() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let blue = [0.0, 0.2, 1.0, 0.5];
    let (first, cache, _) = rebuild(&canvas(red, 0.0, 15.0), None);
    let (recoloured, _, built) = rebuild(&canvas(blue, 0.0, 15.0), Some(&cache));
    assert_eq!(built, 0);
    let (fresh, _, _) = rebuild(&canvas(blue, 0.0, 15.0), None);
    // Fills and strokes come out exactly as if built in the new colour;
    // the shadow, whose colour did not change, is the same mesh.
    for index in [0, 1, 3] {
        assert!(!Arc::ptr_eq(&first[index], &recoloured[index]));
        assert_eq!(*recoloured[index], *fresh[index]);
    }
    assert!(Arc::ptr_eq(&first[2], &recoloured[2]));
}

fn stroke_mesh(path: PaintPath, stroke: StrokeStyle) -> PathMesh {
    let built = build_ops(&[PaintOp::StrokePath {
        path: Arc::new(path),
        paint: ResolvedPaint::Solid([1.0, 0.2, 0.2, 1.0]),
        stroke,
    }]);
    only_mesh(&built).clone()
}

fn fill_mesh_only(path: PaintPath) -> PathMesh {
    let built = build_ops(&[PaintOp::FillPath {
        path: Arc::new(path),
        paint: ResolvedPaint::Solid([0.2, 0.4, 0.9, 1.0]),
    }]);
    only_mesh(&built).clone()
}

#[test]
fn round_caps_extend_along_horizontal_vertical_and_diagonal_strokes() {
    let mut horiz = PaintPath::new();
    horiz.move_to(20.0, 50.0).line_to(80.0, 50.0);
    let h_mesh = stroke_mesh(horiz, StrokeStyle::new(20.0).cap(LineCap::Round));
    for p in [[12.7, 50.3], [87.3, 49.7], [16.3, 43.9], [50.3, 50.7]] {
        let (cov, hits) = coverage_at(&h_mesh, p);
        assert_eq!(hits, 1, "horiz inside cap {p:?}: hits={hits} cov={cov}");
        assert!((cov - 1.0).abs() < 1e-4, "horiz {p:?}: cov={cov}");
    }
    assert_eq!(coverage_at(&h_mesh, [11.3, 42.1]).1, 0);

    let mut vert = PaintPath::new();
    vert.move_to(50.0, 20.0).line_to(50.0, 80.0);
    let v_mesh = stroke_mesh(vert, StrokeStyle::new(20.0).cap(LineCap::Round));
    for p in [[50.3, 12.7], [49.7, 87.3]] {
        let (cov, hits) = coverage_at(&v_mesh, p);
        assert_eq!(hits, 1, "vert inside cap {p:?}");
        assert!((cov - 1.0).abs() < 1e-4);
    }
    assert_eq!(coverage_at(&v_mesh, [42.1, 11.3]).1, 0);

    let mut diag = PaintPath::new();
    diag.move_to(20.0, 20.0).line_to(80.0, 80.0);
    let d_mesh = stroke_mesh(diag, StrokeStyle::new(20.0).cap(LineCap::Round));
    for p in [[14.9, 15.3], [85.1, 84.7]] {
        let (cov, hits) = coverage_at(&d_mesh, p);
        assert_eq!(hits, 1, "diag inside cap {p:?}");
        assert!((cov - 1.0).abs() < 1e-4);
    }
    assert_eq!(coverage_at(&d_mesh, [11.3, 27.1]).1, 0);
}

#[test]
fn open_stroke_joins_honor_miter_limit_bevel_round_and_avoid_inner_bowties() {
    let bend = || {
        let mut p = PaintPath::new();
        p.move_to(20.0, 50.0)
            .line_to(60.0, 50.0)
            .line_to(60.0, 90.0);
        p
    };

    let miter = stroke_mesh(
        bend(),
        StrokeStyle::new(20.0)
            .join(LineJoin::Miter)
            .miter_limit(4.0),
    );
    assert_eq!(coverage_at(&miter, [67.3, 42.9]), (1.0, 1));
    assert_eq!(coverage_at(&miter, [69.1, 40.6]), (1.0, 1));
    assert_eq!(coverage_at(&miter, [54.3, 55.9]), (1.0, 1));

    let clipped_miter = stroke_mesh(
        bend(),
        StrokeStyle::new(20.0)
            .join(LineJoin::Miter)
            .miter_limit(1.2),
    );
    assert_eq!(coverage_at(&clipped_miter, [69.1, 40.6]).1, 0);
    assert_eq!(coverage_at(&clipped_miter, [61.7, 48.1]), (1.0, 1));

    let bevel = stroke_mesh(bend(), StrokeStyle::new(20.0).join(LineJoin::Bevel));
    assert_eq!(coverage_at(&bevel, [67.3, 42.9]).1, 0);
    assert_eq!(coverage_at(&bevel, [61.7, 48.1]), (1.0, 1));

    let round = stroke_mesh(bend(), StrokeStyle::new(20.0).join(LineJoin::Round));
    assert_eq!(coverage_at(&round, [65.3, 44.3]), (1.0, 1));
    assert_eq!(coverage_at(&round, [68.9, 41.1]).1, 0);

    let mut hairpin = PaintPath::new();
    hairpin
        .move_to(20.0, 30.0)
        .line_to(70.0, 45.0)
        .line_to(20.0, 60.0);
    for join in [LineJoin::Miter, LineJoin::Bevel, LineJoin::Round] {
        let mesh = stroke_mesh(hairpin.clone(), StrokeStyle::new(24.0).join(join));
        for p in [[52.31, 45.73], [42.71, 44.37], [34.13, 45.91]] {
            let (cov, hits) = coverage_at(&mesh, p);
            assert!(cov <= 1.0 + 1e-4, "hairpin {join:?} {p:?}: cov={cov}");
            if cov > 0.5 {
                assert_eq!(hits, 1, "hairpin {join:?} {p:?} should hit once");
            }
        }
    }
}

#[test]
fn closed_stroke_honors_round_bevel_miter_joins_without_overlap() {
    let tri = || {
        let mut p = PaintPath::new();
        p.move_to(30.0, 70.0)
            .line_to(90.0, 70.0)
            .line_to(60.0, 20.0)
            .close();
        p
    };

    let miter = stroke_mesh(
        tri(),
        StrokeStyle::new(16.0)
            .join(LineJoin::Miter)
            .miter_limit(4.0),
    );
    assert_eq!(coverage_at(&miter, [60.13, 8.37]), (1.0, 1));

    let round = stroke_mesh(tri(), StrokeStyle::new(16.0).join(LineJoin::Round));
    assert_eq!(coverage_at(&round, [60.13, 8.37]).1, 0);
    assert_eq!(coverage_at(&round, [60.13, 14.17]), (1.0, 1));
    assert_eq!(coverage_at(&round, [60.13, 70.37]), (1.0, 1));

    let bevel = stroke_mesh(tri(), StrokeStyle::new(16.0).join(LineJoin::Bevel));
    assert_eq!(coverage_at(&bevel, [60.13, 14.17]).1, 0);
    assert_eq!(coverage_at(&bevel, [60.13, 70.37]), (1.0, 1));
}

#[test]
fn dashed_and_crossing_subpath_strokes_merge_without_double_hits() {
    let mut cross_path = PaintPath::new();
    cross_path
        .move_to(20.0, 50.0)
        .line_to(80.0, 50.0)
        .move_to(50.0, 20.0)
        .line_to(50.0, 80.0);
    let cross_mesh = stroke_mesh(cross_path, StrokeStyle::new(16.0));
    assert_eq!(coverage_at(&cross_mesh, [47.3, 52.7]), (1.0, 1));
    assert_eq!(coverage_at(&cross_mesh, [52.1, 47.9]), (1.0, 1));

    let mut dashed_path = PaintPath::new();
    dashed_path.move_to(10.0, 50.0).line_to(110.0, 50.0);
    let dashed_mesh = stroke_mesh(
        dashed_path,
        StrokeStyle::new(10.0).dash(vec![20.0, 10.0], 0.0),
    );
    assert_eq!(coverage_at(&dashed_mesh, [20.3, 50.7]), (1.0, 1));
    assert_eq!(coverage_at(&dashed_mesh, [35.3, 50.7]).1, 0);
    assert_eq!(coverage_at(&dashed_mesh, [50.3, 50.7]), (1.0, 1));
}

#[test]
fn bowtie_and_double_loop_fills_match_contains_under_evenodd_and_nonzero() {
    let mut bowtie = PaintPath::new();
    bowtie
        .move_to(20.0, 20.0)
        .line_to(80.0, 80.0)
        .line_to(80.0, 20.0)
        .line_to(20.0, 80.0)
        .close();

    for rule in [FillRule::EvenOdd, FillRule::NonZero] {
        let path = bowtie.clone().with_fill_rule(rule);
        let mesh = fill_mesh_only(path.clone());
        for p in [[31.3, 50.7], [68.7, 49.3], [50.3, 28.7], [49.7, 71.3]] {
            let want = path.contains(p);
            let (cov, hits) = coverage_at(&mesh, p);
            assert_eq!(cov > 0.5, want, "bowtie {rule:?} {p:?}");
            assert_eq!(hits, usize::from(want), "bowtie {rule:?} {p:?} hits");
        }
    }

    let mut concentric = PaintPath::new();
    concentric
        .move_to(10.0, 10.0)
        .line_to(90.0, 10.0)
        .line_to(90.0, 90.0)
        .line_to(10.0, 90.0)
        .close()
        .move_to(30.0, 30.0)
        .line_to(70.0, 30.0)
        .line_to(70.0, 70.0)
        .line_to(30.0, 70.0)
        .close();

    for rule in [FillRule::EvenOdd, FillRule::NonZero] {
        let path = concentric.clone().with_fill_rule(rule);
        let mesh = fill_mesh_only(path.clone());
        for p in [[18.3, 50.7], [51.3, 49.7]] {
            let want = path.contains(p);
            let (cov, hits) = coverage_at(&mesh, p);
            assert_eq!(cov > 0.5, want, "concentric {rule:?} {p:?}");
            assert_eq!(hits, usize::from(want), "concentric {rule:?} {p:?} hits");
        }
    }
}

#[test]
fn depth_two_island_inside_hole_fills_and_extrudes_outward() {
    let mut nested = PaintPath::new();
    nested
        .rect(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        })
        .rect(LayoutBox {
            x: 20.0,
            y: 20.0,
            width: 60.0,
            height: 60.0,
        })
        .rect(LayoutBox {
            x: 35.0,
            y: 35.0,
            width: 30.0,
            height: 30.0,
        })
        .rect(LayoutBox {
            x: 45.0,
            y: 45.0,
            width: 10.0,
            height: 10.0,
        });
    let path = nested.with_fill_rule(FillRule::EvenOdd);
    let mesh = fill_mesh_only(path.clone());

    for p in [[10.3, 50.7], [27.3, 50.7], [40.3, 50.7], [50.3, 49.7]] {
        let want = path.contains(p);
        let (cov, hits) = coverage_at(&mesh, p);
        assert_eq!(cov > 0.5, want, "nested depth point {p:?}");
        assert_eq!(hits, usize::from(want), "nested depth hits {p:?}");
    }

    // Depth-2 island corner (35, 35) must extrude outward into the moat (-x, -y).
    let island_outer_corner = mesh.vertices.iter().find(|v| {
        v.coverage == 0.0
            && (v.position[0] - 35.0).abs() < 0.05
            && (v.position[1] - 35.0).abs() < 0.05
    });
    let corner = island_outer_corner.expect("island corner AA outer vertex");
    assert!(
        corner.extrude[0] < -0.5 && corner.extrude[1] < -0.5,
        "island corner should extrude outward into surrounding hole: {:?}",
        corner.extrude
    );
}

#[test]
fn clipped_fill_path_erase_outside_and_inset_shadow_hole_parity() {
    let mut diamond = PaintPath::new();
    diamond
        .move_to(50.0, 10.0)
        .line_to(90.0, 50.0)
        .line_to(50.0, 90.0)
        .line_to(10.0, 50.0)
        .close();
    let clipped_built = build_ops(&[
        PaintOp::PushClip {
            path: Arc::new(circle(50.0, 50.0, 25.0)),
        },
        PaintOp::FillPath {
            path: Arc::new(diamond.clone()),
            paint: ResolvedPaint::Solid([0.1, 0.8, 0.3, 1.0]),
        },
        PaintOp::PopClip,
    ]);
    let clipped_mesh = only_mesh(&clipped_built);
    assert_eq!(coverage_at(clipped_mesh, [50.3, 50.7]), (1.0, 1));
    assert_eq!(coverage_at(clipped_mesh, [82.3, 50.7]).1, 0);
    let clipped_aa = clipped_mesh.vertices.iter().any(|v| {
        v.coverage == 0.0
            && (v.position[0] - 75.0).abs() < 0.3
            && (v.position[1] - 50.0).abs() < 2.0
            && v.extrude[0] > 0.5
    });
    assert!(clipped_aa, "clipped boundary must retain outward AA fringe");

    let mut hollow_diamond = diamond;
    hollow_diamond
        .move_to(50.0, 30.0)
        .line_to(70.0, 50.0)
        .line_to(50.0, 70.0)
        .line_to(30.0, 50.0)
        .close();
    let hollow_diamond = hollow_diamond.with_fill_rule(FillRule::EvenOdd);
    let masked = build_ops(&[
        PaintOp::PushClip {
            path: Arc::new(hollow_diamond),
        },
        text_op(ResolvedPaint::Solid([1.0; 4])),
        PaintOp::PopClip,
    ]);
    let BuiltOp::LayerEnd {
        mask: Some((erase, _)),
    } = &masked[2]
    else {
        panic!("expected masked layer erase: {masked:?}");
    };
    assert_eq!(coverage_at(erase, [14.3, 14.7]), (1.0, 1));
    assert_eq!(coverage_at(erase, [22.3, 50.7]).1, 0);
    assert_eq!(coverage_at(erase, [50.3, 49.7]), (1.0, 1));

    let mut hollow_box = PaintPath::new();
    hollow_box
        .rect(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        })
        .rect(LayoutBox {
            x: 35.0,
            y: 35.0,
            width: 30.0,
            height: 30.0,
        });
    let shadow_built = build_ops(&[PaintOp::Shadow {
        path: Arc::new(hollow_box.with_fill_rule(FillRule::EvenOdd)),
        shadow: ComponentElevation {
            color: nana_ui_core::PaintColor::Srgb {
                rgba: [0.0, 0.0, 0.0, 0.5],
            },
            offset_x: 0.0,
            offset_y: 0.0,
            blur_radius: 6.0,
            spread_radius: 0.0,
            inset: true,
        },
    }]);
    let shadow_mesh = only_mesh(&shadow_built);
    assert_eq!(coverage_at(shadow_mesh, [50.3, 49.7]).1, 0);
    let (edge_cov, edge_hits) = coverage_at(shadow_mesh, [2.3, 50.7]);
    assert_eq!(edge_hits, 1);
    assert!(edge_cov > 0.3, "edge inset coverage: {edge_cov}");
}

fn concave_l_path() -> PaintPath {
    let mut l_shape = PaintPath::new();
    l_shape
        .move_to(10.0, 10.0)
        .line_to(90.0, 10.0)
        .line_to(90.0, 35.0)
        .line_to(40.0, 35.0)
        .line_to(40.0, 90.0)
        .line_to(10.0, 90.0)
        .close();
    l_shape
}

#[test]
fn concave_clip_erase_outside_spread_shadow_and_recording_contains_stay_consistent() {
    let mut l_holed = concave_l_path();
    l_holed
        .move_to(18.0, 18.0)
        .line_to(18.0, 28.0)
        .line_to(28.0, 28.0)
        .line_to(28.0, 18.0)
        .close();
    let l_arc = Arc::new(l_holed.with_fill_rule(FillRule::EvenOdd));
    let ops = vec![
        PaintOp::Shadow {
            path: Arc::clone(&l_arc),
            shadow: ComponentElevation {
                color: nana_ui_core::PaintColor::Srgb {
                    rgba: [0.0, 0.0, 0.0, 0.5],
                },
                offset_x: 0.0,
                offset_y: 0.0,
                blur_radius: 0.0,
                spread_radius: 4.0,
                inset: false,
            },
        },
        PaintOp::Shadow {
            path: Arc::clone(&l_arc),
            shadow: ComponentElevation {
                color: nana_ui_core::PaintColor::Srgb {
                    rgba: [0.0, 0.0, 0.0, 0.5],
                },
                offset_x: 0.0,
                offset_y: 0.0,
                blur_radius: 4.0,
                spread_radius: 4.0,
                inset: false,
            },
        },
        PaintOp::PushClip {
            path: Arc::clone(&l_arc),
        },
        PaintOp::Text {
            rect: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            content: Arc::from("clipped"),
            paint: ResolvedPaint::Solid([1.0; 4]),
            size: 13.0,
            weight: None,
            family: None,
            italic: false,
            line_height: None,
            wrap: false,
            max_lines: None,
            horizontal: Default::default(),
            vertical: Default::default(),
        },
        PaintOp::PopClip,
    ];
    let recording = PaintRecording {
        behind_children: ops.clone(),
        over_children: Vec::new(),
    };
    let built = build_ops(&ops);
    let BuiltOp::Mesh(spread_sharp) = &built[0] else {
        panic!("expected sharp spread shadow mesh: {built:?}");
    };
    let BuiltOp::Mesh(spread_blurred) = &built[1] else {
        panic!("expected blurred spread shadow mesh: {built:?}");
    };
    let BuiltOp::LayerEnd {
        mask: Some((erase, _)),
    } = &built[4]
    else {
        panic!("expected masked layer erase: {built:?}");
    };

    // 1. Inside concave L arm: clipped content hits, erase mask stays clear, spread shadows don't overlap.
    assert!(recording.contains([14.3, 55.7], [100.0, 100.0]));
    assert_eq!(coverage_at(erase, [14.3, 55.7]).1, 0);
    assert!(coverage_at(spread_sharp, [14.3, 55.7]).1 <= 1);
    assert!(coverage_at(spread_blurred, [14.3, 55.7]).1 <= 1);

    // 2. Interior hole: recording does not hit, erase mask covers completely.
    assert!(!recording.contains([23.3, 23.7], [100.0, 100.0]));
    let (hole_cov, hole_hits) = coverage_at(erase, [23.3, 23.7]);
    assert_eq!(hole_hits, 1);
    assert!((hole_cov - 1.0).abs() < 1e-5, "hole erase cov={hole_cov}");

    // 3. Spread band outside L right edge (x = 92.3, y = 20.7): spread sharp covers 1 triangle.
    assert!(!recording.contains([92.3, 20.7], [100.0, 100.0]));
    let (spread_cov, spread_hits) = coverage_at(spread_sharp, [92.3, 20.7]);
    assert_eq!(spread_hits, 1);
    assert!(
        (spread_cov - 1.0).abs() < 1e-5,
        "spread sharp cov={spread_cov}"
    );
    assert!(coverage_at(spread_blurred, [92.3, 20.7]).1 <= 1);

    // 4. Concave L exterior notch (60.3, 60.7): erase mask covers, recording/shadows do not hit.
    assert!(!recording.contains([60.3, 60.7], [100.0, 100.0]));
    let (notch_cov, notch_hits) = coverage_at(erase, [60.3, 60.7]);
    assert_eq!(notch_hits, 1);
    assert!(
        (notch_cov - 1.0).abs() < 1e-5,
        "notch erase cov={notch_cov}"
    );
    assert_eq!(coverage_at(spread_sharp, [60.3, 60.7]).1, 0);
    assert_eq!(coverage_at(spread_blurred, [60.3, 60.7]).1, 0);
}
