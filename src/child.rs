//! Subprocess lifecycle helpers.
//!
//! Capture + encode run entirely in a native child process; Glass only starts
//! and stops it. Stopping is graceful (SIGINT for GStreamer `-e`, `q` for
//! FFmpeg) so the MP4 muxer finalizes a playable file.

use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

/// Poll `try_wait` until the child exits or `timeout` elapses.
/// Returns `Ok(None)` on timeout (caller may then force-kill).
pub fn wait_with_timeout(
    child: &mut Child,
    timeout: Duration,
) -> std::io::Result<Option<ExitStatus>> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if start.elapsed() >= timeout {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Send SIGINT (GStreamer `-e` turns this into a clean EOS + finalize).
#[cfg(unix)]
pub fn send_sigint(child: &Child) {
    // SAFETY: `kill` only delivers a signal to a pid we own; no memory access.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGINT);
    }
}

/// Send SIGTERM as a last-resort graceful stop before SIGKILL.
#[cfg(unix)]
pub fn send_sigterm(child: &Child) {
    // SAFETY: same as `send_sigint` — signal delivery to our own child.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[cfg(unix)]
    #[test]
    fn wait_returns_exit_status() {
        let mut child = Command::new("true")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn true");
        let status = wait_with_timeout(&mut child, Duration::from_secs(5))
            .expect("wait")
            .expect("exited");
        assert!(status.success());
    }

    #[cfg(unix)]
    #[test]
    fn wait_times_out_on_long_child() {
        let mut child = Command::new("sleep")
            .arg("5")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep");
        let result = wait_with_timeout(&mut child, Duration::from_millis(150)).expect("wait");
        assert!(result.is_none());
        let _ = child.kill();
        let _ = child.wait();
    }
}