//! Which items of a keyed collection are selected, independent of any control.
//!
//! [`SelectionState`] holds the application's own item keys: the selected
//! set, the *current* item (where the keyboard is) and the *anchor* a range
//! starts from. It never stores positions, rows or nodes, so the same state
//! serves an `each_virtual` list, a `List`, a grid and a virtual table, and a
//! row that is scrolled away (not built) stays selected.
//!
//! Anything that needs order — a Shift range, the arrow keys, select-all,
//! dropping keys the data no longer has — reads it from a [`SelectionOrder`]
//! the caller passes in: a slice of keys, or [`keyed_order`] over the
//! application's items. The order is never copied in.
//!
//! One entry point, [`SelectionState::apply`], takes the
//! [`SelectionGesture`] a press or a key produced, so pointer and keyboard
//! follow the same rules:
//!
//! | gesture | Single | Multiple |
//! | --- | --- | --- |
//! | press | select only it | select only it |
//! | Ctrl press | select it, or clear it if selected | add it or take it out |
//! | Shift press | select only it | the range from the anchor, replacing |
//! | Ctrl+Shift press | select only it | add the range from the anchor |
//! | move (arrow) | select only it | select only it |
//! | Ctrl move | select only it | move the current item, select nothing |
//! | Shift move | select only it | the range from the anchor, replacing |
//! | toggle (Ctrl+Space) | as Ctrl press | as Ctrl press |
//! | select all (Ctrl+A) | nothing | every selectable item |
//!
//! A press or toggle makes the item both current and the anchor; a range
//! keeps the anchor and makes its far end current.

use std::collections::HashSet;
use std::hash::Hash;

/// How many items a [`SelectionState`] may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SelectionMode {
    /// At most one item.
    #[default]
    Single,
    /// Any number: Ctrl toggles an item, Shift selects a range.
    Multiple,
}

/// The modifier keys a press or a keyboard move came with, by what they mean
/// to a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SelectionModifiers {
    /// Ctrl (Cmd on macOS): a press adds or removes one item; a keyboard move
    /// moves the current item without selecting it.
    pub toggle: bool,
    /// Shift: select the range from the anchor.
    pub extend: bool,
}

impl SelectionModifiers {
    pub const NONE: Self = Self {
        toggle: false,
        extend: false,
    };
    pub const TOGGLE: Self = Self {
        toggle: true,
        extend: false,
    };
    pub const EXTEND: Self = Self {
        toggle: false,
        extend: true,
    };
}

/// One selection trigger, from a pointer or the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionGesture {
    /// A press on the item.
    Press(SelectionModifiers),
    /// The keyboard moved to the item (an arrow, Home / End, Page Up / Down).
    Move(SelectionModifiers),
    /// Add the item or take it out, the rest kept (Ctrl+Space).
    Toggle,
    /// Every selectable item (Ctrl+A). Only in [`SelectionMode::Multiple`].
    SelectAll,
}

/// A keyboard step through the order, for [`SelectionState::navigate`].
/// Steps skip items that are not selectable and stop at the ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionStep {
    Previous,
    Next,
    First,
    Last,
    /// This many items back (Page Up).
    Backward(usize),
    /// This many items on (Page Down).
    Forward(usize),
}

/// The items a selection is over, in the order the user sees them.
///
/// Implemented for slices and vectors of keys and, through [`keyed_order`],
/// for the application's own items. Only [`Self::len`] and [`Self::key_at`]
/// are required; implement [`Self::index_of`] with an index the application
/// keeps when its lists are large, since the default scans.
pub trait SelectionOrder<K: PartialEq> {
    fn len(&self) -> usize;

    fn key_at(&self, index: usize) -> Option<K>;

    /// Where `key` is. The default walks the order.
    fn index_of(&self, key: &K) -> Option<usize> {
        (0..self.len()).find(|&index| self.key_at(index).as_ref() == Some(key))
    }

    /// Whether the item at `index` can be selected and stepped onto: `false`
    /// for a disabled row or a group heading. Every item is by default.
    fn is_selectable(&self, index: usize) -> bool {
        let _ = index;
        true
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<K: PartialEq + Clone> SelectionOrder<K> for [K] {
    fn len(&self) -> usize {
        <[K]>::len(self)
    }

    fn key_at(&self, index: usize) -> Option<K> {
        self.get(index).cloned()
    }

    fn index_of(&self, key: &K) -> Option<usize> {
        self.iter().position(|candidate| candidate == key)
    }
}

impl<K: PartialEq + Clone> SelectionOrder<K> for Vec<K> {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn key_at(&self, index: usize) -> Option<K> {
        self.get(index).cloned()
    }

    fn index_of(&self, key: &K) -> Option<usize> {
        self.iter().position(|candidate| candidate == key)
    }
}

/// [`SelectionOrder`] over the application's items, each keyed by `key`;
/// made by [`keyed_order`].
#[derive(Clone, Copy)]
pub struct KeyedOrder<'a, T, F, S = fn(&T) -> bool> {
    items: &'a [T],
    key: F,
    selectable: S,
}

/// The order of `items` by `key`, every item selectable; narrow it with
/// [`KeyedOrder::selectable`].
pub fn keyed_order<T, K, F>(items: &[T], key: F) -> KeyedOrder<'_, T, F>
where
    F: Fn(&T) -> K,
{
    KeyedOrder {
        items,
        key,
        selectable: |_| true,
    }
}

impl<'a, T, F, S> KeyedOrder<'a, T, F, S> {
    /// Only items `selectable` answers `true` for can be selected or stepped
    /// onto.
    pub fn selectable<G>(self, selectable: G) -> KeyedOrder<'a, T, F, G>
    where
        G: Fn(&T) -> bool,
    {
        KeyedOrder {
            items: self.items,
            key: self.key,
            selectable,
        }
    }
}

impl<T, K, F, S> SelectionOrder<K> for KeyedOrder<'_, T, F, S>
where
    K: PartialEq,
    F: Fn(&T) -> K,
    S: Fn(&T) -> bool,
{
    fn len(&self) -> usize {
        self.items.len()
    }

    fn key_at(&self, index: usize) -> Option<K> {
        self.items.get(index).map(&self.key)
    }

    fn index_of(&self, key: &K) -> Option<usize> {
        self.items.iter().position(|item| (self.key)(item) == *key)
    }

    fn is_selectable(&self, index: usize) -> bool {
        self.items.get(index).is_some_and(&self.selectable)
    }
}

/// The selected keys of a collection, with its current item and range anchor.
/// See the module docs.
#[derive(Debug, Clone)]
pub struct SelectionState<K> {
    mode: SelectionMode,
    selected: HashSet<K>,
    current: Option<K>,
    anchor: Option<K>,
    /// Where the current item last was in an order, to land near it when
    /// the data drops it.
    current_index: usize,
    revision: u64,
}

impl<K: Eq + Hash> PartialEq for SelectionState<K> {
    /// Same mode, selection, current item and anchor; the revision is not
    /// compared.
    fn eq(&self, other: &Self) -> bool {
        self.mode == other.mode
            && self.selected == other.selected
            && self.current == other.current
            && self.anchor == other.anchor
    }
}

impl<K: Eq + Hash> Eq for SelectionState<K> {}

impl<K: Eq + Hash + Clone> Default for SelectionState<K> {
    fn default() -> Self {
        Self::new(SelectionMode::Single)
    }
}

impl<K: Eq + Hash + Clone> SelectionState<K> {
    pub fn new(mode: SelectionMode) -> Self {
        Self {
            mode,
            selected: HashSet::new(),
            current: None,
            anchor: None,
            current_index: 0,
            revision: 0,
        }
    }

    pub fn single() -> Self {
        Self::new(SelectionMode::Single)
    }

    pub fn multiple() -> Self {
        Self::new(SelectionMode::Multiple)
    }

    pub fn mode(&self) -> SelectionMode {
        self.mode
    }

    /// Switch mode. Going to [`SelectionMode::Single`] keeps the current
    /// item if it is selected and clears the rest.
    pub fn set_mode(&mut self, mode: SelectionMode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.mode = mode;
        if mode == SelectionMode::Single && self.selected.len() > 1 {
            let keep = self
                .current
                .as_ref()
                .filter(|current| self.selected.contains(*current))
                .cloned();
            self.selected.clear();
            self.selected.extend(keep);
        }
        self.bump();
        true
    }

    pub fn is_selected(&self, key: &K) -> bool {
        self.selected.contains(key)
    }

    pub fn len(&self) -> usize {
        self.selected.len()
    }

    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    /// The selected keys, in no particular order; [`Self::selected_in`] gives
    /// them in the order's.
    pub fn iter(&self) -> impl Iterator<Item = &K> {
        self.selected.iter()
    }

    /// The selected keys in `order`'s order. Keys `order` does not have are
    /// left out.
    pub fn selected_in<O: SelectionOrder<K> + ?Sized>(&self, order: &O) -> Vec<K> {
        let mut out = Vec::with_capacity(self.selected.len());
        if self.selected.is_empty() {
            return out;
        }
        for index in 0..order.len() {
            if let Some(key) = order.key_at(index)
                && self.selected.contains(&key)
            {
                out.push(key);
                if out.len() == self.selected.len() {
                    break;
                }
            }
        }
        out
    }

    /// The item the keyboard is on; it need not be selected.
    pub fn current(&self) -> Option<&K> {
        self.current.as_ref()
    }

    /// Where a Shift range starts.
    pub fn anchor(&self) -> Option<&K> {
        self.anchor.as_ref()
    }

    /// Bumped by every change: equal before and after, nothing changed.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Select only `key`; it becomes current and the anchor.
    pub fn select_only(&mut self, key: K) -> bool {
        let same = self.selected.len() == 1
            && self.selected.contains(&key)
            && self.current.as_ref() == Some(&key)
            && self.anchor.as_ref() == Some(&key);
        if same {
            return false;
        }
        self.selected.clear();
        self.selected.insert(key.clone());
        self.anchor = Some(key.clone());
        self.current = Some(key);
        self.bump();
        true
    }

    /// Add `key` or take it out; it becomes current and the anchor. In
    /// [`SelectionMode::Single`] selecting it drops any other.
    pub fn toggle(&mut self, key: K) -> bool {
        if self.selected.remove(&key) {
            self.anchor = Some(key.clone());
            self.current = Some(key);
            self.bump();
            return true;
        }
        if self.mode == SelectionMode::Single {
            self.selected.clear();
        }
        self.selected.insert(key.clone());
        self.anchor = Some(key.clone());
        self.current = Some(key);
        self.bump();
        true
    }

    /// Take `key` out of the selection; current and anchor stay.
    pub fn deselect(&mut self, key: &K) -> bool {
        if !self.selected.remove(key) {
            return false;
        }
        self.bump();
        true
    }

    /// Make `key` current without selecting it (Ctrl with an arrow).
    pub fn set_current(&mut self, key: K) -> bool {
        if self.current.as_ref() == Some(&key) {
            return false;
        }
        self.current = Some(key);
        self.bump();
        true
    }

    /// Put the anchor at `key`.
    pub fn set_anchor(&mut self, key: K) -> bool {
        if self.anchor.as_ref() == Some(&key) {
            return false;
        }
        self.anchor = Some(key);
        self.bump();
        true
    }

    /// Select the selectable items from the anchor to `to`, `to` becoming
    /// current; with `additive` the range adds to the selection instead of
    /// replacing it. Without an anchor in `order`, `to` becomes the anchor.
    /// In [`SelectionMode::Single`] this selects only `to`.
    pub fn select_range<O: SelectionOrder<K> + ?Sized>(
        &mut self,
        to: K,
        order: &O,
        additive: bool,
    ) -> bool {
        if self.mode == SelectionMode::Single {
            return self.select_only(to);
        }
        let Some(end) = order.index_of(&to) else {
            return false;
        };
        let anchored = self
            .anchor
            .as_ref()
            .and_then(|anchor| order.index_of(anchor));
        let start = anchored.unwrap_or(end);
        let (low, high) = (start.min(end), start.max(end));
        if anchored.is_some()
            && !additive
            && self.current.as_ref() == Some(&to)
            && self.range_matches(order, low, high)
        {
            self.current_index = end;
            return false;
        }
        if !additive {
            self.selected.clear();
        }
        for index in low..=high {
            if order.is_selectable(index)
                && let Some(key) = order.key_at(index)
            {
                self.selected.insert(key);
            }
        }
        if anchored.is_none() {
            self.anchor = Some(to.clone());
        }
        self.current = Some(to);
        self.current_index = end;
        self.bump();
        true
    }

    /// Whether the selection is exactly the selectable items of
    /// `low..=high`.
    fn range_matches<O: SelectionOrder<K> + ?Sized>(
        &self,
        order: &O,
        low: usize,
        high: usize,
    ) -> bool {
        let mut count = 0;
        for index in low..=high {
            if !order.is_selectable(index) {
                continue;
            }
            match order.key_at(index) {
                Some(key) if self.selected.contains(&key) => count += 1,
                _ => return false,
            }
        }
        count == self.selected.len()
    }

    /// Every selectable item of `order`. Only in
    /// [`SelectionMode::Multiple`]; current and anchor stay.
    pub fn select_all<O: SelectionOrder<K> + ?Sized>(&mut self, order: &O) -> bool {
        if self.mode != SelectionMode::Multiple {
            return false;
        }
        let mut changed = false;
        for index in 0..order.len() {
            if order.is_selectable(index)
                && let Some(key) = order.key_at(index)
            {
                changed |= self.selected.insert(key);
            }
        }
        if changed {
            self.bump();
        }
        changed
    }

    /// Select nothing; current and anchor stay.
    pub fn clear(&mut self) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        self.selected.clear();
        self.bump();
        true
    }

    /// Replace the selection with `keys` (in [`SelectionMode::Single`], the
    /// last of them). Current and anchor stay.
    pub fn set_selection(&mut self, keys: impl IntoIterator<Item = K>) -> bool {
        let mut next: HashSet<K> = match self.mode {
            SelectionMode::Multiple => keys.into_iter().collect(),
            SelectionMode::Single => keys.into_iter().last().into_iter().collect(),
        };
        if next == self.selected {
            return false;
        }
        std::mem::swap(&mut self.selected, &mut next);
        self.bump();
        true
    }

    /// Apply what a press or a key on `key` asks for; see the module docs for
    /// the table. Returns whether anything changed. A press or toggle on an
    /// item `order` does not have, or that is not selectable, changes
    /// nothing; a keyboard move onto one makes it current only.
    pub fn apply<O: SelectionOrder<K> + ?Sized>(
        &mut self,
        gesture: SelectionGesture,
        key: K,
        order: &O,
    ) -> bool {
        if gesture == SelectionGesture::SelectAll {
            return self.select_all(order);
        }
        let index = order.index_of(&key);
        let selectable = index.is_some_and(|index| order.is_selectable(index));
        if let Some(index) = index {
            self.current_index = index;
        }
        let multiple = self.mode == SelectionMode::Multiple;
        match gesture {
            SelectionGesture::SelectAll => unreachable!("handled above"),
            SelectionGesture::Toggle => selectable && self.toggle(key),
            SelectionGesture::Press(modifiers) => {
                if !selectable {
                    false
                } else if multiple && modifiers.extend {
                    self.select_range(key, order, modifiers.toggle)
                } else if modifiers.toggle {
                    self.toggle(key)
                } else {
                    self.select_only(key)
                }
            }
            SelectionGesture::Move(modifiers) => {
                if !selectable || index.is_none() {
                    self.set_current(key)
                } else if multiple && modifiers.extend {
                    self.select_range(key, order, modifiers.toggle)
                } else if multiple && modifiers.toggle {
                    self.set_current(key)
                } else {
                    self.select_only(key)
                }
            }
        }
    }

    /// Step the current item through `order` and apply the move to where it
    /// lands ([`SelectionGesture::Move`] with `modifiers`). Returns the new
    /// current key, or `None` when the step goes nowhere (an end, or nothing
    /// selectable). Without a current item a forward step lands on the first
    /// selectable item and a backward one on the last.
    pub fn navigate<O: SelectionOrder<K> + ?Sized>(
        &mut self,
        step: SelectionStep,
        modifiers: SelectionModifiers,
        order: &O,
    ) -> Option<K> {
        let len = order.len();
        if len == 0 {
            return None;
        }
        let from = self
            .current
            .as_ref()
            .and_then(|current| order.index_of(current));
        let forward = |start: usize| (start..len).find(|&index| order.is_selectable(index));
        let backward = |end: usize| (0..=end).rev().find(|&index| order.is_selectable(index));
        let target = match (step, from) {
            (SelectionStep::First, _) => forward(0),
            (SelectionStep::Last, _) => backward(len - 1),
            (SelectionStep::Next | SelectionStep::Forward(_), None) => forward(0),
            (SelectionStep::Previous | SelectionStep::Backward(_), None) => backward(len - 1),
            (SelectionStep::Next, Some(from)) => forward(from + 1),
            (SelectionStep::Previous, Some(from)) => from.checked_sub(1).and_then(backward),
            (SelectionStep::Forward(count), Some(from)) => {
                let goal = from.saturating_add(count.max(1)).min(len - 1);
                backward(goal)
                    .filter(|&index| index > from)
                    .or_else(|| forward(goal))
            }
            (SelectionStep::Backward(count), Some(from)) => {
                let goal = from.saturating_sub(count.max(1));
                forward(goal)
                    .filter(|&index| index < from)
                    .or_else(|| backward(goal))
            }
        }?;
        if from == Some(target) {
            return None;
        }
        let key = order.key_at(target)?;
        self.apply(SelectionGesture::Move(modifiers), key.clone(), order);
        Some(key)
    }

    /// Bring the state in line with `order` after the data changed: selected
    /// keys `order` no longer has are dropped, a current item that went away
    /// moves to the selectable item nearest where it was, and an anchor that
    /// went away moves to the current item. Reordering keeps everything.
    /// Built rows play no part, so a row scrolled out of a virtual list
    /// keeps its selection. Returns whether anything changed.
    pub fn reconcile<O: SelectionOrder<K> + ?Sized>(&mut self, order: &O) -> bool {
        let mut changed = false;
        if !self.selected.is_empty() {
            let before = self.selected.len();
            if before <= 32 {
                self.selected.retain(|key| order.index_of(key).is_some());
            } else {
                let mut present = 0;
                for index in 0..order.len() {
                    if order
                        .key_at(index)
                        .is_some_and(|key| self.selected.contains(&key))
                    {
                        present += 1;
                    }
                }
                if present < before {
                    let mut kept = HashSet::with_capacity(present);
                    for index in 0..order.len() {
                        if let Some(key) = order.key_at(index)
                            && self.selected.contains(&key)
                        {
                            kept.insert(key);
                        }
                    }
                    self.selected = kept;
                }
            }
            changed |= self.selected.len() != before;
        }
        if let Some(current) = &self.current {
            match order.index_of(current) {
                Some(index) => self.current_index = index,
                None => {
                    let len = order.len();
                    let near = (len > 0).then(|| self.current_index.min(len - 1));
                    let landing = near.and_then(|near| {
                        (0..=near)
                            .rev()
                            .find(|&index| order.is_selectable(index))
                            .or_else(|| (near..len).find(|&index| order.is_selectable(index)))
                    });
                    if let Some(index) = landing {
                        self.current_index = index;
                    }
                    self.current = landing.and_then(|index| order.key_at(index));
                    changed = true;
                }
            }
        }
        if let Some(anchor) = &self.anchor
            && order.index_of(anchor).is_none()
        {
            self.anchor = self.current.clone();
            changed = true;
        }
        if changed {
            self.bump();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(n: u32) -> Vec<u32> {
        (0..n).collect()
    }

    fn sorted(state: &SelectionState<u32>) -> Vec<u32> {
        let mut out = state.iter().copied().collect::<Vec<_>>();
        out.sort_unstable();
        out
    }

    #[test]
    fn plain_press_selects_one_and_moves_current_and_anchor() {
        let order = keys(5);
        let mut state = SelectionState::multiple();
        assert!(state.apply(SelectionGesture::Press(SelectionModifiers::NONE), 2, &order));
        assert_eq!(sorted(&state), [2]);
        assert_eq!(state.current(), Some(&2));
        assert_eq!(state.anchor(), Some(&2));
        assert!(
            !state.apply(SelectionGesture::Press(SelectionModifiers::NONE), 2, &order),
            "pressing it again changes nothing"
        );
    }

    #[test]
    fn ctrl_press_toggles_and_shift_press_ranges_from_the_anchor() {
        let order = keys(10);
        let mut state = SelectionState::multiple();
        state.apply(SelectionGesture::Press(SelectionModifiers::NONE), 2, &order);
        state.apply(
            SelectionGesture::Press(SelectionModifiers::TOGGLE),
            6,
            &order,
        );
        assert_eq!(sorted(&state), [2, 6]);
        assert_eq!(state.anchor(), Some(&6));

        // Shift: the range from the anchor replaces the selection.
        state.apply(
            SelectionGesture::Press(SelectionModifiers::EXTEND),
            4,
            &order,
        );
        assert_eq!(sorted(&state), [4, 5, 6]);
        assert_eq!(state.anchor(), Some(&6), "a range keeps the anchor");
        assert_eq!(state.current(), Some(&4));

        // Shift again, the other way: still from the same anchor.
        state.apply(
            SelectionGesture::Press(SelectionModifiers::EXTEND),
            8,
            &order,
        );
        assert_eq!(sorted(&state), [6, 7, 8]);

        // Ctrl+Shift adds a range instead.
        state.apply(
            SelectionGesture::Press(SelectionModifiers::TOGGLE),
            0,
            &order,
        );
        let both = SelectionModifiers {
            toggle: true,
            extend: true,
        };
        state.apply(SelectionGesture::Press(both), 1, &order);
        assert_eq!(sorted(&state), [0, 1, 6, 7, 8]);

        // Ctrl on a selected item takes it out.
        state.apply(
            SelectionGesture::Press(SelectionModifiers::TOGGLE),
            7,
            &order,
        );
        assert_eq!(sorted(&state), [0, 1, 6, 8]);
    }

    #[test]
    fn keyboard_moves_select_extend_or_only_move_current() {
        let order = keys(10);
        let mut state = SelectionState::multiple();
        assert_eq!(
            state.navigate(SelectionStep::Next, SelectionModifiers::NONE, &order),
            Some(0),
            "with nothing current a forward step lands on the first item"
        );
        assert_eq!(sorted(&state), [0]);
        state.navigate(SelectionStep::Next, SelectionModifiers::EXTEND, &order);
        state.navigate(SelectionStep::Next, SelectionModifiers::EXTEND, &order);
        assert_eq!(sorted(&state), [0, 1, 2]);
        assert_eq!(state.anchor(), Some(&0));

        // Ctrl with an arrow moves the current item only.
        state.navigate(
            SelectionStep::Forward(4),
            SelectionModifiers::TOGGLE,
            &order,
        );
        assert_eq!(state.current(), Some(&6));
        assert_eq!(sorted(&state), [0, 1, 2]);
        // ... and Ctrl+Space adds it.
        state.apply(SelectionGesture::Toggle, 6, &order);
        assert_eq!(sorted(&state), [0, 1, 2, 6]);

        assert_eq!(
            state.navigate(SelectionStep::Last, SelectionModifiers::NONE, &order),
            Some(9)
        );
        assert_eq!(sorted(&state), [9]);
        assert_eq!(
            state.navigate(SelectionStep::Next, SelectionModifiers::NONE, &order),
            None,
            "no wrap past the end"
        );
        assert_eq!(
            state.navigate(
                SelectionStep::Backward(100),
                SelectionModifiers::NONE,
                &order
            ),
            Some(0)
        );
    }

    #[test]
    fn single_mode_never_holds_two() {
        let order = keys(5);
        let mut state = SelectionState::single();
        state.apply(SelectionGesture::Press(SelectionModifiers::NONE), 1, &order);
        state.apply(
            SelectionGesture::Press(SelectionModifiers::TOGGLE),
            3,
            &order,
        );
        assert_eq!(sorted(&state), [3]);
        state.apply(
            SelectionGesture::Press(SelectionModifiers::EXTEND),
            0,
            &order,
        );
        assert_eq!(sorted(&state), [0]);
        assert!(!state.apply(SelectionGesture::SelectAll, 0, &order));
        // Ctrl on the selected item clears it.
        state.apply(
            SelectionGesture::Press(SelectionModifiers::TOGGLE),
            0,
            &order,
        );
        assert!(state.is_empty());

        let mut multiple = SelectionState::multiple();
        multiple.select_all(&order);
        multiple.set_current(2);
        assert!(multiple.set_mode(SelectionMode::Single));
        assert_eq!(sorted(&multiple), [2]);
    }

    #[test]
    fn unselectable_items_are_skipped_by_ranges_steps_and_select_all() {
        let items = (0..6u32).collect::<Vec<_>>();
        let order = keyed_order(&items, |item| *item).selectable(|item| item % 2 == 0);
        let mut state = SelectionState::multiple();
        assert!(
            !state.apply(SelectionGesture::Press(SelectionModifiers::NONE), 1, &order),
            "a disabled item does not select"
        );
        state.apply(SelectionGesture::Press(SelectionModifiers::NONE), 0, &order);
        state.apply(
            SelectionGesture::Press(SelectionModifiers::EXTEND),
            4,
            &order,
        );
        assert_eq!(sorted(&state), [0, 2, 4]);
        state.apply(SelectionGesture::Press(SelectionModifiers::NONE), 0, &order);
        assert_eq!(
            state.navigate(SelectionStep::Next, SelectionModifiers::NONE, &order),
            Some(2)
        );
        state.clear();
        state.select_all(&order);
        assert_eq!(sorted(&state), [0, 2, 4]);
    }

    #[test]
    fn reconcile_drops_removed_keys_and_keeps_reordered_ones() {
        let mut order = keys(10);
        let mut state = SelectionState::multiple();
        state.apply(SelectionGesture::Press(SelectionModifiers::NONE), 3, &order);
        state.apply(
            SelectionGesture::Press(SelectionModifiers::EXTEND),
            6,
            &order,
        );
        assert_eq!(sorted(&state), [3, 4, 5, 6]);

        // Reversed: nothing is lost, and a Shift range now runs the new way.
        order.reverse();
        assert!(!state.reconcile(&order));
        assert_eq!(sorted(&state), [3, 4, 5, 6]);

        // 5 and 6 (the current item) go away.
        order.retain(|key| *key != 5 && *key != 6);
        let revision = state.revision();
        assert!(state.reconcile(&order));
        assert!(state.revision() > revision);
        assert_eq!(sorted(&state), [3, 4]);
        let current = *state.current().unwrap();
        assert!(
            order.contains(&current),
            "the current item moved to a live one"
        );
        assert_eq!(state.anchor(), Some(&3));

        // Everything goes.
        assert!(state.reconcile(&Vec::<u32>::new()));
        assert!(state.is_empty());
        assert_eq!(state.current(), None);
        assert_eq!(state.anchor(), None);
    }

    #[test]
    fn reconcile_of_a_large_selection_scans_the_order_once() {
        let order = keys(1_000);
        let mut state = SelectionState::multiple();
        state.select_all(&order);
        let mut fewer = order.clone();
        fewer.retain(|key| key % 3 != 0);
        assert!(state.reconcile(&fewer));
        assert_eq!(state.len(), fewer.len());
        assert_eq!(state.selected_in(&fewer), fewer);
    }

    #[test]
    fn selection_survives_for_keys_that_are_not_built() {
        // The state is keys only: an item nobody built (scrolled away in a
        // virtual list) selects, ranges and reports like any other.
        let order = keys(100_000);
        let mut state = SelectionState::multiple();
        state.apply(
            SelectionGesture::Press(SelectionModifiers::NONE),
            10,
            &order,
        );
        state.apply(
            SelectionGesture::Press(SelectionModifiers::EXTEND),
            90_000,
            &order,
        );
        assert_eq!(state.len(), 89_991);
        assert!(state.is_selected(&50_000));
        assert_eq!(state.selected_in(&order).first(), Some(&10));
    }

    #[test]
    fn repeating_a_range_does_not_report_a_change() {
        let order = keys(10);
        let mut state = SelectionState::multiple();
        state.apply(SelectionGesture::Press(SelectionModifiers::NONE), 2, &order);
        assert!(state.select_range(5, &order, false));
        let revision = state.revision();
        assert!(!state.select_range(5, &order, false));
        assert_eq!(state.revision(), revision);
    }
}
