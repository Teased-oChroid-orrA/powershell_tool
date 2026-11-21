//! Helpers shared by the headless and snapshot tests: the adapter, the serialising lock, texture read-back and a unit cube.
use super::*;

pub(super) static HEADLESS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// Software fallback adapter where one exists (Linux CI), otherwise the platform's default
/// adapter (Metal on macOS, DX12 on Windows); `None` when the machine has no graphics adapter.
pub(super) async fn test_adapter() -> Option<wgpu::Adapter> {
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
pub(super) fn headless_guard() -> std::sync::MutexGuard<'static, ()> {
    HEADLESS_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Copy an RGBA8 texture back to the CPU as tightly packed rows.
pub(super) fn read_back(renderer: &Renderer, texture: &wgpu::Texture) -> Vec<u8> {
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
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue.submit(Some(encoder.finish()));
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    renderer
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("device poll");
    rx.recv().unwrap().unwrap();
    let padded = buffer
        .slice(..)
        .get_mapped_range()
        .expect("mapped range")
        .to_vec();
    buffer.unmap();
    padded
        .chunks_exact(stride as usize)
        .flat_map(|row| row[..(width * 4) as usize].iter().copied())
        .collect()
}

pub(super) fn cube() -> Scene {
    let positions: Vec<[f32; 3]> = (0..8)
        .map(|i| {
            [
                (i & 1) as f32 - 0.5,
                ((i >> 1) & 1) as f32 - 0.5,
                ((i >> 2) & 1) as f32 - 0.5,
            ]
        })
        .collect();
    let quads = [
        [0, 1, 3, 2],
        [4, 6, 7, 5],
        [0, 4, 5, 1],
        [2, 3, 7, 6],
        [0, 2, 6, 4],
        [1, 5, 7, 3],
    ];
    Scene {
        label: "cube".into(),
        samples: vec![[0.0; 4]; 8],
        positions,
        triangles: quads
            .iter()
            .flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]])
            .collect(),
        times: vec![0.0],
        range: [0.0, 1.0],
        gain: 1.0,
        harmonic_period: None,
        field_label: String::new(),
        fields: Vec::new(),
    }
}
