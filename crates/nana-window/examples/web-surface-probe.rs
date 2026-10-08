//! Renders an animated page through a headless WebSurface and reports what
//! actually arrived: frame rate, whether consecutive frames differ (the page
//! keeps running offscreen), and how much of the transparent page stayed
//! transparent. Saves the last frame as `web-surface-probe.png`.
//!
//! `cargo run -p nana-window --example web-surface-probe [-- --window]`
//! With `--window` the page is also shown in its interaction window for a few
//! seconds; frames must keep flowing while it is open.
//!
//! For cost measurements: `--seconds N`, `--size WxH`, `--fps N`, and
//! `--static` for a page that never changes; measure the probe and the web
//! engine's processes with `ps`.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nana_window::{
    BrowserPolicy, WebFrame, WebSurface, WebSurfaceCommand, WebSurfaceDesc, web_surface_support,
};

const STATIC_PAGE: &str = r#"<!doctype html><html><body style="margin:0;background:transparent">
<div style="font:64px sans-serif;color:#e33;background:rgba(0,0,0,.5);width:400px">static</div>
</body></html>"#;

const PAGE: &str = r#"<!doctype html><html><body style="margin:0;background:transparent">
<div id=n style="font:64px sans-serif;color:#e33">0</div>
<div style="width:120px;height:120px;border-radius:60px;background:#3c6;animation:m 1s linear infinite alternate"></div>
<style>@keyframes m{from{transform:translateX(0)}to{transform:translateX(480px)}}</style>
<script>let n=0;(function f(){document.getElementById('n').textContent=++n;requestAnimationFrame(f)})()</script>
</body></html>"#;

fn serve(page: &'static str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = format!("http://{}/", listener.local_addr().expect("address"));
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buffer = [0u8; 2048];
            let _ = stream.read(&mut buffer);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                page.len()
            );
        }
    });
    address
}

fn pump(duration: Duration) {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            static kCFRunLoopDefaultMode: *const std::ffi::c_void;
            fn CFRunLoopRunInMode(
                mode: *const std::ffi::c_void,
                seconds: f64,
                return_after_source_handled: u8,
            ) -> i32;
        }
        let end = Instant::now() + duration;
        while Instant::now() < end {
            unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, 0) };
        }
    }
    #[cfg(not(target_os = "macos"))]
    std::thread::sleep(duration);
}

#[derive(Default)]
struct Received {
    frames: Vec<(Instant, u64)>,
    last: Option<WebFrame>,
}

fn main() -> std::process::ExitCode {
    if !web_surface_support() {
        println!("no headless web engine on this platform");
        return std::process::ExitCode::SUCCESS;
    }
    let args: Vec<String> = std::env::args().collect();
    let option = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|index| args.get(index + 1))
            .cloned()
    };
    let show_window = args.iter().any(|arg| arg == "--window");
    let still = args.iter().any(|arg| arg == "--static");
    let seconds: u64 = option("--seconds")
        .and_then(|value| value.parse().ok())
        .unwrap_or(4);
    let size = option("--size")
        .and_then(|value| {
            let (width, height) = value.split_once('x')?;
            Some([width.parse().ok()?, height.parse().ok()?])
        })
        .unwrap_or([960, 540]);
    let received = Arc::new(Mutex::new(Received::default()));
    let sink = received.clone();
    let desc = WebSurfaceDesc {
        size,
        max_fps: option("--fps")
            .and_then(|value| value.parse().ok())
            .unwrap_or(30),
        ..WebSurfaceDesc::default()
    };
    let mut surface = match WebSurface::new(
        BrowserPolicy { allow_web: true },
        desc,
        Arc::new(move |frame: WebFrame| {
            let digest = frame.rgba.iter().step_by(97).fold(0u64, |hash, byte| {
                hash.wrapping_mul(31).wrapping_add(*byte as u64)
            });
            let mut received = sink.lock().unwrap();
            received.frames.push((Instant::now(), digest));
            received.last = Some(frame);
        }),
        Arc::new(|| {}),
    ) {
        Ok(surface) => surface,
        Err(error) => {
            eprintln!("FAIL: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let address = serve(if still { STATIC_PAGE } else { PAGE });
    surface
        .command(1, Some(&WebSurfaceCommand::Navigate(address)))
        .expect("navigate");
    pump(Duration::from_secs(1));
    received.lock().unwrap().frames.clear();
    let started = Instant::now();
    if show_window {
        surface
            .command(
                2,
                Some(&WebSurfaceCommand::ShowWindow {
                    title: "WebSurface probe".into(),
                }),
            )
            .expect("show");
    }
    pump(Duration::from_secs(seconds));
    let elapsed = started.elapsed().as_secs_f64();
    if show_window {
        surface
            .command(3, Some(&WebSurfaceCommand::HideWindow))
            .expect("hide");
        pump(Duration::from_millis(200));
    }
    for event in surface.take_events() {
        println!("event r{}: {:?}", event.revision, event.event);
    }
    let received = received.lock().unwrap();
    let count = received.frames.len();
    let mut digests: Vec<_> = received.frames.iter().map(|(_, digest)| *digest).collect();
    digests.dedup();
    println!(
        "frames {count} in {elapsed:.1}s ({:.1} fps), {} changed between consecutive frames",
        count as f64 / elapsed,
        digests.len()
    );
    let Some(frame) = received.last.as_ref() else {
        eprintln!("FAIL: no frame arrived");
        return std::process::ExitCode::FAILURE;
    };
    let transparent = frame
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[3] == 0)
        .count();
    println!(
        "last frame {}x{} #{}, {:.0}% transparent",
        frame.width,
        frame.height,
        frame.sequence,
        transparent as f64 * 100.0 / (frame.width * frame.height) as f64
    );
    let file = std::fs::File::create("web-surface-probe.png").expect("create png");
    let mut encoder = png::Encoder::new(file, frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&frame.rgba))
        .expect("write png");
    if still {
        return std::process::ExitCode::SUCCESS;
    }
    if count < 20 || digests.len() < count / 2 {
        eprintln!("FAIL: the page did not keep rendering offscreen");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
