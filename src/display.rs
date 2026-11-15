//! Native resolution + refresh rate via `display-info`.
//!
//! This reads monitor metadata only (width / height / frequency) — it never
//! captures or screenshots anything. On a headless machine (CI) the query can
//! fail, so we fall back to a sane 1080p60 default instead of panicking.

use crate::pipeline::CaptureSpec;

/// Query the primary monitor's native resolution and refresh rate.
pub fn primary_spec() -> CaptureSpec {
    match display_info::DisplayInfo::all() {
        Ok(displays) => {
            let chosen = displays
                .iter()
                .find(|d| d.is_primary)
                .or_else(|| displays.first());
            match chosen {
                Some(d) => {
                    let mut hz = d.frequency.round() as i64;
                    if hz < 1 {
                        hz = 60;
                    }
                    CaptureSpec::new(d.width.max(1), d.height.max(1), hz as u32)
                }
                None => fallback(),
            }
        }
        Err(_) => fallback(),
    }
}

/// Used when no display can be queried (e.g. headless CI).
pub fn fallback() -> CaptureSpec {
    CaptureSpec::new(1920, 1080, 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_is_sane() {
        let s = fallback();
        assert_eq!((s.width, s.height, s.hz), (1920, 1080, 60));
    }
}