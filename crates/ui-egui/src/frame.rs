//! The one place windows are made. Every command dialog, panel and pop-up window comes from
//! here, so they all shrink to their content within the same width limits (a test checks that no
//! other file creates an `egui::Window`, and that every dialog stays within the limits).
//!
//! - [`docked`]: command dialogs and command panels, flush to the right edge of the viewport.
//! - [`window`]: floating windows (Preferences, Parameters, prompts…), centred by default.
//! - [`right_aligned`]: right-align a row's last items without widening the window (a
//!   right-to-left layout would stretch it to its maximum width).

use egui::{Align2, Pos2, Ui, WidgetText, vec2};

/// Width limits of a docked command dialog.
pub const DIALOG_MIN_W: f32 = 260.0;
pub const DIALOG_MAX_W: f32 = 380.0;
/// Docked dialogs holding a table (motion study, configurations, exploded views).
pub const DIALOG_WIDE_MAX_W: f32 = 560.0;
/// Floating windows: prompts and panels, and those holding a table (Parameters, Keyboard
/// Shortcuts).
pub const WINDOW_MAX_W: f32 = 460.0;
pub const WINDOW_WIDE_MAX_W: f32 = 760.0;

/// How wide a window may get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Normal,
    Wide,
}

/// A command dialog docked at `anchor` (its top right corner), shrink-wrapped to its content.
pub fn docked<'a>(title: impl Into<WidgetText>, id: egui::Id, anchor: Pos2, max_height: f32, width: Width) -> egui::Window<'a> {
    let max = match width {
        Width::Normal => DIALOG_MAX_W,
        Width::Wide => DIALOG_WIDE_MAX_W,
    };
    egui::Window::new(title)
        .enabled(!crate::issue_report::cad_input_blocked())
        .id(id)
        .title_bar(false)
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(anchor)
        .constrain(false)
        .auto_sized()
        .collapsible(false)
        .max_width(max)
        .max_height(max_height.max(120.0))
}

/// A floating window, centred on the screen unless the caller places it, shrink-wrapped to its
/// content up to its width limit.
pub fn window<'a>(ctx: &egui::Context, title: impl Into<WidgetText>, width: Width) -> egui::Window<'a> {
    let max = match width {
        Width::Normal => WINDOW_MAX_W,
        Width::Wide => WINDOW_WIDE_MAX_W,
    };
    let c = ctx.content_rect().center();
    egui::Window::new(title)
        .enabled(!crate::issue_report::cad_input_blocked())
        .collapsible(false)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(c - vec2(0.0, 60.0))
        .max_width(max)
}

/// Floating model controls share the report's read-only drawing boundary with windows.
pub fn area(id: egui::Id) -> egui::Area {
    egui::Area::new(id).enabled(!crate::issue_report::cad_input_blocked())
}

/// The width a window's content had last frame (at least `min`), for laying out rows that end at
/// the right edge.
pub fn content_width(ui: &Ui, key: egui::Id, min: f32) -> f32 {
    ui.ctx().data(|d| d.get_temp::<f32>(key)).unwrap_or(min).max(min)
}

/// Remember the content width for next frame (call at the end of the window's content).
pub fn remember_width(ui: &Ui, key: egui::Id) {
    let w = ui.min_rect().width();
    ui.ctx().data_mut(|d| d.insert_temp(key, w));
}

/// In a horizontal row: push what `add` draws (`width` wide) to the right edge of a content
/// `total` wide, without making the row any wider than that.
pub fn right_aligned<R>(ui: &mut Ui, total: f32, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let used = ui.min_rect().width();
    let gap = (total - used - width - ui.spacing().item_spacing.x).max(0.0);
    ui.add_space(gap);
    add(ui)
}

#[cfg(test)]
mod tests {
    #[test]
    fn feature_forms_use_available_height_and_keep_actions_visible() {
        use crate::scenario::Harness;
        use serde_json::json;
        for (width, height, scale) in [(1400.0, 900.0, 1.0), (1400.0, 900.0, 2.0)] {
            let mut h = Harness::new();
            h.app.preferences.ui_scale = scale;
            h.call("ui.resize", json!({"width":width / scale,"height":height / scale}));
            h.app.run("sketch.create", json!({"plane":"XY","name":"Base"})).unwrap();
            h.app.run("sketch.rectangle.two_point", json!({"p0":[0,0],"p1":[10,10]})).unwrap();
            h.app.run("sketch.finish", json!({})).unwrap();
            h.step(&json!({"start":"solid.extrude"})).unwrap();
            h.frames(10);
            let dialog = h.app.viewport.dialog_rect.unwrap();
            let viewport = h.app.viewport.rect.unwrap();
            assert!(dialog.height() > 230.0, "scale={scale}: {dialog:?}");
            assert!(viewport.expand(1.0).contains_rect(dialog), "scale={scale}: {dialog:?} outside {viewport:?}");
            assert!(h.widget("Distance").is_some(), "distance must be visible without scrolling");
            if scale == 1.0 {
                assert!(h.widget("Operation").is_some(), "operation must be visible without scrolling");
            }
            assert!(dialog.contains(crate::scenario::handle_at("dialog:OK").unwrap()));
            assert!(h.widget("Cancel").is_some(), "cancel must be visible without scrolling");
            h.step(&json!({"widget":"Cancel"})).unwrap();
            assert!(h.app.dialog.is_none());
            h.step(&json!({"start":"solid.shell"})).unwrap();
            h.frames(8);
            let short = h.app.viewport.dialog_rect.unwrap();
            assert!(short.height() < dialog.height(), "short forms should shrink: {short:?}, {dialog:?}");
            assert!(h.widget("Cancel").is_some());
            h.step(&json!({"widget":"Cancel"})).unwrap();
            h.step(&json!({"start":"solid.hole"})).unwrap();
            if let Some(crate::dialogs::Dialog { kind: crate::dialogs::Kind::Hole { kind, opts, .. }, .. }) = h.app.dialog.as_mut() {
                *kind = 1;
                opts.tapped = true;
            }
            h.frames(8);
            let long = h.app.viewport.dialog_rect.unwrap();
            assert!(viewport.expand(1.0).contains_rect(long), "long form clipped: {long:?}");
            assert!(h.widget("Cancel").is_some(), "long-form actions must stay visible");
            assert!(long.contains(crate::scenario::handle_at("dialog:OK").unwrap()));
            h.step(&json!({"widget":"Cancel"})).unwrap();
            assert!(h.app.dialog.is_none());
        }
    }
    /// Windows are made in this file only, so none can skip the width limits.
    #[test]
    fn windows_come_from_the_frame_helper() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut bad = Vec::new();
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "rs") && p.file_name().is_some_and(|n| n != "frame.rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                if text.contains("Window::new(") {
                    bad.push(p.display().to_string());
                }
            }
        }
        assert!(bad.is_empty(), "use crate::frame::{{docked, window}} instead of egui::Window::new in {bad:?}");
    }
}
