//! Viewport, pan and deformation render tests.
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
            samples: vec![
                [0.0, 0.0, 0.0, 0.2],
                [0.0, 0.0, 0.0, 0.5],
                [0.0, 0.0, 0.0, 0.8],
            ],
            times: vec![0.0],
            range: [0.0, 1.0],
            gain: 1.0,
            harmonic_period: None,
            field_label: String::new(),
            fields: Vec::new(),
        };
        let (w, h) = (128u32, 96u32);
        let renderer = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h)
            .await
            .unwrap();
        let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let draw = |viewport| {
            renderer.draw_display(
                &texture.create_view(&Default::default()),
                &scene,
                0.0,
                1.0,
                [0.0, 0.0, 1.0, 1.0],
                DisplayOptions {
                    viewport,
                    ..Default::default()
                },
            );
            read_back(&renderer, &texture)
        };
        let clear = [
            (0.025f32 * 255.0).round() as u8,
            (0.03f32 * 255.0).round() as u8,
            (0.045f32 * 255.0).round() as u8,
        ];
        let is_clear = |px: &[u8]| (0..3).all(|k| px[k].abs_diff(clear[k]) <= 1);
        let full = draw(None);
        let painted = |img: &[u8]| img.chunks_exact(4).filter(|p| !is_clear(p)).count();
        assert!(
            painted(&full) > (w * h) as usize / 2,
            "the oversized triangle covers most of the target"
        );
        let rect = [32u32, 16, 64, 48];
        let img = draw(Some(rect));
        let mut inside = 0;
        for y in 0..h {
            for x in 0..w {
                let px = &img[((y * w + x) * 4) as usize..][..4];
                let in_rect =
                    x >= rect[0] && x < rect[0] + rect[2] && y >= rect[1] && y < rect[1] + rect[3];
                if in_rect {
                    inside += usize::from(!is_clear(px));
                } else {
                    assert!(
                        is_clear(px),
                        "pixel ({x}, {y}) outside the viewport was painted"
                    );
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
            samples: vec![
                [0.0, 0.0, 0.0, 0.2],
                [0.0, 0.0, 0.0, 0.5],
                [0.0, 0.0, 0.0, 0.8],
            ],
            times: vec![0.0],
            range: [0.0, 1.0],
            gain: 1.0,
            harmonic_period: None,
            field_label: String::new(),
            fields: Vec::new(),
        };
        let (w, h) = (128u32, 128u32);
        let renderer = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h)
            .await
            .unwrap();
        let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let centroid = |pan: [f32; 2]| {
            renderer.draw_display(
                &texture.create_view(&Default::default()),
                &scene,
                0.0,
                1.0,
                [0.0, 0.0, 1.0, 1.0],
                DisplayOptions {
                    pan,
                    ..Default::default()
                },
            );
            let img = read_back(&renderer, &texture);
            let clear = [
                (0.025f32 * 255.0).round() as u8,
                (0.03f32 * 255.0).round() as u8,
                (0.045f32 * 255.0).round() as u8,
            ];
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
        assert!(
            (x1 - x0 - 0.25 * f64::from(w)).abs() < 1.5,
            "x {x0} -> {x1}"
        );
        assert!(
            (y0 - y1 - 0.125 * f64::from(h)).abs() < 1.5,
            "y {y0} -> {y1}"
        );
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
        let mut renderer = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, 64, 64)
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
                DisplayOptions {
                    show_mesh: mesh,
                    show_contour: contour,
                    ..Default::default()
                },
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
            let o = DisplayOptions {
                field,
                ..Default::default()
            };
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
        assert_ne!(
            primary,
            draw_field(1),
            "field selection changes the contour"
        );
        assert_eq!(
            primary,
            draw_field(9),
            "out-of-range field falls back to the primary scalar"
        );
        // The egui chrome is painted over the scene without clearing it.
        let (tw, th) = (texture.width(), texture.height());
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(tw as f32, th as f32),
            )),
            ..Default::default()
        };
        let frame = ctx.run_ui(raw, |ui| {
            egui::Panel::top("t").exact_size(20.0).show(ui, |ui| {
                ui.label("chrome");
            });
        });
        let view = texture.create_view(&Default::default());
        renderer.draw_display(
            &view,
            &multi,
            0.0,
            1.0,
            [0.0, 0.0, 1.6, 1.0],
            DisplayOptions::default(),
        );
        let before = read_back(&renderer, &texture);
        renderer.paint_egui(&view, &ctx, frame, [tw, th]);
        let after = read_back(&renderer, &texture);
        assert_ne!(before, after, "the chrome changes pixels");
        let row =
            |img: &[u8], y: u32| img[(y * tw * 4) as usize..((y + 1) * tw * 4) as usize].to_vec();
        assert_eq!(
            row(&before, th - 1),
            row(&after, th - 1),
            "pixels below the panel keep the scene"
        );
        // A frame that could not be shown still carries texture changes (the font atlas grows with new text); handing
        // it to `discard_egui` keeps the next painted frame valid instead of panicking in egui.
        let frame = |text: &str| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(tw as f32, th as f32),
                )),
                ..Default::default()
            };
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
