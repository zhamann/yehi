# Yehi

A CLI that renders video from a JSON spec as fast as the machine allows: GPU compositing (wgpu), GPU color conversion, and the fastest ffmpeg encoder available. It's built for agents to drive. It stays a renderer: no editing features, no UI.

## Layout

- `src/spec.rs`: the JSON format, validated into the composition model. Unknown fields and fields on the wrong layer type are errors.
- `src/anim.rs`, `src/expr.rs`: animatable values (constants, expressions, keyframes) and the expression language.
- `src/scene.rs`: loading assets onto the GPU, and evaluating a frame into instances and ops.
- `src/gpu.rs`, `src/shaders/`: pipelines, the compositor, YUV/RGBA conversion, and shader-layer compilation.
- `src/text.rs`: font loading, shaping, the glyph atlas. `src/video.rs`: video-layer decoding. `src/encode.rs`: encoder choice and the ffmpeg command. `src/sheet.rs`: contact sheets.
- `docs/spec.md`: the spec reference, compiled into the binary as `yehi docs`.
- `skills/yehi/SKILL.md`: the agent skill.

## Rules

- **Speed is the feature.** Measure before and after any change on the hot path (`yehi video <spec> -o x.mp4 --benchmark` skips encoding to time rendering alone). Nothing per frame on the CPU should scale with pixels.
- **Every frame is a pure function of time.** No state carried between frames except decoder read-ahead; any frame must render the same alone, in any order, on any machine.
- **Errors are for agents.** Name the field path and say what's allowed.
- **The docs are the interface.** A change to the spec format updates `docs/spec.md` in the same change, and `yehi check` must catch what the docs forbid.
- **Portable.** Nothing tied to one OS or GPU; hardware encoders are chosen at run time with a software fallback.

## Commands

- `cargo test`: unit tests and end-to-end tests on `examples/hello.json` (need a GPU or Mesa's software Vulkan, and ffmpeg).
- `cargo clippy --all-targets` and `cargo fmt --check`: the checks CI runs.
