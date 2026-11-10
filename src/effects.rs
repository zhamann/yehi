//! Built-in shader effects: plain WGSL bodies run through the same path as custom `code`.
//! Each returns a premultiplied color.

use serde_json::{Value, json};

pub const NAMES: &[&str] = &["vignette", "grain", "linear_gradient", "radial_gradient"];

pub fn code(name: &str) -> Option<&'static str> {
    Some(match name {
        "vignette" => {
            "let d = length((uv - 0.5) * 2.0);
             let k = smoothstep(inner, outer, d) * amount * color.a;
             return vec4f(color.rgb * k, k);"
        }
        // Signed noise with zero alpha adds to what's below (the compositor works in float).
        "grain" => {
            "let p = floor(px / max(size_px, 1.0));
             let n = (hash(p + vec2f(frame * 13.1, frame * 7.7)) - 0.5) * 2.0;
             return vec4f(vec3f(n * amount), 0.0);"
        }
        "linear_gradient" => {
            "let dir = to - from;
             let k = clamp(dot(uv - from, dir) / max(dot(dir, dir), 1e-6), 0.0, 1.0);
             let c = mix(color_a, color_b, k);
             return vec4f(c.rgb * c.a, c.a);"
        }
        "radial_gradient" => {
            "let d = length((uv - center) / max(radius, vec2f(1e-6)));
             let k = smoothstep(inner, 1.0, d);
             let c = mix(color_a, color_b, k);
             return vec4f(c.rgb * c.a, c.a);"
        }
        _ => return None,
    })
}

pub fn defaults(name: &str) -> Vec<(&'static str, Value)> {
    match name {
        "vignette" => {
            vec![("amount", json!(0.5)), ("color", json!("#000000")), ("inner", json!(0.4)), ("outer", json!(1.5))]
        }
        "grain" => vec![("amount", json!(0.04)), ("size_px", json!(1.0))],
        "linear_gradient" => vec![
            ("from", json!([0.5, 0.0])),
            ("to", json!([0.5, 1.0])),
            ("color_a", json!("#000000")),
            ("color_b", json!("#ffffff")),
        ],
        "radial_gradient" => vec![
            ("center", json!([0.5, 0.5])),
            ("radius", json!([0.5, 0.5])),
            ("inner", json!(0.0)),
            ("color_a", json!("#ffffff")),
            ("color_b", json!("#000000")),
        ],
        _ => vec![],
    }
}
