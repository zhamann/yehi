# Contributing

Thanks for helping. yehi is a renderer and stays one: a JSON spec goes in, a video comes out. Changes that make it faster, more correct, more portable, or clearer to drive are welcome; editing features and UI are out of scope.

## Build and test

You need [Rust](https://rustup.rs) 1.90 or later and [ffmpeg](https://ffmpeg.org). The end-to-end tests render on the GPU, or on the CPU through a software Vulkan driver on Linux (`apt install mesa-vulkan-drivers`).

```bash
cargo test                              # unit tests, and end-to-end tests on examples/hello.json
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo build --release && ./target/release/yehi video examples/hello.json -o x.mp4 --benchmark
```

`--benchmark` renders without encoding, to time rendering alone.

## Rules

[AGENTS.md](AGENTS.md) has the layout and the rules every change follows. In short:

- **Speed is the feature.** Measure before and after any change on the hot path, and say what you measured in the pull request.
- **Every frame is a pure function of time.** Any frame renders the same alone, in any order, on any machine.
- **Errors are for agents.** Name the field path and say what's allowed.
- **The docs are the interface.** A change to the spec format updates [docs/spec.md](docs/spec.md) in the same change, and `yehi check` catches what the docs forbid.
- **Portable.** Nothing tied to one OS or GPU.

## Releasing

Maintainers: set `version` in `Cargo.toml` and merge that change, then push a matching tag on a commit in `main`:

```bash
git tag v0.2.0
git push origin v0.2.0
```

The release workflow checks the tag matches `Cargo.toml` and is in `main`, runs the CI checks, builds the binary for each platform (Linux ones against glibc 2.17), publishes `yehi-<platform>` and then the launcher, `@zhamann/yehi`, to npm with provenance, and attaches the binaries, their licenses, and `SHA256SUMS` to a GitHub release. A prerelease tag like `v0.2.0-rc.1` publishes under npm's `next` tag, so `latest` stays on the newest stable release.

Publishing uses npm trusted publishing for packages that have it set up (each package's settings on npmjs.com, with this repository and `release.yml`), and the `NPM_TOKEN` secret otherwise. A failed release can be re-run: versions already on npm are skipped.
