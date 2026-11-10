# Yehi

Fast video rendering for AI agents. Describe a video as one JSON file (canvas, timeline, layers, keyframes, expressions) and `yehi` turns it into an MP4 in seconds: frames are composited on the GPU, converted to YUV there, and handed to the fastest encoder the machine has.

The name is the Hebrew "let there be" of Genesis 1:3: you describe the video, and there it is.

It does one thing. There's no editor, no timeline UI, and no browser: a spec goes in, a video comes out, and every frame is a pure function of time.

## Development

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

## License

[MIT](LICENSE).
