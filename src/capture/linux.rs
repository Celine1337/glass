//! Linux capture: Wayland (xdg-desktop-portal ScreenCast -> PipeWire ->
//! GStreamer) and X11 (`ximagesrc` -> GStreamer).
//!
//! The portal runs on a dedicated thread with a current-thread tokio runtime
//! (ashpd is async). The PipeWire remote fd is handed to the GStreamer child
//! at fd 3, so the Rust process never reads a frame.

use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;

use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
    StartCastOptions,
};
use ashpd::desktop::{CreateSessionOptions, Session};

use crate::capture::{Backend, Recording};
use crate::pipeline::{self, CaptureSpec};

/// True when running under a Wayland session (portal path); otherwise X11.
pub fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false)
}

/// The portal result handed back to the UI thread.
pub struct PortalReady {
    pub fd: OwnedFd,
    pub node: u32,
}

/// Keeps the portal session alive on its thread; closed when recording stops.
pub struct PortalHandle {
    stop_tx: tokio::sync::oneshot::Sender<()>,
    // Dropping the JoinHandle detaches the thread; it exits after closing.
    _join: std::thread::JoinHandle<()>,
}

impl PortalHandle {
    /// Signal the portal thread to close the session and exit.
    pub fn stop(self) {
        let _ = self.stop_tx.send(());
    }
}

/// Everything the UI needs to drive a Wayland recording start.
pub struct WaylandStart {
    pub rx: Receiver<Result<PortalReady, String>>,
    pub handle: PortalHandle,
    pub out: PathBuf,
    pub encoder: String,
    pub h264parse: bool,
}

/// Run the ScreenCast portal flow and return (session, PipeWire fd, node id).
async fn portal_flow() -> Result<(Session<Screencast>, OwnedFd, u32), String> {
    let screencast = Screencast::new().await.map_err(|e| e.to_string())?;
    let session = screencast
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(|e| e.to_string())?;
    screencast
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_multiple(false)
                .set_sources(SourceType::MONITOR)
                .set_cursor_mode(CursorMode::EMBEDDED),
        )
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| e.to_string())?;
    let streams = screencast
        .start(&session, None, StartCastOptions::default())
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| e.to_string())?;
    let fd = screencast
        .open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default())
        .await
        .map_err(|e| e.to_string())?;
    let node = streams
        .streams()
        .first()
        .map(|s| s.pipe_wire_node_id())
        .ok_or_else(|| "portal returned no streams".to_string())?;
    Ok((session, fd, node))
}

/// Spawn the portal thread. The UI shows a "requesting permission" state until
/// the ready channel yields the PipeWire fd + node (or an error / cancel).
pub fn start_wayland(
    out: PathBuf,
    encoder: String,
    h264parse: bool,
) -> Result<WaylandStart, String> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();

    let join = std::thread::Builder::new()
        .name("glass-portal".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("could not start async runtime: {e}")));
                    return;
                }
            };
            rt.block_on(async move {
                match portal_flow().await {
                    Ok((session, fd, node)) => {
                        if ready_tx.send(Ok(PortalReady { fd, node })).is_err() {
                            let _ = session.close().await;
                            return;
                        }
                        // Keep the runtime (and thus the portal session) alive
                        // until the UI tells us to stop.
                        let _ = stop_rx.await;
                        let _ = session.close().await;
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            });
        })
        .map_err(|e| format!("could not start portal thread: {e}"))?;

    Ok(WaylandStart {
        rx: ready_rx,
        handle: PortalHandle {
            stop_tx,
            _join: join,
        },
        out,
        encoder,
        h264parse,
    })
}

/// Map the portal's PipeWire remote onto child fd 3, then spawn GStreamer.
///
/// `std`'s proposed `CommandExt::fd_mappings` is not stable, so we use the
/// equivalent `pre_exec` + `dup2`: the duplicate clears `FD_CLOEXEC`, so fd 3
/// survives `exec` and `pipewiresrc fd=3` can read it.
pub fn finish_wayland(
    ready: PortalReady,
    handle: PortalHandle,
    out: PathBuf,
    encoder: String,
    h264parse: bool,
) -> Result<Recording, String> {
    let args = pipeline::gst_wayland_args(
        ready.node,
        pipeline::PIPEWIRE_CHILD_FD,
        &encoder,
        &out,
        h264parse,
    );
    let log = out.with_extension("log");
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(crate::capture::stderr_log(&log, &args));
    map_fd_to_child(&mut cmd, &ready.fd);
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not start gst-launch-1.0: {e}"))?;
    // The child owns a duplicate at fd 3; the parent copy is no longer needed.
    drop(ready.fd);
    Ok(Recording::new(child, out, Backend::Gst, log).with_portal(handle))
}

/// X11: full-screen `ximagesrc`, no portal needed.
pub fn start_x11(
    spec: CaptureSpec,
    out: PathBuf,
    encoder: String,
    h264parse: bool,
) -> Result<Recording, String> {
    let args = pipeline::gst_x11_args(spec, &encoder, &out, h264parse);
    let log = out.with_extension("log");
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(crate::capture::stderr_log(&log, &args));
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not start gst-launch-1.0: {e}"))?;
    Ok(Recording::new(child, out, Backend::Gst, log))
}

/// Duplicate `fd` onto child fd 3 in the forked child, before `exec`.
fn map_fd_to_child(cmd: &mut Command, fd: &OwnedFd) {
    use std::os::unix::process::CommandExt;
    let raw = fd.as_raw_fd();
    // SAFETY: the closure runs in the forked child before exec. `dup2` is
    // async-signal-safe and allocates nothing. Mapping the CLOEXEC portal fd
    // onto fd 3 clears CLOEXEC on the duplicate so it survives exec — the same
    // semantics as the proposed `CommandExt::fd_mappings`.
    unsafe {
        cmd.pre_exec(move || {
            if libc::dup2(raw, pipeline::PIPEWIRE_CHILD_FD) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spawn the real Wayland argv exactly like `finish_wayland` (same
    /// builder, same pre_exec fd-3 mapping), with /dev/null standing in for
    /// the portal remote. The pipeline must PARSE: runtime may fail on the
    /// fake fd, but a syntax error means the argv itself is broken.
    #[test]
    fn spawned_wayland_argv_parses_without_syntax_error() {
        let dir = std::env::temp_dir().join(format!("glass-Видео-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let out = dir.join("probe.mp4");
        let args =
            pipeline::gst_wayland_args(72, pipeline::PIPEWIRE_CHILD_FD, "x264enc", &out, false);
        let mut cmd = Command::new(&args[0]);
        cmd.args(&args[1..]);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let devnull = OwnedFd::from(std::fs::File::open("/dev/null").expect("open /dev/null"));
        map_fd_to_child(&mut cmd, &devnull);
        let mut child = cmd.spawn().expect("spawn gst-launch");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if child.try_wait().expect("try_wait").is_some() {
                break;
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let output = child.wait_with_output().expect("wait");
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(
            !err.contains("syntax error") && !err.contains("синтаксическая ошибка"),
            "gst-launch failed to parse the spawned argv: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
