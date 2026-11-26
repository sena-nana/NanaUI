//! Generic accessibility tree contract validation for offscreen tests and
//! agent sessions.
//!
//! Validates structural invariants (unique IDs, single root, parent-child
//! symmetry, acyclicity) and semantic contracts for standard NanaUI
//! accessibility roles without embedding any application-specific rules.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use super::AccessibilityDumpNode;

/// Interactive roles that must carry a non-empty accessible label, positive
/// finite bounds while enabled, and distinct sibling labels under the same
/// parent.
pub const INTERACTIVE_NAMED_ROLES: &[&str] = &[
    "button",
    "checkbox",
    "combo-box",
    "menu-item",
    "radio",
    "slider",
    "switch",
    "tab",
    "tab-list",
    "text-input",
    "list-item",
];

/// Roles that must expose a `checked` (or `mixed`) state.
pub const CHECKED_ROLES: &[&str] = &["checkbox", "radio", "switch"];

/// Roles that must expose a `selected` state.
pub const SELECTED_ROLES: &[&str] = &["tab"];

/// Classification of a single accessibility tree contract violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityViolationKind {
    EmptyTree,
    DuplicateNodeId,
    RootCount {
        count: usize,
    },
    MissingParent {
        parent_id: u64,
    },
    ParentMissingChild {
        parent_id: u64,
    },
    MissingChild {
        child_id: u64,
    },
    ChildParentMismatch {
        child_id: u64,
        expected_parent: u64,
        actual_parent: Option<u64>,
    },
    ParentCycle,
    EmptyRole,
    MissingLabel,
    InvalidBounds,
    InvalidSliderRange,
    InvalidSliderValue,
    MissingCheckedState,
    MissingSelectedState,
    RadioCheckedSelectedMismatch,
    DuplicateSiblingLabel {
        label: String,
    },
}

/// One contract violation found by [`validate_accessibility_tree`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityViolation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub kind: AccessibilityViolationKind,
    pub message: String,
}

impl AccessibilityViolation {
    fn new(
        node: Option<u64>,
        role: Option<&str>,
        kind: AccessibilityViolationKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            node,
            role: role.map(str::to_owned),
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for AccessibilityViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AccessibilityViolation {}

/// Format a slice of violations into a human-readable multi-line diagnostic
/// summary.
pub fn format_accessibility_violations(violations: &[AccessibilityViolation]) -> String {
    let mut out = format!(
        "accessibility tree contract failed with {} violation(s):",
        violations.len()
    );
    for violation in violations {
        out.push_str("\n  - ");
        out.push_str(&violation.message);
    }
    out
}

/// Validate generic structural and role-contract invariants across a projected
/// accessibility tree.
///
/// Collects all violations in a single pass so callers receive the full set of
/// broken invariants instead of stopping at the first error.
pub fn validate_accessibility_tree(
    nodes: &[AccessibilityDumpNode],
) -> Result<(), Vec<AccessibilityViolation>> {
    let mut violations = Vec::new();
    if nodes.is_empty() {
        violations.push(AccessibilityViolation::new(
            None,
            None,
            AccessibilityViolationKind::EmptyTree,
            "accessibility tree is empty",
        ));
        return Err(violations);
    }

    let mut by_id: BTreeMap<u64, &AccessibilityDumpNode> = BTreeMap::new();
    let mut roots = 0usize;
    for node in nodes {
        if by_id.insert(node.id, node).is_some() {
            violations.push(AccessibilityViolation::new(
                Some(node.id),
                Some(&node.role),
                AccessibilityViolationKind::DuplicateNodeId,
                format!("duplicate accessibility node id {}", node.id),
            ));
        }
        if node.parent.is_none() {
            roots += 1;
        }
    }
    if roots != 1 {
        violations.push(AccessibilityViolation::new(
            None,
            None,
            AccessibilityViolationKind::RootCount { count: roots },
            format!("expected exactly 1 accessibility root, found {roots}"),
        ));
    }

    for node in nodes {
        if let Some(parent_id) = node.parent {
            match by_id.get(&parent_id) {
                None => violations.push(AccessibilityViolation::new(
                    Some(node.id),
                    Some(&node.role),
                    AccessibilityViolationKind::MissingParent { parent_id },
                    format!(
                        "node {} ({}) references missing parent {parent_id}",
                        node.id, node.role
                    ),
                )),
                Some(parent) => {
                    if !parent.children.contains(&node.id) {
                        violations.push(AccessibilityViolation::new(
                            Some(node.id),
                            Some(&node.role),
                            AccessibilityViolationKind::ParentMissingChild { parent_id },
                            format!(
                                "parent {parent_id} does not list child {} ({})",
                                node.id, node.role
                            ),
                        ));
                    }
                }
            }
        }

        for &child_id in &node.children {
            match by_id.get(&child_id) {
                None => violations.push(AccessibilityViolation::new(
                    Some(node.id),
                    Some(&node.role),
                    AccessibilityViolationKind::MissingChild { child_id },
                    format!(
                        "node {} ({}) lists missing child {child_id}",
                        node.id, node.role
                    ),
                )),
                Some(child) => {
                    if child.parent != Some(node.id) {
                        violations.push(AccessibilityViolation::new(
                            Some(node.id),
                            Some(&node.role),
                            AccessibilityViolationKind::ChildParentMismatch {
                                child_id,
                                expected_parent: node.id,
                                actual_parent: child.parent,
                            },
                            format!(
                                "child {child_id} points to parent {:?} instead of {}",
                                child.parent, node.id
                            ),
                        ));
                    }
                }
            }
        }

        // Acyclicity along the parent chain.
        let mut seen = BTreeSet::new();
        let mut cursor = Some(node.id);
        while let Some(id) = cursor {
            if !seen.insert(id) {
                violations.push(AccessibilityViolation::new(
                    Some(node.id),
                    Some(&node.role),
                    AccessibilityViolationKind::ParentCycle,
                    format!("accessibility parent cycle involving node {id}"),
                ));
                break;
            }
            cursor = by_id.get(&id).and_then(|entry| entry.parent);
        }

        if node.role.trim().is_empty() {
            violations.push(AccessibilityViolation::new(
                Some(node.id),
                Some(&node.role),
                AccessibilityViolationKind::EmptyRole,
                format!("node {} has an empty role", node.id),
            ));
        }

        if INTERACTIVE_NAMED_ROLES.contains(&node.role.as_str()) {
            let label = node.label.as_deref().map(str::trim).unwrap_or("");
            if label.is_empty() {
                violations.push(AccessibilityViolation::new(
                    Some(node.id),
                    Some(&node.role),
                    AccessibilityViolationKind::MissingLabel,
                    format!(
                        "interactive node {} (role='{}', agent_id={:?}, agent_path={:?}) is missing an accessible label",
                        node.id, node.role, node.agent_id, node.agent_path
                    ),
                ));
            }
            if !node.disabled {
                let bounds = &node.bounds;
                let valid = bounds.x.is_finite()
                    && bounds.y.is_finite()
                    && bounds.width.is_finite()
                    && bounds.height.is_finite()
                    && bounds.width > 0.0
                    && bounds.height > 0.0;
                if !valid {
                    violations.push(AccessibilityViolation::new(
                        Some(node.id),
                        Some(&node.role),
                        AccessibilityViolationKind::InvalidBounds,
                        format!(
                            "enabled interactive node {} (role='{}', label={:?}) has invalid bounds {:?}",
                            node.id, node.role, node.label, node.bounds
                        ),
                    ));
                }
            }
        }

        if node.role == "slider" {
            let range_ok = matches!(
                (node.numeric_minimum, node.numeric_maximum, node.numeric_step),
                (Some(min), Some(max), Some(step))
                    if min.is_finite() && max.is_finite() && step.is_finite() && min <= max
            );
            if !range_ok {
                violations.push(AccessibilityViolation::new(
                    Some(node.id),
                    Some(&node.role),
                    AccessibilityViolationKind::InvalidSliderRange,
                    format!(
                        "slider {} (label={:?}) has invalid numeric range min={:?} max={:?} step={:?}",
                        node.id,
                        node.label,
                        node.numeric_minimum,
                        node.numeric_maximum,
                        node.numeric_step
                    ),
                ));
            }
            if let Some(value) = node.numeric_value
                && !value.is_finite()
            {
                violations.push(AccessibilityViolation::new(
                    Some(node.id),
                    Some(&node.role),
                    AccessibilityViolationKind::InvalidSliderValue,
                    format!(
                        "slider {} (label={:?}) has non-finite value {value:?}",
                        node.id, node.label
                    ),
                ));
            }
        }

        if CHECKED_ROLES.contains(&node.role.as_str()) && node.checked.is_none() && !node.mixed {
            violations.push(AccessibilityViolation::new(
                Some(node.id),
                Some(&node.role),
                AccessibilityViolationKind::MissingCheckedState,
                format!(
                    "node {} (role='{}', label={:?}) must expose checked or mixed state",
                    node.id, node.role, node.label
                ),
            ));
        }

        if SELECTED_ROLES.contains(&node.role.as_str()) && node.selected.is_none() {
            violations.push(AccessibilityViolation::new(
                Some(node.id),
                Some(&node.role),
                AccessibilityViolationKind::MissingSelectedState,
                format!(
                    "node {} (role='{}', label={:?}) must expose selected state",
                    node.id, node.role, node.label
                ),
            ));
        }

        if node.role == "radio"
            && let Some(selected) = node.selected
            && node.checked != Some(selected)
        {
            violations.push(AccessibilityViolation::new(
                Some(node.id),
                Some(&node.role),
                AccessibilityViolationKind::RadioCheckedSelectedMismatch,
                format!(
                    "radio {} (label={:?}) has mismatched checked={:?} and selected={:?}",
                    node.id, node.label, node.checked, node.selected
                ),
            ));
        }

        // Interactive siblings under the same parent must have distinct names.
        let mut sibling_labels: BTreeSet<String> = BTreeSet::new();
        for &child_id in &node.children {
            let Some(child) = by_id.get(&child_id) else {
                continue;
            };
            if !INTERACTIVE_NAMED_ROLES.contains(&child.role.as_str()) {
                continue;
            }
            let Some(trimmed) = child.label.as_deref().map(str::trim) else {
                continue;
            };
            if trimmed.is_empty() {
                continue;
            }
            if !sibling_labels.insert(trimmed.to_owned()) {
                violations.push(AccessibilityViolation::new(
                    Some(child.id),
                    Some(&child.role),
                    AccessibilityViolationKind::DuplicateSiblingLabel {
                        label: trimmed.to_owned(),
                    },
                    format!(
                        "parent {} has duplicate interactive sibling label {:?}",
                        node.id, trimmed
                    ),
                ));
            }
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::BoundsDump;

    fn node(id: u64, parent: Option<u64>, role: &str) -> AccessibilityDumpNode {
        AccessibilityDumpNode {
            id,
            parent,
            children: Vec::new(),
            role: role.into(),
            label: None,
            value: None,
            focused: false,
            disabled: false,
            bounds: BoundsDump {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 32.0,
            },
            agent_id: None,
            agent_path: None,
            description: None,
            checked: None,
            mixed: false,
            orientation: None,
            selected: None,
            level: None,
            expanded: None,
            multiline: false,
            editable: false,
            selection: None,
            modal: false,
            busy: false,
            invalid: false,
            numeric_minimum: None,
            numeric_maximum: None,
            numeric_step: None,
            numeric_value: None,
        }
    }

    fn clean_tree() -> Vec<AccessibilityDumpNode> {
        let mut root = node(1, None, "document");
        root.children = vec![2, 3, 4, 5, 6];

        let mut btn = node(2, Some(1), "button");
        btn.label = Some("Save".into());

        let mut slider = node(3, Some(1), "slider");
        slider.label = Some("Volume".into());
        slider.numeric_minimum = Some(0.0);
        slider.numeric_maximum = Some(100.0);
        slider.numeric_step = Some(1.0);
        slider.numeric_value = Some(50.0);

        let mut cb = node(4, Some(1), "checkbox");
        cb.label = Some("Enable".into());
        cb.checked = Some(true);

        let mut tab = node(5, Some(1), "tab");
        tab.label = Some("General".into());
        tab.selected = Some(true);

        let mut radio = node(6, Some(1), "radio");
        radio.label = Some("Option A".into());
        radio.checked = Some(false);
        radio.selected = Some(false);

        vec![root, btn, slider, cb, tab, radio]
    }

    #[test]
    fn clean_accessibility_tree_passes_validation() {
        assert_eq!(validate_accessibility_tree(&clean_tree()), Ok(()));
    }

    #[test]
    fn detects_structural_violations() {
        let empty_err = validate_accessibility_tree(&[]).unwrap_err();
        assert_eq!(empty_err[0].kind, AccessibilityViolationKind::EmptyTree);

        // Missing parent/child symmetry + duplicate ID + multiple roots.
        let mut n1 = node(1, None, "document");
        n1.children = vec![99];
        let n2 = node(2, None, "generic");
        let n2_dup = node(2, Some(1), "generic");
        let errs = validate_accessibility_tree(&[n1, n2, n2_dup]).unwrap_err();
        let kinds: Vec<_> = errs.iter().map(|v| &v.kind).collect();
        assert!(kinds.contains(&&AccessibilityViolationKind::DuplicateNodeId));
        assert!(kinds.contains(&&AccessibilityViolationKind::RootCount { count: 2 }));
        assert!(kinds.contains(&&AccessibilityViolationKind::MissingChild { child_id: 99 }));

        // Parent cycle.
        let mut a = node(10, Some(11), "generic");
        a.children = vec![11];
        let mut b = node(11, Some(10), "generic");
        b.children = vec![10];
        let errs = validate_accessibility_tree(&[a, b]).unwrap_err();
        assert!(
            errs.iter()
                .any(|v| v.kind == AccessibilityViolationKind::ParentCycle)
        );
    }

    #[test]
    fn detects_role_contract_and_sibling_name_violations() {
        let mut root = node(1, None, "document");
        root.children = vec![2, 3, 4, 5, 6, 7, 8];

        let mut unnamed_btn = node(2, Some(1), "button");
        unnamed_btn.label = Some("   ".into());

        let mut zero_bounds_btn = node(3, Some(1), "button");
        zero_bounds_btn.label = Some("Zero".into());
        zero_bounds_btn.bounds.width = 0.0;

        let mut bad_slider = node(4, Some(1), "slider");
        bad_slider.label = Some("Gain".into());
        bad_slider.numeric_minimum = Some(10.0);
        bad_slider.numeric_maximum = Some(5.0);
        bad_slider.numeric_step = Some(1.0);
        bad_slider.numeric_value = Some(f64::NAN);

        let mut unchecked_switch = node(5, Some(1), "switch");
        unchecked_switch.label = Some("Mute".into());

        let mut unselected_tab = node(6, Some(1), "tab");
        unselected_tab.label = Some("Audio".into());

        let mut bad_radio = node(7, Some(1), "radio");
        bad_radio.label = Some("Mode".into());
        bad_radio.checked = Some(true);
        bad_radio.selected = Some(false);

        let mut dup_btn = node(8, Some(1), "button");
        dup_btn.label = Some("Zero".into());

        let errs = validate_accessibility_tree(&[
            root,
            unnamed_btn,
            zero_bounds_btn,
            bad_slider,
            unchecked_switch,
            unselected_tab,
            bad_radio,
            dup_btn,
        ])
        .unwrap_err();
        let kinds: Vec<_> = errs.iter().map(|v| &v.kind).collect();
        assert!(kinds.contains(&&AccessibilityViolationKind::MissingLabel));
        assert!(kinds.contains(&&AccessibilityViolationKind::InvalidBounds));
        assert!(kinds.contains(&&AccessibilityViolationKind::InvalidSliderRange));
        assert!(kinds.contains(&&AccessibilityViolationKind::InvalidSliderValue));
        assert!(kinds.contains(&&AccessibilityViolationKind::MissingCheckedState));
        assert!(kinds.contains(&&AccessibilityViolationKind::MissingSelectedState));
        assert!(kinds.contains(&&AccessibilityViolationKind::RadioCheckedSelectedMismatch));
        assert!(
            kinds.contains(&&AccessibilityViolationKind::DuplicateSiblingLabel {
                label: "Zero".into()
            })
        );

        // Disabled interactive controls may have zero bounds without triggering InvalidBounds.
        let mut disabled_root = node(1, None, "document");
        disabled_root.children = vec![2];
        let mut disabled_btn = node(2, Some(1), "button");
        disabled_btn.label = Some("Hidden".into());
        disabled_btn.disabled = true;
        disabled_btn.bounds.width = 0.0;
        assert_eq!(
            validate_accessibility_tree(&[disabled_root, disabled_btn]),
            Ok(())
        );
    }
}
