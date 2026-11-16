//! Slots addressed by a number: one run from zero while the numbers in use
//! are close together, pages once they are not.
//!
//! Node ids are handed out in order and never reused, so a table indexed by
//! id as one vector is as long as the highest id it has seen: a session that
//! had created 65,000 nodes kept 17 MB of empty record slots, and as much
//! again in the layout tables, however few nodes were live. A page built at
//! once has ids in a row, and one vector is the fastest thing to index them
//! with (the run of a [`Slots`]); when what is held thins out to an eighth of the
//! span it covers, the entries move to pages ([`Paged`]): only the pages
//! that hold something are allocated, a page whose last entry goes is given
//! back, and the directory of pages covers no more than the span from the
//! lowest page in use to the highest.

/// Slots to a page. Ids arrive in runs (a view is built at once), so a page
/// is usually full or empty; a smaller one costs a directory entry more
/// often, a larger one keeps more empty slots beside a lone survivor.
const PAGE_BITS: u32 = 6;
const PAGE: usize = 1 << PAGE_BITS;
const SLOT: usize = PAGE - 1;

/// A page: its slots and nothing else, so a page of records is the
/// power of two an allocator hands out without rounding.
type Page<T> = Box<[Option<T>; PAGE]>;

fn empty_page<T>() -> Page<T> {
    Box::new([const { None }; PAGE])
}

#[derive(Clone)]
pub(crate) struct Paged<T> {
    /// Page `base + n` at `n`; `None` where no slot of the page is in use.
    pages: Vec<Option<Page<T>>>,
    /// Which slots of page `base + n` hold a value, a bit each.
    used: Vec<u64>,
    base: usize,
    len: usize,
    /// One emptied page kept back, so entries coming and going across a
    /// page's first slot do not allocate a page each time.
    spare: Option<Page<T>>,
}

impl<T> Default for Paged<T> {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            used: Vec::new(),
            base: 0,
            len: 0,
            spare: None,
        }
    }
}

impl<T> Paged<T> {
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        let page = (index >> PAGE_BITS).wrapping_sub(self.base);
        self.pages.get(page)?.as_deref()?[index & SLOT].as_ref()
    }

    #[inline]
    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        let page = (index >> PAGE_BITS).wrapping_sub(self.base);
        self.pages.get_mut(page)?.as_deref_mut()?[index & SLOT].as_mut()
    }

    /// Puts `value` at `index` and gives back what was there.
    pub(crate) fn insert(&mut self, index: usize, value: T) -> Option<T> {
        let number = index >> PAGE_BITS;
        if self.pages.is_empty() {
            self.base = number;
        } else if number < self.base {
            let before = self.base - number;
            self.pages
                .splice(0..0, std::iter::repeat_with(|| None).take(before));
            self.used.splice(0..0, std::iter::repeat_n(0, before));
            self.base = number;
        }
        let at = number - self.base;
        if at >= self.pages.len() {
            self.pages.resize_with(at + 1, || None);
            self.used.resize(at + 1, 0);
        }
        let spare = &mut self.spare;
        let page = self.pages[at].get_or_insert_with(|| spare.take().unwrap_or_else(empty_page));
        let previous = page[index & SLOT].replace(value);
        if previous.is_none() {
            self.used[at] |= 1 << (index & SLOT);
            self.len += 1;
        }
        previous
    }

    pub(crate) fn remove(&mut self, index: usize) -> Option<T> {
        let at = (index >> PAGE_BITS).wrapping_sub(self.base);
        let removed = self.pages.get_mut(at)?.as_deref_mut()?[index & SLOT].take()?;
        self.len -= 1;
        self.used[at] &= !(1 << (index & SLOT));
        if self.used[at] == 0 {
            self.spare = self.pages[at].take();
            self.trim();
        }
        Some(removed)
    }

    /// Drops the directory entries before the first page in use and after
    /// the last.
    fn trim(&mut self) {
        while matches!(self.pages.last(), Some(None)) {
            self.pages.pop();
        }
        self.used.truncate(self.pages.len());
        let leading = self.pages.iter().take_while(|page| page.is_none()).count();
        if leading > 0 {
            self.pages.drain(..leading);
            self.used.drain(..leading);
            self.base += leading;
        }
    }

    /// Keeps the values `keep` answers for, given each with its index.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(usize, &mut T) -> bool) {
        let base = self.base;
        for (number, (entry, used)) in self.pages.iter_mut().zip(&mut self.used).enumerate() {
            let Some(page) = entry.as_deref_mut() else {
                continue;
            };
            let first = (base + number) << PAGE_BITS;
            let mut left = *used;
            while left != 0 {
                let slot = left.trailing_zeros() as usize;
                left &= left - 1;
                if page[slot]
                    .as_mut()
                    .is_some_and(|value| !keep(first | slot, value))
                {
                    page[slot] = None;
                    *used &= !(1 << slot);
                    self.len -= 1;
                }
            }
            if *used == 0 {
                self.spare = entry.take();
            }
        }
        self.trim();
    }

    /// Every value with its index, in index order.
    pub(crate) fn iter(&self) -> Iter<'_, T> {
        Iter {
            pages: &self.pages,
            used: &self.used,
            base: self.base,
            page: 0,
            left: self.used.first().copied().unwrap_or(0),
        }
    }

    /// Pages allocated, for tests of what a table holds.
    #[cfg(test)]
    pub(crate) fn pages(&self) -> usize {
        self.pages.iter().flatten().count()
    }

    /// Directory entries, for tests of what a table holds.
    #[cfg(test)]
    pub(crate) fn span(&self) -> usize {
        self.pages.len()
    }
}

/// [`Paged::iter`].
pub(crate) struct Iter<'a, T> {
    pages: &'a [Option<Page<T>>],
    used: &'a [u64],
    base: usize,
    page: usize,
    /// The slots of `page` still to give.
    left: u64,
}

impl<T> Clone for Iter<'_, T> {
    fn clone(&self) -> Self {
        Self {
            pages: self.pages,
            used: self.used,
            base: self.base,
            page: self.page,
            left: self.left,
        }
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = (usize, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        while self.left == 0 {
            self.page += 1;
            self.left = *self.used.get(self.page)?;
        }
        let slot = self.left.trailing_zeros() as usize;
        self.left &= self.left - 1;
        let value = self.pages[self.page].as_deref()?[slot].as_ref()?;
        Some((((self.page + self.base) << PAGE_BITS) | slot, value))
    }
}

/// Every value with its index, in index order, by value.
pub(crate) struct IntoIter<T> {
    pages: std::iter::Zip<std::vec::IntoIter<Option<Page<T>>>, std::vec::IntoIter<u64>>,
    page: Option<Page<T>>,
    /// The number of the page after `page`.
    next: usize,
    left: u64,
}

impl<T> Iterator for IntoIter<T> {
    type Item = (usize, T);

    fn next(&mut self) -> Option<(usize, T)> {
        while self.left == 0 {
            let (page, used) = self.pages.next()?;
            self.page = page;
            self.next += 1;
            self.left = used;
        }
        let slot = self.left.trailing_zeros() as usize;
        self.left &= self.left - 1;
        let value = self.page.as_deref_mut()?[slot].take()?;
        Some((((self.next - 1) << PAGE_BITS) | slot, value))
    }
}

impl<T> IntoIterator for Paged<T> {
    type Item = (usize, T);
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> IntoIter<T> {
        IntoIter {
            next: self.base,
            pages: self.pages.into_iter().zip(self.used),
            page: None,
            left: 0,
        }
    }
}

/// Empty slots a run may hold beside each entry, and on top of that, before
/// its entries move to pages.
const SPARSE: usize = 8;
const SPARSE_SLACK: usize = 1024;

/// One run of slots from index zero while the indices in use are close
/// together, [`Paged`] once they are not.
///
/// A lookup tries the run first and that is all it does while the table is
/// dense: one bounds check and the slot, as a plain vector costs. On pages
/// the run is empty and the same check falls through to them.
#[derive(Clone)]
pub(crate) struct Slots<T> {
    /// Slot `n` at `n`; empty once the entries are on pages.
    run: Vec<Option<T>>,
    /// Entries in `run`.
    held: usize,
    paged: Paged<T>,
    on_pages: bool,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self {
            run: Vec::new(),
            held: 0,
            paged: Paged::default(),
            on_pages: false,
        }
    }
}

impl<T> Slots<T> {
    pub(crate) fn len(&self) -> usize {
        self.held + self.paged.len()
    }

    /// The slot the run has for `index`, full or empty. `None` past the
    /// run: the value is on pages if anywhere ([`Self::past_run`]).
    ///
    /// This is the whole lookup while the table is dense, kept apart from
    /// the rest so that it is small enough to be inlined wherever a node is
    /// looked up; with the pages in the same body it was left as a call.
    #[inline(always)]
    pub(crate) fn run_slot(&self, index: usize) -> Option<&Option<T>> {
        self.run.get(index)
    }

    #[inline(always)]
    pub(crate) fn run_slot_mut(&mut self, index: usize) -> Option<&mut Option<T>> {
        self.run.get_mut(index)
    }

    /// The value at `index` when it is not in the run.
    #[inline]
    pub(crate) fn past_run(&self, index: usize) -> Option<&T> {
        if self.on_pages {
            return self.paged.get(index);
        }
        None
    }

    #[inline]
    pub(crate) fn past_run_mut(&mut self, index: usize) -> Option<&mut T> {
        if self.on_pages {
            return self.paged.get_mut(index);
        }
        None
    }

    #[cfg(test)]
    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        match self.run_slot(index) {
            Some(slot) => slot.as_ref(),
            None => self.past_run(index),
        }
    }

    #[cfg(test)]
    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index < self.run.len() {
            return self.run[index].as_mut();
        }
        self.past_run_mut(index)
    }

    /// Whether a run reaching `slots` slots for `entries` entries is still
    /// worth holding as one.
    fn dense(slots: usize, entries: usize) -> bool {
        slots <= entries.saturating_mul(SPARSE) + SPARSE_SLACK
    }

    /// Grows `run` to hold at least `required_len` slots with bounded slack (+25% above 256 slots,
    /// +12.5% above 512 slots) so large dense tables do not keep 2x doubling slop in resident heap.
    fn grow_run(&mut self, required_len: usize) {
        let cap = self.run.capacity();
        if required_len <= cap {
            return;
        }
        let target = if cap == 0 && required_len >= 256 {
            required_len
        } else if cap < 256 {
            required_len.max(cap.saturating_mul(2)).max(16)
        } else if cap < 512 {
            required_len.max(cap + (cap >> 2))
        } else {
            required_len.max(cap + (cap >> 3))
        };
        self.run.reserve_exact(target - self.run.len());
    }

    /// Room for the slots up to `highest` when the table is one run and
    /// `entries` entries keep it dense; pages are made as entries arrive.
    pub(crate) fn reserve_through(&mut self, highest: usize, entries: usize) {
        if !self.on_pages && Self::dense(highest + 1, entries) {
            self.grow_run(highest + 1);
        }
    }

    /// Puts `value` at `index` and gives back what was there. A slot the
    /// run already has is written in place: a record taken out to be worked
    /// on and put back costs what it does in a plain vector.
    #[inline]
    pub(crate) fn insert(&mut self, index: usize, value: T) -> Option<T> {
        if let Some(slot) = self.run.get_mut(index) {
            let previous = slot.replace(value);
            self.held += usize::from(previous.is_none());
            return previous;
        }
        self.insert_past_run(index, value)
    }

    fn insert_past_run(&mut self, index: usize, value: T) -> Option<T> {
        if !self.on_pages {
            if Self::dense(index + 1, self.held + 1) {
                self.grow_run(index + 1);
                self.run.resize_with(index, || None);
                self.run.push(Some(value));
                self.held += 1;
                return None;
            }
            self.turn_to_pages();
        }
        self.paged.insert(index, value)
    }

    #[inline]
    pub(crate) fn remove(&mut self, index: usize) -> Option<T> {
        if let Some(slot) = self.run.get_mut(index) {
            let removed = slot.take()?;
            self.held -= 1;
            if !Self::dense(self.run.len(), self.held) {
                self.thinned();
            }
            return Some(removed);
        }
        if self.on_pages {
            return self.paged.remove(index);
        }
        None
    }

    /// A run whose entries have thinned out: what is left moves to pages,
    /// or the slots are given back when nothing is.
    #[cold]
    fn thinned(&mut self) {
        if self.held == 0 {
            self.run = Vec::new();
        } else {
            self.turn_to_pages();
        }
    }

    /// Moves the run's entries to pages.
    fn turn_to_pages(&mut self) {
        for (index, slot) in std::mem::take(&mut self.run).into_iter().enumerate() {
            if let Some(value) = slot {
                self.paged.insert(index, value);
            }
        }
        self.held = 0;
        self.on_pages = true;
    }

    /// Keeps the values `keep` answers for, given each with its index.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(usize, &mut T) -> bool) {
        if self.on_pages {
            self.paged.retain(keep);
            return;
        }
        for (index, slot) in self.run.iter_mut().enumerate() {
            if slot.as_mut().is_some_and(|value| !keep(index, value)) {
                *slot = None;
                self.held -= 1;
            }
        }
        if !Self::dense(self.run.len(), self.held) {
            self.thinned();
        }
    }

    /// Empties the table; it starts over as one run.
    pub(crate) fn clear(&mut self) {
        self.run.clear();
        self.held = 0;
        if self.on_pages {
            self.paged = Paged::default();
            self.on_pages = false;
        }
    }

    /// Every value with its index, in index order.
    pub(crate) fn iter(&self) -> SlotsIter<'_, T> {
        SlotsIter {
            run: self.run.iter().enumerate(),
            paged: self.paged.iter(),
        }
    }

    /// Whether the entries are on pages, for tests of what a table holds.
    #[cfg(test)]
    pub(crate) fn is_paged(&self) -> bool {
        self.on_pages
    }
}

/// [`Slots::iter`]: the run, or the pages; one of them is empty.
pub(crate) struct SlotsIter<'a, T> {
    run: std::iter::Enumerate<std::slice::Iter<'a, Option<T>>>,
    paged: Iter<'a, T>,
}

impl<'a, T> Iterator for SlotsIter<'a, T> {
    type Item = (usize, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        self.run
            .find_map(|(index, slot)| Some((index, slot.as_ref()?)))
            .or_else(|| self.paged.next())
    }
}

/// Every value of a [`Slots`] with its index, in index order, by value.
pub(crate) struct SlotsIntoIter<T> {
    run: std::iter::Enumerate<std::vec::IntoIter<Option<T>>>,
    paged: IntoIter<T>,
}

impl<T> Iterator for SlotsIntoIter<T> {
    type Item = (usize, T);

    fn next(&mut self) -> Option<(usize, T)> {
        self.run
            .find_map(|(index, slot)| Some((index, slot?)))
            .or_else(|| self.paged.next())
    }
}

impl<T> IntoIterator for Slots<T> {
    type Item = (usize, T);
    type IntoIter = SlotsIntoIter<T>;

    fn into_iter(self) -> SlotsIntoIter<T> {
        SlotsIntoIter {
            run: self.run.into_iter().enumerate(),
            paged: self.paged.into_iter(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// Inserts, replacements, removals and filters at indices near and far,
    /// checked against an ordered map after every step.
    #[test]
    fn a_paged_table_holds_what_an_ordered_map_holds() {
        for seed in 1..=30u64 {
            let mut state = seed;
            let mut draw = |below: usize| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                ((state >> 33) as usize) % below
            };
            // Odd seeds start on pages; even ones as a run that the far
            // indices below turn to pages along the way.
            let mut paged = Slots::<u64>::default();
            if seed % 2 == 1 {
                paged.turn_to_pages();
            }
            let mut model = BTreeMap::<usize, u64>::new();
            for step in 0..600 {
                // Mostly a narrow band, sometimes far below or above it.
                let index = match draw(10) {
                    0 => draw(50),
                    1 => 100_000 + draw(5_000),
                    _ => 3_000 + draw(400),
                };
                match draw(7) {
                    0..=2 => {
                        let value = step as u64;
                        assert_eq!(paged.insert(index, value), model.insert(index, value));
                    }
                    3 | 4 => assert_eq!(paged.remove(index), model.remove(&index)),
                    5 if draw(20) == 0 => {
                        let cut = draw(4) as u64;
                        // What is kept turns on the index too: retain
                        // hands each value its own.
                        paged.retain(|at, value| (*value + at as u64) % 4 != cut);
                        model.retain(|at, value| (*value + *at as u64) % 4 != cut);
                    }
                    _ => {
                        if let Some(value) = paged.get_mut(index) {
                            *value += 1;
                        }
                        if let Some(value) = model.get_mut(&index) {
                            *value += 1;
                        }
                    }
                }
                assert_eq!(paged.get(index), model.get(&index));
                assert_eq!(paged.len(), model.len(), "seed {seed}, step {step}");
                assert!(
                    paged
                        .iter()
                        .map(|(index, value)| (index, *value))
                        .eq(model.iter().map(|(index, value)| (*index, *value))),
                    "seed {seed}, step {step}"
                );
            }
            let held: Vec<(usize, u64)> = model.iter().map(|(at, value)| (*at, *value)).collect();
            assert_eq!(paged.clone().into_iter().collect::<Vec<_>>(), held);
            for index in model.keys().copied().collect::<Vec<_>>() {
                assert!(paged.remove(index).is_some());
            }
            assert_eq!(paged.len(), 0);
            assert_eq!((paged.paged.pages(), paged.paged.span()), (0, 0));
            assert!(paged.run.is_empty());
        }
    }

    /// Indices in a row stay one run; a run thinned to an eighth, or an
    /// index far from it, moves the entries to pages; emptied, a table
    /// starts over as a run.
    #[test]
    fn a_table_is_one_run_while_its_indices_are_close() {
        let mut slots = Slots::<u32>::default();
        for index in 1..20_000 {
            slots.insert(index, index as u32);
        }
        assert!(!slots.is_paged());
        // Every other entry gone: still a run.
        for index in (1..20_000).step_by(2) {
            slots.remove(index);
        }
        assert!(!slots.is_paged());
        // Down to a twentieth: pages, with everything still there.
        for index in 1..20_000 {
            if index % 20 != 2 {
                slots.remove(index);
            }
        }
        assert!(slots.is_paged());
        assert_eq!(slots.len(), 1_000);
        assert!(
            slots
                .iter()
                .all(|(index, value)| index % 20 == 2 && *value == index as u32)
        );
        slots.clear();
        assert!(!slots.is_paged());

        // A run and one index far above it.
        let mut slots = Slots::<u32>::default();
        for index in 1_000..1_100 {
            slots.insert(index, 0);
        }
        assert!(!slots.is_paged());
        slots.insert(10_000_000, 1);
        assert!(slots.is_paged());
        assert_eq!((slots.len(), slots.get(10_000_000)), (101, Some(&1)));
        assert_eq!(slots.get(1_050), Some(&0));

        // A few indices far from zero never make a run.
        let mut slots = Slots::<u32>::default();
        slots.insert(70_000, 7);
        slots.insert(70_001, 8);
        assert!(slots.is_paged() && slots.run.is_empty());
        assert_eq!(
            slots.iter().collect::<Vec<_>>(),
            [(70_000, &7), (70_001, &8)]
        );
    }

    /// What is held follows what is in use, not the highest index seen.
    #[test]
    fn pages_follow_the_entries_in_use() {
        let mut paged = Paged::<[u8; 256]>::default();
        // A window of a hundred live entries moving up through a million.
        for index in 0..1_000_000usize {
            paged.insert(index, [0; 256]);
            if index >= 100 {
                paged.remove(index - 100);
            }
            assert!(paged.pages() <= 100 / PAGE + 2, "at {index}");
            assert!(paged.span() <= 100 / PAGE + 2, "at {index}");
        }
        // One old survivor far below keeps its page and the span between.
        paged.insert(0, [1; 256]);
        assert_eq!(paged.pages(), 3);
        assert_eq!(paged.span(), (999_999 >> PAGE_BITS) + 1);
        paged.remove(0);
        assert!(paged.span() <= 100 / PAGE + 2);
    }
}
