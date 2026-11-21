//! Headless render tests on a native adapter (ignored where none exists): state faces, swing, translucency, key colours, styles.
use super::*;

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
        let black = Some(([0u8; 3], [0u8; 3]));
        let draw = |style, shade, viewport, background| {
            renderer.draw_display(
                &texture.create_view(&Default::default()),
                &scene,
                0.0,
                1.0,
                [0.9, -0.4, 1.2, 1.0],
                DisplayOptions {
                    style,
                    shade,
                    viewport,
                    background,
                    ..Default::default()
                },
            );
            read_back(&renderer, &texture)
        };
        let painted = |img: &[u8]| {
            img.chunks_exact(4)
                .filter(|p| p[..3].iter().any(|c| *c > 3))
                .count()
        };
        let plain = draw(DisplayStyle::Wireframe, false, None, black);
        let lit = draw(DisplayStyle::Wireframe, true, None, black);
        assert!(
            painted(&lit) > painted(&plain) + 500,
            "the state face is filled in wireframe: {} vs {}",
            painted(&lit),
            painted(&plain)
        );
        let hlr = draw(DisplayStyle::HiddenLinesRemoved, true, None, black);
        assert!(
            painted(&hlr) > painted(&draw(DisplayStyle::HiddenLinesRemoved, false, None, black)),
            "and in hidden lines removed"
        );
        // The gradient reaches the pixels beside the viewport rectangle.
        let img = draw(
            DisplayStyle::Wireframe,
            false,
            Some([60, 0, 40, 160]),
            Some(([0; 3], [200; 3])),
        );
        let px = |x: u32, y: u32| img[((y * w + x) * 4) as usize];
        assert!(
            px(2, h - 3) > px(2, 2) + 150,
            "left strip: bottom {} top {}",
            px(2, h - 3),
            px(2, 2)
        );
        assert!(
            px(w - 3, h - 3) > px(w - 3, 2) + 150,
            "right strip: bottom {} top {}",
            px(w - 3, h - 3),
            px(w - 3, 2)
        );
    });
}

/// A static result swings: at the start of a swing the part is undeformed, half a period later fully deformed.
#[test]
#[ignore = "requires a native graphics adapter"]
fn a_static_result_swings_between_undeformed_and_deformed() {
    let _guard = headless_guard();
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let mut scene = cube();
        // Every vertex moves +x by 0.4 in the one stored frame; a field makes it a result.
        scene.samples = scene
            .positions
            .iter()
            .map(|_| [0.4, 0.0, 0.0, 0.0])
            .collect();
        scene
            .fields
            .push(crate::gpu_viewer::scene::ScalarField::from_f64("f", &[0.0; 8]).unwrap());
        let (w, h) = (160u32, 160u32);
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
        let draw = |time: f32, swing: Option<f32>| {
            renderer.draw_display(
                &texture.create_view(&Default::default()),
                &scene,
                time,
                1.0,
                [0.0, 0.0, 1.0, 1.0],
                DisplayOptions {
                    swing,
                    ..Default::default()
                },
            );
            read_back(&renderer, &texture)
        };
        // The mean x of the painted pixels.
        let centre = |img: &[u8]| {
            let (mut sum, mut n) = (0.0f64, 0.0f64);
            for (i, p) in img.chunks_exact(4).enumerate() {
                if p[..3].iter().any(|c| *c > 20) {
                    sum += (i as u32 % w) as f64;
                    n += 1.0;
                }
            }
            sum / n.max(1.0)
        };
        let start = centre(&draw(0.0, Some(2.0)));
        let half = centre(&draw(1.0, Some(2.0)));
        let still = centre(&draw(1.0, None));
        let full = centre(&draw(0.0, None));
        let undeformed = centre(&draw(0.0, Some(2.0)));
        assert!(
            (start - undeformed).abs() < 1e-9,
            "the same time draws the same frame"
        );
        assert!(
            half > start + 10.0,
            "half a period later the part has moved right: {start} -> {half}"
        );
        assert!(
            (half - still).abs() < 1.5 && (still - full).abs() < 1.5,
            "a static draw with the same gain is the fully deformed one: {half} {still} {full}"
        );
    });
}
/// A translucent tail lets what it encloses show through: a small cube inside a big one is hidden while the big one
/// is solid, shows through it at the given opacity, and a hidden-line style ignores the translucency.
#[test]
#[ignore = "requires a native graphics adapter"]
fn a_translucent_tail_shows_the_solid_part_it_encloses() {
    let _guard = headless_guard();
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (inner, outer) = (cube(), cube());
        let mut scene = cube();
        // The inner cube (value 1, red end of the map) first, then the outer one (value 0, blue), twice its size.
        scene.positions = inner
            .positions
            .iter()
            .map(|p| p.map(|c| 0.4 * c))
            .chain(outer.positions.iter().copied())
            .collect();
        scene.samples = (0..16)
            .map(|i| [0.0, 0.0, 0.0, if i < 8 { 1.0 } else { 0.0 }])
            .collect();
        scene.triangles = inner
            .triangles
            .iter()
            .copied()
            .chain(outer.triangles.iter().map(|i| i + 8))
            .collect();
        let first = inner.triangles.len() as u32;
        let (w, h) = (160u32, 160u32);
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
        let draw = |style, translucent| {
            renderer.draw_display(
                &texture.create_view(&Default::default()),
                &scene,
                0.0,
                1.0,
                [0.9, -0.4, 1.2, 1.0],
                DisplayOptions {
                    style,
                    translucent,
                    ..Default::default()
                },
            );
            read_back(&renderer, &texture)
        };
        let reddish = |img: &[u8]| {
            img.chunks_exact(4)
                .filter(|p| p[0] as i32 > p[2] as i32 + 20)
                .count()
        };
        assert_eq!(
            reddish(&draw(DisplayStyle::Shaded, None)),
            0,
            "a solid outer cube hides the inner one"
        );
        let seen = reddish(&draw(DisplayStyle::Shaded, Some((first, 0.35))));
        assert!(seen > 300, "the inner cube shows through: {seen} pixels");
        assert!(
            reddish(&draw(DisplayStyle::Shaded, Some((first, 0.9)))) < seen,
            "less of it through a more opaque cube"
        );
        assert_eq!(
            reddish(&draw(DisplayStyle::HiddenLinesRemoved, Some((first, 0.35)))),
            0,
            "hidden lines draw it solid"
        );
    });
}

/// The model's state colours are the user's: a face coded as a support is drawn in the chosen colour, and the default
/// colours are what the contour map always gave.
#[test]
#[ignore = "requires a native graphics adapter"]
fn the_key_colours_decide_how_the_models_states_are_drawn() {
    let _guard = headless_guard();
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let mut scene = cube();
        for (i, s) in scene.samples.iter_mut().enumerate() {
            s[3] = if i & 1 == 0 { 1.0 } else { 0.0 };
        }
        scene.range = [0.0, 3.0];
        let (w, h) = (160u32, 160u32);
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
        let draw = |key: Option<[[u8; 3]; 4]>, style| {
            renderer.draw_display(
                &texture.create_view(&Default::default()),
                &scene,
                0.0,
                1.0,
                [0.9, -0.4, 1.2, 1.0],
                DisplayOptions {
                    style,
                    shade: true,
                    key_colours: key,
                    ..Default::default()
                },
            );
            read_back(&renderer, &texture)
        };
        let red = |img: &[u8]| {
            img.chunks_exact(4)
                .filter(|p| {
                    p[0] > 120 && p[0] as i32 > p[1] as i32 + 80 && p[0] as i32 > p[2] as i32 + 80
                })
                .count()
        };
        let default = crate::gpu_viewer::display::default_key_colours();
        let plain = draw(None, DisplayStyle::Shaded);
        let same = draw(Some(default), DisplayStyle::Shaded);
        // The defaults reproduce the contour map's colours (to rounding).
        let diff = plain
            .iter()
            .zip(&same)
            .filter(|(a, b)| (**a as i32 - **b as i32).abs() > 2)
            .count();
        assert!(
            diff < 40,
            "{diff} bytes differ between the map and its default key"
        );
        assert_eq!(red(&plain), 0, "teal support faces have no red");
        let mut custom = default;
        custom[1] = [255, 0, 0];
        assert!(
            red(&draw(Some(custom), DisplayStyle::Shaded)) > 300,
            "the support face is red when the support colour is"
        );
        // The highlight pass of a wireframe uses the key too.
        assert!(
            red(&draw(Some(custom), DisplayStyle::Wireframe)) > 100,
            "and in a wireframe"
        );
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
        let black = Some(([0u8; 3], [0u8; 3]));
        let draw = |style, background| {
            renderer.draw_display(
                &texture.create_view(&Default::default()),
                &scene,
                0.0,
                1.0,
                [0.6, -0.5, 1.2, 1.0],
                DisplayOptions {
                    style,
                    background,
                    ..Default::default()
                },
            );
            read_back(&renderer, &texture)
        };
        let painted = |img: &[u8]| {
            img.chunks_exact(4)
                .filter(|p| p[..3].iter().any(|c| *c > 3))
                .count()
        };
        let bright = |img: &[u8]| {
            img.chunks_exact(4)
                .filter(|p| p[..3].iter().all(|c| *c > 200))
                .count()
        };
        let shaded = draw(DisplayStyle::Shaded, black);
        let with_edges = draw(DisplayStyle::ShadedEdges, black);
        let hlv = draw(DisplayStyle::HiddenLinesVisible, black);
        let hlr = draw(DisplayStyle::HiddenLinesRemoved, black);
        let wire = draw(DisplayStyle::Wireframe, black);
        assert!(
            painted(&shaded) > (w * h) as usize / 8,
            "shaded fills the faces"
        );
        assert_eq!(bright(&shaded), 0, "shaded draws no edges");
        assert!(
            bright(&with_edges) > 100,
            "shaded with edges draws its edges"
        );
        assert!(
            painted(&wire) < painted(&shaded) / 4 && bright(&wire) > 100,
            "wireframe is edges only"
        );
        assert!(
            painted(&hlr) < painted(&shaded) / 4 && bright(&hlr) > 50,
            "hidden lines removed shows no fill, only edges"
        );
        assert!(
            bright(&hlr) < bright(&wire),
            "the edges behind the cube are removed"
        );
        assert!(
            painted(&hlv) > painted(&hlr),
            "hidden lines visible adds the dimmed hidden edges"
        );
        assert_eq!(
            bright(&hlv),
            bright(&hlr),
            "the visible edges are the same ones as with hidden lines removed"
        );
        // A gradient runs from the top colour to the bottom one.
        let grad = draw(DisplayStyle::Wireframe, Some(([0; 3], [200; 3])));
        let px = |img: &[u8], x: u32, y: u32| img[((y * w + x) * 4) as usize];
        assert!(
            px(&grad, 2, h - 3) > px(&grad, 2, 2) + 150,
            "bottom {} top {}",
            px(&grad, 2, h - 3),
            px(&grad, 2, 2)
        );
    });
}
