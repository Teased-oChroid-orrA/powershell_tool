struct Params {
    frames: vec4<u32>, settings: vec4<f32>, camera: vec4<f32>, pan: vec4<f32>,
    line: vec4<f32>, hidden: vec4<f32>, mesh: vec4<f32>, bg_top: vec4<f32>, bg_bottom: vec4<f32>,
    key: array<vec4<f32>, 4>,
};
@group(0) @binding(0) var<storage, read> samples: array<vec4<f32>>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> fieldvals: array<f32>;
struct Out { @builtin(position) position: vec4<f32>, @location(0) value: f32, @location(1) view: vec3<f32>, };
fn place(pos: vec3<f32>, i: u32) -> Out {
    let a = samples[params.frames.x * params.frames.z + i];
    let b = samples[params.frames.y * params.frames.z + i];
    let s = mix(a, b, params.settings.x);
    let p = pos + s.xyz * params.settings.y;
    let cy = cos(params.camera.x); let sy = sin(params.camera.x);
    let cp = cos(params.camera.y); let sp = sin(params.camera.y);
    let q = vec3<f32>(cy*p.x+sy*p.z, p.y, -sy*p.x+cy*p.z);
    let r = vec3<f32>(q.x, cp*q.y-sp*q.z, sp*q.y+cp*q.z);
    var o: Out;
    o.position = vec4<f32>(r.x*params.camera.z/params.camera.w + params.pan.x, r.y*params.camera.z + params.pan.y, 0.5+r.z*0.25, 1.0);
    o.view = r;
    var v = s.w;
    let field = params.frames.w >> 8u;
    if field > 0u { v = fieldvals[(field - 1u) * params.frames.z + i]; }
    o.value = (v-params.settings.z)/max(params.settings.w-params.settings.z,1e-20);
    return o;
}
@vertex fn vs(@location(0) pos: vec3<f32>, @builtin(vertex_index) i: u32) -> Out { return place(pos, i); }
// Edge and mesh lines lie exactly on the surface; wgpu rejects a pipeline depth bias on line topology, so they are
// pulled toward the camera here instead (clip depth is 0.5 + 0.25 z, so 1e-5 is far below one pixel of parallax).
@vertex fn vs_mesh(@location(0) pos: vec3<f32>, @builtin(vertex_index) i: u32) -> Out {
    var o = place(pos, i);
    o.position.z = o.position.z - 1e-5;
    return o;
}
// The model's state colour at a normalised scene value (codes 0..3 over the 0..1 range): the user's key colours,
// blended between neighbouring codes exactly as the contour map is, so a face next to a support fades the same way.
fn key_colour(t: f32) -> vec3<f32> {
    let x = clamp(t, 0.0, 1.0) * 3.0;
    let i = min(u32(floor(x)), 2u);
    return mix(params.key[i].rgb, params.key[i + 1u].rgb, x - f32(i));
}
// Bit 0 of `frames.w`: no contour, paint the neutral body colour. Bit 1: light the surface from the viewer
// (a geometry view; a result keeps its contour colours exact).
@fragment fn fs(o: Out) -> @location(0) vec4<f32> {
    // Screen-space derivatives give the face normal without vertex normals; they must run in uniform control flow.
    let n = cross(dpdx(o.view), dpdy(o.view));
    let light = 0.58 + 0.42 * abs(n.z) / max(length(n), 1e-20);
    var c: vec3<f32>;
    if (params.frames.w & 1u) != 0u {
        c = vec3<f32>(0.6, 0.65, 0.72);
    } else {
        let t = clamp(o.value,0.0,1.0);
        c = vec3<f32>(t, 0.3+0.6*(1.0-abs(2.0*t-1.0)), 1.0-t);
        if (params.frames.w & 4u) != 0u { c = key_colour(o.value); }
    }
    if (params.frames.w & 2u) != 0u { c = c * light; }
    return vec4<f32>(c, 1.0);
}
// A surface-less style still shows the entities the user works on (a support, a load, the selection): only the
// faces coloured by their state (code above 0.5 on the geometry view's 0..3 scale) are painted.
@fragment fn fs_highlight(o: Out) -> @location(0) vec4<f32> {
    let code = o.value * (params.settings.w - params.settings.z) + params.settings.z;
    if code < 0.5 { discard; }
    let t = clamp(o.value, 0.0, 1.0);
    if (params.frames.w & 4u) != 0u { return vec4<f32>(key_colour(o.value), 1.0); }
    return vec4<f32>(t, 0.3 + 0.6 * (1.0 - abs(2.0 * t - 1.0)), 1.0 - t, 1.0);
}
@fragment fn fs_mesh(o: Out) -> @location(0) vec4<f32> { return vec4<f32>(params.mesh.rgb, 1.0); }
@fragment fn fs_edge(o: Out) -> @location(0) vec4<f32> { return vec4<f32>(params.line.rgb, 1.0); }
@fragment fn fs_hidden(o: Out) -> @location(0) vec4<f32> { return vec4<f32>(params.hidden.rgb, 1.0); }

struct Bg { @builtin(position) position: vec4<f32>, @location(0) y: f32, };
// One triangle covering the viewport; the gradient runs from its top edge to its bottom edge.
@vertex fn vs_bg(@builtin(vertex_index) i: u32) -> Bg {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    var o: Bg;
    o.position = vec4<f32>(x, y, 0.9999, 1.0);
    o.y = 0.5 - 0.5 * y;
    return o;
}
@fragment fn fs_bg(o: Bg) -> @location(0) vec4<f32> { return vec4<f32>(mix(params.bg_top.rgb, params.bg_bottom.rgb, clamp(o.y, 0.0, 1.0)), 1.0); }
