use super::Scene;
use wgpu::util::DeviceExt;
#[cfg(test)]
static HEADLESS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    frames: [u32; 4],
    settings: [f32; 4],
    camera: [f32; 4],
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    edges: wgpu::Buffer,
    edge_count: u32,
    depth: wgpu::TextureView,
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
        let bytes = scene.samples.len() as u64 * 16;
        if bytes > limits.max_storage_buffer_binding_size as u64 || bytes > limits.max_buffer_size {
            return Err("scene exceeds this adapter's storage-buffer limit".into());
        }
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("FEA viewport"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default().using_resolution(limits),
                },
                None,
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
            size: 48,
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
            ],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: samples.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("direct WGSL deformation"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("FEA surface"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs",
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
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
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
        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("FEA mesh overlay"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_mesh",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: -1,
                    slope_scale: 0.0,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            multiview: None,
        });
        let edge_data = Self::edge_indices(scene);
        let edges = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh edges"),
            contents: bytemuck::cast_slice(&edge_data),
            usage: wgpu::BufferUsages::INDEX,
        });
        Ok(Self {
            device,
            queue,
            pipeline,
            mesh_pipeline,
            layout,
            bind,
            uniform,
            vertices,
            indices,
            count: scene.triangles.len() as u32,
            edges,
            edge_count: edge_data.len() as u32,
            depth,
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
        let bytes = scene.samples.len() as u64 * 16;
        let limits = self.device.limits();
        if bytes > limits.max_storage_buffer_binding_size as u64 || bytes > limits.max_buffer_size {
            return Err("mode exceeds this adapter's storage-buffer limit".into());
        }
        let samples = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("selected mode samples"),
                contents: bytemuck::cast_slice(&scene.samples),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: samples.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.uniform.as_entire_binding(),
                },
            ],
        });
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
        self.draw_display(view, scene, time, gain, camera, false, true);
    }
    #[allow(clippy::too_many_arguments)]
    pub fn draw_display(
        &self,
        view: &wgpu::TextureView,
        scene: &Scene,
        time: f32,
        gain: f32,
        camera: [f32; 4],
        show_mesh: bool,
        show_contour: bool,
    ) {
        let (a, b, blend) = scene.frame_at(time);
        let gain = scene.harmonic_period.map_or(gain, |period| {
            gain * (std::f32::consts::TAU * time / period).sin()
        });
        self.queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&Params {
                frames: [
                    a as u32,
                    b as u32,
                    scene.positions.len() as u32,
                    u32::from(!show_contour),
                ],
                settings: [blend, gain, scene.range[0], scene.range[1]],
                camera,
            }),
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.025,
                            g: 0.03,
                            b: 0.045,
                            a: 1.0,
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
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.count, 0, 0..1);
            if show_mesh {
                pass.set_pipeline(&self.mesh_pipeline);
                pass.set_index_buffer(self.edges.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.edge_count, 0, 0..1);
            }
        }
        self.queue.submit(Some(encoder.finish()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a native graphics adapter; explicitly exercised during Phase 18 validation"]
    fn headless_deformation_changes_pixels() {
        let _guard = HEADLESS_LOCK.lock().unwrap();
        pollster::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: None,
                    force_fallback_adapter: true,
                })
                .await
                .expect("headless adapter required");
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
                    mesh,
                    contour,
                );
                let buffer = renderer.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: 64 * 256,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = renderer.device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_buffer(
                    wgpu::ImageCopyTexture {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::ImageCopyBuffer {
                        buffer: &buffer,
                        layout: wgpu::ImageDataLayout {
                            offset: 0,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(64),
                        },
                    },
                    wgpu::Extent3d {
                        width: 64,
                        height: 64,
                        depth_or_array_layers: 1,
                    },
                );
                renderer.queue.submit(Some(encoder.finish()));
                let (tx, rx) = std::sync::mpsc::channel();
                buffer
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
                renderer.device.poll(wgpu::Maintain::Wait);
                rx.recv().unwrap().unwrap();
                let bytes = buffer.slice(..).get_mapped_range().to_vec();
                buffer.unmap();
                bytes
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
        let _guard = HEADLESS_LOCK.lock().unwrap();
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
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    force_fallback_adapter: true,
                    ..Default::default()
                })
                .await
                .expect("adapter required");
            let start = std::time::Instant::now();
            let r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, 512, 256)
                .await
                .unwrap();
            r.device.poll(wgpu::Maintain::Wait);
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
            r.device.poll(wgpu::Maintain::Wait);
            eprintln!("GPU profile: adapter={:?}, vertices={n}, triangles={}, frames={}, sample_bytes={}, uniform_bytes_per_frame=48, upload_ms={:.3}, playback_ms={:.3}",adapter.get_info(),scene.triangles.len()/3,scene.times.len(),scene.samples.len()*16,upload.as_secs_f64()*1000.0,start.elapsed().as_secs_f64()*1000.0);
        });
    }
}
