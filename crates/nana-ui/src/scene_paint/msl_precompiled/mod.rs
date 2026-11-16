//! Precompiled Metal Shading Language (MSL) stages for built-in shaders.
//!
//! When targeting Apple platforms without `runtime-wgsl`, [`super::wgsl::Program::pipeline`]
//! decompresses the single checked-in archive (`stages.msl.zz`) into a transient buffer,
//! registers both stage modules via [`wgpu::Device::create_shader_module_passthrough`],
//! builds the render pipeline, and drops the decompressed buffer immediately so zero
//! uncompressed MSL or `naga` IR stays resident on the CPU heap.

mod generated;

#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct MslStage {
    pub source_hash: u64,
    pub entry: &'static str,
    pub msl_entry: &'static str,
    pub constants: &'static [(&'static str, u64)],
    pub workgroup_size: (u32, u32, u32),
    pub sized_bindings: &'static [(u32, u32, u32)],
    pub offset: u32,
    pub len: u32,
}

static COMPRESSED_MSL: &[u8] = include_bytes!("stages.msl.zz");

#[cfg(test)]
static BYPASS_PRECOMPILED: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(super) fn bypassing() -> bool {
    BYPASS_PRECOMPILED.load(Ordering::Relaxed)
}

pub(super) fn is_metal(device: &wgpu::Device) -> bool {
    // SAFETY: only inspects whether the active HAL backend is Metal without mutating resources.
    unsafe { device.as_hal::<wgpu::hal::api::Metal>() }.is_some()
}

fn find_stage(
    source_hash: u64,
    entry: &str,
    constants: &[(&str, f64)],
) -> Option<&'static MslStage> {
    generated::STAGES.iter().find(|stage| {
        stage.source_hash == source_hash
            && stage.entry == entry
            && stage.constants.len() == constants.len()
            && stage.constants.iter().all(|&(name, bits)| {
                constants
                    .iter()
                    .any(|&(cname, value)| cname == name && value.to_bits() == bits)
            })
    })
}

pub(super) fn pipeline(
    device: &wgpu::Device,
    label: &'static str,
    source_hash: u64,
    vertex: &str,
    fragment: &str,
    constants: &[(&str, f64)],
    build: impl FnOnce(
        &wgpu::ShaderModule,
        &str,
        &wgpu::ShaderModule,
        &str,
        &[(&str, f64)],
    ) -> wgpu::RenderPipeline,
) -> Option<wgpu::RenderPipeline> {
    if !is_metal(device) {
        return None;
    }
    let vs = find_stage(source_hash, vertex, &[]).unwrap_or_else(|| {
        panic!(
            "missing precompiled MSL vertex stage `{vertex}` for `{label}` ({source_hash:#018x})"
        )
    });
    let fs = find_stage(source_hash, fragment, constants).unwrap_or_else(|| {
        panic!(
            "missing precompiled MSL fragment stage `{fragment}` for `{label}` \
             ({source_hash:#018x}) with constants {constants:?}"
        )
    });

    let mut raw = Vec::with_capacity(generated::UNCOMPRESSED_LEN);
    let mut decoder = flate2::read::ZlibDecoder::new(COMPRESSED_MSL);
    std::io::Read::read_to_end(&mut decoder, &mut raw)
        .expect("precompiled MSL archive decompresses");

    let vs_end = vs.offset as usize + vs.len as usize;
    let fs_end = fs.offset as usize + fs.len as usize;
    let vs_src = std::str::from_utf8(&raw[vs.offset as usize..vs_end])
        .expect("precompiled MSL vertex stage is UTF-8");
    let fs_src = std::str::from_utf8(&raw[fs.offset as usize..fs_end])
        .expect("precompiled MSL fragment stage is UTF-8");

    let make_module = |stage: &MslStage, msl: &str| {
        // SAFETY: MSL was translated from NanaUI's own WGSL shaders by the locked wgpu/naga.
        unsafe {
            device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                label: Some(label),
                entry_points: std::borrow::Cow::Borrowed(&[wgpu::PassthroughShaderEntryPoint {
                    name: std::borrow::Cow::Borrowed(stage.msl_entry),
                    workgroup_size: stage.workgroup_size,
                    metal_sized_bindings: std::borrow::Cow::Borrowed(stage.sized_bindings),
                }]),
                msl: Some(std::borrow::Cow::Borrowed(msl)),
                ..Default::default()
            })
        }
    };

    let vs_module = make_module(vs, vs_src);
    let fs_module = make_module(fs, fs_src);
    // Drop uncompressed MSL immediately so 0 B stays resident on the CPU heap.
    drop(raw);

    Some(build(
        &vs_module,
        vs.msl_entry,
        &fs_module,
        fs.msl_entry,
        &[],
    ))
}

#[cfg(test)]
pub(in crate::scene_paint) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;
    use std::sync::Mutex;

    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct StageKey {
        source_hash: u64,
        entry: String,
        constants: Vec<(String, u64)>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CapturedStage {
        msl_entry: String,
        workgroup_size: (u32, u32, u32),
        sized_bindings: Vec<(u32, u32, u32)>,
        msl: String,
    }

    #[derive(Clone)]
    struct PipelineRequest {
        source_hash: u64,
        vertex: String,
        fragment: String,
        fragment_constants: Vec<(String, u64)>,
    }

    thread_local! {
        static ACTIVE_REQUEST: RefCell<Option<PipelineRequest>> = const { RefCell::new(None) };
    }

    static CAPTURED: Mutex<BTreeMap<StageKey, CapturedStage>> = Mutex::new(BTreeMap::new());

    fn canonical_constants(constants: &[(&str, f64)]) -> Vec<(String, u64)> {
        let mut out: Vec<(String, u64)> = constants
            .iter()
            .map(|&(name, value)| (name.to_owned(), value.to_bits()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub(in crate::scene_paint) fn recording_pipeline<R>(
        source_hash: u64,
        vertex: &str,
        fragment: &str,
        constants: &[(&str, f64)],
        run: impl FnOnce() -> R,
    ) -> R {
        struct Guard(Option<PipelineRequest>);
        impl Drop for Guard {
            fn drop(&mut self) {
                ACTIVE_REQUEST.with(|cell| {
                    *cell.borrow_mut() = self.0.take();
                });
            }
        }
        let next = PipelineRequest {
            source_hash,
            vertex: vertex.to_owned(),
            fragment: fragment.to_owned(),
            fragment_constants: canonical_constants(constants),
        };
        let prev = ACTIVE_REQUEST.with(|cell| cell.borrow_mut().replace(next));
        let _guard = Guard(prev);
        run()
    }

    struct StageLogger;

    impl log::Log for StageLogger {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.target() == "wgpu_hal::metal::stage_meta"
        }

        fn log(&self, record: &log::Record) {
            if !self.enabled(record.metadata()) {
                return;
            }
            let Some(req) = ACTIVE_REQUEST.with(|cell| cell.borrow().clone()) else {
                return;
            };
            let msg = format!("{}", record.args());
            let Some((header, rest)) = msg.split_once('\n') else {
                return;
            };
            let Some((len_line, msl)) = rest.split_once('\n') else {
                return;
            };
            let msl_len: usize = len_line
                .strip_prefix("msl_len=")
                .and_then(|s| s.parse().ok())
                .expect("stage_meta msl_len line");
            assert_eq!(msl.len(), msl_len, "stage_meta MSL length mismatch");

            let mut entry = None;
            let mut stage = None;
            let mut translated = None;
            let mut wg = (0u32, 0u32, 0u32);
            let mut sized = Vec::new();
            for field in header.split('\t') {
                if let Some(v) = field.strip_prefix("entry=") {
                    entry = Some(v);
                } else if let Some(v) = field.strip_prefix("stage=") {
                    stage = Some(v);
                } else if let Some(v) = field.strip_prefix("translated=") {
                    translated = Some(v);
                } else if let Some(v) = field.strip_prefix("wg=") {
                    let parts: Vec<u32> = v.split(',').filter_map(|p| p.parse().ok()).collect();
                    assert_eq!(parts.len(), 3, "stage_meta wg field");
                    wg = (parts[0], parts[1], parts[2]);
                } else if let Some(v) = field.strip_prefix("sized=") {
                    if !v.is_empty() {
                        for item in v.split(';') {
                            let parts: Vec<u32> =
                                item.split(',').filter_map(|p| p.parse().ok()).collect();
                            assert_eq!(parts.len(), 3, "stage_meta sized field");
                            sized.push((parts[0], parts[1], parts[2]));
                        }
                    }
                }
            }

            let entry = entry.expect("stage_meta entry");
            let stage = stage.expect("stage_meta stage");
            let msl_entry = translated.expect("stage_meta translated").to_owned();
            let constants = match stage {
                "Vertex" => {
                    assert_eq!(entry, req.vertex);
                    Vec::new()
                }
                "Fragment" => {
                    assert_eq!(entry, req.fragment);
                    req.fragment_constants
                }
                other => panic!("unexpected stage `{other}` in render pipeline capture"),
            };

            let key = StageKey {
                source_hash: req.source_hash,
                entry: entry.to_owned(),
                constants,
            };
            let captured = CapturedStage {
                msl_entry,
                workgroup_size: wg,
                sized_bindings: sized,
                msl: msl.to_owned(),
            };

            let mut guard = CAPTURED.lock().unwrap();
            if let Some(existing) = guard.get(&key) {
                assert_eq!(existing, &captured, "conflicting MSL captured for {key:?}");
            } else {
                guard.insert(key, captured);
            }
        }

        fn flush(&self) {}
    }

    fn locked_wgpu() -> String {
        let lock =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"))
                .expect("workspace Cargo.lock");
        let mut lines = lock.lines();
        while let Some(line) = lines.next() {
            if line.trim() == "name = \"wgpu\"" {
                let version = lines.next().expect("wgpu version line");
                return version
                    .trim()
                    .trim_start_matches("version = \"")
                    .trim_end_matches('"')
                    .to_owned();
            }
        }
        panic!("wgpu is not in Cargo.lock");
    }

    #[test]
    fn precompiled_msl_stages_are_fresh() {
        assert_eq!(
            generated::WGPU,
            locked_wgpu(),
            "wgpu changed: regenerate precompiled MSL (run regenerate_precompiled_msl)"
        );
        let mut raw = Vec::new();
        let mut decoder = flate2::read::ZlibDecoder::new(COMPRESSED_MSL);
        std::io::Read::read_to_end(&mut decoder, &mut raw)
            .expect("precompiled MSL archive decompresses");
        assert_eq!(
            raw.len(),
            generated::UNCOMPRESSED_LEN,
            "stale UNCOMPRESSED_LEN in msl_precompiled/generated.rs"
        );
        for program in super::super::naga_ir::tests::builtin_programs() {
            let hash = super::super::precompiled::fnv1a(program.source.as_bytes());
            let count = generated::STAGES
                .iter()
                .filter(|stage| stage.source_hash == hash)
                .count();
            assert!(
                count >= 2,
                "missing precompiled MSL stages for built-in shader `{}` (hash {hash:#018x}); \
                 run regenerate_precompiled_msl",
                program.slug
            );
        }
    }

    #[test]
    #[ignore = "regenerates checked-in Metal MSL archive after WGSL shader or wgpu edits"]
    fn regenerate_precompiled_msl() {
        assert!(
            cfg!(feature = "hosted") && cfg!(feature = "charts"),
            "run regenerate_precompiled_msl with --features full so all feature-gated shaders are emitted"
        );

        struct BypassGuard;
        impl Drop for BypassGuard {
            fn drop(&mut self) {
                BYPASS_PRECOMPILED.store(false, Ordering::SeqCst);
            }
        }
        BYPASS_PRECOMPILED.store(true, Ordering::SeqCst);
        let _bypass = BypassGuard;

        let _ = log::set_boxed_logger(Box::new(StageLogger));
        log::set_max_level(log::LevelFilter::Debug);
        CAPTURED.lock().unwrap().clear();

        let gpu = crate::test_gpu::fresh_context();
        let device = nana_gpu::__framework::device(&gpu);
        assert!(
            is_metal(device),
            "regenerate_precompiled_msl requires Metal"
        );

        let painter =
            crate::SceneWgpuPainter::new(&gpu, nana_gpu::GpuTextureFormat::BGRA8_UNORM_SRGB);
        let motion_layout =
            super::super::motion::motion_bind_layout_with_policy(device, Some(gpu.policy()));
        #[cfg(feature = "charts")]
        let _chart = super::super::chart::ChartPipeline::new(
            device,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            &motion_layout,
        );
        super::super::dest::DestPipelines::prewarm_all_for_msl_regen(device, gpu.policy());
        super::super::motion::prewarm_for_msl_regen(device, &motion_layout);
        crate::default_gpu_view::prewarm_for_msl_regen(&gpu);
        let _t1 = crate::output::transfer::OutputTransfer::new(
            &gpu,
            nana_gpu::GpuTextureFormat::BGRA8_UNORM_SRGB,
            false,
        );
        let _t2 = crate::output::transfer::OutputTransfer::new(
            &gpu,
            nana_gpu::GpuTextureFormat::BGRA8_UNORM,
            true,
        );
        let _t3 = crate::output::transfer::OutputTransfer::new(
            &gpu,
            nana_gpu::GpuTextureFormat::BGRA8_UNORM,
            false,
        );
        #[cfg(feature = "hosted")]
        crate::output::window_output::prewarm_for_msl_regen(&gpu);
        #[cfg(feature = "hosted")]
        crate::native_content::NativeContentRenderer::prewarm_for_msl_regen(&gpu);
        drop(painter);

        let programs = super::super::naga_ir::tests::builtin_programs();
        let mut by_hash: BTreeMap<u64, (&'static str, Option<&'static str>)> = BTreeMap::new();
        for program in &programs {
            let hash = super::super::precompiled::fnv1a(program.source.as_bytes());
            by_hash.insert(hash, (program.slug, program.cfg_gate));
        }

        let captured = CAPTURED.lock().unwrap();
        let captured_hashes: BTreeSet<u64> = captured.keys().map(|k| k.source_hash).collect();
        for (&hash, &(slug, _)) in &by_hash {
            assert!(
                captured_hashes.contains(&hash),
                "regenerate_precompiled_msl did not capture stages for `{slug}` ({hash:#018x})"
            );
        }

        let mut blob = Vec::new();
        let mut dedup: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
        let mut rows = String::new();
        for (key, stage) in captured.iter() {
            let (offset, len) = if let Some(&span) = dedup.get(stage.msl.as_str()) {
                span
            } else {
                let offset = u32::try_from(blob.len()).expect("MSL blob offset fits u32");
                let len = u32::try_from(stage.msl.len()).expect("MSL blob len fits u32");
                blob.extend_from_slice(stage.msl.as_bytes());
                dedup.insert(stage.msl.as_str(), (offset, len));
                (offset, len)
            };
            let &(_, cfg_gate) = by_hash
                .get(&key.source_hash)
                .unwrap_or_else(|| panic!("unexpected shader hash {:#018x}", key.source_hash));
            if let Some(gate) = cfg_gate {
                rows.push_str(&format!("    #[cfg({gate})]\n"));
            }
            let consts = key
                .constants
                .iter()
                .map(|(name, bits)| format!("({name:?}, {bits:#018x})"))
                .collect::<Vec<_>>()
                .join(", ");
            let sized = stage
                .sized_bindings
                .iter()
                .map(|(g, b, i)| format!("({g}, {b}, {i})"))
                .collect::<Vec<_>>()
                .join(", ");
            rows.push_str(&format!(
                "    MslStage {{\n        \
                    source_hash: {:#018x},\n        \
                    entry: {:?},\n        \
                    msl_entry: {:?},\n        \
                    constants: &[{consts}],\n        \
                    workgroup_size: {:?},\n        \
                    sized_bindings: &[{sized}],\n        \
                    offset: {offset},\n        \
                    len: {len},\n    \
                 }},\n",
                key.source_hash, key.entry, stage.msl_entry, stage.workgroup_size,
            ));
        }

        let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/scene_paint/msl_precompiled");
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut encoder, &blob).expect("compress MSL archive");
        let compressed = encoder.finish().expect("finish MSL archive");
        std::fs::write(out_dir.join("stages.msl.zz"), compressed).expect("write stages.msl.zz");

        let wgpu_ver = locked_wgpu();
        let uncompressed_len = blob.len();
        let generated_rs = format!(
            "// Generated by `cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_msl --test-threads=1`.\n\n\
             use super::MslStage;\n\n\
             #[cfg(test)]\n\
             pub(super) const WGPU: &str = {wgpu_ver:?};\n\n\
             pub(super) const UNCOMPRESSED_LEN: usize = {uncompressed_len};\n\n\
             pub(super) static STAGES: &[MslStage] = &[\n\
             {rows}\
             ];\n"
        );
        std::fs::write(out_dir.join("generated.rs"), generated_rs).expect("write generated.rs");
    }
}
