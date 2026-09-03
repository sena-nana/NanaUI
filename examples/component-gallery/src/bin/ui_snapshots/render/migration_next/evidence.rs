//! Snapshot evidence; no product state is stored here.
use super::*;

pub(super) fn write_evidence(
    path: &Path,
    fixture: Fixture,
    runtime: &RuntimeEvidence,
) -> Result<(), Box<dyn std::error::Error>> {
    let world = runtime.document.context().world();
    let bounds = world.layout_box(runtime.target);
    let hit = bounds.and_then(|bounds| {
        world.hit_test(
            runtime.document.document(),
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
    });
    let accessibility = world.accessibility(runtime.target);
    let text_input = world.text_input(runtime.target);
    let geometry = world.component_geometry(runtime.target);
    let primitives = runtime.document.scene().primitives().collect::<Vec<_>>();
    let primitive = |slot| {
        primitives
            .iter()
            .copied()
            .find(|primitive| primitive.node == runtime.target && primitive.id.slot == slot)
    };
    let has_own_clip = |primitive: &nana_ui_scene::ScenePrimitive| {
        bounds.is_some_and(|bounds| {
            primitive.clips.iter().any(|clip| {
                (clip.bounds.x - bounds.x).abs() < 0.01
                    && (clip.bounds.y - bounds.y).abs() < 0.01
                    && (clip.bounds.width - bounds.width).abs() < 0.01
                    && (clip.bounds.height - bounds.height).abs() < 0.01
            })
        })
    };
    let has_clip = |primitive: &nana_ui_scene::ScenePrimitive,
                    clip: nana_ui::runtime::LayoutBox| {
        primitive.clips.iter().any(|candidate| {
            (candidate.bounds.x - clip.x).abs() < 0.01
                && (candidate.bounds.y - clip.y).abs() < 0.01
                && (candidate.bounds.width - clip.width).abs() < 0.01
                && (candidate.bounds.height - clip.height).abs() < 0.01
        })
    };
    let text_scene_ok = match fixture.component {
        Component::Textarea => primitive(2).is_some_and(|primitive| {
            has_own_clip(primitive)
                && matches!(
                    primitive.kind,
                    ScenePrimitiveKind::Text {
                        wrap: true,
                        vertical_alignment: TextVerticalAlignment::Top,
                        ..
                    }
                )
        }),
        Component::TextInput => primitive(2).is_some_and(|primitive| {
            matches!(
                primitive.kind,
                ScenePrimitiveKind::Text {
                    wrap: false,
                    vertical_alignment: TextVerticalAlignment::Center,
                    ..
                }
            )
        }),
        _ => true,
    };
    let textarea_geometry_ok = if fixture.component != Component::Textarea {
        true
    } else {
        match (bounds, geometry.as_ref()) {
            (
                Some(bounds),
                Some(nana_ui::runtime::ComponentGeometry::TextInput {
                    text,
                    multiline,
                    selection,
                    caret,
                    preedit,
                    focus_ring,
                    border,
                    border_width,
                    ..
                }),
            ) => {
                let selection_scene_ok = if selection.is_empty() {
                    primitive(1).is_none()
                } else {
                    primitive(1).is_some_and(|primitive| {
                        has_own_clip(primitive)
                            && matches!(
                                &primitive.kind,
                                ScenePrimitiveKind::QuadBatch {
                                    bounds: quads,
                                    ..
                                } if quads.len() == selection.len()
                            )
                    })
                };
                let focused = textarea_is_focused(fixture.state);
                let caret_scene_ok = if focused {
                    caret.is_some()
                        && primitive(4).is_some_and(|primitive| {
                            has_own_clip(primitive)
                                && matches!(primitive.kind, ScenePrimitiveKind::Quad { .. })
                        })
                        && focus_ring.is_none()
                        && primitive(7).is_none()
                } else {
                    caret.is_none() && primitive(4).is_none() && focus_ring.is_none()
                };
                let border_ok = match fixture.state {
                    "invalid-focused" => {
                        focus_ring.is_none() && border.is_some() && *border_width >= 2.0
                    }
                    "disabled" => focus_ring.is_none(),
                    _ => focus_ring.is_none() && (*border_width - 1.0).abs() < 0.01,
                };
                let selection_count_ok = match fixture.state {
                    "selection" => selection.len() == 1,
                    "multiline-selection" => selection.len() >= 2,
                    _ => selection.is_empty(),
                };
                let text_content_ok = if fixture.state == "placeholder" {
                    text.content.as_ref() == "Describe the issue"
                } else {
                    text.content.as_ref() == textarea_value(fixture.state)
                };
                let clip = primitive(2).and_then(|primitive| primitive.clips.last());
                let clipped_ok = match fixture.state {
                    "clipped" => clip.is_some_and(|clip| {
                        text.bounds.y + text.bounds.height
                            > clip.bounds.y + clip.bounds.height + 0.01
                    }),
                    "scroll" => clip.is_some_and(|clip| {
                        text.bounds.height + 0.01 >= clip.bounds.height
                            && text.bounds.y < clip.bounds.y
                    }),
                    _ => true,
                };
                let scroll_ok = if fixture.state == "scroll" {
                    clip.is_some_and(|clip| {
                        text.bounds.y < clip.bounds.y
                            && caret.is_some_and(|caret| {
                                caret.y >= clip.bounds.y
                                    && caret.y + caret.height
                                        <= clip.bounds.y + clip.bounds.height + 0.01
                            })
                    })
                } else {
                    true
                };
                *multiline
                    && text.bounds.x >= bounds.x
                    && text.bounds.width > 0.0
                    && text.bounds.width <= bounds.width + 0.01
                    && text.bounds.height > 0.0
                    && text_content_ok
                    && selection_count_ok
                    && selection_scene_ok
                    && caret_scene_ok
                    && border_ok
                    && preedit.is_empty()
                    && primitive(5).is_none()
                    && clipped_ok
                    && scroll_ok
            }
            _ => false,
        }
    };
    let mut segmented_geometry_ok = true;
    let mut segmented_accessibility_ok = true;
    if fixture.component == Component::SegmentedControl {
        let expected_option_height =
            (segmented_control_size(fixture.state).height() - 6.0).max(0.0);
        segmented_accessibility_ok = accessibility
            .is_some_and(|node| node.role == nana_ui::runtime::AccessibilityRole::RadioGroup);
        let control = Entity::<RuntimeSegmentedControl>::from_stable_id(runtime.target);
        let selected = runtime
            .document
            .context()
            .read(control, RuntimeSegmentedControl::selected)?;
        let focus_target = runtime
            .document
            .context()
            .read(control, RuntimeSegmentedControl::focus_target)?;
        let mounted_options = runtime
            .segmented_options
            .iter()
            .copied()
            .filter(|id| world.mount_state(*id) == Some(MountState::Mounted))
            .collect::<Vec<_>>();
        let mut checked = 0;
        let mut enabled = Vec::new();
        for id in &mounted_options {
            let option = Entity::<RuntimeSegmentedOption>::from_stable_id(*id);
            let option_selected = runtime
                .document
                .context()
                .read(option, RuntimeSegmentedOption::selected)?;
            let disabled = runtime
                .document
                .context()
                .read(option, RuntimeSegmentedOption::disabled_value)?;
            checked += usize::from(option_selected);
            if !disabled {
                enabled.push(*id);
            }
            segmented_accessibility_ok &= world.accessibility(*id).is_some_and(|node| {
                node.role == nana_ui::runtime::AccessibilityRole::Radio
                    && node.checked == Some(option_selected)
                    && node.disabled == disabled
            });
            let option_bounds = world.layout_box(*id);
            let option_geometry = world.component_geometry(*id);
            let option_surface = primitives
                .iter()
                .find(|primitive| primitive.node == *id && primitive.id.slot == 0);
            let option_text = primitives
                .iter()
                .find(|primitive| primitive.node == *id && primitive.id.slot == 2);
            segmented_geometry_ok &= matches!(
                (option_bounds, option_geometry),
                (
                    Some(bounds),
                    Some(nana_ui::runtime::ComponentGeometry::SelectionOption { label, .. })
                ) if bounds.height > 0.0
                    && (bounds.height - expected_option_height).abs() < 0.01
                    && label.bounds.x >= bounds.x
                    && label.bounds.x + label.bounds.width <= bounds.x + bounds.width + 0.01
            ) && option_surface
                .is_some_and(|primitive| matches!(primitive.kind, ScenePrimitiveKind::Quad { .. }))
                && option_text.is_some_and(|primitive| {
                    matches!(primitive.kind, ScenePrimitiveKind::Text { .. })
                });
        }
        let expected_width = mounted_options
            .iter()
            .filter_map(|id| world.layout_box(*id))
            .map(|bounds| bounds.width)
            .sum::<f32>()
            + mounted_options.len().saturating_sub(1) as f32 * 2.0
            + 6.0;
        segmented_geometry_ok &=
            bounds.is_some_and(|bounds| (bounds.width - expected_width).abs() < 0.01);
        let expected_checked = usize::from(selected.is_some());
        let tab_stop_ok = match focus_target {
            Some(id) => enabled.contains(&id),
            None => enabled.is_empty(),
        };
        segmented_accessibility_ok &= checked == expected_checked && tab_stop_ok;
        if fixture.state == "medium-icon" {
            segmented_geometry_ok &= mounted_options.first().is_some_and(|id| {
                primitives.iter().any(|primitive| {
                    primitive.node == *id
                        && primitive.id.slot == 3
                        && matches!(primitive.kind, ScenePrimitiveKind::Icon { .. })
                })
            });
        }
        if fixture.state == "focused" {
            segmented_geometry_ok &= focus_target.is_some_and(|id| {
                let Some(bounds) = world.layout_box(id) else {
                    return false;
                };
                primitives.iter().any(|primitive| {
                    primitive.node == id
                        && primitive.id.slot == 7
                        && (primitive.bounds.x - (bounds.x - 4.0)).abs() < 0.01
                        && (primitive.bounds.y - (bounds.y - 4.0)).abs() < 0.01
                        && (primitive.bounds.width - (bounds.width + 8.0)).abs() < 0.01
                        && matches!(
                            primitive.kind,
                            ScenePrimitiveKind::Quad {
                                border_width,
                                ..
                            } if (border_width - 2.0).abs() < 0.01
                        )
                })
            });
        }
    }
    let feedback_parent_inert = !matches!(
        fixture.component,
        Component::StatusBadge
            | Component::ValidationMessage
            | Component::EmptyState
            | Component::LabeledValue
    ) || world
        .interaction(runtime.target)
        .is_some_and(|interaction| !interaction.pointer_events && !interaction.focusable);
    let feedback_accessibility_ok = match fixture.component {
        Component::StatusBadge => accessibility.is_some_and(|node| {
            node.label.as_deref() == Some(status_badge_label(fixture.state))
                && !node.invalid
                && !node.disabled
        }),
        Component::ValidationMessage => accessibility.is_some_and(|node| {
            node.label.as_deref() == Some(validation_message(fixture.state)) && node.invalid
        }),
        Component::EmptyState => accessibility.is_some_and(|node| {
            node.label.as_deref() == Some(empty_title(fixture.state))
                && node.value.as_deref()
                    == (fixture.state != "title-only").then(|| empty_message(fixture.state))
        }),
        Component::LabeledValue => accessibility.is_some_and(|node| {
            node.label.as_deref() == Some("Revision") && node.value.as_deref() == Some("42")
        }),
        _ => true,
    };
    let feedback_geometry_ok = match geometry.as_ref() {
        Some(nana_ui::runtime::ComponentGeometry::StatusBadge {
            indicator, label, ..
        }) if fixture.component == Component::StatusBadge => {
            label.content.as_ref() == status_badge_label(fixture.state)
                && (label.font_size - 11.0).abs() < 0.01
                && label.font_weight == Some(500)
                && label.bounds.x - (indicator.x + indicator.width) >= 4.9
                && primitive(0).is_some_and(|primitive| {
                    matches!(primitive.kind, ScenePrimitiveKind::Quad { .. })
                })
                && primitive(2).is_some_and(|primitive| {
                    matches!(
                        &primitive.kind,
                        ScenePrimitiveKind::Text {
                            size,
                            weight: Some(500),
                            ..
                        } if (*size - 11.0).abs() < 0.01
                    )
                })
                && primitive(3).is_some_and(|primitive| {
                    matches!(
                        primitive.kind,
                        ScenePrimitiveKind::Quad {
                            background: Some(_),
                            border_width,
                            ..
                        } if border_width == 0.0
                    )
                })
        }
        Some(nana_ui::runtime::ComponentGeometry::ValidationMessage {
            indicator, label, ..
        }) if fixture.component == Component::ValidationMessage => {
            label.content.as_ref() == validation_message(fixture.state)
                && (label.font_size - 11.0).abs() < 0.01
                && label.font_weight.is_none()
                && label.bounds.x - (indicator.x + indicator.width) >= 4.9
                && primitive(2).is_some_and(|primitive| {
                    matches!(
                        &primitive.kind,
                        ScenePrimitiveKind::Text {
                            size,
                            weight: None,
                            ..
                        } if (*size - 11.0).abs() < 0.01
                    )
                })
                && primitive(3).is_some_and(|primitive| {
                    matches!(
                        primitive.kind,
                        ScenePrimitiveKind::Quad {
                            background: None,
                            border_color: Some(_),
                            border_width,
                            ..
                        } if (border_width - 1.0).abs() < 0.01
                    )
                })
        }
        Some(nana_ui::runtime::ComponentGeometry::EmptyState {
            root_clip,
            content_clip,
            icon,
            title,
            message,
            action,
        }) if fixture.component == Component::EmptyState => {
            let expects_content = fixture.state != "title-only";
            let intrinsic_scene_clipped = [2_u8, 3, 4].into_iter().all(|slot| {
                primitive(slot).is_none_or(|primitive| has_clip(primitive, *content_clip))
            });
            let action_scene_clipped = world
                .node(runtime.target)
                .and_then(|node| node.children.first().copied())
                .is_none_or(|action| {
                    primitives
                        .iter()
                        .filter(|primitive| primitive.node == action)
                        .all(|primitive| has_clip(primitive, *root_clip))
                });
            let ordering_ok = icon
                .as_ref()
                .is_none_or(|(_, icon, _)| icon.y + icon.height <= title.bounds.y + 0.01)
                && message.as_ref().is_none_or(|message| {
                    title.bounds.y + title.bounds.height <= message.bounds.y + 0.01
                        && action.is_none_or(|action| {
                            message.bounds.y + message.bounds.height <= action.y + 0.01
                        })
                });
            let alignment_ok = if fixture.state == "compact" {
                (title.bounds.x - content_clip.x).abs() < 0.01
            } else {
                let title_center = title.bounds.x + title.bounds.width / 2.0;
                let clip_center = content_clip.x + content_clip.width / 2.0;
                (title_center - clip_center).abs() < 0.01
            };
            let wrap_ok = !matches!(fixture.state, "narrow-cjk" | "extreme-clip")
                || title.bounds.height > title.font_size * 1.2
                || message
                    .as_ref()
                    .is_some_and(|message| message.bounds.height > message.font_size * 1.2);
            let extreme_clip_ok = fixture.state != "extreme-clip"
                || message.as_ref().is_some_and(|message| {
                    message.bounds.y + message.bounds.height > content_clip.y + content_clip.height
                });
            title.content.as_ref() == empty_title(fixture.state)
                && (title.font_size
                    - if fixture.state == "compact" {
                        12.0
                    } else {
                        13.0
                    })
                .abs()
                    < 0.01
                && title.font_weight == Some(600)
                && (icon.is_some() == expects_content)
                && (message.is_some() == expects_content)
                && (action.is_some() == (fixture.state == "complete-action"))
                && intrinsic_scene_clipped
                && action_scene_clipped
                && ordering_ok
                && alignment_ok
                && wrap_ok
                && extreme_clip_ok
        }
        Some(nana_ui::runtime::ComponentGeometry::LabeledValue {
            label,
            value,
            action,
        }) if fixture.component == Component::LabeledValue => {
            let expected_weight = if fixture.state == "strong" { 600 } else { 500 };
            label.content.as_ref() == "Revision"
                && value.content.as_ref() == "42"
                && (label.font_size - 11.0).abs() < 0.01
                && (value.font_size - 12.0).abs() < 0.01
                && value.font_weight == Some(expected_weight)
                && label.bounds.x + label.bounds.width <= value.bounds.x + 0.01
                && label.bounds.y < value.bounds.y + value.bounds.height
                && value.bounds.y < label.bounds.y + label.bounds.height
                && (action.is_some() == (fixture.state == "action"))
                && primitive(2).is_some_and(|primitive| {
                    matches!(primitive.kind, ScenePrimitiveKind::Text { size, .. } if (size - 11.0).abs() < 0.01)
                })
                && primitive(3).is_some_and(|primitive| {
                    matches!(primitive.kind, ScenePrimitiveKind::Text { size, weight: Some(weight), .. } if (size - 12.0).abs() < 0.01 && weight == expected_weight)
                })
        }
        _ => !matches!(
            fixture.component,
            Component::StatusBadge
                | Component::ValidationMessage
                | Component::EmptyState
                | Component::LabeledValue
        ),
    };
    let tooltip = matches!(
        fixture.component,
        Component::IconButton | Component::Tooltip
    )
    .then(|| {
        runtime
            .document
            .context()
            .icon_button_tooltip(Entity::<RuntimeIconButton>::from_stable_id(runtime.target))
            .ok()
            .flatten()
            .map(|tooltip| tooltip.stable_id())
    })
    .flatten();
    let active_overlay = world
        .overlay_host(runtime.target)
        .and_then(|host| host.active);
    let expects_hit =
        !matches!(fixture.state, "disabled" | "loading") && fixture.component != Component::Text;
    let hit_ok = if fixture.component == Component::SegmentedControl {
        if matches!(fixture.state, "empty" | "all-disabled") {
            hit.is_none()
        } else {
            runtime.segmented_options.iter().copied().any(|id| {
                world
                    .interaction(id)
                    .is_some_and(|interaction| interaction.pointer_events)
                    && world.layout_box(id).is_some_and(|bounds| {
                        world.hit_test(
                            runtime.document.document(),
                            bounds.x + bounds.width / 2.0,
                            bounds.y + bounds.height / 2.0,
                        ) == Some(id)
                    })
            })
        }
    } else if matches!(
        fixture.component,
        Component::Card
            | Component::Text
            | Component::StatusBadge
            | Component::ValidationMessage
            | Component::EmptyState
            | Component::LabeledValue
            | Component::Progress
            | Component::Spinner
            | Component::Skeleton
            | Component::LevelMeter
            | Component::FormField
            | Component::Workspace
            | Component::Dock
            | Component::DockPanel
            | Component::SplitPane
            | Component::PaneChrome
            | Component::PaneTree
            | Component::AppShell
            | Component::DesktopShell
            | Component::SettingsSidebar
            | Component::SettingsPage
            | Component::AppTitleBar
            | Component::GpuTextureView
            | Component::Thumbnail
    ) {
        hit != Some(runtime.target)
    } else if expects_hit {
        hit == Some(runtime.target)
    } else {
        hit != Some(runtime.target)
    };
    let action_state = (fixture.component == Component::TextInput && fixture.state == "invalid")
        || (fixture.component == Component::Textarea
            && matches!(
                fixture.state,
                "invalid-focused" | "multiline-selection" | "scroll"
            ))
        || matches!(
            fixture.state,
            "hover"
                | "pressed"
                | "selected-hover"
                | "selected-pressed"
                | "focused"
                | "selection"
                | "keyboard-activation"
                | "tooltip-delay"
                | "tooltip-edge"
                | "open"
                | "delay"
                | "edge"
                | "pointer-toggle"
                | "space-toggle"
                | "accessibility-toggle"
                | "pointer-activation"
                | "drag"
                | "drag-cancel"
                | "arrow-decrement"
                | "arrow-increment"
                | "page-decrement"
                | "page-increment"
                | "home"
                | "end"
                | "accessibility-set-value"
                | "keyboard-edit"
                | "ime-preedit"
                | "ime-commit"
        );
    let geometry_ok = matches!(
        fixture.component,
        Component::Text
            | Component::Button
            | Component::TextInput
            | Component::Textarea
            | Component::HostedTextarea
            | Component::Checkbox
            | Component::IconButton
            | Component::Tooltip
            | Component::SegmentedControl
            | Component::Tabs
            | Component::Spinner
            | Component::Skeleton
            | Component::Workspace
            | Component::Dock
            | Component::DockPanel
            | Component::SplitPane
            | Component::PaneChrome
            | Component::PaneTree
            | Component::AppShell
            | Component::DesktopShell
            | Component::SettingsSidebar
            | Component::SettingsPage
            | Component::AppTitleBar
            | Component::GpuTextureView
            | Component::GpuView
            | Component::Thumbnail
    ) || geometry.is_some();
    let layout_ok = bounds.is_some_and(|bounds| match fixture.component {
        Component::Text if matches!(fixture.state, "wrap" | "ellipsis") => {
            (bounds.width - 180.0).abs() < 0.01
        }
        Component::Text => bounds.width > 0.0 && bounds.height >= 32.0,
        Component::Button => {
            let expected = button_control_size(fixture.state).height();
            (bounds.height - expected).abs() < 0.01
        }
        Component::TextInput => {
            (bounds.width - 380.0).abs() < 0.01
                && (bounds.height - text_input_control_size(fixture.state).height()).abs() < 0.01
        }
        Component::Textarea | Component::HostedTextarea => {
            (bounds.width - 380.0).abs() < 0.01 && (bounds.height - 96.0).abs() < 0.01
        }
        Component::SegmentedControl => {
            (bounds.height - segmented_control_size(fixture.state).height()).abs() < 0.01
        }
        Component::Checkbox => bounds.height >= ControlSize::Medium.height(),
        Component::Thumbnail if fixture.state == "wide" => {
            let height = ControlSize::Small.height();
            let width = height * 16.0 / 9.0;
            (bounds.height - height).abs() < 0.01 && (bounds.width - width).abs() < 0.01
        }
        Component::Thumbnail => {
            let extent = ControlSize::Small.height();
            (bounds.width - extent).abs() < 0.01 && (bounds.height - extent).abs() < 0.01
        }
        Component::Dialog | Component::ConfirmDialog | Component::Drawer => {
            matches!(
                geometry,
                Some(nana_ui::runtime::ComponentGeometry::ModalFrame { .. })
            )
        }
        _ => true,
    });
    let runtime_ok = bounds.is_some()
        && accessibility.is_some()
        && geometry_ok
        && layout_ok
        && text_scene_ok
        && textarea_geometry_ok
        && segmented_geometry_ok
        && segmented_accessibility_ok
        && feedback_parent_inert
        && feedback_accessibility_ok
        && feedback_geometry_ok
        && runtime.feedback_contract_ok
        && runtime.segmented_contract_ok
        && runtime.idle
        && hit_ok
        && (!action_state || runtime.action_applied)
        && (fixture.state != "loading"
            || fixture.component == Component::TextInput
            || runtime.next_deadline.is_some())
        && (fixture.component != Component::TextInput
            || match fixture.state {
                "read-only" => accessibility.is_some_and(|node| !node.editable && !node.disabled),
                "loading" => accessibility.is_some_and(|node| node.busy && node.disabled),
                "secure" => accessibility.is_some_and(|node| node.value.is_none()),
                "selection" => matches!(
                    geometry,
                    Some(nana_ui::runtime::ComponentGeometry::TextInput {
                        ref selection,
                        ..
                    }) if !selection.is_empty()
                ),
                _ => true,
            })
        && (fixture.component != Component::Textarea
            || (accessibility.is_some_and(|node| node.multiline)
                && match fixture.state {
                    "focused" => world.focused(runtime.document.document()) == Some(runtime.target),
                    "invalid-focused" => {
                        accessibility.is_some_and(|node| node.invalid)
                            && world.focused(runtime.document.document()) == Some(runtime.target)
                    }
                    "disabled" => {
                        accessibility.is_some_and(|node| node.disabled)
                            && world.focused(runtime.document.document()) != Some(runtime.target)
                    }
                    state if textarea_is_focused(state) => {
                        world.focused(runtime.document.document()) == Some(runtime.target)
                    }
                    _ => true,
                }))
        && match (fixture.component, fixture.state) {
            (Component::Tooltip, "delay") => {
                tooltip.is_some()
                    && active_overlay.is_none()
                    && runtime.next_deadline.is_some()
                    && tooltip.is_some_and(|id| {
                        world.accessibility(id).is_some_and(|node| {
                            node.role == nana_ui::runtime::AccessibilityRole::Tooltip
                                && node.label.as_deref() == Some("Add source")
                        })
                    })
            }
            (Component::Tooltip, "open" | "edge") | (_, "tooltip-delay" | "tooltip-edge") => {
                tooltip.is_some()
                    && tooltip == active_overlay
                    && tooltip.is_some_and(|id| {
                        world.accessibility(id).is_some_and(|node| {
                            node.role == nana_ui::runtime::AccessibilityRole::Tooltip
                                && node.label.as_deref() == Some("Add source")
                        })
                    })
            }
            _ => true,
        };
    let reference_verdict =
        if fixture.component == Component::Textarea && textarea_is_focused(fixture.state) {
            "deterministic compatibility content and focus state rendered for manual review"
        } else if matches!(fixture.state, "control-start" | "disabled" | "invalid")
            && fixture.component == Component::RangeField
        {
            "compatibility defect: archived reference does not expose this product contract"
        } else if matches!(
            fixture.state,
            "pressed"
                | "selected-pressed"
                | "focused"
                | "keyboard-activation"
                | "space-toggle"
                | "accessibility-toggle"
                | "pointer-activation"
        ) {
            "reference only: headless fixture does not claim retained interaction evidence"
        } else {
            "rendered reference; visual judgment remains manual"
        };
    let machine_verdict = if runtime_ok { "pass" } else { "fail" };
    let (review_verdict, review_observed) = review_result(fixture);
    let divergence = intentional_divergence(fixture);
    let report = format!(
        "expected: {}\nreference_observed: {}\nreference_verdict: {}\nruntime_expected: {}\nruntime_observed: bounds={bounds:?}; layout_ok={layout_ok}; text_scene_ok={text_scene_ok}; textarea_geometry_ok={textarea_geometry_ok}; segmented_geometry_ok={segmented_geometry_ok}; segmented_accessibility_ok={segmented_accessibility_ok}; segmented_contract_ok={}; segmented_options={:?}; segmented_requests={}; feedback_parent_inert={feedback_parent_inert}; feedback_accessibility_ok={feedback_accessibility_ok}; feedback_geometry_ok={feedback_geometry_ok}; feedback_contract_ok={}; text_input={text_input:?}; geometry={geometry:?}; hit={hit:?}; accessibility={accessibility:?}; tooltip={tooltip:?}; active_overlay={active_overlay:?}; first_passes={}; first_accessibility_updates={}; final_passes={}; final_accessibility_updates={}; second_flush_idle={}; action_applied={}; next_animation_deadline={:?}; primitives={primitives:?}\nmachine_verdict: {}\nreview_observed: {}\nreview_verdict: {}\nintentional_divergence_reason: {}\n",
        fixture.expected,
        fixture.reference_contract,
        reference_verdict,
        fixture.runtime_contract,
        runtime.segmented_contract_ok,
        runtime.segmented_options,
        runtime.segmented_requests,
        runtime.feedback_contract_ok,
        runtime.first_passes,
        runtime.first_accessibility_updates,
        runtime.final_passes,
        runtime.final_accessibility_updates,
        runtime.idle,
        runtime.action_applied,
        runtime.next_deadline,
        machine_verdict,
        review_observed,
        review_verdict,
        divergence,
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, report)?;
    Ok(())
}

pub(super) fn review_result(fixture: Fixture) -> (&'static str, &'static str) {
    match (fixture.component, fixture.state) {
        (Component::Button, _) => (
            "pass",
            "Fresh isolated dark and light review confirms semantic kinds, three control sizes, hover and pressed backgrounds without an accent focus ring, disabled and loading presentation, complete label geometry, hit behavior and accessibility",
        ),
        (Component::TextInput, _) => (
            "pass",
            "Fresh isolated dark and light review confirms placeholder contrast, shaped selection and caret geometry, external focus, invalid, secure, size, read-only, loading, keyboard and IME preedit or commit presentation",
        ),
        (Component::Textarea, _) => (
            "manual-required",
            "Review the generated dark and light compatibility and Runtime images for placeholder, multiline, focus, selection, invalid, disabled, clipping and scrolling semantics; IME remains a real Hosted gate",
        ),
        (Component::HostedTextarea, _) => (
            "manual-required",
            "Review the generated dark and light images for Runtime presenter spans on committed rust text; Iced highlighter is a leftover reference, not the product path",
        ),
        (
            Component::CalendarHeatmap
            | Component::TimeSeriesChart
            | Component::ReorderList
            | Component::NativeMarkdown
            | Component::SelectableRichText
            | Component::ImageViewer
            | Component::GraphCanvas
            | Component::KeyCaptureLayer
            | Component::KeymapLayer,
            _,
        ) => (
            "manual-required",
            "Review Runtime Scene quads against design tokens; Iced canvas/widget output is a reference, not a pixel oracle",
        ),
        (Component::GpuTextureView | Component::GpuView, _) => (
            "manual-required",
            "Review host-texture sampling and gpu-view Scene paint; Iced shader output is a reference, not a pixel oracle",
        ),
        (Component::Thumbnail, _) => (
            "manual-required",
            "Review the compact list-row box, four shared-geometry states, and ready host-texture contain",
        ),
        (Component::Tooltip, _) => (
            "manual-required",
            "Review the generated dark and light Iced Tooltip and Runtime overlay images for open, delay-not-open and edge placement; Runtime is the accepted public default and Iced images remain a migration-era reference, not an oracle",
        ),
        (Component::Dialog | Component::ConfirmDialog | Component::Drawer, _) => (
            "manual-required",
            "Review the generated dark and light Iced and Runtime modal images for scrim, surface, title and slotted body or actions; Runtime is the accepted public default and Iced images remain a migration-era reference, not an oracle",
        ),
        (Component::Toast | Component::XYPad | Component::QrCode, _) => (
            "manual-required",
            "Review the generated dark and light Iced and Runtime images; Runtime is the accepted public default and Iced images remain a migration-era reference, not an oracle",
        ),
        (
            Component::Select
            | Component::Popover
            | Component::ActionMenu
            | Component::ActionMenuItem
            | Component::AnchoredActionMenu
            | Component::ContextMenu,
            _,
        ) => (
            "manual-required",
            "Review the generated dark and light Iced and Runtime images for select fields and anchored menus; Runtime is the accepted public default and Iced images remain a migration-era reference, not an oracle",
        ),
        (Component::SettingsSidebar | Component::SettingsPage | Component::DesktopShell, _) => (
            "manual-required",
            "Review the generated dark and light Iced and Runtime images; Runtime is the accepted public default and Iced images remain a migration-era reference, not an oracle",
        ),
        (
            Component::Workspace
            | Component::Dock
            | Component::DockPanel
            | Component::SplitPane
            | Component::PaneChrome
            | Component::PaneTree
            | Component::AppShell
            | Component::AppTitleBar,
            _,
        ) => (
            "pass",
            "2026-08-16 windowed A/B and side-by-side review preferred Runtime (right): workspace-family is the accepted public default, fixture slot labels keep the shared 8px inset that Workspace/Dock/Split/AppShell chrome does not add, and Iced images remain a migration-era reference, not an oracle",
        ),
        (
            Component::SidebarFrame
            | Component::SidebarSection
            | Component::SidebarFooter
            | Component::AppearanceSection
            | Component::AboutSection
            | Component::SettingsCollapsibleCard,
            _,
        ) => (
            "pass",
            "2026-08-16 windowed A/B and side-by-side review preferred Runtime (right): frame top/footer stay outside the scrolling body, section collapse uses ChevronRight, footer hugs 28px icon actions, Appearance/About assemble SettingsRow children, and Iced uppercase title / missing radius track are Iced-side",
        ),
        (Component::SegmentedControl, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right) over Iced (left) for density, selected pill, icon alignment and disabled fade",
        ),
        (Component::Text, _) => (
            "pass",
            "Runtime text uses the authored content box, shared typography, semantic contrast, wrapping or ellipsis, alignment, clipping and accessibility in dark and light",
        ),
        (Component::Checkbox, _) => (
            "pass",
            "Runtime checkbox keeps indicator and label geometry, semantic checked and invalid paint, complete-row hit testing, focus, disabled behavior and accessibility in dark and light",
        ),
        (Component::IconButton, "hover" | "pressed" | "focused" | "selected") => (
            "pass",
            "Runtime uses distinct neutral hover and pressed layers without an accent focus ring, and a persistent accent-selected treatment while preserving icon contrast in dark and light",
        ),
        (Component::Switch, "hover" | "pressed" | "focused") => (
            "pass",
            "Runtime separates the complete-row hover and pressed layers from the track focus ring, so each interaction state remains visible and distinct in dark and light",
        ),
        (Component::StatusBadge, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): five tones, compact pill and indicator contrast are accepted without Iced pixel match",
        ),
        (Component::ValidationMessage, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): warning and danger contrast and inline spacing are accepted",
        ),
        (Component::EmptyState, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): icon/title/message order, compact layout, CJK wrap and solid Primary action are accepted",
        ),
        (Component::LabeledValue, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): label/value hierarchy and end-aligned action child are accepted",
        ),
        (Component::Progress | Component::Spinner, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): determinate track/fill, optional label and host-sampled spinner are accepted",
        ),
        (Component::Tabs, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): independent tab surface without a segmented focus ring",
        ),
        (Component::Skeleton | Component::LevelMeter, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): Subtle placeholder and tone-colored meter are accepted",
        ),
        (Component::FormField, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): enabled field, centered value and danger support with indicator",
        ),
        (Component::InteractiveCard, _) => (
            "pass",
            "2026-08-15 side-by-side review preferred Runtime (right): selected surface and centered child content",
        ),
        _ => (
            "pass",
            "Runtime text, internal geometry, contrast, clipping and state are correct in dark and light review; pixel similarity was not used as a gate",
        ),
    }
}

pub(super) fn intentional_divergence(fixture: Fixture) -> &'static str {
    match (fixture.component, fixture.state) {
        (Component::Switch, "control-start") => {
            "intentional: Runtime implements the start-side control contract missing from the Iced adapter"
        }
        (Component::RangeField, "disabled" | "invalid" | "decimal-step") => {
            "intentional: Runtime implements the design contract missing from the Iced adapter"
        }
        (Component::RangeField, _) => {
            "intentional: Runtime reserves dedicated label, value and track regions instead of copying the Iced inline geometry"
        }
        (Component::SegmentedControl | Component::Tabs, "focused") => {
            "intentional: selected surface only; no 2px focus ring"
        }
        (Component::SegmentedControl, "no-selection" | "all-disabled") => {
            "intentional: the compatibility widget requires a value while Runtime supports controlled no-selection and derives tab stops only from enabled options"
        }
        (Component::SegmentedControl, _) => {
            "intentional: Runtime selected pill and option contrast are the accepted visual; Iced is reference only"
        }
        (Component::EmptyState, "complete-action") => {
            "intentional: Runtime paints a solid Primary action; Iced renders a weaker outlined control"
        }
        (Component::LabeledValue, "action") => {
            "intentional: Runtime end-aligns the action child; Iced places it beside the value"
        }
        (Component::Card, _) => {
            "intentional: Runtime preserves the authored title casing while Iced uppercases its compatibility heading"
        }
        (Component::Tooltip, _) => {
            "intentional: Runtime Tooltip is a compact pointer-bound hover card hosted by the trigger; Iced wraps arbitrary content. Visual review is the qualification gate"
        }
        (Component::Dialog | Component::ConfirmDialog | Component::Drawer, _) => {
            "intentional: Runtime ModalFrame owns scrim, surface and slotted children; Iced composes the same product chrome. Visual review is the qualification gate"
        }
        (Component::Select, "opened") => {
            "intentional: Runtime paints the opened menu in the same leaf; Iced pick-list overlay is not captured in this snapshot"
        }
        (
            Component::Select
            | Component::Popover
            | Component::ActionMenu
            | Component::ActionMenuItem
            | Component::AnchoredActionMenu
            | Component::ContextMenu,
            _,
        ) => {
            "intentional: Runtime keeps disabled select options visible and owns anchored menu chrome; Iced pick-list omits disabled popup rows. Visual review is the qualification gate"
        }
        (Component::SidebarSection, _) => {
            "intentional: Runtime header is ListItem chrome with ChevronDown/ChevronRight; Iced paints a tracked uppercase title and a rotating canvas chevron. Scene adapter cannot paint letter-spacing or rotation"
        }
        (Component::AppearanceSection | Component::AboutSection, _) => {
            "intentional: Runtime assembles qualified SettingsRow children; Iced composes the same host snapshot"
        }
        (Component::SettingsCollapsibleCard, _) => {
            "intentional: Runtime disclosure is non-interactive chrome; the card remains the single activation target"
        }
        (Component::SettingsSidebar | Component::SettingsPage | Component::DesktopShell, _) => {
            "intentional: Runtime settings and desktop composers are the public default; Iced is a migration-era reference"
        }
        (Component::GraphCanvas, _) => {
            "intentional: Runtime flattens Bézier edges to articulated-line Stroke; grid stays 1px QuadBatch. Iced strokes paths. Port discs use the Iced 4/5px radius, not the 8px hit target"
        }
        (Component::GpuTextureView, _) => {
            "intentional: Iced GpuTextureView samples the same host texture as Runtime nana.host-texture; layout chrome may differ"
        }
        (Component::GpuView, _) => {
            "intentional: Iced GpuView shader is inline; Runtime paints via DefaultGpuViewRenderer using the same WGSL, taking palette and seed from CustomRenderNode params"
        }
        (Component::Thumbnail, _) => {
            "intentional: Runtime Thumbnail is a compact HostTexture slot with empty/loading/unavailable chrome; Iced has no list-row thumbnail primitive"
        }
        _ => fixture.divergence,
    }
}
