//! Optically-aligned inline icon tooltip trigger ([`TooltipGlyph`]).
//!
//! Matches [`crate::SettingsRow`]'s help trigger geometry: a compact hit target
//! (`SpacingStep::Page`) that hangs symmetrically outside the surrounding text
//! line via theme-resolved negative margins so adding a help/info mark never
//! inflates single-line row heights.

use std::sync::Arc;

use nana_ui_core::{
    Icon, LengthSpec, SemanticColorRole, SpacingStep, ThemeExpr, ThemeToken, TypeRole,
};

use super::node::{IntoView, ViewBuilder};
use super::prop::{IntoProp, PropSource};

/// Optically-aligned inline icon trigger with a hover/focus tooltip.
pub struct TooltipGlyph {
    icon: Icon,
    tooltip: PropSource<Arc<str>>,
    label: Option<PropSource<Arc<str>>>,
    role: Option<SemanticColorRole>,
    expanded: Option<PropSource<bool>>,
    on_activate: Option<Box<dyn FnMut() + Send + 'static>>,
}

/// Create an optically-aligned inline [`TooltipGlyph`] (defaults to
/// [`Icon::HelpCircle`]).
pub fn tooltip_glyph(tooltip: impl IntoProp<Arc<str>>) -> TooltipGlyph {
    TooltipGlyph {
        icon: Icon::HelpCircle,
        tooltip: tooltip.into_source(),
        label: None,
        role: None,
        expanded: None,
        on_activate: None,
    }
}

/// Convenience alias for a help-circle [`TooltipGlyph`].
pub fn help_glyph(tooltip: impl IntoProp<Arc<str>>) -> TooltipGlyph {
    tooltip_glyph(tooltip)
}

impl TooltipGlyph {
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = icon;
        self
    }

    /// Override the spoken accessible name (defaults to the tooltip text).
    pub fn label(mut self, label: impl IntoProp<Arc<str>>) -> Self {
        self.label = Some(label.into_source());
        self
    }

    pub fn role(mut self, role: SemanticColorRole) -> Self {
        self.role = Some(role);
        self
    }

    pub fn expanded(mut self, expanded: impl IntoProp<bool>) -> Self {
        self.expanded = Some(expanded.into_source());
        self
    }

    pub fn on_activate(mut self, handler: impl FnMut() + Send + 'static) -> Self {
        self.on_activate = Some(Box::new(handler));
        self
    }
}

/// Theme-reactive symmetric overhang keeping a compact (`Page`) target flush
/// with a `Body` glyph inside a single-line row.
fn optical_overhang_expr() -> ThemeExpr {
    ThemeExpr::from(ThemeToken::Space(SpacingStep::Page))
        .plus(ThemeExpr::from(ThemeToken::Type(TypeRole::Body)).scaled(-1.0))
        .scaled(-0.5)
}

fn optical_overhang_default() -> f32 {
    (nana_ui_core::SpacingTokens::DEFAULT.page - nana_ui_core::TypographyTokens::DEFAULT.body)
        * -0.5
}

struct IconButtonExpanded;

impl super::prop::FieldWrite<crate::IconButton, bool> for IconButtonExpanded {
    const FIELD: &'static str = "IconButton.expanded";

    fn write(target: &mut crate::IconButton, value: bool) {
        target.expanded = Some(value);
    }

    fn differs(target: &crate::IconButton, value: &bool) -> bool {
        target.expanded != Some(*value)
    }
}

struct IconButtonTooltipWrite;

impl super::prop::FieldWrite<crate::IconButton, Arc<str>> for IconButtonTooltipWrite {
    const FIELD: &'static str = "IconButton.tooltip";

    fn write(target: &mut crate::IconButton, value: Arc<str>) {
        let config = target
            .tooltip
            .as_ref()
            .map(|t| t.config.clone())
            .unwrap_or_default();
        target.tooltip = Some(crate::IconButtonTooltip {
            label: value,
            config,
        });
    }

    fn differs(target: &crate::IconButton, value: &Arc<str>) -> bool {
        target.tooltip.as_ref().map(|t| t.label.as_ref()) != Some(value.as_ref())
    }
}

impl IntoView for TooltipGlyph {
    fn build(self, vb: &mut ViewBuilder<'_, '_, '_>) {
        let tooltip = Arc::new(std::sync::Mutex::new(self.tooltip));
        let label = self
            .label
            .map(|source| Arc::new(std::sync::Mutex::new(source)))
            .unwrap_or_else(|| Arc::clone(&tooltip));
        let initial_tooltip = super::untrack(|| tooltip.lock().unwrap().get());
        let initial_label = super::untrack(|| label.lock().unwrap().get());
        let overhang_px = optical_overhang_default();
        let overhang_theme = optical_overhang_expr();
        let role = self.role;

        let mut base = crate::IconButton::new(self.icon, initial_label)
            .compact(true)
            .with_tooltip(initial_tooltip);
        if let Some(role) = role {
            base.style.foreground = Some(role);
        }
        {
            let layout = std::sync::Arc::make_mut(&mut base.style.layout);
            layout.margin_top = Some(LengthSpec::Px(overhang_px));
            layout.margin_bottom = Some(LengthSpec::Px(overhang_px));
            layout.margin_left = Some(LengthSpec::Px(overhang_px));
            layout.margin_right = Some(LengthSpec::Px(overhang_px));
            layout.theme.margin_top = Some(overhang_theme.clone());
            layout.theme.margin_bottom = Some(overhang_theme.clone());
            layout.theme.margin_left = Some(overhang_theme.clone());
            layout.theme.margin_right = Some(overhang_theme);
        }
        let mut btn = super::node::widget(base)
            .label(move || label.lock().unwrap().get())
            .prop::<Arc<str>, IconButtonTooltipWrite>(move || tooltip.lock().unwrap().get());

        if let Some(expanded) = self.expanded {
            btn = btn.prop::<bool, IconButtonExpanded>(move || expanded.get());
        }
        if let Some(on_activate) = self.on_activate {
            btn = btn.on_activate(on_activate);
        }
        btn.build(vb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{row, text};
    use crate::{AppContext, DocumentId, LayoutViewport};

    #[test]
    fn tooltip_glyph_uses_theme_overhang_and_preserves_line_height() {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let view = cx
            .mount_view_root(document, || {
                row()
                    .children((text("Alpha"), tooltip_glyph("Explains alpha")))
                    .into_any()
            })
            .unwrap();
        cx.flush_reactive().unwrap();
        cx.layout_document(document, LayoutViewport::new(300.0, 100.0))
            .unwrap();

        let root = view.roots()[0];
        let children = cx.world().node(root).unwrap().children.clone();
        let glyph_id = children[1];
        let style = cx.world().node_style(glyph_id).unwrap();
        assert!(style.layout.theme.margin_top.is_some());
        assert!(style.layout.theme.margin_left.is_some());
        let a11y = cx.world().accessibility(glyph_id).unwrap();
        assert_eq!(a11y.label.as_deref(), Some("Explains alpha"));
    }
}
