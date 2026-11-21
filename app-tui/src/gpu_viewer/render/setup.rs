//! `Renderer::new`: the device, the pipelines and the initial scene buffers.
use super::{
    buffers::{check_budget, field_buffer, scene_bind},
    *,
};

impl Renderer {
    pub async fn new(
        adapter: &wgpu::Adapter,
        format: wgpu::TextureFormat,
        scene: &Scene,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        scene.validate()?;
        let limits = adapter.limits();
        check_budget(&limits, scene, "scene")?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("FEA viewport"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default().using_resolution(limits),
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        let samples = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("all deformation frames"),
            contents: bytemuck::cast_slice(&scene.samples),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("playback uniforms"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let fields = field_buffer(&device, scene);
        let bind = scene_bind(&device, &layout, &samples, &uniform, &fields);
        let egui = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("direct WGSL deformation"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shader.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |label: &str,
                    vs: &str,
                    fs: &str,
                    lines: bool,
                    depth_write: bool,
                    compare: wgpu::CompareFunction,
                    colour: wgpu::ColorWrites,
                    blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 12,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0=>Float32x3],
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: colour,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: if lines {
                        wgpu::PrimitiveTopology::LineList
                    } else {
                        wgpu::PrimitiveTopology::TriangleList
                    },
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::CompareFunction::{Always, Less, LessEqual};
        let pipeline = make(
            "FEA surface",
            "vs",
            "fs",
            false,
            true,
            Less,
            wgpu::ColorWrites::ALL,
            None,
        );
        let depth_pipeline = make(
            "FEA surface depth only",
            "vs",
            "fs",
            false,
            true,
            Less,
            wgpu::ColorWrites::empty(),
            None,
        );
        let mesh_pipeline = make(
            "FEA mesh overlay",
            "vs_mesh",
            "fs_mesh",
            true,
            false,
            LessEqual,
            wgpu::ColorWrites::ALL,
            None,
        );
        let highlight_pipeline = make(
            "FEA highlighted faces",
            "vs_mesh",
            "fs_highlight",
            false,
            false,
            LessEqual,
            wgpu::ColorWrites::ALL,
            None,
        );
        let edge_pipeline = make(
            "FEA feature edges",
            "vs_mesh",
            "fs_edge",
            true,
            false,
            LessEqual,
            wgpu::ColorWrites::ALL,
            None,
        );
        let hidden_pipeline = make(
            "FEA hidden edges",
            "vs_mesh",
            "fs_hidden",
            true,
            false,
            Always,
            wgpu::ColorWrites::ALL,
            None,
        );
        // colour = opacity * surface + (1 - opacity) * what is behind, the opacity being the pass's blend constant.
        let over = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Constant,
            dst_factor: wgpu::BlendFactor::OneMinusConstant,
            operation: wgpu::BlendOperation::Add,
        };
        let translucent_pipeline = make(
            "FEA translucent surface",
            "vs",
            "fs",
            false,
            false,
            LessEqual,
            wgpu::ColorWrites::ALL,
            Some(wgpu::BlendState {
                color: over,
                alpha: over,
            }),
        );
        let bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("FEA background"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_bg"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_bg"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&scene.positions),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&scene.triangles),
            usage: wgpu::BufferUsages::INDEX,
        });
        let depth = Self::depth(&device, width, height);
        let edge_data = Self::edge_indices(scene);
        let edges = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh edges"),
            contents: bytemuck::cast_slice(&edge_data),
            usage: wgpu::BufferUsages::INDEX,
        });
        let feature_data = scene.feature_edges(FEATURE_CREASE_DEG);
        let feature = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("feature edges"),
            contents: bytemuck::cast_slice(&feature_data),
            usage: wgpu::BufferUsages::INDEX,
        });
        Ok(Self {
            device,
            queue,
            pipeline,
            mesh_pipeline,
            depth_pipeline,
            edge_pipeline,
            highlight_pipeline,
            hidden_pipeline,
            translucent_pipeline,
            bg_pipeline,
            feature,
            feature_count: feature_data.len() as u32,
            layout,
            bind,
            uniform,
            vertices,
            indices,
            count: scene.triangles.len() as u32,
            edges,
            edge_count: edge_data.len() as u32,
            depth,
            size: (width.max(1), height.max(1)),
            egui,
            srgb: format.is_srgb(),
        })
    }
}
