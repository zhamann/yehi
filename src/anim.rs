//! Animatable values: a constant, an expression string, or a list of keyframes.

use crate::expr::{self, Expr, Vars};
use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

#[derive(Clone, Debug)]
pub enum Ease {
    Named(String),
    Bezier([f64; 4]),
    Hold,
}

impl Ease {
    fn apply(&self, x: f64) -> f64 {
        match self {
            Ease::Named(n) => expr::ease(n, x).unwrap_or(x),
            Ease::Bezier(b) => cubic_bezier(*b, x),
            Ease::Hold => 0.0,
        }
    }
}

/// CSS-style cubic-bezier(x1, y1, x2, y2).
fn cubic_bezier([x1, y1, x2, y2]: [f64; 4], x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    let bez = |a: f64, b: f64, s: f64| {
        let i = 1.0 - s;
        3.0 * i * i * s * a + 3.0 * i * s * s * b + s * s * s
    };
    let (mut lo, mut hi) = (0.0, 1.0);
    let mut s = x;
    for _ in 0..40 {
        let v = bez(x1, x2, s);
        if (v - x).abs() < 1e-7 {
            break;
        }
        if v < x {
            lo = s;
        } else {
            hi = s;
        }
        s = (lo + hi) / 2.0;
    }
    bez(y1, y2, s)
}

#[derive(Clone, Debug)]
pub struct Key {
    pub t: f64,
    pub v: f64,
    pub ease: Ease,
}

#[derive(Clone, Debug)]
pub enum AnimF {
    Const(f64),
    Expr(Expr),
    Keys(Vec<Key>),
}

impl AnimF {
    pub fn eval(&self, v: &Vars) -> f64 {
        match self {
            AnimF::Const(x) => *x,
            AnimF::Expr(e) => e.eval(v),
            AnimF::Keys(k) => eval_keys(k, v.t),
        }
    }
    pub fn constant(&self) -> Option<f64> {
        match self {
            AnimF::Const(x) => Some(*x),
            _ => None,
        }
    }
}

fn eval_keys(k: &[Key], t: f64) -> f64 {
    if t <= k[0].t {
        return k[0].v;
    }
    for w in k.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if t < b.t {
            let span = b.t - a.t;
            let x = if span > 0.0 { (t - a.t) / span } else { 1.0 };
            return a.v + (b.v - a.v) * b.ease.apply(x);
        }
    }
    k[k.len() - 1].v
}

fn parse_ease(v: Option<&Value>) -> Result<Ease> {
    match v {
        None | Some(Value::Null) => Ok(Ease::Named("linear".into())),
        Some(Value::String(s)) if s == "hold" => Ok(Ease::Hold),
        Some(Value::String(s)) if expr::is_ease_name(s) => Ok(Ease::Named(s.clone())),
        Some(Value::String(s)) => bail!(
            "unknown ease `{s}`; use hold, linear, inSine/outSine/inOutSine, inQuad.., inCubic.., inQuart.., \
             inExpo.., inBack.., outElastic, outBounce, or [x1, y1, x2, y2]"
        ),
        Some(Value::Array(a)) if a.len() == 4 && a.iter().all(Value::is_number) => {
            let b: Vec<f64> = a.iter().map(|x| x.as_f64().unwrap()).collect();
            Ok(Ease::Bezier([b[0], b[1], b[2], b[3]]))
        }
        Some(other) => bail!("ease must be a name or [x1, y1, x2, y2], got {other}"),
    }
}

fn is_keyframes(v: &Value) -> bool {
    matches!(v, Value::Array(a) if !a.is_empty() && a.iter().all(|k| k.get("t").is_some()))
}

/// Parse keyframes whose values are split into `width` components by `split`.
fn parse_keys(v: &Value, width: usize, split: &dyn Fn(&Value) -> Result<Vec<f64>>) -> Result<Vec<Vec<Key>>> {
    let arr = v.as_array().unwrap();
    let mut out = vec![Vec::new(); width];
    let mut last = f64::NEG_INFINITY;
    for (i, k) in arr.iter().enumerate() {
        let obj = k.as_object().ok_or_else(|| anyhow!("keyframe {i} must be an object like {{\"t\": 0, \"v\": 1}}"))?;
        for key in obj.keys() {
            if !matches!(key.as_str(), "t" | "v" | "ease") {
                bail!("keyframe {i} has unknown field `{key}`; keyframes take t, v and ease");
            }
        }
        let t = obj["t"].as_f64().ok_or_else(|| anyhow!("keyframe {i}: `t` must be a number of seconds"))?;
        if t < last {
            bail!("keyframe {i}: times must be in order ({t} comes after {last})");
        }
        last = t;
        let val = obj.get("v").ok_or_else(|| anyhow!("keyframe {i} needs a value `v`"))?;
        let comps = split(val).map_err(|e| anyhow!("keyframe {i}: {e}"))?;
        let ease = parse_ease(obj.get("ease")).map_err(|e| anyhow!("keyframe {i}: {e}"))?;
        for (c, x) in comps.into_iter().enumerate() {
            out[c].push(Key { t, v: x, ease: ease.clone() });
        }
    }
    Ok(out)
}

impl AnimF {
    pub fn from_value(v: &Value) -> Result<AnimF> {
        match v {
            Value::Number(n) => Ok(AnimF::Const(n.as_f64().unwrap())),
            Value::String(s) => {
                let e = Expr::parse(s)?;
                Ok(match e.constant() {
                    Some(c) => AnimF::Const(c),
                    None => AnimF::Expr(e),
                })
            }
            v if is_keyframes(v) => {
                let mut k = parse_keys(v, 1, &|x| {
                    x.as_f64().map(|f| vec![f]).ok_or_else(|| anyhow!("`v` must be a number, got {x}"))
                })?;
                Ok(AnimF::Keys(k.remove(0)))
            }
            other => {
                bail!("expected a number, an expression string, or keyframes [{{\"t\":..,\"v\":..}}], got {other}")
            }
        }
    }
}

impl<'de> Deserialize<'de> for AnimF {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        AnimF::from_value(&v).map_err(serde::de::Error::custom)
    }
}

/// A 2D value: [x, y] where each part animates on its own, or keyframes of [x, y] pairs.
/// A single number sets both (used for scale).
#[derive(Clone, Debug)]
pub struct AnimV2(pub [AnimF; 2]);

impl AnimV2 {
    pub fn eval(&self, v: &Vars) -> [f64; 2] {
        [self.0[0].eval(v), self.0[1].eval(v)]
    }
    pub fn constant(&self) -> Option<[f64; 2]> {
        Some([self.0[0].constant()?, self.0[1].constant()?])
    }
}

impl<'de> Deserialize<'de> for AnimV2 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let parse = || -> Result<AnimV2> {
            if is_keyframes(&v) {
                let mut k = parse_keys(&v, 2, &|x| match x {
                    Value::Array(a) if a.len() == 2 && a.iter().all(Value::is_number) => {
                        Ok(a.iter().map(|n| n.as_f64().unwrap()).collect())
                    }
                    Value::Number(n) => Ok(vec![n.as_f64().unwrap(); 2]),
                    _ => bail!("`v` must be [x, y], got {x}"),
                })?;
                let y = k.pop().unwrap();
                let x = k.pop().unwrap();
                return Ok(AnimV2([AnimF::Keys(x), AnimF::Keys(y)]));
            }
            match &v {
                Value::Array(a) if a.len() == 2 => Ok(AnimV2([AnimF::from_value(&a[0])?, AnimF::from_value(&a[1])?])),
                Value::Number(_) | Value::String(_) => {
                    let f = AnimF::from_value(&v)?;
                    Ok(AnimV2([f.clone(), f]))
                }
                _ => bail!("expected [x, y], got {v}"),
            }
        };
        parse().map_err(serde::de::Error::custom)
    }
}

/// A color: "#rgb", "#rrggbb", "#rrggbbaa", [r, g, b, a?] with 0..1 parts that can animate,
/// or keyframes of colors.
#[derive(Clone, Debug)]
pub struct AnimColor(pub [AnimF; 4]);

pub fn parse_hex(s: &str) -> Result<[f64; 4]> {
    let h = s.strip_prefix('#').ok_or_else(|| anyhow!("color `{s}` must start with #"))?;
    let digits: Vec<u8> = h
        .chars()
        .map(|c| c.to_digit(16).map(|d| d as u8))
        .collect::<Option<_>>()
        .ok_or_else(|| anyhow!("color `{s}` has a non-hex digit"))?;
    let px = |a: u8, b: u8| (a * 16 + b) as f64 / 255.0;
    Ok(match digits.len() {
        3 => [px(digits[0], digits[0]), px(digits[1], digits[1]), px(digits[2], digits[2]), 1.0],
        4 => [px(digits[0], digits[0]), px(digits[1], digits[1]), px(digits[2], digits[2]), px(digits[3], digits[3])],
        6 => [px(digits[0], digits[1]), px(digits[2], digits[3]), px(digits[4], digits[5]), 1.0],
        8 => [px(digits[0], digits[1]), px(digits[2], digits[3]), px(digits[4], digits[5]), px(digits[6], digits[7])],
        _ => bail!("color `{s}` must be #rgb, #rgba, #rrggbb or #rrggbbaa"),
    })
}

impl AnimColor {
    pub fn constant_rgba(c: [f64; 4]) -> AnimColor {
        AnimColor(c.map(AnimF::Const))
    }
    pub fn eval(&self, v: &Vars) -> [f64; 4] {
        [self.0[0].eval(v), self.0[1].eval(v), self.0[2].eval(v), self.0[3].eval(v)]
    }
    pub fn from_value(v: &Value) -> Result<AnimColor> {
        if is_keyframes(v) {
            let k = parse_keys(v, 4, &|x| match x {
                Value::String(s) => Ok(parse_hex(s)?.to_vec()),
                _ => bail!("`v` must be a color like \"#ff8800\", got {x}"),
            })?;
            let mut it = k.into_iter().map(AnimF::Keys);
            return Ok(AnimColor([it.next().unwrap(), it.next().unwrap(), it.next().unwrap(), it.next().unwrap()]));
        }
        match v {
            Value::String(s) => Ok(AnimColor::constant_rgba(parse_hex(s)?)),
            Value::Array(a) if a.len() == 3 || a.len() == 4 => {
                let mut parts: Vec<AnimF> = a.iter().map(AnimF::from_value).collect::<Result<_>>()?;
                if parts.len() == 3 {
                    parts.push(AnimF::Const(1.0));
                }
                let mut it = parts.into_iter();
                Ok(AnimColor([it.next().unwrap(), it.next().unwrap(), it.next().unwrap(), it.next().unwrap()]))
            }
            _ => bail!("expected a color like \"#ff8800\" or [r, g, b, a] in 0..1, got {v}"),
        }
    }
}

impl<'de> Deserialize<'de> for AnimColor {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        AnimColor::from_value(&v).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(a: &AnimF, t: f64) -> f64 {
        a.eval(&Vars { t, ..Default::default() })
    }

    #[test]
    fn keyframes_interpolate_and_hold() {
        let a =
            AnimF::from_value(&json!([{"t": 1, "v": 0}, {"t": 2, "v": 10}, {"t": 3, "v": 0, "ease": "hold"}])).unwrap();
        assert_eq!(at(&a, 0.0), 0.0);
        assert_eq!(at(&a, 1.5), 5.0);
        assert_eq!(at(&a, 2.5), 10.0);
        assert_eq!(at(&a, 3.5), 0.0);
    }

    #[test]
    fn eased_and_bezier_keys() {
        let a = AnimF::from_value(&json!([{"t": 0, "v": 0}, {"t": 1, "v": 1, "ease": "outExpo"}])).unwrap();
        assert!(at(&a, 0.5) > 0.9);
        let b = AnimF::from_value(&json!([{"t": 0, "v": 0}, {"t": 1, "v": 1, "ease": [0.42, 0, 0.58, 1]}])).unwrap();
        assert!((at(&b, 0.5) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn vectors_and_colors() {
        let v: AnimV2 = serde_json::from_value(json!([{"t": 0, "v": [0, 10]}, {"t": 1, "v": [10, 20]}])).unwrap();
        assert_eq!(v.eval(&Vars { t: 0.5, ..Default::default() }), [5.0, 15.0]);
        let mixed: AnimV2 = serde_json::from_value(json!([540, "960 + t * 10"])).unwrap();
        assert_eq!(mixed.eval(&Vars { t: 1.0, ..Default::default() }), [540.0, 970.0]);
        let c: AnimColor = serde_json::from_value(json!("#ff000080")).unwrap();
        let rgba = c.eval(&Vars::default());
        assert_eq!(rgba[0], 1.0);
        assert!((rgba[3] - 128.0 / 255.0).abs() < 1e-9);
    }

    #[test]
    fn bad_values_explain_themselves() {
        let e = AnimF::from_value(&json!([{"t": 1, "v": 0}, {"t": 0, "v": 1}])).unwrap_err();
        assert!(e.to_string().contains("in order"));
        let e = AnimF::from_value(&json!([{"t": 0, "v": 0, "ease": "bouncy"}])).unwrap_err();
        assert!(e.to_string().contains("unknown ease"));
    }
}
