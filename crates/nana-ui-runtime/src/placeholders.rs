use std::sync::Arc;

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, LengthSpec,
    MutationQueue, NodeKind, NodeStyle, SemanticColorRole, StableNodeId, StandardVisual, UiWorld,
};

fn sanitize_surface_height(height: f32) -> f32 {
    height.max(1.0)
}

fn sanitize_surface_width(width: LengthSpec) -> LengthSpec {
    match width {
        LengthSpec::Px(value) => LengthSpec::Px(value.max(0.0)),
        width => width,
    }
}

fn clamp_level(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn status_foreground(tone: nana_ui_core::StatusTone) -> SemanticColorRole {
    crate::components::status_tone_role(tone)
}

fn inert() -> InteractionState {
    InteractionState {
        pointer_events: false,
        focusable: false,
    }
}

/// Non-interactive placeholder surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Skeleton {
    pub width: LengthSpec,
    pub height: f32,
    pub style: NodeStyle,
}

impl Skeleton {
    pub fn new(width: impl Into<LengthSpec>, height: f32) -> Self {
        Self {
            width: sanitize_surface_width(width.into()),
            height: sanitize_surface_height(height),
            style: NodeStyle::default(),
        }
    }

    pub fn fill_width(height: f32) -> Self {
        Self::new(LengthSpec::Fill, height)
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = sanitize_surface_height(height);
        self
    }

    pub fn width(mut self, width: LengthSpec) -> Self {
        self.width = sanitize_surface_width(width);
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn effective_style(&self, world: &UiWorld) -> NodeStyle {
        let mut style = self.style.clone();
        style.background = Some(SemanticColorRole::Subtle);
        style.border = None;
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(sanitize_surface_width(self.width));
        layout.height = Some(LengthSpec::Px(sanitize_surface_height(self.height)));
        layout.border_width = Some(0.0);
        layout.border_radius = Some(world.theme_metrics().radius_sm);
        style
    }
}

impl ComponentView for Skeleton {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "skeleton".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(world),
            inert(),
            AccessibilityState {
                role: AccessibilityRole::Generic,
                ..AccessibilityState::default()
            },
        );
    }
}

/// Compact determinate meter for continuously sampled levels.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelMeter {
    pub value: f32,
    pub height: f32,
    pub tone: nana_ui_core::StatusTone,
    pub style: NodeStyle,
}

impl LevelMeter {
    pub fn new(value: f32) -> Self {
        Self {
            value,
            height: 4.0,
            tone: nana_ui_core::StatusTone::Success,
            style: NodeStyle::default(),
        }
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = sanitize_surface_height(height);
        self
    }

    pub fn tone(mut self, tone: nana_ui_core::StatusTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    fn girth(&self) -> f32 {
        sanitize_surface_height(self.height)
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        style.foreground = Some(status_foreground(self.tone));
        style.background = Some(SemanticColorRole::Background);
        style.border = None;
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Px(self.girth()));
        layout.border_width = Some(0.0);
        style
    }
}

impl ComponentView for LevelMeter {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "level-meter".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let value = clamp_level(self.value);
        let girth = self.girth();
        let visual = StandardVisual::LevelMeter {
            value_ratio: value,
            girth,
            tone: self.tone,
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        let percent = (f64::from(value) * 100.0).round();
        project_common(
            id,
            world,
            mutations,
            &self.effective_style(),
            inert(),
            AccessibilityState {
                role: AccessibilityRole::ProgressIndicator,
                value: Some(Arc::from(format!("{percent:.0}%"))),
                numeric_minimum: Some(0.0),
                numeric_maximum: Some(1.0),
                numeric_value: Some(f64::from(value)),
                ..AccessibilityState::default()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn skeleton_projects_subtle_surface_without_visual() {
        let clamped = Skeleton::new(LengthSpec::Px(-8.0), 0.0);
        assert_eq!(clamped.width, LengthSpec::Px(0.0));
        assert_eq!(clamped.height, 1.0);
        assert_eq!(Skeleton::fill_width(18.0).width, LengthSpec::Fill);

        let mut context = AppContext::new();
        let skeleton = context
            .create_component(document(), Skeleton::new(LengthSpec::Px(120.0), 16.0))
            .unwrap();
        let id = skeleton.stable_id();

        assert!(matches!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element { tag } if tag == "skeleton"
        ));
        assert_eq!(context.world().standard_visual(id), None);
        assert_eq!(context.world().text(id), Some(""));
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Subtle));
        assert_eq!(style.border, None);
        assert_eq!(style.layout.border_width, Some(0.0));
        assert_eq!(
            style.layout.border_radius,
            Some(context.world().theme_metrics().radius_sm)
        );
        assert_eq!(style.layout.width, Some(LengthSpec::Px(120.0)));
        assert_eq!(style.layout.height, Some(LengthSpec::Px(16.0)));
        assert_eq!(context.world().interaction(id), Some(inert()));
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.role, AccessibilityRole::Generic);
        assert_eq!(accessibility.label, None);
    }

    #[test]
    fn level_meter_clamps_value_and_uses_tone() {
        let mut context = AppContext::new();
        let meter = context
            .create_component(document(), LevelMeter::new(-0.25))
            .unwrap();
        let id = meter.stable_id();

        assert!(matches!(
            context.world().node(id).unwrap().kind,
            NodeKind::Element { tag } if tag == "level-meter"
        ));
        assert_eq!(
            context.world().standard_visual(id),
            Some(StandardVisual::LevelMeter {
                value_ratio: 0.0,
                girth: 4.0,
                tone: nana_ui_core::StatusTone::Success,
            })
        );
        let style = context.world().node_style(id).unwrap();
        assert_eq!(style.background, Some(SemanticColorRole::Background));
        assert_eq!(style.foreground, Some(SemanticColorRole::Success));
        assert_eq!(style.border, None);
        assert_eq!(style.layout.width, Some(LengthSpec::Fill));
        assert_eq!(style.layout.height, Some(LengthSpec::Px(4.0)));
        assert_eq!(context.world().interaction(id), Some(inert()));
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.role, AccessibilityRole::ProgressIndicator);
        assert_eq!(accessibility.value.as_deref(), Some("0%"));
        assert_eq!(accessibility.numeric_minimum, Some(0.0));
        assert_eq!(accessibility.numeric_maximum, Some(1.0));
        assert_eq!(accessibility.numeric_value, Some(0.0));

        context
            .update_component(meter, |meter, _| {
                meter.value = 1.5;
                meter.height = 8.0;
                meter.tone = nana_ui_core::StatusTone::Danger;
            })
            .unwrap();
        assert_eq!(
            context.world().standard_visual(id),
            Some(StandardVisual::LevelMeter {
                value_ratio: 1.0,
                girth: 8.0,
                tone: nana_ui_core::StatusTone::Danger,
            })
        );
        assert_eq!(
            context.world().node_style(id).unwrap().foreground,
            Some(SemanticColorRole::Danger)
        );
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.value.as_deref(), Some("100%"));
        assert_eq!(accessibility.numeric_value, Some(1.0));

        context
            .update_component(meter, |meter, _| {
                meter.value = f32::NAN;
                meter.tone = nana_ui_core::StatusTone::Info;
            })
            .unwrap();
        assert_eq!(
            context.world().standard_visual(id),
            Some(StandardVisual::LevelMeter {
                value_ratio: 0.0,
                girth: 8.0,
                tone: nana_ui_core::StatusTone::Info,
            })
        );
        assert_eq!(
            context.world().node_style(id).unwrap().foreground,
            Some(SemanticColorRole::Accent)
        );

        context
            .update_component(meter, |meter, _| {
                meter.value = 0.5;
                meter.tone = nana_ui_core::StatusTone::Warning;
            })
            .unwrap();
        assert_eq!(
            context.world().node_style(id).unwrap().foreground,
            Some(SemanticColorRole::Warning)
        );
        let accessibility = context.world().accessibility(id).unwrap();
        assert_eq!(accessibility.value.as_deref(), Some("50%"));
        assert_eq!(accessibility.numeric_value, Some(0.5));

        context
            .update_component(meter, |meter, _| {
                meter.tone = nana_ui_core::StatusTone::Neutral;
            })
            .unwrap();
        assert_eq!(
            context.world().node_style(id).unwrap().foreground,
            Some(SemanticColorRole::Muted)
        );
    }

    #[test]
    fn idle_project_does_not_dirty() {
        let mut context = AppContext::new();
        let skeleton = context
            .create_component(document(), Skeleton::fill_width(12.0))
            .unwrap();
        let meter = context
            .create_component(document(), LevelMeter::new(0.3))
            .unwrap();
        let _ = context.take_system_work();
        context.update_component(skeleton, |_, _| {}).unwrap();
        context.update_component(meter, |_, _| {}).unwrap();
        assert!(context.take_system_work().is_empty());
    }
}
