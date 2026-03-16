//! Simple colored quad renderer for UI backgrounds (tab bar, overlay, etc.).
//!
//! Renders axis-aligned colored rectangles using a minimal wgpu pipeline.

use wgpu::{self, util::DeviceExt};

const SHADER_SRC: &str = r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

/// Maximum number of quads that can be drawn in a single frame.
const MAX_QUADS: usize = 512;
/// 6 vertices per quad (2 triangles).
const VERTS_PER_QUAD: usize = 6;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

/// A colored rectangle to draw on screen (coordinates in physical pixels).
#[derive(Debug, Clone, Copy)]
pub struct Quad {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// RGBA color, each component 0.0–1.0.
    pub color: [f32; 4],
}

/// GPU pipeline for drawing colored rectangles.
pub struct QuadRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    num_vertices: u32,
}

impl QuadRenderer {
    /// Create a new quad renderer for the given surface format.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad-pipeline-layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quad-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        // position
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        // color
                        wgpu::VertexAttribute {
                            offset: 8,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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

        // Pre-allocate vertex buffer for MAX_QUADS.
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad-vertex-buffer"),
            contents: &vec![0u8; MAX_QUADS * VERTS_PER_QUAD * std::mem::size_of::<Vertex>()],
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            pipeline,
            vertex_buffer,
            num_vertices: 0,
        }
    }

    /// Upload quads to the GPU. Call before render().
    /// Coordinates are in screen pixels; screen_width/screen_height are used to convert to NDC.
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        quads: &[Quad],
        screen_width: f32,
        screen_height: f32,
    ) {
        let count = quads.len().min(MAX_QUADS);
        let mut vertices = Vec::with_capacity(count * VERTS_PER_QUAD);

        for quad in &quads[..count] {
            // Convert pixel coordinates to NDC (-1..1).
            let x0 = (quad.x / screen_width) * 2.0 - 1.0;
            let y0 = 1.0 - (quad.y / screen_height) * 2.0;
            let x1 = ((quad.x + quad.width) / screen_width) * 2.0 - 1.0;
            let y1 = 1.0 - ((quad.y + quad.height) / screen_height) * 2.0;
            let c = quad.color;

            // Two triangles: top-left, top-right, bottom-left + bottom-left, top-right, bottom-right
            vertices.push(Vertex {
                position: [x0, y0],
                color: c,
            });
            vertices.push(Vertex {
                position: [x1, y0],
                color: c,
            });
            vertices.push(Vertex {
                position: [x0, y1],
                color: c,
            });
            vertices.push(Vertex {
                position: [x0, y1],
                color: c,
            });
            vertices.push(Vertex {
                position: [x1, y0],
                color: c,
            });
            vertices.push(Vertex {
                position: [x1, y1],
                color: c,
            });
        }

        self.num_vertices = vertices.len() as u32;
        if self.num_vertices > 0 {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }
    }

    /// Draw quads in the given render pass. Call after prepare().
    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.num_vertices == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.num_vertices, 0..1);
    }
}
