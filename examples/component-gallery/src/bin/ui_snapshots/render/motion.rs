//! Deterministic frames through the product Runtime / Scene painter.
use super::*;
use nana_ui::runtime::{
    ActionMenuItem, AnchoredActionMenu, Dialog, OverlayHost, SidebarSection, Skeleton, Stack,
};
use std::time::Duration;

pub(super) fn generate(
    snapshots: &mut OffscreenSnapshots,
    output: &Path,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut paths = Vec::new();
    for theme in [ThemeMode::Dark, ThemeMode::Light] {
        let doc = DocumentId::new(1).unwrap();
        let mut document = RuntimeDocument::new(doc);
        let cx = document.context_mut();
        cx.set_theme(theme)?;
        let canvas = cx.create_component(doc, Stack::fill_column(0.0))?;
        let root = cx.create_component(
            doc,
            Stack::column(12.0)
                .width(LengthSpec::Px(260.0))
                .padding(20.0),
        )?;
        cx.append_child(canvas, root)?;
        let button = cx.create_component(doc, RuntimeButton::new("Open workspace"))?;
        let switch = cx.create_component(doc, RuntimeSwitch::new("Notifications", false))?;
        let skeleton = cx.create_component(doc, Skeleton::new(LengthSpec::Px(200.0), 18.0))?;
        let spec = SidebarSection::new("Projects").collapsible(true);
        let disclosure = cx.create_component(doc, spec.disclosure_mark())?;
        let title = cx.create_component(doc, spec.title_label())?;
        let spec = spec
            .disclosure(disclosure.stable_id())
            .title_slot(title.stable_id());
        let header = cx.create_component(doc, spec.header_item())?;
        cx.append_child(header, disclosure)?;
        cx.append_child(header, title)?;
        let body = cx.create_component(doc, SidebarSection::body_port())?;
        for label in ["Workspace", "Recent files", "Archive"] {
            let row = cx.create_component(doc, nana_ui::runtime::SidebarRow::new(label))?;
            cx.append_child(body, row)?;
        }
        let section =
            cx.create_component(doc, spec.header(header.stable_id()).body(body.stable_id()))?;
        cx.append_child(section, header)?;
        cx.append_child(section, body)?;
        cx.append_child(root, button)?;
        cx.append_child(root, switch)?;
        cx.append_child(root, skeleton)?;
        cx.append_child(root, section)?;
        let menu = cx.create_component(doc, AnchoredActionMenu::new(310.0, 30.0).open(false))?;
        cx.append_child(canvas, menu)?;
        for label in ["New file", "Open folder", "Settings"] {
            let item = cx.create_component(doc, ActionMenuItem::new(label))?;
            cx.append_child(menu, item)?;
        }
        let mut overlay = OverlayHost::new();
        let layout = Arc::make_mut(&mut overlay.style.layout);
        layout.position = nana_ui_core::PositionSpec::Fixed;
        layout.offset_left = Some(LengthSpec::Px(0.0));
        layout.offset_top = Some(LengthSpec::Px(0.0));
        layout.width = Some(LengthSpec::Percent(100.0));
        layout.height = Some(LengthSpec::Percent(100.0));
        let host = cx.create_component(doc, overlay)?;
        cx.append_child(canvas, host)?;
        let dialog = cx.create_component(
            doc,
            Dialog::new("Workspace settings").description("Changes apply to this workspace."),
        )?;
        cx.append_child(host, dialog)?;
        let size = Size::new(600, 420);
        let viewport = LayoutViewport::new(size.width as f32, size.height as f32);
        let mut shaper = NanaTextShaper::default();
        document.flush(viewport, &mut shaper)?;
        let theme_name = if theme == ThemeMode::Dark {
            "dark"
        } else {
            "light"
        };
        for ms in [0, 60, 140, 180, 260, 320, 400, 440, 520, 590, 660, 730, 800] {
            let cx = document.context_mut();
            cx.advance_animations(Duration::from_millis(ms));
            if ms == 0 {
                cx.update_component(menu, |menu, _| menu.open = true)?;
                cx.update_component(switch, |switch, _| switch.checked = true)?;
                cx.set_pointer_hover_at(doc, 1, Some(button.stable_id()), Duration::ZERO)?;
                cx.activate_sidebar_section(section)?;
            } else if ms == 260 {
                cx.update_component(menu, |menu, _| menu.open = false)?;
                cx.update_component(switch, |switch, _| switch.checked = false)?;
                cx.set_pointer_hover_at(doc, 1, None, Duration::from_millis(ms))?;
                cx.activate_sidebar_section(section)?;
            } else if ms == 520 {
                cx.activate_overlay(host, dialog)?;
            } else if ms == 660 {
                cx.dismiss_overlay(host)?;
            }
            document.flush(viewport, &mut shaper)?;
            paths.push(offscreen::write_scene(
                snapshots,
                output,
                &format!("motion-{theme_name}-{ms:04}.png"),
                document.scene(),
                size,
                clear_color(theme),
                None,
                None,
            )?);
        }
    }
    Ok(paths)
}
