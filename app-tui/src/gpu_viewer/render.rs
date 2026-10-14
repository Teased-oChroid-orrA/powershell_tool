use super::{display::{line_colors, DisplayStyle}, Scene};
use wgpu::util::DeviceExt;
#[cfg(test)]
static HEADLESS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// Software fallback adapter where one exists (Linux CI), otherwise the platform's default
/// adapter (Metal on macOS, DX12 on Windows); `None` when the machine has no graphics adapter.
#[cfg(test)]
async fn test_adapter() -> Option<wgpu::Adapter> {
    let instance = wgpu::Instance::default();
    for fallback in [true, false] {
        if let Ok(a) = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: fallback,
                ..Default::default()
            })
            .await
        {
            return Some(a);
        }
    }
    None
}
#[cfg(test)]
fn headless_guard() -> std::sync::MutexGuard<'static, ()> {
    HEADLESS_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    frames: [u32; 4],
    settings: [f32; 4],
    camera: [f32; 4],
    /// `xy`: screen-space shift of the view in NDC.
    pan: [f32; 4],
    /// Linear colours of the feature edges, the hidden edges, the mesh lines and the background's top and bottom.
    line: [f32; 4],
    hidden: [f32; 4],
    mesh: [f32; 4],
    bg_top: [f32; 4],
    bg_bottom: [f32; 4],
}

/// Two faces meeting at more than this angle share a visible edge.
const FEATURE_CREASE_DEG: f32 = 35.0;

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    /// The surface written to the depth buffer only (hidden-line styles).
    depth_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    /// Only the state-coloured faces (support, load, selection), for the styles without a surface.
    highlight_pipeline: wgpu::RenderPipeline,
    hidden_pipeline: wgpu::RenderPipeline,
    bg_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    edges: wgpu::Buffer,
    edge_count: u32,
    /// The model's feature edges (`Scene::feature_edges`).
    feature: wgpu::Buffer,
    feature_count: u32,
    depth: wgpu::TextureView,
    size: (u32, u32),
    /// Paints the egui chrome over the scene (`paint_egui`).
    egui: egui_wgpu::Renderer,
    /// The target stores sRGB: a clear colour has to be given linear.
    srgb: bool,
}

/// What `draw_display` shows besides the geometry itself.
#[derive(Clone, Copy, Debug)]
pub struct DisplayOptions {
    pub show_mesh: bool,
    pub show_contour: bool,
    /// 0 = primary scalar of the scene, k = `scene.fields[k - 1]`.
    pub field: usize,
    /// Pixel rectangle `[x, y, width, height]` the scene is drawn into (docked UI panels surround it);
    /// `None` is the whole target. The caller passes the matching aspect in the camera parameters.
    pub viewport: Option<[u32; 4]>,
    /// Background (sRGB) behind the model as `(top, bottom)`; `None` is the dark default.
    pub background: Option<([u8; 3], [u8; 3])>,
    pub style: DisplayStyle,
    /// Light the surface (a geometry view); a result keeps its contour colours exact.
    pub shade: bool,
    /// Screen-space shift of the view in NDC units.
    pub pan: [f32; 2],
}
impl Default for DisplayOptions {
    fn default() -> Self {
        Self { show_mesh: false, show_contour: true, field: 0, viewport: None, background: None, style: DisplayStyle::Shaded, shade: false, pan: [0.0, 0.0] }
    }
}

fn field_buffer(device: &wgpu::Device, scene: &Scene) -> wgpu::Buffer {
    let flat: Vec<f32> = if scene.fields.is_empty() {
        vec![0.0; 4]
    } else {
        scene.fields.iter().flat_map(|f| f.values.iter().copied()).collect()
    };
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("alternative scalar fields"),
        contents: bytemuck::cast_slice(&flat),
        usage: wgpu::BufferUsages::STORAGE,
    })
}
fn scene_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    samples: &wgpu::Buffer,
    uniform: &wgpu::Buffer,
    fields: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: samples.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: fields.as_entire_binding() },
        ],
    })
}
fn check_budget(limits: &wgpu::Limits, scene: &Scene, what: &str) -> Result<(), String> {
    let max = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
    let sample_bytes = scene.samples.len() as u64 * 16;
    let field_bytes = scene.positions.len() as u64 * scene.fields.len() as u64 * 4;
    if sample_bytes > max || field_bytes > max {
        return Err(format!("{what} exceeds this adapter's storage-buffer limit"));
    }
    Ok(())
}
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
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("FEA viewport"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default().using_resolution(limits),
                    ..Default::default()
                },
            )
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
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |label: &str, vs: &str, fs: &str, lines: bool, depth_write: bool, compare: wgpu::CompareFunction, colour: wgpu::ColorWrites| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 12, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0=>Float32x3] })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: colour })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: if lines { wgpu::PrimitiveTopology::LineList } else { wgpu::PrimitiveTopology::TriangleList },
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
        let pipeline = make("FEA surface", "vs", "fs", false, true, Less, wgpu::ColorWrites::ALL);
        let depth_pipeline = make("FEA surface depth only", "vs", "fs", false, true, Less, wgpu::ColorWrites::empty());
        let mesh_pipeline = make("FEA mesh overlay", "vs_mesh", "fs_mesh", true, false, LessEqual, wgpu::ColorWrites::ALL);
        let highlight_pipeline = make("FEA highlighted faces", "vs_mesh", "fs_highlight", false, false, LessEqual, wgpu::ColorWrites::ALL);
        let edge_pipeline = make("FEA feature edges", "vs_mesh", "fs_edge", true, false, LessEqual, wgpu::ColorWrites::ALL);
        let hidden_pipeline = make("FEA hidden edges", "vs_mesh", "fs_hidden", true, false, Always, wgpu::ColorWrites::ALL);
        let bg_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("FEA background"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_bg"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_bg"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
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
    fn edge_indices(scene: &Scene) -> Vec<u32> {
        scene
            .triangles
            .chunks_exact(3)
            .flat_map(|t| [t[0], t[1], t[1], t[2], t[2], t[0]])
            .collect()
    }
    /// Change the active result on the same device and pipelines; upload immutable data once per selection.
    pub fn set_scene(&mut self, scene: &Scene) -> Result<(), String> {
        scene.validate()?;
        check_budget(&self.device.limits(), scene, "result")?;
        let samples = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("selected mode samples"),
                contents: bytemuck::cast_slice(&scene.samples),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let fields = field_buffer(&self.device, scene);
        let bind = scene_bind(&self.device, &self.layout, &samples, &self.uniform, &fields);
        let vertices = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&scene.positions),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let indices = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&scene.triangles),
                usage: wgpu::BufferUsages::INDEX,
            });
        let data = Self::edge_indices(scene);
        let edges = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::INDEX,
            });
        let feature_data = scene.feature_edges(FEATURE_CREASE_DEG);
        self.feature = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("feature edges"),
            contents: bytemuck::cast_slice(&feature_data),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.feature_count = feature_data.len() as u32;
        self.bind = bind;
        self.vertices = vertices;
        self.indices = indices;
        self.edges = edges;
        self.count = scene.triangles.len() as u32;
        self.edge_count = data.len() as u32;
        Ok(())
    }
    fn depth(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: width.max(1),
                    height: height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }
    pub fn resize(&mut self, w: u32, h: u32) {
        self.size = (w.max(1), h.max(1));
        self.depth = Self::depth(&self.device, w, h);
    }
    pub fn draw(
        &self,
        view: &wgpu::TextureView,
        scene: &Scene,
        time: f32,
        gain: f32,
        camera: [f32; 4],
    ) {
        self.draw_display(view, scene, time, gain, camera, DisplayOptions::default());
    }
    pub fn draw_display(
        &self,
        view: &wgpu::TextureView,
        scene: &Scene,
        time: f32,
        gain: f32,
        camera: [f32; 4],
        options: DisplayOptions,
    ) {
        let DisplayOptions { show_mesh, show_contour, field, viewport, background, style, shade, pan } = options;
        // A stale index (result switched, field absent) falls back to the primary scalar.
        let field = if field <= scene.fields.len() { field } else { 0 };
        let range = match field.checked_sub(1) {
            Some(k) => scene.fields[k].range,
            None => scene.range,
        };
        let (a, b, blend) = scene.frame_at(time);
        let gain = scene.harmonic_period.map_or(gain, |period| {
            gain * (std::f32::consts::TAU * time / period).sin()
        });
        let (top, bottom) = background.unwrap_or(([6, 8, 12], [6, 8, 12]));
        let lines = line_colors(top, bottom);
        let lin = |c: f32| {
            if !self.srgb { c } else if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let rgba = |c: [f32; 3]| [lin(c[0]), lin(c[1]), lin(c[2]), 1.0];
        let rgb8 = |c: [u8; 3]| rgba(c.map(|v| f32::from(v) / 255.0));
        self.queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&Params {
                frames: [
                    a as u32,
                    b as u32,
                    scene.positions.len() as u32,
                    u32::from(!show_contour) | u32::from(shade) << 1 | (field as u32) << 8,
                ],
                settings: [blend, gain, range[0], range[1]],
                camera,
                pan: [pan[0], pan[1], 0.0, 0.0],
                line: rgba(lines.visible),
                hidden: rgba(lines.hidden),
                mesh: rgba(lines.mesh),
                bg_top: rgb8(top),
                bg_bottom: rgb8(bottom),
            }),
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear({
                            let [r, g, b, _] = rgb8(top);
                            wgpu::Color { r: f64::from(r), g: f64::from(g), b: f64::from(b), a: 1.0 }
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                multiview_mask: None,
                occlusion_query_set: None,
            });
            // Clamp to the target: a viewport outside it is a validation error, an empty one draws nothing.
            let full = [0, 0, self.size.0, self.size.1];
            let vp = viewport.map_or(full, |[x, y, w, h]| {
                let (x, y) = (x.min(self.size.0), y.min(self.size.1));
                [x, y, w.min(self.size.0 - x), h.min(self.size.1 - y)]
            });
            let draw_scene = vp[2] > 0 && vp[3] > 0;
            pass.set_bind_group(0, &self.bind, &[]);
            // The background covers the whole window (the docks and the strip beside the viewport show it too);
            // the model is drawn into the viewport rectangle.
            if background.is_some_and(|(t, b)| t != b) {
                pass.set_viewport(0.0, 0.0, self.size.0 as f32, self.size.1 as f32, 0.0, 1.0);
                pass.set_scissor_rect(0, 0, self.size.0, self.size.1);
                pass.set_pipeline(&self.bg_pipeline);
                pass.draw(0..3, 0..1);
            }
            if draw_scene {
                pass.set_viewport(vp[0] as f32, vp[1] as f32, vp[2] as f32, vp[3] as f32, 0.0, 1.0);
                pass.set_scissor_rect(vp[0], vp[1], vp[2], vp[3]);
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                if style.occludes() {
                    pass.set_pipeline(if style.paints_surface() { &self.pipeline } else { &self.depth_pipeline });
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..self.count, 0, 0..1);
                }
                if shade && !style.paints_surface() {
                    pass.set_pipeline(&self.highlight_pipeline);
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..self.count, 0, 0..1);
                }
                if style.draws_edges() && self.feature_count > 0 {
                    pass.set_index_buffer(self.feature.slice(..), wgpu::IndexFormat::Uint32);
                    if style.draws_hidden_edges() {
                        pass.set_pipeline(&self.hidden_pipeline);
                        pass.draw_indexed(0..self.feature_count, 0, 0..1);
                    }
                    pass.set_pipeline(&self.edge_pipeline);
                    pass.draw_indexed(0..self.feature_count, 0, 0..1);
                }
                if show_mesh {
                    pass.set_pipeline(&self.mesh_pipeline);
                    pass.set_index_buffer(self.edges.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..self.edge_count, 0, 0..1);
                }
            }
        }
        self.queue.submit(Some(encoder.finish()));
    }

    /// Hand the frame's texture changes to the egui renderer (the font atlas grows as text appears); the textures
    /// egui wants freed are returned so they can be released after the frame is painted. Every frame's output must
    /// pass through here, painted or not: egui panics when texture changes are dropped unhandled.
    fn upload_egui_textures(&mut self, output: &mut egui::FullOutput) -> Vec<egui::TextureId> {
        for (id, deltas) in output.textures_delta.set.drain() {
            for delta in &deltas {
                self.egui.update_texture(&self.device, &self.queue, id, delta);
            }
        }
        std::mem::take(&mut output.textures_delta.free).into_iter().collect()
    }

    /// A frame that cannot be shown (no surface texture this time): keep the egui renderer's textures in step.
    pub fn discard_egui(&mut self, mut output: egui::FullOutput) {
        for id in self.upload_egui_textures(&mut output) {
            self.egui.free_texture(&id);
        }
    }

    /// Paint one egui frame over `view` (call after `draw_display`): uploads the frame's texture changes, tessellates
    /// its shapes and draws them without clearing what is there.
    pub fn paint_egui(&mut self, view: &wgpu::TextureView, ctx: &egui::Context, mut output: egui::FullOutput, size_px: [u32; 2]) {
        let ppp = output.pixels_per_point;
        let freed = self.upload_egui_textures(&mut output);
        let jobs = ctx.tessellate(std::mem::take(&mut output.shapes), ppp);
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: size_px, pixels_per_point: ppp };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let extra = self.egui.update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui chrome"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            self.egui.render(&mut pass, &jobs, &screen);
        }
        self.queue.submit(extra.into_iter().chain(Some(encoder.finish())));
        for id in &freed {
            self.egui.free_texture(id);
        }
    }
}

    /// Copy an RGBA8 texture back to the CPU as tightly packed rows.
    #[cfg(test)]
    fn read_back(renderer: &Renderer, texture: &wgpu::Texture) -> Vec<u8> {
        let (width, height) = (texture.width(), texture.height());
        let stride = (width * 4).div_ceil(256) * 256;
        let buffer = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (stride * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = renderer.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        renderer.queue.submit(Some(encoder.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
        renderer.device.poll(wgpu::PollType::wait_indefinitely()).expect("device poll");
        rx.recv().unwrap().unwrap();
        let padded = buffer.slice(..).get_mapped_range().expect("mapped range").to_vec();
        buffer.unmap();
        padded
            .chunks_exact(stride as usize)
            .flat_map(|row| row[..(width * 4) as usize].iter().copied())
            .collect()
    }

    #[cfg(test)]
    fn cube() -> Scene {
        let positions: Vec<[f32; 3]> = (0..8).map(|i| [(i & 1) as f32 - 0.5, ((i >> 1) & 1) as f32 - 0.5, ((i >> 2) & 1) as f32 - 0.5]).collect();
        let quads = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
        Scene {
            label: "cube".into(),
            samples: vec![[0.0; 4]; 8],
            positions,
            triangles: quads.iter().flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]]).collect(),
            times: vec![0.0],
            range: [0.0, 1.0],
            gain: 1.0,
            harmonic_period: None,
            field_label: String::new(),
            fields: Vec::new(),
        }
    }

    /// Surface-less styles still show the faces that carry a state (a support, a load, the selection), and the
    /// background gradient covers the whole target, not only the viewport the model is drawn into.
    #[test]
    #[ignore = "requires a native graphics adapter"]
    fn wireframe_highlights_state_faces_and_the_background_spans_the_window() {
        let _guard = headless_guard();
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let mut scene = cube();
            // The x = 0 face (vertices with bit 0 clear) is a load (code 2) on the geometry view's 0..3 scale.
            for (i, s) in scene.samples.iter_mut().enumerate() {
                s[3] = if i & 1 == 0 { 2.0 } else { 0.0 };
            }
            scene.range = [0.0, 3.0];
            let (w, h) = (160u32, 160u32);
            let renderer = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h).await.unwrap();
            let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let black = Some(([0u8; 3], [0u8; 3]));
            let draw = |style, shade, viewport, background| {
                renderer.draw_display(&texture.create_view(&Default::default()), &scene, 0.0, 1.0, [0.9, -0.4, 1.2, 1.0], DisplayOptions { style, shade, viewport, background, ..Default::default() });
                read_back(&renderer, &texture)
            };
            let painted = |img: &[u8]| img.chunks_exact(4).filter(|p| p[..3].iter().any(|c| *c > 3)).count();
            let plain = draw(DisplayStyle::Wireframe, false, None, black);
            let lit = draw(DisplayStyle::Wireframe, true, None, black);
            assert!(painted(&lit) > painted(&plain) + 500, "the state face is filled in wireframe: {} vs {}", painted(&lit), painted(&plain));
            let hlr = draw(DisplayStyle::HiddenLinesRemoved, true, None, black);
            assert!(painted(&hlr) > painted(&draw(DisplayStyle::HiddenLinesRemoved, false, None, black)), "and in hidden lines removed");
            // The gradient reaches the pixels beside the viewport rectangle.
            let img = draw(DisplayStyle::Wireframe, false, Some([60, 0, 40, 160]), Some(([0; 3], [200; 3])));
            let px = |x: u32, y: u32| img[((y * w + x) * 4) as usize];
            assert!(px(2, h - 3) > px(2, 2) + 150, "left strip: bottom {} top {}", px(2, h - 3), px(2, 2));
            assert!(px(w - 3, h - 3) > px(w - 3, 2) + 150, "right strip: bottom {} top {}", px(w - 3, h - 3), px(w - 3, 2));
        });
    }

    /// Each display style paints what CAD says: shaded fills the faces, wireframe only the edges, hidden lines
    /// removed only the edges in front, hidden lines visible those and the dimmed rest; a gradient runs top to bottom.
    #[test]
    #[ignore = "requires a native graphics adapter"]
    fn display_styles_paint_what_cad_says() {
        let _guard = headless_guard();
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let scene = cube();
            let (w, h) = (160u32, 160u32);
            let renderer = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h).await.unwrap();
            let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let black = Some(([0u8; 3], [0u8; 3]));
            let draw = |style, background| {
                renderer.draw_display(&texture.create_view(&Default::default()), &scene, 0.0, 1.0, [0.6, -0.5, 1.2, 1.0], DisplayOptions { style, background, ..Default::default() });
                read_back(&renderer, &texture)
            };
            let painted = |img: &[u8]| img.chunks_exact(4).filter(|p| p[..3].iter().any(|c| *c > 3)).count();
            let bright = |img: &[u8]| img.chunks_exact(4).filter(|p| p[..3].iter().all(|c| *c > 200)).count();
            let shaded = draw(DisplayStyle::Shaded, black);
            let with_edges = draw(DisplayStyle::ShadedEdges, black);
            let hlv = draw(DisplayStyle::HiddenLinesVisible, black);
            let hlr = draw(DisplayStyle::HiddenLinesRemoved, black);
            let wire = draw(DisplayStyle::Wireframe, black);
            assert!(painted(&shaded) > (w * h) as usize / 8, "shaded fills the faces");
            assert_eq!(bright(&shaded), 0, "shaded draws no edges");
            assert!(bright(&with_edges) > 100, "shaded with edges draws its edges");
            assert!(painted(&wire) < painted(&shaded) / 4 && bright(&wire) > 100, "wireframe is edges only");
            assert!(painted(&hlr) < painted(&shaded) / 4 && bright(&hlr) > 50, "hidden lines removed shows no fill, only edges");
            assert!(bright(&hlr) < bright(&wire), "the edges behind the cube are removed");
            assert!(painted(&hlv) > painted(&hlr), "hidden lines visible adds the dimmed hidden edges");
            assert_eq!(bright(&hlv), bright(&hlr), "the visible edges are the same ones as with hidden lines removed");
            // A gradient runs from the top colour to the bottom one.
            let grad = draw(DisplayStyle::Wireframe, Some(([0; 3], [200; 3])));
            let px = |img: &[u8], x: u32, y: u32| img[((y * w + x) * 4) as usize];
            assert!(px(&grad, 2, h - 3) > px(&grad, 2, 2) + 150, "bottom {} top {}", px(&grad, 2, h - 3), px(&grad, 2, 2));
        });
    }

#[cfg(test)]
mod tests {
    use super::*;
    /// The scene is drawn only inside the viewport rectangle; the rest of the target keeps the clear
    /// colour (the docked chrome surrounds it) and an out-of-range rectangle is clamped, not an error.
    #[test]
    #[ignore = "requires a native graphics adapter"]
    fn viewport_confines_the_scene_to_its_rectangle() {
        let _guard = headless_guard();
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let scene = Scene {
                label: "big".into(),
                positions: vec![[-1.5, -1.5, 0.0], [1.5, -1.5, 0.0], [0.0, 1.5, 0.0]],
                triangles: vec![0, 1, 2],
                samples: vec![[0.0, 0.0, 0.0, 0.2], [0.0, 0.0, 0.0, 0.5], [0.0, 0.0, 0.0, 0.8]],
                times: vec![0.0],
                range: [0.0, 1.0],
                gain: 1.0,
                harmonic_period: None,
                field_label: String::new(),
                fields: Vec::new(),
            };
            let (w, h) = (128u32, 96u32);
            let renderer = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h).await.unwrap();
            let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let draw = |viewport| {
                renderer.draw_display(&texture.create_view(&Default::default()), &scene, 0.0, 1.0, [0.0, 0.0, 1.0, 1.0], DisplayOptions { viewport, ..Default::default() });
                read_back(&renderer, &texture)
            };
            let clear = [(0.025f32 * 255.0).round() as u8, (0.03f32 * 255.0).round() as u8, (0.045f32 * 255.0).round() as u8];
            let is_clear = |px: &[u8]| (0..3).all(|k| px[k].abs_diff(clear[k]) <= 1);
            let full = draw(None);
            let painted = |img: &[u8]| img.chunks_exact(4).filter(|p| !is_clear(p)).count();
            assert!(painted(&full) > (w * h) as usize / 2, "the oversized triangle covers most of the target");
            let rect = [32u32, 16, 64, 48];
            let img = draw(Some(rect));
            let mut inside = 0;
            for y in 0..h {
                for x in 0..w {
                    let px = &img[((y * w + x) * 4) as usize..][..4];
                    let in_rect = x >= rect[0] && x < rect[0] + rect[2] && y >= rect[1] && y < rect[1] + rect[3];
                    if in_rect {
                        inside += usize::from(!is_clear(px));
                    } else {
                        assert!(is_clear(px), "pixel ({x}, {y}) outside the viewport was painted");
                    }
                }
            }
            assert!(inside > 0, "the scene is drawn inside the viewport");
            // A rectangle past the target edge is clamped; an empty one draws nothing and does not fail.
            assert!(painted(&draw(Some([100, 80, 500, 500]))) > 0);
            assert_eq!(painted(&draw(Some([10, 10, 0, 0]))), 0);
        });
    }
    #[test]
    #[ignore = "requires a native graphics adapter; explicitly exercised during Phase 18 validation"]
    fn pan_shifts_the_scene_on_the_screen_by_its_ndc_offset() {
        let _guard = headless_guard();
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let scene = Scene {
                label: "small".into(),
                positions: vec![[-0.2, -0.2, 0.0], [0.2, -0.2, 0.0], [0.0, 0.2, 0.0]],
                triangles: vec![0, 1, 2],
                samples: vec![[0.0, 0.0, 0.0, 0.2], [0.0, 0.0, 0.0, 0.5], [0.0, 0.0, 0.0, 0.8]],
                times: vec![0.0],
                range: [0.0, 1.0],
                gain: 1.0,
                harmonic_period: None,
                field_label: String::new(),
                fields: Vec::new(),
            };
            let (w, h) = (128u32, 128u32);
            let renderer = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h).await.unwrap();
            let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let centroid = |pan: [f32; 2]| {
                renderer.draw_display(&texture.create_view(&Default::default()), &scene, 0.0, 1.0, [0.0, 0.0, 1.0, 1.0], DisplayOptions { pan, ..Default::default() });
                let img = read_back(&renderer, &texture);
                let clear = [(0.025f32 * 255.0).round() as u8, (0.03f32 * 255.0).round() as u8, (0.045f32 * 255.0).round() as u8];
                let (mut sx, mut sy, mut n) = (0.0f64, 0.0f64, 0.0f64);
                for y in 0..h {
                    for x in 0..w {
                        let px = &img[((y * w + x) * 4) as usize..][..4];
                        if !(0..3).all(|k| px[k].abs_diff(clear[k]) <= 1) {
                            sx += f64::from(x);
                            sy += f64::from(y);
                            n += 1.0;
                        }
                    }
                }
                assert!(n > 100.0, "the triangle is drawn");
                (sx / n, sy / n)
            };
            let (x0, y0) = centroid([0.0, 0.0]);
            // NDC +0.5 is a quarter of the width to the right; +0.25 is an eighth of the height up (y grows downward).
            let (x1, y1) = centroid([0.5, 0.25]);
            assert!((x1 - x0 - 0.25 * f64::from(w)).abs() < 1.5, "x {x0} -> {x1}");
            assert!((y0 - y1 - 0.125 * f64::from(h)).abs() < 1.5, "y {y0} -> {y1}");
        });
    }
    #[test]
    #[ignore = "requires a native graphics adapter; explicitly exercised during Phase 18 validation"]
    fn headless_deformation_changes_pixels() {
        let _guard = headless_guard();
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("headless adapter required");
            eprintln!("adapter: {:?}", adapter.get_info());
            let scene = Scene {
                label: "test".into(),
                positions: vec![[-0.4, -0.3, 0.0], [0.2, -0.3, 0.0], [-0.1, 0.3, 0.0]],
                triangles: vec![0, 1, 2],
                samples: vec![
                    [0.0, 0.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0, 0.5],
                    [0.0, 0.0, 0.0, 1.0],
                    [0.3, 0.0, 0.0, 0.0],
                    [0.3, 0.0, 0.0, 0.5],
                    [0.3, 0.0, 0.0, 1.0],
                ],
                times: vec![0.0, 1.0],
                range: [0.0, 1.0],
                gain: 1.0,
                harmonic_period: None,
                field_label: String::new(),
                fields: Vec::new(),
            };
            let mut renderer =
                Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, 64, 64)
                    .await
                    .unwrap();
            let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let read = |renderer: &Renderer, scene: &Scene, time, mesh, contour| {
                renderer.draw_display(
                    &texture.create_view(&Default::default()),
                    scene,
                    time,
                    1.0,
                    [0.0, 0.0, 1.6, 1.0],
                    DisplayOptions { show_mesh: mesh, show_contour: contour, ..Default::default() },
                );
                read_back(renderer, &texture)
            };
            let a = read(&renderer, &scene, 0.0, false, true);
            let b = read(&renderer, &scene, 1.0, false, true);
            let mid = read(&renderer, &scene, 0.5, false, true);
            assert_ne!(
                a,
                read(&renderer, &scene, 0.0, true, true),
                "mesh toggle changes pixels"
            );
            assert_ne!(
                a,
                read(&renderer, &scene, 0.0, false, false),
                "contour toggle changes pixels"
            );
            let mut mode = scene.clone();
            mode.samples = vec![[0.3, 0.0, 0.0, 0.5]; 3];
            mode.times = vec![0.0];
            mode.harmonic_period = Some(1.0);
            renderer.set_scene(&mode).unwrap();
            assert_ne!(
                read(&renderer, &mode, 0.25, false, true),
                read(&renderer, &mode, 0.75, false, true),
                "compact modal sine changes geometry"
            );
            let mut invalid = mode.clone();
            invalid.triangles[0] = 100;
            assert!(renderer.set_scene(&invalid).is_err());
            // Alternative scalar field: different colours and its own range; stale index falls back.
            let mut multi = scene.clone();
            multi.fields = vec![crate::gpu_viewer::scene::ScalarField {
                label: "ramp".into(),
                range: [0.0, 1.0],
                values: vec![0.0, 1.0, 0.5],
            }];
            renderer.set_scene(&multi).unwrap();
            let draw_field = |field| {
                let o = DisplayOptions { field, ..Default::default() };
                renderer.draw_display(
                    &texture.create_view(&Default::default()),
                    &multi,
                    0.0,
                    1.0,
                    [0.0, 0.0, 1.6, 1.0],
                    o,
                );
                read_back(&renderer, &texture)
            };
            let primary = draw_field(0);
            assert_ne!(primary, draw_field(1), "field selection changes the contour");
            assert_eq!(primary, draw_field(9), "out-of-range field falls back to the primary scalar");
            // The egui chrome is painted over the scene without clearing it.
            let (tw, th) = (texture.width(), texture.height());
            let ctx = egui::Context::default();
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(tw as f32, th as f32))), ..Default::default() };
            let frame = ctx.run_ui(raw, |ui| {
                egui::Panel::top("t").exact_size(20.0).show(ui, |ui| {
                    ui.label("chrome");
                });
            });
            let view = texture.create_view(&Default::default());
            renderer.draw_display(&view, &multi, 0.0, 1.0, [0.0, 0.0, 1.6, 1.0], DisplayOptions::default());
            let before = read_back(&renderer, &texture);
            renderer.paint_egui(&view, &ctx, frame, [tw, th]);
            let after = read_back(&renderer, &texture);
            assert_ne!(before, after, "the chrome changes pixels");
            let row = |img: &[u8], y: u32| img[(y * tw * 4) as usize..((y + 1) * tw * 4) as usize].to_vec();
            assert_eq!(row(&before, th - 1), row(&after, th - 1), "pixels below the panel keep the scene");
            // A frame that could not be shown still carries texture changes (the font atlas grows with new text); handing
            // it to `discard_egui` keeps the next painted frame valid instead of panicking in egui.
            let frame = |text: &str| {
                let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(tw as f32, th as f32))), ..Default::default() };
                ctx.run_ui(raw, |ui| {
                    egui::Panel::top("t2").show(ui, |ui| {
                        ui.label(text);
                    });
                })
            };
            renderer.discard_egui(frame("never shown: \u{2603} glyphs the atlas has not seen"));
            renderer.paint_egui(&view, &ctx, frame("shown afterwards"), [tw, th]);
            assert_ne!(a, b);
            assert_ne!(a, mid);
            assert_ne!(b, mid);
            assert!(
                a.chunks_exact(4).any(|p| p[0] > 20 && p[2] > 20),
                "surface must paint colored pixels"
            );
        });
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    #[test]
    #[ignore = "native adapter playback measurement; run explicitly with --ignored --nocapture"]
    fn headless_playback_profile() {
        let _guard = headless_guard();
        pollster::block_on(async {
            let mesh = fea_core::generate::grid(
                fea_core::Physics::PlaneStress { thickness: 1.0 },
                fea_core::ElementKind::Quad4,
                fea_core::Elastic::new(1.0, 0.3),
                [64, 32, 1],
                &|p| [2.0 * p[0], p[1], 0.0],
            )
            .unwrap();
            let n = mesh.nodes.len();
            let frames: Vec<Vec<f64>> = (0..=240)
                .map(|s| {
                    mesh.nodes
                        .iter()
                        .flat_map(|p| {
                            [
                                0.0,
                                0.02 * p[0]
                                    * p[0]
                                    * (std::f64::consts::TAU * s as f64 / 240.0).sin(),
                            ]
                        })
                        .collect()
                })
                .collect();
            let scene = Scene::from_frames(
                &mesh,
                &frames,
                (0..=240).map(|s| s as f32 / 120.0).collect(),
                &mesh.nodes.iter().map(|p| p[0]).collect::<Vec<_>>(),
                "profile; illustrative harmonic field".into(),
            )
            .unwrap();
            let adapter = test_adapter().await.expect("adapter required");
            let start = std::time::Instant::now();
            let r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, 512, 256)
                .await
                .unwrap();
            r.device.poll(wgpu::PollType::wait_indefinitely()).expect("device poll");
            let upload = start.elapsed();
            let texture = r.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 512,
                    height: 256,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let start = std::time::Instant::now();
            for t in &scene.times {
                r.draw(&view, &scene, *t, scene.gain, [0.0, 0.0, 1.6, 2.0]);
            }
            r.device.poll(wgpu::PollType::wait_indefinitely()).expect("device poll");
            eprintln!("GPU profile: adapter={:?}, vertices={n}, triangles={}, frames={}, sample_bytes={}, uniform_bytes_per_frame=64, upload_ms={:.3}, playback_ms={:.3}",adapter.get_info(),scene.triangles.len()/3,scene.times.len(),scene.samples.len()*16,upload.as_secs_f64()*1000.0,start.elapsed().as_secs_f64()*1000.0);
        });
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::gpu_viewer::{controller::ViewState, editor::authoring::Authoring, gui::Gui, results::GuiInput, transport};

    fn target(r: &Renderer, w: u32, h: u32) -> wgpu::Texture {
        r.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// One chrome + scene frame into `texture`, like the window does: run egui twice (the second pass has the
    /// measured sizes of the first), draw the scene into the viewport, paint the chrome.
    fn draw(r: &mut Renderer, gui: &mut Gui, texture: &wgpu::Texture, input: &GuiInput, camera: [f32; 2], show_mesh: bool) {
        let (w, h) = (texture.width(), texture.height());
        let raw = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))), ..Default::default() };
        let view = texture.create_view(&Default::default());
        let ctx = gui.ctx().clone();
        // The first passes only measure (a scroll area and a floating card take the size of what they held the
        // pass before); their texture uploads still have to reach the renderer.
        for _ in 0..3 {
            let (pass, _) = gui.run(raw(), input);
            r.paint_egui(&view, &ctx, pass, [w, h]);
        }
        let vp = gui.viewport_px((w, h));
        let cam = crate::gpu_viewer::editor::camera::Camera { yaw: camera[0], pitch: camera[1], zoom: 1.5, aspect: vp.w as f32 / vp.h as f32, pan: [0.0, 0.0] };
        let overlay = input.editor.map(|e| e.overlay(&cam, [vp.w as f32, vp.h as f32])).unwrap_or_default();
        let with_overlay = GuiInput { view: input.view, scenes: input.scenes, selected: input.selected, editor: input.editor, error: input.error, box_drag: input.box_drag, overlay: &overlay };
        let (output, _) = gui.run(raw(), &with_overlay);
        r.draw_display(
            &view,
            input.scene(),
            input.scene().duration() * 0.5,
            input.view.gain,
            [camera[0], camera[1], 1.5, vp.w as f32 / vp.h as f32],
            DisplayOptions {
                show_mesh,
                show_contour: true,
                field: input.view.field,
                viewport: Some(if input.editor.is_some_and(|e| !e.start_options().is_empty()) { [vp.x, vp.y, 0, 0] } else { [vp.x, vp.y, vp.w, vp.h] }),
                background: Some((gui.background_rgb(), gui.background_rgb())),
                ..Default::default()
            },
        );
        r.paint_egui(&view, &ctx, output, [w, h]);
    }

    /// Renders a session file with the real chrome into a raw RGBA dump for visual inspection:
    /// `FEA_SNAPSHOT_SCENE=scene.json FEA_SNAPSHOT_OUT=out.rgba cargo test ... -- --ignored`.
    #[test]
    #[ignore = "needs a graphics adapter and FEA_SNAPSHOT_SCENE / FEA_SNAPSHOT_OUT"]
    fn hud_snapshot() {
        let (Ok(scene_path), Ok(out)) = (std::env::var("FEA_SNAPSHOT_SCENE"), std::env::var("FEA_SNAPSHOT_OUT")) else {
            return;
        };
        let _guard = headless_guard();
        let field: usize = std::env::var("FEA_SNAPSHOT_FIELD").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let project = transport::read_project(std::fs::File::open(scene_path).unwrap()).unwrap();
        let scene = &project.scenes[project.selected];
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let (w, h) = (1280u32, 800u32);
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, scene, w, h).await.unwrap();
            let mut view = ViewState::new(scene);
            view.field = field;
            view.show_help = std::env::var("FEA_SNAPSHOT_HELP").is_ok();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            let input = GuiInput { view: &view, scenes: &project.scenes, selected: project.selected, editor: None, error: None, box_drag: None, overlay: &[] };
            draw(&mut r, &mut gui, &texture, &input, [0.35, 0.4], true);
            std::fs::write(out, read_back(&r, &texture)).unwrap();
        });
    }

    /// The right-click menu of a selected face and the background panel over the editor window, as one raw RGBA dump:
    /// `FEA_SNAPSHOT_MENU_OUT=/dir cargo test ... context_menu_snapshot -- --ignored`.
    #[test]
    #[ignore = "needs a graphics adapter and FEA_SNAPSHOT_MENU_OUT"]
    fn context_menu_snapshot() {
        use crate::gpu_viewer::editor::solid::{SolidAction, SolidSession};
        let Ok(dir) = std::env::var("FEA_SNAPSHOT_MENU_OUT") else { return };
        let _guard = headless_guard();
        let mut problem = fea_problem::templates::templates().into_iter().map(|(_, p)| p).find(|p| p.analysis == fea_problem::Analysis::Solid).unwrap();
        problem.mesh.size = 0.3;
        let mut session = SolidSession::new(problem, "demo-menu.json".into());
        let t = std::time::Instant::now();
        while session.scene().map_or(true, |s| !s.label.contains("body (not meshed)")) {
            session.poll();
            assert!(t.elapsed().as_secs() < 60, "body did not finish");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        session.do_action(SolidAction::SelectSurface(1));
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let (w, h) = (1280u32, 800u32);
            let scene = session.scene().unwrap();
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h).await.unwrap();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            gui.ctx().set_theme(egui::Theme::Dark);
            let view = ViewState::new(&scene);
            let scenes = [scene];
            let input = GuiInput { view: &view, scenes: &scenes, selected: 0, editor: Some(&session as &dyn Authoring), error: None, box_drag: None, overlay: &[] };
            let ctx = crate::gpu_viewer::menus::MenuContext { view: &view, scene: input.scene(), results: 1, editor: input.editor, visible: Default::default(), adaptive: true };
            gui.open_context((640.0, 300.0), crate::gpu_viewer::menus::context(&ctx));
            gui.toggle_background_picker();
            draw(&mut r, &mut gui, &texture, &input, [-0.6, -0.5], false);
            std::fs::write(format!("{dir}/context.rgba"), read_back(&r, &texture)).unwrap();
            // The same face with the Pressure card open beside where the menu was.
            gui.toggle_background_picker();
            gui.close_floating();
            let mut session = session;
            session.do_action(SolidAction::OpenCard(crate::gpu_viewer::editor::solid::CardForm::Pressure));
            let input = GuiInput { view: &view, scenes: &scenes, selected: 0, editor: Some(&session as &dyn Authoring), error: None, box_drag: None, overlay: &[] };
            draw(&mut r, &mut gui, &texture, &input, [-0.6, -0.5], false);
            std::fs::write(format!("{dir}/card.rgba"), read_back(&r, &texture)).unwrap();
        });
    }

    /// The un-meshed 3D body in every display style and the same body meshed, as raw RGBA dumps:
    /// `FEA_SNAPSHOT_BODY_OUT=/dir cargo test ... body_styles_snapshot -- --ignored`.
    #[test]
    #[ignore = "needs a graphics adapter and FEA_SNAPSHOT_BODY_OUT"]
    fn body_styles_snapshot() {
        use crate::gpu_viewer::editor::solid::{SolidAction, SolidSession};
        let Ok(dir) = std::env::var("FEA_SNAPSHOT_BODY_OUT") else { return };
        let _guard = headless_guard();
        let mut problem = fea_problem::templates::templates().into_iter().map(|(_, p)| p).find(|p| p.analysis == fea_problem::Analysis::Solid).unwrap();
        problem.mesh.size = 0.3;
        let mut session = SolidSession::new(problem, "demo-body.json".into());
        let t = std::time::Instant::now();
        while session.scene().map_or(true, |s| !s.label.contains("body (not meshed)")) {
            session.poll();
            assert!(t.elapsed().as_secs() < 60, "body did not finish");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let body = session.scene().unwrap();
        session.do_action(SolidAction::GenerateMesh);
        while session.preview_summary().is_err() {
            session.poll();
            assert!(t.elapsed().as_secs() < 120, "mesh did not finish");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let meshed = session.scene().unwrap();
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let (w, h) = (640u32, 420u32);
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &body, w, h).await.unwrap();
            let texture = target(&r, w, h);
            let shot = |r: &mut Renderer, scene: &Scene, name: &str, style: DisplayStyle, mesh: bool, bg: ([u8; 3], [u8; 3])| {
                r.set_scene(scene).unwrap();
                r.draw_display(&texture.create_view(&Default::default()), scene, 0.0, 1.0, [-0.6, -0.5, 1.5, w as f32 / h as f32], DisplayOptions { show_mesh: mesh, style, shade: true, background: Some(bg), ..Default::default() });
                std::fs::write(format!("{dir}/{name}.rgba"), read_back(r, &texture)).unwrap();
            };
            let grey_white = ([112u8; 3], [255u8; 3]);
            for (k, style) in DisplayStyle::ALL.into_iter().enumerate() {
                shot(&mut r, &body, &format!("body-{k}"), style, false, grey_white);
            }
            shot(&mut r, &body, "hl-wire", DisplayStyle::Wireframe, false, grey_white);
            shot(&mut r, &body, "hl-hlr", DisplayStyle::HiddenLinesRemoved, false, grey_white);
            shot(&mut r, &meshed, "meshed-wire", DisplayStyle::Wireframe, true, ([0; 3], [112; 3]));
            shot(&mut r, &meshed, "meshed-shaded-edges", DisplayStyle::ShadedEdges, true, grey_white);
        });
    }

    /// The editor window in each of its states (light and dark), as raw RGBA dumps:
    /// `FEA_SNAPSHOT_EDITOR_OUT=/dir cargo test ... solid_snapshot -- --ignored`.
    #[test]
    #[ignore = "needs a graphics adapter and FEA_SNAPSHOT_EDITOR_OUT"]
    fn solid_snapshot() {
        use crate::gpu_viewer::editor::solid::SolidSession;
        let Ok(dir) = std::env::var("FEA_SNAPSHOT_EDITOR_OUT") else { return };
        let _guard = headless_guard();
        let mut problem = fea_problem::templates::templates().remove(0).1;
        problem.mesh.size = 0.4;
        let mut session = SolidSession::new(problem, "demo-solid.json".into());
        session.do_action(crate::gpu_viewer::editor::solid::SolidAction::GenerateMesh);
        let t = std::time::Instant::now();
        while session.preview_summary().is_err() {
            session.poll();
            assert!(t.elapsed().as_secs() < 60, "mesh did not finish");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        session.do_action(crate::gpu_viewer::editor::solid::SolidAction::SelectSurface(1));
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let (w, h) = (1600u32, 900u32);
            let first = session.scene().unwrap();
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &first, w, h).await.unwrap();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            let mut frame = |r: &mut Renderer, session: &mut SolidSession, name: &str, mesh_lines: bool, tab: Option<&str>, dark: bool| {
                gui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                let scene = session.scene().unwrap();
                r.set_scene(&scene).unwrap();
                let mut view = ViewState::new(&scene);
                view.show_mesh = mesh_lines;
                let scenes = [scene];
                let input = GuiInput { view: &view, scenes: &scenes, selected: 0, editor: Some(&*session as &dyn Authoring), error: None, box_drag: None, overlay: &[] };
                if let Some(tab) = tab {
                    gui.select_tab_named(&input, tab);
                }
                draw(r, &mut gui, &texture, &input, [0.0, 0.0], mesh_lines);
                std::fs::write(format!("{dir}/{name}.rgba"), read_back(r, &texture)).unwrap();
            };
            frame(&mut r, &mut session, "solid-before", true, Some("Mesh"), true);
            frame(&mut r, &mut session, "solid-before-light", true, Some("Model"), false);
            session.solve();
            let t = std::time::Instant::now();
            while !session.poll() || session.solving() {
                assert!(t.elapsed().as_secs() < 120);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            frame(&mut r, &mut session, "solid-after", false, Some("Simulation"), true);
            frame(&mut r, &mut session, "solid-after-light", false, Some("Simulation"), false);
            session.do_action(crate::gpu_viewer::editor::solid::SolidAction::ClearModel);
            frame(&mut r, &mut session, "solid-empty", false, Some("Model"), true);
            frame(&mut r, &mut session, "solid-empty-light", false, Some("Model"), false);
        });
    }

    /// The failure-guidance card and the status-bar fix after a failed solve (ADR-020).
    #[test]
    #[ignore = "needs a graphics adapter and FEA_SNAPSHOT_EDITOR_OUT"]
    fn guidance_snapshot() {
        use crate::gpu_viewer::editor::solid::{SolidAction, SolidSession};
        let Ok(dir) = std::env::var("FEA_SNAPSHOT_EDITOR_OUT") else { return };
        let _guard = headless_guard();
        let mut problem = fea_problem::templates::templates().remove(0).1;
        problem.mesh.size = 0.4;
        problem.supports = vec![fea_problem::Support::roller("left", 0)];
        let mut session = SolidSession::new(problem, "demo-guidance.json".into());
        session.do_action(SolidAction::GenerateMesh);
        let t = std::time::Instant::now();
        while session.preview_summary().is_err() {
            session.poll();
            assert!(t.elapsed().as_secs() < 60, "mesh did not finish");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        session.solve();
        let t = std::time::Instant::now();
        while !session.poll() || session.solving() {
            assert!(t.elapsed().as_secs() < 120);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let (w, h) = (1600u32, 900u32);
            let first = session.scene().unwrap();
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &first, w, h).await.unwrap();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            for (name, dark) in [("guidance-failed", true), ("guidance-failed-light", false)] {
                gui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                let scene = session.scene().unwrap();
                r.set_scene(&scene).unwrap();
                let view = ViewState::new(&scene);
                let scenes = [scene];
                let input = GuiInput { view: &view, scenes: &scenes, selected: 0, editor: Some(&session as &dyn Authoring), error: None, box_drag: None, overlay: &[] };
                gui.select_tab_named(&input, "Simulation");
                draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
                std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
            }
        });
    }

    /// The viewport while the pointer is on a guidance fix: the surface it would touch is lit, nothing is applied.
    #[test]
    #[ignore = "needs a graphics adapter and FEA_SNAPSHOT_EDITOR_OUT"]
    fn fix_preview_snapshot() {
        use crate::gpu_viewer::editor::solid::{FixRef, SolidAction, SolidSession};
        let Ok(dir) = std::env::var("FEA_SNAPSHOT_EDITOR_OUT") else { return };
        let _guard = headless_guard();
        let mut problem = fea_problem::templates::templates().remove(0).1;
        problem.mesh.size = 0.4;
        problem.supports.clear();
        let mut session = SolidSession::new(problem, "demo-fixpreview.json".into());
        session.do_action(SolidAction::GenerateMesh);
        let t = std::time::Instant::now();
        while session.preview_summary().is_err() {
            session.poll();
            assert!(t.elapsed().as_secs() < 60, "mesh did not finish");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let (w, h) = (1600u32, 900u32);
            let first = session.scene().unwrap();
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &first, w, h).await.unwrap();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            gui.ctx().set_theme(egui::Theme::Dark);
            for (name, fix) in [("fix-preview-off", None), ("fix-preview-on", Some(FixRef::Remedy(0, 0)))] {
                session.do_action(SolidAction::PreviewFix(fix));
                let scene = session.scene().unwrap();
                r.set_scene(&scene).unwrap();
                let view = ViewState::new(&scene);
                let scenes = [scene];
                let input = GuiInput { view: &view, scenes: &scenes, selected: 0, editor: Some(&session as &dyn Authoring), error: None, box_drag: None, overlay: &[] };
                draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
                std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
            }
        });
    }

    /// Sketch mode: a constrained plate outline with a hole, a half-drawn line and a selected edge.
    #[test]
    #[ignore = "needs a graphics adapter and FEA_SNAPSHOT_EDITOR_OUT"]
    fn sketch_snapshot() {
        use crate::gpu_viewer::editor::{solid::{SketchAction, SolidAction, SolidSession}, sketcher::Tool};
        use fea_problem::history::{FeatureKind, History, Plane};
        use fea_problem::sketch::{Constraint as C, Sketch};
        let Ok(dir) = std::env::var("FEA_SNAPSHOT_EDITOR_OUT") else { return };
        let _guard = headless_guard();
        let mut problem = fea_problem::templates::templates().remove(0).1;
        let mut sk = Sketch::default();
        let p = [sk.add_point(0.0, 0.0), sk.add_point(12.0, 0.0), sk.add_point(12.0, 4.0), sk.add_point(0.0, 4.0)];
        let l = [sk.add_line(p[0], p[1]), sk.add_line(p[1], p[2]), sk.add_line(p[2], p[3]), sk.add_line(p[3], p[0])];
        for (i, id) in l.iter().enumerate() {
            sk.add_constraint(if i % 2 == 0 { C::Horizontal { line: *id } } else { C::Vertical { line: *id } });
        }
        sk.add_constraint(C::Fixed { p: p[0], x: 0.0, y: 0.0 });
        sk.add_constraint(C::Length { line: l[0], d: 12.0 });
        sk.add_constraint(C::Length { line: l[1], d: 4.0 });
        sk.add_circle(6.0, 2.0, 0.8);
        let mut h = History::default();
        let id = h.add("Sketch1", FeatureKind::Sketch { plane: Plane::Front { offset: 0.0 }, sketch: sk });
        problem.history = Some(h);
        let mut session = SolidSession::new(problem, "demo-sketch.json".into());
        session.do_action(SolidAction::Sketch(SketchAction::Edit(id)));
        let finished = std::env::var("FEA_SNAPSHOT_FINISHED").is_ok();
        if finished {
            // The model after Finish, before any mesh exists: the sketch outline stays visible.
            session.do_action(SolidAction::Sketch(SketchAction::Finish));
        } else {
            session.do_action(SolidAction::Sketch(SketchAction::Tool(Tool::Line)));
        }
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let (w, h) = (1600u32, 900u32);
            let scene = session.scene().unwrap();
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h).await.unwrap();
            let texture = target(&r, w, h);
            r.set_scene(&scene).unwrap();
            let view = ViewState::new(&scene);
            let scenes = [scene];
            let input = GuiInput { view: &view, scenes: &scenes, selected: 0, editor: Some(&session as &dyn Authoring), error: None, box_drag: None, overlay: &[] };
            for (dark, suffix) in [(true, ""), (false, "-light")] {
                let mut gui = Gui::new();
                gui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
                std::fs::write(format!("{dir}/{}{suffix}.rgba", if finished { "sketch-finished" } else { "sketch" }), read_back(&r, &texture)).unwrap();
            }
        });
    }

    #[test]
    #[ignore = "needs a graphics adapter and FEA_SNAPSHOT_EDITOR_OUT"]
    fn editor_snapshot() {
        use crate::gpu_viewer::editor::{camera::Hit, caps::Operation, session::Session};
        let Ok(dir) = std::env::var("FEA_SNAPSHOT_EDITOR_OUT") else { return };
        let _guard = headless_guard();
        let template: usize = std::env::var("FEA_SNAPSHOT_TEMPLATE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut session = Session::new(fea_problem::members::templates().remove(template), "demo.members.json".into());
        session.state.select(Some(Hit::Node(session.state.problem.nodes.len() as u32)));
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let (w, h) = (1600u32, 900u32);
            let first = session.scene().unwrap();
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &first, w, h).await.unwrap();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            let mut frame = |r: &mut Renderer, session: &mut Session, name: &str| {
                let scene = session.scene().unwrap();
                r.set_scene(&scene).unwrap();
                let view = ViewState::new(&scene);
                let scenes = [scene];
                let input = GuiInput { view: &view, scenes: &scenes, selected: 0, editor: Some(&*session as &dyn Authoring), error: None, box_drag: None, overlay: &[] };
                draw(r, &mut gui, &texture, &input, [0.55, 0.35], false);
                std::fs::write(format!("{dir}/{name}.rgba"), read_back(r, &texture)).unwrap();
            };
            frame(&mut r, &mut session, "editor-before");
            session.run(Operation::Solve);
            let t = std::time::Instant::now();
            while !session.poll() {
                assert!(t.elapsed().as_secs() < 30);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            frame(&mut r, &mut session, "editor-after");
        });
    }
}
