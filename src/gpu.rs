//! The GPU side: device, pipelines, textures, and executing a frame's draw list.

use crate::spec::Blend;
use anyhow::{Result, anyhow, bail};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub const TARGET: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug)]
pub struct Inst {
    pub m: [f32; 4],
    pub t_size: [f32; 4],
    pub uv: [f32; 4],
    pub color: [f32; 4],
    pub clip_m: [f32; 4],
    pub clip_t: [f32; 4],
    pub lbox: [f32; 4],
    pub reveal: [f32; 4],
    pub mask_r: [f32; 4],
    pub params: [f32; 4],
}

/// One step of a frame.
pub enum Op {
    Draw { blend: Blend, tex: usize, mask: usize, first: u32, count: u32 },
    Shader { prog: usize, blend: Blend, uoff: u32 },
    Isolate { target: usize, ops: Vec<Op> },
}

pub struct Tex {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub bind: wgpu::BindGroup,
    pub w: u32,
    pub h: u32,
}

/// A render target: the composition, or an isolated group's own layer.
pub struct Target {
    pub tex: usize,
    pub globals: wgpu::BindGroup,
    pub backdrop: Option<usize>,
}

pub struct ShaderProg {
    pipelines: HashMap<Blend, Vec<wgpu::RenderPipeline>>,
    bind: wgpu::BindGroup,
}

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub max_texture: u32,
    sampler: wgpu::Sampler,
    globals_layout: wgpu::BindGroupLayout,
    tex_layout: wgpu::BindGroupLayout,
    shader_layout: wgpu::BindGroupLayout,
    convert_layout: wgpu::BindGroupLayout,
    composite: HashMap<Blend, Vec<wgpu::RenderPipeline>>,
    yuv: wgpu::ComputePipeline,
    rgba: wgpu::ComputePipeline,
    pub textures: Vec<Tex>,
    pub targets: Vec<Target>,
    pub progs: Vec<ShaderProg>,
    inst_buf: wgpu::Buffer,
    uni_buf: wgpu::Buffer,
    pub uni_align: u32,
    shader_module_layout: wgpu::PipelineLayout,
}

pub const WHITE: usize = 0;

/// The fixed-function blend passes for a mode, on premultiplied color.
fn blend_states(b: Blend) -> Vec<wgpu::BlendState> {
    use wgpu::{BlendComponent as C, BlendFactor as F, BlendOperation as O};
    let c = |src_factor, dst_factor| C { src_factor, dst_factor, operation: O::Add };
    let over = c(F::One, F::OneMinusSrcAlpha);
    match b {
        Blend::Normal => vec![wgpu::BlendState { color: over, alpha: over }],
        Blend::Add => vec![wgpu::BlendState { color: c(F::One, F::One), alpha: c(F::One, F::One) }],
        Blend::Screen => vec![wgpu::BlendState { color: c(F::One, F::OneMinusSrc), alpha: over }],
        // s*d + d*(1-sa) + s*(1-da) needs two passes: the multiply with alpha held, then the source
        // where nothing was below
        Blend::Multiply => vec![
            wgpu::BlendState { color: c(F::Dst, F::OneMinusSrcAlpha), alpha: c(F::Zero, F::One) },
            wgpu::BlendState { color: c(F::OneMinusDstAlpha, F::One), alpha: c(F::OneMinusDstAlpha, F::One) },
        ],
    }
}

const BLENDS: [Blend; 4] = [Blend::Normal, Blend::Add, Blend::Multiply, Blend::Screen];

/// Check WGSL with naga so a bad shader is a readable error instead of a crash.
pub fn check_wgsl(src: &str, what: &str) -> Result<()> {
    use wgpu::naga;
    let module = naga::front::wgsl::parse_str(src).map_err(|e| anyhow!("{what}: {}", e.emit_to_string(src)))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .map_err(|e| anyhow!("{what}: {}", e.emit_to_string(src)))?;
    Ok(())
}

impl Gpu {
    pub fn new() -> Result<Gpu> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|e| {
            anyhow!(
                "no GPU adapter found ({e}). On a Linux machine without a GPU, install a software Vulkan driver \
                 (Debian/Ubuntu: apt install mesa-vulkan-drivers)"
            )
        })?;
        let info = adapter.get_info();
        let al = adapter.limits();
        let limits = wgpu::Limits {
            max_texture_dimension_2d: al.max_texture_dimension_2d,
            max_buffer_size: al.max_buffer_size,
            max_storage_buffer_binding_size: al.max_storage_buffer_binding_size,
            ..wgpu::Limits::downlevel_defaults()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("yehi"),
            required_features: wgpu::Features::empty(),
            required_limits: limits.clone(),
            ..Default::default()
        }))?;
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            eprintln!("GPU error: {e}");
            std::process::exit(2);
        }));

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uniform_entry = |binding, dynamic| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: None,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[uniform_entry(0, false), sampler_entry(1)],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex"),
            entries: &[tex_entry(0)],
        });
        let shader_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader layer"),
            entries: &[uniform_entry(0, true), tex_entry(1), sampler_entry(2)],
        });
        let convert_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("convert"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                uniform_entry(2, false),
            ],
        });

        let composite_src = include_str!("shaders/composite.wgsl");
        check_wgsl(composite_src, "composite.wgsl")?;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite"),
            source: wgpu::ShaderSource::Wgsl(composite_src.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite"),
            bind_group_layouts: &[Some(&globals_layout), Some(&tex_layout), Some(&tex_layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4,
                                             5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4];
        let mut composite = HashMap::new();
        for b in BLENDS {
            let ps = blend_states(b)
                .into_iter()
                .map(|bs| {
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("composite"),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: &module,
                            entry_point: Some("vs"),
                            compilation_options: Default::default(),
                            buffers: &[Some(wgpu::VertexBufferLayout {
                                array_stride: std::mem::size_of::<Inst>() as u64,
                                step_mode: wgpu::VertexStepMode::Instance,
                                attributes: &attrs,
                            })],
                        },
                        primitive: wgpu::PrimitiveState::default(),
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: &module,
                            entry_point: Some("fs"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format: TARGET,
                                blend: Some(bs),
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        multiview_mask: None,
                        cache: None,
                    })
                })
                .collect();
            composite.insert(b, ps);
        }

        let convert_src = include_str!("shaders/convert.wgsl");
        check_wgsl(convert_src, "convert.wgsl")?;
        let cmod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("convert"),
            source: wgpu::ShaderSource::Wgsl(convert_src.into()),
        });
        let clayout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("convert"),
            bind_group_layouts: &[Some(&convert_layout)],
            immediate_size: 0,
        });
        let compute = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&clayout),
                module: &cmod,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let yuv = compute("yuv");
        let rgba = compute("rgba");

        let uni_align = device.limits().min_uniform_buffer_offset_alignment.max(256);
        let shader_module_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader layer"),
            bind_group_layouts: &[Some(&shader_layout)],
            immediate_size: 0,
        });
        let inst_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: 64 * 1024,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uni_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader uniforms"),
            size: 64 * 1024,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let max_texture = limits.max_texture_dimension_2d;
        let mut gpu = Gpu {
            device,
            queue,
            info,
            max_texture,
            sampler,
            globals_layout,
            tex_layout,
            shader_layout,
            convert_layout,
            composite,
            yuv,
            rgba,
            textures: vec![],
            targets: vec![],
            progs: vec![],
            inst_buf,
            uni_buf,
            uni_align,
            shader_module_layout,
        };
        gpu.texture_rgba8(1, 1, &[vec![255, 255, 255, 255]], "white");
        Ok(gpu)
    }

    fn add_texture(&mut self, texture: wgpu::Texture) -> usize {
        let view = texture.create_view(&Default::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.tex_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
        });
        let (w, h) = (texture.width(), texture.height());
        self.textures.push(Tex { texture, view, bind, w, h });
        self.textures.len() - 1
    }

    /// An RGBA8 (premultiplied) texture from its mip levels, largest first.
    pub fn texture_rgba8(&mut self, w: u32, h: u32, mips: &[Vec<u8>], label: &str) -> usize {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, data) in mips.iter().enumerate() {
            let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(lw * 4), rows_per_image: Some(lh) },
                wgpu::Extent3d { width: lw, height: lh, depth_or_array_layers: 1 },
            );
        }
        self.add_texture(texture)
    }

    pub fn texture_r8(&mut self, w: u32, h: u32, data: &[u8]) -> usize {
        let texture = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("glyphs"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            data,
        );
        self.add_texture(texture)
    }

    pub fn write_rgba8(&self, tex: usize, data: &[u8]) {
        let t = &self.textures[tex];
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &t.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(t.w * 4), rows_per_image: Some(t.h) },
            wgpu::Extent3d { width: t.w, height: t.h, depth_or_array_layers: 1 },
        );
    }

    fn float_texture(&mut self, w: u32, h: u32, label: &str) -> usize {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.add_texture(texture)
    }

    /// A render target of a size; `backdrop` gives it a copy texture for shaders that read what's below.
    pub fn target(&mut self, w: u32, h: u32, backdrop: bool) -> Result<usize> {
        if w > self.max_texture || h > self.max_texture {
            bail!("{w}x{h} is larger than this GPU's largest texture ({})", self.max_texture);
        }
        let tex = self.float_texture(w, h, "target");
        let backdrop = backdrop.then(|| self.float_texture(w, h, "backdrop"));
        let ubuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals"),
            contents: bytemuck::cast_slice(&[w as f32, h as f32, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let globals = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &self.globals_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.targets.push(Target { tex, globals, backdrop });
        Ok(self.targets.len() - 1)
    }

    /// Compile a shader layer drawing into `target`.
    pub fn shader_prog(&mut self, src: &str, what: &str, target: usize) -> Result<usize> {
        check_wgsl(src, what)?;
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(what),
            source: wgpu::ShaderSource::Wgsl(src.into()),
        });
        let mut pipelines = HashMap::new();
        for b in BLENDS {
            let ps = blend_states(b)
                .into_iter()
                .map(|bs| {
                    self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some(what),
                        layout: Some(&self.shader_module_layout),
                        vertex: wgpu::VertexState {
                            module: &module,
                            entry_point: Some("vs"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: wgpu::PrimitiveState::default(),
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: &module,
                            entry_point: Some("fs"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format: TARGET,
                                blend: Some(bs),
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        multiview_mask: None,
                        cache: None,
                    })
                })
                .collect();
            pipelines.insert(b, ps);
        }
        let backdrop = self.targets[target].backdrop.unwrap_or(WHITE);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(what),
            layout: &self.shader_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uni_buf,
                        offset: 0,
                        size: wgpu::BufferSize::new(SHADER_UNIFORM_SIZE),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.textures[backdrop].view),
                },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.progs.push(ShaderProg { pipelines, bind });
        Ok(self.progs.len() - 1)
    }

    /// Upload a frame's instances and shader uniforms, growing the buffers when needed.
    pub fn upload(&mut self, inst: &[Inst], uniforms: &[u8]) {
        let ib = bytemuck::cast_slice(inst);
        if ib.len() as u64 > self.inst_buf.size() {
            self.inst_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instances"),
                size: (ib.len() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !ib.is_empty() {
            self.queue.write_buffer(&self.inst_buf, 0, ib);
        }
        if uniforms.len() as u64 > self.uni_buf.size() {
            // the shader programs' bind groups point at the old buffer; rebuild by growing in place is not possible,
            // so the scene sizes this up front (see Scene::uniform_capacity)
            panic!("shader uniform buffer too small");
        }
        if !uniforms.is_empty() {
            self.queue.write_buffer(&self.uni_buf, 0, uniforms);
        }
    }

    pub fn ensure_uniform_capacity(&mut self, bytes: u64) {
        if bytes > self.uni_buf.size() {
            self.uni_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("shader uniforms"),
                size: bytes.next_power_of_two(),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
    }

    /// Record a target's ops: draws share one render pass until a shader needs the backdrop or a group
    /// needs its own layer.
    pub fn record(&self, enc: &mut wgpu::CommandEncoder, target: usize, clear: [f64; 4], ops: &[Op]) {
        let mut cleared = false;
        let mut i = 0;
        while i < ops.len() {
            if let Op::Isolate { target: t, ops: inner } = &ops[i] {
                self.record(enc, *t, [0.0; 4], inner);
                i += 1;
                continue;
            }
            if self.needs_backdrop(&ops[i], target) {
                if !cleared {
                    self.pass(enc, target, Some(clear), &[]);
                    cleared = true;
                }
                let tg = &self.targets[target];
                let src = &self.textures[tg.tex].texture;
                let dst = &self.textures[tg.backdrop.unwrap()].texture;
                enc.copy_texture_to_texture(src.as_image_copy(), dst.as_image_copy(), src.size());
            }
            let mut j = i + 1;
            while j < ops.len() && !matches!(ops[j], Op::Isolate { .. }) && !self.needs_backdrop(&ops[j], target) {
                j += 1;
            }
            self.pass(enc, target, (!cleared).then_some(clear), &ops[i..j]);
            cleared = true;
            i = j;
        }
        if !cleared {
            self.pass(enc, target, Some(clear), &[]);
        }
    }

    fn needs_backdrop(&self, op: &Op, target: usize) -> bool {
        matches!(op, Op::Shader { .. }) && self.targets[target].backdrop.is_some() && self.shader_reads_backdrop(op)
    }

    fn shader_reads_backdrop(&self, op: &Op) -> bool {
        match op {
            Op::Shader { uoff, .. } => (*uoff & READS_BACKDROP) != 0,
            _ => false,
        }
    }

    fn pass(&self, enc: &mut wgpu::CommandEncoder, target: usize, clear: Option<[f64; 4]>, ops: &[Op]) {
        let tg = &self.targets[target];
        let view = &self.textures[tg.tex].view;
        let load = match clear {
            Some(c) => wgpu::LoadOp::Clear(wgpu::Color { r: c[0] * c[3], g: c[1] * c[3], b: c[2] * c[3], a: c[3] }),
            None => wgpu::LoadOp::Load,
        };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let stride = std::mem::size_of::<Inst>() as u64;
        for op in ops {
            match op {
                Op::Draw { blend, tex, mask, first, count } => {
                    pass.set_bind_group(0, &tg.globals, &[]);
                    pass.set_bind_group(1, &self.textures[*tex].bind, &[]);
                    pass.set_bind_group(2, &self.textures[*mask].bind, &[]);
                    pass.set_vertex_buffer(
                        0,
                        self.inst_buf.slice(*first as u64 * stride..(*first + *count) as u64 * stride),
                    );
                    for p in &self.composite[blend] {
                        pass.set_pipeline(p);
                        pass.draw(0..6, 0..*count);
                    }
                }
                Op::Shader { prog, blend, uoff } => {
                    let p = &self.progs[*prog];
                    pass.set_bind_group(0, &p.bind, &[*uoff & !READS_BACKDROP]);
                    for pl in &p.pipelines[blend] {
                        pass.set_pipeline(pl);
                        pass.draw(0..6, 0..1);
                    }
                }
                Op::Isolate { .. } => unreachable!(),
            }
        }
    }

    /// Convert the composition target to YUV 4:2:0 or RGBA8 in `out`.
    pub fn convert(&self, enc: &mut wgpu::CommandEncoder, target: usize, out: &wgpu::Buffer, frame: u32, yuv: bool) {
        let tg = &self.targets[target];
        let t = &self.textures[tg.tex];
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("convert"),
            contents: bytemuck::cast_slice(&[t.w, t.h, frame, 0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("convert"),
            layout: &self.convert_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&t.view) },
                wgpu::BindGroupEntry { binding: 1, resource: out.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
            ],
        });
        let mut cp = enc.begin_compute_pass(&Default::default());
        cp.set_pipeline(if yuv { &self.yuv } else { &self.rgba });
        cp.set_bind_group(0, &bind, &[]);
        if yuv {
            cp.dispatch_workgroups((t.w / 8).div_ceil(8), (t.h / 2).div_ceil(8), 1);
        } else {
            cp.dispatch_workgroups(t.w.div_ceil(8), t.h.div_ceil(8), 1);
        }
    }
}

/// Size of one shader layer's uniform block (must match `U` in shader_layer.wgsl).
pub const SHADER_UNIFORM_SIZE: u64 = 5 * 16 + 16 * 16;
/// Flag carried in a Shader op's uniform offset: the shader reads the backdrop. Offsets are 256-aligned,
/// so the low bit is free.
pub const READS_BACKDROP: u32 = 1;
