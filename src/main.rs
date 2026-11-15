//! Glass — a tiny, beautiful screen recorder.
//!
//! The Rust process is an orchestrator + egui UI only; capture and encoding
//! happen in a native child subprocess (GStreamer on Linux, FFmpeg on Windows),
//! so Glass never touches a video frame and stays light on RAM/CPU.

mod app;
mod capture;
mod child;
mod display;
mod encode;
mod format;
mod pipeline;
mod theme;

use std::path::PathBuf;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("--version") | Some("-V") => {
            println!("glass {VERSION}");
            ExitCode::SUCCESS
        }
        Some("--help") | Some("-h") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some("--probe") => {
            probe();
            ExitCode::SUCCESS
        }
        Some("--dump-pipeline") => dump_pipeline(&args[1..]),
        Some(other) if other.starts_with('-') => {
            eprintln!("unknown option: {other}\n");
            print_help();
            ExitCode::FAILURE
        }
        _ => launch(),
    }
}

fn launch() -> ExitCode {
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([400.0, 520.0])
            .with_resizable(false)
            .with_title("Glass"),
        ..Default::default()
    };
    match eframe::run_native(
        "Glass",
        native_options,
        Box::new(|cc| Ok(Box::new(app::GlassApp::new(cc)))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Glass failed to start: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("Glass {VERSION} — a tiny, beautiful screen recorder.");
    println!();
    println!("USAGE:");
    println!("  glass                 Launch the recorder");
    println!("  glass --version       Print version");
    println!("  glass --help          Print this help");
    println!("  glass --probe         Print display + encoder info as JSON (no capture)");
    println!("  glass --dump-pipeline <wayland|x11|windows> <w> <h> <hz> <out.mp4>");
    println!("                        Print the native capture command (no capture)");
    println!();
    println!("Keys: Space record/stop · Esc stop");
}

/// Print display + encoder detection as JSON. Reads monitor metadata only;
/// never starts a capture or a portal session.
fn probe() {
    let spec = display::primary_spec();
    let gst = encode::probe_gst();
    let ff = encode::probe_ffmpeg();
    let wayland = {
        #[cfg(unix)]
        {
            capture::linux::is_wayland()
        }
        #[cfg(windows)]
        {
            false
        }
    };
    let gst_list = gst
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ");
    println!("{{");
    println!("  \"version\": \"{VERSION}\",");
    println!("  \"platform\": \"{}\",", std::env::consts::OS);
    println!(
        "  \"display\": {{ \"width\": {}, \"height\": {}, \"hz\": {} }},",
        spec.width, spec.height, spec.hz
    );
    println!("  \"gst_encoder\": \"{}\",", encode::choose_gst(&gst));
    println!("  \"ffmpeg_encoder\": \"{}\",", encode::choose_ffmpeg(&ff));
    println!("  \"gst_available\": [{gst_list}],");
    println!("  \"wayland\": {wayland}");
    println!("}}");
}

/// Print the exact native capture command for a platform. Pure builders; no
/// capture, no portal, no window. Used by the test-suite to assert pipelines.
fn dump_pipeline(args: &[String]) -> ExitCode {
    if args.len() < 5 {
        eprintln!(
            "usage: glass --dump-pipeline <wayland|x11|windows> <width> <height> <hz> <output.mp4>"
        );
        return ExitCode::FAILURE;
    }
    let platform = args[0].as_str();
    let width: u32 = args[1].parse().unwrap_or(1920);
    let height: u32 = args[2].parse().unwrap_or(1080);
    let hz: u32 = args[3].parse().unwrap_or(60);
    let out = PathBuf::from(&args[4]);
    let spec = pipeline::CaptureSpec::new(width, height, hz);
    let argv = match platform {
        "wayland" => pipeline::gst_wayland_args(
            42,
            pipeline::PIPEWIRE_CHILD_FD,
            spec,
            "x264enc",
            &out,
            false,
        ),
        "x11" => pipeline::gst_x11_args(spec, "x264enc", &out, false),
        "windows" => pipeline::ffmpeg_windows_args(spec, "libx264", &out, "ddagrab"),
        _ => {
            eprintln!("unknown platform: {platform} (use wayland|x11|windows)");
            return ExitCode::FAILURE;
        }
    };
    println!("# glass capture: {width}x{height} @ {hz} Hz ({platform})");
    println!("{}", pipeline::render_command(&argv));
    ExitCode::SUCCESS
}