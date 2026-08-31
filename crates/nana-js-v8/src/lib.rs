//! V8 backend via crates.io [`v8`] `152.2.0` (rusty_v8 successor).
//!
//! Enable feature `engine` to compile and link V8. Default workspace builds stay
//! free of the V8 download/link cost. Do not pin legacy `rusty_v8 = "0.32.1"`.

#[cfg(feature = "engine")]
mod engine;

#[cfg(feature = "engine")]
pub use engine::V8Engine;

#[cfg(not(feature = "engine"))]
mod stub {
    use nana_js_engine::{
        HostApiRegistry, HostValue, JsEngine, JsEngineError, JsFunctionId, RuntimeArtifact,
    };

    /// Placeholder until feature `engine` is enabled.
    #[derive(Debug, Default)]
    pub struct V8Engine;

    impl JsEngine for V8Engine {
        fn initialize(&mut self, _artifact: RuntimeArtifact) -> Result<(), JsEngineError> {
            Err(JsEngineError::new(
                "nana-js-v8: enable feature `engine` to link crates.io v8 152.2.0",
            ))
        }

        fn register_host_api(&mut self, _api: &HostApiRegistry) -> Result<(), JsEngineError> {
            Err(JsEngineError::new(
                "nana-js-v8: enable feature `engine` to link crates.io v8 152.2.0",
            ))
        }

        fn resolve_function(&mut self, _name: &str) -> Result<JsFunctionId, JsEngineError> {
            Err(JsEngineError::new(
                "nana-js-v8: enable feature `engine` to link crates.io v8 152.2.0",
            ))
        }

        fn invoke(
            &mut self,
            _target: JsFunctionId,
            _args: &[HostValue],
        ) -> Result<HostValue, JsEngineError> {
            Err(JsEngineError::new(
                "nana-js-v8: enable feature `engine` to link crates.io v8 152.2.0",
            ))
        }

        fn run_microtasks(&mut self) -> Result<(), JsEngineError> {
            Ok(())
        }

        fn interrupt(&mut self) {}
        fn request_gc(&mut self) {}
        fn shutdown(&mut self) {}
    }
}

#[cfg(not(feature = "engine"))]
pub use stub::V8Engine;
