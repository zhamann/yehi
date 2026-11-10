---
name: yehi
description: Render videos fast from a JSON spec with the `yehi` CLI (GPU compositing, hardware encoding). Use when making, previewing or exporting any video, motion graphic, social clip, title card or animated text, or when an existing video build is too slow.
---

# yehi

`yehi` turns one JSON file (canvas, timeline, layers) into a video. Frames are composited on the GPU and encoded with the fastest encoder on the machine, so a minute of 1080x1920 video takes seconds and a still takes a fraction of one.

Run `yehi docs` first and follow it: it's the complete spec, with patterns for scenes, slot rises, write-ons, camera shake, fades and shaders. If `yehi` isn't on the PATH, ask before installing it (`cargo install --git https://github.com/zhamann/yehi`).

## Work loop

1. **Make the assets the spec can't**: art and photos as PNG/JPEG, fonts as files, music and voice as audio files. Anything drawable as shapes, text, gradients, noise or per-pixel math belongs in the spec instead (rects, ellipses, text, `shader` layers), not in pre-rendered images.
2. **Write the spec** next to its assets; paths are relative to the spec file. Generate it from a script when the video comes from data (one scene group per item, timings computed), and keep that script with the project.
3. **`yehi check spec.json`** until it passes; errors name the exact field.
4. **Look before rendering the whole thing**: `yehi sheet spec.json -o sheet.jpg` (12 labeled frames on one image), and `yehi still spec.json <t>... -o dir/` at the moments that matter: each scene once settled, and mid-animation where things move. Read the images and fix overflow, collisions, clipping and contrast.
5. **`yehi video spec.json -o out.mp4`.** Use `--start`/`--end` to preview one part, `--quality draft` for a quick full pass, and `--codec hevc` when the destination takes HEVC (every major social platform does) and speed matters.

## Defaults worth knowing

- Every layer's `position` is where its anchor sits (default: the center of the layer, placed at the center of its parent).
- Children of a group count time from the group's `start`; use one group per scene.
- Expressions see `t` (layer time), `dur`, `time`, `frame`; easing functions clamp, so `outExpo(t / 0.4)` animates for 0.4s and holds.
- Use font files rather than installed family names so the output is the same on every machine.
- Audio goes in the top-level `audio` list; `loudness: -14` normalizes the mix for social platforms.
