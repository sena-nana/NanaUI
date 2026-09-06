//! Sparse virtual placement, with the same keyed materializer and Runtime authority.

use super::*;
use crate::Stack;
use nana_ui_core::{PositionSpec, VirtualViewport};

#[cfg(test)]
mod tests;

impl AppContext {
    /// Materialize a positioned content list inside a ScrollView. Each mounted
    /// component has one framework-owned placement container at its logical Y.
    /// The List's height is the full data extent; its width remains caller-owned.
    /// Item content, including editable state and children, is never reprojected
    /// just because its index changes. Do not mix this with the legacy unplaced
    /// materializer on the same `items` instance.
    ///
    /// Focus and a previously observed IME owner are retained automatically.
    /// `retained_keys` additionally protects application editing sessions. The
    /// inverse must resolve current data indices; missing or mismatched keys are
    /// released. Selection reads only mounted items, active ancestors and requested
    /// keys; removal uses the framework's existing event-handler cleanup.
    #[allow(clippy::too_many_arguments)]
    pub fn materialize_virtual_list_retained_in<K, C>(
        &mut self,
        list: Entity<List>,
        items: &mut VirtualListItems<K, C>,
        layout: &VirtualListLayout,
        viewport: VirtualViewport,
        retained_keys: &[K],
        mut key_at: impl FnMut(usize) -> K,
        mut index_of_key: impl FnMut(&K) -> Option<usize>,
        mut build: impl FnMut(usize, &K) -> C,
    ) -> Result<VirtualListWindow, FrameworkError>
    where
        K: Clone + Eq + Hash,
        C: ComponentView,
    {
        let mut content = self.read(list, Clone::clone)?;
        let root = self
            .world
            .node(list.id)
            .ok_or(FrameworkError::MissingView(list.id))?;
        let mounted = items.materializer.mounted();
        let owned = items
            .containers
            .iter()
            .map(|(key, entity)| (entity.id, key))
            .collect::<HashMap<_, _>>();
        if mounted.len() != items.entities.len()
            || mounted.len() != items.containers.len()
            || owned.len() != mounted.len()
            || root.children.len() != owned.len()
            || root.children.iter().any(|id| !owned.contains_key(id))
        {
            return Err(FrameworkError::InvalidVirtualization);
        }
        for key in mounted {
            let container = items
                .containers
                .get(key)
                .ok_or(FrameworkError::InvalidVirtualization)?;
            let entity = items
                .entities
                .get(key)
                .ok_or(FrameworkError::InvalidVirtualization)?;
            if self
                .world
                .node(container.id)
                .is_none_or(|node| node.parent != Some(list.id) || node.children != [entity.id])
                || self
                    .world
                    .node(entity.id)
                    .is_none_or(|node| node.parent != Some(container.id))
                || self.read(*container, |_| ()).is_err()
                || self.read(*entity, |_| ()).is_err()
            {
                return Err(FrameworkError::InvalidVirtualization);
            }
        }

        let focused = self.world.focused(root.document);
        let ime_owner = focused
            .filter(|id| self.world.ime(*id).is_some())
            .or_else(|| items.ime_owner.filter(|id| self.world.ime(*id).is_some()));
        let mut active = retained_keys.to_vec();
        for target in [focused, ime_owner].into_iter().flatten() {
            let mut current = Some(target);
            while let Some(id) = current {
                if id == list.id {
                    break;
                }
                if let Some(key) = owned.get(&id) {
                    active.push((*key).clone());
                    break;
                }
                current = self.world.node(id).and_then(|node| node.parent);
            }
        }
        let retained = active
            .iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .filter_map(|key| {
                let index = index_of_key(key)?;
                (index < layout.len() && key_at(index) == *key).then_some(index)
            })
            .collect::<Vec<_>>();
        let plan = items
            .materializer
            .prepare_retained(layout, viewport, retained.iter().copied(), &mut key_at)
            .map_err(|_| FrameworkError::InvalidVirtualization)?;
        let indices = layout
            .retained_ranges(&plan.window, retained)
            .into_iter()
            .flatten();

        let mut mutations = MutationQueue::new();
        let mut next_entities = items.entities.clone();
        let mut next_containers = items.containers.clone();
        let mut removed = HashSet::new();
        for key in &plan.unmounts {
            let id = items.containers[key].id;
            let mut stack = vec![id];
            while let Some(id) = stack.pop() {
                let node = self
                    .world
                    .node(id)
                    .ok_or(FrameworkError::InvalidVirtualization)?;
                stack.extend(node.children);
                removed.insert(id);
            }
            mutations.despawn_subtree(id);
            next_entities.remove(key);
            next_containers.remove(key);
        }
        let mut staged_items = Vec::new();
        let mut staged_containers = Vec::new();
        for (index, key) in indices.zip(&plan.order) {
            let top = layout.extent(0..index);
            let height = layout.extent(index..index + 1);
            let container = Stack::column(0.0).style(crate::NodeStyle {
                layout: Arc::new(nana_ui_core::LayoutStyle {
                    position: PositionSpec::Absolute,
                    offset_top: Some(LengthSpec::Px(top)),
                    offset_left: Some(LengthSpec::Px(0.0)),
                    width: Some(LengthSpec::Percent(100.0)),
                    height: Some(LengthSpec::Px(height)),
                    flex_shrink: Some(0.0),
                    ..Default::default()
                }),
                ..Default::default()
            });
            if let Some(entity) = next_containers.get(key).copied() {
                if self.read(entity, |old| old != &container)? {
                    container.project(entity.id, &self.world, &mut mutations);
                    staged_containers.push((entity.id, container));
                }
            } else {
                let component = build(index, key);
                let container_id = self.allocate_id();
                let item_id = self.allocate_id();
                mutations.create(container_id, root.document, container.node_kind());
                container.project(container_id, &self.world, &mut mutations);
                mutations.create(item_id, root.document, component.node_kind());
                component.project(item_id, &self.world, &mut mutations);
                mutations.insert(container_id, item_id, None);
                next_containers.insert(key.clone(), Entity::from_stable_id(container_id));
                next_entities.insert(key.clone(), Entity::from_stable_id(item_id));
                staged_containers.push((container_id, container));
                staged_items.push((item_id, component));
            }
        }
        let desired = plan
            .order
            .iter()
            .map(|key| next_containers[key].id)
            .collect::<Vec<_>>();
        if desired != root.children {
            for id in desired {
                mutations.insert(list.id, id, None);
            }
        }
        let content_layout = Arc::make_mut(&mut content.style.layout);
        content_layout.height = Some(LengthSpec::Px(layout.total_extent()));
        content_layout.min_height = content_layout.height;
        content_layout.max_height = content_layout.height;
        content_layout.flex_shrink = Some(0.0);
        if self.read(list, |old| old.style.layout != content.style.layout)? {
            content.project(list.id, &self.world, &mut mutations);
        }

        // Publish external identities and component data only after atomic commit.
        if !mutations.is_empty() {
            self.world.commit(mutations)?;
        }
        if !removed.is_empty() {
            self.remove_event_handlers_for(&removed);
            for id in &removed {
                self.views.remove(id);
            }
        }
        self.views.insert(list.id, Box::new(content));
        for (id, component) in staged_items {
            self.views.insert(id, Box::new(component));
        }
        for (id, container) in staged_containers {
            self.views.insert(id, Box::new(container));
        }
        items.entities = next_entities;
        items.containers = next_containers;
        items.ime_owner = ime_owner.or(focused).filter(|id| !removed.contains(id));
        let window = plan.window.clone();
        items
            .materializer
            .commit(plan)
            .map_err(|_| FrameworkError::InvalidVirtualization)?;
        Ok(window)
    }

    /// Materialize both axes, frozen prefixes and active cells in one commit.
    /// Table/row/cell positioning is managed by this entry; nested editor content
    /// remains application-owned. Missing keys end retention. Frozen cells use
    /// the Surface token when no explicit background is supplied.
    #[allow(clippy::too_many_arguments)]
    pub fn materialize_virtual_table_retained_in<R, C>(
        &mut self,
        table: Entity<Table>,
        items: &mut VirtualTableItems<R, C>,
        layout: &VirtualTableLayout,
        viewport: VirtualViewport,
        frozen: [usize; 2],
        retained_cells: &[(R, C)],
        mut row_key_at: impl FnMut(usize) -> R,
        mut row_index_of_key: impl FnMut(&R) -> Option<usize>,
        mut column_key_at: impl FnMut(usize) -> C,
        mut column_index_of_key: impl FnMut(&C) -> Option<usize>,
        build_row: impl FnMut(usize, &R) -> TableRow,
        build_cell: impl FnMut(usize, &R, usize, &C) -> TableCell,
    ) -> Result<VirtualTableWindow, FrameworkError>
    where
        R: Clone + Eq + Hash,
        C: Clone + Eq + Hash,
    {
        let mut viewport = viewport;
        for (axis, total) in [
            layout.column_layout().total_extent(),
            layout.row_layout().total_extent(),
        ]
        .into_iter()
        .enumerate()
        {
            let extent = viewport.extent[axis];
            viewport.extent[axis] = if extent.is_finite() {
                extent.max(0.0)
            } else {
                0.0
            };
            let offset = viewport.offset[axis];
            viewport.offset[axis] = if offset.is_finite() {
                offset
                    .max(0.0)
                    .min((total - viewport.extent[axis]).max(0.0))
            } else {
                0.0
            };
        }
        self.read(table, |_| ())?;
        let document = self
            .world
            .node(table.id)
            .ok_or(FrameworkError::MissingView(table.id))?
            .document;
        let focused = self.world.focused(document);
        let ime = focused
            .filter(|id| self.world.ime(*id).is_some())
            .or_else(|| items.ime_owner.filter(|id| self.world.ime(*id).is_some()));
        let cell_keys = items
            .cells
            .iter()
            .map(|(key, entity)| (entity.id, key))
            .collect::<HashMap<_, _>>();
        let mut active = retained_cells.iter().cloned().collect::<HashSet<_>>();
        for target in [focused, ime].into_iter().flatten() {
            let mut current = Some(target);
            while let Some(id) = current {
                if id == table.id {
                    break;
                }
                if let Some(key) = cell_keys.get(&id) {
                    active.insert((*key).clone());
                    break;
                }
                current = self.world.node(id).and_then(|node| node.parent);
            }
        }
        let mut rows = Vec::new();
        let mut columns = Vec::new();
        for (row, column) in active {
            if let (Some(r), Some(c)) = (row_index_of_key(&row), column_index_of_key(&column))
                && r < layout.row_count()
                && c < layout.column_count()
                && row_key_at(r) == row
                && column_key_at(c) == column
            {
                rows.push(r);
                columns.push(c);
            }
        }
        let plan = items
            .materializer
            .prepare_retained(
                layout,
                viewport,
                frozen,
                rows,
                columns,
                row_key_at,
                column_key_at,
            )
            .map_err(|_| FrameworkError::InvalidVirtualization)?;
        let window = self.commit_virtual_table(
            table,
            items,
            plan,
            Some((layout, viewport, frozen)),
            build_row,
            build_cell,
        )?;
        items.ime_owner = ime.or(focused).filter(|id| self.world.contains(*id));
        Ok(window)
    }

    /// Positioned virtualization over only the tree's expanded row sequence.
    #[allow(clippy::too_many_arguments)]
    pub fn materialize_virtual_tree_retained_in<K, C>(
        &mut self,
        tree: Entity<List>,
        items: &mut VirtualTreeItems<K, C>,
        layout: &VirtualTreeLayout,
        viewport: VirtualViewport,
        retained_keys: &[K],
        key_at: impl FnMut(usize) -> K,
        index_of_key: impl FnMut(&K) -> Option<usize>,
        build: impl FnMut(usize, &K) -> C,
    ) -> Result<VirtualListWindow, FrameworkError>
    where
        K: Clone + Eq + Hash,
        C: ComponentView,
    {
        self.materialize_virtual_list_retained_in(
            tree,
            &mut items.items,
            layout.row_layout(),
            viewport,
            retained_keys,
            key_at,
            index_of_key,
            build,
        )
    }
}

pub(super) fn position_table_row(
    row: &mut TableRow,
    layout: &VirtualTableLayout,
    viewport: VirtualViewport,
    frozen: [usize; 2],
    index: usize,
) {
    let style = Arc::make_mut(&mut row.style.layout);
    style.position = PositionSpec::Absolute;
    style.offset_top = Some(LengthSpec::Px(layout.row_layout().extent(0..index)));
    style.offset_left = Some(LengthSpec::Px(0.0));
    style.width = Some(LengthSpec::Px(layout.column_layout().total_extent()));
    style.height = Some(LengthSpec::Px(layout.row_layout().extent(index..index + 1)));
    style.flex_shrink = Some(0.0);
    style.z_index = Some(if index < frozen[1] { 2 } else { 0 });
    style.transform = (index < frozen[1]).then_some(nana_ui_core::PaintTransform {
        f: viewport.offset[1],
        ..Default::default()
    });
}

pub(super) fn position_table_cell(
    cell: &mut TableCell,
    layout: &VirtualTableLayout,
    viewport: VirtualViewport,
    frozen: [usize; 2],
    row: usize,
    column: usize,
) {
    let style = Arc::make_mut(&mut cell.style.layout);
    style.position = PositionSpec::Absolute;
    style.offset_left = Some(LengthSpec::Px(layout.column_layout().extent(0..column)));
    style.offset_top = Some(LengthSpec::Px(0.0));
    style.width = Some(LengthSpec::Px(
        layout.column_layout().extent(column..column + 1),
    ));
    style.height = Some(LengthSpec::Px(layout.row_layout().extent(row..row + 1)));
    style.flex_shrink = Some(0.0);
    style.z_index = Some(if column < frozen[0] { 1 } else { 0 });
    style.transform = (column < frozen[0]).then_some(nana_ui_core::PaintTransform {
        e: viewport.offset[0],
        ..Default::default()
    });
    cell.style
        .background
        .get_or_insert(nana_ui_core::SemanticColorRole::Surface);
}
