//! Encoder detection. The Rust process never touches video frames — it only
//! probes which H.264 encoder the native backend can use, preferring hardware
//! so CPU stays near zero. Results are cached by the caller at startup.

use std::process::{Command, Stdio};

/// GStreamer H.264 encoders, best (hardware) first.
pub const GST_ENCODERS: &[&str] = &["vah264enc", "nvh264enc", "x264enc"];

/// FFmpeg H.264 encoders, best (hardware) first.
pub const FFMPEG_ENCODERS: &[&str] = &["h264_nvenc", "h264_amf", "h264_qsv", "h264_mf", "libx264"];

/// Parse `ffmpeg -hide_banner -encoders` output into encoder names.
///
/// Encoder rows look like ` V..... h264_nvenc   NVIDIA NVENC ...`: a flags
/// column (a letter followed by dots) then the name. Pure + unit-tested.
pub fn parse_ffmpeg_encoders(output: &str) -> Vec<String> {
    let mut names = Vec::new();
    // Real output prints a legend (e.g. ` V..... = Video`) before a `------`
    // separator; encoder rows follow it. Skip the legend when present.
    let has_separator = output.lines().any(|l| l.trim() == "------");
    let mut past_header = false;
    for line in output.lines() {
        if has_separator && !past_header {
            if line.trim() == "------" {
                past_header = true;
            }
            continue;
        }
        let mut parts = line.split_whitespace();
        let flags = match parts.next() {
            Some(f) => f,
            None => continue,
        };
        let kind = flags.chars().next().unwrap_or(' ');
        if !matches!(kind, 'V' | 'A' | 'S') || !flags.contains('.') {
            continue;
        }
        if let Some(name) = parts.next() {
            if name != "=" {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// First candidate present in `available`, preserving preference order.
/// Pure + unit-tested.
pub fn select_first_available<'a>(candidates: &[&'a str], available: &[String]) -> Option<&'a str> {
    candidates
        .iter()
        .copied()
        .find(|c| available.iter().any(|a| a == c))
}

/// Probe which GStreamer encoders exist via `gst-inspect-1.0 <name>`.
pub fn probe_gst() -> Vec<String> {
    GST_ENCODERS
        .iter()
        .filter(|name| gst_has_encoder(name))
        .map(|s| s.to_string())
        .collect()
}

fn gst_has_encoder(name: &str) -> bool {
    Command::new("gst-inspect-1.0")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Whether `h264parse` is installed (needed to feed some HW encoders to mp4mux).
pub fn gst_has_h264parse() -> bool {
    gst_has_encoder("h264parse")
}

/// Probe which FFmpeg encoders exist by parsing `ffmpeg -encoders`.
pub fn probe_ffmpeg() -> Vec<String> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(o) if o.status.success() => parse_ffmpeg_encoders(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

/// Convenience: the preferred available GStreamer encoder, or a software fallback.
pub fn choose_gst(available: &[String]) -> String {
    select_first_available(GST_ENCODERS, available)
        .unwrap_or("x264enc")
        .to_string()
}

/// Convenience: the preferred available FFmpeg encoder, or a software fallback.
pub fn choose_ffmpeg(available: &[String]) -> String {
    select_first_available(FFMPEG_ENCODERS, available)
        .unwrap_or("libx264")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Encoders:
 V..... = Video
 A..... = Audio
 S..... = Subtitle
 ------
 V..... a64multi              Multicolor charset for Commodore 64
 V..... h264_nvenc            NVIDIA NVENC H.264 encoder
 V..... libx264               libx264 H.264 / AVC / MPEG-4 AVC
 A..... aac                   AAC (Advanced Audio Coding)
";

    #[test]
    fn parses_video_and_audio_names() {
        let names = parse_ffmpeg_encoders(SAMPLE);
        assert!(names.contains(&"h264_nvenc".to_string()));
        assert!(names.contains(&"libx264".to_string()));
        assert!(names.contains(&"aac".to_string()));
    }

    #[test]
    fn skips_legend_and_header() {
        let names = parse_ffmpeg_encoders(SAMPLE);
        assert!(!names.iter().any(|n| n == "=" || n == "------"));
        assert!(!names.contains(&"Video".to_string()));
    }

    #[test]
    fn empty_output_yields_nothing() {
        assert!(parse_ffmpeg_encoders("").is_empty());
    }

    #[test]
    fn no_separator_still_filters_legend() {
        let out = " V..... = Video\n V..... h264_nvenc  NVIDIA\n";
        let names = parse_ffmpeg_encoders(out);
        assert_eq!(names, vec!["h264_nvenc".to_string()]);
    }

    #[test]
    fn prefers_hardware_over_software() {
        let available = vec!["libx264".to_string(), "h264_nvenc".to_string()];
        assert_eq!(
            select_first_available(FFMPEG_ENCODERS, &available),
            Some("h264_nvenc")
        );
    }

    #[test]
    fn falls_back_to_software_when_no_hardware() {
        let available = vec!["libx264".to_string()];
        assert_eq!(
            select_first_available(FFMPEG_ENCODERS, &available),
            Some("libx264")
        );
    }

    #[test]
    fn returns_none_when_nothing_available() {
        let available: Vec<String> = vec![];
        assert_eq!(select_first_available(GST_ENCODERS, &available), None);
    }

    #[test]
    fn gst_preference_order() {
        let available = vec!["x264enc".to_string(), "vah264enc".to_string()];
        assert_eq!(
            select_first_available(GST_ENCODERS, &available),
            Some("vah264enc")
        );
    }

    #[test]
    fn choose_defaults_to_software() {
        assert_eq!(choose_gst(&[]), "x264enc");
        assert_eq!(choose_ffmpeg(&[]), "libx264");
    }
}
