//! Borrowed semantic input. Props stay in the bridge; Runtime owns topology.
use super::*;
use std::{cell::RefCell, collections::HashMap, ops::Deref};

pub(crate) struct SemanticWidgetView<'a> {
    widget: &'a crate::SemanticWidget,
    pub children: Arc<[u64]>,
    pub parent: Option<u64>,
}

impl Deref for SemanticWidgetView<'_> {
    type Target = crate::SemanticWidget;
    fn deref(&self) -> &Self::Target {
        self.widget
    }
}

enum Source<'a> {
    Snapshot {
        snapshot: &'a crate::SemanticSnapshot,
        index: HashMap<u64, &'a crate::SemanticWidget>,
    },
    Bridge {
        bridge: &'a crate::MessageBridge,
        document: &'a NanaTreeDocument,
    },
}

pub(crate) struct SemanticRead<'a> {
    source: Source<'a>,
    // A per-prepare read cache, discarded before commit. Runtime remains authoritative.
    topology: RefCell<HashMap<u64, (Option<u64>, Arc<[u64]>)>>,
    pub revision: u64,
    pub theme: nana_ui_core::ThemeMode,
    pub changes: crate::bridge::SnapshotChanges,
}

impl<'a> SemanticRead<'a> {
    pub fn snapshot(snapshot: &'a crate::SemanticSnapshot) -> Self {
        Self {
            source: Source::Snapshot {
                snapshot,
                index: snapshot
                    .widgets
                    .iter()
                    .map(|widget| (widget.id, widget))
                    .collect(),
            },
            topology: RefCell::new(HashMap::new()),
            revision: snapshot.revision,
            theme: snapshot.theme,
            changes: snapshot.changes.clone(),
        }
    }
    pub fn bridge(
        bridge: &'a crate::MessageBridge,
        document: &'a NanaTreeDocument,
        changes: crate::bridge::SnapshotChanges,
    ) -> Self {
        Self {
            source: Source::Bridge { bridge, document },
            topology: RefCell::new(HashMap::new()),
            revision: bridge.revision(),
            theme: bridge.theme(),
            changes,
        }
    }
    pub fn get(&self, id: u64) -> Option<SemanticWidgetView<'a>> {
        let widget = match &self.source {
            Source::Snapshot { index, .. } => *index.get(&id)?,
            Source::Bridge { bridge, document } => {
                if !document.nodes.contains_key(&id) {
                    return None;
                }
                bridge.get(id)?
            }
        };
        let mut topology = self.topology.borrow_mut();
        let (parent, children) = topology.entry(id).or_insert_with(|| match &self.source {
            Source::Snapshot { .. } => (widget.parent, widget.children.clone().into()),
            Source::Bridge { bridge, document } => {
                let visible = |id| document.nodes.contains_key(&id) && bridge.get(id).is_some();
                (
                    document.live_parent(id).filter(|id| visible(*id)),
                    document
                        .live_children(id)
                        .into_iter()
                        .filter(|id| visible(*id))
                        .collect(),
                )
            }
        });
        Some(SemanticWidgetView {
            widget,
            parent: *parent,
            children: children.clone(),
        })
    }
    #[cfg(test)]
    pub fn topology_work(&self) -> (usize, usize) {
        let topology = self.topology.borrow();
        (
            topology.len(),
            topology.values().map(|(_, children)| children.len()).sum(),
        )
    }
    pub fn projection_ids(&self, full: bool) -> Vec<u64> {
        match &self.source {
            Source::Snapshot { snapshot, .. } => snapshot
                .widgets
                .iter()
                .filter(|widget| full || self.changes.dirty.contains(&widget.id))
                .map(|widget| widget.id)
                .collect(),
            Source::Bridge { bridge, document } => {
                if full {
                    let mut ids: Vec<_> = document
                        .runtime
                        .document_order(document.runtime.document.document())
                        .into_iter()
                        .map(StableNodeId::get)
                        .filter(|id| bridge.get(*id).is_some())
                        .collect();
                    let seen: HashSet<_> = ids.iter().copied().collect();
                    ids.extend(bridge.widget_ids().filter(|id| !seen.contains(id)));
                    return ids;
                }
                let mut seen = HashSet::new();
                let mut ids = Vec::new();
                for dirty in &self.changes.dirty {
                    let mut chain = Vec::new();
                    let mut current = Some(*dirty);
                    while let Some(id) = current {
                        if !seen.insert(id) {
                            break;
                        }
                        if bridge.get(id).is_some() {
                            chain.push(id);
                        }
                        current = document.live_parent(id);
                    }
                    ids.extend(chain.into_iter().rev());
                }
                ids
            }
        }
    }
}

pub(super) struct PreparedSemanticSync {
    pub mutations: MutationQueue,
    pub pending: PendingAssembly,
    pub component_owned_layout: HashSet<u64>,
    pub projected: Vec<u64>,
    pub full_pass: bool,
    pub revision: u64,
}
