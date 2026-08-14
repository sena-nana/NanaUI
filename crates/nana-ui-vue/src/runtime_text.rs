pub use nana_ui::IcedTextShaper;

#[cfg(test)]
mod tests {
    use nana_ui_runtime::{
        ComputedStyle, StableNodeId, TextContent, TextShapeConstraints, TextShaper,
    };

    use super::IcedTextShaper;

    #[test]
    fn shapes_cjk_through_the_visible_renderer_backend() {
        let mut shaper = IcedTextShaper;
        let metrics = shaper.shape(
            StableNodeId::new(1).unwrap(),
            &TextContent {
                value: "输入法".into(),
            },
            &ComputedStyle::default(),
            TextShapeConstraints::default(),
        );
        assert!(metrics.width > 0.0);
        assert!(metrics.height > 0.0);
    }
}
