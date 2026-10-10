struct Params { frames: vec4<u32>, settings: vec4<f32>, camera: vec4<f32>, };
@group(0) @binding(0) var<storage, read> samples: array<vec4<f32>>;
@group(0) @binding(1) var<uniform> params: Params;
struct Out { @builtin(position) position: vec4<f32>, @location(0) value: f32, };
@vertex fn vs(@location(0) pos: vec3<f32>, @builtin(vertex_index) i: u32) -> Out {
    let a = samples[params.frames.x * params.frames.z + i];
    let b = samples[params.frames.y * params.frames.z + i];
    let s = mix(a, b, params.settings.x);
    let p = pos + s.xyz * params.settings.y;
    let cy = cos(params.camera.x); let sy = sin(params.camera.x);
    let cp = cos(params.camera.y); let sp = sin(params.camera.y);
    let q = vec3<f32>(cy*p.x+sy*p.z, p.y, -sy*p.x+cy*p.z);
    let r = vec3<f32>(q.x, cp*q.y-sp*q.z, sp*q.y+cp*q.z);
    var o: Out;
    o.position = vec4<f32>(r.x*params.camera.z/params.camera.w, r.y*params.camera.z, 0.5+r.z*0.25, 1.0);
    o.value = (s.w-params.settings.z)/max(params.settings.w-params.settings.z,1e-20);
    return o;
}
@fragment fn fs(o: Out) -> @location(0) vec4<f32> {
    if (params.frames.w & 1u) != 0u { return vec4<f32>(0.6, 0.65, 0.72, 1.0); }
    let t = clamp(o.value,0.0,1.0);
    return vec4<f32>(t, 0.3+0.6*(1.0-abs(2.0*t-1.0)), 1.0-t, 1.0);
}
@fragment fn fs_mesh(o: Out) -> @location(0) vec4<f32> { return vec4<f32>(0.04, 0.05, 0.07, 1.0); }
