use super::*;

/// Pipelines built from precompiled stages, for the tests that need them used.
pub(in crate::scene_paint) static LOADED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// A family whose WGSL or layout changed without its DXIL being regenerated
/// quietly compiles its WGSL again, and the startup is slow again.
#[test]
fn precompiled_stages_are_current_with_the_wgsl_and_layout() {
    let (wgsl, layout, _) = crate::scene_paint::quad::precompiled_inputs();
    assert!(
        QUAD.fresh(wgsl, layout),
        "the quad family changed: regenerate its DXIL (see generated.rs)"
    );
}

/// naga writes the HLSL the DXIL is compiled from, with wgpu's binding
/// conventions, and both may change in any wgpu release.
#[test]
fn precompiled_stages_were_compiled_with_the_locked_wgpu() {
    assert_eq!(
        generated::WGPU,
        locked_wgpu(),
        "wgpu changed: regenerate the precompiled DXIL (see generated.rs)"
    );
}

/// The wgpu version `Cargo.lock` resolves.
fn locked_wgpu() -> String {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"))
        .expect("the workspace lock file");
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "name = \"wgpu\"" {
            let version = lines.next().expect("a version line");
            return version
                .trim()
                .trim_start_matches("version = \"")
                .trim_end_matches('"')
                .to_owned();
        }
    }
    panic!("wgpu is not in Cargo.lock")
}

/// Rebuilds the precompiled stages: compiles each family's WGSL the way wgpu
/// does on DX12, keeps the HLSL naga wrote for it, compiles that with DXC for
/// shader model 6.0, validates and reflects the DXIL, and writes it with
/// `generated.rs`. `NANA_DXC_PATH` names `dxcompiler.dll`. Run alone:
///
/// `cargo test -p nana-ui --lib --features gpu,hosted,full -- --ignored regenerate_precompiled_dxil --test-threads=1`
#[cfg(target_os = "windows")]
mod regenerate {
    use super::*;
    use std::sync::Mutex;
    use windows::Win32::Graphics::Direct3D::Dxc;
    use windows::core::{GUID, HRESULT, HSTRING, Interface, PCWSTR, s, w};

    static CAPTURED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Keeps the HLSL wgpu-hal's DX12 backend writes for each stage.
    struct Capture;
    impl log::Log for Capture {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.target().starts_with("wgpu_hal")
        }
        fn log(&self, record: &log::Record) {
            let text = record.args().to_string();
            if text.starts_with("Naga generated shader") {
                CAPTURED.lock().unwrap().push(text);
            }
        }
        fn flush(&self) {}
    }

    /// The HLSL captured for `entry`, the latest first.
    fn captured(entry: &str) -> String {
        let captured = CAPTURED.lock().unwrap();
        let text = captured
            .iter()
            .rev()
            .find(|text| text.starts_with(&format!("Naga generated shader for \"{entry}\"")))
            .unwrap_or_else(|| panic!("no HLSL for {entry} among {}", captured.len()));
        text.split_once(":\n").expect("a header line").1.to_owned()
    }

    /// A DX12 device compiling its WGSL with `dxc`, with release flags.
    fn dx12(dxc: &str) -> nana_gpu::GpuContext {
        let mut descriptor = wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            flags: wgpu::InstanceFlags::empty(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        };
        descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::DynamicDxc {
            dxc_path: dxc.to_owned(),
        };
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("a DX12 adapter (WARP serves where there is no GPU)");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("nana-ui precompiled dxil"),
            required_features: adapter.features() & wgpu::Features::DUAL_SOURCE_BLENDING,
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .expect("a DX12 device");
        nana_gpu::__framework::adopt(adapter, device, queue)
    }

    type Create =
        unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut core::ffi::c_void) -> HRESULT;

    /// DXC, loaded from `path`.
    struct Compiler {
        create: Create,
    }

    impl Compiler {
        fn load(path: &str) -> Self {
            use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
            // SAFETY: `dxcompiler.dll` exports `DxcCreateInstance` with this
            // signature; the library stays loaded for the process.
            unsafe {
                let library = LoadLibraryW(&HSTRING::from(path)).expect("dxcompiler.dll");
                Self {
                    create: std::mem::transmute::<unsafe extern "system" fn() -> isize, Create>(
                        GetProcAddress(library, s!("DxcCreateInstance"))
                            .expect("DxcCreateInstance"),
                    ),
                }
            }
        }

        fn instance<T: Interface>(&self, class: &GUID) -> T {
            let mut instance: Option<T> = None;
            // SAFETY: `DxcCreateInstance` writes an interface of `T::IID`.
            unsafe { (self.create)(class, &T::IID, <*mut _>::cast(&mut instance)) }
                .ok()
                .expect("a DXC instance");
            instance.expect("a DXC instance")
        }

        /// `major.minor (commit count, hash)`.
        fn version(&self) -> String {
            let compiler: Dxc::IDxcCompiler3 = self.instance(&Dxc::CLSID_DxcCompiler);
            let info: Dxc::IDxcVersionInfo2 = compiler.cast().expect("DXC version info");
            // SAFETY: plain out-parameters; the hash string is DXC's to free.
            unsafe {
                let (mut major, mut minor) = (0, 0);
                info.GetVersion(&mut major, &mut minor).expect("version");
                let mut count = 0;
                let mut hash = std::ptr::null_mut();
                info.GetCommitInfo(&mut count, &mut hash).expect("commit");
                let hash = std::ffi::CStr::from_ptr(hash.cast())
                    .to_string_lossy()
                    .into_owned();
                format!("{major}.{minor} ({count}, {hash})")
            }
        }

        /// `source` compiled for `target` with the arguments wgpu-hal passes,
        /// release flags, then validated on its own.
        fn compile(&self, source: &str, entry: &str, target: &str) -> Vec<u8> {
            let compiler: Dxc::IDxcCompiler3 = self.instance(&Dxc::CLSID_DxcCompiler);
            let entry = HSTRING::from(entry);
            let target = HSTRING::from(target);
            let args = [
                w!("-E"),
                PCWSTR(entry.as_ptr()),
                w!("-T"),
                PCWSTR(target.as_ptr()),
                w!("-HV"),
                w!("2018"),
                w!("-no-warnings"),
                Dxc::DXC_ARG_ENABLE_STRICTNESS,
            ];
            let buffer = Dxc::DxcBuffer {
                Ptr: source.as_ptr().cast(),
                Size: source.len(),
                Encoding: Dxc::DXC_CP_UTF8.0,
            };
            // SAFETY: DXC reads `buffer` and `args` for the call only.
            let result: Dxc::IDxcResult =
                unsafe { compiler.Compile(&buffer, Some(&args), None) }.expect("Compile");
            let errors: Dxc::IDxcBlobUtf8 = output(&result, Dxc::DXC_OUT_ERRORS);
            // SAFETY: the blob owns `len` UTF-8 bytes at its pointer.
            let message = unsafe {
                String::from_utf8_lossy(std::slice::from_raw_parts(
                    errors.GetStringPointer().0,
                    errors.GetStringLength(),
                ))
                .into_owned()
            };
            assert!(message.is_empty(), "DXC: {message}");
            let object: Dxc::IDxcBlob = output(&result, Dxc::DXC_OUT_OBJECT);
            self.validate(&object);
            // SAFETY: the blob owns its buffer.
            unsafe {
                std::slice::from_raw_parts(
                    object.GetBufferPointer().cast::<u8>(),
                    object.GetBufferSize(),
                )
                .to_vec()
            }
        }

        /// The DXIL validator's verdict, on top of the one DXC signed it with.
        fn validate(&self, object: &Dxc::IDxcBlob) {
            let validator: Dxc::IDxcValidator = self.instance(&Dxc::CLSID_DxcValidator);
            // SAFETY: validation reads the blob.
            unsafe {
                let result = validator
                    .Validate(object, Dxc::DxcValidatorFlags_Default)
                    .expect("Validate");
                let status = result.GetStatus().expect("status");
                assert!(
                    status.is_ok(),
                    "the DXIL validator refused a stage: {status:?}"
                );
            }
        }

        /// How many inputs a vertex stage reads from its vertex buffers.
        fn vertex_inputs(&self, dxil: &[u8]) -> u32 {
            use windows::Win32::Graphics::Direct3D12::ID3D12ShaderReflection;
            let utils: Dxc::IDxcUtils = self.instance(&Dxc::CLSID_DxcUtils);
            let buffer = Dxc::DxcBuffer {
                Ptr: dxil.as_ptr().cast(),
                Size: dxil.len(),
                Encoding: 0,
            };
            let mut reflection: Option<ID3D12ShaderReflection> = None;
            // SAFETY: reflection reads the buffer for the call only.
            unsafe {
                utils
                    .CreateReflection(
                        &buffer,
                        &ID3D12ShaderReflection::IID,
                        <*mut _>::cast(&mut reflection),
                    )
                    .expect("reflection");
                let reflection = reflection.expect("reflection");
                let mut desc = Default::default();
                reflection.GetDesc(&mut desc).expect("shader desc");
                // The vertex and instance indices come in as system values.
                (0..desc.InputParameters)
                    .filter(|&index| {
                        let mut input = Default::default();
                        reflection
                            .GetInputParameterDesc(index, &mut input)
                            .expect("input desc");
                        input.SystemValueType
                            == windows::Win32::Graphics::Direct3D::D3D_NAME_UNDEFINED
                    })
                    .count() as u32
            }
        }
    }

    fn output<T: Interface>(result: &Dxc::IDxcResult, kind: Dxc::DXC_OUT_KIND) -> T {
        let mut output: Option<T> = None;
        // SAFETY: `GetOutput` writes an interface of `T::IID`.
        unsafe { result.GetOutput::<T>(kind, &mut None, <*mut _>::cast(&mut output)) }
            .expect("GetOutput");
        output.expect("an output")
    }

    #[test]
    #[ignore = "writes the precompiled stages; needs DX12 and NANA_DXC_PATH"]
    fn regenerate_precompiled_dxil() {
        let dxc_path = std::env::var("NANA_DXC_PATH").expect("NANA_DXC_PATH");
        log::set_boxed_logger(Box::new(Capture)).expect("the only logger");
        log::set_max_level(log::LevelFilter::Debug);
        let compiler = Compiler::load(&dxc_path);
        let gpu = dx12(&dxc_path);
        let device = nana_gpu::__framework::device(&gpu);
        let format = wgpu::TextureFormat::Rgba8Unorm;

        // The quad family, from its WGSL: the HLSL wgpu writes for it is what
        // the DXIL is compiled from.
        drop(crate::scene_paint::quad::QuadPipeline::new_with_policy(
            device,
            format,
            Some(gpu.policy()),
            false,
        ));
        let (wgsl, layout, attributes) = crate::scene_paint::quad::precompiled_inputs();
        let vertex = compiler.compile(&captured("solid_vs_main"), "solid_vs_main", "vs_6_0");
        let fragment = compiler.compile(&captured("solid_fs_main"), "solid_fs_main", "ps_6_0");
        assert_eq!(
            compiler.vertex_inputs(&vertex) as usize,
            attributes,
            "the quad vertex stage reads every instance attribute"
        );

        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/scene_paint/precompiled");
        std::fs::write(format!("{dir}/quad.vs.dxil"), &vertex).unwrap();
        std::fs::write(format!("{dir}/quad.fs.dxil"), &fragment).unwrap();
        let generated = format!(
            "// Generated by `cargo test -p nana-ui --lib --features gpu,hosted,full --\n\
             // --ignored regenerate_precompiled_dxil`; do not edit.\n\
             use super::{{Family, Stage}};\n\
             \n\
             /// The wgpu, and the DXC, the stages were compiled with: the checks read them.\n\
             #[allow(dead_code)]\n\
             pub(super) const WGPU: &str = {wgpu:?};\n\
             #[allow(dead_code)]\n\
             pub(super) const DXC: &str = {dxc:?};\n\
             \n\
             pub(in crate::scene_paint) const QUAD: Family = Family {{\n    \
                 wgsl: {wgsl:#018x},\n    \
                 layout: {layout:#018x},\n    \
                 vertex: Stage {{\n        \
                     entry: \"solid_vs_main\",\n        \
                     dxil: include_bytes!(\"quad.vs.dxil\"),\n    \
                 }},\n    \
                 fragment: Stage {{\n        \
                     entry: \"solid_fs_main\",\n        \
                     dxil: include_bytes!(\"quad.fs.dxil\"),\n    \
                 }},\n\
             }};\n",
            wgpu = locked_wgpu(),
            dxc = compiler.version(),
            wgsl = fnv1a(wgsl.as_bytes()),
        );
        std::fs::write(format!("{dir}/generated.rs"), generated).unwrap();
    }
}
