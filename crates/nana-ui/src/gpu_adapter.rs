//! Which GPU adapter the hosted device runs on: what a program asks for at
//! startup ([`crate::StartupOptions::with_adapter`]), and what the host found
//! and chose ([`crate::RuntimeProgramContext::gpu_adapters`]).

use nana_gpu::{GpuBackend, GpuDeviceType};

/// Minimal adapter capabilities considered while ranking surface-compatible
/// adapters at startup or recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdapterCapabilitySummary {
    pub backend: GpuBackend,
    pub supports_view_formats: bool,
    pub has_renderable_surface_format: bool,
}

/// Rank adapters so complete hardware backends supporting `VIEW_FORMATS` and
/// renderable surface formats always beat incomplete GL/CPU software adapters.
fn capability_tier(summary: AdapterCapabilitySummary) -> u8 {
    if !summary.has_renderable_surface_format {
        return 4;
    }
    let is_gl = summary.backend == GpuBackend::Gl;
    match (summary.supports_view_formats, is_gl) {
        (true, false) => 0,
        (false, false) => 1,
        (true, true) => 2,
        (false, true) => 3,
    }
}

pub(crate) fn automatic_adapter_with_capabilities(
    adapters: &[GpuAdapterInfo],
    summaries: &[AdapterCapabilitySummary],
    env_name: Option<&str>,
    power: wgpu::PowerPreference,
) -> Option<usize> {
    let valid = |index: usize| {
        summaries
            .get(index)
            .is_none_or(|s| s.has_renderable_surface_format)
    };
    if let Some(name) = env_name {
        let name = name.to_lowercase();
        return adapters
            .iter()
            .enumerate()
            .filter(|(index, adapter)| valid(*index) && adapter.name.to_lowercase().contains(&name))
            .min_by_key(|(index, _)| {
                let tier = summaries.get(*index).copied().map_or(0, capability_tier);
                (tier, *index)
            })
            .map(|(index, _)| index);
    }
    let integrated_first = power == wgpu::PowerPreference::LowPower;
    adapters
        .iter()
        .enumerate()
        .filter(|(index, _)| valid(*index))
        .min_by_key(|(index, adapter)| {
            let summary = summaries.get(*index).copied();
            let tier = summary.map_or(0, capability_tier);
            let cpu_penalty = u8::from(adapter.device_type == GpuDeviceType::Cpu);
            let power_rank = if power == wgpu::PowerPreference::None {
                0
            } else {
                adapter_order(adapter.device_type, integrated_first)
            };
            (tier, cpu_penalty, power_rank, *index)
        })
        .map(|(index, _)| index)
}

pub(crate) fn pick_adapter_with_capabilities(
    adapters: &[GpuAdapterInfo],
    summaries: &[AdapterCapabilitySummary],
    choice: &GpuAdapterChoice,
    env_name: Option<&str>,
    power: wgpu::PowerPreference,
) -> (Option<usize>, bool) {
    if let GpuAdapterChoice::Matching(id) = choice
        && let Some(index) = adapters.iter().position(|adapter| id.matches(adapter))
        && summaries
            .get(index)
            .is_none_or(|s| s.has_renderable_surface_format)
    {
        return (Some(index), false);
    }
    let requested_missing = matches!(choice, GpuAdapterChoice::Matching(_));
    (
        automatic_adapter_with_capabilities(adapters, summaries, env_name, power),
        requested_missing,
    )
}

/// One adapter that can present the primary window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuAdapterInfo {
    pub name: String,
    /// PCI vendor id (0 where the backend does not report one).
    pub vendor: u32,
    /// PCI device id (0 where the backend does not report one).
    pub device: u32,
    pub device_type: GpuDeviceType,
    /// The PCI bus address, which tells two cards of the same model apart.
    /// Empty where the backend does not report one.
    pub pci_bus_id: String,
}

impl GpuAdapterInfo {
    pub(crate) fn from_wgpu(info: &wgpu::AdapterInfo) -> Self {
        Self {
            name: info.name.clone(),
            vendor: info.vendor,
            device: info.device,
            device_type: match info.device_type {
                wgpu::DeviceType::Other => GpuDeviceType::Other,
                wgpu::DeviceType::IntegratedGpu => GpuDeviceType::IntegratedGpu,
                wgpu::DeviceType::DiscreteGpu => GpuDeviceType::DiscreteGpu,
                wgpu::DeviceType::VirtualGpu => GpuDeviceType::VirtualGpu,
                wgpu::DeviceType::Cpu => GpuDeviceType::Cpu,
            },
            pci_bus_id: info.device_pci_bus_id.clone(),
        }
    }

    /// The identity to save and ask for again on a later run.
    pub fn id(&self) -> GpuAdapterId {
        GpuAdapterId {
            name: self.name.clone(),
            vendor: self.vendor,
            device: self.device,
            pci_bus_id: self.pci_bus_id.clone(),
        }
    }
}

/// A saved adapter identity. It matches an adapter of the same vendor and
/// device id; the PCI bus address then decides between identical cards, or,
/// where either side has none, the name does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GpuAdapterId {
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    pub pci_bus_id: String,
}

impl GpuAdapterId {
    pub fn matches(&self, adapter: &GpuAdapterInfo) -> bool {
        self.vendor == adapter.vendor
            && self.device == adapter.device
            && if self.pci_bus_id.is_empty() || adapter.pci_bus_id.is_empty() {
                self.name == adapter.name
            } else {
                self.pci_bus_id == adapter.pci_bus_id
            }
    }
}

/// The adapter a program asks the host to run on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum GpuAdapterChoice {
    /// The host's own choice: `WGPU_ADAPTER_NAME`, then `WGPU_POWER_PREF`,
    /// then the first adapter that can present the window.
    #[default]
    Automatic,
    /// This adapter when it is present and can present the window; otherwise
    /// the automatic choice, reported as [`GpuAdapterSelection::requested_missing`].
    Matching(GpuAdapterId),
}

/// What the host found and chose when it created the current device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpuAdapterSelection {
    /// Every adapter that could present the window, in enumeration order.
    pub adapters: Vec<GpuAdapterInfo>,
    /// The one the device runs on, as an index into `adapters`.
    pub active: Option<usize>,
    /// A [`GpuAdapterChoice::Matching`] adapter was asked for but was absent,
    /// could not present the window, or failed to create a device.
    pub requested_missing: bool,
}

impl GpuAdapterSelection {
    pub fn active_adapter(&self) -> Option<&GpuAdapterInfo> {
        self.active.and_then(|index| self.adapters.get(index))
    }
}

/// The index of the adapter `choice` selects among surface-compatible
/// `adapters`, and whether a requested adapter had to be passed over. `None`
/// sends the caller to wgpu's own request, which also reports why nothing fits.
#[cfg(test)]
pub(crate) fn pick_adapter(
    adapters: &[GpuAdapterInfo],
    choice: &GpuAdapterChoice,
    env_name: Option<&str>,
    power: wgpu::PowerPreference,
) -> (Option<usize>, bool) {
    if let GpuAdapterChoice::Matching(id) = choice
        && let Some(index) = adapters.iter().position(|adapter| id.matches(adapter))
    {
        return (Some(index), false);
    }
    let requested_missing = matches!(choice, GpuAdapterChoice::Matching(_));
    (
        automatic_adapter(adapters, env_name, power),
        requested_missing,
    )
}

/// The adapter `request_adapter` would choose: the first whose name contains
/// `WGPU_ADAPTER_NAME` when that is set, else the first in enumeration order
/// after the stable `WGPU_POWER_PREF` sort `request_adapter` applies.
#[cfg(test)]
pub(crate) fn automatic_adapter(
    adapters: &[GpuAdapterInfo],
    env_name: Option<&str>,
    power: wgpu::PowerPreference,
) -> Option<usize> {
    if let Some(name) = env_name {
        let name = name.to_lowercase();
        return adapters
            .iter()
            .position(|adapter| adapter.name.to_lowercase().contains(&name));
    }
    if power == wgpu::PowerPreference::None {
        return (!adapters.is_empty()).then_some(0);
    }
    let integrated_first = power == wgpu::PowerPreference::LowPower;
    adapters
        .iter()
        .enumerate()
        .min_by_key(|(index, adapter)| {
            (adapter_order(adapter.device_type, integrated_first), *index)
        })
        .map(|(index, _)| index)
}

/// `request_adapter`'s power-preference rank (wgpu-core `get_order`).
fn adapter_order(device_type: GpuDeviceType, integrated_first: bool) -> u8 {
    match device_type {
        GpuDeviceType::DiscreteGpu if integrated_first => 2,
        GpuDeviceType::IntegratedGpu if integrated_first => 1,
        GpuDeviceType::DiscreteGpu => 1,
        GpuDeviceType::IntegratedGpu => 2,
        GpuDeviceType::VirtualGpu => 4,
        GpuDeviceType::Cpu => 5,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(name: &str, device: u32, kind: GpuDeviceType, bus: &str) -> GpuAdapterInfo {
        GpuAdapterInfo {
            name: name.into(),
            vendor: 0x10de,
            device,
            device_type: kind,
            pci_bus_id: bus.into(),
        }
    }

    fn adapters() -> Vec<GpuAdapterInfo> {
        vec![
            adapter(
                "Integrated",
                1,
                GpuDeviceType::IntegratedGpu,
                "0000:00:02.0",
            ),
            adapter("Discrete", 2, GpuDeviceType::DiscreteGpu, "0000:01:00.0"),
            adapter("Discrete", 2, GpuDeviceType::DiscreteGpu, "0000:02:00.0"),
        ]
    }

    #[test]
    fn automatic_takes_the_first_adapter_without_a_preference() {
        let none = wgpu::PowerPreference::None;
        assert_eq!(
            pick_adapter(&adapters(), &GpuAdapterChoice::Automatic, None, none),
            (Some(0), false)
        );
        assert_eq!(automatic_adapter(&[], None, none), None);
    }

    #[test]
    fn automatic_follows_the_environment() {
        let list = adapters();
        let high = wgpu::PowerPreference::HighPerformance;
        assert_eq!(automatic_adapter(&list, None, high), Some(1));
        assert_eq!(
            automatic_adapter(&list, None, wgpu::PowerPreference::LowPower),
            Some(0)
        );
        assert_eq!(
            automatic_adapter(&list, Some("discrete"), wgpu::PowerPreference::None),
            Some(1)
        );
        assert_eq!(automatic_adapter(&list, Some("absent"), high), None);
    }

    #[test]
    fn a_matching_choice_tells_identical_cards_apart_by_bus() {
        let list = adapters();
        let second = GpuAdapterChoice::Matching(list[2].id());
        assert_eq!(
            pick_adapter(
                &list,
                &second,
                Some("integrated"),
                wgpu::PowerPreference::None
            ),
            (Some(2), false)
        );
    }

    #[test]
    fn a_matching_choice_without_a_bus_compares_names() {
        let mut list = adapters();
        list[1].pci_bus_id.clear();
        let mut id = list[1].id();
        id.pci_bus_id = "0000:09:00.0".into();
        assert_eq!(
            pick_adapter(
                &list,
                &GpuAdapterChoice::Matching(id.clone()),
                None,
                wgpu::PowerPreference::None
            ),
            (Some(1), false)
        );
        id.name = "Renamed".into();
        assert!(!id.matches(&list[1]));
    }

    #[test]
    fn a_missing_choice_falls_back_to_automatic_and_says_so() {
        let mut id = adapters()[1].id();
        id.device = 99;
        assert_eq!(
            pick_adapter(
                &adapters(),
                &GpuAdapterChoice::Matching(id),
                None,
                wgpu::PowerPreference::HighPerformance
            ),
            (Some(1), true)
        );
    }

    #[test]
    fn automatic_prefers_hardware_with_view_formats_over_incomplete_gl_or_cpu() {
        let infos = vec![
            adapter("llvmpipe", 1, GpuDeviceType::Cpu, ""),
            adapter("Mesa GLES", 2, GpuDeviceType::IntegratedGpu, ""),
            adapter("RADV Vulkan", 3, GpuDeviceType::DiscreteGpu, "0000:01:00.0"),
        ];
        let summaries = vec![
            AdapterCapabilitySummary {
                backend: GpuBackend::Gl,
                supports_view_formats: false,
                has_renderable_surface_format: true,
            },
            AdapterCapabilitySummary {
                backend: GpuBackend::Gl,
                supports_view_formats: false,
                has_renderable_surface_format: true,
            },
            AdapterCapabilitySummary {
                backend: GpuBackend::Vulkan,
                supports_view_formats: true,
                has_renderable_surface_format: true,
            },
        ];
        assert_eq!(
            automatic_adapter_with_capabilities(
                &infos,
                &summaries,
                None,
                wgpu::PowerPreference::None,
            ),
            Some(2)
        );

        let unrenderable = vec![
            AdapterCapabilitySummary {
                backend: GpuBackend::Vulkan,
                supports_view_formats: true,
                has_renderable_surface_format: false,
            };
            3
        ];
        assert_eq!(
            automatic_adapter_with_capabilities(
                &infos,
                &unrenderable,
                None,
                wgpu::PowerPreference::HighPerformance,
            ),
            None
        );
    }
}
