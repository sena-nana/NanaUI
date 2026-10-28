//! The page the mobile prototype measures (`docs/reference/mobile-host.md`):
//! a search field for the native text proxy, a switch and a slider, a
//! camera preview area, and a grid of 200 performance buttons in a vertical
//! scroll view. The iOS and Android hosts mount the same page so their
//! numbers compare.
//!
//! The preview area has no background: a native camera layer under a
//! transparent surface shows through it. With CPU preview on, the host
//! instead uploads frames into [`PREVIEW_SLOT`] and the page samples them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use nana_mobile_host::MobileRuntime;
use nana_ui::runtime::view::{El, entity_ref, widget, with_refs};
use nana_ui::runtime::{
    Activate, Button, Entity, FrameworkError, GpuTextureView, List, RangeChanged, RangeField,
    ScrollAxes, ScrollView, Switch, Text, TextChanged, TextInput, ToggleChanged,
};
use nana_ui::{
    GpuContext, GpuError, GpuTexture, GpuTextureDescriptor, GpuTextureFormat, GpuTextureRegion,
    GpuTextureUsages, HostTexture, HostTextureAlphaMode, HostTextureRegistry,
};
use nana_ui_core::{ContentFit, FlexDirection, FlexWrap, LayoutStyle, LengthSpec, LogicalRect};

/// Host texture slot the CPU preview uploads into.
pub const PREVIEW_SLOT: &str = "probe-camera";
/// Buttons in the grid.
pub const GRID_LEN: usize = 200;
/// Logical height of the preview area.
const PREVIEW_HEIGHT: f32 = 220.0;
const PAGE_PADDING: f32 = 16.0;
const GAP: f32 = 12.0;

/// What the host reads back from the page.
#[derive(Clone, Default)]
pub struct ProbeState {
    presses: Arc<AtomicU32>,
    cpu_preview: Arc<AtomicBool>,
    pressed: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl ProbeState {
    /// `pressed` runs on every button activation: the host's haptic.
    pub fn with_press_feedback(pressed: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            pressed: Some(Arc::new(pressed)),
            ..Self::default()
        }
    }

    pub fn presses(&self) -> u32 {
        self.presses.load(Ordering::Relaxed)
    }

    /// Whether the page asks for the CPU-uploaded preview instead of the
    /// native layer.
    pub fn cpu_preview(&self) -> bool {
        self.cpu_preview.load(Ordering::Relaxed)
    }
}

/// The mounted page.
pub struct Probe {
    pub state: ProbeState,
    preview: Entity<GpuTextureView>,
    #[cfg_attr(not(test), allow(dead_code))]
    first_button: Entity<Button>,
}

impl Probe {
    pub fn mount(runtime: &mut MobileRuntime, state: ProbeState) -> Result<Self, FrameworkError> {
        let page_state = state.clone();
        let (_, (preview, first_button)) = runtime.mount(move || {
            let (preview, first_button) = (entity_ref(), entity_ref());
            let toggles = page_state.cpu_preview.clone();
            let buttons: Vec<_> = (0..GRID_LEN)
                .map(|index| {
                    let button = grid_button(index, &page_state);
                    if index == 0 {
                        button.entity_ref(first_button)
                    } else {
                        button
                    }
                })
                .collect();
            with_refs(
                widget(container(|layout| {
                    layout.direction = Some(FlexDirection::Column);
                    layout.width = Some(LengthSpec::Fill);
                    layout.height = Some(LengthSpec::Fill);
                    layout.gap = Some(LengthSpec::Px(GAP));
                    layout.padding = Some(LengthSpec::Px(PAGE_PADDING));
                }))
                .key("probe-page")
                .children((
                    widget(Text::new("NanaUI 移动端原型")).key("title"),
                    widget(TextInput::new("")).key("search").on_cx(
                        |field: &mut TextInput, event: &TextChanged, _| {
                            field.state.replace_value(event.value.clone());
                        },
                    ),
                    widget(container(|layout| {
                        layout.direction = Some(FlexDirection::Row);
                        layout.width = Some(LengthSpec::Fill);
                        layout.gap = Some(LengthSpec::Px(GAP));
                    }))
                    .key("controls")
                    .children((
                        widget(Switch::new("CPU 预览", false))
                            .key("cpu-preview")
                            .on_cx(move |switch: &mut Switch, event: &ToggleChanged, _| {
                                switch.checked = event.checked;
                                toggles.store(event.checked, Ordering::Relaxed);
                            }),
                        widget(RangeField::new(50.0, 0.0, 100.0, 1.0))
                            .key("strength")
                            .on_cx(|range: &mut RangeField, event: &RangeChanged, _| {
                                range.value = event.value;
                            }),
                    )),
                    widget(preview_view()).key("preview").entity_ref(preview),
                    widget(ScrollView::new(ScrollAxes::Vertical).with_layout(|layout| {
                        layout.width = Some(LengthSpec::Fill);
                        layout.flex_grow = Some(1.0);
                        layout.min_height = Some(LengthSpec::Px(0.0));
                    }))
                    .key("grid-scroll")
                    .children(
                        widget(container(|layout| {
                            layout.direction = Some(FlexDirection::Row);
                            layout.flex_wrap = FlexWrap::Wrap;
                            layout.width = Some(LengthSpec::Fill);
                            layout.gap = Some(LengthSpec::Px(GAP));
                        }))
                        .key("grid")
                        .children(buttons),
                    ),
                )),
                (preview, first_button),
            )
        })?;
        Ok(Self {
            state,
            preview,
            first_button,
        })
    }

    /// Where the preview area is, in logical px of the surface: the shell
    /// lays its native camera layer out to it.
    pub fn preview_rect(&self, runtime: &MobileRuntime) -> Option<LogicalRect> {
        let layout = runtime
            .document()
            .context()
            .world()
            .layout_box(self.preview.stable_id())?;
        Some(LogicalRect::new(
            layout.x,
            layout.y,
            layout.width,
            layout.height,
        ))
    }
}

/// The CPU preview path: camera frames copied into [`PREVIEW_SLOT`] each
/// frame, for comparing against a native preview layer.
#[derive(Default)]
pub struct PreviewUploader {
    texture: Option<(GpuTexture, HostTexture, u32, u32)>,
}

impl PreviewUploader {
    /// Upload one BGRA8 frame and mark the slot changed. The first frame,
    /// and any size change, registers a new texture.
    pub fn push_bgra(
        &mut self,
        gpu: &GpuContext,
        textures: &HostTextureRegistry,
        bytes: &[u8],
        width: u32,
        height: u32,
        bytes_per_row: u32,
    ) -> Result<(), GpuError> {
        let stale = self
            .texture
            .as_ref()
            .is_none_or(|(_, _, w, h)| (*w, *h) != (width, height));
        if stale {
            let texture = gpu.create_texture(&GpuTextureDescriptor {
                label: Some("nana-mobile-probe preview"),
                width,
                height,
                format: GpuTextureFormat::BGRA8_UNORM_SRGB,
                usage: GpuTextureUsages::COPY_DST | GpuTextureUsages::SAMPLED,
            })?;
            let host = HostTexture::new(&texture);
            textures.register(
                PREVIEW_SLOT,
                host.clone(),
                width,
                height,
                HostTextureAlphaMode::Opaque,
            );
            self.texture = Some((texture, host, width, height));
        }
        let (texture, ..) = self.texture.as_ref().expect("preview texture");
        gpu.write_texture(
            texture,
            GpuTextureRegion::full(width, height),
            bytes,
            bytes_per_row,
        )?;
        textures.invalidate(PREVIEW_SLOT);
        Ok(())
    }

    /// Back to the native layer: the slot samples nothing again.
    pub fn clear(&mut self, textures: &HostTextureRegistry) {
        if self.texture.take().is_some() {
            textures.remove(PREVIEW_SLOT);
        }
    }
}

fn grid_button(index: usize, state: &ProbeState) -> El<Button> {
    let presses = state.presses.clone();
    let pressed = state.pressed.clone();
    widget(Button::new(format!("表演 {}", index + 1)))
        .key(format!("perform-{index}"))
        .on(move |_: &Activate| {
            presses.fetch_add(1, Ordering::Relaxed);
            if let Some(pressed) = &pressed {
                pressed();
            }
        })
}

fn preview_view() -> GpuTextureView {
    let mut view = GpuTextureView::new(PREVIEW_SLOT);
    view.label = Some("摄像头预览".into());
    view.fit = ContentFit::Cover;
    view.corner_radius = 12.0;
    let layout = Arc::make_mut(&mut view.style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Px(PREVIEW_HEIGHT));
    view
}

fn container(f: impl FnOnce(&mut LayoutStyle)) -> List {
    let mut list = List::new();
    f(Arc::make_mut(&mut list.style.layout));
    list
}

#[cfg(test)]
mod tests {
    use nana_mobile_host::{
        MobileAppearance, MobilePlatform, MobileViewport, SafeArea, TouchPhase,
    };
    use nana_ui_platform::{
        SharedClipboardHost, TextInputContext, UnsupportedClipboard, shared_clipboard,
    };

    use super::*;

    struct Inert;

    impl MobilePlatform for Inert {
        fn set_text_input(&mut self, _state: Option<&TextInputContext>) {}

        fn clipboard(&self) -> SharedClipboardHost {
            shared_clipboard(UnsupportedClipboard)
        }
    }

    fn mounted() -> (MobileRuntime, Probe) {
        let viewport = MobileViewport::new((1179, 2556), 3.0, SafeArea::default());
        let mut runtime =
            MobileRuntime::new(viewport, MobileAppearance::default(), Box::new(Inert)).unwrap();
        let probe = Probe::mount(&mut runtime, ProbeState::default()).unwrap();
        (runtime, probe)
    }

    #[test]
    fn the_preview_area_lays_out_inside_the_surface() {
        let (runtime, probe) = mounted();
        let rect = probe.preview_rect(&runtime).expect("preview laid out");
        assert_eq!(rect.height, PREVIEW_HEIGHT);
        assert!(rect.width > 300.0 && rect.width <= 393.0, "{rect:?}");
    }

    #[test]
    fn a_grid_button_counts_a_press() {
        let (mut runtime, probe) = mounted();
        let first = runtime
            .document()
            .context()
            .world()
            .layout_box(probe.first_button.stable_id())
            .expect("first grid button");
        let (x, y) = ((first.x + 10.0) * 3.0, (first.y + 10.0) * 3.0);
        runtime.push_touch(TouchPhase::Down, x, y, 1).unwrap();
        runtime.push_touch(TouchPhase::Up, x, y, 1).unwrap();
        assert_eq!(probe.state.presses(), 1);
    }
}
