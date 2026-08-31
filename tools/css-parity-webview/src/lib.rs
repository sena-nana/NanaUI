//! Optional WebView reference for `nana-css-parity`.
//!
//! Live `wry` measurement requires a display server. Headless CI should skip
//! (`NANA_CSS_PARITY_SKIP_WEBVIEW=1` or absent DISPLAY). Prefer fixture
//! `expected` boxes for default `cargo test`.

use nana_css_parity::{ExpectedBox, FixtureCase, WEBVIEW_MEASURE_JS, parse_webview_boxes};

/// Measure fixture boxes via wry/WKWebView.
///
/// Returns `Err` with a `skip:` prefix when the environment cannot host a WebView.
pub fn measure_webview(case: &FixtureCase) -> Result<Vec<ExpectedBox>, String> {
    if std::env::var_os("NANA_CSS_PARITY_SKIP_WEBVIEW").is_some() {
        return Err("skip: NANA_CSS_PARITY_SKIP_WEBVIEW set".into());
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err("skip: no DISPLAY/WAYLAND_DISPLAY".into());
        }
    }

    measure_with_wry(case)
}

fn measure_with_wry(case: &FixtureCase) -> Result<Vec<ExpectedBox>, String> {
    use std::sync::{Arc, Mutex};

    use tao::event::{Event, StartCause, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::platform::run_return::EventLoopExtRunReturn;
    use tao::window::WindowBuilder;
    use wry::WebViewBuilder;

    let html = nana_css_parity::fixture_to_html(case);
    let result: Arc<Mutex<Option<Result<String, String>>>> = Arc::new(Mutex::new(None));
    let result_set = result.clone();
    let result_ready = result.clone();

    let mut event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("nana-css-parity")
        .with_inner_size(tao::dpi::LogicalSize::new(
            f64::from(case.viewport[0]),
            f64::from(case.viewport[1]),
        ))
        .with_visible(false)
        .build(&event_loop)
        .map_err(|e| format!("skip: window: {e}"))?;

    let webview = WebViewBuilder::new()
        .with_html(html)
        .build(&window)
        .map_err(|e| format!("skip: webview: {e}"))?;

    let mut ticks = 0u32;
    let _ = WEBVIEW_MEASURE_JS;
    event_loop.run_return(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;
        match event {
            Event::NewEvents(StartCause::Init) | Event::MainEventsCleared => {
                ticks = ticks.saturating_add(1);
                if ticks == 10 {
                    let flag = result_set.clone();
                    if let Err(e) =
                        webview.evaluate_script_with_callback(WEBVIEW_MEASURE_JS, move |value| {
                            *flag.lock().unwrap() = Some(Ok(value));
                        })
                    {
                        *result_set.lock().unwrap() = Some(Err(format!("eval: {e}")));
                    }
                }
                if ticks >= 10 && result_ready.lock().unwrap().is_some() {
                    *control_flow = ControlFlow::Exit;
                }
                if ticks > 200 {
                    let mut g = result_ready.lock().unwrap();
                    if g.is_none() {
                        *g = Some(Err("skip: webview measure timeout".into()));
                    }
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    });

    let json = result
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| Err("skip: no webview result".into()))?;
    parse_webview_boxes(&json)
}
