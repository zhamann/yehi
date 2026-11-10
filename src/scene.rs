//! Loading a composition's assets onto the GPU, and turning each frame into instances and ops.

use crate::anim::AnimF;
use crate::expr::Vars;
use crate::gpu::{Gpu, Inst, Op, READS_BACKDROP, SHADER_UNIFORM_SIZE, WHITE};
use crate::spec::{Audio, Blend, Comp, Fit, Kind, Layer, ParamValue};
use crate::text::{self, Atlas, Fonts, TextLayout};
use crate::video::{self, Decoder};
use anyhow::{Context, Result, anyhow, bail};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug)]
pub struct Aff {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

impl Aff {
    const ID: Aff = Aff { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };
    fn translate(x: f64, y: f64) -> Aff {
        Aff { tx: x, ty: y, ..Aff::ID }
    }
    fn scale(x: f64, y: f64) -> Aff {
        Aff { a: x, d: y, ..Aff::ID }
    }
    fn rotate(deg: f64) -> Aff {
        let (s, c) = deg.to_radians().sin_cos();
        Aff { a: c, b: s, c: -s, d: c, tx: 0.0, ty: 0.0 }
    }
    /// self ∘ o: apply o, then self.
    fn then(self, o: Aff) -> Aff {
        Aff {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            tx: self.a * o.tx + self.c * o.ty + self.tx,
            ty: self.b * o.tx + self.d * o.ty + self.ty,
        }
    }
    fn det(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }
    fn inverse(&self) -> Option<Aff> {
        let det = self.det();
        if det.abs() < 1e-12 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Aff { a, b, c, d, tx: -(a * self.tx + c * self.ty), ty: -(b * self.tx + d * self.ty) })
    }
}

enum Res {
    Image { tex: usize, w: f64, h: f64 },
    Video { tex: usize, w: f64, h: f64, dec: usize },
    Text { layout: TextLayout, fit: f64 },
    Shader { prog: usize },
    Isolated { target: usize, w: f64, h: f64 },
    None,
}

#[derive(Clone, Copy)]
struct Clip {
    inv: Aff,
    rect: [f64; 4],
}

#[derive(Clone, Copy)]
struct Ctx {
    aff: Aff,
    box_size: [f64; 2],
    opacity: f64,
    blend: Blend,
    clip: Option<Clip>,
    res: [f64; 2],
}

pub struct Frame {
    pub inst: Vec<Inst>,
    pub uniforms: Vec<u8>,
    pub ops: Vec<Op>,
}

pub struct Scene<'a> {
    pub comp: &'a Comp,
    pub gpu: Gpu,
    res: HashMap<usize, Res>,
    masks: HashMap<PathBuf, usize>,
    atlas: Atlas,
    pages: Vec<usize>,
    decoders: Vec<Decoder>,
    pub main: usize,
    /// Audio from video layers, placed on the composition's timeline.
    pub video_audio: Vec<Audio>,
    uni_stride: usize,
}

/// An image's size and premultiplied mip levels.
type Decoded = (u32, u32, Vec<Vec<u8>>);

fn premultiply(mut px: Vec<u8>) -> Vec<u8> {
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a < 255 {
            p[0] = ((p[0] as u32 * a + 127) / 255) as u8;
            p[1] = ((p[1] as u32 * a + 127) / 255) as u8;
            p[2] = ((p[2] as u32 * a + 127) / 255) as u8;
        }
    }
    px
}

fn mips(w: u32, h: u32, base: Vec<u8>) -> Vec<Vec<u8>> {
    let mut out = vec![base];
    let (mut w, mut h) = (w, h);
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let src = out.last().unwrap();
        let mut dst = vec![0u8; (nw * nh * 4) as usize];
        dst.par_chunks_mut((nw * 4) as usize).enumerate().for_each(|(y, row)| {
            let y0 = (y as u32 * 2).min(h - 1);
            let y1 = (y0 + 1).min(h - 1);
            for x in 0..nw as usize {
                let x0 = (x as u32 * 2).min(w - 1);
                let x1 = (x0 + 1).min(w - 1);
                for c in 0..4 {
                    let s = |xx: u32, yy: u32| src[((yy * w + xx) * 4) as usize + c] as u32;
                    row[x * 4 + c] = ((s(x0, y0) + s(x1, y0) + s(x0, y1) + s(x1, y1) + 2) / 4) as u8;
                }
            }
        });
        out.push(dst);
        w = nw;
        h = nh;
    }
    out
}

fn reads_backdrop(code: &str) -> bool {
    code.contains("backdrop(")
}

/// Whether a target's own layers (through non-isolated groups) include a shader that reads the backdrop.
fn target_needs_backdrop(layers: &[Layer]) -> bool {
    layers.iter().any(|l| match &l.kind {
        Kind::Shader(s) => reads_backdrop(&s.code),
        Kind::Group { layers, isolate: false, .. } => target_needs_backdrop(layers),
        _ => false,
    })
}

fn count_shaders(layers: &[Layer]) -> usize {
    layers
        .iter()
        .map(|l| match &l.kind {
            Kind::Shader(_) => 1,
            Kind::Group { layers, .. } => count_shaders(layers),
            _ => 0,
        })
        .sum()
}

fn shader_source(s: &crate::spec::ShaderSpec) -> String {
    let mut params = String::new();
    for (k, (name, v)) in s.params.iter().enumerate() {
        let sw = match v {
            ParamValue::F(_) => ".x",
            ParamValue::V(c) if c.len() == 2 => ".xy",
            ParamValue::V(c) if c.len() == 3 => ".xyz",
            ParamValue::V(_) => "",
        };
        params += &format!("    let {name} = u.p[{k}]{sw};\n");
    }
    let code = if s.code.contains("return") {
        s.code.clone()
    } else {
        format!("    return {};", s.code.trim().trim_end_matches(';'))
    };
    let output =
        if s.premultiplied { "    return c * u.opacity;" } else { "    return vec4f(c.rgb * c.a, c.a) * u.opacity;" };
    include_str!("shaders/shader_layer.wgsl")
        .replace("//PARAMS", &params)
        .replace("//CODE", &code)
        .replace("//OUTPUT", output)
}

fn const_size(v: &Option<crate::anim::AnimV2>, default: [f64; 2], what: &str) -> Result<[f64; 2]> {
    match v {
        None => Ok(default),
        Some(s) => s.constant().ok_or_else(|| anyhow!("{what}: `size` can't animate here")),
    }
}

impl<'a> Scene<'a> {
    pub fn load(comp: &'a Comp, mut gpu: Gpu) -> Result<Scene<'a>> {
        // decode every image and mask in parallel
        let mut paths: Vec<PathBuf> = Vec::new();
        fn collect(layers: &[Layer], out: &mut Vec<PathBuf>) {
            for l in layers {
                if let Kind::Image { src, .. } = &l.kind {
                    out.push(src.clone());
                }
                if let Some(m) = &l.mask {
                    out.push(m.src.clone());
                }
                if let Kind::Group { layers, .. } = &l.kind {
                    collect(layers, out);
                }
            }
        }
        collect(&comp.layers, &mut paths);
        paths.sort();
        paths.dedup();
        let max = gpu.max_texture;
        let decoded: Vec<(PathBuf, Result<Decoded>)> = paths
            .into_par_iter()
            .map(|p| {
                let r = (|| {
                    let img = image::open(&p).with_context(|| format!("can't read image {}", p.display()))?;
                    let img = if img.width() > max || img.height() > max {
                        img.resize(max, max, image::imageops::FilterType::Lanczos3)
                    } else {
                        img
                    };
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    Ok((w, h, mips(w, h, premultiply(rgba.into_raw()))))
                })();
                (p, r)
            })
            .collect();
        let mut images = HashMap::new();
        for (p, r) in decoded {
            let (w, h, levels) = r?;
            let tex = gpu.texture_rgba8(w, h, &levels, &p.to_string_lossy());
            images.insert(p, (tex, w, h));
        }

        let n_shaders = count_shaders(&comp.layers);
        let uni_stride = (SHADER_UNIFORM_SIZE as usize).div_ceil(gpu.uni_align as usize) * gpu.uni_align as usize;
        gpu.ensure_uniform_capacity((n_shaders.max(1) * uni_stride) as u64);
        let main = gpu.target(comp.width, comp.height, target_needs_backdrop(&comp.layers))?;

        let mut scene = Scene {
            comp,
            gpu,
            res: HashMap::new(),
            masks: HashMap::new(),
            atlas: Atlas::new(),
            pages: vec![],
            decoders: vec![],
            main,
            video_audio: vec![],
            uni_stride,
        };
        let mut fonts = Fonts::new();
        let mut shaper = swash::shape::ShapeContext::new();
        let mut loader = Loader { images: &images, fonts: &mut fonts, shaper: &mut shaper };
        let whole = Window { offset: 0.0, from: 0.0, to: comp.duration };
        scene.load_layers(&comp.layers, [comp.width as f64, comp.height as f64], main, whole, &mut loader)?;
        let pages = std::mem::take(&mut scene.atlas.pages);
        for p in &pages {
            let t = scene.gpu.texture_r8(text::PAGE, text::PAGE, p);
            scene.pages.push(t);
        }
        Ok(scene)
    }

    fn load_layers(
        &mut self,
        layers: &[Layer],
        parent: [f64; 2],
        target: usize,
        win: Window,
        ld: &mut Loader,
    ) -> Result<()> {
        for l in layers {
            self.load_layer(l, parent, target, win, ld).with_context(|| l.path.clone())?;
        }
        Ok(())
    }

    fn load_layer(&mut self, l: &Layer, parent: [f64; 2], target: usize, win: Window, ld: &mut Loader) -> Result<()> {
        let win = win.child(l);
        if let Some(m) = &l.mask {
            let (tex, _, _) = ld.images[&m.src];
            self.masks.insert(m.src.clone(), tex);
        }
        let res = match &l.kind {
            Kind::Image { src, .. } => {
                let (tex, w, h) = ld.images[src];
                Res::Image { tex, w: w as f64, h: h as f64 }
            }
            Kind::Video { src, trim, looped, volume, muted, .. } => {
                let info = video::probe(src)?;
                let (w, h) = (info.width.min(self.gpu.max_texture), info.height.min(self.gpu.max_texture));
                let tex = self.gpu.texture_rgba8(w, h, &[vec![0; (w * h * 4) as usize]], "video");
                let dec = Decoder::new(src.clone(), *trim, self.comp.fps, w, h, *looped, &info);
                self.decoders.push(dec);
                // the soundtrack plays while the layer and all its parents are visible
                let trim = trim + (win.from - win.offset).max(0.0); // a parent's clip can start later than the layer
                let mut play = win.to - win.from;
                if !*looped {
                    play = play.min(info.duration - trim);
                }
                if info.has_audio && !*muted && *volume > 0.0 && play > 0.0 {
                    self.video_audio.push(Audio {
                        src: src.clone(),
                        start: win.from,
                        trim,
                        duration: Some(play),
                        volume: *volume,
                        fade_in: 0.0,
                        fade_out: 0.0,
                        looped: *looped,
                    });
                }
                Res::Video { tex, w: w as f64, h: h as f64, dec: self.decoders.len() - 1 }
            }
            Kind::Text(t) => {
                let font = ld.fonts.load(&t.font)?;
                let layout = text::layout(
                    ld.fonts,
                    &mut self.atlas,
                    ld.shaper,
                    font,
                    &t.text,
                    t.font_size as f32,
                    t.line_height as f32,
                    t.align,
                    t.oversample as f32,
                );
                let fit = match t.max_width {
                    None => 1.0,
                    Some(mw) => {
                        let track = t
                            .tracking
                            .as_ref()
                            .map_or(0.0, |a| a.eval(&Vars { t: l.duration, dur: l.duration, ..Default::default() }));
                        let (w, _) = layout.box_size((track * t.font_size) as f32);
                        if w as f64 > mw { mw / w as f64 } else { 1.0 }
                    }
                };
                Res::Text { layout, fit }
            }
            Kind::Shader(s) => {
                let src = shader_source(s);
                let prog = self.gpu.shader_prog(&src, "shader", target)?;
                Res::Shader { prog }
            }
            Kind::Group { layers, size, isolate } => {
                let size = const_size(size, parent, "an isolated group")
                    .or_else(|e| if *isolate { Err(e) } else { Ok(parent) })?;
                if *isolate {
                    let (w, h) = (size[0].round().max(1.0) as u32, size[1].round().max(1.0) as u32);
                    let t = self.gpu.target(w, h, target_needs_backdrop(layers))?;
                    self.load_layers(layers, size, t, win, ld)?;
                    Res::Isolated { target: t, w: w as f64, h: h as f64 }
                } else {
                    self.load_layers(layers, size, target, win, ld)?;
                    Res::None
                }
            }
            Kind::Rect { .. } | Kind::Ellipse { .. } => Res::None,
        };
        self.res.insert(l.id, res);
        Ok(())
    }

    /// Everything the GPU needs to draw one frame. Video frames are uploaded here.
    pub fn frame(&mut self, frame: u32) -> Result<Frame> {
        let time = frame as f64 / self.comp.fps;
        let mut f = Frame { inst: Vec::with_capacity(256), uniforms: Vec::new(), ops: Vec::new() };
        let ctx = Ctx {
            aff: Aff::ID,
            box_size: [self.comp.width as f64, self.comp.height as f64],
            opacity: 1.0,
            blend: Blend::Normal,
            clip: None,
            res: [self.comp.width as f64, self.comp.height as f64],
        };
        let mut ops = Vec::new();
        let comp = self.comp;
        self.layers(&comp.layers, time, frame, &ctx, &mut f, &mut ops)?;
        f.ops = ops;
        Ok(f)
    }

    fn layers(
        &mut self,
        layers: &[Layer],
        t_parent: f64,
        frame: u32,
        ctx: &Ctx,
        f: &mut Frame,
        ops: &mut Vec<Op>,
    ) -> Result<()> {
        for l in layers {
            if t_parent < l.start || t_parent >= l.start + l.duration {
                continue;
            }
            self.layer(l, t_parent - l.start, frame, ctx, f, ops)?;
        }
        Ok(())
    }

    fn layer(&mut self, l: &Layer, t: f64, frame: u32, ctx: &Ctx, f: &mut Frame, ops: &mut Vec<Op>) -> Result<()> {
        let v = Vars { t, time: frame as f64 / self.comp.fps, frame: frame as f64, dur: l.duration, i: 0.0, n: 0.0 };
        let ev = |a: &Option<AnimF>, d: f64| a.as_ref().map_or(d, |a| a.eval(&v));
        let opacity = ctx.opacity * ev(&l.opacity, 1.0).clamp(0.0, 1.0);
        if opacity <= 0.0 && !matches!(l.kind, Kind::Shader(_)) {
            return Ok(());
        }
        let blend = l.blend.unwrap_or(ctx.blend);
        let clip = match &l.clip {
            Some(c) => ctx
                .aff
                .inverse()
                .map(|inv| Clip { inv, rect: [c[0].eval(&v), c[1].eval(&v), c[2].eval(&v), c[3].eval(&v)] }),
            None => ctx.clip,
        };

        // the layer's box size, before its own scale
        let res = self.res.get(&l.id).ok_or_else(|| anyhow!("{}: not loaded", l.path))?;
        let mut extra_scale = 1.0;
        let mut text_track = 0.0;
        let size: [f64; 2] = match (&l.kind, res) {
            (Kind::Image { size, .. }, Res::Image { w, h, .. })
            | (Kind::Video { size, .. }, Res::Video { w, h, .. }) => size.as_ref().map_or([*w, *h], |s| s.eval(&v)),
            (Kind::Text(ts), Res::Text { layout, fit }) => {
                extra_scale = *fit;
                text_track = ts.tracking.as_ref().map_or(0.0, |a| a.eval(&v)) * ts.font_size;
                let (bw, bh) = layout.box_size(text_track as f32);
                [bw as f64, bh as f64]
            }
            (Kind::Rect { size, .. }, _) | (Kind::Ellipse { size, .. }, _) => size.eval(&v),
            (Kind::Shader(s), _) => s.size.as_ref().map_or(ctx.box_size, |s| s.eval(&v)),
            (Kind::Group { .. }, Res::Isolated { w, h, .. }) => [*w, *h],
            (Kind::Group { size, .. }, _) => size.as_ref().map_or(ctx.box_size, |s| s.eval(&v)),
            _ => bail!("{}: resource mismatch", l.path),
        };

        // text with an ink box is positioned by its glyphs' bounds
        let mut glyph_quads = None;
        let mut ink_origin = [0.0, 0.0];
        let mut size = size;
        if let (Kind::Text(ts), Res::Text { layout, .. }) = (&l.kind, res) {
            let (q, _, _) = layout.quads(&self.atlas, text_track as f32);
            if ts.ink_box && !q.is_empty() {
                let x0 = q.iter().map(|g| g.x).fold(f32::MAX, f32::min);
                let y0 = q.iter().map(|g| g.y).fold(f32::MAX, f32::min);
                let x1 = q.iter().map(|g| g.x + g.w).fold(f32::MIN, f32::max);
                let y1 = q.iter().map(|g| g.y + g.h).fold(f32::MIN, f32::max);
                ink_origin = [x0 as f64, y0 as f64];
                size = [(x1 - x0) as f64, (y1 - y0) as f64];
            }
            glyph_quads = Some(q);
        }

        let anchor = l.anchor.as_ref().map_or([0.5, 0.5], |a| a.eval(&v));
        let pos = l.position.as_ref().map_or([ctx.box_size[0] / 2.0, ctx.box_size[1] / 2.0], |p| p.eval(&v));
        let sc = l.scale.as_ref().map_or([1.0, 1.0], |s| s.eval(&v));
        let rot = ev(&l.rotation, 0.0);
        let local = Aff::translate(pos[0], pos[1])
            .then(Aff::rotate(rot))
            .then(Aff::scale(sc[0] * extra_scale, sc[1] * extra_scale))
            .then(Aff::translate(-anchor[0] * size[0], -anchor[1] * size[1]));
        let aff = ctx.aff.then(local);

        let reveal = l.reveal.as_ref().map(|r| {
            let (s, c) = r.angle.to_radians().sin_cos();
            let proj = [0.0, size[0] * c, size[1] * s, size[0] * c + size[1] * s];
            let pmin = proj.iter().cloned().fold(f64::MAX, f64::min);
            let pmax = proj.iter().cloned().fold(f64::MIN, f64::max);
            let px = 1.0 / aff.det().abs().sqrt().max(1e-9);
            let soft = r.softness.max(px);
            let p = r.progress.eval(&v).clamp(0.0, 1.0);
            [c as f32, s as f32, (pmin + p * (pmax - pmin + soft)) as f32, soft as f32]
        });
        let mask = l.mask.as_ref().map(|m| {
            let rect = m.rect.unwrap_or([0.0, 0.0, size[0], size[1]]);
            (self.masks[&m.src], m.mode as u32 as f32, rect)
        });
        let q = Quad { aff, opacity, blend, clip, reveal, mask };

        match (&l.kind, res) {
            (Kind::Image { fit, tint, .. }, Res::Image { tex, w, h }) => {
                let tint = tint.as_ref().map_or([1.0; 4], |c| c.eval(&v));
                let tex = *tex;
                let (iw, ih) = (*w, *h);
                push_fitted(f, ops, &q, tex, *fit, size, iw, ih, tint);
            }
            (Kind::Video { fit, tint, .. }, Res::Video { tex, w, h, dec }) => {
                let tint = tint.as_ref().map_or([1.0; 4], |c| c.eval(&v));
                let (tex, iw, ih, dec) = (*tex, *w, *h, *dec);
                let idx = (t * self.comp.fps + 1e-6).floor() as i64;
                if let Some(px) = self.decoders[dec].frame(idx)? {
                    self.gpu.write_rgba8(tex, &px);
                }
                push_fitted(f, ops, &q, tex, *fit, size, iw, ih, tint);
            }
            (Kind::Text(ts), Res::Text { layout, .. }) => {
                let col = ts.color.eval(&v);
                let quads = glyph_quads.unwrap();
                let n = layout.visible as f64;
                for g in quads {
                    let a = self.atlas.glyphs[g.atlas];
                    let page = self.pages[a.page as usize];
                    let (gx, gy) = (g.x as f64 - ink_origin[0], g.y as f64 - ink_origin[1]);
                    let (gw, gh) = (g.w as f64, g.h as f64);
                    let mut gaff = Aff::translate(gx, gy);
                    let mut gop = 1.0;
                    if let Some(gl) = &ts.glyphs {
                        let gv = Vars { i: g.index as f64, n, ..v };
                        let off = gl.offset.as_ref().map_or([0.0, 0.0], |o| o.eval(&gv));
                        let gs = gl.scale.as_ref().map_or([1.0, 1.0], |s| s.eval(&gv));
                        let gr = gl.rotation.as_ref().map_or(0.0, |r| r.eval(&gv));
                        gop = gl.opacity.as_ref().map_or(1.0, |o| o.eval(&gv)).clamp(0.0, 1.0);
                        let (cx, cy) = (gx + gw / 2.0, gy + gh / 2.0);
                        gaff = Aff::translate(cx + off[0], cy + off[1])
                            .then(Aff::rotate(gr))
                            .then(Aff::scale(gs[0], gs[1]))
                            .then(Aff::translate(-gw / 2.0, -gh / 2.0));
                    }
                    if gop <= 0.0 {
                        continue;
                    }
                    let pw = text::PAGE as f32;
                    let uv = [a.x as f32 / pw, a.y as f32 / pw, (a.x + a.w) as f32 / pw, (a.y + a.h) as f32 / pw];
                    let o = (opacity * gop) as f32;
                    let color = [
                        col[0] as f32 * col[3] as f32 * o,
                        col[1] as f32 * col[3] as f32 * o,
                        col[2] as f32 * col[3] as f32 * o,
                        col[3] as f32 * o,
                    ];
                    q.push(f, ops, aff.then(gaff), [gw, gh], [gx, gy, gw, gh], uv, color, 1, 0.0, 0.0, page);
                }
            }
            (Kind::Rect { color, radius, .. }, _) => {
                let c = premul(color.eval(&v), opacity);
                let r = radius.as_ref().map_or(0.0, |r| r.eval(&v));
                q.push(
                    f,
                    ops,
                    aff,
                    size,
                    [0.0, 0.0, size[0], size[1]],
                    [0.0, 0.0, 1.0, 1.0],
                    c,
                    2,
                    r,
                    pad(&aff),
                    WHITE,
                );
            }
            (Kind::Ellipse { color, .. }, _) => {
                let c = premul(color.eval(&v), opacity);
                q.push(
                    f,
                    ops,
                    aff,
                    size,
                    [0.0, 0.0, size[0], size[1]],
                    [0.0, 0.0, 1.0, 1.0],
                    c,
                    3,
                    0.0,
                    pad(&aff),
                    WHITE,
                );
            }
            (Kind::Shader(s), Res::Shader { prog }) => {
                let off = f.uniforms.len();
                f.uniforms.resize(off + self.uni_stride, 0);
                let mut u = [0f32; (SHADER_UNIFORM_SIZE / 4) as usize];
                u[..8].copy_from_slice(&[
                    aff.a as f32,
                    aff.b as f32,
                    aff.c as f32,
                    aff.d as f32,
                    aff.tx as f32,
                    aff.ty as f32,
                    size[0] as f32,
                    size[1] as f32,
                ]);
                u[8..20].copy_from_slice(&[
                    ctx.res[0] as f32,
                    ctx.res[1] as f32,
                    size[0] as f32,
                    size[1] as f32,
                    v.time as f32,
                    t as f32,
                    frame as f32,
                    l.duration as f32,
                    opacity as f32,
                    0.0,
                    0.0,
                    0.0,
                ]);
                for (k, (_, p)) in s.params.iter().enumerate() {
                    let base = 20 + k * 4;
                    match p {
                        ParamValue::F(a) => u[base] = a.eval(&v) as f32,
                        ParamValue::V(c) => {
                            for (j, a) in c.iter().enumerate() {
                                u[base + j] = a.eval(&v) as f32;
                            }
                        }
                    }
                }
                f.uniforms[off..off + SHADER_UNIFORM_SIZE as usize].copy_from_slice(bytemuck::cast_slice(&u));
                let flag = if reads_backdrop(&s.code) { READS_BACKDROP } else { 0 };
                ops.push(Op::Shader { prog: *prog, blend, uoff: off as u32 | flag });
            }
            (Kind::Group { layers, .. }, Res::Isolated { target, w, h }) => {
                let (target, w, h) = (*target, *w, *h);
                let inner_ctx =
                    Ctx { aff: Aff::ID, box_size: [w, h], opacity: 1.0, blend: Blend::Normal, clip: None, res: [w, h] };
                let mut inner = Vec::new();
                self.layers(layers, t, frame, &inner_ctx, f, &mut inner)?;
                ops.push(Op::Isolate { target, ops: inner });
                let tex = self.gpu.targets[target].tex;
                let o = opacity as f32;
                q.push(f, ops, aff, [w, h], [0.0, 0.0, w, h], [0.0, 0.0, 1.0, 1.0], [o; 4], 0, 0.0, pad(&aff), tex);
            }
            (Kind::Group { layers, .. }, _) => {
                let inner = Ctx { aff, box_size: size, opacity, blend, clip, res: ctx.res };
                self.layers(layers, t, frame, &inner, f, ops)?;
            }
            _ => bail!("{}: resource mismatch", l.path),
        }
        Ok(())
    }
}

/// Where a layer sits on the composition's timeline while loading: its children's time origin, and the span in
/// which it and all its parents are visible, in composition seconds.
#[derive(Clone, Copy)]
struct Window {
    offset: f64,
    from: f64,
    to: f64,
}

impl Window {
    /// The window of a child layer `l` of this one.
    fn child(self, l: &Layer) -> Window {
        let start = self.offset + l.start;
        Window { offset: start, from: self.from.max(start), to: self.to.min(start + l.duration) }
    }
}

struct Loader<'a> {
    images: &'a HashMap<PathBuf, (usize, u32, u32)>,
    fonts: &'a mut Fonts,
    shaper: &'a mut swash::shape::ShapeContext,
}

fn premul(c: [f64; 4], o: f64) -> [f32; 4] {
    let a = c[3] * o;
    [(c[0] * a) as f32, (c[1] * a) as f32, (c[2] * a) as f32, a as f32]
}

/// About one output pixel, in the layer's own units, for anti-aliased edges.
fn pad(aff: &Aff) -> f64 {
    1.0 / aff.det().abs().sqrt().max(1e-9)
}

struct Quad {
    aff: Aff,
    opacity: f64,
    blend: Blend,
    clip: Option<Clip>,
    reveal: Option<[f32; 4]>,
    mask: Option<(usize, f32, [f64; 4])>,
}

impl Quad {
    /// Push one quad occupying (0,0)-(size) under `aff`; `lbox` is that quad in layer-local coordinates.
    #[allow(clippy::too_many_arguments)]
    fn push(
        &self,
        f: &mut Frame,
        ops: &mut Vec<Op>,
        aff: Aff,
        size: [f64; 2],
        lbox: [f64; 4],
        uv: [f32; 4],
        color: [f32; 4],
        kind: u32,
        radius: f64,
        pad: f64,
        tex: usize,
    ) {
        let (clip_m, clip_t) = match &self.clip {
            Some(c) => (
                [c.inv.a as f32, c.inv.b as f32, c.inv.c as f32, c.inv.d as f32],
                [
                    (c.inv.tx - c.rect[0]) as f32,
                    (c.inv.ty - c.rect[1]) as f32,
                    c.rect[2].max(0.0) as f32,
                    c.rect[3].max(0.0) as f32,
                ],
            ),
            None => ([1.0, 0.0, 0.0, 1.0], [0.0, 0.0, -1.0, -1.0]),
        };
        let (mask_tex, mode, mrect) = match &self.mask {
            Some((t, m, r)) => (*t, *m, [r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32]),
            None => (WHITE, 0.0, [0.0, 0.0, 1.0, 1.0]),
        };
        let inst = Inst {
            m: [aff.a as f32, aff.b as f32, aff.c as f32, aff.d as f32],
            t_size: [aff.tx as f32, aff.ty as f32, size[0] as f32, size[1] as f32],
            uv,
            color,
            clip_m,
            clip_t,
            lbox: [lbox[0] as f32, lbox[1] as f32, lbox[2] as f32, lbox[3] as f32],
            reveal: self.reveal.unwrap_or([1.0, 0.0, 0.0, -1.0]),
            mask_r: mrect,
            params: [kind as f32, radius as f32, mode, pad as f32],
        };
        let idx = f.inst.len() as u32;
        f.inst.push(inst);
        if let Some(Op::Draw { blend, tex: t, mask, first, count }) = ops.last_mut()
            && *blend == self.blend
            && *t == tex
            && *mask == mask_tex
            && *first + *count == idx
        {
            *count += 1;
            return;
        }
        ops.push(Op::Draw { blend: self.blend, tex, mask: mask_tex, first: idx, count: 1 });
    }
}

#[allow(clippy::too_many_arguments)]
fn push_fitted(
    f: &mut Frame,
    ops: &mut Vec<Op>,
    q: &Quad,
    tex: usize,
    fit: Fit,
    size: [f64; 2],
    iw: f64,
    ih: f64,
    tint: [f64; 4],
) {
    let color = premul(tint, q.opacity);
    let (qx, qy, qw, qh, uv) = match fit {
        Fit::Fill => (0.0, 0.0, size[0], size[1], [0.0, 0.0, 1.0, 1.0]),
        Fit::Contain => {
            let s = (size[0] / iw).min(size[1] / ih);
            let (w, h) = (iw * s, ih * s);
            ((size[0] - w) / 2.0, (size[1] - h) / 2.0, w, h, [0.0, 0.0, 1.0, 1.0])
        }
        Fit::Cover => {
            let s = (size[0] / iw).max(size[1] / ih);
            let (uw, uh) = (size[0] / (iw * s), size[1] / (ih * s));
            let (u0, v0) = ((1.0 - uw) / 2.0, (1.0 - uh) / 2.0);
            (0.0, 0.0, size[0], size[1], [u0 as f32, v0 as f32, (u0 + uw) as f32, (v0 + uh) as f32])
        }
    };
    let aff = q.aff.then(Aff::translate(qx, qy));
    q.push(f, ops, aff, [qw, qh], [qx, qy, qw, qh], uv, color, 0, 0.0, pad(&aff), tex);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_effects_compile() {
        for name in crate::effects::NAMES {
            let spec = format!(
                r#"{{"width": 64, "height": 64, "fps": 30, "duration": 1, "layers": [{{"type": "shader", "effect": "{name}"}}]}}"#
            );
            let c = crate::spec::parse(&spec, std::path::Path::new(".")).unwrap();
            let Kind::Shader(s) = &c.layers[0].kind else { panic!() };
            crate::gpu::check_wgsl(&shader_source(s), name).unwrap();
        }
    }

    #[test]
    fn affine_inverse_round_trips() {
        let a = Aff::translate(10.0, 20.0).then(Aff::rotate(30.0)).then(Aff::scale(2.0, 3.0));
        let i = a.inverse().unwrap();
        let id = a.then(i);
        assert!((id.a - 1.0).abs() < 1e-9 && id.b.abs() < 1e-9 && id.tx.abs() < 1e-9 && id.ty.abs() < 1e-9);
    }
}
