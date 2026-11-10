mod anim;
mod effects;
mod encode;
mod expr;
mod gpu;
mod scene;
mod sheet;
mod spec;
mod text;
mod video;

use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};
use encode::{Codec, Opts, Quality};
use gpu::Gpu;
use scene::Scene;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::sync_channel;
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "yehi",
    version,
    about = "Fast GPU video rendering from a JSON spec. Run `yehi docs` for the spec format."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Render the video.
    Video {
        spec: PathBuf,
        /// Output file: .mp4, .mov or .mkv (ProRes: .mov or .mkv).
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, value_enum, default_value = "h264")]
        codec: Codec,
        /// auto, hardware, software, or an ffmpeg encoder name such as libx264.
        #[arg(long, default_value = "auto")]
        encoder: String,
        #[arg(long, value_enum, default_value = "standard")]
        quality: Quality,
        /// Target bitrate such as 12M, overriding --quality's.
        #[arg(long)]
        bitrate: Option<String>,
        /// Leave out the audio.
        #[arg(long)]
        no_audio: bool,
        /// Render from this time in seconds (a preview of part of the video, or one chunk of a split render).
        #[arg(long)]
        start: Option<f64>,
        /// Render up to this time in seconds.
        #[arg(long)]
        end: Option<f64>,
        /// Render without encoding, to measure rendering alone.
        #[arg(long, hide = true)]
        benchmark: bool,
    },
    /// Render still frames at times in seconds, as PNG (or JPEG with a .jpg out).
    Still {
        spec: PathBuf,
        /// Times in seconds.
        #[arg(required = true, num_args = 1..)]
        times: Vec<f64>,
        /// A directory (one still_<time>.png per time), or a file name when there is one time.
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Render a contact sheet: evenly spaced frames in one image, each labeled with its time.
    Sheet {
        spec: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        /// How many frames.
        #[arg(long, default_value_t = 12, value_parser = clap::value_parser!(u32).range(1..=400))]
        count: u32,
        /// Exact times in seconds instead of evenly spaced ones.
        #[arg(long, num_args = 1.., value_delimiter = ',')]
        times: Option<Vec<f64>>,
        /// Width of each frame in the sheet.
        #[arg(long, default_value_t = 270, value_parser = clap::value_parser!(u32).range(16..=3840))]
        width: u32,
    },
    /// Validate a spec and load everything it uses, without rendering.
    Check { spec: PathBuf },
    /// Print the spec reference.
    Docs,
    /// Show the GPU and encoders this machine would use.
    Info,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Docs => {
            print!("{}", include_str!("../docs/spec.md"));
            Ok(())
        }
        Cmd::Info => {
            let g = Gpu::new()?;
            println!("GPU: {} ({:?}, {:?})", g.info.name, g.info.backend, g.info.device_type);
            for c in [Codec::H264, Codec::Hevc, Codec::Prores] {
                println!("{c:?} encoder: {}", encode::pick(c, "auto").unwrap_or_else(|e| format!("none ({e})")));
            }
            Ok(())
        }
        Cmd::Check { spec } => {
            let t0 = Instant::now();
            let comp = spec::load(&spec)?;
            check_audio(&comp)?;
            let gpu = Gpu::new()?;
            let mut scene = Scene::load(&comp, gpu)?;
            let mut draws = 0;
            for f in [0, comp.frames / 2, comp.frames - 1] {
                draws = draws.max(scene.frame(f)?.inst.len());
            }
            println!(
                "ok: {}x{} at {} fps, {:.2}s ({} frames), {} top-level layers, up to {draws} quads a frame; loaded in {:.2}s",
                comp.width,
                comp.height,
                comp.fps,
                comp.duration,
                comp.frames,
                comp.layers.len(),
                t0.elapsed().as_secs_f64()
            );
            if comp.width % 8 != 0 {
                eprintln!(
                    "warning: width {} isn't a multiple of 8, so `yehi video` will refuse it (stills and sheets work)",
                    comp.width
                );
            }
            Ok(())
        }
        Cmd::Video { spec, out, codec, encoder, quality, bitrate, no_audio, start, end, benchmark } => {
            let t0 = Instant::now();
            encode::check_output(&out, codec)?;
            if let Some(b) = &bitrate {
                encode::check_bitrate(b)?;
            }
            let comp = spec::load(&spec)?;
            if comp.width % 8 != 0 {
                bail!("width must be a multiple of 8 for video output (got {})", comp.width);
            }
            let first = match start {
                None => 0,
                Some(s) if s.is_finite() && s >= 0.0 && s < comp.duration => time_to_frame(&comp, s),
                Some(s) => bail!("--start must be from 0 to before the video's end at {}s, got {s}", comp.duration),
            };
            let last = match end {
                None => comp.frames,
                Some(e) if e.is_finite() && e <= comp.duration + 1e-9 => {
                    ((e * comp.fps).round() as u32).min(comp.frames)
                }
                Some(e) => bail!("--end must be at most the video's {}s, got {e}", comp.duration),
            };
            if last <= first {
                bail!("--end must come at least a frame after --start");
            }
            let range = first..last;
            let enc = if benchmark { "no encoder (--benchmark)".to_string() } else { encode::pick(codec, &encoder)? };
            let gpu = Gpu::new()?;
            let mut scene = Scene::load(&comp, gpu)?;
            let mut audio = if no_audio { vec![] } else { comp.audio.clone() };
            if !no_audio {
                audio.extend(scene.video_audio.iter().cloned());
            }
            let loaded = t0.elapsed().as_secs_f64();
            let opts = Opts { quality, bitrate };
            if let Some(dir) = out.parent()
                && !dir.as_os_str().is_empty()
            {
                std::fs::create_dir_all(dir)?;
            }
            let child = if benchmark {
                None
            } else {
                let span = (range.start as f64 / comp.fps, range.end as f64 / comp.fps);
                Some(encode::spawn(&out, &comp, &audio, &enc, &opts, span)?)
            };
            let t1 = Instant::now();
            let n = range.len() as u32;
            render_video(&mut scene, child, range)?;
            let secs = t1.elapsed().as_secs_f64();
            println!(
                "{}: {} frames ({:.2}s of video) in {:.2}s ({:.0} fps), plus {:.2}s loading, with {} on {} ({:?})",
                out.display(),
                n,
                n as f64 / comp.fps,
                secs,
                n as f64 / secs,
                loaded,
                enc,
                scene.gpu.info.name,
                scene.gpu.info.backend
            );
            Ok(())
        }
        Cmd::Still { spec, times, out } => {
            let comp = spec::load(&spec)?;
            let mut scene = Scene::load(&comp, Gpu::new()?)?;
            for &t in &times {
                check_time(&comp, t)?;
            }
            let single = times.len() == 1 && out.extension().is_some();
            if !single {
                std::fs::create_dir_all(&out)?;
            }
            for t in times {
                let img = still(&mut scene, time_to_frame(&comp, t))?;
                let path = if single { out.clone() } else { out.join(format!("still_{t}.png")) };
                save(&img, &path)?;
                println!("{}", path.display());
            }
            Ok(())
        }
        Cmd::Sheet { spec, out, count, times, width } => {
            let comp = spec::load(&spec)?;
            let mut scene = Scene::load(&comp, Gpu::new()?)?;
            let times =
                times.unwrap_or_else(|| (0..count).map(|k| (k as f64 + 0.5) * comp.duration / count as f64).collect());
            for &t in &times {
                check_time(&comp, t)?;
            }
            let mut frames = Vec::new();
            for &t in &times {
                frames.push((t, still(&mut scene, time_to_frame(&comp, t))?));
            }
            let img = sheet::compose(&frames, width);
            save(&img, &out)?;
            let list: Vec<String> = times.iter().map(|t| format!("{t:.2}")).collect();
            println!("{}: frames at {}s", out.display(), list.join(", "));
            Ok(())
        }
    }
}

fn check_time(comp: &spec::Comp, t: f64) -> Result<()> {
    if !(t.is_finite() && t >= 0.0 && t <= comp.duration) {
        bail!("time {t} is outside the video, which runs from 0 to {}s", comp.duration);
    }
    Ok(())
}

/// Every audio file holds sound ffmpeg can read. Skipped, with a warning, when ffprobe isn't installed.
fn check_audio(comp: &spec::Comp) -> Result<()> {
    if comp.audio.is_empty() {
        return Ok(());
    }
    if !encode::installed("ffprobe") {
        eprintln!("warning: ffprobe isn't installed, so the audio files weren't checked");
        return Ok(());
    }
    let mut errors = Vec::new();
    for (i, a) in comp.audio.iter().enumerate() {
        match video::has_audio(&a.src) {
            Ok(true) => {}
            Ok(false) => errors.push(format!("audio[{i}].src: {} has no audio stream", a.src.display())),
            Err(e) => errors.push(format!("audio[{i}].src: {e:#}")),
        }
    }
    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }
    Ok(())
}

fn time_to_frame(comp: &spec::Comp, t: f64) -> u32 {
    ((t * comp.fps + 1e-6).floor() as i64).clamp(0, comp.frames as i64 - 1) as u32
}

fn save(img: &image::RgbaImage, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    if ext == "jpg" || ext == "jpeg" {
        let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
        let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 90).encode_image(&rgb)?;
    } else {
        let f = std::io::BufWriter::new(std::fs::File::create(path)?);
        let enc = image::codecs::png::PngEncoder::new_with_quality(
            f,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Adaptive,
        );
        image::ImageEncoder::write_image(
            enc,
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )?;
    }
    Ok(())
}

fn record_frame(scene: &mut Scene, f: u32, out: &wgpu::Buffer, yuv: bool) -> Result<wgpu::CommandEncoder> {
    let fr = scene.frame(f)?;
    scene.gpu.upload(&fr.inst, &fr.uniforms);
    let mut enc = scene.gpu.device.create_command_encoder(&Default::default());
    scene.gpu.record(&mut enc, scene.main, scene.comp.background, &fr.ops);
    scene.gpu.convert(&mut enc, scene.main, out, f, yuv);
    Ok(enc)
}

fn storage(gpu: &Gpu, size: u64) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("converted"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn readback(gpu: &Gpu, size: u64) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn still(scene: &mut Scene, f: u32) -> Result<image::RgbaImage> {
    let (w, h) = (scene.comp.width, scene.comp.height);
    let size = (w * h * 4) as u64;
    let buf = storage(&scene.gpu, size);
    let rb = readback(&scene.gpu, size);
    let mut enc = record_frame(scene, f, &buf, false)?;
    enc.copy_buffer_to_buffer(&buf, 0, &rb, 0, size);
    let idx = scene.gpu.queue.submit([enc.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    rb.map_async(wgpu::MapMode::Read, .., move |r| {
        let _ = tx.send(r);
    });
    scene.gpu.device.poll(wgpu::PollType::Wait { submission_index: Some(idx), timeout: None })?;
    rx.recv()
        .map_err(|_| anyhow!("the GPU never returned the frame"))?
        .map_err(|e| anyhow!("reading the frame back from the GPU failed: {e}"))?;
    let data = rb.get_mapped_range(..)?.to_vec();
    Ok(image::RgbaImage::from_raw(w, h, data).unwrap())
}

/// Render every frame into ffmpeg's stdin (or nowhere, with no ffmpeg, to time rendering alone), keeping
/// several frames in flight so the GPU, the copy back, and the encoder all work at once.
fn render_video(scene: &mut Scene, mut child: Option<std::process::Child>, frames: std::ops::Range<u32>) -> Result<()> {
    const RING: usize = 4;
    let (w, h) = (scene.comp.width as u64, scene.comp.height as u64);
    let size = w * h * 3 / 2;
    let conv = storage(&scene.gpu, size);
    let slots: Vec<wgpu::Buffer> = (0..RING).map(|_| readback(&scene.gpu, size)).collect();

    let mut stdin: Box<dyn Write + Send> = match child.as_mut() {
        Some(c) => Box::new(c.stdin.take().unwrap()),
        None => Box::new(std::io::sink()),
    };
    let mut stderr = child.as_mut().and_then(|c| c.stderr.take());
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let (tx, rx) = sync_channel::<Vec<u8>>(RING);
    let (back_tx, back_rx) = sync_channel::<Vec<u8>>(RING + 2);
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        for buf in rx {
            stdin.write_all(&buf)?;
            let _ = back_tx.try_send(buf);
        }
        Ok(())
    });

    let mut pending: VecDeque<(usize, wgpu::SubmissionIndex)> = VecDeque::new();
    let drain = |scene: &Scene, pending: &mut VecDeque<(usize, wgpu::SubmissionIndex)>| -> Result<bool> {
        let (slot, idx) = pending.pop_front().unwrap();
        scene.gpu.device.poll(wgpu::PollType::Wait { submission_index: Some(idx), timeout: None })?;
        let mut buf = back_rx.try_recv().unwrap_or_else(|_| vec![0u8; size as usize]);
        buf.copy_from_slice(&slots[slot].get_mapped_range(..)?);
        slots[slot].unmap();
        Ok(tx.send(buf).is_ok())
    };

    let mut ok = true;
    for f in frames {
        let slot = f as usize % RING;
        if pending.len() == RING {
            ok = drain(scene, &mut pending)?;
            if !ok {
                break;
            }
        }
        let mut enc = record_frame(scene, f, &conv, true)?;
        enc.copy_buffer_to_buffer(&conv, 0, &slots[slot], 0, size);
        let idx = scene.gpu.queue.submit([enc.finish()]);
        // a failed map shows up in drain's get_mapped_range; one still pending when ffmpeg quits early is
        // cancelled when the buffers drop, which is fine
        slots[slot].map_async(wgpu::MapMode::Read, .., |_| {});
        pending.push_back((slot, idx));
    }
    while ok && !pending.is_empty() {
        ok = drain(scene, &mut pending)?;
    }
    drop(tx);
    let wrote = writer.join().unwrap();
    let status = match child.as_mut() {
        Some(c) => Some(c.wait()?),
        None => None,
    };
    let err = err_thread.join().unwrap_or_default();
    if status.is_some_and(|s| !s.success()) || wrote.is_err() {
        let err = err.trim();
        bail!("ffmpeg failed: {}", if err.is_empty() { "it stopped without saying why" } else { err });
    }
    Ok(())
}
