//! Reproducible Rust/Vue spacing acceptance: normal/narrow, nested cards, scroll end.
use nana_js_engine::{HostValue, JsEngine, RuntimeArtifact};
use nana_js_v8::V8Engine;
use nana_ui::runtime::{
    Card, DocumentId, Entity, LengthSpec, RuntimeDocument, ScrollView, SettingsPage, Stack, Text,
};
use nana_ui_core::{SettingsModel, SettingsState, SettingsTab};
use nana_ui_devtools::agent::{RuntimeAgentSession, VueAgentSession};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::path::Path::new("target/spacing-layout");
    std::fs::create_dir_all(output)?;
    for width in [640, 280] {
        let id = DocumentId::new(61).unwrap();
        let mut document = RuntimeDocument::new(id);
        let context = document.context_mut();
        context.set_theme(nana_ui::ThemeMode::Light)?;
        let model =
            SettingsModel::new("appearance", [SettingsTab::new("appearance", "Appearance")])?
                .hide_header(true);
        let state = SettingsState::new(&model);
        let content = context.create_detached_component(id, Stack::column(16.0))?;
        let mut cards = Vec::new();
        for i in 0..5 {
            let mut card = Card::new().height(80.0);
            std::sync::Arc::make_mut(&mut card.style.layout).width = Some(LengthSpec::Fill);
            let card = context.create_detached_component(id, card)?;
            let body = context.create_detached_component(id, Stack::column(8.0))?;
            let title =
                context.create_detached_component(id, Text::new(format!("Section {}", i + 1)))?;
            context.append_child(content, card)?;
            context.append_child(card, body)?;
            context.append_child(body, title)?;
            cards.push((card.stable_id(), title.stable_id()));
        }
        let page = context.create_component(
            id,
            SettingsPage::new(model, state).content(content.stable_id()),
        )?;
        context.assemble_settings_page(page)?;
        let assembly = context.read(page, |page| page.assembly.clone().unwrap())?;
        let scroll = Entity::<ScrollView>::from_stable_id(assembly.scroll.unwrap());
        let mut runtime = RuntimeAgentSession::new(document, width, 300)?;
        let world = runtime.document().context().world();
        let first = world.layout_box(cards[0].0).unwrap();
        assert_eq!(first.x, 24.0);
        assert_eq!(first.y, 20.0);
        assert_eq!(first.width, width as f32 - 48.0);
        let text = world.layout_box(cards[0].1).unwrap();
        assert_eq!(text.x - first.x, 16.0);
        assert_eq!(text.y - first.y, 14.0);
        runtime.screenshot_png(output.join(format!("rust-{width}-top.png")))?;
        let metrics = runtime
            .document()
            .context()
            .world()
            .scroll_metrics(scroll.stable_id())
            .unwrap();
        assert!(metrics.max_offset().y > 0.0);
        runtime
            .document_mut()
            .context_mut()
            .scroll_to(scroll, metrics.max_offset())?;
        runtime.flush()?;
        let world = runtime.document().context().world();
        let last = world.layout_box(cards[4].0).unwrap();
        let offset = world.scroll_offset(scroll.stable_id()).unwrap();
        assert!((300.0 - (last.y + last.height - offset.y) - 24.0).abs() < 0.1);
        runtime.screenshot_png(output.join(format!("rust-{width}-bottom.png")))?;
        std::fs::write(
            output.join(format!("rust-{width}-a11y.json")),
            serde_json::to_vec_pretty(&runtime.accessibility_dump())?,
        )?;

        let source = match std::env::args().nth(1) {
            Some(path) => std::fs::read_to_string(path)?,
            None => include_str!("spacing-layout.js").to_owned(),
        };
        let artifact = RuntimeArtifact::from_source("spacing-layout.js", &source);
        let mut vue = VueAgentSession::new(V8Engine::new(), artifact, width, 300)?;
        let page_id = vue
            .semantic_dump()
            .into_iter()
            .find(|node| node.agent_id == "page")
            .unwrap()
            .id;
        let document = vue.host().document();
        let ids = vue
            .semantic_dump()
            .into_iter()
            .filter(|node| node.agent_id.starts_with("card-"))
            .map(|node| nana_ui::runtime::StableNodeId::new(node.id).unwrap())
            .collect::<Vec<_>>();
        let (vue_assembly, vue_cards) = {
            let doc = document.lock().unwrap();
            let page = Entity::<SettingsPage>::from_stable_id(
                nana_ui::runtime::StableNodeId::new(page_id).unwrap(),
            );
            let assembly = doc.context().read(page, |p| p.assembly.clone().unwrap())?;
            (assembly, ids)
        };
        std::fs::write(
            output.join(format!("vue-{width}-semantic.json")),
            serde_json::to_vec_pretty(&vue.semantic_dump())?,
        )?;
        std::fs::write(
            output.join(format!("vue-{width}-top-a11y.json")),
            serde_json::to_vec_pretty(&vue.accessibility_dump())?,
        )?;
        {
            let doc = document.lock().unwrap();
            let first_vue = doc.context().world().layout_box(vue_cards[0]).unwrap();
            assert_eq!(first_vue, first);
            let title = doc.context().world().node(vue_cards[0]).unwrap().children[0];
            let title = doc.context().world().node(title).unwrap().children[0];
            let title = doc.context().world().layout_box(title).unwrap();
            assert_eq!(title.x - first_vue.x, 16.0);
            assert_eq!(title.y - first_vue.y, 14.0);
        }
        vue.screenshot_png(output.join(format!("vue-{width}-top.png")))?;
        let function = vue.engine_mut().resolve_function("spacingScroll")?;
        vue.engine_mut().invoke(
            function,
            &[HostValue::Number(vue_assembly.scroll.unwrap().get() as f64)],
        )?;
        vue.pump()?;
        {
            let doc = document.lock().unwrap();
            let world = doc.context().world();
            let last = world.layout_box(*vue_cards.last().unwrap()).unwrap();
            let offset = world.scroll_offset(vue_assembly.scroll.unwrap()).unwrap();
            assert!((300.0 - (last.y + last.height - offset.y) - 24.0).abs() < 0.1);
        }
        vue.screenshot_png(output.join(format!("vue-{width}-bottom.png")))?;
        std::fs::write(
            output.join(format!("vue-{width}-a11y.json")),
            serde_json::to_vec_pretty(&vue.accessibility_dump())?,
        )?;
        let function = vue.engine_mut().resolve_function("spacingPadding")?;
        vue.engine_mut()
            .invoke(function, &[HostValue::Number(0.0)])?;
        vue.pump()?;
        {
            let doc = document.lock().unwrap();
            assert!(
                doc.context()
                    .world()
                    .node_style(vue_assembly.body.unwrap())
                    .unwrap()
                    .layout
                    .resolved_padding()
                    .is_zero()
            );
        }
        println!(
            "spacing acceptance {width}x300: Rust/Vue geometry, padding update and scroll end passed"
        );
    }
    Ok(())
}
