//! The display passes: `draw` and `draw_display` (background, surface, hidden lines, edges, mesh, highlights).
use super::*;

impl Renderer {
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
        let DisplayOptions {
            show_mesh,
            show_contour,
            field,
            viewport,
            background,
            style,
            swing,
            shade,
            shade_intensity,
            key_colours,
            pan,
            translucent,
        } = options;
        // A stale index (result switched, field absent) falls back to the primary scalar.
        let field = if field <= scene.fields.len() {
            field
        } else {
            0
        };
        let range = match field.checked_sub(1) {
            Some(k) => scene.fields[k].range,
            None => scene.range,
        };
        let (a, b, blend) = scene.frame_at(time);
        let gain = scene.harmonic_period.map_or(gain, |period| {
            gain * (std::f32::consts::TAU * time / period).sin()
        });
        // A static result swings smoothly from undeformed to deformed and back.
        let gain = swing.map_or(gain, |period| {
            gain * 0.5 * (1.0 - (std::f32::consts::TAU * time / period).cos())
        });
        let (top, bottom) = background.unwrap_or(([6, 8, 12], [6, 8, 12]));
        let lines = line_colors(top, bottom);
        let lin = |c: f32| {
            if !self.srgb {
                c
            } else if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
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
                    u32::from(!show_contour)
                        | u32::from(shade) << 1
                        | u32::from(key_colours.is_some()) << 2
                        | (field as u32) << 8,
                ],
                settings: [blend, gain, range[0], range[1]],
                camera,
                pan: [pan[0], pan[1], shade_intensity, 0.0],
                line: rgba(lines.visible),
                hidden: rgba(lines.hidden),
                mesh: rgba(lines.mesh),
                bg_top: rgb8(top),
                bg_bottom: rgb8(bottom),
                key: key_colours.map_or([[0.0; 4]; 4], |k| k.map(rgb8)),
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
                            wgpu::Color {
                                r: f64::from(r),
                                g: f64::from(g),
                                b: f64::from(b),
                                a: 1.0,
                            }
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
                pass.set_viewport(
                    vp[0] as f32,
                    vp[1] as f32,
                    vp[2] as f32,
                    vp[3] as f32,
                    0.0,
                    1.0,
                );
                pass.set_scissor_rect(vp[0], vp[1], vp[2], vp[3]);
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                // A translucent tail is drawn after the solid triangles: its depth first, then its nearest layer blended
                // over them (one layer, so its own overlaps never darken it). A hidden-line style draws it as solid.
                let tail =
                    translucent
                        .filter(|_| style.paints_surface())
                        .map(|(first, opacity)| {
                            (first.min(self.count) / 3 * 3, opacity.clamp(0.0, 1.0))
                        });
                let solid = tail.map_or(self.count, |(first, _)| first);
                if style.occludes() {
                    pass.set_pipeline(if style.paints_surface() {
                        &self.pipeline
                    } else {
                        &self.depth_pipeline
                    });
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..solid, 0, 0..1);
                }
                if let Some((first, opacity)) = tail {
                    pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.set_pipeline(&self.depth_pipeline);
                    pass.draw_indexed(first..self.count, 0, 0..1);
                    pass.set_pipeline(&self.translucent_pipeline);
                    pass.set_blend_constant(wgpu::Color {
                        r: f64::from(opacity),
                        g: f64::from(opacity),
                        b: f64::from(opacity),
                        a: f64::from(opacity),
                    });
                    pass.draw_indexed(first..self.count, 0, 0..1);
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
}
