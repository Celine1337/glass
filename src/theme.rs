//! Glass design tokens — a calm, premium dark palette with a soft rose accent.
//! Near-black warm charcoal, muted mauve text, hairline borders. No gradients,
//! no glassmorphism, no emoji.

use egui::{Color32, CornerRadius, Stroke};

/// Warm near-black background.
pub const BG: Color32 = Color32::from_rgb(0x10, 0x0e, 0x10);
/// Slightly lifted surface for chips and buttons.
pub const SURFACE: Color32 = Color32::from_rgb(0x1a, 0x16, 0x1a);
/// Hover surface.
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(0x24, 0x1d, 0x22);
/// Soft rose accent — the record action.
pub const ACCENT: Color32 = Color32::from_rgb(0xeb, 0xa7, 0xb9);
/// Muted mauve — secondary text.
pub const MUTED: Color32 = Color32::from_rgb(0xa8, 0x92, 0x9a);
/// Off-white — primary text.
pub const TEXT: Color32 = Color32::from_rgb(0xf8, 0xe3, 0xea);
/// Hairline border.
pub const HAIRLINE: Color32 = Color32::from_rgb(0x2b, 0x21, 0x26);

/// Large rounded surfaces (16–20px per the design contract).
pub const ROUND_LG: CornerRadius = CornerRadius::same(18);
/// Small rounded controls.
pub const ROUND_SM: CornerRadius = CornerRadius::same(10);

pub fn hairline() -> Stroke {
    Stroke::new(1.0_f32, HAIRLINE)
}

/// Apply the Glass dark theme to an egui context (once, at startup).
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals.dark_mode = true;
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = BG;
    style.visuals.extreme_bg_color = SURFACE;
    style.visuals.faint_bg_color = SURFACE;
    style.visuals.code_bg_color = SURFACE;
    style.visuals.warn_fg_color = ACCENT;
    style.visuals.error_fg_color = Color32::from_rgb(0xe0, 0x8a, 0x9a);
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.selection.bg_fill = Color32::from_rgb(0x3a, 0x2a, 0x31);
    style.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);

    style.visuals.widgets.noninteractive.bg_fill = SURFACE;
    style.visuals.widgets.noninteractive.bg_stroke = hairline();
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED);
    style.visuals.widgets.noninteractive.corner_radius = ROUND_SM;

    style.visuals.widgets.inactive.bg_fill = SURFACE;
    style.visuals.widgets.inactive.bg_stroke = hairline();
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.inactive.corner_radius = ROUND_SM;

    style.visuals.widgets.hovered.bg_fill = SURFACE_HOVER;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.hovered.corner_radius = ROUND_SM;

    style.visuals.widgets.active.bg_fill = SURFACE_HOVER;
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.active.corner_radius = ROUND_SM;

    style.visuals.widgets.open.bg_fill = SURFACE;
    style.visuals.widgets.open.bg_stroke = hairline();
    style.visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT);
    style.visuals.widgets.open.corner_radius = ROUND_SM;

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size = egui::vec2(40.0, 24.0);

    ctx.set_style(style);
}
