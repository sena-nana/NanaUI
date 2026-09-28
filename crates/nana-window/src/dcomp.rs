//! DirectComposition helpers shared by the splash and the shadow companion.

use windows::Win32::Foundation::{HMODULE, POINT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::DirectComposition::{IDCompositionDevice, IDCompositionSurface};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM,
};

/// A short-lived D3D11 device for DirectComposition surface uploads: hardware
/// if there is one, WARP otherwise. It never touches wgpu's adapter.
pub(crate) fn d3d_device() -> windows::core::Result<ID3D11Device> {
    let create = |driver: D3D_DRIVER_TYPE| {
        let mut device = None;
        // SAFETY: out-pointer to a local; no adapter, default feature levels.
        unsafe {
            D3D11CreateDevice(
                None::<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        }
        .map(|()| device)
    };
    match create(D3D_DRIVER_TYPE_HARDWARE) {
        Ok(Some(device)) => Ok(device),
        _ => create(D3D_DRIVER_TYPE_WARP)?
            .ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL)),
    }
}

pub(crate) fn upload(
    device: &IDCompositionDevice,
    context: &ID3D11DeviceContext,
    width: u32,
    height: u32,
    bgra: &[u8],
) -> windows::core::Result<IDCompositionSurface> {
    // SAFETY: plain COM calls on live objects; `bgra` holds width*height rows.
    unsafe {
        let surface = device.CreateSurface(
            width,
            height,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            DXGI_ALPHA_MODE_PREMULTIPLIED,
        )?;
        let mut offset = POINT::default();
        let texture: ID3D11Texture2D = surface.BeginDraw(None, &mut offset)?;
        let region = D3D11_BOX {
            left: offset.x as u32,
            top: offset.y as u32,
            front: 0,
            right: offset.x as u32 + width,
            bottom: offset.y as u32 + height,
            back: 1,
        };
        context.UpdateSubresource(
            &texture,
            0,
            Some(&raw const region),
            bgra.as_ptr().cast(),
            width * 4,
            0,
        );
        surface.EndDraw()?;
        Ok(surface)
    }
}
