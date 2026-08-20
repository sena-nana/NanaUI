//! Unified component type registry for builtins and plugins.
//!
//! L3 `create_component`, Vue tags, and `UiExtension` all resolve through
//! [`ComponentRegistry`]. Application business state stays in `AppContext`
//! views; this table only names types and projects generic UI into `UiWorld`.

use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use bevy_ecs::component::Component;
use nana_ui_core::{ButtonKind, ControlSize, Icon, LayoutStyle};

use crate::{ComponentView, FrameworkError, MutationQueue, StableNodeId, UiWorld};

/// Stable component identity (`nana.button`, `app.bilibili-user-card`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Component)]
pub struct ComponentTypeId(Arc<str>);

impl ComponentTypeId {
    pub fn new(id: impl Into<String>) -> Result<Self, FrameworkError> {
        let id = id.into();
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(FrameworkError::InvalidComponentType);
        }
        Ok(Self(Arc::from(trimmed)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ComponentTypeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Narrow L1/L2 property bag. Not application business state.
#[derive(Debug, Clone)]
pub struct SemanticSpec<'a> {
    pub type_id: &'a ComponentTypeId,
    pub label: &'a str,
    pub value: &'a str,
    pub hint: &'a str,
    pub placeholder: &'a str,
    pub disabled: bool,
    pub loading: bool,
    pub invalid: bool,
    pub active: bool,
    pub toggled: bool,
    pub read_only: bool,
    pub secure: bool,
    pub button_kind: ButtonKind,
    pub size: ControlSize,
    pub layout: &'a Arc<LayoutStyle>,
    pub icon: Option<Icon>,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub number: f32,
    /// Extra Vue/host attributes. Plugins read these instead of extending this struct.
    pub attrs: &'a [(&'a str, &'a str)],
}

impl<'a> SemanticSpec<'a> {
    pub fn from_parts(type_id: &'a ComponentTypeId, layout: &'a Arc<LayoutStyle>) -> Self {
        Self {
            type_id,
            label: "",
            value: "",
            hint: "",
            placeholder: "",
            disabled: false,
            loading: false,
            invalid: false,
            active: false,
            toggled: false,
            read_only: false,
            secure: false,
            button_kind: ButtonKind::Ghost,
            size: ControlSize::Medium,
            layout,
            icon: None,
            min: 0.0,
            max: 1.0,
            step: 0.1,
            number: 0.0,
            attrs: &[],
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then_some(*value))
    }

    pub fn display_label(&self) -> &str {
        if !self.label.is_empty() {
            self.label
        } else {
            self.value
        }
    }
}

pub(crate) struct ComponentBindRequest<'a> {
    pub id: StableNodeId,
    pub world: &'a UiWorld,
    pub mutations: &'a mut MutationQueue,
    pub spec: &'a SemanticSpec<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentBindKind {
    /// `from_semantic` projected a real control; skip generic Vue style.
    Projected,
    /// Tag-only / layout primitive; keep Vue generic style/a11y.
    Layout,
}

type Binder = Arc<
    dyn Fn(&mut ComponentBindRequest<'_>) -> Result<ComponentBindKind, FrameworkError>
        + Send
        + Sync,
>;

pub(crate) struct RegisteredComponentType {
    pub id: ComponentTypeId,
    rust_type: Option<TypeId>,
    binder: Binder,
}

/// Types that builtins and plugins register through the same ABI.
pub trait RegisterableComponent: ComponentView {
    const TYPE_ID: &'static str;
    const TAGS: &'static [&'static str];
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self;
}

#[derive(Default)]
pub struct ComponentRegistry {
    by_id: HashMap<ComponentTypeId, RegisteredComponentType>,
    by_tag: HashMap<String, ComponentTypeId>,
    by_rust: HashMap<TypeId, ComponentTypeId>,
}

impl ComponentRegistry {
    pub(crate) fn get_by_rust(&self, type_id: TypeId) -> Option<&RegisteredComponentType> {
        self.by_rust.get(&type_id).and_then(|id| self.by_id.get(id))
    }

    pub fn resolve_tag(&self, tag: &str) -> Option<&ComponentTypeId> {
        self.by_tag.get(&normalize_tag(tag))
    }

    pub(crate) fn insert(&mut self, entry: RegisteredComponentType) -> Result<(), FrameworkError> {
        if self.by_id.contains_key(&entry.id) {
            return Err(FrameworkError::DuplicateComponentType(
                entry.id.as_str().to_owned(),
            ));
        }
        if let Some(rust_type) = entry.rust_type
            && self.by_rust.contains_key(&rust_type)
        {
            return Err(FrameworkError::DuplicateComponentType(
                entry.id.as_str().to_owned(),
            ));
        }
        if let Some(rust_type) = entry.rust_type {
            self.by_rust.insert(rust_type, entry.id.clone());
        }
        self.by_id.insert(entry.id.clone(), entry);
        Ok(())
    }

    pub(crate) fn insert_with_tags(
        &mut self,
        entry: RegisteredComponentType,
        tags: impl IntoIterator<Item = String>,
    ) -> Result<(), FrameworkError> {
        let tags = tags
            .into_iter()
            .filter(|tag| !tag.is_empty())
            .collect::<Vec<_>>();
        for tag in &tags {
            if self.by_tag.contains_key(tag) {
                return Err(FrameworkError::DuplicateComponentTag(tag.clone()));
            }
        }
        let id = entry.id.clone();
        self.insert(entry)?;
        for tag in tags {
            self.by_tag.insert(tag, id.clone());
        }
        Ok(())
    }

    pub(crate) fn extend(&mut self, other: ComponentRegistry) -> Result<(), FrameworkError> {
        for tag in other.by_tag.keys() {
            if self.by_tag.contains_key(tag) {
                return Err(FrameworkError::DuplicateComponentTag(tag.clone()));
            }
        }
        for id in other.by_id.keys() {
            if self.by_id.contains_key(id) {
                return Err(FrameworkError::DuplicateComponentType(
                    id.as_str().to_owned(),
                ));
            }
        }
        for (rust_type, id) in &other.by_rust {
            if self.by_rust.contains_key(rust_type) {
                return Err(FrameworkError::DuplicateComponentType(
                    id.as_str().to_owned(),
                ));
            }
        }
        self.by_rust.extend(other.by_rust);
        self.by_tag.extend(other.by_tag);
        self.by_id.extend(other.by_id);
        Ok(())
    }

    pub(crate) fn bind(
        &self,
        request: &mut ComponentBindRequest<'_>,
    ) -> Result<ComponentBindKind, FrameworkError> {
        let Some(entry) = self.by_id.get(request.spec.type_id) else {
            return Err(FrameworkError::MissingComponentType(
                request.spec.type_id.as_str().to_owned(),
            ));
        };
        (entry.binder)(request)
    }
}

fn normalized_tags(tags: &[&str]) -> Vec<String> {
    tags.iter()
        .map(|tag| normalize_tag(tag))
        .filter(|tag| !tag.is_empty())
        .collect()
}

pub(crate) fn registerable_entry<C: RegisterableComponent>()
-> Result<(RegisteredComponentType, Vec<String>), FrameworkError> {
    Ok((
        RegisteredComponentType {
            id: ComponentTypeId::new(C::TYPE_ID)?,
            rust_type: Some(TypeId::of::<C>()),
            binder: Arc::new(|request| {
                let component = C::from_semantic(request.spec);
                component.project(request.id, request.world, request.mutations);
                Ok(ComponentBindKind::Projected)
            }),
        },
        normalized_tags(C::TAGS),
    ))
}

pub(crate) fn tag_entry(
    type_id: &'static str,
    tags: &'static [&'static str],
) -> Result<(RegisteredComponentType, Vec<String>), FrameworkError> {
    Ok((
        RegisteredComponentType {
            id: ComponentTypeId::new(type_id)?,
            rust_type: None,
            binder: Arc::new(|_| Ok(ComponentBindKind::Layout)),
        },
        normalized_tags(tags),
    ))
}

pub fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_start_matches("nana-").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_id_rejects_empty() {
        assert_eq!(
            ComponentTypeId::new("  ").unwrap_err(),
            FrameworkError::InvalidComponentType
        );
    }

    #[test]
    fn tags_normalize_nana_prefix() {
        assert_eq!(normalize_tag("nana-button"), "button");
        assert_eq!(normalize_tag("Button"), "button");
    }
}
