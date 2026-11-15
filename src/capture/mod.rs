//! Capture orchestration. Glass spawns a native child (GStreamer on Linux,
//! FFmpeg on Windows) and never touches a video frame itself.
//!
//! This module owns the [`Recording`] handle: the child process plus, on
//! Wayland, the portal session keep-alive. Stopping is graceful so the MP4
//! muxer finalizes a playable file.

#[cfg(unix)]
pub mod linux;
#[cfg(windows)]
pub mod windows;

use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::time::Duration;

use crate::child;

/// Child stderr goes to a log file beside the recording, prefixed with the
/// exact command line so a failed capture is diagnosable. Falls back to null
/// if the file cannot be created (recording must not fail because of logging).
pub(crate) fn stderr_log(log: &Path, argv: &[String]) -> Stdio {
    match std::fs::File::create(log) {
        Ok(mut f) => {
            use std::io::Write;
            let _ = writeln!(f, "$ {}", argv.join(" "));
            Stdio::from(f)
        }
        Err(_) => Stdio::null(),
    }
}

/// Which native backend is doing the capture/encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Gst,
    Ffmpeg,
}

/// An in-flight recording: the child process and its output path.
pub struct Recording {
    child: Child,
    output: PathBuf,
    backend: Backend,
    /// Child stderr is captured here so failures are diagnosable.
    log: PathBuf,
    #[cfg(unix)]
    portal: Option<linux::PortalHandle>,
}

impl Recording {
    pub fn new(child: Child, output: PathBuf, backend: Backend, log: PathBuf) -> Self {
        Self {
            child,
            output,
            backend,
            log,
            #[cfg(unix)]
            portal: None,
        }
    }

    /// Attach the Wayland portal keep-alive (closed when the recording stops).
    #[cfg(unix)]
    pub fn with_portal(mut self, portal: linux::PortalHandle) -> Self {
        self.portal = Some(portal);
        self
    }

    /// Non-blocking check whether the child already exited (e.g. crashed).
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Gracefully stop the child and finalize the file. Blocks up to ~14s;
    /// the UI runs this on a worker thread so it never freezes.
    pub fn finish(mut self) -> Result<PathBuf, String> {
        let result = self.stop_child();
        #[cfg(unix)]
        if let Some(portal) = self.portal.take() {
            portal.stop();
        }
        result
    }

    fn stop_child(&mut self) -> Result<PathBuf, String> {
        match self.backend {
            Backend::Gst => {
                // `gst-launch-1.0 -e` converts SIGINT into EOS + mux finalize.
                #[cfg(unix)]
                child::send_sigint(&self.child);
                #[cfg(windows)]
                {
                    let _ = self.child.kill();
                }
            }
            Backend::Ffmpeg => {
                // Ask FFmpeg to quit cleanly via its stdin command.
                if let Some(mut stdin) = self.child.stdin.take() {
                    use std::io::Write;
                    let _ = stdin.write_all(b"q\n");
                    let _ = stdin.flush();
                }
            }
        }

        let status = match child::wait_with_timeout(&mut self.child, Duration::from_secs(12)) {
            Ok(Some(status)) => Some(status),
            Ok(None) => {
                // Still running: escalate gracefully, then force.
                #[cfg(unix)]
                {
                    if self.backend == Backend::Ffmpeg {
                        child::send_sigint(&self.child);
                    } else {
                        child::send_sigterm(&self.child);
                    }
                    if child::wait_with_timeout(&mut self.child, Duration::from_secs(2))
                        .ok()
                        .flatten()
                        .is_none()
                    {
                        let _ = self.child.kill();
                    }
                }
                #[cfg(windows)]
                {
                    let _ = self.child.kill();
                }
                self.child.wait().ok()
            }
            Err(e) => return Err(format!("could not stop recorder: {e}")),
        };

        // A zero-byte MP4 is as useless as a missing one (muxer never ran).
        match std::fs::metadata(&self.output) {
            Ok(m) if m.len() > 0 => Ok(self.output.clone()),
            _ => Err(failure_message(status, &self.log)),
        }
    }
}

/// Actionable failure: exit code plus the tail of the child's stderr log.
fn failure_message(status: Option<ExitStatus>, log: &Path) -> String {
    let code = match status.and_then(|s| s.code()) {
        Some(c) => c.to_string(),
        None => "signal".to_string(),
    };
    let content = std::fs::read_to_string(log).unwrap_or_default();
    failure_message_from_parts(&code, &content)
}

fn failure_message_from_parts(code: &str, log_content: &str) -> String {
    let tail = log_tail(log_content, 300);
    if tail.is_empty() {
        format!("recorder exited (code {code}) before writing any video")
    } else {
        format!("recorder failed (code {code}): {tail}")
    }
}

/// Last `max` chars of the log, whitespace flattened for the single-line UI.
fn log_tail(content: &str, max: usize) -> String {
    let flat = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = flat.chars().count();
    if count <= max {
        flat
    } else {
        flat.chars().skip(count - max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_message_includes_code_and_log_tail() {
        let msg = failure_message_from_parts("1", "ERROR: not-negotiated\nstream error");
        assert!(msg.contains("code 1"));
        assert!(msg.contains("not-negotiated"));
    }

    #[test]
    fn failure_message_without_log_says_no_video() {
        let msg = failure_message_from_parts("1", "  \n");
        assert!(msg.contains("before writing any video"));
    }

    #[test]
    fn log_tail_keeps_last_chars() {
        let tail = log_tail("aaaa bbbb cccc", 9);
        assert_eq!(tail, "bbbb cccc");
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glass-mod-test-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn stderr_log_prefixes_exact_command_line() {
        let dir = temp_dir("argvlog");
        let log = dir.join("cmd.log");
        let argv = vec!["gst-launch-1.0".to_string(), "-e".to_string()];
        let _stdio = stderr_log(&log, &argv);
        let content = std::fs::read_to_string(&log).expect("read log");
        assert!(content.starts_with("$ gst-launch-1.0 -e"), "got: {content}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn dead_child() -> Child {
        std::process::Command::new("true")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn true")
    }

    #[cfg(unix)]
    #[test]
    fn finish_reports_stderr_when_no_file() {
        let dir = temp_dir("nofile");
        let log = dir.join("rec.log");
        std::fs::write(&log, "ERROR: not-negotiated\n").expect("write log");
        let rec = Recording::new(dead_child(), dir.join("missing.mp4"), Backend::Gst, log);
        let err = rec.finish().expect_err("must fail without a file");
        assert!(err.contains("not-negotiated"), "got: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn finish_rejects_empty_file() {
        let dir = temp_dir("empty");
        let out = dir.join("rec.mp4");
        std::fs::write(&out, b"").expect("write empty");
        let rec = Recording::new(dead_child(), out, Backend::Gst, dir.join("rec.log"));
        assert!(rec.finish().is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn finish_returns_path_for_nonempty_file() {
        let dir = temp_dir("ok");
        let out = dir.join("rec.mp4");
        std::fs::write(&out, b"video bytes").expect("write");
        let rec = Recording::new(dead_child(), out.clone(), Backend::Gst, dir.join("rec.log"));
        assert_eq!(rec.finish().expect("success"), out);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
