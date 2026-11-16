//! Precompiled, entry-point-cut, compacted `naga::Module` IR for built-in shaders.
//!
//! When built without `runtime-wgsl`, [`super::wgsl::Program::stage`] loads the
//! serialized `naga::Module` directly and hands `wgpu::ShaderSource::Naga` to the
//! device, keeping `naga::front::wgsl` and `naga::back::wgsl` out of production
//! binaries. See [`tests::regenerate_precompiled_naga_ir`] to refresh `.naga`
//! assets after editing any shader source.

mod generated;

fn decompress(bytes: &[u8]) -> Vec<u8> {
    let mut decoder = flate2::read::ZlibDecoder::new(bytes);
    let mut raw = Vec::new();
    std::io::Read::read_to_end(&mut decoder, &mut raw).expect("precompiled naga IR decompresses");
    raw
}

pub(super) fn stage(source_hash: u64, entry: &str) -> Option<wgpu::naga::Module> {
    let (_, _, bytes) = generated::STAGES
        .iter()
        .find(|&&(hash, name, _)| hash == source_hash && name == entry)?;
    let raw = decompress(bytes);
    Some(bincode::deserialize(&raw).expect("precompiled naga IR deserializes"))
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
        pub source: String,
        pub cfg_gate: Option<&'static str>,
    }

    pub(in crate::scene_paint) fn builtin_programs() -> Vec<BuiltinProgram> {
        #[cfg_attr(not(feature = "hosted"), allow(unused_mut))]
        let mut programs = vec![
            BuiltinProgram {
                slug: "quad",
                source: super::super::quad::quad_shader(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "mesh",
                source: super::super::wgsl::joined(&[
                    super::super::wgsl::TRIANGLE,
                    super::super::wgsl::TRIANGLE_SOLID,
                    super::super::wgsl::PATH,
                    super::super::wgsl::COLOR,
                ]),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "chart",
                source: super::super::wgsl::joined(&[
                    super::super::wgsl::CHART,
                    super::super::wgsl::COLOR,
                ]),
                cfg_gate: Some(r#"any(test, feature = "charts")"#),
            },
            BuiltinProgram {
                slug: "group_blend",
                source: super::super::wgsl::joined(&[
                    super::super::wgsl::COLOR,
                    super::super::wgsl::LAYER,
                    super::super::wgsl::LAYER_BLEND,
                ]),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "group",
                source: super::super::wgsl::joined(&[
                    super::super::wgsl::COLOR,
                    super::super::wgsl::LAYER,
                ]),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "blit",
                source: super::super::wgsl::BLIT.to_owned(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "motion",
                source: super::super::wgsl::MOTION.to_owned(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "backdrop_copy",
                source: super::super::wgsl::BACKDROP_COPY.to_owned(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "backdrop_blur",
                source: super::super::wgsl::BACKDROP_BLUR.to_owned(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "backdrop_composite",
                source: super::super::wgsl::joined(&[
                    super::super::wgsl::COLOR,
                    super::super::wgsl::QUAD_PAINT_DATA,
                    super::super::wgsl::BACKDROP_COMPOSITE,
                ]),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "icon",
                source: super::super::icon::icon_shader(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "text",
                source: super::super::text::text_shader(false),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "text_dual_source",
                source: super::super::text::text_shader(true),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "gpu_view",
                source: crate::gpu_view::GPU_VIEW_SHADER.to_owned(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "host_texture",
                source: crate::gpu_texture::SOURCE.to_owned(),
                cfg_gate: None,
            },
            BuiltinProgram {
                slug: "transfer",
                source: crate::output::transfer::TRANSFER_WGSL.to_owned(),
                cfg_gate: None,
            },
        ];
        #[cfg(feature = "hosted")]
        programs.push(BuiltinProgram {
            slug: "native_content",
            source: crate::native_content::NATIVE_CONTENT_WGSL.to_owned(),
            cfg_gate: Some(r#"feature = "hosted""#),
        });
        #[cfg(feature = "hosted")]
        programs.push(BuiltinProgram {
            slug: "window_output_straight",
            source: crate::output::window_output::STRAIGHT_ALPHA_WGSL.to_owned(),
            cfg_gate: Some(r#"feature = "hosted""#),
        });
        programs
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
                    bincode::serialize(&expected).expect("serialize expected naga IR");
                let loaded = stage(hash, &point.name).unwrap_or_else(|| {
                    panic!(
                        "missing precompiled naga IR for `{}` (hash {hash:#018x}) entry `{}`; \
                         run regenerate_precompiled_naga_ir",
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
                let loaded_bytes = bincode::serialize(&loaded).expect("serialize loaded naga IR");
                assert_eq!(
                    loaded_bytes, expected_bytes,
                    "stale precompiled naga IR for `{}`::{}; run regenerate_precompiled_naga_ir",
                    program.slug, point.name
                );
            }
        }
    }

    #[test]
    #[ignore = "regenerates checked-in .naga IR assets after WGSL shader edits"]
    fn regenerate_precompiled_naga_ir() {
        assert!(
            cfg!(feature = "hosted"),
            "run regenerate_precompiled_naga_ir with --features full so all feature-gated shaders are emitted"
        );
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
                let raw = bincode::serialize(&cut_module).expect("serialize naga IR");
                let bytes = compress(&raw);
                let file_name = format!("{}__{}.naga", program.slug, point.name);
                std::fs::write(out_dir.join(&file_name), &bytes).expect("write .naga file");
                expected_files.insert(file_name.clone());
                if let Some(gate) = program.cfg_gate {
                    generated.push_str(&format!("    #[cfg({gate})]\n"));
                }
                generated.push_str(&format!(
                    "    ({hash:#018x}, {:?}, include_bytes!({file_name:?})),\n",
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
