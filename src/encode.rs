//! Encoding through ffmpeg: frames arrive as raw YUV 4:2:0 on stdin, already converted on the GPU, and the
//! fastest encoder the machine has turns them into a file.

use crate::spec::Comp;
use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use std::path::Path;
use std::process::{Child, Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum Codec {
    H264,
    Hevc,
    Prores,
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum Quality {
    Draft,
    Standard,
    High,
}

#[derive(Clone, Debug)]
pub struct Opts {
    pub quality: Quality,
    pub bitrate: Option<String>,
}

fn encoders() -> Result<String> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .context("can't run ffmpeg; install it (https://ffmpeg.org)")?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether an encoder actually works here (a listed NVENC encoder still needs an NVIDIA GPU).
fn works(name: &str) -> bool {
    Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=s=256x256:d=0.1",
            "-frames:v",
            "2",
            "-c:v",
            name,
            "-f",
            "null",
            "-",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn pick(codec: Codec, pref: &str) -> Result<String> {
    let list = encoders()?;
    let has = |n: &str| list.split_whitespace().any(|w| w == n);
    let (hw, sw): (&[&str], &str) = match codec {
        Codec::H264 => (&["h264_videotoolbox", "h264_nvenc", "h264_qsv", "h264_vaapi"], "libx264"),
        Codec::Hevc => (&["hevc_videotoolbox", "hevc_nvenc", "hevc_qsv", "hevc_vaapi"], "libx265"),
        Codec::Prores => (&["prores_videotoolbox"], "prores_ks"),
    };
    match pref {
        "software" => {
            if !has(sw) {
                bail!("ffmpeg here has no {sw} encoder");
            }
            Ok(sw.to_string())
        }
        "auto" | "hardware" => {
            for n in hw {
                // VAAPI needs a device and filter setup this tool doesn't do; skip it in auto
                if has(n) && !n.ends_with("vaapi") && works(n) {
                    return Ok(n.to_string());
                }
            }
            if pref == "hardware" {
                bail!("no working hardware {codec:?} encoder found");
            }
            if !has(sw) {
                bail!("ffmpeg here has no {codec:?} encoder");
            }
            Ok(sw.to_string())
        }
        name => {
            if !has(name) {
                bail!("ffmpeg here has no encoder named {name}");
            }
            Ok(name.to_string())
        }
    }
}

fn video_args(enc: &str, o: &Opts, comp: &Comp) -> Vec<String> {
    let bpp = match o.quality {
        Quality::Draft => 0.05,
        Quality::Standard => 0.16,
        Quality::High => 0.3,
    };
    let bitrate = o.bitrate.clone().unwrap_or_else(|| {
        let bps = comp.width as f64 * comp.height as f64 * comp.fps * bpp;
        format!("{}k", (bps / 1000.0).round() as u64)
    });
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let mut a = s(&["-c:v", enc]);
    match enc {
        "libx264" | "libx265" => {
            let (preset, crf) = match o.quality {
                Quality::Draft => ("ultrafast", if enc == "libx264" { 28 } else { 30 }),
                Quality::Standard => ("veryfast", if enc == "libx264" { 20 } else { 23 }),
                Quality::High => ("medium", if enc == "libx264" { 17 } else { 20 }),
            };
            a.extend(s(&["-preset", preset]));
            match &o.bitrate {
                Some(b) => a.extend(s(&["-b:v", b])),
                None => a.extend(s(&["-crf", &crf.to_string()])),
            }
            if enc == "libx265" {
                a.extend(s(&["-tag:v", "hvc1", "-x265-params", "log-level=error"]));
            }
        }
        "prores_ks" => a.extend(s(&["-profile:v", if o.quality == Quality::Draft { "1" } else { "3" }])),
        "prores_videotoolbox" => a.extend(s(&["-profile:v", if o.quality == Quality::Draft { "lt" } else { "hq" }])),
        e if e.contains("nvenc") => {
            a.extend(s(&["-preset", if o.quality == Quality::High { "p5" } else { "p3" }, "-b:v", &bitrate]));
        }
        e => {
            a.extend(s(&["-b:v", &bitrate]));
            if e.contains("videotoolbox") {
                // same quality and size here, a fifth faster
                a.extend(s(&["-allow_sw", "1", "-prio_speed", "1"]));
            }
            if e.starts_with("hevc") {
                a.extend(s(&["-tag:v", "hvc1"]));
            }
        }
    }
    if !enc.starts_with("prores") {
        a.extend(s(&["-g", &((comp.fps * 2.0).round() as u32).to_string()]));
    }
    a
}

/// Start ffmpeg reading raw frames from stdin.
pub fn spawn(out: &Path, comp: &Comp, enc: &str, o: &Opts) -> Result<Child> {
    let duration = comp.duration;
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-nostdin"]);
    cmd.args([
        "-f",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        "-color_range",
        "tv",
        "-colorspace",
        "bt709",
        "-color_primaries",
        "bt709",
        "-color_trc",
        "bt709",
    ]);
    cmd.args(["-s", &format!("{}x{}", comp.width, comp.height), "-r", &comp.fps.to_string(), "-i", "pipe:0"]);
    cmd.args(video_args(enc, o, comp));
    cmd.args(["-t", &duration.to_string()]);
    let ext = out.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "mp4" || ext == "mov" || ext == "m4v" {
        cmd.args(["-movflags", "+faststart"]);
    }
    cmd.arg(out);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    cmd.spawn().context("can't start ffmpeg; install it (https://ffmpeg.org)")
}
