#[path = "hosted_gpu/context.rs"]
mod context;
#[path = "hosted_gpu/panel.rs"]
mod panel;
#[path = "hosted_gpu/performance.rs"]
mod performance;
#[path = "hosted_gpu/runner.rs"]
mod runner;
#[path = "hosted_gpu/scene.rs"]
mod scene;

fn main() -> Result<(), iced_winit::winit::error::EventLoopError> {
    runner::run(std::time::Instant::now())
}
