//! Typed Motion codecs. Custom GPU properties must register here; ordinary
//! components never supply an untyped byte layout.

use std::collections::HashMap;

use super::property::AnimatableProperty;

/// Identifies a value codec. Built-in compositor codecs occupy `1..=255`;
/// registered shader parameters start at [`MotionCodecId::CUSTOM_START`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MotionCodecId(u16);

impl MotionCodecId {
    pub const OPACITY: Self = Self(1);
    pub const TRANSFORM: Self = Self(2);
    pub const CLIP: Self = Self(3);
    /// First id available to [`MotionCodecRegistry::register`].
    pub const CUSTOM_START: u16 = 256;

    pub const fn get(self) -> u16 {
        self.0
    }

    pub const fn from_raw(value: u16) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    /// Built-in codec for a compositor property. `ShaderParameter` has none
    /// until a registry entry is supplied.
    pub fn for_property(property: AnimatableProperty) -> Option<Self> {
        match property {
            AnimatableProperty::Opacity => Some(Self::OPACITY),
            AnimatableProperty::Transform => Some(Self::TRANSFORM),
            AnimatableProperty::Clip => Some(Self::CLIP),
            AnimatableProperty::ShaderParameter
            | AnimatableProperty::Color
            | AnimatableProperty::Background
            | AnimatableProperty::Blur
            | AnimatableProperty::Filter
            | AnimatableProperty::Shadow
            | AnimatableProperty::Width
            | AnimatableProperty::Height
            | AnimatableProperty::Padding
            | AnimatableProperty::Margin
            | AnimatableProperty::FontSize
            | AnimatableProperty::FontAxis
            | AnimatableProperty::Display
            | AnimatableProperty::Progress => None,
        }
    }
}

/// Channel layout a codec interpolates. Not a free-form byte blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionValueKind {
    Scalar,
    Color,
    Transform,
    Discrete,
}

/// One registered (or built-in) codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotionCodecInfo {
    pub id: MotionCodecId,
    pub name: &'static str,
    pub value_kind: MotionValueKind,
}

/// Built-in compositor codecs plus typed custom shader parameters.
#[derive(Debug, Clone)]
pub struct MotionCodecRegistry {
    custom: HashMap<MotionCodecId, MotionCodecInfo>,
    by_name: HashMap<&'static str, MotionCodecId>,
    next_custom: u16,
}

impl Default for MotionCodecRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl MotionCodecRegistry {
    pub fn builtin() -> Self {
        Self {
            custom: HashMap::new(),
            by_name: HashMap::new(),
            next_custom: MotionCodecId::CUSTOM_START,
        }
    }

    pub fn get(&self, id: MotionCodecId) -> Option<MotionCodecInfo> {
        match id {
            MotionCodecId::OPACITY => Some(MotionCodecInfo {
                id,
                name: "opacity",
                value_kind: MotionValueKind::Scalar,
            }),
            MotionCodecId::TRANSFORM => Some(MotionCodecInfo {
                id,
                name: "transform",
                value_kind: MotionValueKind::Transform,
            }),
            MotionCodecId::CLIP => Some(MotionCodecInfo {
                id,
                name: "clip",
                value_kind: MotionValueKind::Scalar,
            }),
            other => self.custom.get(&other).copied(),
        }
    }

    /// Register a typed shader-parameter codec. Duplicate names fail; the
    /// payload kind is required so callers cannot upload arbitrary bytes.
    pub fn register(
        &mut self,
        name: &'static str,
        value_kind: MotionValueKind,
    ) -> Result<MotionCodecId, MotionCodecError> {
        if name.is_empty() || self.builtin_name(name) || self.by_name.contains_key(name) {
            return Err(MotionCodecError::DuplicateName);
        }
        if matches!(value_kind, MotionValueKind::Discrete) {
            return Err(MotionCodecError::UnsupportedKind);
        }
        if self.next_custom == 0 {
            return Err(MotionCodecError::Exhausted);
        }
        let id = MotionCodecId(self.next_custom);
        self.next_custom = self.next_custom.saturating_add(1);
        let info = MotionCodecInfo {
            id,
            name,
            value_kind,
        };
        self.custom.insert(id, info);
        self.by_name.insert(name, id);
        Ok(id)
    }

    fn builtin_name(&self, name: &str) -> bool {
        matches!(name, "opacity" | "transform" | "clip")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionCodecError {
    DuplicateName,
    UnsupportedKind,
    Exhausted,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::AnimationClass;

    #[test]
    fn width_is_not_a_transform_codec() {
        assert_eq!(MotionCodecId::for_property(AnimatableProperty::Width), None);
        assert_eq!(
            MotionCodecId::for_property(AnimatableProperty::Transform),
            Some(MotionCodecId::TRANSFORM)
        );
        assert_ne!(
            AnimatableProperty::Width.animation_class(),
            AnimationClass::Compositor
        );
    }

    #[test]
    fn shader_parameter_requires_registration() {
        let mut registry = MotionCodecRegistry::builtin();
        assert_eq!(
            MotionCodecId::for_property(AnimatableProperty::ShaderParameter),
            None
        );
        let id = registry
            .register("nana.shader.glow", MotionValueKind::Scalar)
            .expect("register");
        assert!(id.get() >= MotionCodecId::CUSTOM_START);
        assert!(registry.get(id).is_some());
        assert_eq!(
            registry.register("nana.shader.glow", MotionValueKind::Scalar),
            Err(MotionCodecError::DuplicateName)
        );
        assert_eq!(
            registry.register("opacity", MotionValueKind::Scalar),
            Err(MotionCodecError::DuplicateName)
        );
    }
}
