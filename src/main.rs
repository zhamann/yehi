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

use anyhow::{Result, bail};
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
        /// Output file (.mp4, .mov, .mkv, .webm).
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
        #[arg(long, default_value_t = 12)]
        count: usize,
        /// Exact times in seconds instead of evenly spaced ones.
        #[arg(long, num_args = 1.., value_delimiter = ',')]
        times: Option<Vec<f64>>,
        /// Width of each frame in the sheet.
        #[arg(long, default_value_t = 270)]
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
            Ok(())
        }
        Cmd::Video { spec, out, codec, encoder, quality, bitrate, no_audio, start, end, benchmark } => {
            let t0 = Instant::now();
            let comp = spec::load(&spec)?;
            if comp.width % 8 != 0 {
                bail!("width must be a multiple of 8 for video output (got {})", comp.width);
            }
            let first = start.map_or(0, |s| time_to_frame(&comp, s));
            let last = end.map_or(comp.frames, |e| ((e * comp.fps).round() as u32).min(comp.frames));
            if last <= first {
                bail!("--end must come after --start, inside the video's {}s", comp.duration);
            }
            let range = first..last;
            let enc = encode::pick(codec, &encoder)?;
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
                std::process::Command::new("sh")
                    .args(["-c", "cat > /dev/null"])
                    .stdin(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()?
            } else {
                let span = (range.start as f64 / comp.fps, range.end as f64 / comp.fps);
                encode::spawn(&out, &comp, &audio, &enc, &opts, span)?
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
    rb.map_async(wgpu::MapMode::Read, .., |r| r.expect("map failed"));
    scene.gpu.device.poll(wgpu::PollType::Wait { submission_index: Some(idx), timeout: None })?;
    let data = rb.get_mapped_range(..)?.to_vec();
    Ok(image::RgbaImage::from_raw(w, h, data).unwrap())
}

/// Render every frame into ffmpeg's stdin, keeping several frames in flight so the GPU, the copy back,
/// and the encoder all work at once.
fn render_video(scene: &mut Scene, mut child: std::process::Child, frames: std::ops::Range<u32>) -> Result<()> {
    const RING: usize = 4;
    let (w, h) = (scene.comp.width as u64, scene.comp.height as u64);
    let size = w * h * 3 / 2;
    let conv = storage(&scene.gpu, size);
    let slots: Vec<wgpu::Buffer> = (0..RING).map(|_| readback(&scene.gpu, size)).collect();

    let mut stdin = child.stdin.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
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
        slots[slot].map_async(wgpu::MapMode::Read, .., |r| r.expect("map failed"));
        pending.push_back((slot, idx));
    }
    while ok && !pending.is_empty() {
        ok = drain(scene, &mut pending)?;
    }
    drop(tx);
    let wrote = writer.join().unwrap();
    let status = child.wait()?;
    let err = err_thread.join().unwrap_or_default();
    if !status.success() || wrote.is_err() {
        bail!("ffmpeg failed: {}", err.trim());
    }
    Ok(())
}
