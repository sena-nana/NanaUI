//! Real URL decoding and GPU output through a host transport, without sockets.
use super::*;
use crate::NanaTextShaper;
use nana_ui_core::{BackgroundImage, BackgroundImageFit, ImageSampling};
use nana_ui_platform::{FetchError, FetchHost, FetchPolicy, FetchRequest, FetchResponse};
use nana_ui_runtime::{LayoutViewport, Stack};
use std::sync::{Arc, Mutex};

const URL: &str = "https://headless-output.invalid/asset.png";

#[derive(Debug)]
struct PngHost {
    policy: FetchPolicy,
    png: Vec<u8>,
    requests: Mutex<Vec<String>>,
}

impl PngHost {
    fn new(rgba: [u8; 4]) -> Arc<Self> {
        let image = image::RgbaImage::from_pixel(32, 24, image::Rgba(rgba));
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        Arc::new(Self {
            policy: FetchPolicy::default()
                .with_allowed_origin("https://headless-output.invalid")
                .unwrap(),
            png: png.into_inner(),
            requests: Mutex::new(Vec::new()),
        })
    }
}

impl FetchHost for PngHost {
    fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        assert_eq!(request.url, URL, "fixture never fetches another origin");
        self.requests.lock().unwrap().push(request.url.clone());
        Ok(FetchResponse {
            url: request.url,
            status: 200,
            status_text: "OK".into(),
            headers: vec![("Content-Type".into(), "image/png".into())],
            body: self.png.clone(),
            redirected: false,
        })
    }

    fn policy(&self) -> &FetchPolicy {
        &self.policy
    }
}

fn scene() -> Arc<UiScene> {
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    document
        .context_mut()
        .create_component(
            id,
            Stack::fill_column(0.0).with_layout(|layout| {
                layout.paint.background_layers = vec![
                    BackgroundImage::url_with_fit(URL, BackgroundImageFit::Stretch)
                        .with_sampling(ImageSampling::Resample),
                ];
            }),
        )
        .unwrap();
    document
        .flush(
            LayoutViewport::new(64.0, 48.0),
            &mut NanaTextShaper::default(),
        )
        .unwrap();
    document.shared_scene()
}

fn draw(driver: &mut PresentationDriver, scene: &UiScene) -> ExternalRenderOutcome {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let outcome = driver
            .render_scene(scene, [64.0, 48.0], 0, [0.0; 4], None, None)
            .unwrap()[0];
        if !matches!(outcome, ExternalRenderOutcome::Deferred) {
            return outcome;
        }
        assert!(
            Instant::now() < deadline,
            "URL presentation made no progress"
        );
        driver.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn settled_pixel(driver: &mut PresentationDriver, scene: &UiScene) -> [u8; 4] {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let wanted = match draw(driver, scene) {
            ExternalRenderOutcome::Submitted {
                content_revision, ..
            }
            | ExternalRenderOutcome::Reused {
                content_revision, ..
            } => content_revision,
            ExternalRenderOutcome::Deferred => unreachable!(),
        };
        driver.poll();
        if !driver.has_pending_image_work()
            && let Ok(frame) = driver.sample(OutputConsumerId(1))
            && frame.content_revision() == wanted
        {
            let pixels = read_pixels(frame.texture().unwrap());
            let center = ((EXTENT[1] / 2 * EXTENT[0] + EXTENT[0] / 2) * 4) as usize;
            return pixels[center..center + 4].try_into().unwrap();
        }
        assert!(Instant::now() < deadline, "HTTP URL image never settled");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn wait_for_wake(driver: &mut PresentationDriver, wakes: &Mutex<Vec<String>>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if wakes.lock().unwrap().iter().any(|key| key == URL) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "headless image completion did not wake its host"
        );
        driver.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn headless_http_images_use_the_current_host_and_wake_before_and_after_configuration() {
    for install_before_configure in [true, false] {
        let scene = scene();
        let blue = PngHost::new([0, 0, 255, 255]);
        let green = PngHost::new([0, 255, 0, 255]);
        let mut driver = PresentationDriver::new(&test_gpu::context());
        let wakes = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&wakes);
        let wake: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |key| {
            observed.lock().unwrap().push(key.to_owned());
        });
        if install_before_configure {
            driver.set_resource_fetch_host(Some(blue.clone()));
            driver.set_image_update_waker(Arc::clone(&wake));
        }
        driver.configure(&[consumer(1)]).unwrap();
        if !install_before_configure {
            driver.set_resource_fetch_host(Some(blue.clone()));
            driver.set_image_update_waker(wake);
        }
        draw(&mut driver, &scene);
        wait_for_wake(&mut driver, &wakes);
        assert_eq!(settled_pixel(&mut driver, &scene), [255, 0, 0, 255]);
        assert!(!blue.requests.lock().unwrap().is_empty());
        wakes.lock().unwrap().clear();
        driver.set_resource_fetch_host(Some(green.clone()));
        draw(&mut driver, &scene);
        wait_for_wake(&mut driver, &wakes);
        assert_eq!(settled_pixel(&mut driver, &scene), [0, 255, 0, 255]);
        assert!(!green.requests.lock().unwrap().is_empty());
        // Installing the same host must preserve settled resources and cache keys.
        driver.set_resource_fetch_host(Some(green.clone()));
        let blue_calls = blue.requests.lock().unwrap().len();
        let green_calls = green.requests.lock().unwrap().len();
        for _ in 0..8 {
            assert!(matches!(
                draw(&mut driver, &scene),
                ExternalRenderOutcome::Reused { .. }
            ));
            assert_eq!(driver.work().output_ui_renders, Some(0));
            assert_eq!(driver.work().output_submissions, Some(0));
            assert_eq!(driver.work().output_gpu_copies, Some(0));
        }
        driver.set_resource_fetch_host(None);
        assert_eq!(
            settled_pixel(&mut driver, &scene),
            [0; 4],
            "removing HTTP egress must withdraw previously loaded pixels"
        );
        for _ in 0..8 {
            assert!(matches!(
                draw(&mut driver, &scene),
                ExternalRenderOutcome::Reused { .. }
            ));
            assert_eq!(driver.work().output_submissions, Some(0));
        }
        assert_eq!(blue.requests.lock().unwrap().len(), blue_calls);
        assert_eq!(green.requests.lock().unwrap().len(), green_calls);
    }
}

#[test]
fn replacing_or_removing_headless_http_egress_cancels_pending_fetch_without_another_frame() {
    use nana_ui_platform::{FetchCancellation, FetchErrorKind};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Debug)]
    struct PendingHost {
        policy: FetchPolicy,
        started: Mutex<Option<FetchCancellation>>,
        cancelled: AtomicBool,
    }
    impl FetchHost for PendingHost {
        fn fetch(&self, _: FetchRequest) -> Result<FetchResponse, FetchError> {
            panic!("remote image loading must use cancellable host transport");
        }

        fn fetch_cancellable(
            &self,
            request: FetchRequest,
            cancellation: FetchCancellation,
        ) -> Result<FetchResponse, FetchError> {
            assert_eq!(request.url, URL);
            *self.started.lock().unwrap() = Some(cancellation.clone());
            let deadline = Instant::now() + Duration::from_secs(30);
            while !cancellation.is_cancelled() {
                if Instant::now() >= deadline {
                    return Err(FetchError::new(
                        FetchErrorKind::Timeout,
                        "fixture was not cancelled",
                    ));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            self.cancelled.store(true, Ordering::Release);
            Err(FetchError::new(
                FetchErrorKind::Cancelled,
                "fixture cancelled",
            ))
        }

        fn policy(&self) -> &FetchPolicy {
            &self.policy
        }
    }

    for replace in [false, true] {
        let host = Arc::new(PendingHost {
            policy: FetchPolicy::default()
                .with_allowed_origin("https://headless-output.invalid")
                .unwrap(),
            started: Mutex::new(None),
            cancelled: AtomicBool::new(false),
        });
        let mut driver = PresentationDriver::new(&test_gpu::context());
        driver.configure(&[consumer(1)]).unwrap();
        driver.set_resource_fetch_host(Some(host.clone()));
        // The UI frame completes while the transport is still waiting.
        draw(&mut driver, &scene());
        drop(completed(&mut driver, OutputConsumerId(1)));
        let deadline = Instant::now() + Duration::from_secs(30);
        let cancellation = loop {
            if let Some(token) = host.started.lock().unwrap().clone() {
                break token;
            }
            assert!(
                Instant::now() < deadline,
                "remote worker did not enter its host"
            );
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(!cancellation.is_cancelled());
        assert!(!host.cancelled.load(Ordering::Acquire));
        let started = Instant::now();
        driver.set_resource_fetch_host(if replace {
            Some(PngHost::new([0, 255, 0, 255]))
        } else {
            None
        });
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "egress retirement blocked the host thread"
        );
        assert!(
            cancellation.is_cancelled(),
            "setter must cancel immediately without a subsequent render or trim"
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        while !host.cancelled.load(Ordering::Acquire) {
            assert!(
                Instant::now() < deadline,
                "retired host did not observe cancellation"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
