use super::*;

/// Pipelines built from precompiled stages, for the tests that need them used.
pub(in crate::scene_paint) static LOADED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Each precompiled family, with the program and layout key it has now.
fn families() -> Vec<(&'static str, &'static Family, String, u64)> {
    let (quad, quad_layout, _) = crate::scene_paint::quad::precompiled_inputs();
    let (mesh, mesh_layout) = crate::scene_paint::mesh::precompiled_inputs();
    let (text, text_layout) = crate::scene_paint::text::precompiled_inputs(false);
    let (dual, dual_layout) = crate::scene_paint::text::precompiled_inputs(true);
    let (group, group_layout) = crate::scene_paint::dest::precompiled_inputs(false);
    let (blend, blend_layout) = crate::scene_paint::dest::precompiled_inputs(true);
    let (icon, icon_layout) = crate::scene_paint::icon::precompiled_inputs();
    let (host_texture, host_texture_layout) = crate::gpu_texture::precompiled_inputs();
    vec![
        ("quad", &QUAD, quad, quad_layout),
        ("mesh", &MESH, mesh, mesh_layout),
        ("text", &TEXT, text, text_layout),
        ("text_dual_source", &TEXT_DUAL_SOURCE, dual, dual_layout),
        ("group", &GROUP, group, group_layout),
        ("group_blend", &GROUP_BLEND, blend, blend_layout),
        ("icon", &ICON, icon, icon_layout),
        (
            "host_texture",
            &HOST_TEXTURE,
            host_texture,
            host_texture_layout,
        ),
    ]
}

/// A family whose program or layout changed without its DXIL being
/// regenerated quietly compiles its program again, and the startup is slow
/// again.
#[test]
fn precompiled_stages_are_current_with_their_programs_and_layouts() {
    for (name, family, program, layout) in families() {
        assert!(
            family.fresh(&program, layout),
            "the {name} family changed: regenerate its DXIL (see generated.rs)"
        );
    }
}

/// The stages were generated from a Windows checkout; one with LF endings
/// (Linux, macOS, a packaged crate) must still find them current.
#[test]
fn precompiled_hash_ignores_line_endings() {
    let (wgsl, _, _) = crate::scene_paint::quad::precompiled_inputs();
    let crlf = wgsl.replace("\r\n", "\n").replace('\n', "\r\n");
    assert_eq!(fnv1a(crlf.as_bytes()), fnv1a(wgsl.as_bytes()));
}

/// Every stage a family's pipelines ask for is precompiled: a missing one
/// would compile its program after all.
#[test]
fn every_stage_the_families_ask_for_is_precompiled() {
    let constants = |samples: f64| [("PATH_SAMPLES", samples)];
    for (family, entry, constants) in [
        (&QUAD, "solid_vs_main", &[][..]),
        (&QUAD, "solid_fs_main", &[]),
        (&MESH, "solid_vs_main", &[]),
        (&MESH, "solid_fs_main", &[]),
        (&MESH, "path_vs_main", &[]),
        (&MESH, "path_fs_main", &constants(1.0)),
        (&MESH, "path_fs_main", &constants(4.0)),
        (&MESH, "path_keep_fs_main", &constants(1.0)),
        (&TEXT, "vs_main", &[]),
        (&TEXT, "fs_main", &[]),
        (&TEXT_DUAL_SOURCE, "vs_main", &[]),
        (&TEXT_DUAL_SOURCE, "fs_main", &[]),
        (&GROUP, "vs_main", &[]),
        (&GROUP, "fs_main", &[]),
        (&GROUP_BLEND, "vs_main", &[]),
        (&GROUP_BLEND, "fs_blend", &[]),
        (&ICON, "vs_main", &[]),
        (&ICON, "fs_main", &[]),
        (&HOST_TEXTURE, "vertex_main", &[]),
        (&HOST_TEXTURE, "fragment_main", &[]),
        (&HOST_TEXTURE, "vertex_main_simple", &[]),
        (&HOST_TEXTURE, "fragment_simple_blend", &[]),
        (&HOST_TEXTURE, "fragment_simple_replace", &[]),
    ] {
        assert!(
            family.stage(entry, constants).is_some(),
            "{entry} {constants:?}"
        );
    }
    assert!(MESH.stage("path_fs_main", &constants(2.0)).is_none());
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
        if line.trim() == "name = \"nana-wgpu\"" {
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

/// The pipeline a program is compiling stages for from its IR: the
/// program's [`fnv1a`], its vertex and fragment entry points, and the
/// fragment stage's constants. Regeneration reads it to tell whose HLSL
/// wgpu-hal is writing.
#[cfg(target_os = "windows")]
type Request = (u64, String, String, Vec<(String, f64)>);

#[cfg(target_os = "windows")]
static REQUEST: std::sync::Mutex<Option<Request>> = std::sync::Mutex::new(None);

/// Called by [`crate::scene_paint::wgsl::Program::pipeline`] before it
/// builds a pipeline from its IR.
#[cfg(target_os = "windows")]
pub(in crate::scene_paint) fn requesting(
    program: &str,
    vertex: &str,
    fragment: &str,
    constants: &[(&str, f64)],
) {
    *REQUEST.lock().unwrap() = Some((
        fnv1a(program.as_bytes()),
        vertex.to_owned(),
        fragment.to_owned(),
        constants
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect(),
    ));
}

/// Rebuilds the precompiled stages: builds each family's pipelines from its
/// program the way wgpu does on DX12, keeps the HLSL naga wrote for each
/// stage, compiles that with DXC for shader model 6.0, validates and
/// reflects the DXIL, and writes it with `generated.rs`. `NANA_DXC_PATH`
/// names `dxcompiler.dll`. Run alone:
///
/// `cargo test -p nana-ui --lib --features gpu,hosted,full -- --ignored regenerate_precompiled_dxil --test-threads=1`
#[cfg(target_os = "windows")]
mod regenerate {
    use super::*;
    use std::sync::Mutex;
    use windows::Win32::Graphics::Direct3D::Dxc;
    use windows::core::{GUID, HRESULT, HSTRING, Interface, PCWSTR, s, w};

    /// One stage wgpu-hal wrote HLSL for.
    struct Captured {
        program: u64,
        entry: String,
        /// `vs` or `ps`.
        kind: &'static str,
        constants: Vec<(String, f64)>,
        hlsl: String,
    }

    static CAPTURED: Mutex<Vec<Captured>> = Mutex::new(Vec::new());

    /// Keeps the HLSL wgpu-hal's DX12 backend writes for each stage, with
    /// the program and constants [`REQUEST`] names.
    struct Capture;
    impl log::Log for Capture {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.target().starts_with("wgpu_hal")
        }
        fn log(&self, record: &log::Record) {
            let text = record.args().to_string();
            let Some(rest) = text.strip_prefix("Naga generated shader for \"") else {
                return;
            };
            let (entry, rest) = rest.split_once('"').expect("an entry point");
            let (header, hlsl) = rest.split_once(":\n").expect("a header line");
            let request = REQUEST.lock().unwrap();
            let (program, vertex, fragment, constants) =
                request.as_ref().expect("a program is building a pipeline");
            let (kind, constants) = if header.contains("Vertex") {
                assert_eq!(entry, vertex);
                ("vs", Vec::new())
            } else {
                assert!(header.contains("Fragment"), "{header}");
                assert_eq!(entry, fragment);
                ("ps", constants.clone())
            };
            CAPTURED.lock().unwrap().push(Captured {
                program: *program,
                entry: entry.to_owned(),
                kind,
                constants,
                hlsl: hlsl.to_owned(),
            });
        }
        fn flush(&self) {}
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

    /// `stage` of `family`, with its constants, as a file name.
    fn file_name(family: &str, stage: &Captured) -> String {
        let mut name = format!("{family}.{}", stage.entry);
        for (constant, value) in &stage.constants {
            name.push_str(&format!(".{constant}{value}"));
        }
        name + ".dxil"
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
        let queue = nana_gpu::__framework::queue(&gpu);
        let policy = gpu.policy();
        let format = wgpu::TextureFormat::Rgba8Unorm;

        // Every precompiled family, from its program, one after another on
        // this thread: the HLSL wgpu writes for each stage is what its DXIL
        // is compiled from.
        drop(crate::scene_paint::quad::QuadPipeline::new_with_policy(
            device,
            format,
            Some(policy),
            false,
        ));
        drop(crate::scene_paint::mesh::MeshPipeline::new_with_policy(
            device,
            format,
            Some(policy),
            false,
        ));
        assert!(
            device
                .features()
                .contains(wgpu::Features::DUAL_SOURCE_BLENDING),
            "the subpixel text program needs dual-source blending"
        );
        let mut text = crate::scene_paint::text::TextPipeline::new_with_policy(
            device, queue, format, policy, false,
        );
        text.prewarm_subpixel(device);
        drop(text);
        // The group compositing the host builds at startup, and the group
        // blend a frame with a backdrop-reading blend mode builds.
        drop(
            crate::scene_paint::dest::DestPipelines::try_new(
                device,
                format,
                format,
                crate::ScenePresentationProfile::sdr(nana_gpu::GpuTextureFormat::RGBA8_UNORM),
                policy,
            )
            .expect("dest pipelines"),
        );
        drop(
            crate::scene_paint::dest::try_reading_blend_pipeline(device, format, policy)
                .expect("group blend pipeline"),
        );
        drop(
            crate::scene_paint::icon::IconPipeline::try_new_with_policy(
                device,
                format,
                Some(policy),
            )
            .expect("icon pipeline"),
        );
        drop(
            crate::gpu_texture::GpuTexturePipeline::try_new_with_policy(
                device,
                queue,
                format,
                Some(policy),
                Some(&gpu),
            )
            .expect("host texture pipeline"),
        );

        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/scene_paint/precompiled");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path
                .extension()
                .is_some_and(|extension| extension == "dxil")
            {
                std::fs::remove_file(path).unwrap();
            }
        }
        let captured = CAPTURED.lock().unwrap();
        let mut families_source = String::new();
        // A stage whose DXIL another already has (the two text programs
        // share their vertex stage) names that one's file.
        let mut written: Vec<(Vec<u8>, String)> = Vec::new();
        for (name, _, program, layout) in families() {
            let hash = fnv1a(program.as_bytes());
            let mut stages: Vec<&Captured> = captured
                .iter()
                .filter(|stage| stage.program == hash)
                .collect();
            let order = |stage: &Captured| {
                (
                    stage.entry.clone(),
                    stage.kind,
                    format!("{:?}", stage.constants),
                )
            };
            stages.sort_by_key(|stage| order(stage));
            stages.dedup_by_key(|stage| order(stage));
            assert!(stages.len() >= 2, "{name} built no stages");
            let mut entries = String::new();
            for stage in stages {
                let dxil =
                    compiler.compile(&stage.hlsl, &stage.entry, &format!("{}_6_0", stage.kind));
                if name == "quad" && stage.kind == "vs" {
                    let (_, _, attributes) = crate::scene_paint::quad::precompiled_inputs();
                    assert_eq!(
                        compiler.vertex_inputs(&dxil) as usize,
                        attributes,
                        "the quad vertex stage reads every instance attribute"
                    );
                }
                let file = match written.iter().find(|(bytes, _)| *bytes == dxil) {
                    Some((_, file)) => file.clone(),
                    None => {
                        let file = file_name(name, stage);
                        std::fs::write(format!("{dir}/{file}"), &dxil).unwrap();
                        written.push((dxil, file.clone()));
                        file
                    }
                };
                let constants = stage
                    .constants
                    .iter()
                    .map(|(constant, value)| format!("({constant:?}, {value:?})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                entries.push_str(&format!(
                    "        Stage {{\n            entry: {entry:?},\n            constants: &[{constants}],\n            dxil: include_bytes!({file:?}),\n        }},\n",
                    entry = stage.entry,
                ));
            }
            families_source.push_str(&format!(
                "\npub(in crate::scene_paint) const {constant}: Family = Family {{\n    wgsl: {hash:#018x},\n    layout: {layout:#018x},\n    stages: &[\n{entries}    ],\n}};\n",
                constant = name.to_uppercase(),
            ));
        }
        let generated = format!(
            "// Generated by `cargo test -p nana-ui --lib --features gpu,hosted,full --\n// --ignored regenerate_precompiled_dxil`; do not edit.\nuse super::{{Family, Stage}};\n\n/// The wgpu, and the DXC, the stages were compiled with: the checks read them.\n#[allow(dead_code)]\npub(super) const WGPU: &str = {wgpu:?};\n#[allow(dead_code)]\npub(super) const DXC: &str = {dxc:?};\n{families_source}",
            wgpu = locked_wgpu(),
            dxc = compiler.version(),
        );
        std::fs::write(format!("{dir}/generated.rs"), generated).unwrap();
    }
}
