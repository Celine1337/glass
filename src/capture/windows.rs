//! Windows capture: FFmpeg Desktop Duplication (`ddagrab`) with a `gdigrab`
//! fallback. FFmpeg is bundled next to `glass.exe` (CI downloads a static
//! build); we locate it relative to the executable, falling back to PATH.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::capture::{Backend, Recording};
use crate::pipeline::{self, CaptureSpec};

/// Desktop Duplication by default; `GLASS_WINDOWS_BACKEND=gdigrab` to fall back.
pub fn backend() -> &'static str {
    if std::env::var("GLASS_WINDOWS_BACKEND")
        .map(|v| v.eq_ignore_ascii_case("gdigrab"))
        .unwrap_or(false)
    {
        "gdigrab"
    } else {
        "ddagrab"
    }
}

/// Prefer a bundled `ffmpeg.exe` beside the executable, else PATH.
fn ffmpeg_program() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let bundled = dir.join("ffmpeg.exe");
            if bundled.exists() {
                return bundled;
            }
        }
    }
    PathBuf::from("ffmpeg")
}

/// Spawn FFmpeg to capture the desktop at native resolution + refresh rate.
pub fn start(spec: CaptureSpec, out: PathBuf, encoder: String) -> Result<Recording, String> {
    let args = pipeline::ffmpeg_windows_args(spec, &encoder, &out, backend());
    let mut cmd = Command::new(ffmpeg_program());
    cmd.args(&args[1..]);
    // stdin stays piped so we can send `q` for a clean finalize on stop.
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not start ffmpeg: {e}"))?;
    Ok(Recording::new(child, out, Backend::Ffmpeg))
}