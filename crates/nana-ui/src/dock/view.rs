//! Renderer-neutral dock view projection.

use super::model::{DockAxis, DockBounds, DockId, DockNode, DockSurfaceId};

/// Renderer-neutral geometry for one visible dock item.
///
/// `panel` includes NanaUI-owned title/tab chrome while `content` is the exact
/// rectangle available to application-owned content such as a Runtime tree or
/// a host texture. Only the active member of a tab group is present here.
#[derive(Debug, Clone, PartialEq)]
pub struct DockItemLayout {
    pub id: DockId,
    pub panel: DockBounds,
    pub content: DockBounds,
}

/// Renderer-neutral geometry and ordering for one tab group.
#[derive(Debug, Clone, PartialEq)]
pub struct DockTabsLayout {
    pub tabs: Vec<DockId>,
    pub active: DockId,
    pub bounds: DockBounds,
    pub content: DockBounds,
}

/// Renderer-neutral hit geometry for a dock divider.
#[derive(Debug, Clone, PartialEq)]
pub struct DockSplitLayout {
    pub path: Vec<usize>,
    pub axis: DockAxis,
    pub bounds: DockBounds,
}

/// A deterministic surface projection shared by Runtime and compatibility
/// painters. Native windows and renderer resources remain host-owned.
#[derive(Debug, Clone, PartialEq)]
pub struct DockSurfaceLayout {
    pub surface: DockSurfaceId,
    pub bounds: DockBounds,
    pub items: Vec<DockItemLayout>,
    pub tabs: Vec<DockTabsLayout>,
    pub splits: Vec<DockSplitLayout>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DockViewItem {
    Existing(DockId),
    #[allow(dead_code)]
    Placeholder(DockId),
}

impl DockViewItem {
    pub(super) fn id(&self) -> &DockId {
        match self {
            Self::Existing(id) | Self::Placeholder(id) => id,
        }
    }

    #[cfg(test)]
    pub(super) fn is_placeholder(&self) -> bool {
        matches!(self, Self::Placeholder(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum DockViewNode {
    Item {
        item: DockViewItem,
    },
    Tabs {
        tabs: Vec<DockViewItem>,
        active: DockViewItem,
    },
    Split {
        axis: DockAxis,
        ratio: f32,
        first: Box<DockViewNode>,
        second: Box<DockViewNode>,
    },
}

impl From<&DockNode> for DockViewNode {
    fn from(node: &DockNode) -> Self {
        match node {
            DockNode::Item { id } => Self::Item {
                item: DockViewItem::Existing(id.clone()),
            },
            DockNode::Tabs { tabs, active } => Self::Tabs {
                tabs: tabs.iter().cloned().map(DockViewItem::Existing).collect(),
                active: DockViewItem::Existing(active.clone()),
            },
            DockNode::Split {
                axis,
                ratio,
                first,
                second,
            } => Self::Split {
                axis: *axis,
                ratio: *ratio,
                first: Box::new(Self::from(first.as_ref())),
                second: Box::new(Self::from(second.as_ref())),
            },
        }
    }
}

impl DockViewNode {
    pub(super) fn contains(&self, id: &DockId) -> bool {
        match self {
            Self::Item { item } => item.id() == id,
            Self::Tabs { tabs, .. } => tabs.iter().any(|item| item.id() == id),
            Self::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    #[cfg(test)]
    pub(super) fn contains_placeholder(&self, id: &DockId) -> bool {
        match self {
            Self::Item { item } => item.is_placeholder() && item.id() == id,
            Self::Tabs { tabs, .. } => tabs
                .iter()
                .any(|item| item.is_placeholder() && item.id() == id),
            Self::Split { first, second, .. } => {
                first.contains_placeholder(id) || second.contains_placeholder(id)
            }
        }
    }
}
