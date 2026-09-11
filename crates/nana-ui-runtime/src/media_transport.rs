//! Dual-row media chrome with host-owned scene extras.
//!
//! Built-in controls are play, progress (seek or live meter), volume, settings
//! and fullscreen. Applications append scene-specific widgets into named slots.
//! An empty or fully hidden `secondary` slot collapses the bar to a single row.
//! Idle hide stays a host-fed [`crate::OverlayVisibility`] policy; this control
//! only exposes chrome and events.

use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, Icon, JustifySpec, LengthSpec, PointerEventsSpec,
    PopoverPlacement, PositionSpec, SemanticColorRole, UI_METRICS, space,
};

use crate::component_registry::{RegisterableComponent, SemanticSpec};
use crate::view_components::{
    Activate, IconButton, RangeChanged, RangeField, Stack, Text, project_common,
};
use crate::{
    AccessibilityRole, AccessibilityState, ActionMenu, AppContext, ComponentView, Divider, Entity,
    FrameworkError, InteractionState, MutationQueue, NodeKind, NodeStyle, Popover, Progress,
    StableNodeId, UiWorld,
};

const BAR_MAX_WIDTH: f32 = 820.0;
const BAR_MARGIN: f32 = space::XL;
const BACKPLATE_OPACITY: f32 = 0.88;
const VOLUME_POPOVER_WIDTH: f32 = 240.0;

/// Built-in transport action. Scene extras keep their own events.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaTransportEvent {
    PlayPause,
    Seek(f64),
    Volume(f64),
    Fullscreen,
}

/// Icons the host supplies for chrome that is not in the shell catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaTransportIcons {
    pub play: Icon,
    pub pause: Icon,
    pub volume: Icon,
    pub volume_muted: Icon,
    pub fullscreen: Icon,
    pub fullscreen_exit: Icon,
    pub settings: Icon,
}

impl Default for MediaTransportIcons {
    fn default() -> Self {
        Self {
            play: Icon::MonitorPlay,
            pause: Icon::Minimize,
            volume: Icon::Eye,
            volume_muted: Icon::Eye,
            fullscreen: Icon::Maximize,
            fullscreen_exit: Icon::Restore,
            settings: Icon::Settings,
        }
    }
}

/// Child nodes [`AppContext::assemble_media_transport_bar`] owns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaTransportSlots {
    pub play: Option<StableNodeId>,
    pub leading: Option<StableNodeId>,
    pub time: Option<StableNodeId>,
    pub seek: Option<StableNodeId>,
    pub live_progress: Option<StableNodeId>,
    pub trailing: Option<StableNodeId>,
    pub volume: Option<StableNodeId>,
    pub volume_menu: Option<StableNodeId>,
    pub settings: Option<StableNodeId>,
    pub fullscreen: Option<StableNodeId>,
    pub secondary: Option<StableNodeId>,
    pub secondary_row: Option<StableNodeId>,
}

/// Floating dual-row media bar (`nana.media-transport-bar`).
#[derive(Debug, Clone, PartialEq)]
pub struct MediaTransportBar {
    pub playing: bool,
    pub live: bool,
    pub fullscreen: bool,
    pub muted: bool,
    pub disabled: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub max_width: f32,
    pub icons: MediaTransportIcons,
    pub play_label: Arc<str>,
    pub pause_label: Arc<str>,
    pub volume_label: Arc<str>,
    pub settings_label: Arc<str>,
    pub fullscreen_label: Arc<str>,
    pub fullscreen_exit_label: Arc<str>,
    pub style: NodeStyle,
    pub(crate) slots: MediaTransportSlots,
}

impl MediaTransportBar {
    pub fn new() -> Self {
        Self {
            playing: false,
            live: false,
            fullscreen: false,
            muted: false,
            disabled: false,
            position: 0.0,
            duration: 0.0,
            volume: 100.0,
            max_width: BAR_MAX_WIDTH,
            icons: MediaTransportIcons::default(),
            play_label: Arc::from("播放"),
            pause_label: Arc::from("暂停"),
            volume_label: Arc::from("音量"),
            settings_label: Arc::from("播放设置"),
            fullscreen_label: Arc::from("全屏"),
            fullscreen_exit_label: Arc::from("退出全屏"),
            style: overlay_style(),
            slots: MediaTransportSlots::default(),
        }
    }

    pub fn live(mut self, live: bool) -> Self {
        self.live = live;
        self
    }

    pub fn icons(mut self, icons: MediaTransportIcons) -> Self {
        self.icons = icons;
        self
    }

    pub fn max_width(mut self, max_width: f32) -> Self {
        self.max_width = max_width.max(0.0);
        self
    }

    pub fn slots(&self) -> &MediaTransportSlots {
        &self.slots
    }

    pub fn leading(&self) -> Option<Entity<Stack>> {
        self.slots.leading.map(Entity::from_stable_id)
    }

    pub fn trailing(&self) -> Option<Entity<Stack>> {
        self.slots.trailing.map(Entity::from_stable_id)
    }

    pub fn secondary(&self) -> Option<Entity<Stack>> {
        self.slots.secondary.map(Entity::from_stable_id)
    }

    pub fn settings(&self) -> Option<Entity<ActionMenu>> {
        self.slots.settings.map(Entity::from_stable_id)
    }

    pub fn play(&self) -> Option<Entity<IconButton>> {
        self.slots.play.map(Entity::from_stable_id)
    }

    pub fn time(&self) -> Option<Entity<Text>> {
        self.slots.time.map(Entity::from_stable_id)
    }

    pub fn seek(&self) -> Option<Entity<RangeField>> {
        self.slots.seek.map(Entity::from_stable_id)
    }

    pub fn live_progress(&self) -> Option<Entity<Progress>> {
        self.slots.live_progress.map(Entity::from_stable_id)
    }

    pub fn volume(&self) -> Option<Entity<RangeField>> {
        self.slots.volume.map(Entity::from_stable_id)
    }

    pub fn volume_menu(&self) -> Option<Entity<Popover>> {
        self.slots.volume_menu.map(Entity::from_stable_id)
    }

    pub fn fullscreen_button(&self) -> Option<Entity<IconButton>> {
        self.slots.fullscreen.map(Entity::from_stable_id)
    }
}

impl Default for MediaTransportBar {
    fn default() -> Self {
        Self::new()
    }
}

fn overlay_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.position = PositionSpec::Absolute;
    layout.offset_left = Some(LengthSpec::Px(0.0));
    layout.offset_right = Some(LengthSpec::Px(0.0));
    layout.offset_bottom = Some(LengthSpec::Px(BAR_MARGIN));
    layout.z_index = Some(30);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.justify_content = JustifySpec::Center;
    layout.align_items = AlignSpec::End;
    layout.pointer_events = Some(PointerEventsSpec::None);
    layout.padding_left = Some(LengthSpec::Px(BAR_MARGIN));
    layout.padding_right = Some(LengthSpec::Px(BAR_MARGIN));
    layout.direction = Some(FlexDirection::Row);
    layout.width = Some(LengthSpec::Fill);
    style
}

impl ComponentView for MediaTransportBar {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "media-transport-bar".into(),
        }
    }

    fn reconcile(&mut self, mut next: Self) {
        next.slots = self.slots.clone();
        *self = next;
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.position = PositionSpec::Absolute;
        layout.offset_bottom = Some(LengthSpec::Px(BAR_MARGIN));
        layout.pointer_events = Some(PointerEventsSpec::None);
        layout.justify_content = JustifySpec::Center;
        layout.align_items = AlignSpec::End;
        layout.width = Some(LengthSpec::Fill);
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Toolbar,
                label: Some(Arc::from("播放控制")),
                ..AccessibilityState::default()
            },
        );
    }
}

impl RegisterableComponent for MediaTransportBar {
    const TYPE_ID: &'static str = crate::component_descriptors::MEDIA_TRANSPORT_BAR.type_id;
    const TAGS: &'static [&'static str] = crate::component_descriptors::MEDIA_TRANSPORT_BAR.tags;
    fn from_semantic(spec: &SemanticSpec<'_>) -> Self {
        MediaTransportBar::new()
            .live(
                spec.attr("live")
                    .is_some_and(|value| matches!(value, "true" | "1" | "live")),
            )
            .max_width(
                spec.attr("max-width")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(BAR_MAX_WIDTH),
            )
    }
}

impl AppContext {
    /// Builds the chrome, slot stacks and built-in controls. Idempotent.
    pub fn assemble_media_transport_bar(
        &mut self,
        bar: Entity<MediaTransportBar>,
    ) -> Result<bool, FrameworkError> {
        let document = self
            .world()
            .node(bar.stable_id())
            .ok_or(FrameworkError::MissingView(bar.stable_id()))?
            .document;
        let snapshot = self.read(bar, Clone::clone)?;
        let created = snapshot.slots.play.is_none();
        if created {
            let chrome = self.create_detached_component(
                document,
                Stack::column(space::MD)
                    .hittable()
                    .radius(UI_METRICS.radius_md)
                    .with_layout(|layout| {
                        layout.position = PositionSpec::Relative;
                        layout.width = Some(LengthSpec::Fill);
                        layout.max_width = Some(LengthSpec::Px(snapshot.max_width));
                        layout.min_width = Some(LengthSpec::Px(0.0));
                        layout.flex_grow = Some(1.0);
                        layout.flex_shrink = Some(1.0);
                        layout.pointer_events = Some(PointerEventsSpec::Auto);
                    }),
            )?;
            self.append_child(bar, chrome)?;
            let backplate = self.create_detached_component(
                document,
                Stack::column(0.0)
                    .surface(SemanticColorRole::Surface)
                    .radius(UI_METRICS.radius_md)
                    .with_layout(|layout| {
                        layout.position = PositionSpec::Absolute;
                        layout.offset_left = Some(LengthSpec::Px(0.0));
                        layout.offset_right = Some(LengthSpec::Px(0.0));
                        layout.offset_top = Some(LengthSpec::Px(0.0));
                        layout.offset_bottom = Some(LengthSpec::Px(0.0));
                        layout.opacity = Some(BACKPLATE_OPACITY);
                        layout.pointer_events = Some(PointerEventsSpec::None);
                        layout.z_index = Some(0);
                    }),
            )?;
            self.append_child(chrome, backplate)?;
            let row = self.create_detached_component(
                document,
                Stack::row(space::XL).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Fill);
                    layout.min_width = Some(LengthSpec::Px(0.0));
                    layout.align_items = AlignSpec::Center;
                    layout.padding_left = Some(LengthSpec::Px(space::XL));
                    layout.padding_right = Some(LengthSpec::Px(space::XL));
                    layout.padding_top = Some(LengthSpec::Px(space::XL));
                    layout.padding_bottom = Some(LengthSpec::Px(space::XS));
                    layout.z_index = Some(1);
                }),
            )?;
            self.append_child(chrome, row)?;
            let left = self.create_detached_component(
                document,
                Stack::row(space::XS).with_layout(|layout| {
                    layout.align_items = AlignSpec::Center;
                    layout.flex_shrink = Some(0.0);
                }),
            )?;
            self.append_child(row, left)?;
            let play = self.create_detached_component(
                document,
                chrome_icon(snapshot.icons.play, snapshot.play_label.as_ref()),
            )?;
            self.append_child(left, play)?;
            let leading = self.create_detached_component(
                document,
                Stack::row(space::XS).with_layout(|layout| {
                    layout.align_items = AlignSpec::Center;
                    layout.flex_shrink = Some(0.0);
                }),
            )?;
            self.append_child(left, leading)?;
            let center = self.create_detached_component(
                document,
                Stack::column(space::XXS).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Fill);
                    layout.min_width = Some(LengthSpec::Px(0.0));
                    layout.flex_grow = Some(1.0);
                    layout.flex_shrink = Some(1.0);
                    layout.align_items = AlignSpec::Stretch;
                }),
            )?;
            self.append_child(row, center)?;
            let time = self.create_detached_component(
                document,
                Text::new("0:00 / 0:00")
                    .font_size(12.0)
                    .color(SemanticColorRole::Muted),
            )?;
            self.append_child(center, time)?;
            let mut seek = RangeField::new(0.0, 0.0, 1.0, 1.0)
                .show_value(false)
                .size(ControlSize::Small)
                .label("进度");
            {
                let layout = Arc::make_mut(&mut seek.style.layout);
                layout.width = Some(LengthSpec::Fill);
                layout.min_width = Some(LengthSpec::Px(0.0));
                layout.flex_grow = Some(1.0);
                layout.flex_shrink = Some(1.0);
            }
            let seek = self.create_detached_component(document, seek)?;
            self.append_child(center, seek)?;
            let mut live_progress = Progress::new(1.0, 1.0);
            {
                let layout = Arc::make_mut(&mut live_progress.style.layout);
                layout.width = Some(LengthSpec::Fill);
                layout.min_width = Some(LengthSpec::Px(0.0));
            }
            let live_progress = self.create_detached_component(document, live_progress)?;
            self.append_child(center, live_progress)?;
            let right = self.create_detached_component(
                document,
                Stack::row(space::XS).with_layout(|layout| {
                    layout.align_items = AlignSpec::Center;
                    layout.flex_shrink = Some(0.0);
                }),
            )?;
            self.append_child(row, right)?;
            let trailing = self.create_detached_component(
                document,
                Stack::row(space::XS).with_layout(|layout| {
                    layout.align_items = AlignSpec::Center;
                    layout.flex_shrink = Some(0.0);
                }),
            )?;
            self.append_child(right, trailing)?;
            let volume_menu = self.create_detached_component(
                document,
                Popover::new()
                    .trigger_icon(snapshot.icons.volume, snapshot.volume_label.as_ref())
                    .placement(PopoverPlacement::Top)
                    .width(VOLUME_POPOVER_WIDTH),
            )?;
            self.append_child(right, volume_menu)?;
            let volume = self.create_detached_component(
                document,
                RangeField::new(100.0, 0.0, 100.0, 1.0)
                    .label("音量")
                    .show_value(false)
                    .size(ControlSize::Small),
            )?;
            self.append_child(volume_menu, volume)?;
            let settings = self.create_detached_component(
                document,
                ActionMenu::new()
                    .trigger_icon(snapshot.icons.settings, snapshot.settings_label.as_ref())
                    .placement(PopoverPlacement::Top),
            )?;
            self.append_child(right, settings)?;
            let fullscreen = self.create_detached_component(
                document,
                chrome_icon(
                    snapshot.icons.fullscreen,
                    snapshot.fullscreen_label.as_ref(),
                ),
            )?;
            self.append_child(right, fullscreen)?;
            let secondary_row = self.create_detached_component(
                document,
                Stack::column(space::XS).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Fill);
                    layout.min_width = Some(LengthSpec::Px(0.0));
                    layout.padding_left = Some(LengthSpec::Px(space::XS));
                    layout.padding_right = Some(LengthSpec::Px(space::XS));
                    layout.padding_bottom = Some(LengthSpec::Px(space::XS));
                    layout.z_index = Some(1);
                    layout.hidden = true;
                }),
            )?;
            self.append_child(chrome, secondary_row)?;
            let divider = self.create_detached_component(document, Divider::horizontal())?;
            self.append_child(secondary_row, divider)?;
            let secondary = self.create_detached_component(
                document,
                Stack::row(space::XL).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Fill);
                    layout.min_width = Some(LengthSpec::Px(0.0));
                    layout.align_items = AlignSpec::Center;
                    layout.padding = Some(LengthSpec::Px(space::XS));
                }),
            )?;
            self.append_child(secondary_row, secondary)?;

            self.observe(play, bar, |_, _: &Activate, cx| {
                cx.emit(MediaTransportEvent::PlayPause);
            })?;
            self.observe(seek, bar, |_, event: &RangeChanged, cx| {
                cx.emit(MediaTransportEvent::Seek(event.value));
            })?;
            self.observe(volume, bar, |_, event: &RangeChanged, cx| {
                cx.emit(MediaTransportEvent::Volume(event.value));
            })?;
            self.observe(fullscreen, bar, |_, _: &Activate, cx| {
                cx.emit(MediaTransportEvent::Fullscreen);
            })?;

            self.update_component(bar, |bar, _| {
                bar.slots = MediaTransportSlots {
                    play: Some(play.stable_id()),
                    leading: Some(leading.stable_id()),
                    time: Some(time.stable_id()),
                    seek: Some(seek.stable_id()),
                    live_progress: Some(live_progress.stable_id()),
                    trailing: Some(trailing.stable_id()),
                    volume: Some(volume.stable_id()),
                    volume_menu: Some(volume_menu.stable_id()),
                    settings: Some(settings.stable_id()),
                    fullscreen: Some(fullscreen.stable_id()),
                    secondary: Some(secondary.stable_id()),
                    secondary_row: Some(secondary_row.stable_id()),
                };
            })?;
        }
        self.sync_media_transport_bar(bar)?;
        Ok(created)
    }

    /// Refreshes built-in chrome and collapses an empty second row.
    pub fn sync_media_transport_bar(
        &mut self,
        bar: Entity<MediaTransportBar>,
    ) -> Result<(), FrameworkError> {
        let snapshot = self.read(bar, Clone::clone)?;
        let slots = snapshot.slots.clone();
        if let Some(play) = slots.play {
            let (icon, label) = if snapshot.playing {
                (snapshot.icons.pause, snapshot.pause_label.as_ref())
            } else {
                (snapshot.icons.play, snapshot.play_label.as_ref())
            };
            self.update_component(Entity::<IconButton>::from_stable_id(play), |button, _| {
                button.icon = icon;
                if button.label.as_ref() != label {
                    button.label = Arc::from(label);
                    button.tooltip = Some(crate::IconButtonTooltip {
                        label: button.label.clone(),
                        config: Default::default(),
                    });
                }
                button.disabled = snapshot.disabled;
            })?;
        }
        if let Some(time) = slots.time {
            self.update_component(Entity::<Text>::from_stable_id(time), |text, _| {
                let layout = Arc::make_mut(&mut text.style.layout);
                layout.hidden = snapshot.live;
            })?;
        }
        if let Some(seek) = slots.seek {
            let dragging = self
                .read(Entity::<RangeField>::from_stable_id(seek), |range| {
                    range.dragging.is_some()
                })
                .unwrap_or(false);
            self.update_component(Entity::<RangeField>::from_stable_id(seek), |range, _| {
                let layout = Arc::make_mut(&mut range.style.layout);
                layout.hidden = snapshot.live;
                range.disabled = snapshot.disabled || snapshot.live;
                let maximum = snapshot.duration.max(1.0);
                range.minimum = 0.0;
                range.maximum = maximum;
                if !dragging {
                    range.value = snapshot.position.clamp(0.0, maximum);
                }
            })?;
        }
        if let Some(live_progress) = slots.live_progress {
            self.update_component(
                Entity::<Progress>::from_stable_id(live_progress),
                |progress, _| {
                    let layout = Arc::make_mut(&mut progress.style.layout);
                    layout.hidden = !snapshot.live;
                },
            )?;
        }
        if let Some(volume) = slots.volume {
            let dragging = self
                .read(Entity::<RangeField>::from_stable_id(volume), |range| {
                    range.dragging.is_some()
                })
                .unwrap_or(false);
            self.update_component(Entity::<RangeField>::from_stable_id(volume), |range, _| {
                range.disabled = snapshot.disabled;
                if !dragging {
                    range.value = snapshot.volume.clamp(0.0, 100.0);
                }
            })?;
        }
        if let Some(volume_menu) = slots.volume_menu {
            let muted = snapshot.muted || snapshot.volume <= 0.0;
            let icon = if muted {
                snapshot.icons.volume_muted
            } else {
                snapshot.icons.volume
            };
            self.update_component(Entity::<Popover>::from_stable_id(volume_menu), |menu, _| {
                menu.trigger_icon = Some(icon);
                menu.trigger = Arc::clone(&snapshot.volume_label);
            })?;
        }
        if let Some(fullscreen) = slots.fullscreen {
            let (icon, label) = if snapshot.fullscreen {
                (
                    snapshot.icons.fullscreen_exit,
                    snapshot.fullscreen_exit_label.as_ref(),
                )
            } else {
                (
                    snapshot.icons.fullscreen,
                    snapshot.fullscreen_label.as_ref(),
                )
            };
            self.update_component(
                Entity::<IconButton>::from_stable_id(fullscreen),
                |button, _| {
                    button.icon = icon;
                    if button.label.as_ref() != label {
                        button.label = Arc::from(label);
                        button.tooltip = Some(crate::IconButtonTooltip {
                            label: button.label.clone(),
                            config: Default::default(),
                        });
                    }
                },
            )?;
        }
        if let (Some(secondary), Some(secondary_row)) = (slots.secondary, slots.secondary_row) {
            let visible = self.slot_has_visible_child(secondary);
            self.update_component(Entity::<Stack>::from_stable_id(secondary_row), |row, _| {
                *row = row.clone().with_layout(|layout| layout.hidden = !visible);
            })?;
        }
        Ok(())
    }

    fn slot_has_visible_child(&self, slot: StableNodeId) -> bool {
        self.world()
            .node(slot)
            .map(|node| {
                node.children.iter().any(|child| {
                    self.world()
                        .node_style(*child)
                        .is_some_and(|style| !style.layout.hidden)
                })
            })
            .unwrap_or(false)
    }
}

fn chrome_icon(icon: Icon, label: &str) -> IconButton {
    IconButton::new(icon, label)
        .size(ControlSize::Small)
        .with_tooltip(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use std::sync::{Arc as StdArc, Mutex};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    #[test]
    fn empty_secondary_stays_a_single_row_until_a_child_is_appended() {
        let mut cx = AppContext::new();
        let bar = cx
            .create_component(document(), MediaTransportBar::new())
            .unwrap();
        assert!(cx.assemble_media_transport_bar(bar).unwrap());
        let slots = cx.read(bar, |bar| bar.slots().clone()).unwrap();
        let row = slots.secondary_row.unwrap();
        assert!(
            cx.world().node_style(row).unwrap().layout.hidden,
            "no extras means one row"
        );

        let extra = cx
            .create_detached_component(document(), Stack::row(0.0))
            .unwrap();
        cx.append_child(
            Entity::<Stack>::from_stable_id(slots.secondary.unwrap()),
            extra,
        )
        .unwrap();
        cx.sync_media_transport_bar(bar).unwrap();
        assert!(
            !cx.world().node_style(row).unwrap().layout.hidden,
            "a visible child opens the second row"
        );

        cx.update_component(extra, |stack, _| {
            *stack = stack.clone().with_layout(|layout| layout.hidden = true);
        })
        .unwrap();
        cx.sync_media_transport_bar(bar).unwrap();
        assert!(
            cx.world().node_style(row).unwrap().layout.hidden,
            "hiding every extra collapses the bar"
        );
    }

    #[test]
    fn live_hides_seek_and_shows_a_meter() {
        let mut cx = AppContext::new();
        let bar = cx
            .create_component(document(), MediaTransportBar::new().live(true))
            .unwrap();
        cx.assemble_media_transport_bar(bar).unwrap();
        let slots = cx.read(bar, |bar| bar.slots().clone()).unwrap();
        assert!(
            cx.world()
                .node_style(slots.seek.unwrap())
                .unwrap()
                .layout
                .hidden
        );
        assert!(
            cx.world()
                .node_style(slots.time.unwrap())
                .unwrap()
                .layout
                .hidden
        );
        assert!(
            !cx.world()
                .node_style(slots.live_progress.unwrap())
                .unwrap()
                .layout
                .hidden
        );
    }

    #[test]
    fn play_and_seek_report_transport_events() {
        let mut cx = AppContext::new();
        let bar = cx
            .create_component(document(), MediaTransportBar::new())
            .unwrap();
        cx.assemble_media_transport_bar(bar).unwrap();
        cx.update_component(bar, |bar, _| bar.duration = 120.0)
            .unwrap();
        cx.sync_media_transport_bar(bar).unwrap();
        let slots = cx.read(bar, |bar| bar.slots().clone()).unwrap();
        let seen = StdArc::new(Mutex::new(Vec::new()));
        let out = StdArc::clone(&seen);
        cx.on(bar, move |_, event: &MediaTransportEvent, _| {
            out.lock().unwrap().push(*event);
        })
        .unwrap();
        cx.activate_icon_button(Entity::from_stable_id(slots.play.unwrap()))
            .unwrap();
        cx.set_range_value(Entity::from_stable_id(slots.seek.unwrap()), 12.0)
            .unwrap();
        let events = seen.lock().unwrap().clone();
        assert!(events.contains(&MediaTransportEvent::PlayPause));
        assert!(events.iter().any(|event| matches!(
            event,
            MediaTransportEvent::Seek(value) if (*value - 12.0).abs() < f64::EPSILON
        )));
    }

    #[test]
    fn assemble_is_idempotent() {
        let mut cx = AppContext::new();
        let bar = cx
            .create_component(document(), MediaTransportBar::new())
            .unwrap();
        assert!(cx.assemble_media_transport_bar(bar).unwrap());
        let first = cx.read(bar, |bar| bar.slots().clone()).unwrap();
        assert!(!cx.assemble_media_transport_bar(bar).unwrap());
        let second = cx.read(bar, |bar| bar.slots().clone()).unwrap();
        assert_eq!(first, second);
    }
}
