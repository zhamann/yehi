# Yehi spec reference

A video is one JSON file: the canvas, the timeline, and a list of layers drawn bottom to top. Every frame is a pure function of time, so any frame can be rendered on its own, in any order, on any machine.

```sh
yehi check  video.json                      # validate and load everything, render nothing
yehi sheet  video.json -o sheet.jpg          # 12 evenly spaced frames on one image, labeled with their times
yehi still  video.json 1.5 4 -o stills/      # full-size PNGs at those times
yehi video  video.json -o out.mp4            # the video
```

`yehi video` options: `--codec h264|hevc|prores` (h264), `--quality draft|standard|high` (standard), `--encoder auto|hardware|software|<ffmpeg encoder>` (auto picks the fastest that works here), `--bitrate 12M`.

## Workflow

1. Write the spec. Paths (images, fonts, masks) are relative to the spec file.
2. `yehi check` until it passes. Errors name the exact field: `layers[2].layers[0].opacity: unknown function 'easeOut'`.
3. `yehi sheet` (or `yehi still` at the moments that matter) and look at the frames. Rendering a sheet takes well under a second.
4. `yehi video`.

## The file

```json
{
  "width": 1080, "height": 1920, "fps": 30, "duration": 8,
  "background": "#101014",
  "layers": [ ... ]
}
```

| Field | Meaning |
|---|---|
| `width`, `height` | Even pixel sizes. Video output also needs `width` to be a multiple of 8 (1080, 1920, 720, 1280 are). |
| `fps`, `duration` | Frames per second and length in seconds. |
| `background` | Color under everything. Default black. |
| `layers` | Drawn first to last; later layers cover earlier ones. |

## Values that animate

Any field marked *animatable* takes:

- **a number**: `0.5`
- **an expression string**, evaluated every frame: `"540 + 20 * sin(t * 3)"`, `"clamp(t / 0.4, 0, 1)"`
- **keyframes**: `[{"t": 0, "v": 0}, {"t": 0.5, "v": 1, "ease": "outExpo"}]`. `t` is seconds in the layer's own time. A key's `ease` shapes the segment arriving at it. Before the first key the value holds the first; after the last, the last.

Two-part values (`position`, `anchor`, `scale`, `size`) are `[x, y]` with each part animatable on its own (`[540, "960 - t * 20"]`), or keyframes whose `v` is `[x, y]`. A single number for `scale` scales both. Colors are `"#rgb"`, `"#rrggbb"` or `"#rrggbbaa"`, keyframes of those, or `[r, g, b, a]` with parts from 0 to 1, each animatable.

**Expression variables**: `t` (seconds since the layer started), `dur` (the layer's duration), `time` (seconds since the video started), `frame` (frame number), `i` and `n` (glyph index and count, in text `glyphs` only), `pi`, `e`.

**Operators**: `+ - * / % ^` (`**` also means power), comparisons `< <= > >= == !=`, `&& || !`, and `cond ? a : b`.

**Functions**: `sin cos tan asin acos atan atan2 sqrt exp log pow abs min max clamp(x, lo, hi) floor ceil round fract sign mix(a, b, k) lerp step(edge, x) smoothstep(a, b, x) remap(x, inLo, inHi, outLo, outHi) rand(x)` (stable 0..1 hash) `noise(x)`, `noise2(x, y)` (smooth -1..1).

**Easings** (as functions of 0..1 in expressions, or as a keyframe's `ease`): `linear`, `inSine outSine inOutSine`, `inQuad outQuad inOutQuad`, `inCubic outCubic inOutCubic`, `inQuart outQuart inOutQuart`, `inExpo outExpo inOutExpo`, `inBack outBack inOutBack`, `outElastic`, `outBounce`. Keyframes also take `"hold"` (jump at the key) and `[x1, y1, x2, y2]` (CSS cubic-bezier). Easing functions clamp their input, so `outExpo(t / 0.5)` animates over the first half second and then holds.

## Every layer

| Field | Default | Meaning |
|---|---|---|
| `type` | | `image`, `text`, `rect`, `ellipse`, `shader` or `group` |
| `name` | | Shown in error messages. |
| `start` | 0 | Seconds after its parent starts. |
| `duration` or `end` | to the parent's end | When it disappears. A layer is visible while `start <= t < start + duration`. |
| `position` | parent's center | *Animatable* `[x, y]`: where the anchor sits, in the parent's pixels (top-left origin). |
| `anchor` | `[0.5, 0.5]` | *Animatable*: the point of the layer that `position` places, as a fraction of its size. `[0, 0]` is top-left. Scale and rotation turn around it. |
| `scale` | 1 | *Animatable* number or `[x, y]`. |
| `rotation` | 0 | *Animatable* degrees, clockwise. |
| `opacity` | 1 | *Animatable*, 0 to 1. A non-isolated group multiplies its children by it. |
| `blend` | `normal` | `normal`, `add`, `multiply` or `screen`. Children of a group inherit it. |
| `clip` | | *Animatable* `[x, y, w, h]` in the **parent's** coordinates. Only what falls inside is drawn; it doesn't move with the layer, so a layer sliding inside its clip appears through a slot. A clip on a group covers its children unless a child sets its own. |
| `reveal` | | Wipes the layer on: `{"progress": ..., "angle": 0, "softness": 0}`. `progress` (*animatable*, 0..1) moves the edge across the layer; `angle` is the direction it travels in degrees (0 left to right, 90 top to bottom, -90 bottom to top, 180 right to left); `softness` is the edge's width in the layer's pixels. |
| `mask` | | `{"src": "mask.png", "mode": "alpha", "rect": [x, y, w, h]}`: multiplies the layer by an image. `mode` is `alpha`, `luma`, `invert_alpha` or `invert_luma`. `rect` places the mask in the layer's own pixels (default: stretched over the layer). |

`reveal` and `mask` on a group need `"isolate": true`.

## Layer types

### image

`src` (PNG, JPEG or WebP), `size` (*animatable* `[w, h]`; default the image's own pixels), `fit` (`fill` stretches to `size`, `contain` fits inside it, `cover` fills it and crops; default `fill`), `tint` (*animatable* color multiplied in).

Images are drawn with mipmaps, so heavy downscaling stays smooth. Make images at least as large as they'll appear.

### text

| Field | Default | Meaning |
|---|---|---|
| `text` | | The string. `\n` starts a new line. |
| `font` | | A font file path (`.ttf`, `.otf`, `.ttc`), or an installed family name such as `"Helvetica"`. Prefer files: they render the same on every machine. |
| `font_weight`, `font_style` | 400, `normal` | When `font` is a family name: weight 100–900, `normal` or `italic`. |
| `font_size` | 48 | In pixels. |
| `color` | white | *Animatable*. |
| `tracking` | 0 | *Animatable* letter spacing as a fraction of `font_size` (0.1 adds a tenth of the size after each character). Animate it to track text in or out. |
| `line_height` | 1.2 | Baseline to baseline, as a multiple of `font_size`. |
| `align` | `center` | Lines aligned `left`, `center` or `right` within the text box. |
| `max_width` | | Shrinks the text uniformly so its widest line fits, measured at the layer's final tracking. |
| `box` | `layout` | What the anchor measures: `layout` (advance widths, ascent to descent; lines of different text sit on the same baseline) or `ink` (the glyphs' actual bounds; centers exactly what you see). |
| `glyphs` | | Per-character animation: `offset` `[x, y]`, `opacity`, `scale`, `rotation`, each *animatable* with `i` (0-based index of the visible glyph) and `n` (count) available. Glyphs scale and turn around their own centers. |
| `oversample` | 1 | Rasterize glyphs this many times larger, for text you scale up. |

Text is shaped with kerning and ligatures. Stagger example:

```json
"glyphs": {
  "opacity": "clamp((t - i * 0.04) / 0.2, 0, 1)",
  "offset": [0, "30 * (1 - outBack(clamp((t - i * 0.04) / 0.4, 0, 1)))"]
}
```

### rect and ellipse

`size` (*animatable* `[w, h]`, required), `color` (*animatable*, default white), `radius` (rect only, *animatable* corner radius). Edges are anti-aliased.

### shader

A procedural layer: a WGSL function you write runs for every pixel of the layer's box, and its result is composited like any other layer. Use it for gradients, grain, vignettes, color grades, noise skies, light leaks, anything per-pixel.

| Field | Meaning |
|---|---|
| `code` | The body of a WGSL function returning `vec4f`: a straight (not premultiplied) color, `vec4f(rgb, alpha)`. A single expression without `return` is returned for you. |
| `effect` | Instead of `code`, a built-in: `vignette`, `grain`, `linear_gradient`, `radial_gradient` (see below). |
| `params` | Named values the code reads, up to 16: a number or expression (`f32`), `[a, b]` (`vec2f`), `[a, b, c]` (`vec3f`), `[a, b, c, d]` or `"#hex"` (`vec4f`). All *animatable*. Names must be WGSL identifiers. |
| `size` | The box the shader fills. Default: the parent's size (the whole frame at the top level). |

In scope inside `code`:

| Name | Type | Meaning |
|---|---|---|
| `uv` | `vec2f` | 0..1 across the layer's box. |
| `px` | `vec2f` | Pixel position in the frame (or in the isolated group being drawn). |
| `res`, `size` | `vec2f` | The frame's size; the layer's box size. |
| `t`, `time`, `frame`, `dur` | `f32` | As in expressions. |
| `backdrop(p: vec2f) -> vec4f` | | What's already been drawn below this layer at pixel `p`, premultiplied. Reading it lets a shader regrade, blur, or distort what's underneath (return an opaque color to replace it). |
| `hash(p: vec2f) -> f32` | | Stable 0..1 random value per integer point. |
| `noise(p: vec2f) -> f32` | | Smooth 0..1 value noise. |

```json
{"type": "shader", "params": {"warm": 0.15},
 "code": "let c = backdrop(px); return vec4f(c.r + warm, c.g, c.b - warm * 0.5, 1.0);"}
```

Built-in effects and their params (defaults in parentheses):

- `vignette`: darkens toward the edges. `amount` (0.5), `color` ("#000000"), `inner` (0.4) and `outer` (1.5), the distances from the center (1 = edge midpoints) where it starts and reaches full strength.
- `grain`: film grain that changes every frame. `amount` (0.04), `size_px` (1).
- `linear_gradient`: `from` ([0.5, 0]) and `to` ([0.5, 1]) in uv, `color_a` ("#000000"), `color_b` ("#ffffff").
- `radial_gradient`: `center` ([0.5, 0.5]), `radius` ([0.5, 0.5]) in uv, `inner` (0, where the blend starts as a fraction of the radius), `color_a` at the center ("#ffffff"), `color_b` outside ("#000000").

A shader layer can't take `clip`, `reveal` or `mask`; put it in an isolated group and set them there.

### group

`layers` (its children, drawn in order), `size` (the box its children are laid out in: their default `position` is its center; default: the parent's size), `isolate`.

A group's transform, opacity, blend and clip apply to its children, and its `start` shifts their timeline: a child's `start` and `t` are counted from the group's start. Use groups for scenes (a group per scene with `start` and `duration`), for moving several layers together (a camera shake on one group), and for nesting.

With `"isolate": true` the children are first drawn into the group's own transparent layer of `size` (which can't animate), and that layer is then composited, so `opacity` fades the group as one picture, `blend` blends the result, `reveal` and `mask` work, and shaders inside it read only the group's own backdrop. Isolated groups cost a little more; use them when you need those.

## Colors and compositing

Colors are composited in sRGB-encoded values with premultiplied alpha, the way image editors and browsers do, in floating point so gradients don't band. The output is BT.709 H.264/HEVC 4:2:0 (or ProRes 422 HQ), dithered.

## Patterns

**A scene** is a group with `start` and `duration`. Children start relative to it:

```json
{"type": "group", "start": 4, "duration": 4, "layers": [
  {"type": "image", "src": "bg.jpg", "size": [1080, 1920], "fit": "cover", "scale": "1.08 - 0.08 * t / dur"},
  {"type": "text", "text": "SO HE WAITED", "font": "fonts/Inter-Bold.ttf", "font_size": 96, "start": 0.3,
   "opacity": "clamp(t / 0.3, 0, 1)", "position": [540, "960 + 40 * (1 - outExpo(t / 0.6))"]}
]}
```

**Rise through a slot**: move the layer and clip it to where it ends up.

```json
{"type": "text", "text": "STOPPED HIM", "font": "f.ttf", "font_size": 176, "anchor": [0.5, 0],
 "position": [540, "900 + 110 * (1 - outExpo(t / 0.36))"], "clip": [0, 900, 1080, 180]}
```

**Write on**: `"reveal": {"progress": "clamp(t / 1.2, 0, 1)", "softness": 90}`.

**Camera shake that settles**: on a group, `"position": ["540 + 13 * exp(-t * 9) * sin(t * 38)", "960 + 9 * exp(-t * 9) * sin(t * 31 + 1.3)"]`.

**Ken Burns**: `"scale": "1.1 - 0.1 * t / dur"` with `"position": ["540 + 30 * t / dur", 960]`.

**Fade to a color at the end**: a full-frame `rect` on top with `"opacity": "clamp((t - (dur - 1)) / 1, 0, 1)"`.

**Flash on a cut**: a full-frame white `rect` with `"duration": 0.4, "opacity": "pow(1 - t / 0.4, 2)"`.

## Speed

Frames are composited on the GPU (Metal, Vulkan or DX12; a software Vulkan driver works where there's no GPU) and converted to YUV there, so the CPU only copies bytes to the encoder, which is usually the limit. On an Apple M4, a 64-second 1080x1920 video renders in about 9 s with H.264, 5 s with `--codec hevc`, and 3.5 s with `--quality draft`. Loading reads every image and font once, in parallel. Expressions and keyframes cost nothing measurable; a sheet or still takes a fraction of a second, so check frames freely.
