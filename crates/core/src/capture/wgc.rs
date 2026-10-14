//! Windows.Graphics.Capture backend (Windows 10 1903+): captures a whole monitor through the compositor,
//! which also sees hardware-accelerated content that GDI misses.

use std::time::{Duration, Instant};

use image::RgbaImage;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::{IInspectable, Interface};

use super::CaptureError;

fn e(what: &str, err: windows::core::Error) -> CaptureError {
    CaptureError::Backend(format!("WGC: {what}: {}", err.message()))
}

/// A D3D11 device reused for several monitor grabs.
pub struct Wgc {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
}

impl Wgc {
    pub fn new() -> Result<Wgc, CaptureError> {
        if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
            return Err(CaptureError::Backend("WGC: not supported on this system".into()));
        }
        // SAFETY: COM/WinRT initialisation and plain D3D11 device creation; outputs are checked.
        unsafe {
            // already-initialised (S_FALSE) or a different apartment model are both fine for free-threaded capture
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
            let mut device = None;
            let mut ctx = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
            .map_err(|x| e("D3D11CreateDevice", x))?;
            let (device, ctx) = device
                .zip(ctx)
                .ok_or_else(|| CaptureError::Backend("WGC: no D3D device".into()))?;
            let dxgi: IDXGIDevice = device.cast().map_err(|x| e("IDXGIDevice", x))?;
            let inspectable: IInspectable =
                CreateDirect3D11DeviceFromDXGIDevice(&dxgi).map_err(|x| e("CreateDirect3D11Device", x))?;
            let winrt_device: IDirect3DDevice = inspectable.cast().map_err(|x| e("IDirect3DDevice", x))?;
            Ok(Wgc {
                device,
                ctx,
                winrt_device,
            })
        }
    }

    /// Grabs the full monitor (`handle` = HMONITOR). The image has the monitor's pixel size.
    pub fn grab_monitor(&self, handle: isize, cursor: bool) -> Result<RgbaImage, CaptureError> {
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|x| e("capture item factory", x))?;
        // SAFETY: `handle` comes from EnumDisplayMonitors.
        let item: GraphicsCaptureItem =
            unsafe { interop.CreateForMonitor(HMONITOR(handle as *mut _)) }.map_err(|x| e("CreateForMonitor", x))?;
        let size = item.Size().map_err(|x| e("item size", x))?;

        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &self.winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            size,
        )
        .map_err(|x| e("frame pool", x))?;
        let session = pool.CreateCaptureSession(&item).map_err(|x| e("capture session", x))?;
        let _ = session.SetIsCursorCaptureEnabled(cursor);
        // no yellow frame where Windows allows it; older systems simply refuse
        let _ = session.SetIsBorderRequired(false);

        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let _token = pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
            let _ = tx.send(());
            Ok(())
        }));
        session.StartCapture().map_err(|x| e("StartCapture", x))?;

        let deadline = Instant::now() + Duration::from_millis(1500);
        let result = loop {
            if let Ok(frame) = pool.TryGetNextFrame() {
                break self.read_frame(&frame, size.Width as u32, size.Height as u32);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break Err(CaptureError::Backend("WGC: no frame within 1.5 s".into()));
            }
            let _ = rx.recv_timeout(left.min(Duration::from_millis(50)));
        };
        let _ = session.Close();
        let _ = pool.Close();
        result
    }

    fn read_frame(
        &self,
        frame: &windows::Graphics::Capture::Direct3D11CaptureFrame,
        want_w: u32,
        want_h: u32,
    ) -> Result<RgbaImage, CaptureError> {
        let surface = frame.Surface().map_err(|x| e("frame surface", x))?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(|x| e("surface access", x))?;
        // SAFETY: the texture belongs to our device; staging copy is mapped for reading and unmapped before return.
        unsafe {
            let tex: ID3D11Texture2D = access.GetInterface().map_err(|x| e("texture", x))?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            tex.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            desc.MiscFlags = 0;
            let mut staging = None;
            self.device
                .CreateTexture2D(&desc, None, Some(&mut staging))
                .map_err(|x| e("staging texture", x))?;
            let staging: ID3D11Texture2D =
                staging.ok_or_else(|| CaptureError::Backend("WGC: no staging texture".into()))?;
            self.ctx.CopyResource(&staging, &tex);

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|x| e("Map", x))?;
            let (w, h) = (desc.Width.min(want_w.max(1)), desc.Height.min(want_h.max(1)));
            let mut out = vec![0u8; w as usize * h as usize * 4];
            for y in 0..h as usize {
                let src = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    w as usize * 4,
                );
                let dst = &mut out[y * w as usize * 4..(y + 1) * w as usize * 4];
                for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = 255;
                }
            }
            self.ctx.Unmap(&staging, 0);
            RgbaImage::from_raw(w, h, out).ok_or_else(|| CaptureError::Backend("WGC: buffer size".into()))
        }
    }
}
