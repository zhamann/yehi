// The compositor: every image, glyph, rect, ellipse and isolated group is one instanced quad.
// Colors are premultiplied, in the composition's (sRGB-encoded) space, accumulated in a float target.

struct Globals { res: vec2f, _pad: vec2f }
@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var samp: sampler;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(2) @binding(0) var mask_tex: texture_2d<f32>;

struct Inst {
    @location(0) m: vec4f,       // linear part, local -> px: px = (m.x*x + m.z*y, m.y*x + m.w*y) + t
    @location(1) t_size: vec4f,  // translation (px) and quad size (local units)
    @location(2) uv: vec4f,      // texture window
    @location(3) color: vec4f,   // premultiplied multiplier
    @location(4) clip_m: vec4f,  // px -> clip-local, linear part
    @location(5) clip_t: vec4f,  // px -> clip-local translation, clip size (z < 0: no clip)
    @location(6) lbox: vec4f,    // quad's box in layer-local coordinates, for reveal and mask
    @location(7) reveal: vec4f,  // direction, front, softness (w < 0: no reveal)
    @location(8) mask_r: vec4f,  // mask rect in layer-local coordinates
    @location(9) params: vec4f,  // kind, corner radius, mask mode, anti-alias pad
}

struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) q: vec2f,
    @location(1) @interpolate(flat) size: vec2f,
    @location(2) @interpolate(flat) uv: vec4f,
    @location(3) @interpolate(flat) color: vec4f,
    @location(4) @interpolate(flat) clip_m: vec4f,
    @location(5) @interpolate(flat) clip_t: vec4f,
    @location(6) @interpolate(flat) lbox: vec4f,
    @location(7) @interpolate(flat) reveal: vec4f,
    @location(8) @interpolate(flat) mask_r: vec4f,
    @location(9) @interpolate(flat) params: vec4f,
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, inst: Inst) -> VOut {
    var corners = array<vec2f, 6>(vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0), vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0));
    let c = corners[vi];
    let size = inst.t_size.zw;
    let pad = inst.params.w;
    let q = -vec2f(pad) + c * (size + 2.0 * pad);
    let px = vec2f(inst.m.x * q.x + inst.m.z * q.y, inst.m.y * q.x + inst.m.w * q.y) + inst.t_size.xy;
    var o: VOut;
    o.pos = vec4f(px.x / g.res.x * 2.0 - 1.0, 1.0 - px.y / g.res.y * 2.0, 0.0, 1.0);
    o.q = q;
    o.size = size;
    o.uv = inst.uv;
    o.color = inst.color;
    o.clip_m = inst.clip_m;
    o.clip_t = inst.clip_t;
    o.lbox = inst.lbox;
    o.reveal = inst.reveal;
    o.mask_r = inst.mask_r;
    o.params = inst.params;
    return o;
}

fn luma(c: vec3f) -> f32 { return dot(c, vec3f(0.2126, 0.7152, 0.0722)); }

@fragment
fn fs(in: VOut) -> @location(0) vec4f {
    let size = in.size;
    let st = in.q / size;
    let uv = mix(in.uv.xy, in.uv.zw, clamp(st, vec2f(0.0), vec2f(1.0)));
    let lp = in.lbox.xy + st * in.lbox.zw;
    let muv = (lp - in.mask_r.xy) / in.mask_r.zw;
    let px = in.pos.xy;
    let cl = vec2f(in.clip_m.x * px.x + in.clip_m.z * px.y, in.clip_m.y * px.x + in.clip_m.w * px.y) + in.clip_t.xy;
    // derivatives in uniform control flow
    let duvx = dpdx(uv);
    let duvy = dpdy(uv);
    let dmx = dpdx(muv);
    let dmy = dpdy(muv);
    let fq = max(fwidth(in.q), vec2f(1e-6));
    let fcl = max(fwidth(cl), vec2f(1e-6));

    let kind = u32(in.params.x + 0.5);
    var c: vec4f;
    var cov = 1.0;
    if kind == 1u {
        c = in.color * textureSampleLevel(tex, samp, uv, 0.0).r;
    } else if kind == 2u {
        let half = size * 0.5;
        let r = min(in.params.y, min(half.x, half.y));
        let p = abs(in.q - half) - (half - vec2f(r));
        let d = length(max(p, vec2f(0.0))) + min(max(p.x, p.y), 0.0) - r;
        cov = clamp(0.5 - d / length(fq) * 1.4142, 0.0, 1.0);
        c = in.color;
    } else if kind == 3u {
        let half = size * 0.5;
        let p = (in.q - half) / half;
        let k = length(p);
        let grad = length(p / (half * max(k, 1e-6)));
        let d = (k - 1.0) / max(grad, 1e-6);
        cov = clamp(0.5 - d / length(fq) * 1.4142, 0.0, 1.0);
        c = in.color;
    } else {
        c = textureSampleGrad(tex, samp, uv, duvx, duvy) * in.color;
        let e = min(min(in.q.x, size.x - in.q.x) / fq.x, min(in.q.y, size.y - in.q.y) / fq.y);
        cov = clamp(e + 0.5, 0.0, 1.0);
    }

    if in.clip_t.z >= 0.0 {
        let e = min(min(cl.x, in.clip_t.z - cl.x) / fcl.x, min(cl.y, in.clip_t.w - cl.y) / fcl.y);
        cov *= clamp(e + 0.5, 0.0, 1.0);
    }
    if in.reveal.w > 0.0 {
        cov *= clamp((in.reveal.z - dot(lp, in.reveal.xy)) / in.reveal.w, 0.0, 1.0);
    }
    let mode = u32(in.params.z + 0.5);
    if mode != 0u {
        let m = textureSampleGrad(mask_tex, samp, muv, dmx, dmy);
        var k: f32;
        if mode == 1u { k = m.a; }
        else if mode == 2u { k = luma(m.rgb); }
        else if mode == 3u { k = 1.0 - m.a; }
        else { k = 1.0 - luma(m.rgb); }
        cov *= k;
    }
    return c * cov;
}
