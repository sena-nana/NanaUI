//! Snapshot workspace; no product state is stored here.
use super::*;

pub(super) fn mount_runtime_sidebar_section(
    document: &mut RuntimeDocument,
    expanded: bool,
    labels: &[&str],
    collapsible: bool,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    mount_root(document, || sidebar_section(expanded, labels, collapsible)).map_err(Into::into)
}

fn sidebar_section(expanded: bool, labels: &[&str], collapsible: bool) -> impl IntoView + use<> {
    let spec = RuntimeSidebarSection::new("资源")
        .count(3)
        .collapsible(collapsible)
        .expanded(expanded);
    // The chrome is spelled out rather than left to the section's
    // assembler, which would build the same nodes after the section instead
    // of before it and renumber the recorded fixture.
    {
        let (disclosure, title, count) = (entity_ref(), entity_ref(), entity_ref());
        let mut header = widget(spec.header_item());
        if collapsible {
            header = header.leading(widget(spec.disclosure_mark()).entity_ref(disclosure));
        }
        let header = header
            .content(widget(spec.title_label()).entity_ref(title))
            .trailing(widget(spec.count_label()).entity_ref(count));
        let rows = labels
            .iter()
            .enumerate()
            .map(|(index, label)| {
                widget(RuntimeSidebarRow::new(*label)).key(format!("row-{index}"))
            })
            .collect::<Vec<_>>();
        let built = "the header is built before the section";
        widget(spec.clone())
            .child_slot(header, move |section, header| {
                let section = section
                    .title_slot(title.get().expect(built).stable_id())
                    .count_slot(count.get().expect(built).stable_id())
                    .header(header);
                match disclosure.get() {
                    Some(disclosure) => section.disclosure(disclosure.stable_id()),
                    None => section,
                }
            })
            .child_slot(
                widget(RuntimeSidebarSection::body_port()).children(rows),
                RuntimeSidebarSection::body,
            )
    }
}

pub(super) fn mount_runtime_workspace(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    Ok(mount_root(document, || {
        widget(RuntimeWorkspace::from_model(&WorkspaceModel::new(), []))
            .region(RegionId::GlobalNavigation, slot_label("Nav"))
            .region(RegionId::Resources, slot_label("Files"))
            .region(RegionId::PrimaryToolbar, slot_label("Toolbar"))
            .region(RegionId::Primary, slot_label("Primary"))
            .region(RegionId::Inspector, slot_label("Inspector"))
            .region(RegionId::Diagnostics, slot_label("Diagnostics"))
    })?)
}

fn slot_label(value: &str) -> nana_ui::runtime::view::El<RuntimeText> {
    widget(RuntimeText::new(value).style(slot_label_style()))
}

/// `dock` with `content` as the content of item `id`.
fn bind_dock_content(mut dock: RuntimeDock, id: &str, content: StableNodeId) -> RuntimeDock {
    dock.root.bind_content(id, content);
    dock
}

pub(super) fn mount_runtime_dock(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    Ok(mount_root(document, || {
        widget(
            RuntimeDock::new(RuntimeDockNode::split(
                nana_ui::runtime::DockAxis::Horizontal,
                0.35,
                RuntimeDockNode::tabs(["nav", "files"], "nav", [("nav", None), ("files", None)]),
                RuntimeDockNode::item("primary", None),
            ))
            .title("nav", "Nav")
            .title("files", "Files")
            .title("primary", "Primary"),
        )
        .child_slot(slot_label("Nav"), |dock, id| {
            bind_dock_content(dock, "nav", id)
        })
        .child_slot(slot_label("Files"), |dock, id| {
            bind_dock_content(dock, "files", id)
        })
        .child_slot(slot_label("Primary"), |dock, id| {
            bind_dock_content(dock, "primary", id)
        })
    })?)
}

pub(super) fn mount_runtime_split_pane(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let (_, (first, second, handle)) =
        document
            .context_mut()
            .mount_view_detached(document_id, || {
                let (first, second, handle) = (entity_ref(), entity_ref(), entity_ref());
                with_refs(
                    (
                        slot_label("First").entity_ref(first),
                        slot_label("Second").entity_ref(second),
                        widget(RuntimeText::new(""))
                            .entity_ref(handle)
                            .children(widget(RuntimeText::new(""))),
                    ),
                    (first, second, handle),
                )
            })?;
    // Not a view: a view-built `SplitPane` assembles itself into slot
    // shells, and this fixture shows the pane with its children direct.
    let context = document.context_mut();
    let pane = context.create_component(
        document_id,
        RuntimeSplitPane::from_model(
            &SplitPaneModel::new(SplitAxis::Horizontal, 160.0, 80.0, 280.0),
            first.stable_id(),
            second.stable_id(),
        )
        .handle(handle.stable_id()),
    )?;
    context.append_child(pane, first)?;
    context.append_child(pane, handle)?;
    context.append_child(pane, second)?;
    context.reproject_component(pane)?;
    Ok(pane.stable_id())
}

pub(super) fn mount_runtime_pane_chrome(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    Ok(mount_root(document, || {
        widget(RuntimePaneChrome::new().active(true))
            .tabs(widget(RuntimeText::new("editor.rs")))
            .action(
                nana_ui::runtime::PaneChromeAction::new(
                    nana_ui::runtime::PaneChromeActionKind::CloseItem,
                    "关闭",
                ),
                widget(RuntimeText::new("关闭")),
            )
            .body(widget(RuntimeText::new("Body")))
    })?)
}

/// A pane leaf that can be seen.
///
/// `PaneTree` paints nothing of its own — no chrome, no divider — so a leaf
/// with no surface leaves the split geometry entirely unobservable. Filling the
/// pane and outlining it is what turns this fixture into evidence: the boxes in
/// the baseline *are* the tree.
fn pane_leaf_style() -> NodeStyle {
    let mut style = slot_label_style();
    style.background = Some(SemanticColorRole::Surface);
    style.border = Some(SemanticColorRole::Border);
    style.radius = Some(nana_ui_core::RadiusTier::Sm);
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    layout.border_width = Some(nana_ui_core::HAIRLINE);
    style
}

pub(super) fn mount_runtime_pane_tree(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    // The state is called "nested", so it nests: a 0.4 horizontal split
    // whose right half is itself split down the middle. Both halves of
    // that sentence used to be unobservable — `PaneTree` flattened the
    // tree into one flex line, its ratio never applied, and leaves with no
    // surface of their own drew nothing but two words.
    Ok(mount_root(document, || {
        let leaf = |value: &str| widget(RuntimeText::new(value).style(pane_leaf_style()));
        widget(RuntimePaneTree::new(RuntimePaneTreeNode::split(
            "root",
            SplitAxis::Horizontal,
            0.4,
            RuntimePaneTreeNode::leaf("left"),
            RuntimePaneTreeNode::split(
                "right",
                SplitAxis::Vertical,
                0.5,
                RuntimePaneTreeNode::leaf("right-top"),
                RuntimePaneTreeNode::leaf("right-bottom"),
            ),
        )))
        .child_slot(leaf("left"), |tree, id| bind_pane(tree, "left", id))
        .child_slot(leaf("right top"), |tree, id| {
            bind_pane(tree, "right-top", id)
        })
        .child_slot(leaf("right bottom"), |tree, id| {
            bind_pane(tree, "right-bottom", id)
        })
    })?)
}

/// `tree` with `content` in leaf `pane`.
fn bind_pane(mut tree: RuntimePaneTree, pane: &str, content: StableNodeId) -> RuntimePaneTree {
    fn bind(node: &mut RuntimePaneTreeNode, pane: &str, content: StableNodeId) {
        match node {
            RuntimePaneTreeNode::Leaf {
                pane_id,
                content: slot,
            } if pane_id.as_ref() == pane => *slot = Some(content),
            RuntimePaneTreeNode::Leaf { .. } => {}
            RuntimePaneTreeNode::Split { first, second, .. } => {
                bind(first, pane, content);
                bind(second, pane, content);
            }
        }
    }
    bind(&mut tree.root, pane, content);
    tree
}

/// A title bar that looks the same on every OS.
///
/// `AppTitleBar::new` takes its `native_controls` default from
/// `WindowChrome::platform_default()`, which is a `cfg(target_os)` decision:
/// on macOS the bar reserves 78px for the traffic lights and drops the three
/// custom window buttons entirely. The semantic baseline is committed once and
/// checked on ubuntu, macos and windows alike — it is the *adapter*-independent
/// half of the suite, and a fixture that moves 86px between platforms makes it
/// neither. Pinning the mode is what makes the recorded numbers mean something
/// on all three.
pub(super) fn snapshot_title_bar(title: &str) -> RuntimeAppTitleBar {
    RuntimeAppTitleBar::new(title).native_controls(false)
}

pub(super) fn mount_runtime_app_shell(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    Ok(mount_root(document, || {
        widget(RuntimeAppShell::new())
            .title_bar(widget(snapshot_title_bar("NanaUI")))
            .body(widget(RuntimeText::new("Workspace")))
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
    theme: ThemeAppearance,
) -> Result<nana_ui::runtime::Entity<RuntimeAppearanceSection>, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let (_, section) = document
        .context_mut()
        .mount_view_detached(document_id, || {
            let section = entity_ref();
            with_refs(
                widget(RuntimeAppearanceSection::new(
                    theme,
                    AppearanceSettings::default(),
                ))
                .entity_ref(section),
                section,
            )
        })?;
    Ok(section)
}

pub(super) fn mount_runtime_about_section(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::Entity<RuntimeAboutSection>, Box<dyn std::error::Error>> {
    let document_id = document.document();
    let (_, section) = document
        .context_mut()
        .mount_view_detached(document_id, || {
            let section = entity_ref();
            with_refs(
                widget(RuntimeAboutSection::new(
                    RuntimeAboutMetadata::new("NanaUI Gallery", "0.1.0")
                        .description("Injected product metadata for the about card."),
                ))
                .entity_ref(section),
                section,
            )
        })?;
    Ok(section)
}

pub(super) fn mount_runtime_settings_sidebar(
    document: &mut RuntimeDocument,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    Ok(mount_root(document, || {
        widget(RuntimeSettingsSidebar::new(
            snapshot_settings_model().clone(),
            snapshot_settings_state().clone(),
        ))
    })?)
}

pub(super) fn mount_runtime_settings_page(
    document: &mut RuntimeDocument,
    theme: ThemeAppearance,
    fixture: Fixture,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
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
    Ok(mount_root(document, || {
        widget(RuntimeSettingsPage::new(snapshot_settings_model().clone(), state).content(content))
    })?)
}

pub(super) fn mount_runtime_desktop_shell(
    document: &mut RuntimeDocument,
    theme: ThemeAppearance,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    let model = snapshot_settings_model().clone();
    let state = snapshot_settings_state().clone();
    let content = mount_runtime_appearance_section(document, theme)?;
    Ok(mount_root(document, || {
        // Supply the bar instead of letting `DesktopShell` mint one from
        // `.title(..)`: the minted one takes its control mode from
        // `WindowChrome::platform_default()`, which drops three window buttons
        // and shifts the title 86px on macOS. See [`snapshot_title_bar`].
        widget(RuntimeDesktopShell::from_model(
            WorkspaceModel::with_layout(snapshot_desktop_workspace_layout()),
        ))
        .navigation(widget(RuntimeSettingsSidebar::new(
            model.clone(),
            state.clone(),
        )))
        .primary(widget(
            RuntimeSettingsPage::new(model, state).content(content.stable_id()),
        ))
        .child_slot(
            widget(snapshot_title_bar("NanaUI")),
            RuntimeDesktopShell::title_bar,
        )
    })?)
}

pub(super) fn mount_runtime_sidebar_frame(
    document: &mut RuntimeDocument,
    _fixture: Fixture,
) -> Result<nana_ui::runtime::StableNodeId, Box<dyn std::error::Error>> {
    Ok(mount_root(document, || {
        let section = sidebar_section(
            true,
            &["外观", "工作区", "设置", "关于", "日志", "调试"],
            false,
        );
        widget(RuntimeSidebarFrame::new())
            .top(widget(RuntimeSidebarRow::new("返回")))
            .child_slot(
                widget(RuntimeSidebarFrame::vertical_body_scroll()).children(section),
                RuntimeSidebarFrame::body,
            )
            .footer(widget(RuntimeSidebarFooter::new()).children(widget(
                RuntimeSidebarFooterButton::new("设置", Icon::Settings).selected(true),
            )))
    })?)
}
