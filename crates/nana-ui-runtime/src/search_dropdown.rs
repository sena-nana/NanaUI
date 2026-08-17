//! Searchable single-value field. Query uses committed TextInput state.

use std::sync::Arc;

use nana_ui_core::ControlSize;

use crate::query::query_matches;
use crate::select::{field_style_for_size, menu_option_label, select_option_at};
use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, ComponentView, InteractionState, MutationQueue,
    NodeKind, NodeStyle, SelectOptionData, SemanticPaint, StableNodeId, StandardVisual,
    TextContent, TextInputState, UiWorld,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchDropdownOption {
    pub value: Arc<str>,
    pub label: Arc<str>,
    pub hint: Option<Arc<str>>,
}

impl SearchDropdownOption {
    pub fn new(value: impl Into<Arc<str>>, label: impl Into<Arc<str>>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            hint: None,
        }
    }

    pub fn hint(mut self, hint: impl Into<Arc<str>>) -> Self {
        let hint = hint.into();
        self.hint = (!hint.is_empty()).then_some(hint);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchDropdownEvent {
    Search(String),
    Select(Arc<str>),
    Opened,
    Closed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchDropdown {
    pub value: Option<Arc<str>>,
    pub options: Vec<SearchDropdownOption>,
    pub query: String,
    pub placeholder: Option<Arc<str>>,
    pub size: ControlSize,
    pub disabled: bool,
    pub loading: bool,
    pub invalid: bool,
    pub opened: bool,
    pub highlighted: Option<usize>,
    pub state: TextInputState,
    pub style: NodeStyle,
}

impl SearchDropdown {
    pub fn new(value: Option<impl Into<Arc<str>>>) -> Self {
        Self {
            value: value.map(Into::into),
            options: Vec::new(),
            query: String::new(),
            placeholder: None,
            size: ControlSize::Medium,
            disabled: false,
            loading: false,
            invalid: false,
            opened: false,
            highlighted: None,
            state: TextInputState::new(""),
            style: field_style_for_size(ControlSize::Medium),
        }
    }

    pub fn options(mut self, options: impl IntoIterator<Item = SearchDropdownOption>) -> Self {
        self.options = options.into_iter().collect();
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<Arc<str>>) -> Self {
        let placeholder = placeholder.into();
        self.placeholder = (!placeholder.is_empty()).then_some(placeholder);
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self.style = field_style_for_size(size);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn opened(mut self, opened: bool) -> Self {
        self.opened = opened;
        if opened {
            self.state = TextInputState::new(self.query.clone());
            self.highlighted = self.first_visible();
        } else {
            self.highlighted = None;
        }
        self
    }

    pub fn query(mut self, query: impl Into<String>) -> Self {
        let query = query.into();
        self.query = query.clone();
        self.state = TextInputState::new(query);
        self
    }

    pub fn inactive(&self) -> bool {
        self.disabled || self.loading
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        self.options
            .iter()
            .enumerate()
            .filter(|(_, option)| option_matches(option, &self.query))
            .map(|(index, _)| index)
            .collect()
    }

    pub fn display_label(&self) -> (Arc<str>, bool) {
        if self.opened && !self.inactive() {
            if self.query.is_empty() {
                return (
                    self.placeholder.clone().unwrap_or_else(|| Arc::from("")),
                    true,
                );
            }
            return (Arc::from(self.query.as_str()), false);
        }
        if let Some(value) = &self.value
            && let Some(option) = self.options.iter().find(|option| &option.value == value)
        {
            return (
                menu_option_label(&SelectOptionData {
                    label: Arc::clone(&option.label),
                    hint: option.hint.clone(),
                    disabled: false,
                    checked: false,
                    icon: None,
                }),
                false,
            );
        }
        (
            self.placeholder.clone().unwrap_or_else(|| Arc::from("")),
            true,
        )
    }

    pub fn toggle_open(&mut self) -> Option<SearchDropdownEvent> {
        if self.inactive() {
            return None;
        }
        self.opened = !self.opened;
        if self.opened {
            self.state = TextInputState::new(self.query.clone());
            self.highlighted = self.first_visible();
            Some(SearchDropdownEvent::Opened)
        } else {
            self.highlighted = None;
            Some(SearchDropdownEvent::Closed)
        }
    }

    pub fn close(&mut self) -> Option<SearchDropdownEvent> {
        if !self.opened {
            return None;
        }
        self.opened = false;
        self.highlighted = None;
        Some(SearchDropdownEvent::Closed)
    }

    pub fn set_query(&mut self, query: impl Into<String>) -> SearchDropdownEvent {
        let query = query.into();
        self.query = query.clone();
        self.state.replace_value(query.clone());
        self.highlighted = self.first_visible();
        SearchDropdownEvent::Search(query)
    }

    pub fn highlight_delta(&mut self, delta: i32) -> bool {
        if self.inactive() || !self.opened {
            return false;
        }
        let visible = self.visible_indices();
        if visible.is_empty() {
            return false;
        }
        let current = self
            .highlighted
            .and_then(|index| visible.iter().position(|candidate| *candidate == index));
        let next = match current {
            Some(index) => {
                let len = visible.len() as i32;
                visible[((index as i32 + delta).rem_euclid(len)) as usize]
            }
            None if delta >= 0 => visible[0],
            None => *visible.last().expect("visible is non-empty"),
        };
        if self.highlighted == Some(next) {
            return false;
        }
        self.highlighted = Some(next);
        true
    }

    pub fn commit_highlighted(&mut self) -> Option<SearchDropdownEvent> {
        let index = self.highlighted?;
        self.select_index(index)
    }

    pub fn replace_selection(&mut self, text: &str) -> bool {
        if !self.state.replace_selection(text) {
            return false;
        }
        self.query = self.state.value.clone();
        self.highlighted = self.first_visible();
        true
    }

    pub fn select_index(&mut self, index: usize) -> Option<SearchDropdownEvent> {
        let option = self.options.get(index)?;
        if self.inactive() || !option_matches(option, &self.query) {
            return None;
        }
        let value = Arc::clone(&option.value);
        self.value = Some(Arc::clone(&value));
        let _ = self.close();
        Some(SearchDropdownEvent::Select(value))
    }

    fn first_visible(&self) -> Option<usize> {
        self.visible_indices().into_iter().next()
    }

    fn option_data(&self) -> Vec<SelectOptionData> {
        let visible = self.visible_indices();
        visible
            .into_iter()
            .map(|index| {
                let option = &self.options[index];
                SelectOptionData {
                    label: Arc::clone(&option.label),
                    hint: option.hint.clone(),
                    disabled: false,
                    checked: false,
                    icon: None,
                }
            })
            .collect()
    }

    fn highlighted_visible(&self) -> Option<usize> {
        let highlighted = self.highlighted?;
        self.visible_indices()
            .into_iter()
            .position(|index| index == highlighted)
    }
}

impl ComponentView for SearchDropdown {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "search-dropdown".into(),
        }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let (label, placeholder) = self.display_label();
        let visual = StandardVisual::Select {
            label: Arc::clone(&label),
            placeholder,
            size: self.size,
            opened: self.opened && !self.inactive(),
            invalid: self.invalid,
            loading: self.loading,
            options: self.option_data().into(),
            highlighted: self.highlighted_visible(),
        };
        if world.standard_visual(id) != Some(visual.clone()) {
            mutations.set_standard_visual(id, Some(visual));
        }
        if world.text(id) != Some(label.as_ref()) {
            mutations.set_text(
                id,
                TextContent {
                    value: label.to_string(),
                },
            );
        }
        if self.opened && !self.inactive() {
            if world.text_input(id) != Some(&self.state) {
                mutations.set_text_input(id, Some(self.state.clone()));
            }
        } else if world.text_input(id).is_some() {
            mutations.set_text_input(id, None);
        }
        let mut style = self.style.clone();
        style.foreground = Some(if self.inactive() || placeholder {
            nana_ui_core::SemanticColorRole::Faint
        } else {
            nana_ui_core::SemanticColorRole::Text
        });
        style.background = Some(if self.inactive() {
            nana_ui_core::SemanticColorRole::Subtle
        } else {
            nana_ui_core::SemanticColorRole::Background
        });
        style.border = Some(if self.invalid {
            nana_ui_core::SemanticColorRole::Danger
        } else if self.opened {
            nana_ui_core::SemanticColorRole::BorderSoft
        } else {
            nana_ui_core::SemanticColorRole::Border
        });
        style.interaction.hovered.border = Some(nana_ui_core::SemanticColorRole::BorderStrong);
        style.interaction.focused.border = Some(nana_ui_core::SemanticColorRole::BorderStrong);
        style.interaction.disabled = SemanticPaint {
            foreground: Some(nana_ui_core::SemanticColorRole::Faint),
            background: Some(nana_ui_core::SemanticColorRole::Subtle),
            border: Some(nana_ui_core::SemanticColorRole::Border),
        };
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: !self.inactive(),
                focusable: !self.inactive(),
            },
            AccessibilityState {
                role: AccessibilityRole::ComboBox,
                label: Some(label),
                value: self.value.clone(),
                disabled: self.inactive(),
                busy: self.loading,
                invalid: self.invalid,
                editable: self.opened && !self.inactive(),
                selected: Some(self.opened && !self.inactive()),
                ..AccessibilityState::default()
            },
        );
    }
}

pub(crate) fn activate_search_dropdown_at(
    dropdown: &mut SearchDropdown,
    menu: Option<&crate::SelectMenuGeometry>,
    field: crate::LayoutBox,
    x: f32,
    y: f32,
) -> Option<SearchDropdownEvent> {
    if dropdown.opened {
        if let Some(menu) = menu
            && let Some(visible) = select_option_at(menu, x, y)
        {
            let index = dropdown.visible_indices().get(visible).copied()?;
            return dropdown.select_index(index);
        }
        if field.contains(x, y) {
            return None;
        }
        return dropdown.close();
    }
    if field.contains(x, y) {
        dropdown.toggle_open()
    } else {
        None
    }
}

fn option_matches(option: &SearchDropdownOption, query: &str) -> bool {
    query_matches(&option.label, query)
        || query_matches(&option.value, query)
        || option
            .hint
            .as_ref()
            .is_some_and(|hint| query_matches(hint, query))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn sample() -> SearchDropdown {
        SearchDropdown::new(None::<&str>)
            .placeholder("搜索选项")
            .options([
                SearchDropdownOption::new("1", "第一个选项").hint("Alpha"),
                SearchDropdownOption::new("2", "第二个选项").hint("Beta"),
                SearchDropdownOption::new("3", "第三个选项").hint("Gamma"),
            ])
    }

    #[test]
    fn query_filters_label_and_hint() {
        let mut dropdown = sample().opened(true);
        dropdown.set_query("beta");
        assert_eq!(dropdown.visible_indices(), vec![1]);
        let event = dropdown.select_index(1).expect("select");
        assert_eq!(event, SearchDropdownEvent::Select(Arc::from("2")));
        assert!(!dropdown.opened);
        assert_eq!(dropdown.value.as_deref(), Some("2"));
    }

    #[test]
    fn opened_field_projects_the_query() {
        let mut context = AppContext::new();
        let dropdown = context
            .create_component(document(), sample().query("Be").opened(true))
            .unwrap();
        let id = dropdown.stable_id();
        assert_eq!(context.world().text(id), Some("Be"));
        assert!(context.world().text_input(id).is_some());
        let Some(StandardVisual::Select {
            opened: true,
            options,
            ..
        }) = context.world().standard_visual(id)
        else {
            panic!("search visual");
        };
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].hint.as_deref(), Some("Beta"));
    }

    #[test]
    fn opened_search_commits_ime_into_the_query() {
        let mut context = AppContext::new();
        let dropdown = context
            .create_component(document(), sample().opened(true))
            .unwrap();
        context
            .focus_node(document(), dropdown.stable_id())
            .unwrap();
        assert!(context.commit_ime(document(), "Beta").unwrap());
        context
            .read(dropdown, |dropdown| {
                assert_eq!(dropdown.query, "Beta");
                assert_eq!(dropdown.visible_indices(), vec![1]);
            })
            .unwrap();
    }
}
