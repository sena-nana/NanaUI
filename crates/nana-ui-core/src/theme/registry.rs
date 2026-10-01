use super::{CompiledTheme, ThemeAppearance, ThemeDefinition, ThemeId, builtin_theme_arc};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeChoice {
    pub id: ThemeId,
    pub label: Arc<str>,
    pub appearance: ThemeAppearance,
}

#[derive(Debug, Clone)]
pub struct ThemeResolution {
    pub theme: Arc<CompiledTheme>,
    pub requested: ThemeId,
    pub fell_back_to_light: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThemeRegistryError {
    Compile(super::ThemeCompileError),
    BuiltInRemoval(ThemeId),
    Missing(ThemeId),
}
impl From<super::ThemeCompileError> for ThemeRegistryError {
    fn from(value: super::ThemeCompileError) -> Self {
        Self::Compile(value)
    }
}

#[derive(Debug, Clone)]
pub struct ThemeRegistry {
    themes: BTreeMap<ThemeId, Arc<CompiledTheme>>,
    choices: BTreeMap<ThemeId, Arc<str>>,
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        let mut this = Self {
            themes: BTreeMap::new(),
            choices: BTreeMap::new(),
        };
        this.insert_builtin(ThemeDefinition::NANA_LIGHT, "浅色");
        this.insert_builtin(ThemeDefinition::NANA_DARK, "深色");
        this
    }
}

impl ThemeRegistry {
    fn insert_builtin(&mut self, definition: ThemeDefinition, label: &str) {
        let id = definition.id.clone();
        self.themes.insert(
            id.clone(),
            builtin_theme_arc(match definition.appearance {
                ThemeAppearance::Light => super::ThemeAppearance::Light,
                ThemeAppearance::Dark | ThemeAppearance::Custom => super::ThemeAppearance::Dark,
            }),
        );
        self.choices.insert(id, Arc::from(label));
    }
    pub fn register(&mut self, definition: ThemeDefinition) -> Result<ThemeId, ThemeRegistryError> {
        let label: Arc<str> = Arc::from(definition.display_name.as_ref());
        self.register_with_label(definition, label)
    }

    /// Register with an explicit settings label when the host wants to
    /// override the definition's persisted display name.
    pub fn register_with_label(
        &mut self,
        mut definition: ThemeDefinition,
        label: impl Into<Arc<str>>,
    ) -> Result<ThemeId, ThemeRegistryError> {
        let id = definition.id.clone();
        let label = label.into();
        // Re-installing the same compiled revision is deliberately a no-op:
        // callers use this path when several hosts converge on one registry,
        // and a spurious generation bump would invalidate every style cache.
        if let Some(previous) = self.themes.get(&id)
            && definition.generation == previous.identity().generation
        {
            let candidate = definition.compile()?;
            if previous.as_ref() == &candidate {
                if self.choices.get(&id) != Some(&label) {
                    self.choices.insert(id.clone(), label);
                }
                return Ok(id);
            }
        }
        if let Some(previous) = self.themes.get(&id) {
            let previous_generation = previous.identity().generation;
            if definition.generation <= previous_generation {
                definition.generation = previous_generation.next();
            }
        }
        self.themes
            .insert(id.clone(), Arc::new(definition.compile()?));
        self.choices.insert(id.clone(), label);
        Ok(id)
    }

    /// Replace an existing definition, or register it when the ID is new.
    /// Generation monotonicity is enforced exactly as it is for `register`.
    pub fn replace(&mut self, definition: ThemeDefinition) -> Result<ThemeId, ThemeRegistryError> {
        self.register(definition)
    }

    /// Register using the definition's persisted display metadata.
    pub fn register_definition(
        &mut self,
        definition: ThemeDefinition,
    ) -> Result<ThemeId, ThemeRegistryError> {
        self.register(definition)
    }
    pub fn get(&self, id: &ThemeId) -> Option<Arc<CompiledTheme>> {
        self.themes.get(id).cloned()
    }
    pub fn resolve_or_light(&self, id: &ThemeId) -> Arc<CompiledTheme> {
        self.get(id)
            .or_else(|| self.get(&ThemeDefinition::NANA_LIGHT.id))
            .unwrap_or_else(|| builtin_theme_arc(super::ThemeAppearance::Light))
    }
    pub fn resolve(&self, id: &ThemeId) -> ThemeResolution {
        match self.get(id) {
            Some(theme) => ThemeResolution {
                theme,
                requested: id.clone(),
                fell_back_to_light: false,
            },
            None => ThemeResolution {
                theme: self
                    .get(&ThemeDefinition::NANA_LIGHT.id)
                    .unwrap_or_else(|| builtin_theme_arc(super::ThemeAppearance::Light)),
                requested: id.clone(),
                fell_back_to_light: true,
            },
        }
    }
    pub fn remove(&mut self, id: &ThemeId) -> Result<(), ThemeRegistryError> {
        if *id == ThemeDefinition::NANA_LIGHT.id || *id == ThemeDefinition::NANA_DARK.id {
            return Err(ThemeRegistryError::BuiltInRemoval(id.clone()));
        }
        if self.themes.remove(id).is_none() {
            return Err(ThemeRegistryError::Missing(id.clone()));
        }
        self.choices.remove(id);
        Ok(())
    }
    pub fn choices(&self) -> impl Iterator<Item = ThemeChoice> + '_ {
        self.choices.iter().filter_map(|(id, label)| {
            self.themes.get(id).map(|theme| ThemeChoice {
                id: id.clone(),
                label: label.clone(),
                appearance: theme.appearance(),
            })
        })
    }

    /// Snapshot the options for an Appearance settings page.
    pub fn choices_vec(&self) -> Vec<ThemeChoice> {
        self.choices().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_owned_custom_theme_ids_and_falls_back_to_light() {
        let mut registry = ThemeRegistry::default();
        let id = registry
            .register_with_label(
                ThemeDefinition::NANA_LIGHT.with_id(ThemeId::from_owned("user.sunset")),
                "Sunset",
            )
            .unwrap();
        assert_eq!(id.as_str(), "user.sunset");
        assert!(registry.get(&id).is_some());
        assert_eq!(
            registry
                .resolve_or_light(&ThemeId::new("missing"))
                .appearance(),
            super::super::ThemeAppearance::Light
        );
    }

    #[test]
    fn builtins_cannot_be_removed() {
        let mut registry = ThemeRegistry::default();
        assert!(matches!(
            registry.remove(&ThemeDefinition::NANA_DARK.id),
            Err(ThemeRegistryError::BuiltInRemoval(_))
        ));
    }

    #[test]
    fn replacing_a_theme_advances_generation() {
        let mut registry = ThemeRegistry::default();
        let id = ThemeId::from_owned("user.sunset");
        let first = ThemeDefinition::NANA_LIGHT.with_id(id.clone());
        registry.register_with_label(first, "Sunset").unwrap();
        let generation = registry.get(&id).unwrap().identity().generation;
        registry
            .register_with_label(ThemeDefinition::NANA_DARK.with_id(id.clone()), "Sunset 2")
            .unwrap();
        assert_eq!(
            registry.get(&id).unwrap().identity().generation,
            generation.next()
        );
    }

    #[test]
    fn registering_the_same_revision_is_a_noop() {
        let mut registry = ThemeRegistry::default();
        let id = ThemeId::from_owned("user.sunset");
        let definition = ThemeDefinition::NANA_LIGHT.with_id(id.clone());
        registry.register(definition.clone()).unwrap();
        let before = registry.get(&id).unwrap();
        registry.register(definition).unwrap();
        let after = registry.get(&id).unwrap();
        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(before.identity(), after.identity());
    }

    #[test]
    fn choices_expose_custom_appearance_and_definition_label() {
        let mut registry = ThemeRegistry::default();
        let id = ThemeId::from_owned("user.sunset");
        registry
            .register_definition(
                ThemeDefinition::NANA_LIGHT
                    .with_id(id.clone())
                    .with_display_name("夕暮")
                    .with_appearance(ThemeAppearance::Custom),
            )
            .unwrap();
        let choice = registry.choices().find(|choice| choice.id == id).unwrap();
        assert_eq!(choice.label.as_ref(), "夕暮");
        assert_eq!(choice.appearance, ThemeAppearance::Custom);
    }
}
