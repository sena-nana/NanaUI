//! WebView2 hosted on a Windows.UI.Composition visual, captured with
//! Windows.Graphics.Capture.
//!
//! Each surface owns one STA thread with its own message loop: the WebView2
//! environment, the composition controller and the interaction window live
//! there, so the application's UI thread never pumps WebView2 messages. Frames
//! arrive on the free-threaded capture pool's worker thread, are copied
//! through a staging texture and reach the sink from there.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    CreateCoreWebView2CompositionControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, CursorChangedEventHandler,
    DocumentTitleChangedEventHandler, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler, ProcessFailedEventHandler,
    SourceChangedEventHandler, take_pwstr,
};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{Compositor, ContainerVisual, Visual};
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DQTAT_COM_STA, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
    DestroyWindow, DispatchMessageW, GetMessageW, HCURSOR, HTCLIENT, MSG, PM_NOREMOVE,
    PeekMessageW, PostThreadMessageW, RegisterClassExW, SW_HIDE, SW_SHOWNORMAL, SWP_NOMOVE,
    SWP_NOZORDER, SetCursor, SetForegroundWindow, SetWindowPos, SetWindowTextW, ShowWindow,
    TranslateMessage, WM_APP, WM_CLOSE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR,
    WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSEXW, WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{BOOL, HSTRING, IInspectable, Interface, PCWSTR, PWSTR, Ref, w};
use windows_numerics::Vector2;

use super::{
    SurfaceEvents, WebFrame, WebFrameSink, WebSurfaceCommand, WebSurfaceCompletion, WebSurfaceDesc,
    WebSurfaceEvent, WebSurfaceWake, capture_interval,
};
use crate::{BrowserPolicy, BrowserState};

enum Request {
    Configure(WebSurfaceDesc),
    Command(Option<WebSurfaceCommand>),
    Close,
}

/// State both threads see.
struct Shared {
    events: Mutex<SurfaceEvents>,
    revision: AtomicU64,
    wake: WebSurfaceWake,
}

impl Shared {
    fn publish(&self, event: WebSurfaceEvent) {
        let revision = self.revision.load(Ordering::Acquire);
        if let Ok(mut events) = self.events.lock() {
            events.publish(revision, event);
        }
        (self.wake)();
    }
}

pub(super) struct PlatformSurface {
    requests: Sender<Request>,
    thread_id: u32,
    shared: Arc<Shared>,
    policy: BrowserPolicy,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PlatformSurface {
    pub(super) fn new(
        policy: BrowserPolicy,
        desc: WebSurfaceDesc,
        frames: WebFrameSink,
        wake: WebSurfaceWake,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            events: Mutex::default(),
            revision: AtomicU64::new(0),
            wake,
        });
        let (requests, receiver) = std::sync::mpsc::channel();
        let (started, thread_id) = std::sync::mpsc::sync_channel(1);
        let engine_shared = shared.clone();
        let engine_policy = policy.clone();
        let thread = std::thread::Builder::new()
            .name("nana-web-surface".into())
            .spawn(move || {
                // The queue must exist before the owner may post to it.
                let mut message = MSG::default();
                unsafe {
                    let _ = PeekMessageW(&mut message, None, WM_APP, WM_APP, PM_NOREMOVE);
                }
                let _ = started.send(unsafe { GetCurrentThreadId() });
                run(engine_policy, desc, frames, engine_shared, receiver);
            })
            .map_err(|error| error.to_string())?;
        let thread_id = thread_id
            .recv()
            .map_err(|_| "网页画面线程没有启动".to_string())?;
        Ok(Self {
            requests,
            thread_id,
            shared,
            policy,
            thread: Some(thread),
        })
    }

    fn send(&self, request: Request) {
        if self.requests.send(request).is_ok() {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_APP, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub(super) fn configure(&mut self, desc: WebSurfaceDesc) {
        self.send(Request::Configure(desc));
    }

    pub(super) fn command(
        &mut self,
        revision: u64,
        command: Option<&WebSurfaceCommand>,
    ) -> Result<(), String> {
        if let Some(WebSurfaceCommand::Navigate(url)) = command
            && !self.policy.allows(url)
        {
            return Err("不允许打开此地址".into());
        }
        self.shared.revision.store(revision, Ordering::Release);
        self.send(Request::Command(command.cloned()));
        Ok(())
    }

    pub(super) fn take_events(&mut self) -> Vec<WebSurfaceCompletion> {
        self.shared
            .events
            .lock()
            .map(|mut events| events.take())
            .unwrap_or_default()
    }
}

impl Drop for PlatformSurface {
    fn drop(&mut self) {
        if let Ok(mut events) = self.shared.events.lock() {
            events.close();
        }
        self.send(Request::Close);
        // Teardown releases the engine; it does not wait on the application.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Copies captured textures into frames on the capture pool's worker thread.
struct Capturer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    sink: WebFrameSink,
    interval: Option<Duration>,
    last: Option<Instant>,
    size: [u32; 2],
    sequence: u64,
}

impl Capturer {
    fn frame_arrived(&mut self, pool: &Direct3D11CaptureFramePool) -> windows::core::Result<()> {
        let frame = pool.TryGetNextFrame()?;
        let Some(interval) = self.interval else {
            return Ok(());
        };
        let now = Instant::now();
        if self.last.is_some_and(|last| now < last + interval) {
            return Ok(());
        }
        self.last = Some(now);
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        let staging = match &self.staging {
            Some((staging, width, height)) if *width == desc.Width && *height == desc.Height => {
                staging.clone()
            }
            _ => {
                let mut staging_desc = desc;
                staging_desc.Usage = D3D11_USAGE_STAGING;
                staging_desc.BindFlags = 0;
                staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                staging_desc.MiscFlags = 0;
                staging_desc.MipLevels = 1;
                staging_desc.ArraySize = 1;
                let mut staging = None;
                unsafe {
                    self.device
                        .CreateTexture2D(&staging_desc, None, Some(&mut staging))?
                };
                let staging = staging.ok_or_else(windows::core::Error::empty)?;
                self.staging = Some((staging.clone(), desc.Width, desc.Height));
                staging
            }
        };
        let [width, height] = self.size;
        let (copy_width, copy_height) = (width.min(desc.Width), height.min(desc.Height));
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        unsafe {
            self.context.CopyResource(&staging, &texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let source = mapped.pData.cast::<u8>();
            for row in 0..copy_height as usize {
                let from = std::slice::from_raw_parts(
                    source.add(row * mapped.RowPitch as usize),
                    copy_width as usize * 4,
                );
                let to = &mut rgba[row * width as usize * 4..][..copy_width as usize * 4];
                // BGRA (premultiplied) to RGBA.
                for (to, from) in to
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .zip(from.as_chunks::<4>().0)
                {
                    to.copy_from_slice(&[from[2], from[1], from[0], from[3]]);
                }
            }
            self.context.Unmap(&staging, 0);
        }
        self.sequence += 1;
        (self.sink)(WebFrame {
            width,
            height,
            rgba: rgba.into(),
            sequence: self.sequence,
        });
        Ok(())
    }
}

struct Capture {
    _item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

struct Engine {
    hwnd: HWND,
    policy: BrowserPolicy,
    shared: Arc<Shared>,
    compositor: Compositor,
    root: ContainerVisual,
    target: RefCell<Option<DesktopWindowTarget>>,
    controller: RefCell<Option<ICoreWebView2Controller>>,
    composition: RefCell<Option<ICoreWebView2CompositionController>>,
    webview: RefCell<Option<ICoreWebView2>>,
    direct3d: IDirect3DDevice,
    capturer: Arc<Mutex<Capturer>>,
    capture: RefCell<Option<Capture>>,
    desc: Cell<WebSurfaceDesc>,
    interactive: Cell<bool>,
    loading: Cell<bool>,
    error: RefCell<Option<String>>,
    cursor: Cell<Option<HCURSOR>>,
}

thread_local! {
    static ENGINE: RefCell<Option<Rc<Engine>>> = const { RefCell::new(None) };
}

fn engine() -> Option<Rc<Engine>> {
    ENGINE.with(|engine| engine.borrow().clone())
}

fn run(
    policy: BrowserPolicy,
    desc: WebSurfaceDesc,
    frames: WebFrameSink,
    shared: Arc<Shared>,
    requests: Receiver<Request>,
) {
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    match start(policy, desc, frames, shared.clone()) {
        Ok(engine) => {
            ENGINE.with(|slot| *slot.borrow_mut() = Some(engine.clone()));
            if let Err(error) = engine.create_webview() {
                engine.fail(error);
            }
            pump(&engine, &requests);
            ENGINE.with(|slot| slot.borrow_mut().take());
            engine.shutdown();
        }
        Err(error) => {
            shared.publish(WebSurfaceEvent::State(BrowserState {
                error: Some(error),
                ..Default::default()
            }));
        }
    }
    if com.is_ok() {
        unsafe { CoUninitialize() };
    }
}

fn pump(engine: &Rc<Engine>, requests: &Receiver<Request>) {
    // Requests sent while the environment was being created are still queued.
    if !engine.drain(requests) {
        return;
    }
    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 <= 0 {
            return;
        }
        if message.hwnd.is_invalid() && message.message == WM_APP {
            if !engine.drain(requests) {
                return;
            }
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn start(
    policy: BrowserPolicy,
    desc: WebSurfaceDesc,
    frames: WebFrameSink,
    shared: Arc<Shared>,
) -> Result<Rc<Engine>, String> {
    let error = |error: windows::core::Error| error.message();
    unsafe {
        // Windows.UI.Composition needs a dispatcher queue on this thread.
        let controller = CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_STA,
        })
        .map_err(error)?;
        std::mem::forget(controller);
        let compositor = Compositor::new().map_err(error)?;
        let root = compositor.CreateContainerVisual().map_err(error)?;
        let instance = GetModuleHandleW(None).map_err(error)?;
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("NanaWebSurface"),
            ..Default::default()
        };
        // A second registration of the same class fails harmlessly.
        RegisterClassExW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            w!("NanaWebSurface"),
            w!(""),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            desc.size[0] as i32,
            desc.size[1] as i32,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(error)?;
        let mut device = None;
        let mut context = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(error)?;
        let device = device.ok_or("无法创建网页画面的图形设备")?;
        let context = context.ok_or("无法创建网页画面的图形设备")?;
        let dxgi: IDXGIDevice = device.cast().map_err(error)?;
        let direct3d: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)
            .and_then(|device: IInspectable| device.cast())
            .map_err(error)?;
        Ok(Rc::new(Engine {
            hwnd,
            policy,
            shared,
            compositor,
            root,
            target: RefCell::new(None),
            controller: RefCell::new(None),
            composition: RefCell::new(None),
            webview: RefCell::new(None),
            direct3d,
            capturer: Arc::new(Mutex::new(Capturer {
                device,
                context,
                staging: None,
                sink: frames,
                interval: capture_interval(desc.max_fps),
                last: None,
                size: desc.frame_size(),
                sequence: 0,
            })),
            capture: RefCell::new(None),
            desc: Cell::new(desc),
            interactive: Cell::new(false),
            loading: Cell::new(false),
            error: RefCell::new(None),
            cursor: Cell::new(None),
        }))
    }
}

fn user_data_folder() -> Option<HSTRING> {
    // The default folder sits next to the executable, which an installed app
    // cannot write to.
    let base = std::env::var_os("LOCALAPPDATA")?;
    let name = std::env::current_exe()
        .ok()?
        .file_stem()?
        .to_string_lossy()
        .into_owned();
    let path = std::path::Path::new(&base).join(name).join("WebView2");
    Some(HSTRING::from(path.as_os_str()))
}

fn webview2_error(error: webview2_com::Error) -> String {
    match error {
        webview2_com::Error::WindowsError(error) => {
            if error.code().0 as u32 == 0x8007_0002 {
                "没有找到 WebView2 运行时，请安装 Microsoft Edge WebView2 Runtime".into()
            } else {
                error.message()
            }
        }
        other => other.to_string(),
    }
}

/// An event handler that only re-reads the page's state.
fn republish<A, B>(
    engine: std::rc::Weak<Engine>,
) -> impl FnMut(A, B) -> windows::core::Result<()> + 'static {
    move |_, _| {
        if let Some(engine) = engine.upgrade() {
            engine.publish_state();
        }
        Ok(())
    }
}

impl Engine {
    fn create_webview(self: &Rc<Self>) -> Result<(), String> {
        let folder = user_data_folder();
        let (sender, receiver) = std::sync::mpsc::channel();
        CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                let folder = folder
                    .as_ref()
                    .map_or(PCWSTR::null(), |folder| PCWSTR(folder.as_ptr()));
                CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), folder, None, &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(move |result, environment| {
                result?;
                let _ = sender.send(environment);
                Ok(())
            }),
        )
        .map_err(webview2_error)?;
        let environment = receiver.recv().ok().flatten().ok_or("无法启动 WebView2")?;
        let environment: ICoreWebView2Environment3 =
            environment.cast().map_err(|error| error.message())?;
        let (sender, receiver) = std::sync::mpsc::channel();
        let hwnd = self.hwnd;
        CreateCoreWebView2CompositionControllerCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                environment
                    .CreateCoreWebView2CompositionController(hwnd, &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(move |result, controller| {
                result?;
                let _ = sender.send(controller);
                Ok(())
            }),
        )
        .map_err(webview2_error)?;
        let composition = receiver
            .recv()
            .ok()
            .flatten()
            .ok_or("无法创建 WebView2 视图")?;
        let message = |error: windows::core::Error| error.message();
        unsafe {
            let controller: ICoreWebView2Controller = composition.cast().map_err(message)?;
            composition
                .SetRootVisualTarget(&self.root.cast::<Visual>().map_err(message)?)
                .map_err(message)?;
            controller.SetIsVisible(true).map_err(message)?;
            if let Ok(scaled) = controller.cast::<ICoreWebView2Controller3>() {
                let _ = scaled.SetShouldDetectMonitorScaleChanges(false);
            }
            let webview = controller.CoreWebView2().map_err(message)?;
            self.subscribe(&webview, &composition)?;
            *self.controller.borrow_mut() = Some(controller);
            *self.composition.borrow_mut() = Some(composition);
            *self.webview.borrow_mut() = Some(webview);
        }
        self.apply(self.desc.get(), true);
        Ok(())
    }

    fn subscribe(
        self: &Rc<Self>,
        webview: &ICoreWebView2,
        composition: &ICoreWebView2CompositionController,
    ) -> Result<(), String> {
        let message = |error: windows::core::Error| error.message();
        let mut token = 0i64;
        let weak = Rc::downgrade(self);
        unsafe {
            let this = weak.clone();
            webview
                .add_NavigationStarting(
                    &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                        let (Some(this), Some(args)) = (this.upgrade(), args) else {
                            return Ok(());
                        };
                        let mut uri = PWSTR::null();
                        args.Uri(&mut uri)?;
                        let uri = take_pwstr(uri);
                        if this.policy.allows(&uri) {
                            this.loading.set(true);
                            this.error.borrow_mut().take();
                        } else {
                            args.SetCancel(true)?;
                            *this.error.borrow_mut() = Some("不允许打开此地址".into());
                        }
                        this.publish_state();
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(message)?;
            let this = weak.clone();
            webview
                .add_NavigationCompleted(
                    &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                        let (Some(this), Some(args)) = (this.upgrade(), args) else {
                            return Ok(());
                        };
                        this.loading.set(false);
                        let mut success = BOOL::default();
                        args.IsSuccess(&mut success)?;
                        let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                        args.WebErrorStatus(&mut status)?;
                        if !success.as_bool()
                            && status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED
                        {
                            *this.error.borrow_mut() =
                                Some(format!("网页加载失败（{}）", status.0));
                        }
                        this.publish_state();
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(message)?;
            webview
                .add_SourceChanged(
                    &SourceChangedEventHandler::create(Box::new(republish(weak.clone()))),
                    &mut token,
                )
                .map_err(message)?;
            webview
                .add_DocumentTitleChanged(
                    &DocumentTitleChangedEventHandler::create(Box::new(republish(weak.clone()))),
                    &mut token,
                )
                .map_err(message)?;
            let this = weak.clone();
            webview
                .add_NewWindowRequested(
                    &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                        // Pages that open a new window navigate this one instead.
                        let (Some(this), Some(args)) = (this.upgrade(), args) else {
                            return Ok(());
                        };
                        args.SetHandled(true)?;
                        let mut uri = PWSTR::null();
                        args.Uri(&mut uri)?;
                        let uri = take_pwstr(uri);
                        if this.policy.allows(&uri)
                            && let Some(webview) = this.webview.borrow().as_ref()
                        {
                            webview.Navigate(&HSTRING::from(uri))?;
                        }
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(message)?;
            let this = weak.clone();
            webview
                .add_ProcessFailed(
                    &ProcessFailedEventHandler::create(Box::new(move |_, _| {
                        if let Some(this) = this.upgrade() {
                            *this.error.borrow_mut() = Some("网页进程已退出，正在重新加载".into());
                            this.publish_state();
                            if let Some(webview) = this.webview.borrow().as_ref() {
                                let _ = webview.Reload();
                            }
                        }
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(message)?;
            let this = weak.clone();
            composition
                .add_CursorChanged(
                    &CursorChangedEventHandler::create(Box::new(move |sender, _| {
                        if let (Some(this), Some(sender)) = (this.upgrade(), sender) {
                            let mut cursor = HCURSOR::default();
                            sender.Cursor(&mut cursor)?;
                            this.cursor.set(Some(cursor));
                            if this.interactive.get() {
                                SetCursor(Some(cursor));
                            }
                        }
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(message)?;
        }
        Ok(())
    }

    fn fail(&self, error: String) {
        *self.error.borrow_mut() = Some(error);
        self.publish_state();
    }

    fn publish_state(&self) {
        let mut state = BrowserState {
            attached: self.webview.borrow().is_some(),
            loading: self.loading.get(),
            error: self.error.borrow().clone(),
            ..Default::default()
        };
        if let Some(webview) = self.webview.borrow().as_ref() {
            unsafe {
                let mut value = PWSTR::null();
                if webview.Source(&mut value).is_ok() {
                    state.url = take_pwstr(value);
                }
                let mut value = PWSTR::null();
                if webview.DocumentTitle(&mut value).is_ok() {
                    state.title = take_pwstr(value);
                }
            }
        }
        self.shared.publish(WebSurfaceEvent::State(state));
    }

    /// Size, scale, background and capture rate; `force` re-applies all of them.
    fn apply(&self, desc: WebSurfaceDesc, force: bool) {
        let previous = self.desc.replace(desc);
        let frame = desc.frame_size();
        let resized = force || previous.frame_size() != frame || previous.scale != desc.scale;
        if let Some(controller) = self.controller.borrow().as_ref() {
            unsafe {
                if resized {
                    let _ = controller.SetBounds(RECT {
                        left: 0,
                        top: 0,
                        right: frame[0] as i32,
                        bottom: frame[1] as i32,
                    });
                    if let Ok(scaled) = controller.cast::<ICoreWebView2Controller3>() {
                        let _ = scaled.SetRasterizationScale(desc.scale as f64);
                    }
                }
                if (force || previous.transparent != desc.transparent)
                    && let Ok(colored) = controller.cast::<ICoreWebView2Controller2>()
                {
                    let alpha = if desc.transparent { 0 } else { 255 };
                    let _ = colored.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
                        A: alpha,
                        R: 255,
                        G: 255,
                        B: 255,
                    });
                }
            }
        }
        if let Ok(mut capturer) = self.capturer.lock() {
            capturer.interval = capture_interval(desc.max_fps);
            capturer.size = frame;
        }
        if resized {
            let _ = self.root.SetSize(Vector2 {
                X: frame[0] as f32,
                Y: frame[1] as f32,
            });
            self.capture.borrow_mut().take();
            if let Err(error) = self.start_capture(frame) {
                self.fail(error.message());
            }
            if self.interactive.get() {
                self.fit_window();
            }
        }
    }

    fn start_capture(&self, [width, height]: [u32; 2]) -> windows::core::Result<()> {
        let item = GraphicsCaptureItem::CreateFromVisual(&self.root.cast::<Visual>()?)?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &self.direct3d,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            SizeInt32 {
                Width: width as i32,
                Height: height as i32,
            },
        )?;
        let capturer = self.capturer.clone();
        pool.FrameArrived(&TypedEventHandler::new(
            move |pool: Ref<Direct3D11CaptureFramePool>, _| {
                if let (Some(pool), Ok(mut capturer)) = (pool.as_ref(), capturer.lock()) {
                    let _ = capturer.frame_arrived(pool);
                }
                Ok(())
            },
        ))?;
        let session = pool.CreateCaptureSession(&item)?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        let _ = session.SetIsBorderRequired(false);
        session.StartCapture()?;
        *self.capture.borrow_mut() = Some(Capture {
            _item: item,
            pool,
            session,
        });
        Ok(())
    }

    fn fit_window(&self) {
        let [width, height] = self.desc.get().frame_size();
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        unsafe {
            let _ = AdjustWindowRectEx(&mut rect, WS_OVERLAPPEDWINDOW, false, WS_EX_APPWINDOW);
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOMOVE | SWP_NOZORDER,
            );
        }
    }

    fn show(&self, title: &str) {
        if self.target.borrow().is_none() {
            let target = self
                .compositor
                .cast::<ICompositorDesktopInterop>()
                .and_then(|interop| unsafe { interop.CreateDesktopWindowTarget(self.hwnd, false) })
                .and_then(|target| {
                    target.SetRoot(&self.root.cast::<Visual>()?)?;
                    Ok(target)
                });
            match target {
                Ok(target) => *self.target.borrow_mut() = Some(target),
                Err(error) => {
                    self.fail(error.message());
                    return;
                }
            }
        }
        self.interactive.set(true);
        self.fit_window();
        unsafe {
            let _ = SetWindowTextW(self.hwnd, &HSTRING::from(title));
            let _ = ShowWindow(self.hwnd, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(self.hwnd);
            if let Some(controller) = self.controller.borrow().as_ref() {
                let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
            }
        }
    }

    fn hide(&self) {
        if self.interactive.replace(false) {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.shared.publish(WebSurfaceEvent::WindowClosed);
        }
    }

    /// Returns false once the owner closed the surface.
    fn drain(&self, requests: &Receiver<Request>) -> bool {
        while let Ok(request) = requests.try_recv() {
            match request {
                Request::Close => return false,
                Request::Configure(desc) => self.apply(desc, false),
                Request::Command(command) => {
                    let webview = self.webview.borrow().clone();
                    match (command, webview) {
                        (None, _) => {}
                        (Some(WebSurfaceCommand::Navigate(url)), Some(webview)) => unsafe {
                            self.error.borrow_mut().take();
                            if let Err(error) = webview.Navigate(&HSTRING::from(url)) {
                                *self.error.borrow_mut() = Some(error.message());
                            }
                        },
                        (Some(WebSurfaceCommand::Reload), Some(webview)) => unsafe {
                            self.error.borrow_mut().take();
                            let _ = webview.Reload();
                        },
                        (Some(WebSurfaceCommand::ShowWindow { title }), Some(_)) => {
                            self.show(&title)
                        }
                        (Some(WebSurfaceCommand::HideWindow), _) => self.hide(),
                        (Some(_), None) => {}
                    }
                    self.publish_state();
                }
            }
        }
        true
    }

    fn send_mouse(&self, message: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
        let Some(composition) = self.composition.borrow().clone() else {
            return false;
        };
        let x = (lparam.0 & 0xffff) as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
        let mut point = POINT { x, y };
        let mut data = 0u32;
        if matches!(message, WM_MOUSEWHEEL | WM_MOUSEHWHEEL) {
            // Wheel positions are in screen coordinates.
            unsafe {
                let _ = ScreenToClient(self.hwnd, &mut point);
            }
            data = ((wparam.0 >> 16) & 0xffff) as i16 as i32 as u32;
        } else if matches!(message, WM_XBUTTONDOWN | WM_XBUTTONUP) {
            data = ((wparam.0 >> 16) & 0xffff) as u32;
        }
        unsafe {
            match message {
                WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
                    SetCapture(self.hwnd);
                }
                WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => {
                    let _ = ReleaseCapture();
                }
                WM_MOUSEMOVE => {
                    // Ask for the WM_MOUSELEAVE the page needs to clear hover.
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut track);
                }
                _ => {}
            }
            let _ = composition.SendMouseInput(
                COREWEBVIEW2_MOUSE_EVENT_KIND(message as i32),
                COREWEBVIEW2_MOUSE_EVENT_VIRTUAL_KEYS((wparam.0 & 0xffff) as i32),
                data,
                point,
            );
        }
        true
    }

    fn shutdown(&self) {
        self.capture.borrow_mut().take();
        if let Some(controller) = self.controller.borrow_mut().take() {
            unsafe {
                let _ = controller.Close();
            }
        }
        self.composition.borrow_mut().take();
        self.webview.borrow_mut().take();
        self.target.borrow_mut().take();
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(engine) = engine()
        && engine.hwnd == hwnd
    {
        match message {
            WM_CLOSE => {
                engine.hide();
                return LRESULT(0);
            }
            WM_SETCURSOR if (lparam.0 & 0xffff) as u32 == HTCLIENT => {
                if let Some(cursor) = engine.cursor.get() {
                    unsafe { SetCursor(Some(cursor)) };
                    return LRESULT(1);
                }
            }
            WM_MOUSEMOVE | WM_MOUSELEAVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN
            | WM_RBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_XBUTTONDOWN | WM_XBUTTONUP
            | WM_MOUSEWHEEL | WM_MOUSEHWHEEL
                if engine.send_mouse(message, wparam, lparam) =>
            {
                return LRESULT(0);
            }
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
