# Yehi

A CLI that renders video from a JSON spec as fast as the machine allows: GPU compositing (wgpu), GPU color conversion, and the fastest ffmpeg encoder available. It's built for agents to drive. It stays a renderer: no editing features, no UI.

## Rules

- **Speed is the feature.** Measure before and after any change on the hot path. Nothing per frame on the CPU should scale with pixels.
- **Every frame is a pure function of time.** No state carried between frames except decoder read-ahead; any frame must render the same alone, in any order, on any machine.
- **Errors are for agents.** Name the field path and say what's allowed.
- **The docs are the interface.** A change to the spec format updates its reference in the same change.
- **Portable.** Nothing tied to one OS or GPU; hardware encoders are chosen at run time with a software fallback.

## Commands

- `cargo test`, `cargo clippy --all-targets` and `cargo fmt --check`: the checks CI runs.
