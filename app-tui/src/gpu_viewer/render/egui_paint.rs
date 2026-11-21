//! Painting the egui chrome over the scene: texture upload, discard and the paint pass.
use super::*;

impl Renderer {
    /// Hand the frame's texture changes to the egui renderer (the font atlas grows as text appears); the textures
    /// egui wants freed are returned so they can be released after the frame is painted. Every frame's output must
    /// pass through here, painted or not: egui panics when texture changes are dropped unhandled.
    pub(super) fn upload_egui_textures(
        &mut self,
        output: &mut egui::FullOutput,
    ) -> Vec<egui::TextureId> {
        for (id, deltas) in output.textures_delta.set.drain() {
            for delta in &deltas {
                self.egui
                    .update_texture(&self.device, &self.queue, id, delta);
            }
        }
        std::mem::take(&mut output.textures_delta.free)
            .into_iter()
            .collect()
    }

    /// A frame that cannot be shown (no surface texture this time): keep the egui renderer's textures in step.
    pub fn discard_egui(&mut self, mut output: egui::FullOutput) {
        for id in self.upload_egui_textures(&mut output) {
            self.egui.free_texture(&id);
        }
    }

    /// Paint one egui frame over `view` (call after `draw_display`): uploads the frame's texture changes, tessellates
    /// its shapes and draws them without clearing what is there.
    pub fn paint_egui(
        &mut self,
        view: &wgpu::TextureView,
        ctx: &egui::Context,
        mut output: egui::FullOutput,
        size_px: [u32; 2],
    ) {
        let ppp = output.pixels_per_point;
        let freed = self.upload_egui_textures(&mut output);
        let jobs = ctx.tessellate(std::mem::take(&mut output.shapes), ppp);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size_px,
            pixels_per_point: ppp,
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let extra =
            self.egui
                .update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui chrome"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            self.egui.render(&mut pass, &jobs, &screen);
        }
        self.queue
            .submit(extra.into_iter().chain(Some(encoder.finish())));
        for id in &freed {
            self.egui.free_texture(id);
        }
    }
}
