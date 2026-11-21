//! Snapshot renders of every window (ignored without a native adapter; each writes raw RGBA dumps).
use super::*;
use crate::gpu_viewer::{
    controller::ViewState, editor::authoring::Authoring, gui::Gui, results::GuiInput, transport,
};

pub(super) fn target(r: &Renderer, w: u32, h: u32) -> wgpu::Texture {
    r.device.create_texture(&wgpu::TextureDescriptor {
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
    })
}

/// One chrome + scene frame into `texture`, like the window does: run egui twice (the second pass has the
/// measured sizes of the first), draw the scene into the viewport, paint the chrome.
pub(super) fn draw(
    r: &mut Renderer,
    gui: &mut Gui,
    texture: &wgpu::Texture,
    input: &GuiInput,
    camera: [f32; 2],
    show_mesh: bool,
) {
    let (w, h) = (texture.width(), texture.height());
    let raw = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w as f32, h as f32),
        )),
        ..Default::default()
    };
    let view = texture.create_view(&Default::default());
    let ctx = gui.ctx().clone();
    // The chrome sees the camera the scene is drawn with (the window's chrome and renderer share one `ViewState`).
    let synced = crate::gpu_viewer::controller::ViewState {
        yaw: camera[0],
        pitch: camera[1],
        zoom: 1.5,
        pan: [0.0, 0.0],
        ..input.view.clone()
    };
    let input = &GuiInput {
        view: &synced,
        scenes: input.scenes,
        selected: input.selected,
        editor: input.editor,
        error: input.error,
        box_drag: input.box_drag,
        overlay: input.overlay,
    };
    // The first passes only measure (a scroll area and a floating card take the size of what they held the
    // pass before); their texture uploads still have to reach the renderer.
    for _ in 0..3 {
        let (pass, _) = gui.run(raw(), input);
        r.paint_egui(&view, &ctx, pass, [w, h]);
    }
    let vp = gui.viewport_px((w, h));
    let cam = crate::gpu_viewer::editor::camera::Camera {
        yaw: camera[0],
        pitch: camera[1],
        zoom: 1.5,
        aspect: vp.w as f32 / vp.h as f32,
        pan: [0.0, 0.0],
    };
    let overlay = input
        .editor
        .map(|e| e.overlay(&cam, [vp.w as f32, vp.h as f32]))
        .unwrap_or_default();
    let with_overlay = GuiInput {
        view: input.view,
        scenes: input.scenes,
        selected: input.selected,
        editor: input.editor,
        error: input.error,
        box_drag: input.box_drag,
        overlay: &overlay,
    };
    let (output, _) = gui.run(raw(), &with_overlay);
    let geometry = input.editor.is_some_and(|e| e.geometry_view());
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
            viewport: Some(
                if input
                    .editor
                    .is_some_and(|e| !e.start_options().is_empty() || e.has_document())
                {
                    [vp.x, vp.y, 0, 0]
                } else {
                    [vp.x, vp.y, vp.w, vp.h]
                },
            ),
            background: Some((gui.background_rgb(), gui.background_rgb())),
            // Lit and in the colour key's colours, as the window draws a model (`window/`).
            style: input.view.style_for(input.scene(), geometry),
            shade: geometry,
            key_colours: geometry.then_some(input.view.key_colours),
            translucent: input.editor.and_then(|e| e.translucent()),
            ..Default::default()
        },
    );
    r.paint_egui(&view, &ctx, output, [w, h]);
}

mod analysis_workbenches;
mod chrome;
mod fit_workbenches;
mod model_pages;
mod search;
