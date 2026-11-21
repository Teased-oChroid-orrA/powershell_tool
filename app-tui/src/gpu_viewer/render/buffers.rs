//! Scene buffers and bind groups: the storage-budget check, the per-scene upload and the depth target.
use super::*;

pub(super) fn field_buffer(device: &wgpu::Device, scene: &Scene) -> wgpu::Buffer {
    let flat: Vec<f32> = if scene.fields.is_empty() {
        vec![0.0; 4]
    } else {
        scene
            .fields
            .iter()
            .flat_map(|f| f.values.iter().copied())
            .collect()
    };
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("alternative scalar fields"),
        contents: bytemuck::cast_slice(&flat),
        usage: wgpu::BufferUsages::STORAGE,
    })
}
pub(super) fn scene_bind(
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
            wgpu::BindGroupEntry {
                binding: 0,
                resource: samples.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: fields.as_entire_binding(),
            },
        ],
    })
}
pub(super) fn check_budget(limits: &wgpu::Limits, scene: &Scene, what: &str) -> Result<(), String> {
    let max = limits
        .max_storage_buffer_binding_size
        .min(limits.max_buffer_size);
    let sample_bytes = scene.samples.len() as u64 * 16;
    let field_bytes = scene.positions.len() as u64 * scene.fields.len() as u64 * 4;
    if sample_bytes > max || field_bytes > max {
        return Err(format!(
            "{what} exceeds this adapter's storage-buffer limit"
        ));
    }
    Ok(())
}
impl Renderer {
    pub(super) fn edge_indices(scene: &Scene) -> Vec<u32> {
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
        self.feature = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
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
    pub(super) fn depth(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
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
}
