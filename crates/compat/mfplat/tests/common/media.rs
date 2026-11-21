//! Synthetic inputs for the integration tests. Install FFmpeg with libx264 or
//! set TOMBOX_FFMPEG to an executable with that encoder.

#![allow(dead_code)]

use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

fn ffmpeg(args: &[&str]) -> Vec<u8> {
    let executable = std::env::var_os("TOMBOX_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    let output = Command::new(&executable)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(args)
        .arg("pipe:1")
        .output()
        .unwrap_or_else(|err| panic!("run {executable:?}: {err}. Install FFmpeg or set TOMBOX_FFMPEG"));
    assert!(
        output.status.success(),
        "generate media fixture with {executable:?}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(!output.stdout.is_empty(), "{executable:?} generated an empty media fixture");
    output.stdout
}

pub fn silent_aac() -> Vec<u8> {
    ffmpeg(&[
        "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo", "-t", "1",
        "-c:a", "aac", "-profile:a", "aac_low", "-b:a", "96k",
        "-movflags", "empty_moov+default_base_moof+frag_keyframe", "-f", "mp4",
    ])
}

pub fn black_h264() -> &'static [u8] {
    static STREAM: OnceLock<Vec<u8>> = OnceLock::new();
    STREAM.get_or_init(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/black_640x360_30f.h264");
        match std::fs::read(&path) {
            Ok(bytes) => return bytes,
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => panic!("read {}: {err}", path.display()),
        }
        ffmpeg(&[
            "-f", "lavfi", "-i", "color=black:s=640x360:r=30", "-frames:v", "30",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-profile:v", "baseline",
            "-x264-params", "keyint=30:scenecut=0", "-f", "h264",
        ])
    })
}

pub fn patterned_h264() -> Vec<u8> {
    ffmpeg(&[
        "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30", "-frames:v", "30",
        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-profile:v", "baseline",
        "-x264-params", "keyint=30:scenecut=0",
        "-movflags", "empty_moov+default_base_moof+frag_keyframe", "-f", "mp4",
    ])
}

pub fn staged_fragment(init: &str, segment: &str) -> Option<Vec<u8>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../compat-common/backend_data/fortnite/video");
    match (std::fs::read(root.join(init)), std::fs::read(root.join(segment))) {
        (Ok(mut clip), Ok(rest)) => {
            clip.extend(rest);
            Some(clip)
        }
        (Err(init_err), Err(seg_err))
            if init_err.kind() == ErrorKind::NotFound && seg_err.kind() == ErrorKind::NotFound =>
        {
            None
        }
        (Err(err), _) => panic!("read {}: {err}", root.join(init).display()),
        (_, Err(err)) => panic!("read {}: {err}", root.join(segment).display()),
    }
}
