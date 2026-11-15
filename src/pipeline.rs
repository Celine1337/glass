//! Pure command-line builders for the native capture subprocess.
//!
//! Glass is an orchestrator: it never decodes or encodes a frame itself. These
//! functions only assemble the `argv` for `gst-launch-1.0` (Linux) or `ffmpeg`
//! (Windows). Keeping them pure makes the exact pipeline unit-testable without
//! a display, a portal, or a real screen.

use std::path::Path;

/// Native capture target: full resolution + monitor refresh rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureSpec {
    pub width: u32,
    pub height: u32,
    pub hz: u32,
}

impl CaptureSpec {
    pub fn new(width: u32, height: u32, hz: u32) -> Self {
        Self { width, height, hz }
    }
}

/// The child fd the PipeWire remote is mapped to (see `capture::linux`).
pub const PIPEWIRE_CHILD_FD: i32 = 3;

/// GStreamer pipeline for Wayland: portal PipeWire node -> H.264 -> MP4.
///
/// `gst-launch-1.0 -e` sends EOS on SIGINT so `mp4mux` finalizes the file.
/// `h264parse` is inserted only when available (it converts byte-stream to AVC
/// for some hardware encoders); `x264enc ! mp4mux` also works without it.
pub fn gst_wayland_args(
    node: u32,
    fd: i32,
    spec: CaptureSpec,
    encoder: &str,
    out: &Path,
    h264parse: bool,
) -> Vec<String> {
    let mut a = vec!["gst-launch-1.0".to_string(), "-e".to_string()];
    a.push(format!("pipewiresrc fd={fd} path={node}"));
    a.push("!".to_string());
    a.push(format!("video/x-raw,framerate={}/1", spec.hz));
    gst_tail(&mut a, encoder, h264parse, out);
    a
}

/// GStreamer pipeline for X11: full-screen `ximagesrc` -> H.264 -> MP4.
pub fn gst_x11_args(
    spec: CaptureSpec,
    encoder: &str,
    out: &Path,
    h264parse: bool,
) -> Vec<String> {
    let mut a = vec!["gst-launch-1.0".to_string(), "-e".to_string()];
    a.push("ximagesrc use-damage=false".to_string());
    a.push("!".to_string());
    a.push(format!("video/x-raw,framerate={}/1", spec.hz));
    gst_tail(&mut a, encoder, h264parse, out);
    a
}

/// Shared encode -> (parse) -> mux -> sink tail of a GStreamer pipeline.
fn gst_tail(a: &mut Vec<String>, encoder: &str, h264parse: bool, out: &Path) {
    a.push("!".to_string());
    a.push("videoconvert".to_string());
    a.push("!".to_string());
    a.extend(gst_encoder_elements(encoder));
    if h264parse {
        a.push("!".to_string());
        a.push("h264parse".to_string());
    }
    a.push("!".to_string());
    a.push("mp4mux".to_string());
    a.push("!".to_string());
    a.push(format!("filesink location={}", out.display()));
}

/// FFmpeg command for Windows: Desktop Duplication (`ddagrab`) or `gdigrab`.
pub fn ffmpeg_windows_args(
    spec: CaptureSpec,
    encoder: &str,
    out: &Path,
    backend: &str,
) -> Vec<String> {
    let mut a = vec![
        "ffmpeg".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
    ];
    a.push("-f".to_string());
    a.push(backend.to_string());
    a.push("-framerate".to_string());
    a.push(spec.hz.to_string());
    a.push("-video_size".to_string());
    a.push(format!("{}x{}", spec.width, spec.height));
    a.push("-i".to_string());
    a.push("desktop".to_string());
    a.push("-c:v".to_string());
    a.push(encoder.to_string());
    a.extend(ffmpeg_encoder_params(encoder));
    a.push("-pix_fmt".to_string());
    a.push("yuv420p".to_string());
    a.push("-movflags".to_string());
    a.push("+faststart".to_string());
    a.push(out.display().to_string());
    a
}

/// GStreamer encoder element(s) with low-latency tuning where relevant.
fn gst_encoder_elements(encoder: &str) -> Vec<String> {
    match encoder {
        "x264enc" => vec!["x264enc speed-preset=ultrafast tune=zerolatency".to_string()],
        other => vec![other.to_string()],
    }
}

/// Extra FFmpeg flags per encoder (hardware presets keep CPU ~0).
fn ffmpeg_encoder_params(encoder: &str) -> Vec<String> {
    let pairs: &[&str] = match encoder {
        "libx264" => &["-preset", "ultrafast", "-tune", "zerolatency", "-crf", "23"],
        "h264_nvenc" => &["-preset", "p1", "-tune", "ll"],
        "h264_amf" => &["-quality", "speed"],
        "h264_qsv" => &["-preset", "veryfast"],
        _ => &[],
    };
    pairs.iter().map(|s| s.to_string()).collect()
}

/// Render an argv as a single display string (for `--dump-pipeline`).
pub fn render_command(argv: &[String]) -> String {
    argv.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> CaptureSpec {
        CaptureSpec::new(3840, 2160, 144)
    }

    #[test]
    fn wayland_uses_pipewiresrc_and_child_fd() {
        let a = gst_wayland_args(42, PIPEWIRE_CHILD_FD, spec(), "x264enc", Path::new("/tmp/o.mp4"), false);
        let s = render_command(&a);
        assert!(s.contains("pipewiresrc fd=3 path=42"));
    }

    #[test]
    fn wayland_sets_native_framerate() {
        let a = gst_wayland_args(42, 3, spec(), "x264enc", Path::new("/tmp/o.mp4"), false);
        assert!(render_command(&a).contains("framerate=144/1"));
    }

    #[test]
    fn wayland_muxes_to_mp4() {
        let a = gst_wayland_args(42, 3, spec(), "x264enc", Path::new("/tmp/o.mp4"), false);
        let s = render_command(&a);
        assert!(s.contains("mp4mux"));
        assert!(s.contains("filesink location=/tmp/o.mp4"));
    }

    #[test]
    fn wayland_eos_on_shutdown_flag() {
        let a = gst_wayland_args(42, 3, spec(), "x264enc", Path::new("/tmp/o.mp4"), false);
        assert_eq!(a[1], "-e");
    }

    #[test]
    fn x11_uses_ximagesrc() {
        let a = gst_x11_args(spec(), "x264enc", Path::new("/tmp/o.mp4"), false);
        let s = render_command(&a);
        assert!(s.contains("ximagesrc use-damage=false"));
        assert!(s.contains("framerate=144/1"));
    }

    #[test]
    fn hardware_gst_encoder_is_plain_element() {
        let a = gst_wayland_args(42, 3, spec(), "vah264enc", Path::new("/tmp/o.mp4"), false);
        let s = render_command(&a);
        assert!(s.contains("vah264enc"));
        assert!(!s.contains("speed-preset"));
    }

    #[test]
    fn software_gst_encoder_is_tuned() {
        let a = gst_x11_args(spec(), "x264enc", Path::new("/tmp/o.mp4"), false);
        let s = render_command(&a);
        assert!(s.contains("x264enc speed-preset=ultrafast tune=zerolatency"));
    }

    #[test]
    fn h264parse_inserted_when_available() {
        let a = gst_wayland_args(42, 3, spec(), "h264_nvenc", Path::new("/tmp/o.mp4"), true);
        assert!(render_command(&a).contains("h264parse"));
    }

    #[test]
    fn h264parse_omitted_when_absent() {
        let a = gst_wayland_args(42, 3, spec(), "x264enc", Path::new("/tmp/o.mp4"), false);
        assert!(!render_command(&a).contains("h264parse"));
    }

    #[test]
    fn windows_ddagrab_has_native_size_and_rate() {
        let a = ffmpeg_windows_args(spec(), "libx264", Path::new("o.mp4"), "ddagrab");
        let s = render_command(&a);
        assert!(s.contains("-f ddagrab"));
        assert!(s.contains("-video_size 3840x2160"));
        assert!(s.contains("-framerate 144"));
    }

    #[test]
    fn windows_gdigrab_fallback() {
        let a = ffmpeg_windows_args(spec(), "libx264", Path::new("o.mp4"), "gdigrab");
        assert!(render_command(&a).contains("-f gdigrab"));
    }

    #[test]
    fn windows_faststart_and_pixfmt() {
        let a = ffmpeg_windows_args(spec(), "libx264", Path::new("o.mp4"), "ddagrab");
        let s = render_command(&a);
        assert!(s.contains("-movflags +faststart"));
        assert!(s.contains("-pix_fmt yuv420p"));
    }

    #[test]
    fn windows_nvenc_preset() {
        let a = ffmpeg_windows_args(spec(), "h264_nvenc", Path::new("o.mp4"), "ddagrab");
        let s = render_command(&a);
        assert!(s.contains("-c:v h264_nvenc"));
        assert!(s.contains("-preset p1"));
    }

    #[test]
    fn windows_libx264_ultrafast() {
        let a = ffmpeg_windows_args(spec(), "libx264", Path::new("o.mp4"), "ddagrab");
        assert!(render_command(&a).contains("-preset ultrafast"));
    }
}