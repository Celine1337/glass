//! The Glass egui application: a single compact window, one primary action.
//!
//! States: idle, requesting-permission, recording, saving, error, success.
//! The UI repaints only on input plus ~1 Hz while recording (idle CPU ~0),
//! honours reduced motion, and is fully keyboard operable.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Margin, Rect, Response, RichText, CornerRadius,
    Sense, Stroke, Ui,
};

use crate::capture::{self, Recording};
use crate::display;
use crate::encode;
use crate::format;
use crate::pipeline::CaptureSpec;
use crate::theme::{self, ACCENT, BG, HAIRLINE, MUTED, SURFACE, TEXT};

#[derive(Clone)]
enum Phase {
    Idle,
    Requesting,
    Recording,
    Saving,
    Error(String),
    Success(PathBuf),
}

#[derive(Clone, Copy)]
enum Action {
    Toggle,
    Stop,
    Retry,
    Dismiss,
    OpenFolder,
    OpenFile,
    ChangeFolder,
    SaveFolder,
    CancelFolder,
    ToggleMotion,
}

pub struct GlassApp {
    phase: Phase,
    spec: CaptureSpec,
    output_dir: PathBuf,
    gst_encoder: String,
    h264parse: bool,
    // Only consumed by the Windows (FFmpeg) capture path.
    #[cfg_attr(not(windows), allow(dead_code))]
    ffmpeg_encoder: String,
    reduce_motion: bool,
    started_at: Option<Instant>,
    elapsed_secs: u64,
    recording: Option<Recording>,
    #[cfg(unix)]
    portal_pending: Option<capture::linux::WaylandStart>,
    finishing: Option<Receiver<Result<PathBuf, String>>>,
    editing_folder: bool,
    folder_draft: String,
}

impl GlassApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let spec = display::primary_spec();
        let output_dir = default_output_dir();
        let _ = std::fs::create_dir_all(&output_dir);
        let gst_encoder = encode::choose_gst(&encode::probe_gst());
        let h264parse = encode::gst_has_h264parse();
        let ffmpeg_encoder = encode::choose_ffmpeg(&encode::probe_ffmpeg());
        let reduce_motion = std::env::var_os("GLASS_REDUCED_MOTION").is_some();
        Self {
            phase: Phase::Idle,
            spec,
            output_dir,
            gst_encoder,
            h264parse,
            ffmpeg_encoder,
            reduce_motion,
            started_at: None,
            elapsed_secs: 0,
            recording: None,
            #[cfg(unix)]
            portal_pending: None,
            finishing: None,
            editing_folder: false,
            folder_draft: String::new(),
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Toggle => match self.phase {
                Phase::Recording => self.stop_record(),
                Phase::Requesting => self.cancel_request(),
                Phase::Saving => {}
                _ => self.begin_record(),
            },
            Action::Stop => match self.phase {
                Phase::Recording => self.stop_record(),
                Phase::Requesting => self.cancel_request(),
                _ => {}
            },
            Action::Retry => self.begin_record(),
            Action::Dismiss => {
                self.phase = Phase::Idle;
                self.started_at = None;
                self.elapsed_secs = 0;
            }
            Action::OpenFolder => open_folder(&self.output_dir),
            Action::OpenFile => {
                if let Phase::Success(path) = &self.phase {
                    open_file(path);
                }
            }
            Action::ChangeFolder => {
                self.folder_draft = self.output_dir.display().to_string();
                self.editing_folder = true;
            }
            Action::SaveFolder => {
                let trimmed = self.folder_draft.trim();
                if !trimmed.is_empty() {
                    self.output_dir = PathBuf::from(trimmed);
                    let _ = std::fs::create_dir_all(&self.output_dir);
                }
                self.editing_folder = false;
            }
            Action::CancelFolder => self.editing_folder = false,
            Action::ToggleMotion => self.reduce_motion = !self.reduce_motion,
        }
    }

    fn begin_record(&mut self) {
        if let Err(e) = std::fs::create_dir_all(&self.output_dir) {
            self.phase = Phase::Error(format!("cannot create folder: {e}"));
            return;
        }
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let out = self.output_dir.join(format::timestamp_filename(secs));
        self.elapsed_secs = 0;

        #[cfg(unix)]
        {
            if capture::linux::is_wayland() {
                match capture::linux::start_wayland(self.spec, out, self.gst_encoder.clone(), self.h264parse) {
                    Ok(ws) => {
                        self.portal_pending = Some(ws);
                        self.phase = Phase::Requesting;
                    }
                    Err(e) => self.phase = Phase::Error(e),
                }
            } else {
                match capture::linux::start_x11(self.spec, out, self.gst_encoder.clone(), self.h264parse) {
                    Ok(rec) => self.enter_recording(rec),
                    Err(e) => self.phase = Phase::Error(e),
                }
            }
        }
        #[cfg(windows)]
        {
            match capture::windows::start(self.spec, out, self.ffmpeg_encoder.clone()) {
                Ok(rec) => self.enter_recording(rec),
                Err(e) => self.phase = Phase::Error(e),
            }
        }
    }

    fn enter_recording(&mut self, rec: Recording) {
        self.recording = Some(rec);
        self.started_at = Some(Instant::now());
        self.phase = Phase::Recording;
    }

    fn stop_record(&mut self) {
        if let Some(rec) = self.recording.take() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name("glass-finalize".into())
                .spawn(move || {
                    let _ = tx.send(rec.finish());
                })
                .ok();
            self.finishing = Some(rx);
            self.phase = Phase::Saving;
        }
    }

    fn cancel_request(&mut self) {
        #[cfg(unix)]
        if let Some(ws) = self.portal_pending.take() {
            ws.handle.stop();
        }
        self.phase = Phase::Idle;
    }

    /// Advance portal / finalize channels and detect an early child exit.
    fn poll(&mut self) {
        #[cfg(unix)]
        if matches!(self.phase, Phase::Requesting) {
            let received = self.portal_pending.as_ref().map(|ws| ws.rx.try_recv());
            match received {
                Some(Ok(Ok(ready))) => {
                    if let Some(ws) = self.portal_pending.take() {
                        match capture::linux::finish_wayland(
                            ready,
                            ws.handle,
                            ws.spec,
                            ws.out,
                            ws.encoder,
                            ws.h264parse,
                        ) {
                            Ok(rec) => self.enter_recording(rec),
                            Err(e) => self.phase = Phase::Error(e),
                        }
                    }
                }
                Some(Ok(Err(e))) => {
                    if let Some(ws) = self.portal_pending.take() {
                        ws.handle.stop();
                    }
                    self.phase = Phase::Error(friendly_portal_error(&e));
                }
                Some(Err(TryRecvError::Disconnected)) => {
                    self.portal_pending = None;
                    self.phase = Phase::Error("portal closed unexpectedly".into());
                }
                Some(Err(TryRecvError::Empty)) | None => {}
            }
        }

        if matches!(self.phase, Phase::Saving) {
            let received = self.finishing.as_ref().map(|rx| rx.try_recv());
            match received {
                Some(Ok(Ok(path))) => {
                    self.finishing = None;
                    self.phase = Phase::Success(path);
                }
                Some(Ok(Err(e))) => {
                    self.finishing = None;
                    self.phase = Phase::Error(e);
                }
                Some(Err(TryRecvError::Disconnected)) => {
                    self.finishing = None;
                    self.phase = Phase::Error("recorder stopped unexpectedly".into());
                }
                Some(Err(TryRecvError::Empty)) | None => {}
            }
        }

        if matches!(self.phase, Phase::Recording) {
            let exited = self
                .recording
                .as_mut()
                .and_then(|r| r.try_wait().ok().flatten())
                .is_some();
            if exited {
                self.stop_record();
            }
            if let Some(start) = self.started_at {
                self.elapsed_secs = start.elapsed().as_secs();
            }
        }
    }

    fn schedule_repaint(&self, ctx: &egui::Context) {
        match self.phase {
            Phase::Recording => ctx.request_repaint_after(Duration::from_secs(1)),
            Phase::Requesting | Phase::Saving => {
                ctx.request_repaint_after(Duration::from_millis(150))
            }
            _ => {}
        }
    }
}

impl eframe::App for GlassApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Keyboard shortcuts (suppressed while a text field has focus).
        let editing_text = ctx.wants_keyboard_input();
        let (space, esc) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if !editing_text {
            if space {
                self.apply(Action::Toggle);
            }
            if esc {
                if self.editing_folder {
                    self.apply(Action::CancelFolder);
                } else {
                    self.apply(Action::Stop);
                }
            }
        }

        self.poll();
        self.schedule_repaint(ctx);

        let action = self.draw_ui(ctx);
        if let Some(action) = action {
            self.apply(action);
        }
    }
}

impl GlassApp {
    fn draw_ui(&mut self, ctx: &egui::Context) -> Option<Action> {
        let mut action: Option<Action> = None;
        let phase = self.phase.clone();
        let timer_secs = if matches!(phase, Phase::Recording | Phase::Saving) {
            self.elapsed_secs
        } else {
            0
        };

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20)))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);

                // Header: wordmark + subtitle, motion toggle on the right.
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Glass").size(26.0).color(TEXT).strong());
                        ui.label(RichText::new("screen recorder").size(12.0).color(MUTED));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(2.0);
                        let label = if self.reduce_motion { "motion off" } else { "motion on" };
                        let btn = pill_button(ui, label);
                        if btn.clicked() {
                            action = Some(Action::ToggleMotion);
                        }
                        btn.on_hover_text("Toggle animations (reduced motion)");
                    });
                });

                ui.add_space(14.0);

                // The single primary action.
                ui.vertical_centered(|ui| {
                    let resp = self.record_button(ui);
                    if resp.clicked() {
                        action = Some(Action::Toggle);
                    }
                });

                ui.add_space(2.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(state_caption(&phase))
                            .size(12.0)
                            .color(MUTED),
                    );
                });

                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(format::format_duration(timer_secs))
                            .monospace()
                            .size(30.0)
                            .color(TEXT),
                    );
                });

                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    egui::Frame::new()
                        .fill(SURFACE)
                        .stroke(theme::hairline())
                        .corner_radius(theme::ROUND_LG)
                        .inner_margin(Margin::symmetric(16, 7))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(format::format_res_hz(
                                    self.spec.width,
                                    self.spec.height,
                                    self.spec.hz,
                                ))
                                .monospace()
                                .size(14.0)
                                .color(TEXT),
                            );
                        });
                });

                ui.add_space(12.0);

                // Status / recovery area (error + success states).
                match &phase {
                    Phase::Error(msg) => {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(msg)
                                    .size(13.0)
                                    .color(Color32::from_rgb(0xe0, 0x8a, 0x9a)),
                            );
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if pill_button(ui, "Retry").clicked() {
                                    action = Some(Action::Retry);
                                }
                                if pill_button(ui, "Open folder").clicked() {
                                    action = Some(Action::OpenFolder);
                                }
                            });
                        });
                    }
                    Phase::Success(path) => {
                        let size = std::fs::metadata(path)
                            .map(|m| format::format_bytes(m.len()))
                            .unwrap_or_default();
                        let caption = if size.is_empty() {
                            format::shorten_path(&path.display().to_string(), 34)
                        } else {
                            format!(
                                "{}  ·  {size}",
                                format::shorten_path(&path.display().to_string(), 26)
                            )
                        };
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(caption)
                                    .monospace()
                                    .size(12.0)
                                    .color(MUTED),
                            );
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if pill_button(ui, "Open file").clicked() {
                                    action = Some(Action::OpenFile);
                                }
                                if pill_button(ui, "Open folder").clicked() {
                                    action = Some(Action::OpenFolder);
                                }
                                if pill_button(ui, "Done").clicked() {
                                    action = Some(Action::Dismiss);
                                }
                            });
                        });
                    }
                    _ => {}
                }

                ui.add_space(10.0);

                // Folder row (or inline editor).
                if self.editing_folder {
                    ui.horizontal(|ui| {
                        let edit = egui::TextEdit::singleline(&mut self.folder_draft)
                            .desired_width(220.0)
                            .font(FontId::monospace(12.0));
                        ui.add(edit);
                        if pill_button(ui, "Save").clicked() {
                            action = Some(Action::SaveFolder);
                        }
                        if pill_button(ui, "Cancel").clicked() {
                            action = Some(Action::CancelFolder);
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format::shorten_path(
                                &self.output_dir.display().to_string(),
                                22,
                            ))
                            .monospace()
                            .size(12.0)
                            .color(MUTED),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if pill_button(ui, "Open").clicked() {
                                action = Some(Action::OpenFolder);
                            }
                            if pill_button(ui, "Change").clicked() {
                                action = Some(Action::ChangeFolder);
                            }
                        });
                    });
                }

                ui.add_space(6.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Space record  ·  Esc stop")
                            .size(12.0)
                            .color(MUTED),
                    );
                });
            });

        action
    }

    /// The large circular record / stop button, custom-painted.
    fn record_button(&self, ui: &mut Ui) -> Response {
        let size = egui::vec2(150.0, 150.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        let painter = ui.painter();
        let center = rect.center();
        let t = ui.input(|i| i.time);
        let recording = matches!(self.phase, Phase::Recording);
        let busy = matches!(self.phase, Phase::Requesting | Phase::Saving);

        // Subtle pulse while recording (static when reduced motion is set).
        let pulse = if self.reduce_motion {
            0.5
        } else {
            ((t * std::f64::consts::PI).sin() as f32) * 0.5 + 0.5
        };

        let ring = if response.hovered() {
            ACCENT
        } else {
            ACCENT.gamma_multiply(0.85)
        };
        painter.circle_stroke(center, 64.0, Stroke::new(2.5_f32, ring));
        if response.has_focus() {
            painter.circle_stroke(center, 71.0, Stroke::new(1.5_f32, ACCENT.gamma_multiply(0.5)));
        }

        if busy {
            let angle = if self.reduce_motion {
                -std::f32::consts::FRAC_PI_2
            } else {
                (t as f32 * 2.0) % std::f32::consts::TAU - std::f32::consts::FRAC_PI_2
            };
            let pos = center + egui::vec2(angle.cos(), angle.sin()) * 64.0;
            painter.circle_filled(pos, 5.0, ACCENT);
            painter.text(
                center,
                Align2::CENTER_CENTER,
                "\u{2026}",
                FontId::proportional(30.0),
                MUTED,
            );
        } else if recording {
            painter.circle_filled(center, 60.0, ACCENT.gamma_multiply(0.16 + 0.12 * pulse));
            let sq = Rect::from_center_size(center, egui::vec2(34.0, 34.0));
            painter.rect_filled(sq, CornerRadius::same(6), TEXT);
        } else {
            painter.circle_filled(center, 60.0, ACCENT.gamma_multiply(0.08));
            painter.circle_filled(center, 24.0, ACCENT);
        }

        response
    }
}

fn state_caption(phase: &Phase) -> &'static str {
    match phase {
        Phase::Idle => "Ready",
        Phase::Requesting => "Waiting for permission\u{2026}",
        Phase::Recording => "Recording",
        Phase::Saving => "Saving\u{2026}",
        Phase::Error(_) => "Something went wrong",
        Phase::Success(_) => "Saved",
    }
}

fn friendly_portal_error(e: &str) -> String {
    let lower = e.to_lowercase();
    if lower.contains("cancel") {
        "Recording permission was cancelled.".to_string()
    } else if lower.contains("no streams") {
        "The portal returned no screen to record.".to_string()
    } else {
        format!("Portal error: {e}")
    }
}

fn default_output_dir() -> PathBuf {
    let base = dirs::video_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Glass")
}

fn pill_button(ui: &mut Ui, label: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(13.0).color(TEXT))
            .fill(SURFACE)
            .stroke(Stroke::new(1.0_f32, HAIRLINE))
            .corner_radius(theme::ROUND_SM)
            .min_size(egui::vec2(0.0, 26.0)),
    )
}

fn open_folder(dir: &Path) {
    #[cfg(unix)]
    {
        let _ = Command::new("xdg-open").arg(dir).spawn();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("explorer").arg(dir).spawn();
    }
}

fn open_file(path: &Path) {
    #[cfg(unix)]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("cmd")
            .args(["/C", "start", "", &path.display().to_string()])
            .spawn();
    }
}