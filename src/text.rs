//! Fonts, shaping, and the glyph atlas. Text is shaped once at load; each frame only moves glyph quads,
//! so tracking and per-glyph animation cost nothing extra.

use crate::spec::{Align, FontRef};
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use swash::scale::{Render, ScaleContext, Source, image::Content};
use swash::shape::ShapeContext;
use swash::zeno::Format;

pub const PAGE: u32 = 2048;
const PAD: u32 = 1;

pub struct Fonts {
    db: Option<fontdb::Database>,
    ids: HashMap<FontRef, usize>,
    data: Vec<(Vec<u8>, usize)>,
}

#[derive(Clone, Copy)]
pub struct AtlasGlyph {
    pub page: u32,
    /// Position and size in the page, in texels.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Offset from the pen position on the baseline to the bitmap's top-left, in output pixels.
    pub left: f32,
    pub top: f32,
    /// Output pixels per texel (1 / oversample).
    pub scale: f32,
}

pub struct Atlas {
    pub pages: Vec<Vec<u8>>,
    pub glyphs: Vec<AtlasGlyph>,
    map: HashMap<(usize, u32, u16), Option<usize>>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    scale_ctx: ScaleContext,
}

#[derive(Clone, Copy)]
pub struct PlacedGlyph {
    pub atlas: usize,
    /// Pen position (without tracking) and baseline offset, in output pixels.
    pub x: f32,
    pub y: f32,
    /// Index of the character cluster within its line; tracking adds after each.
    pub cluster: u32,
    /// Index among the visible glyphs of the whole text, for `glyphs` animation.
    pub index: u32,
}

pub struct Line {
    pub glyphs: Vec<PlacedGlyph>,
    pub advance: f32,
    pub clusters: u32,
}

pub struct TextLayout {
    pub lines: Vec<Line>,
    pub ascent: f32,
    pub descent: f32,
    pub line_pitch: f32,
    pub align: Align,
    pub visible: u32,
}

/// A glyph quad for one frame, in the text layer's local coordinates.
pub struct GlyphQuad {
    pub atlas: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub index: u32,
}

impl Fonts {
    pub fn new() -> Self {
        Fonts { db: None, ids: HashMap::new(), data: Vec::new() }
    }

    pub fn load(&mut self, f: &FontRef) -> Result<usize> {
        if let Some(&id) = self.ids.get(f) {
            return Ok(id);
        }
        let (bytes, index) = match f {
            FontRef::File(p) => (std::fs::read(p).map_err(|e| anyhow!("can't read font {}: {e}", p.display()))?, 0),
            FontRef::Family { name, weight, italic } => {
                let db = self.db.get_or_insert_with(|| {
                    let mut db = fontdb::Database::new();
                    db.load_system_fonts();
                    db
                });
                let q = fontdb::Query {
                    families: &[fontdb::Family::Name(name)],
                    weight: fontdb::Weight(*weight),
                    style: if *italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
                    stretch: fontdb::Stretch::Normal,
                };
                let id = db
                    .query(&q)
                    .ok_or_else(|| anyhow!("font family `{name}` isn't installed; give a .ttf/.otf path instead"))?;
                db.with_face_data(id, |d, i| (d.to_vec(), i as usize))
                    .ok_or_else(|| anyhow!("can't load font family `{name}`"))?
            }
        };
        if swash::FontRef::from_index(&bytes, index).is_none() {
            return Err(anyhow!("{f:?} isn't a font swash can read"));
        }
        let id = self.data.len();
        self.data.push((bytes, index));
        self.ids.insert(f.clone(), id);
        Ok(id)
    }

    fn font(&self, id: usize) -> swash::FontRef<'_> {
        let (d, i) = &self.data[id];
        swash::FontRef::from_index(d, *i).unwrap()
    }
}

impl Atlas {
    pub fn new() -> Self {
        Atlas {
            pages: vec![],
            glyphs: vec![],
            map: HashMap::new(),
            shelf_x: 0,
            shelf_y: 0,
            shelf_h: 0,
            scale_ctx: ScaleContext::new(),
        }
    }

    fn glyph(&mut self, fonts: &Fonts, font: usize, px: f32, oversample: f32, id: u16) -> Option<usize> {
        let key = (font, (px * oversample).to_bits(), id);
        if let Some(&g) = self.map.get(&key) {
            return g;
        }
        let f = fonts.font(font);
        let mut scaler = self.scale_ctx.builder(f).size(px * oversample).hint(false).build();
        let img = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
            Source::Outline,
        ])
        .format(Format::Alpha)
        .render(&mut scaler, id);
        let out = img.and_then(|img| {
            let (w, h) = (img.placement.width, img.placement.height);
            if w == 0 || h == 0 {
                return None;
            }
            let alpha: Vec<u8> = match img.content {
                Content::Mask => img.data,
                // colour glyphs (emoji) are drawn as their coverage in the text colour
                Content::Color => img.data.chunks(4).map(|p| p[3]).collect(),
                Content::SubpixelMask => img.data.chunks(4).map(|p| p[1]).collect(),
            };
            let (pw, ph) = (w + 2 * PAD, h + 2 * PAD);
            if pw > PAGE || ph > PAGE {
                return None;
            }
            if self.pages.is_empty() || self.shelf_x + pw > PAGE {
                self.shelf_y += self.shelf_h;
                self.shelf_x = 0;
                self.shelf_h = 0;
            }
            if self.pages.is_empty() || self.shelf_y + ph > PAGE {
                self.pages.push(vec![0; (PAGE * PAGE) as usize]);
                self.shelf_x = 0;
                self.shelf_y = 0;
                self.shelf_h = 0;
            }
            let page = self.pages.len() - 1;
            let (x0, y0) = (self.shelf_x + PAD, self.shelf_y + PAD);
            let buf = &mut self.pages[page];
            for row in 0..h {
                let dst = ((y0 + row) * PAGE + x0) as usize;
                buf[dst..dst + w as usize].copy_from_slice(&alpha[(row * w) as usize..((row + 1) * w) as usize]);
            }
            self.shelf_x += pw;
            self.shelf_h = self.shelf_h.max(ph);
            let s = 1.0 / oversample;
            self.glyphs.push(AtlasGlyph {
                page: page as u32,
                x: x0 - PAD,
                y: y0 - PAD,
                w: pw,
                h: ph,
                left: (img.placement.left as f32 - PAD as f32) * s,
                top: (img.placement.top as f32 + PAD as f32) * s,
                scale: s,
            });
            Some(self.glyphs.len() - 1)
        });
        self.map.insert(key, out);
        out
    }
}

#[allow(clippy::too_many_arguments)]
pub fn layout(
    fonts: &Fonts,
    atlas: &mut Atlas,
    shaper: &mut ShapeContext,
    font: usize,
    text: &str,
    size: f32,
    line_height: f32,
    align: Align,
    oversample: f32,
) -> TextLayout {
    let f = fonts.font(font);
    let m = f.metrics(&[]).scale(size);
    let mut lines = Vec::new();
    let mut index = 0;
    for line in text.split('\n') {
        let mut glyphs = Vec::new();
        let mut pen = 0.0f32;
        let mut cluster = 0u32;
        let mut sh = shaper.builder(f).size(size).build();
        sh.add_str(line);
        let mut shaped = Vec::new();
        sh.shape_with(|c| {
            for g in c.glyphs {
                shaped.push((g.id, g.x, g.y, g.advance, cluster));
            }
            cluster += 1;
        });
        for (id, gx, gy, adv, cl) in shaped {
            if let Some(a) = atlas.glyph(fonts, font, size, oversample, id) {
                glyphs.push(PlacedGlyph { atlas: a, x: pen + gx, y: -gy, cluster: cl, index });
                index += 1;
            }
            pen += adv;
        }
        lines.push(Line { glyphs, advance: pen, clusters: cluster });
    }
    TextLayout { lines, ascent: m.ascent, descent: m.descent, line_pitch: size * line_height, align, visible: index }
}

impl TextLayout {
    /// The layout box (width, height) at a tracking in pixels.
    pub fn box_size(&self, track: f32) -> (f32, f32) {
        let w = self.lines.iter().map(|l| self.line_width(l, track)).fold(0.0, f32::max);
        let h = self.ascent + self.descent + self.line_pitch * (self.lines.len().saturating_sub(1)) as f32;
        (w, h)
    }

    fn line_width(&self, l: &Line, track: f32) -> f32 {
        l.advance + track * l.clusters.saturating_sub(1) as f32
    }

    /// Glyph quads at a tracking in pixels, in layout-box coordinates.
    pub fn quads(&self, atlas: &Atlas, track: f32) -> (Vec<GlyphQuad>, f32, f32) {
        let (bw, bh) = self.box_size(track);
        let mut out = Vec::new();
        for (li, l) in self.lines.iter().enumerate() {
            let lw = self.line_width(l, track);
            let x0 = match self.align {
                Align::Left => 0.0,
                Align::Center => (bw - lw) / 2.0,
                Align::Right => bw - lw,
            };
            let base = self.ascent + self.line_pitch * li as f32;
            for g in &l.glyphs {
                let a = &atlas.glyphs[g.atlas];
                out.push(GlyphQuad {
                    atlas: g.atlas,
                    x: x0 + g.x + track * g.cluster as f32 + a.left,
                    y: base + g.y - a.top,
                    w: a.w as f32 * a.scale,
                    h: a.h as f32 * a.scale,
                    index: g.index,
                });
            }
        }
        (out, bw, bh)
    }
}
