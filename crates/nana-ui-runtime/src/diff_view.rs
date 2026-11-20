//! Structured diff review. The application owns the buffer and feeds hunks.

use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ButtonKind, ControlSize, FlexDirection, LengthSpec, OverflowSpec, PositionSpec,
    SemanticColorRole,
};

use crate::view_components::{Activate, Button, Stack, Text, project_common};
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, Entity, FrameworkError,
    InteractionState, MutationQueue, NodeKind, NodeStyle, ScrollAxes, ScrollView, StableNodeId,
    UiWorld,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffLineKind {
    #[default]
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub old_number: Option<u32>,
    pub new_number: Option<u32>,
    pub kind: DiffLineKind,
    pub text: Arc<str>,
}

impl DiffLine {
    pub fn context(old_number: u32, new_number: u32, text: impl Into<Arc<str>>) -> Self {
        Self {
            old_number: Some(old_number),
            new_number: Some(new_number),
            kind: DiffLineKind::Context,
            text: text.into(),
        }
    }

    pub fn added(new_number: u32, text: impl Into<Arc<str>>) -> Self {
        Self {
            old_number: None,
            new_number: Some(new_number),
            kind: DiffLineKind::Added,
            text: text.into(),
        }
    }

    pub fn removed(old_number: u32, text: impl Into<Arc<str>>) -> Self {
        Self {
            old_number: Some(old_number),
            new_number: None,
            kind: DiffLineKind::Removed,
            text: text.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffHunk {
    pub header: Arc<str>,
    pub lines: Arc<[DiffLine]>,
}

impl DiffHunk {
    pub fn new(header: impl Into<Arc<str>>, lines: impl Into<Arc<[DiffLine]>>) -> Self {
        Self {
            header: header.into(),
            lines: lines.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffLayout {
    #[default]
    Unified,
    Split,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffEvent {
    HunkAccepted { hunk: usize },
    HunkRejected { hunk: usize },
    LineAccepted { hunk: usize, line: usize },
    LineRejected { hunk: usize, line: usize },
    LayoutChanged { layout: DiffLayout },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiffView {
    pub hunks: Arc<[DiffHunk]>,
    pub layout: DiffLayout,
    pub disabled: bool,
    /// Whether hunks and lines carry accept / reject actions. A read-only
    /// review (a commit, the working tree) leaves them out entirely.
    pub review_actions: bool,
    pub style: NodeStyle,
    wired: Vec<StableNodeId>,
}

impl DiffView {
    pub fn new(hunks: impl Into<Arc<[DiffHunk]>>) -> Self {
        let mut style = NodeStyle::default().surface(SemanticColorRole::Surface);
        let layout = Arc::make_mut(&mut style.layout);
        layout.position = PositionSpec::Relative;
        layout.direction = Some(FlexDirection::Column);
        layout.gap = Some(LengthSpec::Px(8.0));
        layout.width = Some(LengthSpec::Fill);
        layout.height = Some(LengthSpec::Fill);
        layout.min_width = Some(LengthSpec::Px(0.0));
        layout.min_height = Some(LengthSpec::Px(0.0));
        layout.overflow_x = OverflowSpec::Hidden;
        layout.overflow_y = OverflowSpec::Hidden;
        layout.text_ext.font_family = Some("Cascadia Mono".to_owned());
        crate::theme_intent::font(layout, nana_ui_core::TypeRole::Meta);
        Self {
            hunks: hunks.into(),
            layout: DiffLayout::Unified,
            disabled: false,
            review_actions: true,
            style,
            wired: Vec::new(),
        }
    }

    pub fn layout(mut self, layout: DiffLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Show or leave out the accept / reject actions.
    pub fn review_actions(mut self, review_actions: bool) -> Self {
        self.review_actions = review_actions;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
}

impl ComponentView for DiffView {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::DIFF.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        assembler: Some(crate::AppContext::assemble_diff_view),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element { tag: "diff".into() }
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(Arc::clone(&world.framework_strings().diff_title)),
                disabled: self.disabled,
                ..Default::default()
            },
        );
    }
}

impl AppContext {
    pub fn assemble_diff_view(&mut self, diff: Entity<DiffView>) -> Result<bool, FrameworkError> {
        let snapshot = self.read(diff, Clone::clone)?;
        let strings = self.world().framework_strings().clone();
        self.mount(diff, |ui| {
            let unified = snapshot.layout == DiffLayout::Unified;
            let mut toolbar = NodeStyle::default();
            {
                let layout = Arc::make_mut(&mut toolbar.layout);
                layout.direction = Some(FlexDirection::Row);
                layout.align_items = AlignSpec::Center;
                layout.gap = Some(LengthSpec::Px(8.0));
            }
            ui.with_child(
                "toolbar",
                Stack::from_layout(toolbar.layout.as_ref().clone()),
                |ui| {
                    ui.child(
                        "layout-unified",
                        Button::new(strings.diff_unified.as_ref())
                            .kind(if unified {
                                ButtonKind::Selected
                            } else {
                                ButtonKind::Subtle
                            })
                            .size(ControlSize::Small)
                            .disabled(snapshot.disabled),
                    )?;
                    ui.child(
                        "layout-split",
                        Button::new(strings.diff_split.as_ref())
                            .kind(if unified {
                                ButtonKind::Subtle
                            } else {
                                ButtonKind::Selected
                            })
                            .size(ControlSize::Small)
                            .disabled(snapshot.disabled),
                    )?;
                    Ok(())
                },
            )?;

            let mut scroll_style = NodeStyle::default();
            {
                let layout = Arc::make_mut(&mut scroll_style.layout);
                layout.flex_grow = Some(1.0);
                layout.min_height = Some(LengthSpec::Px(0.0));
                layout.width = Some(LengthSpec::Fill);
            }
            ui.with_child(
                "body",
                ScrollView::new(ScrollAxes::Vertical).style(scroll_style),
                |ui| {
                    for (hunk_index, hunk) in snapshot.hunks.iter().enumerate() {
                        mount_hunk(
                            ui,
                            &strings,
                            hunk_index,
                            hunk,
                            snapshot.layout,
                            snapshot.disabled,
                            snapshot.review_actions,
                        )?;
                    }
                    Ok(())
                },
            )?;
            Ok(())
        })?;
        self.wire_diff_actions(diff)
    }

    fn wire_diff_actions(&mut self, diff: Entity<DiffView>) -> Result<bool, FrameworkError> {
        let root = diff.stable_id();
        let strings = self.world().framework_strings().clone();
        if let Some(unified) = find_labeled_button(self, root, &strings.diff_unified) {
            self.observe_diff_once(unified, diff, |view, _: &Activate, cx| {
                if view.disabled || view.layout == DiffLayout::Unified {
                    return;
                }
                view.layout = DiffLayout::Unified;
                cx.emit(DiffEvent::LayoutChanged {
                    layout: DiffLayout::Unified,
                });
            })?;
        }
        if let Some(split) = find_labeled_button(self, root, &strings.diff_split) {
            self.observe_diff_once(split, diff, |view, _: &Activate, cx| {
                if view.disabled || view.layout == DiffLayout::Split {
                    return;
                }
                view.layout = DiffLayout::Split;
                cx.emit(DiffEvent::LayoutChanged {
                    layout: DiffLayout::Split,
                });
            })?;
        }
        let hunks = self.read(diff, |view| view.hunks.len())?;
        for hunk in 0..hunks {
            if let Some(accept) =
                find_labeled_button(self, root, &hunk_label(&strings.diff_accept_hunk, hunk))
            {
                self.observe_diff_once(accept, diff, move |view, _: &Activate, cx| {
                    if !view.disabled {
                        cx.emit(DiffEvent::HunkAccepted { hunk });
                    }
                })?;
            }
            if let Some(reject) =
                find_labeled_button(self, root, &hunk_label(&strings.diff_reject_hunk, hunk))
            {
                self.observe_diff_once(reject, diff, move |view, _: &Activate, cx| {
                    if !view.disabled {
                        cx.emit(DiffEvent::HunkRejected { hunk });
                    }
                })?;
            }
            let lines = self.read(diff, |view| {
                view.hunks
                    .get(hunk)
                    .map(|hunk| hunk.lines.len())
                    .unwrap_or(0)
            })?;
            for line in 0..lines {
                if let Some(accept) = find_labeled_button(
                    self,
                    root,
                    &line_label(&strings.diff_accept_line, hunk, line),
                ) {
                    self.observe_diff_once(accept, diff, move |view, _: &Activate, cx| {
                        if !view.disabled {
                            cx.emit(DiffEvent::LineAccepted { hunk, line });
                        }
                    })?;
                }
                if let Some(reject) = find_labeled_button(
                    self,
                    root,
                    &line_label(&strings.diff_reject_line, hunk, line),
                ) {
                    self.observe_diff_once(reject, diff, move |view, _: &Activate, cx| {
                        if !view.disabled {
                            cx.emit(DiffEvent::LineRejected { hunk, line });
                        }
                    })?;
                }
            }
        }
        Ok(true)
    }

    fn observe_diff_once(
        &mut self,
        source: StableNodeId,
        diff: Entity<DiffView>,
        handler: impl FnMut(&mut DiffView, &Activate, &mut crate::ViewContext<'_, DiffView>)
        + Send
        + 'static,
    ) -> Result<(), FrameworkError> {
        let already = self.read(diff, |view| view.wired.contains(&source))?;
        if already {
            return Ok(());
        }
        self.observe(Entity::<Button>::from_stable_id(source), diff, handler)?;
        self.update_component(diff, |view, _| {
            view.wired.push(source);
        })?;
        Ok(())
    }
}

fn find_labeled_button(
    context: &AppContext,
    root: StableNodeId,
    label: &str,
) -> Option<StableNodeId> {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(node) = context.world().node(id) else {
            continue;
        };
        stack.extend(node.children.iter().rev().copied());
        if context.world().text(id) == Some(label)
            || context
                .world()
                .accessibility(id)
                .is_some_and(|state| state.label.as_deref() == Some(label))
        {
            return Some(id);
        }
    }
    None
}

/// A hunk action's label, from its framework template.
fn hunk_label(template: &str, hunk: usize) -> String {
    nana_ui_core::framework_strings::fill(template, &[("hunk", &hunk)])
}

/// A line action's label, from its framework template.
fn line_label(template: &str, hunk: usize, line: usize) -> String {
    nana_ui_core::framework_strings::fill(template, &[("hunk", &hunk), ("line", &line)])
}

fn mount_hunk(
    ui: &mut crate::framework::AssemblyScope<'_>,
    strings: &nana_ui_core::FrameworkStrings,
    hunk_index: usize,
    hunk: &DiffHunk,
    layout: DiffLayout,
    disabled: bool,
    actions: bool,
) -> Result<(), FrameworkError> {
    let mut hunk_style = NodeStyle::default().surface(SemanticColorRole::Subtle);
    {
        let style = Arc::make_mut(&mut hunk_style.layout);
        style.direction = Some(FlexDirection::Column);
        style.gap = Some(LengthSpec::Px(nana_ui_core::space::XXS));
        style.theme.gap = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Xxs));
        style.padding = Some(LengthSpec::Px(nana_ui_core::space::MD));
        style.theme.padding = Some(crate::theme_intent::space(nana_ui_core::SpacingStep::Md));
        style.border_radius = Some(nana_ui_core::UI_METRICS.radius_sm);
        style.theme.border_radius =
            Some(nana_ui_core::ThemeToken::Radius(nana_ui_core::RadiusTier::Sm).into());
    }
    ui.with_child(
        format!("hunk-{hunk_index}"),
        Stack::from_layout(hunk_style.layout.as_ref().clone()),
        |ui| {
            if actions {
                ui.child(
                    "accept",
                    hunk_action(
                        Button::new(hunk_label(&strings.diff_accept_hunk, hunk_index))
                            .kind(ButtonKind::Subtle)
                            .size(ControlSize::Small)
                            .disabled(disabled),
                    ),
                )?;
                ui.child(
                    "reject",
                    hunk_action(
                        Button::new(hunk_label(&strings.diff_reject_hunk, hunk_index))
                            .kind(ButtonKind::Subtle)
                            .size(ControlSize::Small)
                            .disabled(disabled),
                    ),
                )?;
            }
            ui.child(
                "header",
                Text::new(hunk.header.as_ref()).style(muted_text()),
            )?;
            match layout {
                DiffLayout::Unified => {
                    for (line_index, line) in hunk.lines.iter().enumerate() {
                        mount_line(ui, strings, hunk_index, line_index, line, disabled, actions)?;
                    }
                }
                DiffLayout::Split => {
                    let mut row = NodeStyle::default();
                    {
                        let style = Arc::make_mut(&mut row.layout);
                        style.direction = Some(FlexDirection::Row);
                        style.gap = Some(LengthSpec::Px(8.0));
                    }
                    ui.with_child(
                        "split",
                        Stack::from_layout(row.layout.as_ref().clone()),
                        |ui| {
                            mount_split_column(
                                ui, strings, hunk_index, hunk, disabled, actions, true,
                            )?;
                            mount_split_column(
                                ui, strings, hunk_index, hunk, disabled, actions, false,
                            )?;
                            Ok(())
                        },
                    )?;
                }
            }
            Ok(())
        },
    )?;
    Ok(())
}

fn mount_split_column(
    ui: &mut crate::framework::AssemblyScope<'_>,
    strings: &nana_ui_core::FrameworkStrings,
    hunk_index: usize,
    hunk: &DiffHunk,
    disabled: bool,
    review_actions: bool,
    old_side: bool,
) -> Result<(), FrameworkError> {
    let mut column = NodeStyle::default();
    {
        let style = Arc::make_mut(&mut column.layout);
        style.direction = Some(FlexDirection::Column);
        style.flex_grow = Some(1.0);
        style.min_width = Some(LengthSpec::Px(0.0));
    }
    ui.with_child(
        if old_side { "old" } else { "new" },
        Stack::from_layout(column.layout.as_ref().clone()),
        |ui| {
            for (line_index, line) in hunk.lines.iter().enumerate() {
                let show = match line.kind {
                    DiffLineKind::Context => true,
                    DiffLineKind::Removed => old_side,
                    DiffLineKind::Added => !old_side,
                };
                if show {
                    let actions = review_actions
                        && match line.kind {
                            DiffLineKind::Context => false,
                            DiffLineKind::Removed => old_side,
                            DiffLineKind::Added => !old_side,
                        };
                    mount_line(ui, strings, hunk_index, line_index, line, disabled, actions)?;
                } else if matches!(line.kind, DiffLineKind::Added | DiffLineKind::Removed) {
                    ui.child(
                        format!("pad-{line_index}"),
                        Text::new(" ").style(line_style(line.kind)),
                    )?;
                }
            }
            Ok(())
        },
    )?;
    Ok(())
}

fn mount_line(
    ui: &mut crate::framework::AssemblyScope<'_>,
    strings: &nana_ui_core::FrameworkStrings,
    hunk_index: usize,
    line_index: usize,
    line: &DiffLine,
    disabled: bool,
    show_actions: bool,
) -> Result<(), FrameworkError> {
    let mut row = line_style(line.kind);
    {
        let style = Arc::make_mut(&mut row.layout);
        style.direction = Some(FlexDirection::Row);
        style.align_items = AlignSpec::Center;
        style.gap = Some(LengthSpec::Px(8.0));
        style.min_height = Some(LengthSpec::Px(20.0));
    }
    ui.with_child(
        format!("line-{line_index}"),
        Stack::column(0.0).style(row),
        |ui| {
            let old = line.old_number.map(|n| n.to_string()).unwrap_or_default();
            let new = line.new_number.map(|n| n.to_string()).unwrap_or_default();
            ui.child("old-no", Text::new(old).style(muted_text()))?;
            ui.child("new-no", Text::new(new).style(muted_text()))?;
            let marker = match line.kind {
                DiffLineKind::Added => "+",
                DiffLineKind::Removed => "-",
                DiffLineKind::Context => " ",
            };
            ui.child("mark", Text::new(marker))?;
            ui.child("text", Text::new(line.text.as_ref()))?;
            if show_actions && !matches!(line.kind, DiffLineKind::Context) {
                ui.child(
                    "line-accept",
                    Button::new(line_label(
                        &strings.diff_accept_line,
                        hunk_index,
                        line_index,
                    ))
                    .kind(ButtonKind::Text)
                    .size(ControlSize::Small)
                    .disabled(disabled),
                )?;
                ui.child(
                    "line-reject",
                    Button::new(line_label(
                        &strings.diff_reject_line,
                        hunk_index,
                        line_index,
                    ))
                    .kind(ButtonKind::Text)
                    .size(ControlSize::Small)
                    .disabled(disabled),
                )?;
            }
            Ok(())
        },
    )?;
    Ok(())
}

/// A hunk's review button keeps the width its label gives it at the start
/// of the hunk, which as a block stretches the lines across its width.
fn hunk_action(mut button: Button) -> Button {
    Arc::make_mut(&mut button.style.layout).align_self = Some(AlignSpec::Start);
    button
}

fn line_style(kind: DiffLineKind) -> NodeStyle {
    match kind {
        DiffLineKind::Added => NodeStyle::default().surface_mix(
            nana_ui_core::SemanticColorMix::alpha(SemanticColorRole::Success, 0.14),
        ),
        DiffLineKind::Removed => NodeStyle::default().surface_mix(
            nana_ui_core::SemanticColorMix::alpha(SemanticColorRole::Danger, 0.14),
        ),
        DiffLineKind::Context => NodeStyle::default(),
    }
}

fn muted_text() -> NodeStyle {
    let mut style = NodeStyle::default();
    style.foreground = Some(SemanticColorRole::Muted);
    style
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentId, LayoutViewport};
    use std::sync::{Arc, Mutex};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn sample_hunks() -> Arc<[DiffHunk]> {
        Arc::from([DiffHunk::new(
            "@@ -1,2 +1,2 @@",
            vec![
                DiffLine::context(1, 1, "fn main() {"),
                DiffLine::removed(2, "    let x = 1;"),
                DiffLine::added(2, "    let x = 2;"),
            ],
        )])
    }

    /// A hunk and the columns of a split hunk are blocks: every line row,
    /// its highlight with it, fills the hunk (or its column) and the hunk
    /// fills the body, while a review button stays as wide as its label at
    /// the hunk's start.
    #[test]
    fn line_rows_fill_the_hunk_and_review_buttons_keep_their_width() {
        for layout in [DiffLayout::Unified, DiffLayout::Split] {
            let mut context = AppContext::new();
            let diff = context
                .create_component(document(), DiffView::new(sample_hunks()).layout(layout))
                .unwrap();
            assert!(context.assemble_diff_view(diff).unwrap());
            context
                .layout_document(document(), LayoutViewport::new(480.0, 240.0))
                .unwrap();
            let world = context.world();
            let accept =
                find_labeled_button(&context, diff.stable_id(), "接受块0").expect("hunk accept");
            let hunk = world.parent_id(accept).expect("the hunk");
            let body_content = world.parent_id(hunk).expect("the body");
            let (accept, hunk, body_content) = (
                world.layout_box(accept).unwrap(),
                world.layout_box(hunk).unwrap(),
                world.layout_box(body_content).unwrap(),
            );
            let inset = 8.0;
            assert!(
                (hunk.width - body_content.width).abs() < 0.01,
                "{layout:?}: the hunk {hunk:?} fills the body {body_content:?}"
            );
            assert!(
                (accept.x - hunk.x - inset).abs() < 0.01
                    && accept.width < hunk.width / 2.0
                    && accept.width > 0.0,
                "{layout:?}: the review button {accept:?} keeps its width in {hunk:?}"
            );
            for label in ["接受行0.1", "接受行0.2"] {
                let button =
                    find_labeled_button(&context, diff.stable_id(), label).expect("line accept");
                let row_id = world.parent_id(button).expect("the line row");
                let row = world.layout_box(row_id).unwrap();
                let column = world
                    .layout_box(world.parent_id(row_id).expect("the line's block"))
                    .unwrap();
                let content = column.width
                    - if layout == DiffLayout::Unified {
                        2.0 * inset
                    } else {
                        0.0
                    };
                assert!(
                    (row.width - content).abs() < 0.01 && row.width > hunk.width / 3.0,
                    "{layout:?} {label}: the line {row:?} fills its block {column:?}"
                );
            }
        }
    }

    #[test]
    fn hunk_and_line_actions_emit_requests_without_mutating_hunks() {
        let mut context = AppContext::new();
        let diff = context
            .create_component(document(), DiffView::new(sample_hunks()))
            .unwrap();
        assert!(context.assemble_diff_view(diff).unwrap());
        context
            .layout_document(document(), LayoutViewport::new(480.0, 240.0))
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&events);
        context
            .on(diff, move |_, event: &DiffEvent, _| {
                log.lock().unwrap().push(event.clone());
            })
            .unwrap();

        let hunk_accept =
            find_labeled_button(&context, diff.stable_id(), "接受块0").expect("hunk accept");
        assert!(context.activate_node(hunk_accept).unwrap());
        let hunks_after = context.read(diff, |view| Arc::clone(&view.hunks)).unwrap();
        assert_eq!(hunks_after.len(), 1);
        let log = events.lock().unwrap();
        assert!(matches!(
            log.first(),
            Some(DiffEvent::HunkAccepted { hunk: 0 })
        ));
    }

    #[test]
    fn split_layout_shows_line_actions_on_the_added_column() {
        let mut context = AppContext::new();
        let diff = context
            .create_component(
                document(),
                DiffView::new(sample_hunks()).layout(DiffLayout::Split),
            )
            .unwrap();
        assert!(context.assemble_diff_view(diff).unwrap());
        assert!(
            find_labeled_button(&context, diff.stable_id(), "接受行0.2").is_some(),
            "added lines live on the new column and must keep accept/reject"
        );
        assert!(find_labeled_button(&context, diff.stable_id(), "接受行0.1").is_some());
        assert!(find_labeled_button(&context, diff.stable_id(), "接受行0.0").is_none());
    }

    #[test]
    fn read_only_review_has_no_actions_and_tints_changed_lines() {
        let mut context = AppContext::new();
        let diff = context
            .create_component(
                document(),
                DiffView::new(sample_hunks()).review_actions(false),
            )
            .unwrap();
        assert!(context.assemble_diff_view(diff).unwrap());
        for label in ["接受块0", "拒绝块0", "接受行0.1", "接受行0.2"] {
            assert!(find_labeled_button(&context, diff.stable_id(), label).is_none());
        }
        let added = find_labeled_button(&context, diff.stable_id(), "    let x = 2;")
            .and_then(|text| context.world().node(text).and_then(|node| node.parent))
            .expect("added line row");
        assert!(
            context.world().node_style(added).is_some_and(|style| style
                .interaction
                .base
                .background_mix
                .is_some()),
            "an added line keeps its success wash"
        );
    }
}
