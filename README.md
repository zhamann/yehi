# Yehi

Fast video rendering for AI agents. Describe a video as one JSON file (canvas, timeline, layers, keyframes, expressions) and `yehi` turns it into an MP4 in seconds: frames are composited on the GPU, converted to YUV there, and handed to the fastest encoder the machine has.

The name is the Hebrew "let there be" of Genesis 1:3: you describe the video, and there it is.

It does one thing. There's no editor, no timeline UI, and no browser: a spec goes in, a video comes out, and every frame is a pure function of time.

## How fast

A real 64-second 1080x1920 social video (14 scenes of engraved plates, kinetic type, slot reveals, write-ons, camera shake, grain, vignette, a dozen shader passes) on an Apple M4:

| | Time |
|---|---|
| The project's original Python/numpy renderer | 115 s |
| `yehi video` (H.264) | 9 s |
| `yehi video --codec hevc` | 5.3 s |
| `yehi video --quality draft` | 3.4 s |
| `yehi sheet` (12 labeled frames) or `yehi still` | under 1 s |
| Linux container, no GPU (software Vulkan), draft | 53 s |

The compositor alone runs at about 600 frames a second at 1080x1920; the rest is the encoder.

## Install

yehi needs [ffmpeg](https://ffmpeg.org) on the PATH (`brew install ffmpeg`, `apt install ffmpeg`, `winget install ffmpeg`), or `YEHI_FFMPEG` and `YEHI_FFPROBE` set to its programs. Build it with [Rust](https://rustup.rs):

```bash
cargo install --path .
```

On Linux without a GPU, also install a software Vulkan driver (`apt install mesa-vulkan-drivers`). The Docker image includes both:

```bash
docker build -t yehi .
docker run --rm -v "$PWD:/work" yehi video video.json -o out.mp4
```

`yehi info` shows which GPU and encoders it will use.

## Use

```bash
yehi check video.json                 # validate and load everything
yehi sheet video.json -o sheet.jpg    # evenly spaced frames on one image, labeled with their times
yehi still video.json 1.5 4 -o stills/
yehi video video.json -o out.mp4      # --codec h264|hevc|prores, --quality draft|standard|high
yehi video video.json -o part.mp4 --start 10 --end 14
yehi docs                             # the full spec reference
```

A spec:

```json
{
  "width": 1080, "height": 1920, "fps": 30, "duration": 3, "background": "#14131a",
  "layers": [
    {"type": "shader", "effect": "linear_gradient", "params": {"color_a": "#1d1b2a", "color_b": "#3a3150"}},
    {"type": "text", "text": "EVERY FRAME", "font": "fonts/EBGaramond-SemiBold.ttf", "font_size": 60,
     "color": "#f3ead8", "tracking": "0.18 + 0.5 * (1 - outExpo(t / 0.6))", "opacity": "clamp(t / 0.3, 0, 1)"},
    {"type": "rect", "size": [560, 170], "color": "#e8b27a", "position": [540, 1100], "start": 1,
     "reveal": {"progress": "outExpo(t / 0.3)", "angle": -90}},
    {"type": "shader", "effect": "grain", "params": {"amount": 0.035}}
  ]
}
```

Layers are images, videos, text, rects, ellipses, shaders, and groups. Any property can be a number, a math expression of time (`"540 + 20 * sin(t * 3)"`), or keyframes with easing. Text is shaped with kerning and can animate its tracking or each glyph. Layers can be clipped, wiped on, masked, and blended; groups make scenes and can be isolated into their own layer. Shader layers run your WGSL per pixel and can read what's beneath them, for gradients, grain, grades, and anything else procedural. Audio files and video soundtracks are mixed in the same pass, optionally normalized to a loudness target.

[docs/spec.md](docs/spec.md) is the complete reference (`yehi docs` prints it), and [examples/hello.json](examples/hello.json) is a 9-second example using most of it.

## For agents

The CLI is built to be driven by an agent:

- `yehi docs` prints the whole spec, so an agent can learn it from the tool itself.
- Errors name the field and say what's allowed: `layers[2].layers[0].opacity: unknown function 'easeOut'; available: ...`. Unknown fields are errors, so typos never silently render wrong.
- `yehi sheet` puts a dozen labeled frames on one image in well under a second, so checking the work is cheap enough to do every time.
- Frames are independent, so `--start`/`--end` render any part on its own.

[skills/yehi/SKILL.md](skills/yehi/SKILL.md) is a skill for Claude Code and other agents that read skills: copy or link it into `~/.claude/skills/yehi/` to make `yehi` available in every project.

## How it works

- **Load once.** The spec is parsed and validated, then images and masks are decoded and mipmapped in parallel, text is shaped (swash) into a shared glyph atlas, and shader layers are compiled (naga checks them first, so a mistake is an error message, not a crash).
- **Each frame is a draw list.** Evaluating the layer tree at a time gives one buffer of instanced quads and a few ops; consecutive quads with the same texture batch into one draw. Isolated groups and shaders that read their backdrop split the render pass only where they must.
- **The GPU does the pixels** ([wgpu](https://wgpu.rs): Metal, Vulkan, DX12), compositing premultiplied color in 16-bit float, then converting to dithered BT.709 YUV 4:2:0 in a compute shader so the CPU only copies 3 MB a frame.
- **Everything overlaps.** Four frames are in flight: while the GPU draws one, earlier ones are copied back and piped to ffmpeg on another thread, which encodes with VideoToolbox on macOS, NVENC on NVIDIA, or x264/x265 elsewhere.

Because a frame depends only on its time, a render can be split across processes or machines and the parts joined; `--start`/`--end` is the building block for that.

## Development

```bash
cargo test            # unit tests, plus end-to-end tests that render examples/hello.json (needs a GPU or software Vulkan, and ffmpeg)
cargo clippy --all-targets
cargo fmt
```

## License

[MIT](LICENSE). The example's EB Garamond fonts are under the [SIL Open Font License](examples/fonts/OFL.txt).
