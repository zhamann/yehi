// The finished frame, converted on the GPU so the CPU only copies bytes:
// yuv: planar 4:2:0 BT.709 limited range (what encoders want), each invocation one 8x2 block;
// rgba: 8-bit straight RGBA for stills.

struct Params { width: u32, height: u32, frame: u32, _pad: u32 }
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;
@group(0) @binding(2) var<uniform> p: Params;

fn hash(x: u32, y: u32, f: u32) -> f32 {
    var h = x * 374761393u + y * 668265263u + f * 2246822519u;
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffu) / 65535.0;
}

fn px(x: u32, y: u32) -> vec3f {
    let c = textureLoad(src, vec2u(x, y), 0);
    // flatten onto black; dither by a fraction of a code value so gradients don't band
    let d = (hash(x, y, p.frame) - 0.5) / 255.0;
    return clamp(c.rgb + vec3f(d), vec3f(0.0), vec3f(1.0));
}

fn y_of(c: vec3f) -> u32 {
    return u32(clamp(16.0 + 219.0 * dot(c, vec3f(0.2126, 0.7152, 0.0722)) + 0.5, 0.0, 255.0));
}

@compute @workgroup_size(8, 8)
fn yuv(@builtin(global_invocation_id) id: vec3u) {
    let bx = id.x * 8u;
    let by = id.y * 2u;
    if bx >= p.width || by >= p.height { return; }
    var cb: array<u32, 4>;
    var cr: array<u32, 4>;
    for (var row = 0u; row < 2u; row++) {
        var words = array<u32, 2>(0u, 0u);
        for (var k = 0u; k < 8u; k++) {
            let c = px(bx + k, by + row);
            words[k / 4u] |= y_of(c) << ((k % 4u) * 8u);
        }
        let base = ((by + row) * p.width + bx) / 4u;
        dst[base] = words[0];
        dst[base + 1u] = words[1];
    }
    var u_word = 0u;
    var v_word = 0u;
    for (var k = 0u; k < 4u; k++) {
        let x = bx + k * 2u;
        let c = (px(x, by) + px(x + 1u, by) + px(x, by + 1u) + px(x + 1u, by + 1u)) * 0.25;
        let yy = dot(c, vec3f(0.2126, 0.7152, 0.0722));
        let u = u32(clamp(128.0 + 224.0 * (c.b - yy) / 1.8556 + 0.5, 0.0, 255.0));
        let v = u32(clamp(128.0 + 224.0 * (c.r - yy) / 1.5748 + 0.5, 0.0, 255.0));
        u_word |= u << (k * 8u);
        v_word |= v << (k * 8u);
    }
    let ysize = p.width * p.height;
    let cw = p.width / 2u;
    let crow = by / 2u;
    let uoff = (ysize + crow * cw + bx / 2u) / 4u;
    let voff = (ysize + ysize / 4u + crow * cw + bx / 2u) / 4u;
    dst[uoff] = u_word;
    dst[voff] = v_word;
}

@compute @workgroup_size(8, 8)
fn rgba(@builtin(global_invocation_id) id: vec3u) {
    if id.x >= p.width || id.y >= p.height { return; }
    let c = textureLoad(src, id.xy, 0);
    let a = clamp(c.a, 0.0, 1.0);
    var rgb = c.rgb;
    if a > 0.0 { rgb = rgb / a; }
    let q = vec4u(vec4f(clamp(rgb, vec3f(0.0), vec3f(1.0)), a) * 255.0 + 0.5);
    dst[id.y * p.width + id.x] = q.r | (q.g << 8u) | (q.b << 16u) | (q.a << 24u);
}
