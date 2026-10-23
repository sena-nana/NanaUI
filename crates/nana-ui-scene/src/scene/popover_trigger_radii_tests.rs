// 菜单入口的显式圆角、逐角圆角与主题默认半径绘制回归。
#[test]
fn a_menu_trigger_paints_authored_radii_and_preserves_the_theme_fallback() {
    let trigger_radii = |radius, corners| {
        let mut menu = node(1, None, &[]);
        menu.layout.width = 36.0;
        menu.layout.height = 36.0;
        menu.chrome_radii.sm = 5.0;
        let style = Arc::make_mut(&mut menu.source_style.layout);
        style.border_radius = radius;
        style.paint.border_radii = corners;
        menu.standard_visual = Some(StandardVisual::MenuSurface {
            trigger_image: None,
            open: false,
            kind: nana_ui_runtime::MenuSurfaceKind::Popover,
            trigger: None,
            trigger_icon: Some(nana_ui_core::Icon::Add),
            gap: 0.0,
            overlay: None,
            query: None,
            rows: Arc::from([]),
            highlighted: None,
        });
        menu.component_geometry = Some(Box::new(ComponentGeometry::MenuSurface {
            trigger_image: None,
            trigger_surface: Some(nana_ui_runtime::ComponentTriggerSurface {
                bounds: menu.layout,
                background: Some([0.18, 0.18, 0.2, 1.0]),
                border: None,
            }),
            trigger: None,
            trigger_icon: None,
            surface: LayoutBox::default(),
            search: None,
            search_field: None,
            options: Vec::new(),
            elevation: ComponentElevation {
                color: nana_ui_core::PaintColor::Srgb { rgba: [0.0; 4] },
                offset_x: 0.0,
                offset_y: 0.0,
                blur_radius: 0.0,
                spread_radius: 0.0,
                inset: false,
            },
            background: [0.0; 4],
            border: [0.0; 4],
        }));
        let mut scene = UiScene::new();
        scene.apply_delta([menu], []);
        match scene
            .primitive(PrimitiveId {
                node: id(1),
                slot: 1,
            })
            .expect("trigger background")
            .kind
        {
            ScenePrimitiveKind::Quad { corner_radius, .. } => corner_radius,
            ref other => panic!("trigger background is a quad, got {other:?}"),
        }
    };

    assert_eq!(trigger_radii(None, None), [5.0; 4]);
    assert_eq!(trigger_radii(Some(18.0), None), [18.0; 4]);
    assert_eq!(trigger_radii(Some(0.0), None), [0.0; 4]);
    assert_eq!(
        trigger_radii(
            Some(18.0),
            Some([2.0, 4.0, 6.0, 8.0].map(nana_ui_core::LengthSpec::Px)),
        ),
        [2.0, 4.0, 6.0, 8.0],
    );
}
