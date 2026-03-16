use super::*;

// ─── Uniform Buffer ──────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    screen_size: [f32; 2],
    _padding: [f32; 2],
}

// ─── EguiRenderer (low-level wgpu) ──────────────────────────────────────────

pub(super) struct EguiRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    textures: HashMap<egui::TextureId, (wgpu::Texture, wgpu::BindGroup)>,
    sampler: wgpu::Sampler,
}

impl EguiRenderer {
    pub(super) fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        // Shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("egui_shader"),
            source: wgpu::ShaderSource::Wgsl(EGUI_SHADER_WGSL.into()),
        });

        // Uniform bind group layout (group 0)
        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("egui_uniform_bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        // Texture bind group layout (group 1)
        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("egui_texture_bgl"),
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
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("egui_pipeline_layout"),
            bind_group_layouts: &[&uniform_bind_group_layout, &texture_bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("egui_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 20, // 2*f32 pos + 2*f32 uv + 4*u8 color = 20 bytes
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        // position
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        // uv
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        // color (unorm)
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Unorm8x4,
                            offset: 16,
                            shader_location: 2,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    // Premultiplied alpha blending — matches the explicit
                    // premultiply step in the fragment shader (same as
                    // official egui-wgpu).
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("egui_uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("egui_uniform_bg"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("egui_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            pipeline,
            uniform_buffer,
            uniform_bind_group,
            texture_bind_group_layout,
            textures: HashMap::new(),
            sampler,
        }
    }

    pub(super) fn update_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: egui::TextureId,
        delta: &egui::epaint::ImageDelta,
    ) {
        let pixels: Vec<u8> = match &delta.image {
            egui::ImageData::Color(image) => {
                image.pixels.iter().flat_map(|c| c.to_array()).collect()
            }
            egui::ImageData::Font(image) => image
                .srgba_pixels(None)
                .flat_map(|c| c.to_array())
                .collect(),
        };

        let size = delta.image.size();
        let width = size[0] as u32;
        let height = size[1] as u32;

        if let Some(pos) = delta.pos {
            // Partial update
            if let Some((tex, _)) = self.textures.get(&id) {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: pos[0] as u32,
                            y: pos[1] as u32,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 4),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
            }
        } else {
            // Full creation/replacement
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("egui_texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            let view = texture.create_view(&Default::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("egui_tex_bg"),
                layout: &self.texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });

            self.textures.insert(id, (texture, bind_group));
        }
    }

    pub(super) fn free_texture(&mut self, id: egui::TextureId) {
        self.textures.remove(&id);
    }

    pub(super) fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        color_target: &wgpu::TextureView,
        primitives: &[egui::ClippedPrimitive],
        screen: &ScreenDescriptor,
    ) {
        let width_points = screen.width_px as f32 / screen.pixels_per_point;
        let height_points = screen.height_px as f32 / screen.pixels_per_point;

        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[Uniforms {
                screen_size: [width_points, height_points],
                _padding: [0.0, 0.0],
            }]),
        );

        // Gather all meshes into a single vertex + index buffer
        let mut vertices: Vec<u8> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut draw_calls: Vec<DrawCall> = Vec::new();

        for prim in primitives {
            match &prim.primitive {
                egui::epaint::Primitive::Mesh(mesh) => {
                    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                        continue;
                    }
                    let base_vertex = (vertices.len() / 20) as i32;
                    let index_start = indices.len() as u32;

                    for v in &mesh.vertices {
                        vertices.extend_from_slice(bytemuck::bytes_of(&v.pos.x));
                        vertices.extend_from_slice(bytemuck::bytes_of(&v.pos.y));
                        vertices.extend_from_slice(bytemuck::bytes_of(&v.uv.x));
                        vertices.extend_from_slice(bytemuck::bytes_of(&v.uv.y));
                        vertices.extend_from_slice(&v.color.to_array());
                    }
                    indices.extend_from_slice(&mesh.indices);

                    let index_end = indices.len() as u32;

                    // Compute scissor rect in physical pixels
                    let clip = &prim.clip_rect;
                    let ppp = screen.pixels_per_point;
                    let x = (clip.min.x * ppp).round().max(0.0) as u32;
                    let y = (clip.min.y * ppp).round().max(0.0) as u32;
                    let w = ((clip.max.x - clip.min.x) * ppp).round().max(1.0) as u32;
                    let h = ((clip.max.y - clip.min.y) * ppp).round().max(1.0) as u32;
                    // Clamp to screen
                    let w = w.min(screen.width_px.saturating_sub(x));
                    let h = h.min(screen.height_px.saturating_sub(y));

                    if w == 0 || h == 0 {
                        continue;
                    }

                    draw_calls.push(DrawCall {
                        texture_id: mesh.texture_id,
                        scissor: [x, y, w, h],
                        index_range: index_start..index_end,
                        base_vertex,
                    });
                }
                egui::epaint::Primitive::Callback(_) => {
                    log::warn!("egui paint callbacks not supported in custom renderer");
                }
            }
        }

        if draw_calls.is_empty() {
            return;
        }

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("egui_vb"),
            contents: &vertices,
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("egui_ib"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("egui_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load, // Don't clear — overlay on top of terminal grid
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);

        for dc in &draw_calls {
            if let Some((_, bg)) = self.textures.get(&dc.texture_id) {
                pass.set_bind_group(1, bg, &[]);
                pass.set_scissor_rect(dc.scissor[0], dc.scissor[1], dc.scissor[2], dc.scissor[3]);
                pass.draw_indexed(dc.index_range.clone(), dc.base_vertex, 0..1);
            }
        }
    }
}

struct DrawCall {
    texture_id: egui::TextureId,
    scissor: [u32; 4],
    index_range: std::ops::Range<u32>,
    base_vertex: i32,
}

// ─── WGSL Shader ─────────────────────────────────────────────────────────────

const EGUI_SHADER_WGSL: &str = r#"
struct Uniforms {
    screen_size: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(1) @binding(0) var t_texture: texture_2d<f32>;
@group(1) @binding(1) var t_sampler: sampler;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,  // unorm
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

// Convert sRGB → linear (approximate gamma 2.2).
fn linear_from_srgb(srgb: vec3<f32>) -> vec3<f32> {
    return pow(srgb, vec3<f32>(2.2));
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    // Convert from egui logical pixels to NDC
    out.position = vec4<f32>(
        2.0 * input.position.x / uniforms.screen_size.x - 1.0,
        1.0 - 2.0 * input.position.y / uniforms.screen_size.y,
        0.0,
        1.0,
    );
    out.uv = input.uv;
    // Pass sRGB vertex color through — conversion happens in the
    // fragment shader (matches official egui-wgpu).
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_texture, t_sampler, input.uv);

    // Convert vertex color from sRGB to linear.
    var color = vec4<f32>(linear_from_srgb(input.color.rgb), input.color.a);

    // Multiply with texture (already linear via Rgba8UnormSrgb sampling).
    color = color * tex_color;

    // Premultiply alpha for PREMULTIPLIED_ALPHA_BLENDING.
    color = vec4<f32>(color.rgb * color.a, color.a);

    return color;
}
"#;

// Re-export wgpu::util for buffer_init
use wgpu::util::DeviceExt;
