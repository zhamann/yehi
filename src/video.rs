//! Video layers: frames decoded by an ffmpeg process per clip, read ahead on a thread, resampled to the
//! composition's frame rate so frame k of the layer is source time trim + k / fps.

use anyhow::{Context, Result, anyhow, bail};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, sync_channel};

pub struct Info {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub has_audio: bool,
    pub has_alpha: bool,
}

pub fn probe(src: &PathBuf) -> Result<Info> {
    let out = Command::new(crate::encode::tool("ffprobe"))
        .args(["-v", "error", "-show_entries", "stream=codec_type,width,height,pix_fmt:format=duration", "-of", "json"])
        .arg(src)
        .output()
        .context(crate::encode::MISSING)?;
    if !out.status.success() {
        bail!("ffprobe can't read {}: {}", src.display(), String::from_utf8_lossy(&out.stderr).trim());
    }
    let j: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let streams = j["streams"].as_array().cloned().unwrap_or_default();
    let v = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or_else(|| anyhow!("{} has no video stream", src.display()))?;
    let pix = v["pix_fmt"].as_str().unwrap_or("");
    Ok(Info {
        width: v["width"].as_u64().unwrap_or(0) as u32,
        height: v["height"].as_u64().unwrap_or(0) as u32,
        duration: j["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0),
        has_audio: streams.iter().any(|s| s["codec_type"] == "audio"),
        has_alpha: pix.contains('a')
            && (pix.starts_with("yuva")
                || pix.contains("rgba")
                || pix.contains("argb")
                || pix.contains("bgra")
                || pix.starts_with("gbrap")),
    })
}

pub struct Decoder {
    src: PathBuf,
    trim: f64,
    fps: f64,
    w: u32,
    h: u32,
    looped: bool,
    frames: i64,
    alpha: bool,
    child: Option<Child>,
    rx: Option<Receiver<Vec<u8>>>,
    next: i64,
    current: i64,
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
            child: None,
            rx: None,
            next: 0,
            current: -1,
            last: None,
        }
    }

    fn start(&mut self, at: i64) -> Result<()> {
        self.stop();
        let mut vf = format!("fps={},scale={}:{}:flags=bicubic", self.fps, self.w, self.h);
        if self.alpha {
            vf += ",premultiply=inplace=1";
        }
        let mut child = Command::new(crate::encode::tool("ffmpeg"))
            .args(["-v", "error", "-nostdin", "-ss"])
            .arg(format!("{:.6}", self.trim + at as f64 / self.fps))
            .arg("-i")
            .arg(&self.src)
            .args(["-an", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context(crate::encode::MISSING)?;
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
        self.next = at;
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
        let want = if self.looped { idx.rem_euclid(self.frames) } else { idx.clamp(0, self.frames - 1) };
        if want == self.current {
            return Ok(None);
        }
        if self.rx.is_none() || want < self.next || want > self.next + (self.fps * 2.0) as i64 {
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
        self.current = want;
        Ok(self.last.clone())
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.stop();
    }
}
