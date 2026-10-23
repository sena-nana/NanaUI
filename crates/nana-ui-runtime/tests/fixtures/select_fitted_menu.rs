// 固定字段的长选项菜单使用文本测量并保持视口边界。
#[test]
fn fitted_menu_expands_from_fixed_trigger_and_stays_inside_viewport() {
    for label in [
        "A much longer option than the trigger",
        &"Long option ".repeat(100),
    ] {
        let mut select = Select::new(Some("a"))
            .options([
                SelectOption::new("a", "A"),
                SelectOption::new("long", label),
            ])
            .fit_options(true)
            .opened(true);
        let layout = Arc::make_mut(&mut select.style.layout);
        layout.width = Some(LengthSpec::Px(128.0));
        layout.align_self = Some(nana_ui_core::AlignSpec::End);
        let (context, selects, _) = stacked(vec![select]);
        let id = selects[0].stable_id();
        let field = context.world().layout_box(id).unwrap();
        let Some(ComponentGeometry::Select {
            menu: Some(menu), ..
        }) = context.world().component_geometry(id)
        else {
            panic!("missing select menu");
        };
        assert_eq!(field.width, 128.0);
        assert!(menu.surface.width > field.width);
        assert!(menu.surface.x >= 0.0);
        assert!(menu.surface.x + menu.surface.width <= 600.0);
        assert_eq!(menu.options[1].label.content.as_str(), label);
        let natural = context.world().text_natural_width(id).unwrap();
        assert!(menu.options[1].label.bounds.width >= natural || menu.surface.width == 600.0);
    }
}
