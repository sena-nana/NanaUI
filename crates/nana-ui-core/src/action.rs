use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActionId(String);

impl ActionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for ActionId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ActionId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionPickerNavigation {
    Previous,
    Next,
    First,
    Last,
    Confirm,
    Dismiss,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPaletteItem {
    pub action: ActionId,
    pub label: String,
    pub category: Option<String>,
    pub shortcut: Option<String>,
}

impl CommandPaletteItem {
    pub fn new(action: impl Into<ActionId>, label: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            label: label.into(),
            category: None,
            shortcut: None,
        }
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPaletteEvent {
    Search(String),
    Select(ActionId),
    Navigate(ActionPickerNavigation),
    Dismiss,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyContext {
    tags: BTreeSet<String>,
}

impl KeyContext {
    pub fn new(tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            tags: tags.into_iter().map(Into::into).collect(),
        }
    }

    pub fn contains(&self, tag: &str) -> bool {
        self.tags.contains(tag)
    }

    pub fn insert(&mut self, tag: impl Into<String>) -> bool {
        self.tags.insert(tag.into())
    }

    pub fn remove(&mut self, tag: &str) -> bool {
        self.tags.remove(tag)
    }

    pub fn with(mut self, tag: impl Into<String>) -> Self {
        self.insert(tag);
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPredicate {
    all: BTreeSet<String>,
    any: BTreeSet<String>,
    none: BTreeSet<String>,
}

impl ContextPredicate {
    pub fn always() -> Self {
        Self::default()
    }

    pub fn all_of(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.all.extend(tags.into_iter().map(Into::into));
        self
    }

    pub fn any_of(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.any.extend(tags.into_iter().map(Into::into));
        self
    }

    pub fn none_of(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.none.extend(tags.into_iter().map(Into::into));
        self
    }

    pub fn matches(&self, context: &KeyContext) -> bool {
        self.all.iter().all(|tag| context.contains(tag))
            && (self.any.is_empty() || self.any.iter().any(|tag| context.contains(tag)))
            && self.none.iter().all(|tag| !context.contains(tag))
    }
}
