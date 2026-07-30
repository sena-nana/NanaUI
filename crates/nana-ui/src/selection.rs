#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMove {
    Previous,
    Next,
    First,
    Last,
}

/// Single-selection state with LiliaUI-compatible roving navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SingleSelection {
    selected: usize,
}

impl SingleSelection {
    pub const fn new(selected: usize) -> Self {
        Self { selected }
    }

    pub const fn selected(self) -> usize {
        self.selected
    }

    pub fn select(&mut self, index: usize, enabled: &[bool]) -> bool {
        if enabled.get(index).copied() != Some(true) {
            return false;
        }
        self.selected = index;
        true
    }

    pub fn navigate(&mut self, movement: SelectionMove, enabled: &[bool]) -> Option<usize> {
        let available: Vec<usize> = enabled
            .iter()
            .enumerate()
            .filter_map(|(index, enabled)| enabled.then_some(index))
            .collect();
        if available.is_empty() {
            return None;
        }

        let target = match movement {
            SelectionMove::First => available[0],
            SelectionMove::Last => available[available.len() - 1],
            SelectionMove::Previous | SelectionMove::Next => {
                let current = available.iter().position(|index| *index == self.selected);
                let start = current.unwrap_or_else(|| {
                    if movement == SelectionMove::Next {
                        available.len() - 1
                    } else {
                        0
                    }
                });
                let offset = if movement == SelectionMove::Next {
                    1
                } else {
                    available.len() - 1
                };
                available[(start + offset) % available.len()]
            }
        };

        self.selected = target;
        Some(target)
    }
}

#[cfg(test)]
mod tests {
    use super::{SelectionMove, SingleSelection};

    #[test]
    fn roving_selection_wraps_and_skips_disabled_items() {
        let enabled = [true, false, true];
        let mut selection = SingleSelection::new(0);

        assert_eq!(selection.navigate(SelectionMove::Next, &enabled), Some(2));
        assert_eq!(selection.navigate(SelectionMove::Next, &enabled), Some(0));
        assert_eq!(
            selection.navigate(SelectionMove::Previous, &enabled),
            Some(2)
        );
        assert!(!selection.select(1, &enabled));
        assert_eq!(selection.selected(), 2);
    }

    #[test]
    fn home_and_end_select_the_available_boundaries() {
        let enabled = [false, true, true, false];
        let mut selection = SingleSelection::new(2);

        assert_eq!(selection.navigate(SelectionMove::First, &enabled), Some(1));
        assert_eq!(selection.navigate(SelectionMove::Last, &enabled), Some(2));
    }
}
