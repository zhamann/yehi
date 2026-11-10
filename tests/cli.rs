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
