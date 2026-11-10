//! Video layers: frames decoded by an ffmpeg process per clip and read ahead on a thread. Frame k of the layer
//! shows the source frame on screen at source time trim + k / fps, worked out from the source's own frame
//! timestamps, so a frame is the same whether decoding started at it, before it, or anywhere else.

use anyhow::{Context, Result, anyhow, bail};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, sync_channel};

pub struct Info {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub has_audio: bool,
    pub has_alpha: bool,
    /// The file's start time, which ffmpeg's -ss counts from.
    start: f64,
    /// Presentation times of the first video stream's frames, in order, in the file's own seconds.
    times: Vec<f64>,
}

fn ffprobe(src: &Path, args: &[&str], format: &str) -> Result<Vec<u8>> {
    let out = Command::new(crate::encode::tool("ffprobe"))
        .args(["-v", "error"])
        .args(args)
        .args(["-of", format])
        .arg(src)
        .output()
        .with_context(|| crate::encode::missing("ffprobe"))?;
    if !out.status.success() {
        bail!("ffprobe can't read {}: {}", src.display(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

/// Whether a file holds an audio stream ffmpeg can read.
pub fn has_audio(src: &Path) -> Result<bool> {
    let j: serde_json::Value = serde_json::from_slice(&ffprobe(src, &["-show_entries", "stream=codec_type"], "json")?)?;
    Ok(j["streams"].as_array().is_some_and(|s| s.iter().any(|s| s["codec_type"] == "audio")))
}

pub fn probe(src: &Path) -> Result<Info> {
    let entries = "stream=codec_type,width,height,pix_fmt,avg_frame_rate,start_time:format=duration,start_time";
    let j: serde_json::Value = serde_json::from_slice(&ffprobe(src, &["-show_entries", entries], "json")?)?;
    let streams = j["streams"].as_array().cloned().unwrap_or_default();
    let v = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or_else(|| anyhow!("{} has no video stream", src.display()))?;
    let num = |x: &serde_json::Value| x.as_str().and_then(|d| d.parse::<f64>().ok()).filter(|d| d.is_finite());
    let pix = v["pix_fmt"].as_str().unwrap_or("");
    let duration = num(&j["format"]["duration"]).unwrap_or(0.0);
    let start = num(&j["format"]["start_time"]).unwrap_or(0.0);

    // every frame's time, from the packets (no decoding); packets flagged D are dropped by the decoder
    let csv = ffprobe(src, &["-select_streams", "v:0", "-show_entries", "packet=pts_time,flags"], "csv=p=0")?;
    let mut times = Vec::new();
    let mut complete = true;
    for line in String::from_utf8_lossy(&csv).lines() {
        let (pts, flags) = line.split_once(',').unwrap_or((line, ""));
        if flags.contains('D') {
            continue;
        }
        match pts.trim().parse::<f64>() {
            Ok(t) if t.is_finite() => times.push(t),
            _ => complete = false,
        }
    }
    if !complete || times.is_empty() {
        // no timestamps in the packets: assume a constant frame rate
        let rate = v["avg_frame_rate"]
            .as_str()
            .and_then(|r| r.split_once('/'))
            .and_then(|(a, b)| Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?))
            .filter(|r| r.is_finite() && *r > 0.0)
            .unwrap_or(30.0);
        let first = num(&v["start_time"]).unwrap_or(start);
        let n = ((duration - (first - start)) * rate).round().max(1.0) as usize;
        times = (0..n).map(|k| first + k as f64 / rate).collect();
    }
    times.sort_by(f64::total_cmp);
    times.dedup();
    Ok(Info {
        width: v["width"].as_u64().unwrap_or(0) as u32,
        height: v["height"].as_u64().unwrap_or(0) as u32,
        duration,
        has_audio: streams.iter().any(|s| s["codec_type"] == "audio"),
        has_alpha: pix.contains('a')
            && (pix.starts_with("yuva")
                || pix.contains("rgba")
                || pix.contains("argb")
                || pix.contains("bgra")
                || pix.starts_with("gbrap")),
        start,
        times,
    })
}

/// The option that makes ffmpeg pass every decoded frame through as it is, without duplicating or dropping
/// any to fit a frame rate: `-fps_mode` since ffmpeg 5.1, `-vsync` before.
fn passthrough() -> &'static str {
    static FLAG: OnceLock<&str> = OnceLock::new();
    FLAG.get_or_init(|| {
        let new = Command::new(crate::encode::tool("ffmpeg"))
            .args(["-v", "error", "-f", "lavfi", "-i", "nullsrc=s=16x16:d=0.04", "-fps_mode", "passthrough"])
            .args(["-f", "null", "-"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if new { "-fps_mode" } else { "-vsync" }
    })
}

pub struct Decoder {
    src: PathBuf,
    trim: f64,
    fps: f64,
    w: u32,
    h: u32,
    looped: bool,
    /// Layer frames before the clip ends; a looped clip repeats every this many.
    frames: i64,
    alpha: bool,
    start: f64,
    times: Vec<f64>,
    child: Option<Child>,
    rx: Option<Receiver<Vec<u8>>>,
    /// The source frame the pipe delivers next.
    next: usize,
    /// The source frame in the layer's texture.
    current: Option<usize>,
    last: Option<Vec<u8>>,
}

impl Decoder {
    pub fn new(src: PathBuf, trim: f64, fps: f64, w: u32, h: u32, looped: bool, info: &Info) -> Decoder {
        let frames = (((info.duration - trim) * fps).floor() as i64).max(1);
        Decoder {
            src,
            trim,
            fps,
            w,
            h,
            looped,
            frames,
            alpha: info.has_alpha,
            start: info.start,
            times: info.times.clone(),
            child: None,
            rx: None,
            next: 0,
            current: None,
            last: None,
        }
    }

    /// The source frame on screen at layer frame `idx`: the last one that starts at or before its time.
    fn source_frame(&self, idx: i64) -> usize {
        let k = if self.looped { idx.rem_euclid(self.frames) } else { idx.clamp(0, self.frames - 1) };
        let at = self.start + self.trim + k as f64 / self.fps + 1e-6;
        self.times.partition_point(|&t| t <= at).saturating_sub(1)
    }

    /// Start decoding at source frame `j`. ffmpeg keeps the frames from the seek time on, so seeking to between
    /// frame j - 1 and frame j makes frame j the first out.
    fn start(&mut self, j: usize) -> Result<()> {
        self.stop();
        let seek = if j == 0 { 0.0 } else { (self.times[j - 1] + self.times[j]) / 2.0 - self.start };
        let mut vf = format!("scale={}:{}:flags=bicubic", self.w, self.h);
        if self.alpha {
            vf += ",premultiply=inplace=1";
        }
        let mut child = Command::new(crate::encode::tool("ffmpeg"))
            .args(["-v", "error", "-nostdin", "-ss"])
            .arg(format!("{:.6}", seek.max(0.0)))
            .arg("-i")
            .arg(&self.src)
            .args(["-map", "0:v:0", "-an", "-vf", &vf, passthrough(), "passthrough"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| crate::encode::missing("ffmpeg"))?;
        let mut out = child.stdout.take().unwrap();
        let size = (self.w * self.h * 4) as usize;
        let (tx, rx) = sync_channel(6);
        std::thread::spawn(move || {
            loop {
                let mut buf = vec![0u8; size];
                if out.read_exact(&mut buf).is_err() || tx.send(buf).is_err() {
                    break;
                }
            }
        });
        self.child = Some(child);
        self.rx = Some(rx);
        self.next = j;
        Ok(())
    }

    fn stop(&mut self) {
        self.rx = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// The pixels for layer frame `idx`, or None when the texture already holds them.
    pub fn frame(&mut self, idx: i64) -> Result<Option<Vec<u8>>> {
        let want = self.source_frame(idx);
        if self.current == Some(want) {
            return Ok(None);
        }
        // read forward up to two seconds; seek for anything further or earlier
        let near = self.next <= want && self.times[want] - self.times[self.next] <= 2.0;
        if self.rx.is_none() || !near {
            self.start(want)?;
        }
        while self.next <= want {
            match self.rx.as_ref().unwrap().recv() {
                Ok(buf) => {
                    self.last = Some(buf);
                    self.next += 1;
                }
                Err(_) => break, // the clip ended early: hold its last frame
            }
        }
        self.current = Some(want);
        Ok(self.last.clone())
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.stop();
    }
}
