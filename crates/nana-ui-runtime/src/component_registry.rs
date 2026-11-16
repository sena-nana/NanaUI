//! Unified Runtime component ABI for built-in types and extensions.
//!
//! L3 `create_component`, component tags, and `UiExtension` resolve component
//! identities through [`ComponentRegistry`]. Built-in component descriptors live
//! in [`crate::component_descriptors`] and carry no semantic construction table;
//! compiled built-ins stamp their `TYPE_ID` directly via
//! [`crate::ComponentView::TYPE_ID`].
//!
//! Register layout/hit/Scene extension components here via
//! [`crate::ExtensionRegistrar::register_component`].

use std::{
    any::TypeId,
    borrow::Borrow,
    cmp::Ordering,
    collections::HashMap,
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};

use crate::{ComponentView, FrameworkError, component_descriptors::BUILTIN_COMPONENT_IDS};

/// Stable component identity (`nana.button`, `app.bilibili-user-card`).
#[derive(Clone)]
pub struct ComponentTypeId(ComponentIdRepr);

#[derive(Clone)]
enum ComponentIdRepr {
    Static(&'static str),
    Shared(Arc<str>),
}

impl ComponentTypeId {
    /// Wrap a static component identifier without allocating.
    pub const fn from_static(id: &'static str) -> Self {
        assert!(!id.is_empty(), "ComponentTypeId cannot be empty");
        Self(ComponentIdRepr::Static(id))
    }

    pub fn new(id: impl Into<String>) -> Result<Self, FrameworkError> {
        let id = id.into();
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(FrameworkError::InvalidComponentType);
        }
        Ok(Self(ComponentIdRepr::Shared(Arc::from(trimmed))))
    }

    pub fn as_str(&self) -> &str {
        match &self.0 {
            ComponentIdRepr::Static(id) => id,
            ComponentIdRepr::Shared(id) => id,
        }
    }
}

impl PartialEq for ComponentTypeId {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for ComponentTypeId {}

impl PartialOrd for ComponentTypeId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ComponentTypeId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for ComponentTypeId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl fmt::Debug for ComponentTypeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ComponentTypeId")
            .field(&self.as_str())
            .finish()
    }
}

impl Borrow<str> for ComponentTypeId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for ComponentTypeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Registry metadata for an extension component type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredComponentType {
    pub id: ComponentTypeId,
    pub rust_type: Option<TypeId>,
}

/// Extension component registered through [`crate::ExtensionRegistrar::register_component`].
pub trait RegisterableComponent: ComponentView {
    const TYPE_ID: &'static str;
    const TAGS: &'static [&'static str];
}

#[derive(Default, Clone)]
pub struct ComponentRegistry {
    by_id: HashMap<ComponentTypeId, RegisteredComponentType>,
    by_rust: HashMap<TypeId, ComponentTypeId>,
    by_tag: HashMap<String, ComponentTypeId>,
}

impl ComponentRegistry {
    /// Every registered component type (compiled built-ins plus extensions), in
    /// no particular order.
    #[cfg(test)]
    pub(crate) fn type_ids(&self) -> impl Iterator<Item = &ComponentTypeId> {
        BUILTIN_COMPONENT_IDS
            .iter()
            .filter(|(desc, _)| desc.compiled)
            .map(|(_, id)| id)
            .chain(self.by_id.keys())
    }

    pub(crate) fn type_id_for(&self, rust_type: TypeId) -> Option<&ComponentTypeId> {
        self.by_rust.get(&rust_type)
    }

    pub fn resolve_tag(&self, tag: &str) -> Option<&ComponentTypeId> {
        self.resolve_normalized(&normalize_tag(tag))
    }

    /// Resolve an already-normalized tag (see [`normalize_tag`]). Callers that
    /// probe several candidate spellings can normalize each candidate once and
    /// dedupe before looking up.
    pub fn resolve_normalized(&self, normalized_tag: &str) -> Option<&ComponentTypeId> {
        if let Some(id) = self.by_tag.get(normalized_tag) {
            return Some(id);
        }
        BUILTIN_COMPONENT_IDS
            .iter()
            .find(|(desc, _)| desc.compiled && desc.tags.contains(&normalized_tag))
            .map(|(_, id)| id)
    }

    pub(crate) fn register<C: RegisterableComponent>(&mut self) -> Result<(), FrameworkError> {
        let id = ComponentTypeId::new(<C as RegisterableComponent>::TYPE_ID)?;
        let entry = RegisteredComponentType {
            id,
            rust_type: Some(TypeId::of::<C>()),
        };
        self.insert_with_tags(entry, normalized_tags(C::TAGS))
    }

    pub(crate) fn register_tags(
        &mut self,
        type_id: &'static str,
        tags: &'static [&'static str],
    ) -> Result<(), FrameworkError> {
        let id = ComponentTypeId::new(type_id)?;
        let entry = RegisteredComponentType {
            id,
            rust_type: None,
        };
        self.insert_with_tags(entry, normalized_tags(tags))
    }

    fn contains_type_id(&self, id: &ComponentTypeId) -> bool {
        self.by_id.contains_key(id)
            || BUILTIN_COMPONENT_IDS
                .iter()
                .any(|(desc, _)| desc.compiled && desc.type_id == id.as_str())
    }

    fn contains_tag(&self, tag: &str) -> bool {
        self.by_tag.contains_key(tag)
            || BUILTIN_COMPONENT_IDS
                .iter()
                .any(|(desc, _)| desc.compiled && desc.tags.contains(&tag))
    }

    fn insert(&mut self, entry: RegisteredComponentType) -> Result<(), FrameworkError> {
        if self.contains_type_id(&entry.id) {
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

    fn insert_with_tags(
        &mut self,
        entry: RegisteredComponentType,
        tags: impl IntoIterator<Item = String>,
    ) -> Result<(), FrameworkError> {
        let tags = tags
            .into_iter()
            .filter(|tag| !tag.is_empty())
            .collect::<Vec<_>>();
        for tag in &tags {
            if self.contains_tag(tag) {
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
            if self.contains_tag(tag) {
                return Err(FrameworkError::DuplicateComponentTag(tag.clone()));
            }
        }
        for id in other.by_id.keys() {
            if self.contains_type_id(id) {
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
}

fn normalized_tags(tags: &[&str]) -> Vec<String> {
    tags.iter()
        .map(|tag| normalize_tag(tag))
        .filter(|tag| !tag.is_empty())
        .collect()
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
    fn static_and_shared_type_ids_compare_and_hash_equally() {
        use std::collections::hash_map::DefaultHasher;

        let static_id = ComponentTypeId::from_static("nana.button");
        let shared_id = ComponentTypeId::new("nana.button").unwrap();
        assert_eq!(static_id, shared_id);

        let hash_of = |id: &ComponentTypeId| {
            let mut hasher = DefaultHasher::new();
            id.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_of(&static_id), hash_of(&shared_id));
    }

    #[test]
    fn tags_normalize_nana_prefix() {
        assert_eq!(normalize_tag("nana-button"), "button");
        assert_eq!(normalize_tag("Button"), "button");
    }
}
