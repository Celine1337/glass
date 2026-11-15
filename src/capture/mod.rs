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

use std::path::PathBuf;
use std::process::{Child, ExitStatus};
use std::time::Duration;

use crate::child;

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
    #[cfg(unix)]
    portal: Option<linux::PortalHandle>,
}

impl Recording {
    pub fn new(child: Child, output: PathBuf, backend: Backend) -> Self {
        Self {
            child,
            output,
            backend,
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

        match child::wait_with_timeout(&mut self.child, Duration::from_secs(12)) {
            Ok(Some(_)) => {}
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
                let _ = self.child.wait();
            }
            Err(e) => return Err(format!("could not stop recorder: {e}")),
        }

        if self.output.exists() {
            Ok(self.output.clone())
        } else {
            Err("recording finished but produced no file".to_string())
        }
    }
}