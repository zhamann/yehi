//! End-to-end: the binary on the bundled example. Needs a GPU or a software Vulkan driver, and ffmpeg.

use std::path::PathBuf;
use std::process::Command;

fn yehi(args: &[&str]) -> String {
    let out =
        Command::new(env!("CARGO_BIN_EXE_yehi")).args(args).current_dir(env!("CARGO_MANIFEST_DIR")).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "yehi {args:?} failed:\n{text}");
    text
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yehi-test-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

#[test]
fn example_checks() {
    assert!(yehi(&["check", "examples/hello.json"]).contains("270 frames"));
}

#[test]
fn still_draws_the_frame() {
    let p = tmp("still.png");
    yehi(&["still", "examples/hello.json", "2", "-o", p.to_str().unwrap()]);
    let img = image::open(&p).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (1080, 1920));
    // the amber block behind "OF TIME" sits at (540, 1100); away from its letters it's amber
    let px = img.get_pixel(300, 1100);
    assert!(px[0] > 180 && px[1] > 130 && px[2] < 150, "expected the amber block, got {px:?}");
    // the sky above is dark
    let sky = img.get_pixel(540, 200);
    assert!(sky[0] < 80 && sky[2] < 90, "expected dark sky, got {sky:?}");
}

#[test]
fn sheet_and_video() {
    let s = tmp("sheet.jpg");
    yehi(&["sheet", "examples/hello.json", "-o", s.to_str().unwrap(), "--count", "4"]);
    assert!(image::open(&s).is_ok());
    let v = tmp("part.mp4");
    let out = yehi(&[
        "video",
        "examples/hello.json",
        "-o",
        v.to_str().unwrap(),
        "--start",
        "1",
        "--end",
        "2",
        "--quality",
        "draft",
    ]);
    assert!(out.contains("30 frames"), "{out}");
    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-count_frames", "-show_entries", "stream=nb_read_frames,width,height", "-of", "csv=p=0"])
        .arg(&v)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&probe.stdout).trim(), "1080,1920,30");
}

#[test]
fn errors_point_at_the_field() {
    let p = tmp("bad.json");
    std::fs::write(&p, r#"{"width": 64, "height": 64, "fps": 30, "duration": 1, "layers": [{"type": "rect", "size": [10, 10], "opacity": "sin(t"}]}"#).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_yehi")).args(["check", p.to_str().unwrap()]).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("layers[0].opacity"), "{err}");
}

/// A 24 fps clip in a 30 fps video: each frame shows the source frame on screen at its time, the same whether
/// it's rendered alone, after earlier frames, or after later ones.
#[test]
fn video_frames_are_independent_of_order() {
    let dir = tmp("independent");
    std::fs::create_dir_all(&dir).unwrap();
    // each source frame's brightness encodes its number
    let made = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "nullsrc=s=64x64:r=24:d=2,geq=lum='16+N*4':cb=128:cr=128"])
        .args(["-c:v", "libx264", "-qp", "0", "-g", "12", "-bf", "2", "-pix_fmt", "yuv420p"])
        .arg(dir.join("clip.mp4"))
        .status()
        .unwrap();
    assert!(made.success());
    let spec = dir.join("v.json");
    std::fs::write(
        &spec,
        r#"{"width": 64, "height": 64, "fps": 30, "duration": 2, "layers": [{"type": "video", "src": "clip.mp4", "size": [64, 64]}]}"#,
    )
    .unwrap();
    let times: Vec<String> = (0..60).map(|k| format!("{}", k as f64 / 30.0)).collect();
    let spec = spec.to_str().unwrap();
    let render = |out: &str, ts: &[&String]| {
        let mut args = vec!["still", spec];
        args.extend(ts.iter().map(|t| t.as_str()));
        let out = dir.join(out);
        args.extend(["-o", out.to_str().unwrap()]);
        yehi(&args);
    };
    render("forward", &times.iter().collect::<Vec<_>>());
    render("reverse", &times.iter().rev().collect::<Vec<_>>());
    for k in [7, 31, 0, 44, 13] {
        render(&format!("alone{k}"), &[&times[k]]);
    }
    let shade = |p: PathBuf| image::open(&p).unwrap().to_luma8().get_pixel(32, 32)[0];
    for (k, t) in times.iter().enumerate() {
        let name = format!("still_{t}.png");
        let fwd = shade(dir.join("forward").join(&name));
        assert_eq!(fwd, shade(dir.join("reverse").join(&name)), "frame {k} differs in reverse");
        if [7, 31, 0, 44, 13].contains(&k) {
            assert_eq!(fwd, shade(dir.join(format!("alone{k}")).join(&name)), "frame {k} differs alone");
        }
        // the shade back to the source frame number: 16 + 4n in limited-range luma
        let n = (fwd as f64 * 219.0 / 255.0 / 4.0).round() as i64;
        assert_eq!(n, (k as f64 / 30.0 * 24.0 + 1e-6).floor() as i64, "frame {k} shows the wrong source frame");
    }
}

#[test]
fn bad_arguments_fail_cleanly() {
    for args in [
        &["video", "examples/hello.json", "-o", "x.webm"][..],
        &["video", "examples/hello.json", "-o", "x.mp4", "--codec", "prores"],
        &["video", "examples/hello.json", "-o", "x.mp4", "--bitrate", "banana"],
        &["video", "examples/hello.json", "-o", "x.mp4", "--start", "99"],
        &["video", "examples/hello.json", "-o", "x.mp4", "--start", "NaN"],
        &["still", "examples/hello.json", "99", "-o", "x.png"],
        &["sheet", "examples/hello.json", "-o", "x.jpg", "--count", "0"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_yehi"))
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(if args[0] == "sheet" { 2 } else { 1 }), "{args:?}: {err}");
        assert!(!err.contains("panicked"), "{args:?}: {err}");
    }
}

/// A video's sound plays only while its layer and the group around it are on screen.
#[test]
fn video_sound_stops_with_its_group() {
    let dir = tmp("sound");
    std::fs::create_dir_all(&dir).unwrap();
    let made = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=64x64:r=30:d=2", "-f", "lavfi", "-i"])
        .args(["sine=f=440:d=2", "-shortest", "-c:v", "libx264", "-c:a", "aac"])
        .arg(dir.join("av.mp4"))
        .status()
        .unwrap();
    assert!(made.success());
    let spec = dir.join("g.json");
    std::fs::write(
        &spec,
        r#"{"width": 64, "height": 64, "fps": 30, "duration": 2, "layers": [{"type": "group", "start": 0.25, "duration": 0.5,
            "layers": [{"type": "video", "src": "av.mp4", "size": [64, 64], "start": -0.1}]}]}"#,
    )
    .unwrap();
    let out = dir.join("g.mp4");
    yehi(&["video", spec.to_str().unwrap(), "-o", out.to_str().unwrap()]);
    let pcm = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&out)
        .args(["-map", "0:a", "-ac", "1", "-ar", "1000", "-f", "s16le", "-"])
        .output()
        .unwrap()
        .stdout;
    let samples: Vec<i16> = pcm.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
    let loud = |from: usize, to: usize| samples[from..to].iter().map(|s| s.unsigned_abs()).max().unwrap() > 500;
    assert!(!loud(0, 200), "sound before the group starts");
    assert!(loud(300, 700), "no sound while the group is on screen");
    assert!(!loud(800, 2000), "sound after the group ends");
}
