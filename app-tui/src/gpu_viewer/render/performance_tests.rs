//! Playback measurement (run explicitly with --ignored --nocapture).
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
                            0.02 * p[0] * p[0] * (std::f64::consts::TAU * s as f64 / 240.0).sin(),
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
        r.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");
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
        r.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");
        eprintln!("GPU profile: adapter={:?}, vertices={n}, triangles={}, frames={}, sample_bytes={}, uniform_bytes_per_frame={}, upload_ms={:.3}, playback_ms={:.3}",adapter.get_info(),scene.triangles.len()/3,scene.times.len(),scene.samples.len()*16,r.uniform.size(),upload.as_secs_f64()*1000.0,start.elapsed().as_secs_f64()*1000.0);
    });
}
