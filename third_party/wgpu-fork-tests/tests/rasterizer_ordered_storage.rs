//! Tests for `Features::TEXTURE_STORAGE_RASTERIZER_ORDER`,
//! `StorageTextureAccess::RasterizerOrdered`, and WGSL `rasterizer_ordered`
//! storage textures across naga (WGSL/HLSL/SPIR-V/MSL) and live wgpu adapters.

use nana_wgpu_fork_tests::{device, no_adapter};

const COMBINED_WGSL: &str = r#"
@group(0) @binding(0)
var dst: texture_storage_2d<rgba16float, rasterizer_ordered>;

fn blend_pixel(px: vec2<i32>, src: vec4<f32>) {
    let dims = textureDimensions(dst);
    if px.x >= i32(dims.x) || px.y >= i32(dims.y) {
        return;
    }
    let backdrop = textureLoad(dst, px);
    textureStore(dst, px, src + backdrop * (1.0 - src.a));
}

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi & 2u) * 2 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) {
    if pos.x < 0.0 {
        return;
    }
    blend_pixel(vec2<i32>(pos.xy), vec4<f32>(0.25, 0.1, 0.05, 0.5));
}
"#;

fn parse_and_validate(
    src: &str,
    caps: naga::valid::Capabilities,
) -> Result<(naga::Module, naga::valid::ModuleInfo), String> {
    let module = naga::front::wgsl::parse_str(src).map_err(|e| e.emit_to_string(src))?;
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), caps)
        .validate(&module)
        .map_err(|e| format!("{e:?}"))?;
    Ok((module, info))
}

#[test]
fn feature_name_parses_with_wgpu_prefix() {
    assert_eq!(
        "wgpu-texture-storage-rasterizer-order".parse::<wgpu::Features>(),
        Ok(wgpu::Features::TEXTURE_STORAGE_RASTERIZER_ORDER),
    );
    assert!(
        "texture-storage-rasterizer-order"
            .parse::<wgpu::Features>()
            .is_err()
    );
}

#[test]
fn naga_wgsl_roundtrip_and_validation() {
    // Missing capability is rejected.
    let err = parse_and_validate(COMBINED_WGSL, naga::valid::Capabilities::empty()).unwrap_err();
    assert!(
        err.contains("TEXTURE_STORAGE_RASTERIZER_ORDER"),
        "expected missing capability error, got: {err}"
    );

    // Valid combined VS+FS module succeeds and roundtrips through wgsl-out.
    let caps = naga::valid::Capabilities::TEXTURE_STORAGE_RASTERIZER_ORDER;
    let (module, info) = parse_and_validate(COMBINED_WGSL, caps).expect("valid module");
    let printed =
        naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
            .expect("wgsl-out");
    assert!(
        printed.contains("texture_storage_2d<rgba16float,rasterizer_ordered>"),
        "unexpected wgsl-out:\n{printed}"
    );
    parse_and_validate(&printed, caps).expect("roundtripped WGSL validates");

    // Storage buffers reject `rasterizer_ordered`.
    let storage_buf_src = r#"
        @group(0) @binding(0) var<storage, rasterizer_ordered> buf: array<u32>;
        @fragment fn fs() { let _x = buf[0]; }
    "#;
    assert!(naga::front::wgsl::parse_str(storage_buf_src).is_err());

    // Compute and vertex entry points referencing `rasterizer_ordered` (directly or
    // via a helper function) are rejected.
    let compute_direct = r#"
        @group(0) @binding(0) var dst: texture_storage_2d<rgba8unorm, rasterizer_ordered>;
        @compute @workgroup_size(1) fn cs() {
            textureStore(dst, vec2<i32>(0, 0), vec4<f32>(1.0));
        }
    "#;
    assert!(parse_and_validate(compute_direct, caps).is_err());

    let vertex_indirect = r#"
        @group(0) @binding(0) var dst: texture_storage_2d<rgba8unorm, rasterizer_ordered>;
        fn helper() -> vec4<f32> {
            return textureLoad(dst, vec2<i32>(0, 0));
        }
        @vertex fn vs() -> @builtin(position) vec4<f32> {
            return helper();
        }
    "#;
    assert!(parse_and_validate(vertex_indirect, caps).is_err());
}

#[test]
fn naga_hlsl_writer_emits_rov_for_fragment_and_omits_for_vertex() {
    let caps = naga::valid::Capabilities::TEXTURE_STORAGE_RASTERIZER_ORDER;
    let (module, info) = parse_and_validate(COMBINED_WGSL, caps).unwrap();

    let mut bind_map = naga::back::hlsl::BindTarget::default();
    bind_map.register = 0;
    bind_map.space = 0;
    let mut binding_map = naga::back::hlsl::BindingMap::default();
    binding_map.insert(
        naga::ResourceBinding {
            group: 0,
            binding: 0,
        },
        bind_map,
    );

    let options = naga::back::hlsl::Options {
        shader_model: naga::back::hlsl::ShaderModel::V5_1,
        binding_map: binding_map.clone(),
        ..Default::default()
    };

    // Fragment stage gets RasterizerOrderedTexture2D.
    let mut fs_hlsl = String::new();
    let fs_pipeline = naga::back::hlsl::PipelineOptions {
        entry_point: Some((naga::ShaderStage::Fragment, "fs".into())),
        ..Default::default()
    };
    naga::back::hlsl::Writer::new(&mut fs_hlsl, &options, &fs_pipeline)
        .write(&module, &info, None)
        .expect("fragment HLSL");
    assert!(
        fs_hlsl.contains("RasterizerOrderedTexture2D<float4> dst_ : register(u0);"),
        "missing ROV declaration in fragment HLSL:\n{fs_hlsl}"
    );
    assert!(
        fs_hlsl.contains("NagaRasterizerOrderedDimensions2D"),
        "missing ROV dimensions helper in fragment HLSL:\n{fs_hlsl}"
    );

    // Vertex stage from the same module omits ROV declarations and helpers so FXC/DXC vs_5_1 succeeds.
    let mut vs_hlsl = String::new();
    let vs_pipeline = naga::back::hlsl::PipelineOptions {
        entry_point: Some((naga::ShaderStage::Vertex, "vs".into())),
        ..Default::default()
    };
    naga::back::hlsl::Writer::new(&mut vs_hlsl, &options, &vs_pipeline)
        .write(&module, &info, None)
        .expect("vertex HLSL");
    assert!(
        !vs_hlsl.contains("RasterizerOrdered"),
        "vertex HLSL must omit ROV declarations/helpers:\n{vs_hlsl}"
    );

    // ShaderModel 5.0 is rejected.
    let sm50 = naga::back::hlsl::Options {
        shader_model: naga::back::hlsl::ShaderModel::V5_0,
        binding_map: binding_map.clone(),
        ..Default::default()
    };
    let mut dummy = String::new();
    assert!(matches!(
        naga::back::hlsl::Writer::new(&mut dummy, &sm50, &fs_pipeline).write(&module, &info, None),
        Err(naga::back::hlsl::Error::ShaderModelTooLow(_, _))
    ));

    // Ordinary read_write storage textures still emit RWTexture2D unchanged.
    let rw_src = r#"
        @group(0) @binding(0) var rw: texture_storage_2d<r32float, read_write>;
        @fragment fn fs() {
            let v = textureLoad(rw, vec2<i32>(0, 0));
            textureStore(rw, vec2<i32>(0, 0), v);
        }
    "#;
    let (rw_mod, rw_info) = parse_and_validate(rw_src, caps).unwrap();
    let mut rw_hlsl = String::new();
    naga::back::hlsl::Writer::new(&mut rw_hlsl, &options, &fs_pipeline)
        .write(&rw_mod, &rw_info, None)
        .expect("rw HLSL");
    assert!(
        rw_hlsl.contains("RWTexture2D<float> rw : register(u0);")
            && !rw_hlsl.contains("RasterizerOrdered"),
        "unexpected RW HLSL:\n{rw_hlsl}"
    );
}

#[test]
fn naga_spv_writer_emits_single_interlock_pair_and_coherent_decoration() {
    let caps = naga::valid::Capabilities::TEXTURE_STORAGE_RASTERIZER_ORDER;
    let (module, info) = parse_and_validate(COMBINED_WGSL, caps).unwrap();

    let mut spv_caps = naga::FastHashSet::default();
    spv_caps.insert(spirv::Capability::Shader);
    spv_caps.insert(spirv::Capability::ImageQuery);
    spv_caps.insert(spirv::Capability::StorageImageExtendedFormats);
    spv_caps.insert(spirv::Capability::FragmentShaderPixelInterlockEXT);

    let options = naga::back::spv::Options {
        capabilities: Some(spv_caps),
        ..Default::default()
    };

    let fs_pipeline = naga::back::spv::PipelineOptions {
        shader_stage: naga::ShaderStage::Fragment,
        entry_point: "fs".into(),
    };
    let fs_words =
        naga::back::spv::write_vec(&module, &info, &options, Some(&fs_pipeline)).expect("fs spv");

    let mut has_interlock_cap = false;
    let mut has_interlock_ext = false;
    let mut has_pixel_ordered_mode = false;
    let mut has_coherent = false;
    let mut begin_count = 0usize;
    let mut end_count = 0usize;

    // Skip 5-word SPIR-V header.
    let mut idx = 5;
    while idx < fs_words.len() {
        let word0 = fs_words[idx];
        let wc = (word0 >> 16) as usize;
        let op = word0 & 0xffff;
        let args = &fs_words[idx + 1..idx + wc];
        if op == spirv::Op::Capability as u32
            && args.first() == Some(&(spirv::Capability::FragmentShaderPixelInterlockEXT as u32))
        {
            has_interlock_cap = true;
        } else if op == spirv::Op::Extension as u32 {
            let bytes: Vec<u8> = args
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .take_while(|&b| b != 0)
                .collect();
            if bytes == b"SPV_EXT_fragment_shader_interlock" {
                has_interlock_ext = true;
            }
        } else if op == spirv::Op::ExecutionMode as u32
            && args.get(1) == Some(&(spirv::ExecutionMode::PixelInterlockOrderedEXT as u32))
        {
            has_pixel_ordered_mode = true;
        } else if op == spirv::Op::Decorate as u32
            && args.get(1) == Some(&(spirv::Decoration::Coherent as u32))
        {
            has_coherent = true;
        } else if op == spirv::Op::BeginInvocationInterlockEXT as u32 {
            begin_count += 1;
        } else if op == spirv::Op::EndInvocationInterlockEXT as u32 {
            end_count += 1;
        }
        idx += wc;
    }

    assert!(
        has_interlock_cap,
        "missing FragmentShaderPixelInterlockEXT capability"
    );
    assert!(
        has_interlock_ext,
        "missing SPV_EXT_fragment_shader_interlock extension"
    );
    assert!(
        has_pixel_ordered_mode,
        "missing PixelInterlockOrderedEXT execution mode"
    );
    assert!(
        has_coherent,
        "missing Coherent decoration on ordered storage image"
    );
    assert_eq!(
        begin_count, 1,
        "expected exactly 1 OpBeginInvocationInterlockEXT"
    );
    assert_eq!(
        end_count, 1,
        "expected exactly 1 OpEndInvocationInterlockEXT"
    );

    // Vertex stage from the same module must not emit interlock instructions or capabilities.
    let vs_pipeline = naga::back::spv::PipelineOptions {
        shader_stage: naga::ShaderStage::Vertex,
        entry_point: "vs".into(),
    };
    let vs_words =
        naga::back::spv::write_vec(&module, &info, &options, Some(&vs_pipeline)).expect("vs spv");
    let mut idx = 5;
    while idx < vs_words.len() {
        let word0 = vs_words[idx];
        let wc = (word0 >> 16) as usize;
        let op = word0 & 0xffff;
        let args = &vs_words[idx + 1..idx + wc];
        assert!(
            !(op == spirv::Op::Capability as u32
                && args.first()
                    == Some(&(spirv::Capability::FragmentShaderPixelInterlockEXT as u32))),
            "vertex SPIR-V must not declare FragmentShaderPixelInterlockEXT"
        );
        assert_ne!(op, spirv::Op::BeginInvocationInterlockEXT as u32);
        assert_ne!(op, spirv::Op::EndInvocationInterlockEXT as u32);
        idx += wc;
    }
}

#[test]
fn naga_msl_writer_emits_raster_order_group_and_checks_version() {
    let caps = naga::valid::Capabilities::TEXTURE_STORAGE_RASTERIZER_ORDER;
    let (module, info) = parse_and_validate(COMBINED_WGSL, caps).unwrap();

    let mut ep_resources = naga::back::msl::EntryPointResources::default();
    ep_resources.resources.insert(
        naga::ResourceBinding {
            group: 0,
            binding: 0,
        },
        naga::back::msl::BindTarget {
            texture: Some(0),
            mutable: true,
            ..Default::default()
        },
    );
    let mut per_entry_point_map = naga::back::msl::EntryPointResourceMap::default();
    per_entry_point_map.insert("fs".into(), ep_resources.clone());
    per_entry_point_map.insert("vs".into(), naga::back::msl::EntryPointResources::default());

    let options = naga::back::msl::Options {
        lang_version: (2, 0),
        per_entry_point_map,
        ..Default::default()
    };

    let (msl_fs, _) = naga::back::msl::write_string(
        &module,
        &info,
        &options,
        &naga::back::msl::PipelineOptions {
            entry_point: Some((naga::ShaderStage::Fragment, "fs".into())),
            ..Default::default()
        },
    )
    .expect("MSL 2.0 fragment");
    assert!(
        msl_fs.contains("metal::texture2d<float, metal::access::read_write>")
            && msl_fs.contains("[[texture(0), raster_order_group(0)]]"),
        "missing raster_order_group(0) in MSL output:\n{msl_fs}"
    );

    let (msl_vs, _) = naga::back::msl::write_string(
        &module,
        &info,
        &options,
        &naga::back::msl::PipelineOptions {
            entry_point: Some((naga::ShaderStage::Vertex, "vs".into())),
            ..Default::default()
        },
    )
    .expect("MSL 2.0 vertex");
    assert!(
        !msl_vs.contains("raster_order_group"),
        "vertex MSL must not contain raster_order_group:\n{msl_vs}"
    );

    // MSL 1.2 is rejected.
    let mut options_12 = options.clone();
    options_12.lang_version = (1, 2);
    let err = naga::back::msl::write_string(
        &module,
        &info,
        &options_12,
        &naga::back::msl::PipelineOptions {
            entry_point: Some((naga::ShaderStage::Fragment, "fs".into())),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        naga::back::msl::Error::UnsupportedRasterizerOrderedTexture
    ));
}

fn validation_checks_on_backend(test: &str, backend: wgpu::Backends) {
    // 1. Device WITHOUT feature enabled rejects BGL and shader module.
    let Some((adapter, plain_device, _)) = device(backend, wgpu::Features::empty()) else {
        no_adapter(test, backend);
        return;
    };

    let scope = plain_device.push_error_scope(wgpu::ErrorFilter::Validation);
    let _bgl = plain_device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("unfeatured ordered bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::RasterizerOrdered,
                format: wgpu::TextureFormat::Rgba8Unorm,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        }],
    });
    assert!(
        pollster::block_on(scope.pop()).is_some(),
        "{test} ({backend:?}): BGL must fail without TEXTURE_STORAGE_RASTERIZER_ORDER"
    );

    let scope = plain_device.push_error_scope(wgpu::ErrorFilter::Validation);
    let _sm = plain_device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("unfeatured ordered shader"),
        source: wgpu::ShaderSource::Wgsl(COMBINED_WGSL.into()),
    });
    assert!(
        pollster::block_on(scope.pop()).is_some(),
        "{test} ({backend:?}): shader module must fail without TEXTURE_STORAGE_RASTERIZER_ORDER"
    );
    drop(plain_device);

    // 2. Device WITH feature enabled validates visibility, format, and access matching.
    if !adapter
        .features()
        .contains(wgpu::Features::TEXTURE_STORAGE_RASTERIZER_ORDER)
    {
        println!(
            "{test}: {backend:?} adapter {:?} does not expose TEXTURE_STORAGE_RASTERIZER_ORDER; skipping feature-enabled checks",
            adapter.get_info().name
        );
        return;
    }

    let (_, dev, _) = device(backend, wgpu::Features::TEXTURE_STORAGE_RASTERIZER_ORDER)
        .expect("adapter already found");

    for bad_vis in [
        wgpu::ShaderStages::VERTEX,
        wgpu::ShaderStages::VERTEX_FRAGMENT,
        wgpu::ShaderStages::COMPUTE,
    ] {
        let scope = dev.push_error_scope(wgpu::ErrorFilter::Validation);
        let _bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bad visibility ordered bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: bad_vis,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::RasterizerOrdered,
                    format: wgpu::TextureFormat::Rgba16Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            }],
        });
        assert!(
            pollster::block_on(scope.pop()).is_some(),
            "{test} ({backend:?}): visibility {bad_vis:?} must be rejected"
        );
    }

    // sRGB format lacks STORAGE_READ_WRITE and must be rejected.
    let scope = dev.push_error_scope(wgpu::ErrorFilter::Validation);
    let _bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("srgb ordered bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::RasterizerOrdered,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        }],
    });
    assert!(
        pollster::block_on(scope.pop()).is_some(),
        "{test} ({backend:?}): Rgba8UnormSrgb must be rejected"
    );

    // Mismatch: RasterizerOrdered BGL vs write-only shader must fail pipeline creation.
    let ordered_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ordered bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::RasterizerOrdered,
                format: wgpu::TextureFormat::Rgba16Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        }],
    });
    let pl = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ordered pl"),
        bind_group_layouts: &[Some(&ordered_bgl)],
        immediate_size: 0,
    });
    let write_only_shader = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("write-only shader"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
            @group(0) @binding(0) var dst: texture_storage_2d<rgba16float, write>;
            @vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
            @fragment fn fs() { textureStore(dst, vec2<i32>(0, 0), vec4<f32>(1.0)); }
            "#
            .into(),
        ),
    });
    let scope = dev.push_error_scope(wgpu::ErrorFilter::Validation);
    let _rp = dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mismatched rp"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &write_only_shader,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &write_only_shader,
            entry_point: Some("fs"),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::R8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
            })],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    assert!(
        pollster::block_on(scope.pop()).is_some(),
        "{test} ({backend:?}): write-only shader must not match RasterizerOrdered BGL"
    );
}

#[derive(Clone, Copy)]
struct Quad {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    // Straight color + alpha, converted to premultiplied in both CPU & GPU.
    color: [f32; 4],
}

fn make_overlapping_quads(size: u32) -> Vec<Quad> {
    let mut quads = Vec::with_capacity(48);
    for i in 0..48u32 {
        // Every quad covers the central region [8..56, 8..56], plus staggered edges,
        // so central pixels experience 48 back-to-back overlapping fragment RMWs!
        let inset_l = (i * 5) % 16;
        let inset_t = (i * 7) % 16;
        let inset_r = size - ((i * 3) % 16);
        let inset_b = size - ((i * 11) % 16);
        let r = ((i * 37 + 13) % 97) as f32 / 97.0;
        let g = ((i * 53 + 29) % 89) as f32 / 89.0;
        let b = ((i * 71 + 41) % 83) as f32 / 83.0;
        let a = 0.18 + ((i * 19) % 35) as f32 / 100.0;
        quads.push(Quad {
            x0: inset_l,
            y0: inset_t,
            x1: inset_r,
            y1: inset_b,
            color: [r * a, g * a, b * a, a],
        });
    }
    quads
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1f) as i32;
    let mant = (bits & 0x03ff) as u32;
    if exp == 0 {
        if mant == 0 {
            f32::from_bits(sign << 31)
        } else {
            let v = (mant as f32) * (1.0 / (1u32 << 24) as f32);
            if sign != 0 { -v } else { v }
        }
    } else if exp == 31 {
        f32::from_bits((sign << 31) | 0x7f80_0000 | (mant << 13))
    } else {
        let f32_exp = (exp - 15 + 127) as u32;
        f32::from_bits((sign << 31) | (f32_exp << 23) | (mant << 13))
    }
}

fn run_pixel_ordering_case(
    dev: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    wgsl_format: &str,
) {
    const SIZE: u32 = 64;
    let quads = make_overlapping_quads(SIZE);

    // Pack 6 vertices per quad: (ndc_x, ndc_y, r, g, b, a, skip_flag, pad) = 8 f32s = 32 bytes/vertex.
    // Also insert an interspersed skipped quad every 8 quads to exercise the early `return` path!
    let mut verts: Vec<[f32; 8]> = Vec::new();
    let mut push_quad = |q: Quad, skip: f32| {
        let lx = (q.x0 as f32 / SIZE as f32) * 2.0 - 1.0;
        let rx = (q.x1 as f32 / SIZE as f32) * 2.0 - 1.0;
        // NDC +Y is top in wgpu, texture row 0 is top -> y_ndc = 1.0 - 2.0 * (y / SIZE).
        let ty = 1.0 - (q.y0 as f32 / SIZE as f32) * 2.0;
        let by = 1.0 - (q.y1 as f32 / SIZE as f32) * 2.0;
        let [r, g, b, a] = q.color;
        let corners = [[lx, ty], [rx, ty], [lx, by], [lx, by], [rx, ty], [rx, by]];
        for [x, y] in corners {
            verts.push([x, y, r, g, b, a, skip, 0.0]);
        }
    };

    for (idx, &q) in quads.iter().enumerate() {
        if idx % 8 == 4 {
            push_quad(
                Quad {
                    x0: 0,
                    y0: 0,
                    x1: SIZE,
                    y1: SIZE,
                    color: [1.0, 0.0, 1.0, 1.0],
                },
                1.0, // triggers early return in fragment shader!
            );
        }
        push_quad(q, 0.0);
    }
    let total_quads_in_vb = (verts.len() / 6) as u32;

    let vb_bytes: Vec<u8> = verts
        .iter()
        .flat_map(|v| v.iter().flat_map(|f| f.to_le_bytes()))
        .collect();
    let vb = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ordered quads vb"),
        size: vb_bytes.len() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&vb, 0, &vb_bytes);

    let wgsl = format!(
        r#"
        @group(0) @binding(0)
        var dst: texture_storage_2d<{wgsl_format}, rasterizer_ordered>;

        struct VsOut {{
            @builtin(position) pos: vec4<f32>,
            @location(0) color: vec4<f32>,
            @location(1) skip: f32,
        }};

        @vertex
        fn vs_clear(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {{
            let x = f32(i32(vi & 1u) * 4 - 1);
            let y = f32(i32(vi & 2u) * 2 - 1);
            return vec4<f32>(x, y, 0.0, 1.0);
        }}

        @fragment
        fn fs_clear(@builtin(position) pos: vec4<f32>) {{
            textureStore(dst, vec2<i32>(pos.xy), vec4<f32>(0.0));
        }}

        @vertex
        fn vs_blend(
            @location(0) xy: vec2<f32>,
            @location(1) color: vec4<f32>,
            @location(2) skip_pad: vec2<f32>,
        ) -> VsOut {{
            var out: VsOut;
            out.pos = vec4<f32>(xy, 0.0, 1.0);
            out.color = color;
            out.skip = skip_pad.x;
            return out;
        }}

        @fragment
        fn fs_blend(in: VsOut) {{
            if in.skip > 0.5 {{
                return;
            }}
            let px = vec2<i32>(in.pos.xy);
            let d = textureLoad(dst, px);
            textureStore(dst, px, in.color + d * (1.0 - in.color.a));
        }}
        "#
    );

    let scope = dev.push_error_scope(wgpu::ErrorFilter::Validation);
    let shader = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ordered blend shader"),
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    });

    let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ordered target bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::RasterizerOrdered,
                format,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        }],
    });
    let pl = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ordered target pl"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let dummy_target = [Some(wgpu::ColorTargetState {
        format: wgpu::TextureFormat::R8Unorm,
        blend: None,
        write_mask: wgpu::ColorWrites::empty(),
    })];

    let clear_pipe = dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ordered clear pipe"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_clear"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_clear"),
            targets: &dummy_target,
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });

    let blend_pipe = dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ordered blend pipe"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_blend"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 32,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 8,
                        shader_location: 1,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 24,
                        shader_location: 2,
                    },
                ],
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_blend"),
            targets: &dummy_target,
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });

    let storage_tex = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("ordered storage tex"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let storage_view = storage_tex.create_view(&Default::default());

    let dummy_tex = dev.create_texture(&wgpu::TextureDescriptor {
        label: Some("dummy r8 attachment"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let dummy_view = dummy_tex.create_view(&Default::default());

    let bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ordered target bg"),
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&storage_view),
        }],
    });

    let bpp = match format {
        wgpu::TextureFormat::Rgba8Unorm => 4u32,
        wgpu::TextureFormat::Rgba16Float => 8u32,
        _ => unreachable!(),
    };
    let bytes_per_row = (SIZE * bpp).div_ceil(256) * 256;
    let readback = dev.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ordered readback"),
        size: (bytes_per_row * SIZE) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut enc = dev.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ordered encoder"),
    });
    {
        let mut rpass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("single ordered pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &dummy_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Discard,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rpass.set_bind_group(0, &bg, &[]);
        rpass.set_pipeline(&clear_pipe);
        rpass.draw(0..3, 0..1);

        rpass.set_pipeline(&blend_pipe);
        rpass.set_vertex_buffer(0, vb.slice(..));
        // Split across BOTH multi-quad draws (24 quads in 1 draw) AND individual 1-quad draws
        // in the same render pass — testing both intra-draw and inter-draw primitive ordering!
        let batch_quads = 24u32;
        rpass.draw(0..batch_quads * 6, 0..1);
        for q_idx in batch_quads..total_quads_in_vb {
            rpass.draw(q_idx * 6..(q_idx + 1) * 6, 0..1);
        }
    }
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &storage_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([enc.finish()]);

    if let Some(err) = pollster::block_on(scope.pop()) {
        panic!("unexpected validation error for {format:?}: {err}");
    }

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        let _ = tx.send(res);
    });
    dev.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range().unwrap();

    // Compute exact CPU reference (forward order AND reversed order to prove order sensitivity).
    let simulate = |iter_quads: &[Quad]| -> Vec<[f32; 4]> {
        let mut grid = vec![[0.0f32; 4]; (SIZE * SIZE) as usize];
        for q in iter_quads {
            for y in q.y0..q.y1 {
                for x in q.x0..q.x1 {
                    let dst = &mut grid[(y * SIZE + x) as usize];
                    for c in 0..4 {
                        let v = q.color[c] + dst[c] * (1.0 - q.color[3]);
                        dst[c] = if format == wgpu::TextureFormat::Rgba8Unorm {
                            // Quantize each store to unorm8 just like Rgba8Unorm UAV store/load.
                            ((v.clamp(0.0, 1.0) * 255.0).round()) / 255.0
                        } else {
                            v
                        };
                    }
                }
            }
        }
        grid
    };

    let expected = simulate(&quads);
    let mut rev_quads = quads.clone();
    rev_quads.reverse();
    let reversed = simulate(&rev_quads);

    let mut max_err = 0.0f32;
    let mut diff_from_reversed = 0.0f32;
    for y in 0..SIZE {
        let row = &data[(y * bytes_per_row) as usize..((y * bytes_per_row) + SIZE * bpp) as usize];
        for x in 0..SIZE {
            let idx = (y * SIZE + x) as usize;
            let got = match format {
                wgpu::TextureFormat::Rgba8Unorm => {
                    let o = (x * 4) as usize;
                    [
                        row[o] as f32 / 255.0,
                        row[o + 1] as f32 / 255.0,
                        row[o + 2] as f32 / 255.0,
                        row[o + 3] as f32 / 255.0,
                    ]
                }
                wgpu::TextureFormat::Rgba16Float => {
                    let o = (x * 8) as usize;
                    [
                        f16_to_f32(u16::from_le_bytes([row[o], row[o + 1]])),
                        f16_to_f32(u16::from_le_bytes([row[o + 2], row[o + 3]])),
                        f16_to_f32(u16::from_le_bytes([row[o + 4], row[o + 5]])),
                        f16_to_f32(u16::from_le_bytes([row[o + 6], row[o + 7]])),
                    ]
                }
                _ => unreachable!(),
            };
            for c in 0..4 {
                max_err = max_err.max((got[c] - expected[idx][c]).abs());
                diff_from_reversed = diff_from_reversed.max((got[c] - reversed[idx][c]).abs());
            }
        }
    }
    drop(data);
    readback.unmap();

    let tol = match format {
        // Allow up to 2/255 across 48 chained unorm8 RMW roundings (ties-to-even vs round).
        wgpu::TextureFormat::Rgba8Unorm => 2.5 / 255.0,
        wgpu::TextureFormat::Rgba16Float => 2e-3,
        _ => unreachable!(),
    };
    assert!(
        max_err <= tol,
        "{format:?}: pixel RMW order mismatch: max_err={max_err} > tol={tol}"
    );
    assert!(
        diff_from_reversed > 0.15,
        "{format:?}: test scene was not order-sensitive enough (diff_from_reversed={diff_from_reversed})"
    );
}

fn pixel_ordering_on_backend(test: &str, backend: wgpu::Backends) {
    let Some((adapter, dev, queue)) =
        device(backend, wgpu::Features::TEXTURE_STORAGE_RASTERIZER_ORDER)
    else {
        no_adapter(test, backend);
        return;
    };
    if !dev
        .features()
        .contains(wgpu::Features::TEXTURE_STORAGE_RASTERIZER_ORDER)
    {
        // Note: software adapters (such as older WARP or llvmpipe without interlock) may not
        // advertise ROV / fragment interlock; log clearly and skip pixel execution.
        println!(
            "{test}: {backend:?} adapter {:?} does not support TEXTURE_STORAGE_RASTERIZER_ORDER; skipping pixel test",
            adapter.get_info().name
        );
        return;
    }

    run_pixel_ordering_case(&dev, &queue, wgpu::TextureFormat::Rgba8Unorm, "rgba8unorm");
    run_pixel_ordering_case(
        &dev,
        &queue,
        wgpu::TextureFormat::Rgba16Float,
        "rgba16float",
    );
}

#[test]
fn dx12_rasterizer_ordered_storage() {
    validation_checks_on_backend("dx12_rasterizer_ordered_storage", wgpu::Backends::DX12);
    pixel_ordering_on_backend("dx12_rasterizer_ordered_storage", wgpu::Backends::DX12);
}

#[test]
fn vulkan_rasterizer_ordered_storage() {
    validation_checks_on_backend("vulkan_rasterizer_ordered_storage", wgpu::Backends::VULKAN);
    pixel_ordering_on_backend("vulkan_rasterizer_ordered_storage", wgpu::Backends::VULKAN);
}

#[cfg(target_vendor = "apple")]
#[test]
fn metal_rasterizer_ordered_storage() {
    validation_checks_on_backend("metal_rasterizer_ordered_storage", wgpu::Backends::METAL);
    pixel_ordering_on_backend("metal_rasterizer_ordered_storage", wgpu::Backends::METAL);
}
