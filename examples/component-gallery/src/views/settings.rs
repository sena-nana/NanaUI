use nana_ui::runtime::AboutMetadata;

pub(super) const WORKSPACE_SETTINGS_TITLE: &str = "工作区布局";
pub(super) const WORKSPACE_SETTINGS_HINT: &str = "侧边栏宽度与区域可见状态";
pub(super) const WORKSPACE_SETTINGS_DETAILS: &str =
    "恢复默认布局会重置当前工作区的区域尺寸与可见状态。";
pub(super) const WORKSPACE_SETTINGS_RESET: &str = "恢复默认";

pub(super) fn gallery_about_metadata() -> AboutMetadata {
    AboutMetadata::new("NanaUI Component Gallery", env!("CARGO_PKG_VERSION"))
        .description("Rust 原生 UI 组件库与工作区框架")
}
