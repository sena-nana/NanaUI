#[path = "ui_snapshots/render.rs"]
mod render;
#[path = "ui_snapshots/write.rs"]
mod write;

fn main() {
    for path in render::generate().expect("UI snapshots must render") {
        println!("{}", path.display());
    }
}
