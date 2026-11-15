//! Windows capture: FFmpeg `gdigrab` by default (BGRA frames every encoder
//! accepts), with `ddagrab` (Desktop Duplication, zero-copy D3D11) opt-in via
//! `GLASS_WINDOWS_BACKEND=ddagrab`. FFmpeg is bundled next to `glass.exe`
//! (CI downloads a static build); we locate it relative to the executable,
//! falling back to PATH.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::capture::{Backend, Recording};
use crate::pipeline::{self, CaptureSpec};

/// `gdigrab` by default; `GLASS_WINDOWS_BACKEND=ddagrab` for the GPU path.
pub fn backend() -> &'static str {
    pipeline::windows_backend_for(std::env::var("GLASS_WINDOWS_BACKEND").ok().as_deref())
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
    let log = out.with_extension("log");
    let stderr = std::fs::File::create(&log)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());
    let mut cmd = Command::new(ffmpeg_program());
    cmd.args(&args[1..]);
    // stdin stays piped so we can send `q` for a clean finalize on stop.
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(stderr);
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not start ffmpeg: {e}"))?;
    Ok(Recording::new(child, out, Backend::Ffmpeg, log))
}
