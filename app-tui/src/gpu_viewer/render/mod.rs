//! The wgpu scene renderer: pipelines and buffers (`setup`, `buffers`), the display passes (`draw`) and the egui
//! chrome painted over the scene (`egui_paint`). Test helpers and the headless / snapshot tests are separate
//! `#[cfg(test)]` submodules.
use super::{
    display::{line_colors, DisplayStyle},
    Scene,
};
use wgpu::util::DeviceExt;

mod buffers;
mod draw;
mod egui_paint;
#[cfg(test)]
mod headless_tests;
#[cfg(test)]
mod performance_tests;
mod setup;
#[cfg(test)]
mod snapshot_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
use test_support::*;
#[cfg(test)]
mod tests;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    frames: [u32; 4],
    settings: [f32; 4],
    camera: [f32; 4],
    /// `xy`: screen-space shift of the view in NDC.
    pan: [f32; 4],
    /// Linear colours of the feature edges, the hidden edges, the mesh lines and the background's top and bottom.
    line: [f32; 4],
    hidden: [f32; 4],
    mesh: [f32; 4],
    bg_top: [f32; 4],
    bg_bottom: [f32; 4],
    /// Linear colours of the model's state codes 0..3 (body, support, load, selected).
    key: [[f32; 4]; 4],
}

/// Two faces meeting at more than this angle share a visible edge.
const FEATURE_CREASE_DEG: f32 = 35.0;

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    /// The surface written to the depth buffer only (hidden-line styles).
    depth_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    /// Only the state-coloured faces (support, load, selection), for the styles without a surface.
    highlight_pipeline: wgpu::RenderPipeline,
    hidden_pipeline: wgpu::RenderPipeline,
    /// The surface blended over what is already drawn at the pass's blend constant (`DisplayOptions::translucent`).
    translucent_pipeline: wgpu::RenderPipeline,
    bg_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    edges: wgpu::Buffer,
    edge_count: u32,
    /// The model's feature edges (`Scene::feature_edges`).
    feature: wgpu::Buffer,
    feature_count: u32,
    depth: wgpu::TextureView,
    size: (u32, u32),
    /// Paints the egui chrome over the scene (`paint_egui`).
    egui: egui_wgpu::Renderer,
    /// The target stores sRGB: a clear colour has to be given linear.
    srgb: bool,
}

/// What `draw_display` shows besides the geometry itself.
#[derive(Clone, Copy, Debug)]
pub struct DisplayOptions {
    pub show_mesh: bool,
    pub show_contour: bool,
    /// 0 = primary scalar of the scene, k = `scene.fields[k - 1]`.
    pub field: usize,
    /// Pixel rectangle `[x, y, width, height]` the scene is drawn into (docked UI panels surround it);
    /// `None` is the whole target. The caller passes the matching aspect in the camera parameters.
    pub viewport: Option<[u32; 4]>,
    /// Background (sRGB) behind the model as `(top, bottom)`; `None` is the dark default.
    pub background: Option<([u8; 3], [u8; 3])>,
    pub style: DisplayStyle,
    /// Period (s) of a swing between undeformed and deformed for a static result; `time` is the clock.
    pub swing: Option<f32>,
    /// Light the surface (a geometry view); a result keeps its contour colours exact.
    pub shade: bool,
    /// Surface contrast multiplier; 1.0 is the standard light, always above zero in the Thread Workbench.
    pub shade_intensity: f32,
    /// Colours (sRGB) of the geometry view's state codes; `None` draws them with the contour map. Set only with `shade`.
    pub key_colours: Option<[[u8; 3]; 4]>,
    /// Screen-space shift of the view in NDC units.
    pub pan: [f32; 2],
    /// `(first, opacity)`: the triangles from index `first` of `Scene::triangles` on are drawn at `opacity` over the
    /// ones before them, nearest layer only, in the styles that paint the surface (`Authoring::translucent`).
    pub translucent: Option<(u32, f32)>,
}
impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            show_mesh: false,
            show_contour: true,
            field: 0,
            viewport: None,
            background: None,
            style: DisplayStyle::Shaded,
            swing: None,
            shade: false,
            shade_intensity: 1.0,
            key_colours: None,
            pan: [0.0, 0.0],
            translucent: None,
        }
    }
}
