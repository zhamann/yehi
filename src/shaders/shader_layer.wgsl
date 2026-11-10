// Prelude for shader layers. The layer's code becomes the body of user_main; see docs/spec.md.

struct U {
    m: vec4f,
    t_size: vec4f,
    res: vec2f,
    size: vec2f,
    time: f32,
    t: f32,
    frame: f32,
    dur: f32,
    opacity: f32,
    _a: f32,
    _b: f32,
    _c: f32,
    p: array<vec4f, 16>,
}
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var backdrop_tex: texture_2d<f32>;
@group(0) @binding(2) var backdrop_samp: sampler;

/// What's been drawn below this layer, premultiplied, at a pixel position.
fn backdrop(px: vec2f) -> vec4f {
    return textureSampleLevel(backdrop_tex, backdrop_samp, px / u.res, 0.0);
}

/// A stable 0..1 hash of a 2D point.
fn hash(p: vec2f) -> f32 {
    var q = vec2u(vec2i(floor(p)));
    var h = q.x * 374761393u + q.y * 668265263u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffffu) / 16777215.0;
}

/// Smooth value noise in 0..1.
fn noise(p: vec2f) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2f(1.0, 0.0));
    let c = hash(i + vec2f(0.0, 1.0));
    let d = hash(i + vec2f(1.0, 1.0));
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}

struct VOut { @builtin(position) pos: vec4f, @location(0) uv: vec2f }

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    var corners = array<vec2f, 6>(vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0), vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0));
    let c = corners[vi];
    let q = c * u.t_size.zw;
    let px = vec2f(u.m.x * q.x + u.m.z * q.y, u.m.y * q.x + u.m.w * q.y) + u.t_size.xy;
    var o: VOut;
    o.pos = vec4f(px.x / u.res.x * 2.0 - 1.0, 1.0 - px.y / u.res.y * 2.0, 0.0, 1.0);
    o.uv = c;
    return o;
}

fn user_main(uv: vec2f, px: vec2f) -> vec4f {
    let res = u.res;
    let size = u.size;
    let time = u.time;
    let t = u.t;
    let frame = u.frame;
    let dur = u.dur;
//PARAMS
//CODE
}

@fragment
fn fs(in: VOut) -> @location(0) vec4f {
    let c = user_main(in.uv, in.pos.xy);
//OUTPUT
}
