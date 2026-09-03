//! Snapshot workspace; no product state is stored here.
use super::*;

pub(super) fn mount_runtime_sidebar_section(
    document: &mut RuntimeDocument,
    expanded: bool,
    labels: &[&str],
    collapsible: bool,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let spec = RuntimeSidebarSection::new("资源")
        .count(3)
        .collapsible(collapsible)
        .expanded(expanded);
    Ok(document.context_mut().build(document_id, |ui| {
        let disclosure = collapsible.then(|| ui.leaf(spec.disclosure_mark()));
        let title = ui.leaf(spec.title_label());
        let count = ui.leaf(spec.count_label());
        let spec = spec
            .title_slot(title.stable_id())
            .count_slot(count.stable_id());
        let spec = match &disclosure {
            Some(disclosure) => spec.disclosure(disclosure.stable_id()),
            None => spec,
        };
        let header = ui.leaf(spec.header_item());
        ui.nest(header, |ui| {
            if let Some(disclosure) = disclosure {
                ui.adopt(disclosure);
            }
            ui.adopt(title);
            ui.adopt(count);
        });
        let body = ui.leaf(RuntimeSidebarSection::body_port());
        ui.nest(body, |ui| {
            for (index, label) in labels.iter().enumerate() {
                ui.child(format!("row-{index}"), RuntimeSidebarRow::new(*label));
            }
        });
        let section = ui.child(
            "section",
            spec.header(header.stable_id()).body(body.stable_id()),
        );
        ui.nest(section, |ui| {
            ui.adopt(header);
            ui.adopt(body);
        });
        section.stable_id()
    })?)
}

pub(super) fn mount_runtime_workspace(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let workspace = document.context_mut().build(document_id, |ui| {
        let nav = ui.leaf(RuntimeText::new("Nav").style(slot_label_style()));
        let files = ui.leaf(RuntimeText::new("Files").style(slot_label_style()));
        let toolbar = ui.leaf(RuntimeText::new("Toolbar").style(slot_label_style()));
        let primary = ui.leaf(RuntimeText::new("Primary").style(slot_label_style()));
        let inspector = ui.leaf(RuntimeText::new("Inspector").style(slot_label_style()));
        let diagnostics = ui.leaf(RuntimeText::new("Diagnostics").style(slot_label_style()));
        let workspace = ui.child(
            "workspace",
            RuntimeWorkspace::from_model(
                &WorkspaceModel::new(),
                [
                    WorkspaceRegionSlot::new(RegionId::GlobalNavigation, nav.stable_id()),
                    WorkspaceRegionSlot::new(RegionId::Resources, files.stable_id()),
                    WorkspaceRegionSlot::new(RegionId::PrimaryToolbar, toolbar.stable_id()),
                    WorkspaceRegionSlot::new(RegionId::Primary, primary.stable_id()),
                    WorkspaceRegionSlot::new(RegionId::Inspector, inspector.stable_id()),
                    WorkspaceRegionSlot::new(RegionId::Diagnostics, diagnostics.stable_id()),
                ],
            ),
        );
        ui.nest(workspace, |ui| {
            ui.adopt(nav);
            ui.adopt(files);
            ui.adopt(toolbar);
            ui.adopt(primary);
            ui.adopt(inspector);
            ui.adopt(diagnostics);
        });
        workspace
    })?;
    document.context_mut().assemble_workspace(workspace)?;
    Ok(workspace.stable_id())
}

pub(super) fn mount_runtime_dock(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let dock = document.context_mut().build(document_id, |ui| {
        let nav = ui.leaf(RuntimeText::new("Nav").style(slot_label_style()));
        let files = ui.leaf(RuntimeText::new("Files").style(slot_label_style()));
        let primary = ui.leaf(RuntimeText::new("Primary").style(slot_label_style()));
        let dock = ui.child(
            "dock",
            RuntimeDock::new(RuntimeDockNode::split(
                nana_ui::runtime::DockAxis::Horizontal,
                0.35,
                RuntimeDockNode::tabs(
                    ["nav", "files"],
                    "nav",
                    [
                        ("nav", Some(nav.stable_id())),
                        ("files", Some(files.stable_id())),
                    ],
                ),
                RuntimeDockNode::item("primary", Some(primary.stable_id())),
            ))
            .title("nav", "Nav")
            .title("files", "Files")
            .title("primary", "Primary"),
        );
        ui.nest(dock, |ui| {
            ui.adopt(nav);
            ui.adopt(files);
            ui.adopt(primary);
        });
        dock
    })?;
    document.context_mut().assemble_dock(dock)?;
    Ok(dock.stable_id())
}

pub(super) fn mount_runtime_split_pane(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let pane = document.context_mut().build(document_id, |ui| {
        let first = ui.leaf(RuntimeText::new("First").style(slot_label_style()));
        let second = ui.leaf(RuntimeText::new("Second").style(slot_label_style()));
        let indicator = ui.leaf(RuntimeText::new(""));
        let handle = ui.leaf(RuntimeText::new(""));
        ui.nest(handle, |ui| ui.adopt(indicator));
        let pane = ui.child(
            "pane",
            RuntimeSplitPane::from_model(
                &SplitPaneModel::new(SplitAxis::Horizontal, 160.0, 80.0, 280.0),
                first.stable_id(),
                second.stable_id(),
            )
            .handle(handle.stable_id()),
        );
        ui.nest(pane, |ui| {
            ui.adopt(first);
            ui.adopt(handle);
            ui.adopt(second);
        });
        pane
    })?;
    document.context_mut().update_component(pane, |_, _| {})?;
    Ok(pane.stable_id())
}

pub(super) fn mount_runtime_pane_chrome(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    Ok(document.context_mut().build(document_id, |ui| {
        let header = ui.leaf(RuntimeText::new(""));
        let tabs = ui.leaf(RuntimeText::new("editor.rs"));
        let body = ui.leaf(RuntimeText::new("Body"));
        let close = ui.leaf(RuntimeText::new("关闭"));
        ui.nest(header, |ui| {
            ui.adopt(tabs);
            ui.adopt(close);
        });
        let chrome = ui.child(
            "chrome",
            RuntimePaneChrome::new()
                .header(header.stable_id())
                .tabs(tabs.stable_id())
                .body(body.stable_id())
                .actions([nana_ui::runtime::PaneChromeAction::new(
                    nana_ui::runtime::PaneChromeActionKind::CloseItem,
                    "关闭",
                )
                .target(close.stable_id())])
                .active(true),
        );
        ui.nest(chrome, |ui| {
            ui.adopt(header);
            ui.adopt(body);
        });
        chrome.stable_id()
    })?)
}

pub(super) fn mount_runtime_pane_tree(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    Ok(document.context_mut().build(document_id, |ui| {
        let left = ui.leaf(RuntimeText::new("left").style(slot_label_style()));
        let right = ui.leaf(RuntimeText::new("right").style(slot_label_style()));
        let tree = ui.child(
            "tree",
            RuntimePaneTree::new(RuntimePaneTreeNode::split(
                "root",
                SplitAxis::Horizontal,
                0.4,
                RuntimePaneTreeNode::leaf_content("left", left.stable_id()),
                RuntimePaneTreeNode::leaf_content("right", right.stable_id()),
            )),
        );
        ui.nest(tree, |ui| {
            ui.adopt(left);
            ui.adopt(right);
        });
        tree.stable_id()
    })?)
}

pub(super) fn mount_runtime_app_shell(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    Ok(document.context_mut().build(document_id, |ui| {
        let title = ui.leaf(RuntimeAppTitleBar::new("NanaUI"));
        let body = ui.leaf(RuntimeText::new("Workspace"));
        let shell = ui.child(
            "shell",
            RuntimeAppShell::new()
                .title_bar(title.stable_id())
                .body(body.stable_id()),
        );
        ui.nest(shell, |ui| {
            ui.adopt(title);
            ui.adopt(body);
        });
        shell.stable_id()
    })?)
}

pub(super) fn snapshot_settings_model() -> &'static SettingsModel {
    static MODEL: std::sync::OnceLock<SettingsModel> = std::sync::OnceLock::new();
    MODEL.get_or_init(|| {
        SettingsModel::new(
            "appearance",
            [
                SettingsTab::new("appearance", "外观").icon(Icon::Appearance),
                SettingsTab::new("about", "关于")
                    .icon(Icon::About)
                    .full_page(true),
            ],
        )
        .expect("snapshot settings model")
    })
}

pub(super) fn snapshot_settings_state() -> &'static SettingsState {
    static STATE: std::sync::OnceLock<SettingsState> = std::sync::OnceLock::new();
    STATE.get_or_init(|| SettingsState::new(snapshot_settings_model()))
}

pub(super) fn snapshot_settings_full_state() -> &'static SettingsState {
    static STATE: std::sync::OnceLock<SettingsState> = std::sync::OnceLock::new();
    STATE.get_or_init(|| {
        let model = snapshot_settings_model();
        let mut state = SettingsState::new(model);
        state.select(model, &SettingsTabId::from("about"));
        state
    })
}

pub(super) fn snapshot_desktop_workspace_layout() -> WorkspaceLayout {
    WorkspaceLayout::new([
        RegionState::new(RegionId::Resources, RegionRole::Resources)
            .size(220.0)
            .min_size(180.0)
            .max_size(480.0)
            .collapsible(true)
            .resizable(true),
        RegionState::new(RegionId::Primary, RegionRole::Primary)
            .min_size(160.0)
            .fill_priority(1),
    ])
    .expect("desktop-settings regions")
}

pub(super) fn mount_runtime_appearance_section(
    document: &mut RuntimeDocument,
    theme: ThemeMode,
) -> Result<nana_ui::runtime::Entity<RuntimeAppearanceSection>, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let section = document.context_mut().build_detached(document_id, |ui| {
        ui.leaf(RuntimeAppearanceSection::new(
            theme,
            AppearanceSettings::default(),
        ))
    })?;
    document
        .context_mut()
        .assemble_appearance_section(section)?;
    Ok(section)
}

pub(super) fn mount_runtime_about_section(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::Entity<RuntimeAboutSection>, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let section = document.context_mut().build_detached(document_id, |ui| {
        ui.leaf(RuntimeAboutSection::new(
            RuntimeAboutMetadata::new("NanaUI Gallery", "0.1.0")
                .description("Injected product metadata for the about card."),
        ))
    })?;
    document.context_mut().assemble_about_section(section)?;
    Ok(section)
}

pub(super) fn mount_runtime_settings_sidebar(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let sidebar = document.context_mut().build(document_id, |ui| {
        ui.child(
            "sidebar",
            RuntimeSettingsSidebar::new(
                snapshot_settings_model().clone(),
                snapshot_settings_state().clone(),
            ),
        )
    })?;
    document.context_mut().assemble_settings_sidebar(sidebar)?;
    Ok(sidebar.stable_id())
}

pub(super) fn mount_runtime_settings_page(
    document: &mut RuntimeDocument,
    theme: ThemeMode,
    fixture: Fixture,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let full_page = fixture.state == "settings-page-full";
    let content = if full_page {
        mount_runtime_about_section(document)?.stable_id()
    } else {
        mount_runtime_appearance_section(document, theme)?.stable_id()
    };
    let state = if full_page {
        snapshot_settings_full_state().clone()
    } else {
        snapshot_settings_state().clone()
    };
    let page = document.context_mut().build(document_id, |ui| {
        ui.child(
            "page",
            RuntimeSettingsPage::new(snapshot_settings_model().clone(), state).content(content),
        )
    })?;
    document.context_mut().assemble_settings_page(page)?;
    Ok(page.stable_id())
}

pub(super) fn mount_runtime_desktop_shell(
    document: &mut RuntimeDocument,
    theme: ThemeMode,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let model = snapshot_settings_model().clone();
    let state = snapshot_settings_state().clone();
    let sidebar = document.context_mut().build_detached(document_id, |ui| {
        ui.leaf(RuntimeSettingsSidebar::new(model.clone(), state.clone()))
    })?;
    document.context_mut().assemble_settings_sidebar(sidebar)?;
    let content = mount_runtime_appearance_section(document, theme)?;
    let page = document.context_mut().build_detached(document_id, |ui| {
        ui.leaf(RuntimeSettingsPage::new(model, state).content(content.stable_id()))
    })?;
    document.context_mut().assemble_settings_page(page)?;
    let shell = document.context_mut().build(document_id, |ui| {
        ui.child(
            "shell",
            RuntimeDesktopShell::from_model(WorkspaceModel::with_layout(
                snapshot_desktop_workspace_layout(),
            ))
            .title("NanaUI")
            .navigation(sidebar.stable_id())
            .primary(page.stable_id()),
        )
    })?;
    document.context_mut().assemble_desktop_shell(shell)?;
    Ok(shell.stable_id())
}

pub(super) fn mount_runtime_sidebar_frame(
    document: &mut RuntimeDocument,
    _fixture: Fixture,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let section = mount_runtime_sidebar_section(
        document,
        true,
        &["外观", "工作区", "设置", "关于", "日志", "调试"],
        false,
    )?;
    Ok(document.context_mut().build(document_id, |ui| {
        let top = ui.leaf(RuntimeSidebarRow::new("返回"));
        let body = ui.leaf(RuntimeSidebarFrame::vertical_body_scroll());
        ui.nest(body, |ui| {
            ui.adopt(Entity::<RuntimeSidebarSection>::from_stable_id(section));
        });
        let settings =
            ui.leaf(RuntimeSidebarFooterButton::new("设置", Icon::Settings).selected(true));
        let footer = ui.leaf(RuntimeSidebarFooter::new());
        ui.nest(footer, |ui| ui.adopt(settings));
        let frame = ui.child(
            "frame",
            RuntimeSidebarFrame::new()
                .top(top.stable_id())
                .body(body.stable_id())
                .footer(footer.stable_id()),
        );
        ui.nest(frame, |ui| {
            ui.adopt(top);
            ui.adopt(body);
            ui.adopt(footer);
        });
        frame.stable_id()
    })?)
}
