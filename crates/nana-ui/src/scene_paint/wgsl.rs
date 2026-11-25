//! The scene's WGSL files, one copy of each, without their comments.
//!
//! A program is several files in a row: the colour maths, the motion
//! evaluator, a family's own stages. Spelled with `concat!`, each program
//! embedded its own copy of every file it shares (`color.wgsl` nine times,
//! `motion.wgsl` twice), 309 KiB of text for 163 KiB of source. The files
//! are embedded once here, and a program is joined when its module is
//! created, which is once per device.
//!
//! What is embedded is the file as the shader compiler needs it: comments
//! and trailing blanks are taken out when this crate is compiled
//! ([`stripped`]), about a fifth of the text. Line breaks and indentation
//! stay, so a compile error still names the line of the file, and the
//! material programs that rewrite a statement by its spelling still find
//! it.

/// `source` without its `//` comments and trailing blanks, as a constant.
macro_rules! stripped {
    ($source:expr) => {{
        const SOURCE: &str = $source;
        const LEN: usize = $crate::scene_paint::wgsl::strip::<0>(SOURCE.as_bytes()).1;
        #[allow(long_running_const_eval)]
        const BYTES: [u8; LEN] = $crate::scene_paint::wgsl::strip::<LEN>(SOURCE.as_bytes()).0;
        match ::core::str::from_utf8(&BYTES) {
            Ok(text) => text,
            Err(_) => panic!("stripping WGSL split a character"),
        }
    }};
}
pub(super) use stripped;

/// `source` with every `//` comment and the blanks before a line break
/// taken out, written into the first bytes of an array of `N`, and how many
/// bytes that is. With `N` of zero nothing is written and only the count is
/// right, which is how [`stripped`] sizes the array.
///
/// WGSL has no string literals, so `//` starts a comment wherever it is;
/// none of the scene's files uses a block comment. Cutting at an ASCII
/// byte and dropping ASCII blanks cannot split a character.
pub(super) const fn strip<const N: usize>(source: &[u8]) -> ([u8; N], usize) {
    let mut out = [0u8; N];
    let mut written = 0;
    let mut index = 0;
    while index < source.len() {
        // Where the line's last byte that is not a blank ends.
        let mut kept = written;
        while index < source.len() && source[index] != b'\n' {
            let byte = source[index];
            if byte == b'/' && index + 1 < source.len() && source[index + 1] == b'/' {
                while index < source.len() && source[index] != b'\n' {
                    index += 1;
                }
                break;
            }
            if written < N {
                out[written] = byte;
            }
            written += 1;
            if !matches!(byte, b' ' | b'\t' | b'\r') {
                kept = written;
            }
            index += 1;
        }
        written = kept;
        if index < source.len() {
            if written < N {
                out[written] = b'\n';
            }
            written += 1;
            index += 1;
        }
    }
    (out, written)
}

pub(super) const BACKDROP_BLUR: &str = stripped!(include_str!("shader/backdrop_blur.wgsl"));
pub(super) const BACKDROP_COMPOSITE: &str =
    stripped!(include_str!("shader/backdrop_composite.wgsl"));
pub(super) const BACKDROP_COPY: &str = stripped!(include_str!("shader/backdrop_copy.wgsl"));
pub(super) const BLIT: &str = stripped!(include_str!("shader/blit.wgsl"));
#[cfg(any(test, feature = "charts"))]
pub(super) const CHART: &str = stripped!(include_str!("shader/chart.wgsl"));
pub(super) const COLOR: &str = stripped!(include_str!("shader/color.wgsl"));
pub(super) const LAYER: &str = stripped!(include_str!("shader/layer.wgsl"));
pub(super) const LAYER_BLEND: &str = stripped!(include_str!("shader/layer_blend.wgsl"));
pub(crate) const MOTION: &str = stripped!(include_str!("shader/motion.wgsl"));

const fn replace_motion_group_digit<const N: usize>(src: &[u8], digit: u8) -> [u8; N] {
    let mut out = [0u8; N];
    let pat = b"@group(1)";
    let mut i = 0;
    while i < N {
        let mut matched = i + pat.len() <= N;
        if matched {
            let mut j = 0;
            while j < pat.len() {
                if src[i + j] != pat[j] {
                    matched = false;
                    break;
                }
                j += 1;
            }
        }
        if matched {
            let mut j = 0;
            while j < pat.len() {
                out[i + j] = if j == 7 { digit } else { pat[j] };
                j += 1;
            }
            i += pat.len();
        } else {
            out[i] = src[i];
            i += 1;
        }
    }
    out
}

const MOTION_LEN: usize = MOTION.len();
#[allow(long_running_const_eval)]
const MOTION_G2_BYTES: [u8; MOTION_LEN] =
    replace_motion_group_digit::<MOTION_LEN>(MOTION.as_bytes(), b'2');
pub(super) const MOTION_G2: &str = match ::core::str::from_utf8(&MOTION_G2_BYTES) {
    Ok(s) => s,
    Err(_) => panic!("MOTION_G2 UTF-8"),
};
pub(super) const PATH: &str = stripped!(include_str!("shader/path.wgsl"));
pub(super) const QUAD: &str = stripped!(include_str!("shader/quad.wgsl"));
pub(super) const QUAD_PAINT: &str = stripped!(include_str!("shader/quad_paint.wgsl"));
pub(super) const QUAD_PAINT_DATA: &str = stripped!(include_str!("shader/quad_paint_data.wgsl"));
pub(super) const QUAD_SOLID: &str = stripped!(include_str!("shader/quad_solid.wgsl"));
pub(super) const TEXT_ATLAS: &str = stripped!(include_str!("shader/text_atlas.wgsl"));
pub(super) const TRIANGLE: &str = stripped!(include_str!("shader/triangle.wgsl"));
pub(super) const TRIANGLE_SOLID: &str = stripped!(include_str!("shader/triangle_solid.wgsl"));
pub(super) const VERTEX: &str = stripped!(include_str!("shader/vertex.wgsl"));

/// `files` in a row with a line break between each.
#[cfg(any(test, feature = "runtime-wgsl", target_os = "windows"))]
pub(super) fn joined(files: &[&str]) -> String {
    files.join("\n")
}

#[cfg(not(any(test, feature = "runtime-wgsl", target_os = "windows")))]
pub(crate) use super::precompiled::{fnv1a, fnv1a_concat, fnv1a_joined};

/// Built-in WGSL program: retains source when tests, `runtime-wgsl`, or
/// Windows DXIL validation need it; hashes the source at compile time
/// otherwise so raw built-in WGSL strings do not enter the binary.
#[cfg(any(test, feature = "runtime-wgsl", target_os = "windows"))]
macro_rules! program {
    ($label:expr, $source:expr) => {
        $crate::scene_paint::wgsl::Program::new($label, $source)
    };
}
#[cfg(not(any(test, feature = "runtime-wgsl", target_os = "windows")))]
macro_rules! program {
    ($label:expr, $source:expr) => {{
        #[allow(long_running_const_eval)]
        const HASH: u64 = $crate::scene_paint::wgsl::fnv1a(($source).as_bytes());
        $crate::scene_paint::wgsl::Program::from_hash($label, HASH)
    }};
}

/// Built-in WGSL program assembled by joining `files` with `'\n'`.
#[cfg(any(test, feature = "runtime-wgsl", target_os = "windows"))]
macro_rules! program_joined {
    ($label:expr, [$($file:expr),+ $(,)?]) => {
        $crate::scene_paint::wgsl::Program::joined($label, &[$($file),+])
    };
}
#[cfg(not(any(test, feature = "runtime-wgsl", target_os = "windows")))]
macro_rules! program_joined {
    ($label:expr, [$($file:expr),+ $(,)?]) => {{
        #[allow(long_running_const_eval)]
        const HASH: u64 = $crate::scene_paint::wgsl::fnv1a_joined(&[$($file),+]);
        $crate::scene_paint::wgsl::Program::from_hash($label, HASH)
    }};
}

/// Built-in WGSL program assembled by concatenating `parts` directly.
#[cfg(any(test, feature = "runtime-wgsl", target_os = "windows"))]
macro_rules! program_concat {
    ($label:expr, [$($part:expr),+ $(,)?]) => {
        $crate::scene_paint::wgsl::Program::new($label, [$($part),+].concat())
    };
}
#[cfg(not(any(test, feature = "runtime-wgsl", target_os = "windows")))]
macro_rules! program_concat {
    ($label:expr, [$($part:expr),+ $(,)?]) => {{
        #[allow(long_running_const_eval)]
        const HASH: u64 = $crate::scene_paint::wgsl::fnv1a_concat(&[$($part),+]);
        $crate::scene_paint::wgsl::Program::from_hash($label, HASH)
    }};
}

pub(crate) use {program, program_concat, program_joined};

/// A framework program, handed to the device as naga IR one entry point at a
/// time.
///
/// Given a WGSL module with several entry points, wgpu compiles every
/// pipeline stage from a copy of the whole module that it cuts down to the
/// stage's entry point, and the pipeline keeps the whole module alive (on
/// Metal its IR, since MSL is written per pipeline). Here the program is
/// parsed and validated once, when its first stage is wanted, and each entry
/// point becomes a module of its own that holds only what that entry point
/// reaches (`naga::compact`). Pipelines sharing an entry point share its
/// module, the device validates only the cut-down IR, and a pipeline keeps
/// only its own stages' code.
///
/// A program that does not parse or validate here (none of the framework's
/// does; the tests compile them all) goes to the device as WGSL, whole,
/// which then reports the error as it always has.
///
/// A framework program ([`Self::new`]) is compiled without loop bounding:
/// every loop in the framework's shaders ends whatever the data, so the
/// counter wgpu adds to each loop is dead weight in every pixel. Bounds and
/// integer-division checks stay: indices and divisors come from the CPU's
/// buffers, and Metal has no hardware robustness behind them. A material
/// program ([`Self::material`]) carries an application's or skin's WGSL and
/// keeps every check.
pub(crate) struct Program {
    label: &'static str,
    /// An application's or skin's WGSL is in it: every runtime check stays.
    material: bool,
    #[cfg(any(test, feature = "runtime-wgsl", target_os = "windows"))]
    source: std::borrow::Cow<'static, str>,
    source_hash: u64,
    #[cfg(any(test, feature = "runtime-wgsl"))]
    ir: std::cell::OnceCell<Option<naga::Module>>,
    #[cfg(any(test, feature = "runtime-wgsl", not(target_vendor = "apple")))]
    stages: std::cell::RefCell<Vec<(String, Stage)>>,
    #[cfg(any(test, feature = "runtime-wgsl"))]
    whole: std::cell::OnceCell<wgpu::ShaderModule>,
    /// DXIL compiled from this program, while the device takes it; cleared
    /// when the device refuses it.
    #[cfg(target_os = "windows")]
    precompiled: std::cell::Cell<Option<&'static super::precompiled::Family>>,
}

/// One entry point's module, prepared for [`Program::try_pipeline`].
#[cfg(any(test, feature = "runtime-wgsl", not(target_vendor = "apple")))]
#[derive(Clone)]
pub(crate) struct Stage {
    pub(crate) module: wgpu::ShaderModule,
    /// The overrides the module still declares; `None` for the whole program.
    overrides: Option<Vec<String>>,
}

#[cfg(any(test, feature = "runtime-wgsl", not(target_vendor = "apple")))]
impl Stage {
    /// `constants` less those the stage does not use: a cut-down module no
    /// longer declares them, and a pipeline may only set what its module
    /// declares.
    pub(super) fn constants<'a>(&self, constants: &[(&'a str, f64)]) -> Vec<(&'a str, f64)> {
        constants
            .iter()
            .copied()
            .filter(|(name, _)| {
                self.overrides
                    .as_ref()
                    .is_none_or(|declared| declared.iter().any(|declared| declared == name))
            })
            .collect()
    }
}

impl Program {
    #[cfg(any(test, feature = "runtime-wgsl", target_os = "windows"))]
    pub(crate) fn new(
        label: &'static str,
        source: impl Into<std::borrow::Cow<'static, str>>,
    ) -> Self {
        let source = source.into();
        let source_hash = super::precompiled::fnv1a(source.as_bytes());
        Self {
            label,
            material: false,
            source,
            source_hash,
            #[cfg(any(test, feature = "runtime-wgsl"))]
            ir: std::cell::OnceCell::new(),
            #[cfg(any(test, feature = "runtime-wgsl", not(target_vendor = "apple")))]
            stages: std::cell::RefCell::new(Vec::new()),
            #[cfg(any(test, feature = "runtime-wgsl"))]
            whole: std::cell::OnceCell::new(),
            #[cfg(target_os = "windows")]
            precompiled: std::cell::Cell::new(None),
        }
    }

    #[cfg(not(any(test, feature = "runtime-wgsl", target_os = "windows")))]
    pub(crate) fn from_hash(label: &'static str, source_hash: u64) -> Self {
        Self {
            label,
            material: false,
            source_hash,
            #[cfg(any(test, feature = "runtime-wgsl", not(target_vendor = "apple")))]
            stages: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// A program holding an application's or skin's WGSL, compiled with
    /// every runtime check.
    #[cfg(feature = "runtime-wgsl")]
    pub(super) fn material(
        label: &'static str,
        source: impl Into<std::borrow::Cow<'static, str>>,
    ) -> Self {
        Self {
            material: true,
            ..Self::new(label, source)
        }
    }

    /// The program, whose pipelines load `family`'s DXIL when `runs` (the
    /// device takes precompiled stages, [`super::precompiled::runs_on`]) and
    /// the family was compiled from this very program laid out as `layout`.
    /// Only a Windows build names a family, which keeps the DXIL out of
    /// other targets' binaries.
    #[cfg(target_os = "windows")]
    pub(super) fn precompiled(
        self,
        family: &'static super::precompiled::Family,
        runs: bool,
        layout: impl FnOnce() -> u64,
    ) -> Self {
        if runs && family.fresh(&self.source, layout()) {
            self.precompiled.set(Some(family));
        }
        self
    }

    /// The pipeline `build` makes from the stages for `vertex` and
    /// `fragment`, with `constants` for the fragment stage: precompiled
    /// stages when available, else cut from the program.
    /// `build` gets the constants the fragment module takes (none for a
    /// precompiled stage, which was compiled with them).
    pub(crate) fn try_pipeline(
        &self,
        device: &wgpu::Device,
        vertex: &str,
        fragment: &str,
        constants: &[(&str, f64)],
        build: impl Fn(
            &wgpu::ShaderModule,
            &str,
            &wgpu::ShaderModule,
            &str,
            &[(&str, f64)],
        ) -> wgpu::RenderPipeline,
    ) -> Result<wgpu::RenderPipeline, super::validate::ScenePaintError> {
        self.try_pipeline_inner(
            wgsl_fallback_allowed(),
            device,
            vertex,
            fragment,
            constants,
            &build,
        )
    }

    /// Cached variant of [`Self::try_pipeline`]: preflights shader assets before
    /// querying `policy`'s pipeline registry so missing or incompatible stages fail
    /// without panicking inside the cache closure.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_cached_pipeline(
        &self,
        policy: Option<&nana_gpu::GpuDeviceState>,
        key: impl FnOnce(&nana_gpu::GpuDeviceState) -> nana_gpu::PipelineKey,
        device: &wgpu::Device,
        vertex: &str,
        fragment: &str,
        constants: &[(&str, f64)],
        build: impl Fn(
            &wgpu::ShaderModule,
            &str,
            &wgpu::ShaderModule,
            &str,
            &[(&str, f64)],
        ) -> wgpu::RenderPipeline,
    ) -> Result<wgpu::RenderPipeline, super::validate::ScenePaintError> {
        let Some(policy) = policy else {
            return self.try_pipeline(device, vertex, fragment, constants, build);
        };
        let allow_wgsl_fallback = wgsl_fallback_allowed();
        // Every factory below returns its error instead of a placeholder
        // pipeline: the registry never stores a failure, so a later request
        // for the same key compiles again rather than sharing an invalid
        // pipeline. Shader assets (the MSL archive in particular) are only
        // touched inside the factory, so registry hits never inflate them.
        let flatten = |result: Result<
            Result<wgpu::RenderPipeline, super::validate::ScenePaintError>,
            nana_gpu::GpuError,
        >| {
            result.unwrap_or_else(|error| {
                Err(super::validate::ScenePaintError::ShaderUnavailable {
                    label: self.label,
                    detail: error.to_string(),
                })
            })
        };
        #[cfg(target_vendor = "apple")]
        {
            #[cfg(test)]
            let bypass = super::msl_precompiled::bypassing();
            #[cfg(not(test))]
            let bypass = false;

            if !self.material && !bypass {
                return flatten(nana_gpu::__framework::try_render_pipeline_state(
                    policy,
                    key(policy),
                    || {
                        // Registry check passed: attempt compilation.
                        // MSL preparation moves after registry hit check.
                        match super::msl_precompiled::prepare(
                            device,
                            self.label,
                            self.source_hash,
                            vertex,
                            fragment,
                            constants,
                        ) {
                            Ok(prepared) => {
                                let (res, pipeline) = prepared.build(device, &build);
                                match res {
                                    Ok(()) => return Ok(pipeline),
                                    Err(detail) => {
                                        self.precompiled_failed(allow_wgsl_fallback, detail)?
                                    }
                                }
                            }
                            Err(detail) => self.precompiled_failed(allow_wgsl_fallback, detail)?,
                        }
                        #[cfg(any(test, feature = "runtime-wgsl"))]
                        {
                            let vs = self.try_stage_wgsl(device, vertex)?;
                            let fs = self.try_stage_wgsl(device, fragment)?;
                            let constants = fs.constants(constants);
                            build_pipeline_scoped(device, self.label, "wgsl-cached", || {
                                build(&vs.module, vertex, &fs.module, fragment, &constants)
                            })
                        }
                        #[cfg(not(any(test, feature = "runtime-wgsl")))]
                        {
                            nana_diagnostics::event!(
                                nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED
                            );
                            Err(super::validate::ScenePaintError::ShaderUnavailable {
                                label: self.label,
                                detail: String::from("runtime WGSL shader compiler is disabled"),
                            })
                        }
                    },
                ));
            } else if self.material && !allow_wgsl_fallback {
                nana_diagnostics::event!(nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED);
                return Err(super::validate::ScenePaintError::ShaderUnavailable {
                    label: self.label,
                    detail: String::from(
                        "runtime material shaders require the `runtime-wgsl` feature",
                    ),
                });
            }
        }

        #[cfg(not(target_vendor = "apple"))]
        {
            if !self.material {
                #[cfg(target_os = "windows")]
                let has_dxil = self.precompiled.get().is_some_and(|family| {
                    family.stage(vertex, &[]).is_some()
                        && family.stage(fragment, constants).is_some()
                });
                #[cfg(not(target_os = "windows"))]
                let has_dxil = false;

                if !has_dxil
                    && !allow_wgsl_fallback
                    && (!super::naga_ir::has_stage(self.source_hash, vertex)
                        || !super::naga_ir::has_stage(self.source_hash, fragment))
                {
                    nana_diagnostics::event!(nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED);
                    return Err(super::validate::ScenePaintError::ShaderUnavailable {
                        label: self.label,
                        detail: format!(
                            "missing precompiled naga IR stages `{vertex}`/`{fragment}` for `{}` without `runtime-wgsl`",
                            self.label
                        ),
                    });
                }
            } else if !allow_wgsl_fallback {
                nana_diagnostics::event!(nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED);
                return Err(super::validate::ScenePaintError::ShaderUnavailable {
                    label: self.label,
                    detail: String::from(
                        "runtime material shaders require the `runtime-wgsl` feature",
                    ),
                });
            }
        }

        #[cfg(any(test, feature = "runtime-wgsl", not(target_vendor = "apple")))]
        {
            flatten(nana_gpu::__framework::try_render_pipeline_state(
                policy,
                key(policy),
                || {
                    // On Apple a built-in program only gets here when a test
                    // bypasses the precompiled MSL.
                    #[cfg(all(target_vendor = "apple", any(test, feature = "runtime-wgsl")))]
                    if !self.material {
                        #[cfg(test)]
                        if super::msl_precompiled::bypassing() {
                            return super::msl_precompiled::tests::recording_pipeline(
                                self.source_hash,
                                vertex,
                                fragment,
                                constants,
                                || {
                                    let vs = self.try_stage_wgsl(device, vertex)?;
                                    let fs = self.try_stage_wgsl(device, fragment)?;
                                    let constants = fs.constants(constants);
                                    build_pipeline_scoped(
                                        device,
                                        self.label,
                                        "wgsl-capture",
                                        || {
                                            build(
                                                &vs.module, vertex, &fs.module, fragment,
                                                &constants,
                                            )
                                        },
                                    )
                                },
                            );
                        }
                        let vs = self.try_stage_wgsl(device, vertex)?;
                        let fs = self.try_stage_wgsl(device, fragment)?;
                        let constants = fs.constants(constants);
                        return build_pipeline_scoped(device, self.label, "wgsl-apple", || {
                            build(&vs.module, vertex, &fs.module, fragment, &constants)
                        });
                    }
                    self.try_pipeline_inner(
                        allow_wgsl_fallback,
                        device,
                        vertex,
                        fragment,
                        constants,
                        &build,
                    )
                },
            ))
        }
        #[cfg(not(any(test, feature = "runtime-wgsl", not(target_vendor = "apple"))))]
        {
            let _ = (key, flatten);
            nana_diagnostics::event!(nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED);
            Err(super::validate::ScenePaintError::ShaderUnavailable {
                label: self.label,
                detail: String::from("runtime WGSL shader compiler is disabled"),
            })
        }
    }

    /// The precompiled MSL for a pipeline was missing or refused: an error
    /// without a WGSL fallback, else the fallback event and `Ok` to go on.
    #[cfg(target_vendor = "apple")]
    fn precompiled_failed(
        &self,
        allow_wgsl_fallback: bool,
        detail: String,
    ) -> Result<(), super::validate::ScenePaintError> {
        if !allow_wgsl_fallback {
            nana_diagnostics::event!(nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED);
            return Err(super::validate::ScenePaintError::ShaderUnavailable {
                label: self.label,
                detail,
            });
        }
        nana_diagnostics::event!(nana_diagnostics::framework::gpu::SHADER_PRECOMPILED_FALLBACK);
        Ok(())
    }

    fn try_pipeline_inner(
        &self,
        allow_wgsl_fallback: bool,
        device: &wgpu::Device,
        vertex: &str,
        fragment: &str,
        constants: &[(&str, f64)],
        build: &impl Fn(
            &wgpu::ShaderModule,
            &str,
            &wgpu::ShaderModule,
            &str,
            &[(&str, f64)],
        ) -> wgpu::RenderPipeline,
    ) -> Result<wgpu::RenderPipeline, super::validate::ScenePaintError> {
        #[cfg(target_os = "windows")]
        if let Some(family) = self.precompiled.get()
            && family.stage(vertex, &[]).is_some()
            && family.stage(fragment, constants).is_some()
        {
            match super::precompiled::pipeline(
                device,
                family,
                [(vertex, &[]), (fragment, constants)],
                |[vertex_mod, fragment_mod]| build(vertex_mod, vertex, fragment_mod, fragment, &[]),
            ) {
                Some(pipeline) => return Ok(pipeline),
                // Refused once, refused for every pipeline of the program.
                None => {
                    self.precompiled.set(None);
                    nana_diagnostics::event!(
                        nana_diagnostics::framework::gpu::SHADER_PRECOMPILED_FALLBACK
                    );
                }
            }
        }
        #[cfg(target_vendor = "apple")]
        if !self.material {
            #[cfg(test)]
            let bypass = super::msl_precompiled::bypassing();
            #[cfg(not(test))]
            let bypass = false;
            if !bypass {
                match super::msl_precompiled::prepare(
                    device,
                    self.label,
                    self.source_hash,
                    vertex,
                    fragment,
                    constants,
                ) {
                    Ok(prepared) => {
                        let (res, pipeline) = prepared.build(device, build);
                        match res {
                            Ok(()) => return Ok(pipeline),
                            Err(detail) => {
                                if !allow_wgsl_fallback {
                                    nana_diagnostics::event!(
                                        nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED
                                    );
                                    return Err(
                                        super::validate::ScenePaintError::ShaderUnavailable {
                                            label: self.label,
                                            detail,
                                        },
                                    );
                                }
                                nana_diagnostics::event!(
                                    nana_diagnostics::framework::gpu::SHADER_PRECOMPILED_FALLBACK
                                );
                            }
                        }
                    }
                    Err(detail) => {
                        if !allow_wgsl_fallback {
                            nana_diagnostics::event!(
                                nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED
                            );
                            return Err(super::validate::ScenePaintError::ShaderUnavailable {
                                label: self.label,
                                detail,
                            });
                        }
                        nana_diagnostics::event!(
                            nana_diagnostics::framework::gpu::SHADER_PRECOMPILED_FALLBACK
                        );
                    }
                }
            } else {
                #[cfg(test)]
                return super::msl_precompiled::tests::recording_pipeline(
                    self.source_hash,
                    vertex,
                    fragment,
                    constants,
                    || {
                        let vertex_stage =
                            self.try_stage_with_fallback(allow_wgsl_fallback, device, vertex)?;
                        let fragment_stage =
                            self.try_stage_with_fallback(allow_wgsl_fallback, device, fragment)?;
                        let constants = fragment_stage.constants(constants);
                        build_pipeline_scoped(device, self.label, "wgsl", || {
                            build(
                                &vertex_stage.module,
                                vertex,
                                &fragment_stage.module,
                                fragment,
                                &constants,
                            )
                        })
                    },
                );
            }
        }
        #[cfg(not(any(test, feature = "runtime-wgsl")))]
        #[cfg(target_vendor = "apple")]
        {
            let _ = (
                allow_wgsl_fallback,
                device,
                vertex,
                fragment,
                constants,
                build,
            );
            nana_diagnostics::event!(nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED);
            Err(super::validate::ScenePaintError::ShaderUnavailable {
                label: self.label,
                detail: format!(
                    "missing precompiled MSL stage for built-in shader `{}` or runtime material without `runtime-wgsl`",
                    self.label
                ),
            })
        }
        #[cfg(any(test, feature = "runtime-wgsl", not(target_vendor = "apple")))]
        {
            #[cfg(all(test, target_os = "windows"))]
            super::precompiled::tests::requesting(&self.source, vertex, fragment, constants);
            let vertex_stage = self.try_stage_with_fallback(allow_wgsl_fallback, device, vertex)?;
            let fragment_stage =
                self.try_stage_with_fallback(allow_wgsl_fallback, device, fragment)?;
            let constants = fragment_stage.constants(constants);
            build_pipeline_scoped(device, self.label, "pipeline", || {
                build(
                    &vertex_stage.module,
                    vertex,
                    &fragment_stage.module,
                    fragment,
                    &constants,
                )
            })
        }
    }

    /// The program [`joined`] from `files`.
    #[cfg(any(test, feature = "runtime-wgsl", target_os = "windows"))]
    pub(crate) fn joined(label: &'static str, files: &[&str]) -> Self {
        Self::new(label, joined(files))
    }

    /// The program's text, as the device would parse it.
    #[cfg(test)]
    pub(super) fn source(&self) -> &str {
        &self.source
    }

    #[cfg(any(test, feature = "runtime-wgsl"))]
    fn ir(&self) -> Option<&naga::Module> {
        self.ir
            .get_or_init(|| {
                let module = naga::front::wgsl::parse_str(&self.source).ok()?;
                // Compaction requires a valid module. The device validates
                // each stage again against its own capabilities.
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&module)
                .ok()?;
                Some(module)
            })
            .as_ref()
    }

    #[cfg(any(test, feature = "runtime-wgsl", not(target_vendor = "apple")))]
    fn stage_from_ir(&self, device: &wgpu::Device, ir: wgpu::naga::Module) -> Stage {
        let overrides = ir
            .overrides
            .iter()
            .filter_map(|(_, declared)| declared.name.clone())
            .collect();
        Stage {
            module: self.module(
                device,
                wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(ir)),
            ),
            overrides: Some(overrides),
        }
    }

    #[cfg(any(test, feature = "runtime-wgsl"))]
    fn try_stage_wgsl(
        &self,
        device: &wgpu::Device,
        entry: &str,
    ) -> Result<Stage, super::validate::ScenePaintError> {
        self.try_stage_wgsl_inner(device, entry, true)
    }

    #[cfg(any(test, feature = "runtime-wgsl"))]
    fn try_stage_wgsl_inner(
        &self,
        device: &wgpu::Device,
        entry: &str,
        scoped: bool,
    ) -> Result<Stage, super::validate::ScenePaintError> {
        if let Some((_, stage)) = self.stages.borrow().iter().find(|(name, _)| name == entry) {
            return Ok(stage.clone());
        }
        let build_stage = || match self.ir().and_then(|ir| cut(ir, entry)) {
            Some(ir) => self.stage_from_ir(device, ir),
            None => Stage {
                module: if scoped {
                    self.module(device, wgpu::ShaderSource::Wgsl(self.source.clone()))
                } else {
                    self.whole(device)
                },
                overrides: None,
            },
        };
        let stage = if scoped {
            build_pipeline_scoped(device, self.label, "wgsl stage", build_stage)?
        } else {
            build_stage()
        };
        self.stages
            .borrow_mut()
            .push((entry.to_owned(), stage.clone()));
        Ok(stage)
    }

    /// The module for the stage whose entry point is `entry`: precompiled
    /// compacted IR for built-in shaders, or cut from runtime WGSL when
    /// `runtime-wgsl` (or test validation) is enabled.
    #[cfg(any(test, feature = "runtime-wgsl", not(target_vendor = "apple")))]
    fn try_stage_with_fallback(
        &self,
        allow_wgsl_fallback: bool,
        device: &wgpu::Device,
        entry: &str,
    ) -> Result<Stage, super::validate::ScenePaintError> {
        self.try_stage_with_fallback_inner(allow_wgsl_fallback, device, entry, true)
    }

    #[cfg(any(test, feature = "runtime-wgsl", not(target_vendor = "apple")))]
    fn try_stage_with_fallback_inner(
        &self,
        allow_wgsl_fallback: bool,
        device: &wgpu::Device,
        entry: &str,
        scoped: bool,
    ) -> Result<Stage, super::validate::ScenePaintError> {
        if let Some((_, stage)) = self.stages.borrow().iter().find(|(name, _)| name == entry) {
            return Ok(stage.clone());
        }
        #[cfg(any(test, not(target_vendor = "apple")))]
        let precompiled = if !self.material {
            super::naga_ir::stage(self.source_hash, entry)
        } else {
            None
        };
        #[cfg(not(any(test, not(target_vendor = "apple"))))]
        let precompiled: Option<wgpu::naga::Module> = None;
        let stage = if let Some(ir) = precompiled {
            let built = if scoped {
                build_pipeline_scoped(device, self.label, "naga-ir stage", || {
                    self.stage_from_ir(device, ir)
                })
            } else {
                Ok(self.stage_from_ir(device, ir))
            };
            match built {
                Ok(stage) => stage,
                Err(error) => {
                    if allow_wgsl_fallback {
                        #[cfg(any(test, feature = "runtime-wgsl"))]
                        {
                            if cfg!(not(target_vendor = "apple")) && !self.material {
                                nana_diagnostics::event!(
                                    nana_diagnostics::framework::gpu::SHADER_PRECOMPILED_FALLBACK
                                );
                            }
                            return self.try_stage_wgsl_inner(device, entry, scoped);
                        }
                    }
                    nana_diagnostics::event!(nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED);
                    return Err(error);
                }
            }
        } else if allow_wgsl_fallback {
            #[cfg(any(test, feature = "runtime-wgsl"))]
            {
                if cfg!(not(target_vendor = "apple")) && !self.material {
                    nana_diagnostics::event!(
                        nana_diagnostics::framework::gpu::SHADER_PRECOMPILED_FALLBACK
                    );
                }
                return self.try_stage_wgsl_inner(device, entry, scoped);
            }
            #[cfg(not(any(test, feature = "runtime-wgsl")))]
            {
                nana_diagnostics::event!(nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED);
                return Err(super::validate::ScenePaintError::ShaderUnavailable {
                    label: self.label,
                    detail: format!(
                        "missing precompiled naga IR for built-in shader `{}` entry `{entry}`",
                        self.label
                    ),
                });
            }
        } else {
            nana_diagnostics::event!(nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED);
            return Err(super::validate::ScenePaintError::ShaderUnavailable {
                label: self.label,
                detail: format!(
                    "missing precompiled naga IR for built-in shader `{}` entry `{entry}`",
                    self.label
                ),
            });
        };
        self.stages
            .borrow_mut()
            .push((entry.to_owned(), stage.clone()));
        Ok(stage)
    }

    #[cfg(test)]
    pub(crate) fn try_stage(
        &self,
        device: &wgpu::Device,
        entry: &str,
    ) -> Result<Stage, super::validate::ScenePaintError> {
        self.try_stage_with_fallback_inner(true, device, entry, false)
    }

    #[cfg(any(test, feature = "runtime-wgsl"))]
    fn whole(&self, device: &wgpu::Device) -> wgpu::ShaderModule {
        self.whole
            .get_or_init(|| self.module(device, wgpu::ShaderSource::Wgsl(self.source.clone())))
            .clone()
    }

    /// `source` as a module, with the runtime checks [`Self`] describes.
    #[cfg(any(test, feature = "runtime-wgsl", not(target_vendor = "apple")))]
    fn module(&self, device: &wgpu::Device, source: wgpu::ShaderSource<'_>) -> wgpu::ShaderModule {
        let descriptor = wgpu::ShaderModuleDescriptor {
            label: Some(self.label),
            source,
        };
        if self.material {
            return device.create_shader_module(descriptor);
        }
        let mut checks = wgpu::ShaderRuntimeChecks::checked();
        checks.force_loop_bounding = false;
        // SAFETY: only loop bounding is off, and every loop in the
        // framework's shaders ends for any input (`loop_bounds_hold` checks
        // each one is counted against a bound it cannot overflow).
        unsafe { device.create_shader_module_trusted(descriptor, checks) }
    }
}

#[cfg(test)]
pub(super) static FORBID_WGSL_FALLBACK: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(any(test, feature = "runtime-wgsl", not(target_vendor = "apple")))]
fn build_pipeline_scoped<R>(
    device: &wgpu::Device,
    label: &'static str,
    phase: &str,
    build: impl FnOnce() -> R,
) -> Result<R, super::validate::ScenePaintError> {
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let output = build();
    let gpu_error = pollster::block_on(validation.pop())
        .or_else(|| pollster::block_on(internal.pop()))
        .or_else(|| pollster::block_on(oom.pop()));
    if let Some(error) = gpu_error {
        nana_diagnostics::event!(nana_diagnostics::framework::gpu::SHADER_BUILD_FAILED);
        return Err(super::validate::ScenePaintError::ShaderUnavailable {
            label,
            detail: format!("{phase} `{label}` rejected by device: {error}"),
        });
    }
    Ok(output)
}

/// Whether a built-in program may fall back to runtime WGSL when its
/// precompiled stage is missing or refused: only with `runtime-wgsl` (or
/// test validation, unless a test asks for the release behaviour).
fn wgsl_fallback_allowed() -> bool {
    if std::env::var_os("NANA_EXPECT_NO_RUNTIME_WGSL").is_some() {
        return false;
    }
    #[cfg(test)]
    if FORBID_WGSL_FALLBACK.load(std::sync::atomic::Ordering::SeqCst) {
        return false;
    }
    #[cfg(all(test, target_vendor = "apple"))]
    if super::msl_precompiled::wgsl_fallback_forbidden() {
        return false;
    }
    cfg!(any(test, feature = "runtime-wgsl"))
}

/// `module` with `entry` its only entry point and nothing it does not
/// reach; `None` when it has no such entry point.
#[cfg(any(test, feature = "runtime-wgsl"))]
pub(super) fn cut(module: &naga::Module, entry: &str) -> Option<naga::Module> {
    let mut module = module.clone();
    module.entry_points.retain(|point| point.name == entry);
    if module.entry_points.is_empty() {
        return None;
    }
    naga::compact::compact(&mut module, naga::compact::KeepUnused::No);
    Some(module)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_text(source: &str) -> String {
        let mut out = String::new();
        // The same pass, a byte at a time: sized once, then written.
        let len = strip::<0>(source.as_bytes()).1;
        let (bytes, written) = strip::<8192>(source.as_bytes());
        assert_eq!(written, len);
        out.push_str(std::str::from_utf8(&bytes[..written]).unwrap());
        out
    }

    #[test]
    fn comments_and_trailing_blanks_go_and_lines_stay_where_they_were() {
        let source = "// A file.\r\nfn a() -> f32 { // why\n    return 1.0 / 2.0;  \n}\n\n  // 注释\nlet b = 1;";
        assert_eq!(
            strip_text(source),
            "\nfn a() -> f32 {\n    return 1.0 / 2.0;\n}\n\n\nlet b = 1;"
        );
        assert_eq!(strip_text(""), "");
        assert_eq!(strip_text("//"), "");
        assert_eq!(strip_text("a /"), "a /");
    }

    /// A compile error names a line: it has to be the file's.
    #[test]
    fn every_file_keeps_its_lines_and_loses_its_comments() {
        for (stripped, file) in [
            (COLOR, include_str!("shader/color.wgsl")),
            (MOTION, include_str!("shader/motion.wgsl")),
            (QUAD_SOLID, include_str!("shader/quad_solid.wgsl")),
            (TEXT_ATLAS, include_str!("shader/text_atlas.wgsl")),
            (CHART, include_str!("shader/chart.wgsl")),
        ] {
            assert_eq!(stripped.lines().count(), file.lines().count());
            assert!(!stripped.contains("//"));
            assert!(stripped.len() < file.len());
            for (kept, line) in stripped.lines().zip(file.lines()) {
                assert_eq!(kept, line.split("//").next().unwrap().trim_end());
            }
        }
    }

    /// The scene's programs with more than one entry point, joined as their
    /// families join them.
    fn programs() -> Vec<(&'static str, Program)> {
        vec![
            (
                "quad",
                Program::joined(
                    "quad",
                    &[
                        MOTION,
                        COLOR,
                        QUAD,
                        VERTEX,
                        QUAD_PAINT_DATA,
                        QUAD_PAINT,
                        QUAD_SOLID,
                    ],
                ),
            ),
            (
                "mesh",
                Program::joined("mesh", &[TRIANGLE, TRIANGLE_SOLID, PATH, COLOR]),
            ),
            ("chart", Program::joined("chart", &[CHART, COLOR])),
            (
                "group blend",
                Program::joined("group blend", &[COLOR, LAYER, LAYER_BLEND]),
            ),
            ("group", Program::joined("group", &[COLOR, LAYER])),
            ("blit", Program::new("blit", BLIT)),
            ("motion", Program::new("motion", MOTION)),
            (
                "backdrop composite",
                Program::joined(
                    "backdrop composite",
                    &[COLOR, QUAD_PAINT_DATA, BACKDROP_COMPOSITE],
                ),
            ),
        ]
    }

    fn size(module: &naga::Module) -> usize {
        module
            .functions
            .iter()
            .map(|(_, function)| function.expressions.len())
            .chain(
                module
                    .entry_points
                    .iter()
                    .map(|point| point.function.expressions.len()),
            )
            .sum()
    }

    /// Every entry point of every program becomes a valid module of its
    /// own, smaller than the program, with nothing it does not reach.
    #[test]
    fn every_entry_point_cuts_to_a_valid_module_of_its_own() {
        for (name, program) in programs() {
            let ir = program
                .ir()
                .unwrap_or_else(|| panic!("{name} parses and validates"));
            assert!(ir.entry_points.len() > 1, "{name}");
            for point in &ir.entry_points {
                let stage = cut(ir, &point.name).unwrap();
                assert_eq!(stage.entry_points.len(), 1);
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&stage)
                .unwrap_or_else(|error| panic!("{name}::{}: {error:?}", point.name));
                assert!(
                    size(&stage) < size(ir),
                    "{name}::{} carries the whole program",
                    point.name
                );
            }
            assert!(cut(ir, "no_such_entry").is_none());
        }
    }

    /// Framework programs are compiled without loop bounding
    /// ([`Program::module`]), so every loop in their files has to end for
    /// any data: each is a `for` whose counter starts at a constant, steps
    /// by one and is compared strictly against its bound (or, as
    /// `n - 1u < bound`, one below it, or with `<=` against a constant), so
    /// it ends before the counter can overflow. The few others are listed with why they end.
    #[test]
    fn loop_bounds_hold() {
        // Loops that end for a reason the shape check cannot see.
        const ARGUED: &[(&str, &str)] = &[
            // Binary search: `end - idx` shrinks every turn.
            ("motion", "while (idx < end)"),
            // Float counters, bounded by `taps`, clamped to 1..8.
            ("host texture", "for (var i = 0.0; i < taps.x; i += 1.0)"),
            ("host texture", "for (var j = 0.0; j < taps.y; j += 1.0)"),
        ];
        let files = [
            ("backdrop blur", BACKDROP_BLUR),
            ("backdrop composite", BACKDROP_COMPOSITE),
            ("backdrop copy", BACKDROP_COPY),
            ("blit", BLIT),
            ("chart", CHART),
            ("color", COLOR),
            ("layer", LAYER),
            ("layer blend", LAYER_BLEND),
            ("motion", MOTION),
            ("path", PATH),
            ("quad", QUAD),
            ("quad paint", QUAD_PAINT),
            ("quad paint data", QUAD_PAINT_DATA),
            ("quad solid", QUAD_SOLID),
            ("text atlas", TEXT_ATLAS),
            ("triangle", TRIANGLE),
            ("triangle solid", TRIANGLE_SOLID),
            ("vertex", VERTEX),
            ("host texture", crate::gpu_texture::SOURCE),
        ];
        /// Where `word` stands as a word of `source`.
        fn words<'a>(source: &'a str, word: &'a str) -> impl Iterator<Item = usize> + 'a {
            let part = |c: char| c.is_alphanumeric() || c == '_';
            source
                .match_indices(word)
                .map(|(at, _)| at)
                .filter(move |&at| {
                    !source[..at].chars().next_back().is_some_and(part)
                        && !source[at + word.len()..].chars().next().is_some_and(part)
                })
        }
        let mut counted = 0;
        let mut argued = 0;
        for (name, source) in files {
            // The host texture's source keeps its comments.
            let source = &source
                .lines()
                .map(|line| line.split("//").next().unwrap())
                .collect::<Vec<_>>()
                .join(
                    "
",
                );
            assert_eq!(words(source, "loop").count(), 0, "{name} has a bare loop");
            for at in words(source, "while") {
                let line = source[at..].lines().next().unwrap().trim_end_matches(" {");
                assert!(ARGUED.contains(&(name, line)), "{name}: {line}");
                argued += 1;
            }
            for at in words(source, "for") {
                let open = at + source[at..].find('(').unwrap();
                let mut depth = 0;
                let close = open
                    + source[open..]
                        .char_indices()
                        .find(|&(_, c)| {
                            depth += match c {
                                '(' => 1,
                                ')' => -1,
                                _ => 0,
                            };
                            depth == 0
                        })
                        .unwrap()
                        .0;
                let line = &source[at..=close];
                if ARGUED.contains(&(name, line)) {
                    argued += 1;
                    continue;
                }
                let parts: Vec<&str> = source[open + 1..close].split(';').map(str::trim).collect();
                let [init, condition, step] = parts[..] else {
                    panic!("{name}: {line}");
                };
                let counter = init
                    .strip_prefix("var ")
                    .and_then(|rest| rest.split([' ', ':', '=']).next())
                    .unwrap_or_else(|| panic!("{name}: {line}"));
                let start = init.rsplit('=').next().unwrap().trim();
                assert!(
                    start.trim_end_matches('u').parse::<i64>().is_ok(),
                    "{name}: {line} starts at a constant"
                );
                assert!(
                    [
                        format!("{counter} = {counter} + 1"),
                        format!("{counter} = {counter} + 1u"),
                        format!("{counter} += 1"),
                        format!("{counter} += 1u"),
                        format!("{counter}++"),
                    ]
                    .contains(&step.to_owned()),
                    "{name}: {line} steps by one"
                );
                let constant = |bound: &str| bound.trim_end_matches('u').parse::<i64>().is_ok();
                assert!(
                    condition.starts_with(&format!("{counter} < "))
                        || condition.starts_with(&format!("{counter} - 1u < "))
                        || condition
                            .strip_prefix(&format!("{counter} <= "))
                            .is_some_and(constant),
                    "{name}: {line} is compared strictly, or against a constant"
                );
                counted += 1;
            }
        }
        assert!(counted >= 15, "{counted} counted loops");
        assert_eq!(argued, ARGUED.len());
    }

    /// The program text each family hands the device is the files joined.
    #[test]
    fn a_joined_program_is_its_files_in_a_row() {
        let program = Program::joined("group", &[COLOR, LAYER]);
        assert_eq!(program.source(), format!("{COLOR}\n{LAYER}"));
    }

    /// A stage keeps only the pipeline constants its module still declares;
    /// a program that cannot be cut keeps them all, for the device to judge.
    #[test]
    fn a_stage_keeps_only_the_constants_it_declares() {
        let (device, _queue) = crate::test_gpu::device();
        let mesh = Program::joined("mesh", &[TRIANGLE, TRIANGLE_SOLID, PATH, COLOR]);
        let samples = [("PATH_SAMPLES", 4.0)];
        assert_eq!(
            mesh.try_stage(&device, "path_fs_main")
                .expect("mesh vertex/fragment stage")
                .constants(&samples),
            samples
        );
        assert!(
            mesh.try_stage(&device, "path_keep_fs_main")
                .expect("mesh keep stage")
                .constants(&samples)
                .is_empty()
        );
        let broken = Program::new("broken", "fn nope( {");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        assert_eq!(
            broken
                .try_stage(&device, "fs_main")
                .expect("wgsl validation propagates to caller scope")
                .constants(&samples),
            samples
        );
        assert!(pollster::block_on(scope.pop()).is_some());
    }

    #[test]
    fn missing_precompiled_stage_falls_back_with_wgsl_or_errors_without_fallback() {
        let (device, _queue) = crate::test_gpu::device();
        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wgsl.fallback.test.layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let build = |vs: &wgpu::ShaderModule,
                     vs_ep: &str,
                     fs: &wgpu::ShaderModule,
                     fs_ep: &str,
                     _: &[(&str, f64)]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("wgsl.fallback.test"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: vs,
                    entry_point: Some(vs_ep),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: fs,
                    entry_point: Some(fs_ep),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let missing_hash_src = "@vertex fn vs_main() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }\n\
             @fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }\n// missing precompiled probe";
        let probe = Program::new("wgsl fallback probe", missing_hash_src);

        // Runtime WGSL fallback active: succeeds without panicking.
        let pipeline = probe
            .try_pipeline(&device, "vs_main", "fs_main", &[], build)
            .expect("missing precompiled stage falls back to runtime WGSL cleanly");
        drop(pipeline);

        // Corrupt/invalid WGSL through `try_pipeline`: error scopes absorb validation
        // errors and return explicit `ShaderUnavailable` rather than panicking.
        let broken = Program::new("broken wgsl probe", "fn nope( {");
        let broken_err = broken
            .try_pipeline(&device, "vs_main", "fs_main", &[], build)
            .unwrap_err();
        assert!(matches!(
            broken_err,
            super::super::validate::ScenePaintError::ShaderUnavailable {
                label: "broken wgsl probe",
                ..
            }
        ));

        // Pure-precompiled path (`allow_wgsl_fallback = false`): returns explicit error without panicking.
        let err = probe
            .try_pipeline_inner(false, &device, "vs_main", "fs_main", &[], &build)
            .unwrap_err();
        assert!(matches!(
            err,
            super::super::validate::ScenePaintError::ShaderUnavailable {
                label: "wgsl fallback probe",
                ..
            }
        ));

        {
            struct ForbidGuard;
            impl Drop for ForbidGuard {
                fn drop(&mut self) {
                    FORBID_WGSL_FALLBACK.store(false, std::sync::atomic::Ordering::SeqCst);
                }
            }
            assert!(wgsl_fallback_allowed());
            FORBID_WGSL_FALLBACK.store(true, std::sync::atomic::Ordering::SeqCst);
            let _forbid_guard = ForbidGuard;
            assert!(!wgsl_fallback_allowed());
            let err = probe
                .try_pipeline(&device, "vs_main", "fs_main", &[], build)
                .unwrap_err();
            assert!(matches!(
                err,
                super::super::validate::ScenePaintError::ShaderUnavailable {
                    label: "wgsl fallback probe",
                    ..
                }
            ));
        }

        #[cfg(target_vendor = "apple")]
        {
            struct MissingGuard;
            impl Drop for MissingGuard {
                fn drop(&mut self) {
                    super::super::msl_precompiled::set_force_missing_stage(false);
                }
            }
            let _gpu_guard = crate::test_gpu::lock();
            super::super::msl_precompiled::set_force_missing_stage(true);
            let _missing_guard = MissingGuard;

            let builtin = Program::joined("nana-ui.scene.group.shader", &[COLOR, LAYER]);
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("wgsl.fallback.group.layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
            let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("wgsl.fallback.group.pipeline"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let group_build = |vs: &wgpu::ShaderModule,
                               vs_ep: &str,
                               fs: &wgpu::ShaderModule,
                               fs_ep: &str,
                               _: &[(&str, f64)]| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("wgsl.fallback.group"),
                    layout: Some(&pipe_layout),
                    vertex: wgpu::VertexState {
                        module: vs,
                        entry_point: Some(vs_ep),
                        buffers: &[],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: fs,
                        entry_point: Some(fs_ep),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
            let _fallback_ok = builtin
                .try_pipeline(&device, "vs_main", "fs_main", &[], group_build)
                .expect("simulated missing MSL stage falls back to WGSL");
            let err = builtin
                .try_pipeline_inner(false, &device, "vs_main", "fs_main", &[], &group_build)
                .unwrap_err();
            assert!(matches!(
                err,
                super::super::validate::ScenePaintError::ShaderUnavailable {
                    label: "nana-ui.scene.group.shader",
                    ..
                }
            ));
        }
    }

    #[test]
    fn compile_time_hashes_match_joined_and_concat_sources() {
        let joined_src = joined(&[MOTION, COLOR, QUAD]);
        assert_eq!(
            super::super::precompiled::fnv1a_joined(&[MOTION, COLOR, QUAD]),
            super::super::precompiled::fnv1a(joined_src.as_bytes()),
        );
        let concat_src = [COLOR, LAYER, LAYER_BLEND].concat();
        assert_eq!(
            super::super::precompiled::fnv1a_concat(&[COLOR, LAYER, LAYER_BLEND]),
            super::super::precompiled::fnv1a(concat_src.as_bytes()),
        );
    }
}
