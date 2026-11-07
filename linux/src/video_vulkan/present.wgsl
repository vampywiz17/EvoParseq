struct Output { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vertex(@builtin(vertex_index) index: u32) -> Output {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Output;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}
struct Settings { crop: vec4<f32>, range: vec4<f32> }
@group(0) @binding(0) var y_plane: texture_2d<f32>;
@group(0) @binding(1) var uv_plane: texture_2d<f32>;
@group(0) @binding(2) var filtering: sampler;
@group(0) @binding(3) var<uniform> settings: Settings;
@fragment fn fragment(input: Output) -> @location(0) vec4<f32> {
    var uv_position = input.uv;
    let rotation = u32(settings.range.z);
    if rotation == 1u { uv_position = vec2<f32>(input.uv.y, 1.0 - input.uv.x); }
    if rotation == 2u { uv_position = vec2<f32>(1.0 - input.uv.x, 1.0 - input.uv.y); }
    if rotation == 3u { uv_position = vec2<f32>(1.0 - input.uv.y, input.uv.x); }
    let coordinates = uv_position * settings.crop.xy + settings.crop.zw;
    var y = textureSample(y_plane, filtering, coordinates).r;
    var uv = textureSample(uv_plane, filtering, coordinates).rg - vec2<f32>(128.0 / 255.0);
    if settings.range.x < 0.5 {
        y = (y - 16.0 / 255.0) * (255.0 / 219.0);
        uv *= 255.0 / 224.0;
    }
    var rgb = clamp(vec3<f32>(
        y + 1.5748 * uv.y,
        y - 0.187324 * uv.x - 0.468124 * uv.y,
        y + 1.8556 * uv.x), vec3<f32>(0.0), vec3<f32>(1.0));
    if settings.range.y > 0.5 {
        rgb = select(pow((rgb + 0.055) / 1.055, vec3<f32>(2.4)), rgb / 12.92, rgb <= vec3<f32>(0.04045));
    }
    return vec4<f32>(rgb, 1.0);
}
