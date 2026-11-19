//! `cargo run -p task-workbench`

use nana_ui::{
    ApplicationIdentity, DiagnosticsConfig, NanaApplication, RuntimeApplication, WindowDescriptor,
};
use task_workbench::WorkbenchApp;

fn main() -> Result<(), nana_ui::HostedRunError> {
    let identity = ApplicationIdentity::new(
        "dev.nanaui.task-workbench",
        "NanaUI Task Workbench",
        env!("CARGO_PKG_VERSION"),
    );
    NanaApplication::builder(identity)
        .diagnostics(DiagnosticsConfig::default())
        .run::<RuntimeApplication<WorkbenchApp>>(
            WindowDescriptor::new("NanaUI Task Workbench").initial_size(860.0, 620.0),
        )
}
