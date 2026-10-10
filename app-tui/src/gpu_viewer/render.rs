use super::Scene;
use wgpu::util::DeviceExt;
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
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
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
                    visibility: wgpu::ShaderStages::VERTEX,
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
        Ok(Self {
            device,
            queue,
            pipeline,
            bind,
            uniform,
            vertices,
            indices,
            count: scene.triangles.len() as u32,
            depth,
        })
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
        let (a, b, blend) = scene.frame_at(time);
        self.queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&Params {
                frames: [a as u32, b as u32, scene.positions.len() as u32, 0],
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
            };
            let renderer = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, 64, 64)
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
            let read = |time| {
                renderer.draw(
                    &texture.create_view(&Default::default()),
                    &scene,
                    time,
                    1.0,
                    [0.0, 0.0, 1.6, 1.0],
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
            let a = read(0.0);
            let b = read(1.0);
            let mid = read(0.5);
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
