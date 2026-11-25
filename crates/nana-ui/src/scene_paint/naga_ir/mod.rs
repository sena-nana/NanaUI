//! Precompiled, entry-point-cut, compacted `naga::Module` IR for built-in shaders.
//!
//! When built without `runtime-wgsl`, [`super::wgsl::Program::try_pipeline`] loads the
//! serialized `naga::Module` directly and hands `wgpu::ShaderSource::Naga` to the
//! device, keeping `naga::front::wgsl` and `naga::back::wgsl` out of production
//! binaries. See [`tests::regenerate_precompiled_naga_ir`] to refresh `.naga`
//! assets after editing any shader source.

mod codec;
mod generated;

fn decompress(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = flate2::read::ZlibDecoder::new(bytes);
    let mut raw = Vec::new();
    std::io::Read::read_to_end(&mut decoder, &mut raw).ok()?;
    Some(raw)
}

#[cfg_attr(target_vendor = "apple", allow(dead_code))]
pub(super) fn has_stage(source_hash: u64, entry: &str) -> bool {
    generated::STAGES
        .iter()
        .any(|&(hash, name, _)| hash == source_hash && name == entry)
}

#[cfg_attr(target_vendor = "apple", allow(dead_code))]
pub(super) fn stage(source_hash: u64, entry: &str) -> Option<wgpu::naga::Module> {
    let (_, _, bytes) = generated::STAGES
        .iter()
        .find(|&&(hash, name, _)| hash == source_hash && name == entry)?;
    let raw = decompress(bytes)?;
    codec::deserialize(&raw).ok()
}

#[cfg(test)]
pub(in crate::scene_paint) mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    fn compress(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut encoder, bytes).expect("compress precompiled naga IR");
        encoder.finish().expect("finish precompiled naga IR")
    }

    pub(in crate::scene_paint) struct BuiltinProgram {
        pub slug: &'static str,
        pub label: &'static str,
        pub source: String,
        pub cfg_gate: Option<&'static str>,
    }

    pub(in crate::scene_paint) fn builtin_programs() -> Vec<BuiltinProgram> {
        #[cfg_attr(not(feature = "hosted"), allow(unused_mut))]
        let mut programs = vec![
            BuiltinProgram {
                slug: "quad",
                label: "nana-ui.scene.quad.solid.shader",
                source: super::super::quad::quad_shader(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "mesh",
                label: "nana-ui.scene.triangle.solid.shader",
                source: super::super::wgsl::joined(&[
                    super::super::wgsl::TRIANGLE,
                    super::super::wgsl::MOTION,
                    super::super::wgsl::TRIANGLE_SOLID,
                    super::super::wgsl::PATH,
                    super::super::wgsl::COLOR,
                ]),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "chart",
                label: "nana-ui.scene.chart.shader",
                source: super::super::wgsl::joined(&[
                    super::super::wgsl::CHART,
                    super::super::wgsl::COLOR,
                ]),
                cfg_gate: Some(r#"any(test, feature = "charts")"#),
            },
            BuiltinProgram {
                slug: "group_blend",
                label: "nana-ui.scene.group.blend.shader",
                source: super::super::wgsl::joined(&[
                    super::super::wgsl::COLOR,
                    super::super::wgsl::MOTION,
                    super::super::wgsl::LAYER,
                    super::super::wgsl::LAYER_BLEND,
                ]),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "group",
                label: "nana-ui.scene.group.shader",
                source: super::super::wgsl::joined(&[
                    super::super::wgsl::COLOR,
                    super::super::wgsl::MOTION,
                    super::super::wgsl::LAYER,
                ]),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "blit",
                label: "nana-ui.scene.dest.blit.shader",
                source: super::super::wgsl::BLIT.to_owned(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "motion",
                label: "nana-ui.scene.motion.eval.shader",
                source: super::super::wgsl::MOTION.to_owned(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "backdrop_copy",
                label: "nana-ui.scene.backdrop.copy.shader",
                source: super::super::wgsl::BACKDROP_COPY.to_owned(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "backdrop_blur",
                label: "nana-ui.scene.backdrop.blur.shader",
                source: super::super::wgsl::BACKDROP_BLUR.to_owned(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "backdrop_composite",
                label: "nana-ui.scene.backdrop.composite.shader",
                source: super::super::wgsl::joined(&[
                    super::super::wgsl::COLOR,
                    super::super::wgsl::QUAD_PAINT_DATA,
                    super::super::wgsl::BACKDROP_COMPOSITE,
                ]),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "icon",
                label: "nana-ui.scene.icon.shader",
                source: super::super::icon::icon_shader(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "text",
                label: "nana-ui.scene.text.shader",
                source: super::super::text::text_shader(false),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "text_dual_source",
                label: "nana-ui.scene.text.shader",
                source: super::super::text::text_shader(true),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "gpu_view",
                label: "nana-ui default gpu-view shader",
                source: crate::gpu_view::GPU_VIEW_SHADER.to_owned(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "host_texture",
                label: "nana-ui host texture shader",
                source: super::super::wgsl::joined(&[
                    super::super::wgsl::MOTION,
                    crate::gpu_texture::SOURCE,
                ]),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "transfer",
                label: "nana.output.transfer",
                source: crate::output::transfer::TRANSFER_WGSL.to_owned(),
                cfg_gate: None,
            },
        ];
        #[cfg(feature = "hosted")]
        programs.push(BuiltinProgram {
            slug: "native_content",
            label: "native content opening",
            source: crate::native_content::NATIVE_CONTENT_WGSL.to_owned(),
            cfg_gate: Some(r#"feature = "hosted""#),
        });
        #[cfg(feature = "hosted")]
        programs.push(BuiltinProgram {
            slug: "window_output_straight",
            label: "nana-ui.window-output.straight",
            source: crate::output::transfer::STRAIGHT_ALPHA_WGSL.to_owned(),
            cfg_gate: Some(r#"feature = "hosted""#),
        });
        programs
    }

    fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries = std::fs::read_dir(dir)
            .expect("read source directory")
            .map(|entry| entry.expect("dir entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches!(name, "bin" | "examples") {
                    continue;
                }
                collect_rs_files(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }

    fn scan_macro_labels(src: &str) -> Vec<(String, String)> {
        let bytes = src.as_bytes();
        let mut results = Vec::new();
        let mut brace_depth = 0usize;
        let mut pending_cfg_test = false;
        let mut cfg_test_depth: Option<usize> = None;
        let mut i = 0usize;

        while i < bytes.len() {
            if bytes[i..].starts_with(b"//") {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if bytes[i..].starts_with(b"/*") {
                i += 2;
                let mut depth = 1usize;
                while i + 1 < bytes.len() && depth > 0 {
                    if bytes[i..].starts_with(b"/*") {
                        depth += 1;
                        i += 2;
                    } else if bytes[i..].starts_with(b"*/") {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            // Skip char/byte literals (e.g. `'"'`, `b'"'`, `'{'`, `'}'`, `'\''`) without touching lifetimes.
            if bytes[i] == b'\'' {
                if bytes.get(i + 1) == Some(&b'\\') {
                    let mut j = i + 2;
                    if j < bytes.len() {
                        j += 1;
                    }
                    while j < bytes.len() && bytes[j] != b'\'' && j - i < 16 {
                        j += 1;
                    }
                    if bytes.get(j) == Some(&b'\'') {
                        i = j + 1;
                        continue;
                    }
                } else if let Some(ch) = src[i + 1..].chars().next() {
                    let next = i + 1 + ch.len_utf8();
                    if ch != '\'' && bytes.get(next) == Some(&b'\'') {
                        i = next + 1;
                        continue;
                    }
                }
            }
            if matches!(bytes[i], b'r' | b'b')
                && (i == 0 || !is_ident_byte(bytes[i - 1]))
                && (bytes.get(i + 1) == Some(&b'#')
                    || bytes.get(i + 1) == Some(&b'"')
                    || (bytes[i] == b'b' && bytes.get(i + 1) == Some(&b'r')))
            {
                let mut j = i + 1;
                if bytes[i] == b'b' && bytes.get(j) == Some(&b'r') {
                    j += 1;
                }
                let mut hashes = 0usize;
                while j < bytes.len() && bytes[j] == b'#' {
                    hashes += 1;
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'"' {
                    j += 1;
                    while j < bytes.len() {
                        if hashes == 0 && bytes[j] == b'\\' {
                            j += 2;
                            continue;
                        }
                        if bytes[j] == b'"'
                            && bytes[j + 1..].len() >= hashes
                            && bytes[j + 1..j + 1 + hashes].iter().all(|&b| b == b'#')
                        {
                            j += 1 + hashes;
                            break;
                        }
                        j += 1;
                    }
                    i = j;
                    continue;
                }
            }
            if bytes[i] == b'"' {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            if bytes[i] == b'#'
                && bytes[i..].starts_with(b"#[")
                && let Some(end) = bytes[i..].iter().position(|&b| b == b']')
            {
                let attr = &src[i..i + end + 1];
                if attr.contains("cfg(test)")
                    || (attr.contains("cfg(all(")
                        && attr.contains("test")
                        && !attr.contains("not(test)"))
                {
                    pending_cfg_test = true;
                }
                i += end + 1;
                continue;
            }
            match bytes[i] {
                b'{' => {
                    brace_depth += 1;
                    if pending_cfg_test && cfg_test_depth.is_none() {
                        cfg_test_depth = Some(brace_depth);
                    }
                    pending_cfg_test = false;
                    i += 1;
                    continue;
                }
                b'}' => {
                    if cfg_test_depth == Some(brace_depth) {
                        cfg_test_depth = None;
                    }
                    brace_depth = brace_depth.saturating_sub(1);
                    i += 1;
                    continue;
                }
                b';' => {
                    if cfg_test_depth.is_none() {
                        pending_cfg_test = false;
                    }
                    i += 1;
                    continue;
                }
                _ => {}
            }

            if cfg_test_depth.is_none() && (i == 0 || !is_ident_byte(bytes[i - 1])) {
                for mac in ["program_joined!", "program_concat!", "program!"] {
                    if bytes[i..].starts_with(mac.as_bytes()) {
                        let mut k = i + mac.len();
                        while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                            k += 1;
                        }
                        if bytes.get(k) == Some(&b'(') {
                            k += 1;
                            while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                                k += 1;
                            }
                            assert_eq!(
                                bytes.get(k),
                                Some(&b'"'),
                                "{mac} invocation must begin with a string literal label"
                            );
                            k += 1;
                            let start = k;
                            while k < bytes.len() && bytes[k] != b'"' {
                                k += 1;
                            }
                            let label = src[start..k].to_owned();
                            results.push((mac.to_owned(), label));
                            i = k;
                            break;
                        }
                    }
                }
            }
            i += 1;
        }
        results
    }

    #[test]
    fn builtin_programs_cover_every_production_shader() {
        let programs = builtin_programs();
        let mut slugs = BTreeSet::new();
        let mut hashes = BTreeSet::new();
        let mut active_labels = BTreeSet::new();
        for program in &programs {
            assert!(
                slugs.insert(program.slug),
                "duplicate built-in shader slug `{}`",
                program.slug
            );
            let hash = super::super::precompiled::fnv1a(program.source.as_bytes());
            assert!(
                hashes.insert(hash),
                "duplicate built-in shader hash `{hash:#018x}` for `{}`",
                program.slug
            );
            active_labels.insert(program.label.to_owned());
        }

        // Feature-gated shaders live inside `#[cfg(feature = "hosted")]` modules and
        // are pushed into `builtin_programs()` under that same feature gate.
        let inactive_hosted_labels = if cfg!(feature = "hosted") {
            BTreeSet::new()
        } else {
            BTreeSet::from([
                "native content opening".to_owned(),
                "nana-ui.window-output.straight".to_owned(),
            ])
        };

        let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        collect_rs_files(&src_root, &mut files);
        let wgsl_file = src_root.join("scene_paint/wgsl.rs");
        let mut seen_labels = BTreeSet::new();
        let mut text_shader_sites = 0usize;
        for file in files {
            if file == wgsl_file {
                continue;
            }
            let raw = std::fs::read_to_string(&file).expect("read crate source");
            for (mac, label) in scan_macro_labels(&raw) {
                if label == "nana-ui.scene.text.shader" {
                    text_shader_sites += 1;
                }
                seen_labels.insert(label.clone());
                assert!(
                    active_labels.contains(&label) || inactive_hosted_labels.contains(&label),
                    "production shader {mac}({label:?}) in {} is missing from naga_ir::builtin_programs()",
                    file.display()
                );
            }
        }
        assert!(
            text_shader_sites >= 2,
            "expected both non-dual-source and dual-source text shader invocations"
        );
        for program in &programs {
            assert!(
                seen_labels.contains(program.label),
                "builtin_programs entry `{}` (label {:?}) no longer has a production call site",
                program.slug,
                program.label
            );
        }
    }

    fn compile_and_validate(label: &str, source: &str) -> naga::Module {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|error| panic!("parse built-in shader `{label}`: {error:?}"));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("validate built-in shader `{label}`: {error:?}"));
        module
    }

    fn dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/scene_paint/naga_ir")
    }

    #[test]
    fn naga_ir_stages_are_current_with_their_programs() {
        for program in builtin_programs() {
            let hash = super::super::precompiled::fnv1a(program.source.as_bytes());
            let module = compile_and_validate(program.slug, &program.source);
            assert!(
                !module.entry_points.is_empty(),
                "built-in shader `{}` must declare entry points",
                program.slug
            );
            for point in &module.entry_points {
                let expected = super::super::wgsl::cut(&module, &point.name)
                    .unwrap_or_else(|| panic!("cut `{}` entry `{}`", program.slug, point.name));
                let expected_bytes =
                    codec::serialize(&expected).expect("serialize expected naga IR");
                let (_, _, bytes) = generated::STAGES
                    .iter()
                    .find(|&&(entry_hash, name, _)| entry_hash == hash && name == point.name)
                    .unwrap_or_else(|| {
                        panic!(
                            "missing precompiled naga IR for `{}` (hash {hash:#018x}) entry `{}`; \
                             run regenerate_precompiled_naga_ir",
                            program.slug, point.name
                        )
                    });
                let raw = decompress(bytes).unwrap_or_else(|| {
                    panic!(
                        "failed to decompress naga IR for `{}`::{}",
                        program.slug, point.name
                    )
                });
                assert_eq!(
                    raw, expected_bytes,
                    "stale checked-in .naga IR bytes for `{}`::{}; run regenerate_precompiled_naga_ir",
                    program.slug, point.name
                );
                let loaded: wgpu::naga::Module = codec::deserialize(&raw).unwrap_or_else(|err| {
                    panic!(
                        "failed to deserialize naga IR for `{}`::{}: {err}",
                        program.slug, point.name
                    )
                });
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&loaded)
                .unwrap_or_else(|error| {
                    panic!(
                        "validate loaded `{}`::{}: {error:?}",
                        program.slug, point.name
                    )
                });
                let loaded_bytes = codec::serialize(&loaded).expect("serialize loaded naga IR");
                assert_eq!(
                    loaded_bytes, expected_bytes,
                    "stale precompiled naga IR for `{}`::{}; run regenerate_precompiled_naga_ir",
                    program.slug, point.name
                );
            }
        }
    }

    #[test]
    fn corrupt_or_missing_precompiled_naga_stage_returns_none_without_panicking() {
        assert!(!has_stage(0xdead_beef_cafe_babe, "missing_entry"));
        assert!(stage(0xdead_beef_cafe_babe, "missing_entry").is_none());
        assert!(decompress(&[]).is_none());
        assert!(decompress(b"not-zlib").is_none());
        assert!(codec::deserialize::<wgpu::naga::Module>(b"").is_err());
        assert!(codec::deserialize::<wgpu::naga::Module>(b"BAD1\x00").is_err());
        assert!(codec::deserialize::<wgpu::naga::Module>(b"NAGA1").is_err());
    }

    #[test]
    #[ignore = "regenerates checked-in .naga IR assets after WGSL shader edits"]
    fn regenerate_precompiled_naga_ir() {
        // A runtime check, not `const { assert!(..) }`: the ignored test
        // still compiles under `--features gpu`, which CI tests.
        if !cfg!(feature = "hosted") {
            panic!(
                "run regenerate_precompiled_naga_ir with --features full so all feature-gated shaders are emitted"
            );
        }
        let out_dir = dir();
        let mut expected_files = BTreeSet::new();
        let mut generated = String::from(
            "// Generated by `cargo test -j 8 -p nana-ui --lib --features full -- --ignored regenerate_precompiled_naga_ir --test-threads=1`.\n\n\
             pub(super) static STAGES: &[(u64, &str, &[u8])] = &[\n",
        );

        for program in builtin_programs() {
            let hash = super::super::precompiled::fnv1a(program.source.as_bytes());
            let module = compile_and_validate(program.slug, &program.source);
            for point in &module.entry_points {
                let cut_module =
                    super::super::wgsl::cut(&module, &point.name).expect("entry point exists");
                let raw = codec::serialize(&cut_module).expect("serialize naga IR");
                let bytes = compress(&raw);
                let file_name = format!("{}__{}.naga", program.slug, point.name);
                std::fs::write(out_dir.join(&file_name), &bytes).expect("write .naga file");
                expected_files.insert(file_name.clone());
                if let Some(gate) = program.cfg_gate {
                    generated.push_str(&format!("    #[cfg({gate})]\n"));
                }
                // rustfmt's multi-line tuple layout: every row is wider than
                // its tuple width, so `cargo fmt --check` keeps this output.
                generated.push_str(&format!(
                    "    (\n        \
                         {hash:#018x},\n        \
                         {:?},\n        \
                         include_bytes!({file_name:?}),\n    \
                     ),\n",
                    point.name
                ));
            }
        }
        generated.push_str("];\n");
        std::fs::write(out_dir.join("generated.rs"), generated).expect("write generated.rs");

        for entry in std::fs::read_dir(&out_dir).expect("read naga_ir dir") {
            let entry = entry.expect("dir entry");
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".naga") && !expected_files.contains(name.as_ref()) {
                std::fs::remove_file(entry.path()).expect("remove stale .naga file");
            }
        }
    }
}
