//! The JSON spec an agent writes, and the validated composition built from it.

use crate::anim::{AnimColor, AnimF, AnimV2, parse_hex};
use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpecJson {
    width: u32,
    height: u32,
    fps: f64,
    duration: f64,
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    audio: Vec<AudioJson>,
    #[serde(default)]
    loudness: Option<f64>,
    #[serde(default)]
    layers: Vec<LayerJson>,
    #[serde(default, rename = "$schema")]
    _schema: Option<String>,
    #[serde(default)]
    _comment: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioJson {
    src: String,
    #[serde(default)]
    start: f64,
    #[serde(default)]
    trim: f64,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default = "one")]
    volume: f64,
    #[serde(default)]
    fade_in: f64,
    #[serde(default)]
    fade_out: f64,
    #[serde(default, rename = "loop")]
    looped: bool,
}

fn one() -> f64 {
    1.0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerJson {
    #[serde(rename = "type")]
    kind: String,
    name: Option<String>,
    start: Option<f64>,
    duration: Option<f64>,
    end: Option<f64>,
    position: Option<AnimV2>,
    anchor: Option<AnimV2>,
    scale: Option<AnimV2>,
    rotation: Option<AnimF>,
    opacity: Option<AnimF>,
    blend: Option<String>,
    clip: Option<Value>,
    reveal: Option<RevealJson>,
    mask: Option<MaskJson>,
    size: Option<AnimV2>,
    // image / video
    src: Option<String>,
    fit: Option<String>,
    tint: Option<AnimColor>,
    trim: Option<f64>,
    #[serde(rename = "loop")]
    looped: Option<bool>,
    volume: Option<f64>,
    muted: Option<bool>,
    // text
    text: Option<String>,
    font: Option<String>,
    font_size: Option<f64>,
    font_weight: Option<u16>,
    font_style: Option<String>,
    color: Option<AnimColor>,
    tracking: Option<AnimF>,
    line_height: Option<f64>,
    align: Option<String>,
    max_width: Option<f64>,
    #[serde(rename = "box")]
    text_box: Option<String>,
    glyphs: Option<GlyphsJson>,
    oversample: Option<f64>,
    // shapes
    radius: Option<AnimF>,
    // shader
    code: Option<String>,
    effect: Option<String>,
    params: Option<BTreeMap<String, Value>>,
    // group
    layers: Option<Vec<LayerJson>>,
    isolate: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevealJson {
    progress: AnimF,
    #[serde(default)]
    angle: Option<f64>,
    #[serde(default)]
    softness: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MaskJson {
    src: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    rect: Option<[f64; 4]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GlyphsJson {
    offset: Option<AnimV2>,
    opacity: Option<AnimF>,
    scale: Option<AnimV2>,
    rotation: Option<AnimF>,
}

// ------------------------------------------------------------------ the validated model

pub struct Comp {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    pub frames: u32,
    pub background: [f64; 4],
    pub audio: Vec<Audio>,
    pub loudness: Option<f64>,
    pub layers: Vec<Layer>,
}

/// One audio source placed on the composition's timeline.
#[derive(Clone, Debug)]
pub struct Audio {
    pub src: PathBuf,
    pub start: f64,
    pub trim: f64,
    pub duration: Option<f64>,
    pub volume: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    pub looped: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Blend {
    Normal,
    Add,
    Multiply,
    Screen,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fit {
    Fill,
    Contain,
    Cover,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MaskMode {
    Alpha = 1,
    Luma = 2,
    InvertAlpha = 3,
    InvertLuma = 4,
}

pub struct Reveal {
    pub progress: AnimF,
    pub angle: f64,
    pub softness: f64,
}

pub struct Mask {
    pub src: PathBuf,
    pub mode: MaskMode,
    pub rect: Option<[f64; 4]>,
}

pub struct Glyphs {
    pub offset: Option<AnimV2>,
    pub opacity: Option<AnimF>,
    pub scale: Option<AnimV2>,
    pub rotation: Option<AnimF>,
}

pub struct Layer {
    pub id: usize,
    pub path: String,
    pub start: f64,
    pub duration: f64,
    pub position: Option<AnimV2>,
    pub anchor: Option<AnimV2>,
    pub scale: Option<AnimV2>,
    pub rotation: Option<AnimF>,
    pub opacity: Option<AnimF>,
    pub blend: Option<Blend>,
    pub clip: Option<[AnimF; 4]>,
    pub reveal: Option<Reveal>,
    pub mask: Option<Mask>,
    pub kind: Kind,
}

pub enum Kind {
    Image {
        src: PathBuf,
        size: Option<AnimV2>,
        fit: Fit,
        tint: Option<AnimColor>,
    },
    Video {
        src: PathBuf,
        size: Option<AnimV2>,
        fit: Fit,
        tint: Option<AnimColor>,
        trim: f64,
        looped: bool,
        volume: f64,
        muted: bool,
    },
    Text(Box<TextSpec>),
    Rect {
        size: AnimV2,
        color: AnimColor,
        radius: Option<AnimF>,
    },
    Ellipse {
        size: AnimV2,
        color: AnimColor,
    },
    Shader(Box<ShaderSpec>),
    Group {
        layers: Vec<Layer>,
        size: Option<AnimV2>,
        isolate: bool,
    },
}

pub struct TextSpec {
    pub text: String,
    pub font: FontRef,
    pub font_size: f64,
    pub color: AnimColor,
    pub tracking: Option<AnimF>,
    pub line_height: f64,
    pub align: Align,
    pub max_width: Option<f64>,
    pub ink_box: bool,
    pub glyphs: Option<Glyphs>,
    pub oversample: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FontRef {
    File(PathBuf),
    Family { name: String, weight: u16, italic: bool },
}

pub enum ParamValue {
    F(AnimF),
    V(Vec<AnimF>),
}

pub struct ShaderSpec {
    pub code: String,
    pub premultiplied: bool,
    pub params: Vec<(String, ParamValue)>,
    pub size: Option<AnimV2>,
}

// ------------------------------------------------------------------ loading

pub fn load(path: &Path) -> Result<Comp> {
    let text = std::fs::read_to_string(path).with_context(|| format!("can't read {}", path.display()))?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let dir = if dir.as_os_str().is_empty() { PathBuf::from(".") } else { dir };
    parse(&text, &dir)
}

pub fn parse(text: &str, dir: &Path) -> Result<Comp> {
    let de = &mut serde_json::Deserializer::from_str(text);
    let spec: SpecJson = serde_path_to_error::deserialize(&mut *de).map_err(|e| {
        let path = e.path().to_string();
        let inner = e.into_inner();
        if path == "." || path == "?" { anyhow!("{inner}") } else { anyhow!("{path}: {inner}") }
    })?;
    de.end().map_err(|e| anyhow!("{e}: the file must hold one JSON object and nothing after it"))?;
    if spec.width == 0 || spec.height == 0 || spec.width % 2 == 1 || spec.height % 2 == 1 {
        bail!(
            "width and height must be even and non-zero (video encoders need even sizes), got {}x{}",
            spec.width,
            spec.height
        );
    }
    if !(spec.fps > 0.0 && spec.fps <= 240.0) {
        bail!("fps must be more than 0 and at most 240, got {}", spec.fps);
    }
    if !(spec.duration > 0.0 && spec.duration.is_finite()) {
        bail!("duration must be positive, got {}", spec.duration);
    }
    if let Some(l) = spec.loudness
        && !(-70.0..=-5.0).contains(&l)
    {
        bail!("loudness must be between -70 and -5 LUFS (social platforms target about -14), got {l}");
    }
    let mut b = Builder { dir: dir.to_path_buf(), next_id: 0, errors: Vec::new() };
    let layers = b.layers(spec.layers, "layers", spec.duration);
    let mut audio = Vec::new();
    for (i, a) in spec.audio.into_iter().enumerate() {
        let src = b.asset(&a.src, &format!("audio[{i}].src"));
        for (field, v) in [("trim", a.trim), ("volume", a.volume), ("fade_in", a.fade_in), ("fade_out", a.fade_out)] {
            if !(v >= 0.0 && v.is_finite()) {
                b.errors.push(format!("audio[{i}].{field} must be 0 or more, got {v}"));
            }
        }
        if !a.start.is_finite() {
            b.errors.push(format!("audio[{i}].start must be a number of seconds, got {}", a.start));
        }
        if let Some(d) = a.duration
            && !(d > 0.0 && d.is_finite())
        {
            b.errors.push(format!("audio[{i}].duration must be positive, got {d}"));
        }
        audio.push(Audio {
            src,
            start: a.start,
            trim: a.trim,
            duration: a.duration,
            volume: a.volume,
            fade_in: a.fade_in,
            fade_out: a.fade_out,
            looped: a.looped,
        });
    }
    let background = match spec.background.as_deref() {
        None => [0.0, 0.0, 0.0, 1.0],
        Some(s) => parse_hex(s).map_err(|e| anyhow!("background: {e}"))?,
    };
    if !b.errors.is_empty() {
        bail!("{}", b.errors.join("\n"));
    }
    Ok(Comp {
        width: spec.width,
        height: spec.height,
        fps: spec.fps,
        duration: spec.duration,
        frames: (spec.duration * spec.fps).round().max(1.0) as u32,
        background,
        audio,
        loudness: spec.loudness,
        layers,
    })
}

struct Builder {
    dir: PathBuf,
    next_id: usize,
    errors: Vec<String>,
}

const COMMON: &[&str] = &[
    "type", "name", "start", "duration", "end", "position", "anchor", "scale", "rotation", "opacity", "blend", "clip",
    "reveal", "mask",
];

fn allowed(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "image" => &["src", "size", "fit", "tint"],
        "video" => &["src", "size", "fit", "tint", "trim", "loop", "volume", "muted"],
        "text" => &[
            "text",
            "font",
            "font_size",
            "font_weight",
            "font_style",
            "color",
            "tracking",
            "line_height",
            "align",
            "max_width",
            "box",
            "glyphs",
            "oversample",
        ],
        "rect" => &["size", "color", "radius"],
        "ellipse" => &["size", "color"],
        "shader" => &["size", "code", "effect", "params"],
        "group" => &["layers", "size", "isolate"],
        _ => return None,
    })
}

impl LayerJson {
    fn present(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        macro_rules! chk { ($($f:ident => $n:literal),*) => { $( if self.$f.is_some() { v.push($n); } )* } }
        chk!(src => "src", size => "size", fit => "fit", tint => "tint", trim => "trim", looped => "loop",
             volume => "volume", muted => "muted", text => "text", font => "font", font_size => "font_size",
             font_weight => "font_weight", font_style => "font_style", color => "color", tracking => "tracking",
             line_height => "line_height", align => "align", max_width => "max_width", text_box => "box",
             glyphs => "glyphs", oversample => "oversample", radius => "radius", code => "code", effect => "effect",
             params => "params", layers => "layers", isolate => "isolate");
        v
    }
}

impl Builder {
    fn asset(&mut self, src: &str, at: &str) -> PathBuf {
        let p = self.dir.join(src);
        if !p.is_file() {
            self.errors.push(format!("{at}: file not found: {} (paths are relative to the spec file)", p.display()));
        }
        p
    }

    fn layers(&mut self, list: Vec<LayerJson>, at: &str, parent_dur: f64) -> Vec<Layer> {
        let mut out = Vec::new();
        for (i, l) in list.into_iter().enumerate() {
            let here = format!("{at}[{i}]");
            match self.layer(l, &here, parent_dur) {
                Ok(Some(l)) => out.push(l),
                Ok(None) => {}
                Err(e) => self.errors.push(format!("{here}: {e}")),
            }
        }
        out
    }

    fn layer(&mut self, l: LayerJson, at: &str, parent_dur: f64) -> Result<Option<Layer>> {
        let kind = l.kind.as_str();
        let ok = allowed(kind)
            .ok_or_else(|| anyhow!("unknown type `{kind}`; use image, video, text, rect, ellipse, shader or group"))?;
        let a = if kind.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
        for f in l.present() {
            if !ok.contains(&f) && !COMMON.contains(&f) {
                bail!("`{f}` doesn't apply to {a} {kind} layer; {a} {kind} layer takes {}", ok.join(", "));
            }
        }
        let glyph_vars = [
            ("position", l.position.as_ref().is_some_and(AnimV2::reads_glyph_vars)),
            ("anchor", l.anchor.as_ref().is_some_and(AnimV2::reads_glyph_vars)),
            ("scale", l.scale.as_ref().is_some_and(AnimV2::reads_glyph_vars)),
            ("rotation", l.rotation.as_ref().is_some_and(AnimF::reads_glyph_vars)),
            ("opacity", l.opacity.as_ref().is_some_and(AnimF::reads_glyph_vars)),
            ("size", l.size.as_ref().is_some_and(AnimV2::reads_glyph_vars)),
            ("tint", l.tint.as_ref().is_some_and(AnimColor::reads_glyph_vars)),
            ("color", l.color.as_ref().is_some_and(AnimColor::reads_glyph_vars)),
            ("tracking", l.tracking.as_ref().is_some_and(AnimF::reads_glyph_vars)),
            ("radius", l.radius.as_ref().is_some_and(AnimF::reads_glyph_vars)),
            ("reveal.progress", l.reveal.as_ref().is_some_and(|r| r.progress.reads_glyph_vars())),
        ];
        for (f, reads) in glyph_vars {
            if reads {
                bail!("`{f}` reads `i` or `n`, which exist only in a text layer's `glyphs`");
            }
        }
        let path = match &l.name {
            Some(n) => format!("{at} ({n})"),
            None => at.to_string(),
        };
        let start = l.start.unwrap_or(0.0);
        let duration = match (l.duration, l.end) {
            (Some(_), Some(_)) => bail!("give `duration` or `end`, not both"),
            (Some(d), None) => d,
            (None, Some(e)) => e - start,
            (None, None) => parent_dur - start,
        };
        let blend = match l.blend.as_deref() {
            None => None,
            Some("normal") => Some(Blend::Normal),
            Some("add") => Some(Blend::Add),
            Some("multiply") => Some(Blend::Multiply),
            Some("screen") => Some(Blend::Screen),
            Some(b) => bail!("unknown blend `{b}`; use normal, add, multiply or screen"),
        };
        let clip = match &l.clip {
            None => None,
            Some(Value::Array(a)) if a.len() == 4 => {
                let c = [
                    AnimF::from_value(&a[0]).context("clip x")?,
                    AnimF::from_value(&a[1]).context("clip y")?,
                    AnimF::from_value(&a[2]).context("clip width")?,
                    AnimF::from_value(&a[3]).context("clip height")?,
                ];
                if c.iter().any(AnimF::reads_glyph_vars) {
                    bail!("`clip` reads `i` or `n`, which exist only in a text layer's `glyphs`");
                }
                Some(c)
            }
            Some(v) => bail!("clip must be [x, y, width, height] in the parent's coordinates, got {v}"),
        };
        let reveal = l.reveal.map(|r| Reveal {
            progress: r.progress,
            angle: r.angle.unwrap_or(0.0),
            softness: r.softness.unwrap_or(0.0),
        });
        let mask = match l.mask {
            None => None,
            Some(m) => {
                let mode = match m.mode.as_deref().unwrap_or("alpha") {
                    "alpha" => MaskMode::Alpha,
                    "luma" => MaskMode::Luma,
                    "invert_alpha" => MaskMode::InvertAlpha,
                    "invert_luma" => MaskMode::InvertLuma,
                    o => bail!("unknown mask mode `{o}`; use alpha, luma, invert_alpha or invert_luma"),
                };
                Some(Mask { src: self.asset(&m.src, &format!("{at}.mask.src")), mode, rect: m.rect })
            }
        };
        let fit = |f: Option<&str>| -> Result<Fit> {
            Ok(match f.unwrap_or("fill") {
                "fill" => Fit::Fill,
                "contain" => Fit::Contain,
                "cover" => Fit::Cover,
                o => bail!("unknown fit `{o}`; use fill, contain or cover"),
            })
        };
        let need =
            |v: Option<&str>, what: &str| v.map(str::to_string).ok_or_else(|| anyhow!("a {kind} layer needs `{what}`"));
        let k = match kind {
            "image" => {
                let src = need(l.src.as_deref(), "src")?;
                Kind::Image {
                    src: self.asset(&src, &format!("{at}.src")),
                    size: l.size,
                    fit: fit(l.fit.as_deref())?,
                    tint: l.tint,
                }
            }
            "video" => {
                let src = need(l.src.as_deref(), "src")?;
                Kind::Video {
                    src: self.asset(&src, &format!("{at}.src")),
                    size: l.size,
                    fit: fit(l.fit.as_deref())?,
                    tint: l.tint,
                    trim: l.trim.unwrap_or(0.0),
                    looped: l.looped.unwrap_or(false),
                    volume: l.volume.unwrap_or(1.0),
                    muted: l.muted.unwrap_or(false),
                }
            }
            "text" => {
                let text = need(l.text.as_deref(), "text")?;
                let font = match l.font {
                    None => {
                        bail!("a text layer needs `font`: a font file path (.ttf/.otf) or an installed family name")
                    }
                    Some(f) => {
                        let lower = f.to_ascii_lowercase();
                        if [".ttf", ".otf", ".ttc", ".otc"].iter().any(|e| lower.ends_with(e)) {
                            if l.font_weight.is_some() || l.font_style.is_some() {
                                bail!(
                                    "`font_weight` and `font_style` apply only when `font` is a family name; a font file is one weight and style, so point `font` at the file you want"
                                );
                            }
                            FontRef::File(self.asset(&f, &format!("{at}.font")))
                        } else {
                            let italic = match l.font_style.as_deref() {
                                None | Some("normal") => false,
                                Some("italic") => true,
                                Some(o) => bail!("font_style must be normal or italic, got `{o}`"),
                            };
                            let weight = l.font_weight.unwrap_or(400);
                            if !(100..=900).contains(&weight) {
                                bail!("font_weight must be from 100 to 900, got {weight}");
                            }
                            FontRef::Family { name: f, weight, italic }
                        }
                    }
                };
                let align = match l.align.as_deref().unwrap_or("center") {
                    "left" => Align::Left,
                    "center" => Align::Center,
                    "right" => Align::Right,
                    o => bail!("align must be left, center or right, got `{o}`"),
                };
                let ink_box = match l.text_box.as_deref().unwrap_or("layout") {
                    "layout" => false,
                    "ink" => true,
                    o => bail!("box must be layout or ink, got `{o}`"),
                };
                Kind::Text(Box::new(TextSpec {
                    text,
                    font,
                    font_size: l.font_size.unwrap_or(48.0),
                    color: l.color.unwrap_or_else(|| AnimColor::constant_rgba([1.0; 4])),
                    tracking: l.tracking,
                    line_height: l.line_height.unwrap_or(1.2),
                    align,
                    max_width: l.max_width,
                    ink_box,
                    glyphs: l.glyphs.map(|g| Glyphs {
                        offset: g.offset,
                        opacity: g.opacity,
                        scale: g.scale,
                        rotation: g.rotation,
                    }),
                    oversample: match l.oversample.unwrap_or(1.0) {
                        o if (0.25..=8.0).contains(&o) => o,
                        o => bail!("oversample must be from 0.25 to 8, got {o}"),
                    },
                }))
            }
            "rect" | "ellipse" => {
                let size = l.size.ok_or_else(|| anyhow!("a {kind} layer needs `size`: [width, height]"))?;
                let color = l.color.unwrap_or_else(|| AnimColor::constant_rgba([1.0; 4]));
                if kind == "rect" {
                    Kind::Rect { size, color, radius: l.radius }
                } else {
                    Kind::Ellipse { size, color }
                }
            }
            "shader" => {
                if clip.is_some() || reveal.is_some() || mask.is_some() {
                    bail!(
                        "clip, reveal and mask don't apply to a shader layer; put it in a group with `isolate: true` and set them there"
                    );
                }
                let (code, premultiplied) = match (l.code, l.effect.as_deref()) {
                    (Some(_), Some(_)) => bail!("give `code` or `effect`, not both"),
                    (Some(c), None) => (c, false),
                    (None, Some(e)) => (
                        crate::effects::code(e)
                            .ok_or_else(|| {
                                anyhow!("unknown effect `{e}`; built in: {}", crate::effects::NAMES.join(", "))
                            })?
                            .to_string(),
                        true,
                    ),
                    (None, None) => bail!("a shader layer needs `code` (WGSL) or `effect` (a built-in name)"),
                };
                let mut params = Vec::new();
                let defaults = l.effect.as_deref().map(crate::effects::defaults).unwrap_or_default();
                let mut given = l.params.unwrap_or_default();
                if let Some(e) = l.effect.as_deref() {
                    let names: Vec<&str> = defaults.iter().map(|(n, _)| *n).collect();
                    if let Some(bad) = given.keys().find(|k| !names.contains(&k.as_str())) {
                        bail!("`params.{bad}` isn't a param of the {e} effect; it takes {}", names.join(", "));
                    }
                }
                for (name, v) in defaults {
                    given.entry(name.to_string()).or_insert(v);
                }
                if given.len() > 16 {
                    bail!("a shader takes at most 16 params");
                }
                for (name, v) in given {
                    check_param_name(&name)?;
                    let pv = match &v {
                        Value::String(s) if s.starts_with('#') => ParamValue::V(
                            parse_hex(s).with_context(|| format!("param {name}"))?.map(AnimF::Const).to_vec(),
                        ),
                        Value::Array(a) if (2..=4).contains(&a.len()) && !a.iter().all(|k| k.get("t").is_some()) => {
                            ParamValue::V(
                                a.iter()
                                    .map(AnimF::from_value)
                                    .collect::<Result<_>>()
                                    .with_context(|| format!("param {name}"))?,
                            )
                        }
                        _ => ParamValue::F(AnimF::from_value(&v).with_context(|| format!("param {name}"))?),
                    };
                    let reads = match &pv {
                        ParamValue::F(a) => a.reads_glyph_vars(),
                        ParamValue::V(c) => c.iter().any(AnimF::reads_glyph_vars),
                    };
                    if reads {
                        bail!("`params.{name}` reads `i` or `n`, which exist only in a text layer's `glyphs`");
                    }
                    params.push((name, pv));
                }
                Kind::Shader(Box::new(ShaderSpec { code, premultiplied, params, size: l.size }))
            }
            "group" => {
                let children = l.layers.unwrap_or_default();
                let layers = self.layers(children, &format!("{at}.layers"), duration);
                let isolate = l.isolate.unwrap_or(false);
                if !isolate && (reveal.is_some() || mask.is_some()) {
                    bail!(
                        "reveal and mask on a group need `isolate: true` (the group is drawn to its own layer first)"
                    );
                }
                Kind::Group { layers, size: l.size, isolate }
            }
            _ => unreachable!(),
        };
        if duration <= 0.0 {
            return Ok(None); // starts after its parent ends: never visible, though still checked above
        }
        let id = self.next_id;
        self.next_id += 1;
        Ok(Some(Layer {
            id,
            path,
            start,
            duration,
            position: l.position,
            anchor: l.anchor,
            scale: l.scale,
            rotation: l.rotation,
            opacity: l.opacity,
            blend,
            clip,
            reveal,
            mask,
            kind: k,
        }))
    }
}

/// Names a shader's code already uses, which a param can't take.
const SHADER_NAMES: &[&str] = &[
    "uv",
    "px",
    "res",
    "size",
    "time",
    "t",
    "frame",
    "dur",
    "u",
    "backdrop",
    "hash",
    "noise",
    "backdrop_tex",
    "backdrop_samp",
    "user_main",
    "vs",
    "fs",
    "VOut",
    "U",
];

fn check_param_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || name.starts_with(|c: char| c.is_ascii_digit())
        || name.starts_with("__")
        || name == "_"
    {
        bail!("param name `{name}` must be a WGSL identifier: letters, digits and _, not starting with a digit or __");
    }
    if wgpu::naga::keywords::wgsl::RESERVED.contains(&name) {
        bail!("param name `{name}` is a reserved word in WGSL; pick another name");
    }
    if SHADER_NAMES.contains(&name) {
        bail!("param name `{name}` is already defined in shader code (see `yehi docs`); pick another name");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(s: &str) -> String {
        match parse(s, Path::new(".")) {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn minimal_spec() {
        let c = parse(
            r##"{"width": 1080, "height": 1920, "fps": 30, "duration": 2, "background": "#112233",
                "layers": [{"type": "rect", "size": [100, 100], "color": "#fff"},
                           {"type": "group", "start": 1, "layers": [{"type": "ellipse", "size": [10, 10]}]}]}"##,
            Path::new("."),
        )
        .unwrap();
        assert_eq!(c.frames, 60);
        assert_eq!(c.layers.len(), 2);
        assert_eq!(c.layers[1].duration, 1.0);
        match &c.layers[1].kind {
            Kind::Group { layers, .. } => assert_eq!(layers[0].duration, 1.0),
            _ => panic!(),
        }
    }

    #[test]
    fn typos_are_caught() {
        assert!(
            err(r#"{"width": 100, "height": 100, "fps": 30, "duration": 1, "layrs": []}"#)
                .contains("unknown field `layrs`")
        );
        let e = err(
            r#"{"width": 100, "height": 100, "fps": 30, "duration": 1, "layers": [{"type": "rect", "size": [1, 1], "text": "hi"}]}"#,
        );
        assert!(e.contains("layers[0]") && e.contains("`text` doesn't apply to a rect"), "{e}");
        let e = err(
            r#"{"width": 100, "height": 100, "fps": 30, "duration": 1, "layers": [{"type": "rect", "size": [1, 1], "opacity": "t +"}]}"#,
        );
        assert!(e.contains("layers[0].opacity"), "{e}");
        let e = err(
            r#"{"width": 100, "height": 100, "fps": 30, "duration": 1, "layers": [{"type": "image", "src": "nope.png"}]}"#,
        );
        assert!(e.contains("file not found"), "{e}");
    }

    fn layer_err(layer: &str) -> String {
        err(&format!(r#"{{"width": 100, "height": 100, "fps": 30, "duration": 1, "layers": [{layer}]}}"#))
    }

    #[test]
    fn shader_params_are_checked() {
        let e = layer_err(r#"{"type": "shader", "effect": "grain", "params": {"amout": 1}}"#);
        assert!(e.contains("`params.amout` isn't a param of the grain effect"), "{e}");
        let e = layer_err(r#"{"type": "shader", "code": "vec4f(1.0)", "params": {"from": 1}}"#);
        assert!(e.contains("reserved word"), "{e}");
        let e = layer_err(r#"{"type": "shader", "code": "vec4f(1.0)", "params": {"uv": 1}}"#);
        assert!(e.contains("already defined"), "{e}");
    }

    #[test]
    fn glyph_vars_only_in_glyphs() {
        let e = layer_err(r#"{"type": "rect", "size": [10, 10], "opacity": "i / n"}"#);
        assert!(e.contains("`opacity` reads `i` or `n`"), "{e}");
    }

    #[test]
    fn hidden_layers_are_still_checked() {
        let e = layer_err(r#"{"type": "group", "start": 5, "layers": [{"type": "sprite"}]}"#);
        assert!(e.contains("unknown type `sprite`"), "{e}");
    }

    #[test]
    fn one_json_object_only() {
        let e = err(r#"{"width": 100, "height": 100, "fps": 30, "duration": 1} THIS IS NOT JSON"#);
        assert!(e.contains("nothing after it"), "{e}");
    }

    #[test]
    fn ranges_are_checked() {
        assert!(err(r#"{"width": 100, "height": 100, "fps": 30, "duration": 1, "loudness": 5}"#).contains("loudness"));
        assert!(err(r#"{"width": 100, "height": 100, "fps": 0, "duration": 1}"#).contains("more than 0"));
        let e = layer_err(r#"{"type": "text", "text": "hi", "font": "f.ttf", "font_weight": 700}"#);
        assert!(e.contains("only when `font` is a family name"), "{e}");
        let e = layer_err(r#"{"type": "ellipse", "text": "hi"}"#);
        assert!(e.contains("doesn't apply to an ellipse layer"), "{e}");
    }

    #[test]
    fn odd_sizes_are_rejected() {
        assert!(err(r#"{"width": 101, "height": 100, "fps": 30, "duration": 1}"#).contains("even"));
    }
}
