//! Precompiled Metal Shading Language (MSL) stages for built-in shaders.
//!
//! When targeting Apple platforms without `runtime-wgsl`, [`super::wgsl::Program::try_pipeline`]
//! borrows a scoped shared decompression lease of the single checked-in archive
//! (`stages.msl.zz`), registers both stage modules via [`wgpu::Device::create_shader_module_passthrough`],
//! validates pipeline creation inside GPU error scopes, and drops its lease when the build
//! scope completes so zero uncompressed MSL or `naga` IR stays resident on the CPU heap.

mod generated;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

#[cfg(test)]
use std::sync::atomic::AtomicBool;

pub(super) const REQUIRED_MSL_VERSION: (u8, u8) = (3, 1);

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
#[derive(Default)]
struct ArchiveState {
    cache: Option<Weak<Vec<u8>>>,
    pin_depth: usize,
    pinned: Option<Arc<Vec<u8>>>,
}
static ARCHIVE_STATE: Mutex<ArchiveState> = Mutex::new(ArchiveState {
    cache: None,
    pin_depth: 0,
    pinned: None,
});
static DECOMPRESSIONS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
static BYPASS_PRECOMPILED: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
static FORCE_MISSING_STAGE: AtomicBool = AtomicBool::new(false);
/// Test switch: build as a release without `runtime-wgsl` would, so a
/// missing or refused precompiled stage is an error instead of a fallback.
#[cfg(test)]
static FORBID_WGSL_FALLBACK: AtomicBool = AtomicBool::new(false);
/// Test record of precompiled stage outcomes while `Some`: the stages built
/// (by address in `generated::STAGES`) and every failure.
#[cfg(test)]
static OUTCOMES: Mutex<Option<(std::collections::BTreeSet<usize>, Vec<String>)>> = Mutex::new(None);

#[cfg(test)]
pub(super) fn wgsl_fallback_forbidden() -> bool {
    FORBID_WGSL_FALLBACK.load(Ordering::SeqCst)
}

#[cfg(test)]
fn record_outcome(built: &[&MslStage], failure: Option<&str>) {
    let mut guard = OUTCOMES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((stages, failures)) = guard.as_mut() {
        match failure {
            Some(failure) => failures.push(failure.to_owned()),
            None => stages.extend(built.iter().map(|stage| *stage as *const MslStage as usize)),
        }
    }
}

pub(crate) struct ArchiveLease {
    bytes: Arc<Vec<u8>>,
}

impl ArchiveLease {
    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }
}

/// Lazy pinning scope for [`lease_archive`]: performs zero work on creation;
/// if any pipeline miss decompresses `stages.msl.zz` while the scope is
/// active, the buffer stays pinned across worker threads until the outermost
/// `ArchiveScope` drops, then returns to 0 B resident.
pub(crate) struct ArchiveScope;

impl ArchiveScope {
    pub(crate) fn new() -> Self {
        let mut guard = ARCHIVE_STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.pin_depth += 1;
        Self
    }
}

impl Drop for ArchiveScope {
    fn drop(&mut self) {
        let released = {
            let mut guard = ARCHIVE_STATE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.pin_depth = guard.pin_depth.saturating_sub(1);
            if guard.pin_depth == 0 {
                guard.pinned.take()
            } else {
                None
            }
        };
        drop(released);
    }
}

/// Acquire a scoped shared lease to the decompressed precompiled MSL archive.
///
/// While any lease or [`ArchiveScope`] stays alive (for example across
/// `SceneWgpuPainter::try_new_with_presentation` worker threads), all callers
/// share one decompression; once the final lease and scope drop, the buffer is
/// freed and resident uncompressed bytes return to zero.
pub(crate) fn lease_archive() -> Result<ArchiveLease, String> {
    let mut guard = ARCHIVE_STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(bytes) = guard.pinned.as_ref() {
        return Ok(ArchiveLease {
            bytes: Arc::clone(bytes),
        });
    }
    if let Some(bytes) = guard.cache.as_ref().and_then(Weak::upgrade) {
        if guard.pin_depth > 0 {
            guard.pinned = Some(Arc::clone(&bytes));
        }
        return Ok(ArchiveLease { bytes });
    }
    let started = std::time::Instant::now();
    let mut raw = Vec::with_capacity(generated::UNCOMPRESSED_LEN);
    let mut decoder = flate2::read::ZlibDecoder::new(COMPRESSED_MSL);
    std::io::Read::read_to_end(&mut decoder, &mut raw)
        .map_err(|error| format!("failed to decompress precompiled MSL archive: {error}"))?;
    nana_diagnostics::metric!(
        nana_diagnostics::framework::gpu::MSL_ARCHIVE_DECOMPRESS_NS,
        started.elapsed()
    );
    nana_diagnostics::metric!(
        nana_diagnostics::framework::gpu::MSL_ARCHIVE_DECOMPRESSED_BYTES,
        raw.len() as u64
    );
    let bytes = Arc::new(raw);
    guard.cache = Some(Arc::downgrade(&bytes));
    if guard.pin_depth > 0 {
        guard.pinned = Some(Arc::clone(&bytes));
    }
    DECOMPRESSIONS.fetch_add(1, Ordering::Relaxed);
    nana_diagnostics::metric!(
        nana_diagnostics::framework::gpu::MSL_ARCHIVE_DECOMPRESSIONS,
        1u64
    );
    Ok(ArchiveLease { bytes })
}

#[cfg(test)]
pub(super) fn archive_resident_bytes() -> usize {
    let guard = ARCHIVE_STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .pinned
        .as_ref()
        .map(|arc| arc.len())
        .or_else(|| {
            guard
                .cache
                .as_ref()
                .and_then(Weak::upgrade)
                .map(|arc| arc.len())
        })
        .unwrap_or(0)
}

#[cfg(test)]
pub(super) fn decompression_count() -> usize {
    DECOMPRESSIONS.load(Ordering::Relaxed)
}

#[cfg(test)]
pub(super) fn set_force_missing_stage(missing: bool) {
    FORCE_MISSING_STAGE.store(missing, Ordering::Relaxed);
}

#[cfg(test)]
pub(super) fn bypassing() -> bool {
    BYPASS_PRECOMPILED.load(Ordering::Relaxed)
}

#[cfg(test)]
pub(super) fn is_metal(device: &wgpu::Device) -> bool {
    // SAFETY: only inspects whether the active HAL backend is Metal without mutating resources.
    unsafe { device.as_hal::<wgpu::hal::api::Metal>() }.is_some()
}

#[cfg(test)]
pub(super) fn supports_precompiled_msl(device: &wgpu::Device) -> bool {
    // SAFETY: only inspects whether the active HAL backend is Metal and reads its MSL floor.
    unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
        .is_some_and(|hal| hal.msl_version() >= REQUIRED_MSL_VERSION)
}

fn ensure_msl_version(version: (u8, u8), label: &'static str) -> Result<(), String> {
    if version < REQUIRED_MSL_VERSION {
        return Err(format!(
            "Metal device MSL version {}.{} is older than required {}.{} (macOS 14.0+ / iOS 17.0+) for `{label}`",
            version.0, version.1, REQUIRED_MSL_VERSION.0, REQUIRED_MSL_VERSION.1,
        ));
    }
    Ok(())
}

fn check_device_msl_support(device: &wgpu::Device, label: &'static str) -> Result<(), String> {
    // SAFETY: only inspects whether the active HAL backend is Metal and reads its MSL floor.
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
        .ok_or_else(|| format!("device backend is not Metal for `{label}`"))?;
    ensure_msl_version(hal.msl_version(), label)
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

fn stage_span(
    raw: &[u8],
    stage: &MslStage,
    role: &str,
    label: &'static str,
) -> Result<(usize, usize), String> {
    let start = stage.offset as usize;
    let end = start
        .checked_add(stage.len as usize)
        .ok_or_else(|| format!("corrupt MSL {role} stage bounds for `{label}`"))?;
    let slice = raw
        .get(start..end)
        .ok_or_else(|| format!("out-of-bounds MSL {role} stage slice for `{label}`"))?;
    std::str::from_utf8(slice).map_err(|error| {
        format!("precompiled MSL {role} stage for `{label}` is invalid UTF-8: {error}")
    })?;
    Ok((start, end))
}

pub(super) struct PreparedMsl {
    label: &'static str,
    vs: &'static MslStage,
    vs_span: (usize, usize),
    fs: &'static MslStage,
    fs_span: (usize, usize),
    lease: ArchiveLease,
}

pub(super) fn prepare(
    device: &wgpu::Device,
    label: &'static str,
    source_hash: u64,
    vertex: &str,
    fragment: &str,
    constants: &[(&str, f64)],
) -> Result<PreparedMsl, String> {
    let prepared = prepare_inner(device, label, source_hash, vertex, fragment, constants);
    #[cfg(test)]
    if let Err(error) = &prepared {
        record_outcome(&[], Some(error));
    }
    prepared
}

fn prepare_inner(
    device: &wgpu::Device,
    label: &'static str,
    source_hash: u64,
    vertex: &str,
    fragment: &str,
    constants: &[(&str, f64)],
) -> Result<PreparedMsl, String> {
    check_device_msl_support(device, label)?;
    #[cfg(test)]
    if FORCE_MISSING_STAGE.load(Ordering::Relaxed) {
        return Err(format!(
            "missing precompiled MSL vertex stage `{vertex}` for `{label}` ({source_hash:#018x})"
        ));
    }
    let vs = find_stage(source_hash, vertex, &[]).ok_or_else(|| {
        format!(
            "missing precompiled MSL vertex stage `{vertex}` for `{label}` ({source_hash:#018x})"
        )
    })?;
    let fs = find_stage(source_hash, fragment, constants).ok_or_else(|| {
        format!(
            "missing precompiled MSL fragment stage `{fragment}` for `{label}` ({source_hash:#018x})"
        )
    })?;
    let lease = lease_archive()?;
    let raw = lease.as_slice();
    let vs_span = stage_span(raw, vs, "vertex", label)?;
    let fs_span = stage_span(raw, fs, "fragment", label)?;
    Ok(PreparedMsl {
        label,
        vs,
        vs_span,
        fs,
        fs_span,
        lease,
    })
}

impl PreparedMsl {
    pub(super) fn build(
        self,
        device: &wgpu::Device,
        build: impl FnOnce(
            &wgpu::ShaderModule,
            &str,
            &wgpu::ShaderModule,
            &str,
            &[(&str, f64)],
        ) -> wgpu::RenderPipeline,
    ) -> (Result<(), String>, wgpu::RenderPipeline) {
        let raw = self.lease.as_slice();
        let vs_src = raw
            .get(self.vs_span.0..self.vs_span.1)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or("");
        let fs_src = raw
            .get(self.fs_span.0..self.fs_span.1)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or("");
        let make_module = |stage: &MslStage, msl: &str| {
            // SAFETY: MSL was translated from NanaUI's own WGSL shaders by the locked wgpu/naga.
            unsafe {
                device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                    label: Some(self.label),
                    entry_points: std::borrow::Cow::Borrowed(&[
                        wgpu::PassthroughShaderEntryPoint {
                            name: std::borrow::Cow::Borrowed(stage.msl_entry),
                            workgroup_size: stage.workgroup_size,
                            metal_sized_bindings: std::borrow::Cow::Borrowed(stage.sized_bindings),
                        },
                    ]),
                    msl: Some(std::borrow::Cow::Borrowed(msl)),
                    ..Default::default()
                })
            }
        };

        let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let vs_module = make_module(self.vs, vs_src);
        let fs_module = make_module(self.fs, fs_src);
        // Drop local lease so standalone callers free immediately when no outer scope holds it.
        drop(self.lease);

        let pipeline = build(
            &vs_module,
            self.vs.msl_entry,
            &fs_module,
            self.fs.msl_entry,
            &[],
        );
        let gpu_error = pollster::block_on(validation.pop())
            .or_else(|| pollster::block_on(internal.pop()))
            .or_else(|| pollster::block_on(oom.pop()));
        if let Some(error) = gpu_error {
            let error = format!(
                "precompiled MSL pipeline `{}` rejected by device: {error}",
                self.label
            );
            #[cfg(test)]
            record_outcome(&[], Some(&error));
            return (Err(error), pipeline);
        }
        #[cfg(test)]
        record_outcome(&[self.vs, self.fs], None);
        (Ok(()), pipeline)
    }
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
                } else if let Some(v) = field.strip_prefix("sized=")
                    && !v.is_empty()
                {
                    for item in v.split(';') {
                        let parts: Vec<u32> =
                            item.split(',').filter_map(|p| p.parse().ok()).collect();
                        assert_eq!(parts.len(), 3, "stage_meta sized field");
                        sized.push((parts[0], parts[1], parts[2]));
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

    fn collect_rs_under(
        root: &Path,
        workspace: &Path,
        out: &mut Vec<(String, std::path::PathBuf)>,
    ) {
        if !root.exists() {
            return;
        }
        for entry in std::fs::read_dir(root).expect("read wgpu source dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                collect_rs_under(&path, workspace, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let rel = path
                    .strip_prefix(workspace)
                    .expect("path under workspace")
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push((rel, path));
            }
        }
    }

    /// Fork sources whose edits can change the checked-in MSL: the WGSL
    /// front end, the IR and its processing (validation, compaction,
    /// override substitution), the MSL back end and the Metal HAL that
    /// drives it. Directories are walked recursively.
    const FORK_MSL_SOURCES: &[&str] = &[
        "third_party/wgpu/naga/src/ir",
        "third_party/wgpu/naga/src/keywords",
        "third_party/wgpu/naga/src/front/wgsl",
        "third_party/wgpu/naga/src/proc",
        "third_party/wgpu/naga/src/valid",
        "third_party/wgpu/naga/src/compact",
        "third_party/wgpu/naga/src/back/mod.rs",
        "third_party/wgpu/naga/src/back/continue_forward.rs",
        "third_party/wgpu/naga/src/back/pipeline_constants.rs",
        "third_party/wgpu/naga/src/back/msl",
        "third_party/wgpu/wgpu-hal/src/metal",
    ];

    fn current_fork_msl_hash() -> u64 {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let mut files = Vec::new();
        for source in FORK_MSL_SOURCES {
            let path = workspace.join(source);
            assert!(
                path.exists(),
                "fork MSL hash source `{source}` is missing; update FORK_MSL_SOURCES"
            );
            if path.is_dir() {
                collect_rs_under(&path, workspace, &mut files);
            } else {
                files.push(((*source).to_owned(), path));
            }
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files.dedup_by(|a, b| a.0 == b.0);
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for (rel, path) in files {
            hash = super::super::precompiled::fnv1a_update(hash, rel.as_bytes());
            hash = super::super::precompiled::fnv1a_update(hash, &[0]);
            let mut bytes = std::fs::read(&path).expect("read wgpu fork source");
            // Line endings depend on checkout settings, not on the source.
            bytes.retain(|&byte| byte != b'\r');
            hash = super::super::precompiled::fnv1a_update(hash, &bytes);
            hash = super::super::precompiled::fnv1a_update(hash, &[0]);
        }
        hash
    }

    fn xcrun_metal_sdk_available(sdk: &str) -> bool {
        std::process::Command::new("xcrun")
            .args(["--sdk", sdk, "--find", "metal"])
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn validate_msl_syntax_with_xcrun(sdk: &str, std_flag: &str, entry: &str, msl: &str) {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new("xcrun")
            .args([
                "-sdk",
                sdk,
                "metal",
                "-x",
                "metal",
                std_flag,
                "-fsyntax-only",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn xcrun metal syntax check");
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(msl.as_bytes())
                .expect("write MSL source to xcrun");
        }
        let output = child.wait_with_output().expect("wait on xcrun metal");
        assert!(
            output.status.success(),
            "xcrun {sdk} {std_flag} syntax validation failed for `{entry}`:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn precompiled_msl_stages_are_fresh() {
        assert_eq!(
            generated::WGPU,
            locked_wgpu(),
            "wgpu changed: regenerate precompiled MSL (run regenerate_precompiled_msl)"
        );
        assert_eq!(
            generated::FORK_MSL_HASH,
            current_fork_msl_hash(),
            "fork WGSL front end, IR passes, MSL back end or Metal HAL changed: \
             run regenerate_precompiled_msl"
        );
        assert_eq!(
            generated::MSL_LANG_VERSION,
            REQUIRED_MSL_VERSION,
            "target MSL language version changed or missing: run regenerate_precompiled_msl"
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

        for stage in generated::STAGES {
            let start = stage.offset as usize;
            let end = start
                .checked_add(stage.len as usize)
                .expect("MSL stage span fits usize");
            let slice = raw
                .get(start..end)
                .expect("MSL stage slice stays inside decompressed archive");
            let msl = std::str::from_utf8(slice).expect("MSL stage is valid UTF-8");
            assert!(
                msl.contains("// language: metal3.1"),
                "precompiled MSL stage `{}` must target metal3.1; run regenerate_precompiled_msl",
                stage.entry
            );
        }

        for program in super::super::naga_ir::tests::builtin_programs() {
            let hash = super::super::precompiled::fnv1a(program.source.as_bytes());
            let module = naga::front::wgsl::parse_str(&program.source)
                .unwrap_or_else(|err| panic!("parse `{}` WGSL: {err:?}", program.slug));
            let stages: Vec<&MslStage> = generated::STAGES
                .iter()
                .filter(|stage| stage.source_hash == hash)
                .collect();
            assert!(
                stages.len() >= 2,
                "missing precompiled MSL stages for built-in shader `{}` (hash {hash:#018x}); \
                 run regenerate_precompiled_msl",
                program.slug
            );
            let inherited_unused: &[&str] = match program.slug {
                // `quad` includes `wgsl::MOTION` for helper functions; `motion` owns `motion_eval_*`.
                "quad" => &["motion_eval_vs", "motion_eval_fs"],
                // `group_blend` includes `wgsl::LAYER` for shared structs/vs; `group` owns `fs_main`.
                "group_blend" => &["fs_main"],
                _ => &[],
            };
            for &unused in inherited_unused {
                assert!(
                    module.entry_points.iter().any(|p| p.name == unused),
                    "stale inherited_unused `{unused}` for `{}`",
                    program.slug
                );
            }
            for point in &module.entry_points {
                if inherited_unused.contains(&point.name.as_str()) {
                    continue;
                }
                assert!(
                    stages.iter().any(|stage| stage.entry == point.name),
                    "missing precompiled MSL entry `{}` for `{}`; run regenerate_precompiled_msl",
                    point.name,
                    program.slug
                );
            }
        }
    }

    /// Every unique precompiled stage passes `metal -fsyntax-only` for the
    /// macOS and iOS simulator SDKs at the shipped MSL floor. Spawns one
    /// `xcrun metal` per stage and SDK, so it is opt-in; CI's macOS job runs
    /// it. A missing SDK fails the test instead of skipping it.
    #[test]
    #[ignore = "spawns ~160 `xcrun metal` processes; run on macOS with Xcode installed"]
    fn precompiled_msl_stages_pass_metal_syntax_check() {
        if !cfg!(target_os = "macos") {
            panic!("the Metal syntax check needs macOS with Xcode");
        }
        let mut raw = Vec::new();
        let mut decoder = flate2::read::ZlibDecoder::new(COMPRESSED_MSL);
        std::io::Read::read_to_end(&mut decoder, &mut raw)
            .expect("precompiled MSL archive decompresses");
        let mut unique = BTreeMap::<(u32, u32), &str>::new();
        for stage in generated::STAGES {
            unique
                .entry((stage.offset, stage.len))
                .or_insert(stage.entry);
        }
        let std_flag = format!(
            "-std=metal{}.{}",
            REQUIRED_MSL_VERSION.0, REQUIRED_MSL_VERSION.1
        );
        for sdk in ["macosx", "iphonesimulator"] {
            if xcrun_metal_sdk_available(sdk) {
                for ((offset, len), entry) in &unique {
                    let slice = &raw[*offset as usize..(*offset as usize + *len as usize)];
                    let msl = std::str::from_utf8(slice).expect("unique MSL slice is UTF-8");
                    validate_msl_syntax_with_xcrun(sdk, &std_flag, entry, msl);
                }
            } else {
                panic!(
                    "`xcrun --sdk {sdk} --find metal` failed: install Xcode (and its Metal \
                     toolchain) to validate precompiled MSL"
                );
            }
        }
    }

    #[test]
    fn scene_wgpu_painter_decompresses_msl_once_per_build_and_zero_on_warm_hit() {
        let _gpu_guard = crate::test_gpu::lock();
        let gpu = crate::test_gpu::fresh_context();
        let device = nana_gpu::__framework::device(&gpu);
        assert_eq!(
            archive_resident_bytes(),
            0,
            "uncompressed MSL buffer must not stay resident before painter creation"
        );
        // Empty unused ArchiveScope must not decompress.
        let before_unused = decompression_count();
        {
            let _unused = ArchiveScope::new();
        }
        assert_eq!(decompression_count(), before_unused);

        let before = decompression_count();
        let painter =
            crate::SceneWgpuPainter::try_new(&gpu, nana_gpu::GpuTextureFormat::BGRA8_UNORM_SRGB)
                .expect("build scene painter");
        assert_eq!(
            archive_resident_bytes(),
            0,
            "uncompressed MSL buffer must drop back to 0 B after painter creation"
        );
        if supports_precompiled_msl(device) && !bypassing() {
            assert_eq!(
                decompression_count() - before,
                1,
                "SceneWgpuPainter creation must decompress stages.msl.zz exactly once across all worker threads"
            );
            // Warm SharedPipelineRegistry hit on the same device must perform 0 decompressions.
            let warm_before = decompression_count();
            let warm_painter = crate::SceneWgpuPainter::try_new(
                &gpu,
                nana_gpu::GpuTextureFormat::BGRA8_UNORM_SRGB,
            )
            .expect("build warm scene painter");
            assert_eq!(
                decompression_count() - warm_before,
                0,
                "warm SharedPipelineRegistry hit must perform 0 MSL archive decompressions"
            );
            assert_eq!(archive_resident_bytes(), 0);
            drop(warm_painter);
        }
        drop(painter);
        assert_eq!(archive_resident_bytes(), 0);
    }

    #[cfg(feature = "charts")]
    #[test]
    fn chart_pipeline_decompresses_msl_at_most_once() {
        let _gpu_guard = crate::test_gpu::lock();
        let gpu = crate::test_gpu::fresh_context();
        let device = nana_gpu::__framework::device(&gpu);
        let motion_layout =
            super::super::motion::motion_bind_layout_with_policy(device, Some(gpu.policy()));
        let before = decompression_count();
        let chart = super::super::chart::ChartPipeline::try_new(
            device,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            &motion_layout,
        )
        .expect("build chart pipeline");
        assert_eq!(archive_resident_bytes(), 0);
        if supports_precompiled_msl(device) && !bypassing() {
            assert!(
                decompression_count() - before <= 1,
                "ChartPipeline::try_new must decompress stages.msl.zz at most once across all 6 chart pipelines"
            );
        }
        drop(chart);
    }

    #[test]
    fn older_msl_version_below_floor_is_rejected() {
        let err = ensure_msl_version((3, 0), "msl.floor.probe").unwrap_err();
        assert!(err.contains("macOS 14.0+ / iOS 17.0+"), "{err}");
        let err24 = ensure_msl_version((2, 4), "msl.floor.probe").unwrap_err();
        assert!(err24.contains("2.4"), "{err24}");
        ensure_msl_version((3, 1), "msl.floor.probe").expect("MSL 3.1 satisfies floor");
        ensure_msl_version((3, 2), "msl.floor.probe").expect("MSL 3.2 satisfies floor");
    }

    /// Builds the painter and every pipeline the product creates lazily
    /// (destinations, motion, GPU views, output transfers, hosted
    /// renderers): the set `regenerate_precompiled_msl` captures.
    fn build_every_builtin_pipeline(gpu: &nana_gpu::GpuContext) {
        let gpu = gpu.clone();
        let device = nana_gpu::__framework::device(&gpu);

        let painter =
            crate::SceneWgpuPainter::try_new(&gpu, nana_gpu::GpuTextureFormat::BGRA8_UNORM_SRGB)
                .expect("build painter for msl regen");
        let motion_layout =
            super::super::motion::motion_bind_layout_with_policy(device, Some(gpu.policy()));
        #[cfg(feature = "charts")]
        let _chart = super::super::chart::ChartPipeline::try_new(
            device,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            &motion_layout,
        )
        .expect("build chart pipeline for msl regen");
        super::super::dest::DestPipelines::prewarm_all_for_msl_regen(device, gpu.policy());
        super::super::motion::prewarm_for_msl_regen(device, &motion_layout);
        crate::default_gpu_view::prewarm_for_msl_regen(&gpu);
        let _t1 = crate::output::transfer::OutputTransfer::new(
            &gpu,
            nana_gpu::GpuTextureFormat::BGRA8_UNORM_SRGB,
            crate::output::transfer::TransferOp::Blit { source_srgb: false },
        );
        let _t2 = crate::output::transfer::OutputTransfer::new(
            &gpu,
            nana_gpu::GpuTextureFormat::BGRA8_UNORM,
            crate::output::transfer::TransferOp::Blit { source_srgb: true },
        );
        let _t3 = crate::output::transfer::OutputTransfer::new(
            &gpu,
            nana_gpu::GpuTextureFormat::BGRA8_UNORM,
            crate::output::transfer::TransferOp::Blit { source_srgb: false },
        );
        #[cfg(feature = "hosted")]
        crate::output::window_output::prewarm_for_msl_regen(&gpu);
        #[cfg(feature = "hosted")]
        crate::native_content::NativeContentRenderer::prewarm_for_msl_regen(&gpu);
        drop(painter);
    }

    /// The production path: every built-in pipeline builds from the
    /// precompiled MSL with no WGSL fallback, as in a release without
    /// `runtime-wgsl`, and every checked-in stage is used by some pipeline.
    #[test]
    fn every_builtin_pipeline_builds_from_precompiled_msl_without_fallback() {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                FORBID_WGSL_FALLBACK.store(false, Ordering::SeqCst);
                *OUTCOMES
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            }
        }
        let _gpu_guard = crate::test_gpu::lock();
        // A device of its own: the shared one's pipeline registry may hold
        // pipelines other tests built through the WGSL fallback.
        let gpu = crate::test_gpu::fresh_context();
        let device = nana_gpu::__framework::device(&gpu);
        assert!(
            supports_precompiled_msl(device),
            "this test needs a Metal device with MSL {}.{}",
            REQUIRED_MSL_VERSION.0,
            REQUIRED_MSL_VERSION.1
        );
        let _reset = Reset;
        *OUTCOMES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((BTreeSet::new(), Vec::new()));
        FORBID_WGSL_FALLBACK.store(true, Ordering::SeqCst);

        build_every_builtin_pipeline(&gpu);

        let (built, failures) = OUTCOMES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .expect("outcomes recorded");
        assert!(
            failures.is_empty(),
            "precompiled MSL failed without WGSL fallback:\n{}",
            failures.join("\n")
        );
        // Chart stages stay in the table under `test` so the freshness check
        // sees them, but their pipeline is built only with `charts`.
        let unbuildable: BTreeSet<u64> = super::super::naga_ir::tests::builtin_programs()
            .iter()
            .filter(|program| program.slug == "chart" && !cfg!(feature = "charts"))
            .map(|program| super::super::precompiled::fnv1a(program.source.as_bytes()))
            .collect();
        let unused: Vec<String> = generated::STAGES
            .iter()
            .filter(|stage| !unbuildable.contains(&stage.source_hash))
            .filter(|stage| !built.contains(&(*stage as *const MslStage as usize)))
            .map(|stage| format!("{:#018x} `{}`", stage.source_hash, stage.entry))
            .collect();
        assert!(
            unused.is_empty(),
            "precompiled MSL stages no pipeline built:\n{}",
            unused.join("\n")
        );
    }

    /// An array literal for a `MslStage` field laid out the way rustfmt does,
    /// so `cargo fmt --check` keeps the generated file: one line while the
    /// items fit its 60-column array width, else one item per line.
    fn field_array(items: &[String]) -> String {
        let inline = items.join(", ");
        if inline.len() <= 60 {
            return format!("[{inline}]");
        }
        let mut out = String::from("[\n");
        for item in items {
            out.push_str(&format!("            {item},\n"));
        }
        out.push_str("        ]");
        out
    }

    #[test]
    #[ignore = "regenerates checked-in Metal MSL archive after WGSL shader or wgpu edits"]
    fn regenerate_precompiled_msl() {
        // A runtime check, not `const { assert!(..) }`: the ignored test
        // still compiles under `--features gpu`, which CI tests.
        if !(cfg!(feature = "hosted") && cfg!(feature = "charts")) {
            panic!(
                "run regenerate_precompiled_msl with --features full so all feature-gated shaders are emitted"
            );
        }

        struct BypassGuard;
        impl Drop for BypassGuard {
            fn drop(&mut self) {
                BYPASS_PRECOMPILED.store(false, Ordering::SeqCst);
            }
        }
        struct VersionGuard;
        impl Drop for VersionGuard {
            fn drop(&mut self) {
                wgpu::hal::metal::Device::set_precompile_msl_lang_version(None);
            }
        }
        BYPASS_PRECOMPILED.store(true, Ordering::SeqCst);
        let _bypass = BypassGuard;
        wgpu::hal::metal::Device::set_precompile_msl_lang_version(Some(REQUIRED_MSL_VERSION));
        let _version = VersionGuard;

        let _ = log::set_boxed_logger(Box::new(StageLogger));
        log::set_max_level(log::LevelFilter::Debug);
        CAPTURED.lock().unwrap().clear();

        let _gpu_guard = crate::test_gpu::lock();
        let gpu = crate::test_gpu::fresh_context();
        assert!(
            is_metal(nana_gpu::__framework::device(&gpu)),
            "regenerate_precompiled_msl requires Metal"
        );
        build_every_builtin_pipeline(&gpu);

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
                .collect::<Vec<_>>();
            let consts = field_array(&consts);
            let sized = stage
                .sized_bindings
                .iter()
                .map(|(g, b, i)| format!("({g}, {b}, {i})"))
                .collect::<Vec<_>>();
            let sized = field_array(&sized);
            rows.push_str(&format!(
                "    MslStage {{\n        \
                    source_hash: {:#018x},\n        \
                    entry: {:?},\n        \
                    msl_entry: {:?},\n        \
                    constants: &{consts},\n        \
                    workgroup_size: {:?},\n        \
                    sized_bindings: &{sized},\n        \
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
        let fork_msl_hash = current_fork_msl_hash();
        let (maj, min) = REQUIRED_MSL_VERSION;
        let uncompressed_len = blob.len();
        let generated_rs = format!(
            "// Generated by `cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_msl --test-threads=1`.\n\n\
             use super::MslStage;\n\n\
             #[cfg(test)]\n\
             pub(super) const WGPU: &str = {wgpu_ver:?};\n\n\
             #[cfg(test)]\n\
             pub(super) const FORK_MSL_HASH: u64 = {fork_msl_hash:#018x};\n\n\
             #[cfg(test)]\n\
             pub(super) const MSL_LANG_VERSION: (u8, u8) = ({maj}, {min});\n\n\
             pub(super) const UNCOMPRESSED_LEN: usize = {uncompressed_len};\n\n\
             pub(super) static STAGES: &[MslStage] = &[\n\
             {rows}\
             ];\n"
        );
        std::fs::write(out_dir.join("generated.rs"), generated_rs).expect("write generated.rs");
    }
}
