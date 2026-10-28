// 自定义触发器的布局与状态样式独立于悬挂面板。
#[test]
fn custom_trigger_style_keeps_circular_chrome_and_surface_settings() {
    let mut context = AppContext::new();
    let custom = crate::Stack::row(0.0)
        .width(LengthSpec::Px(36.0))
        .height(LengthSpec::Px(36.0))
        .surface(SemanticColorRole::Surface)
        .radius_px(18.0)
        .node_style();
    let popover = context
        .create_component(
            document(),
            Popover::new()
                .trigger_icon(Icon::Add, "更多")
                .trigger_style(custom)
                .width(280.0)
                .padding(12.0),
        )
        .unwrap();
    let id = popover.stable_id();
    for open in [false, true, false] {
        context
            .update_component(popover, |p, _| p.open = open)
            .unwrap();
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.layout.width, Some(LengthSpec::Px(36.0)));
        assert_eq!(style.layout.height, Some(LengthSpec::Px(36.0)));
        assert_eq!(style.layout.border_radius, Some(18.0));
        assert_eq!(
            context.read(popover, |p| (p.width, p.padding)).unwrap(),
            (280.0, 12.0)
        );
    }
}
