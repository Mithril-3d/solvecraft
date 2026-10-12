//! The Sketch Palette: a panel docked at the right while a sketch is being edited, like a
//! command dialog. Options: the line type of the selected curves (normal, construction,
//! centerline), Look At, and the display toggles kept with the sketch (grid, snap, slice,
//! profiles, points, dimensions, constraints, projected geometry, 3D sketch); the sketch's
//! degrees of freedom; Finish Sketch. Toggles run `sketch.options`, line types
//! `sketch.construction` / `sketch.centerline`.
//!
//! The line type is also a drawing mode, as in Fusion: with nothing selected, Construction (or
//! X) makes what is drawn next construction geometry until it is switched back.

use std::cell::Cell;

use egui::{Color32, RichText, Stroke, vec2};
use serde_json::{Value, json};
use solvecraft_engine::Sel;

use crate::SolveApp;
use crate::theme::Tokens;

thread_local! {
    /// The drawing line type (0 normal, 1 construction, 2 centerline) and the sketch it is for.
    static MODE: Cell<(u8, Option<u64>)> = const { Cell::new((0, None)) };
    static FOLDED: Cell<bool> = const { Cell::new(false) };
    static OPTIONS_FOLDED: Cell<bool> = const { Cell::new(false) };
}

/// Palette width (logical px).
const WIDTH: f32 = 236.0;

/// The display toggles: option key and label.
pub const TOGGLES: [(&str, &str); 9] = [
    ("grid", "Sketch Grid"),
    ("snap", "Snap"),
    ("slice", "Slice"),
    ("show_profile", "Show Profile"),
    ("show_points", "Show Points"),
    ("show_dimensions", "Show Dimensions"),
    ("show_constraints", "Show Constraints"),
    ("show_projected", "Show Projected Geometries"),
    ("sketch_3d", "3D Sketch"),
];

/// The degrees-of-freedom line: "Fully constrained" or "N degrees of freedom".
pub fn dof_text(dof: usize, ok: bool) -> String {
    if !ok {
        "Constraints conflict".into()
    } else if dof == 0 {
        "Fully constrained".into()
    } else if dof == 1 {
        "1 degree of freedom".into()
    } else {
        format!("{dof} degrees of freedom")
    }
}

/// The line type new curves get in the sketch being edited (0 normal, 1 construction,
/// 2 centerline); a new sketch starts normal.
pub fn mode(app: &SolveApp) -> u8 {
    let (m, sk) = MODE.with(Cell::get);
    if sk.is_some() && sk == app.session.active_sketch { m } else { 0 }
}

/// Switch the drawing line type (with a status hint).
pub fn set_mode(app: &mut SolveApp, m: u8) {
    MODE.with(|c| c.set((m, app.session.active_sketch)));
    let hint = match m {
        1 => "Construction mode: new curves are construction geometry (X or the palette's Normal to leave)",
        2 => "Centerline mode: new lines are centerlines",
        _ => "Normal mode",
    };
    app.set_status(hint, false);
}

/// X / the toolbar's Construction: convert the selected curves, or with nothing selected
/// switch construction mode on or off.
pub fn toggle_construction(app: &mut SolveApp) {
    let curves = selected_curves(app);
    if curves.is_empty() && app.session.active_sketch.is_none() {
        app.set_status("Construction mode is for drawing in a sketch: edit a sketch first", false);
    } else if curves.is_empty() {
        let m = if mode(app) == 1 { 0 } else { 1 };
        set_mode(app, m);
    } else {
        let _ = app.run("sketch.construction", json!({ "curves": curves }));
    }
}

/// Curves a drawing command just made take the drawing line type.
pub fn apply_mode(app: &mut SolveApp, id: &str, result: &Value) {
    let m = mode(app);
    if m == 0 || !solvecraft_engine::find_command(id).is_some_and(|c| c.tab == "SKETCH" && c.panel == "CREATE") {
        return;
    }
    let curves: Vec<String> = result["curves"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_string)).collect();
    if curves.is_empty() {
        return;
    }
    let lines: Vec<String> = if m == 2 {
        let st = app.session.model.state();
        let Some(ss) = app.session.active_sketch.and_then(|s| st.sketch(s)) else { return };
        curves
            .iter()
            .filter(|c| {
                ss.sketch
                    .curve_index(c)
                    .and_then(|i| ss.sketch.curves.get(i))
                    .is_some_and(|c| matches!(c.kind, solvecraft_engine::sketch::CurveKind::Line { .. }))
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let rest: Vec<String> = curves.iter().filter(|c| !lines.contains(c)).cloned().collect();
    // Part of the drawing's own undo step.
    let depth = app.session.undo.len();
    if !lines.is_empty() {
        let _ = app.session.execute("sketch.centerline", &json!({ "curves": lines, "value": true }));
    }
    if !rest.is_empty() {
        let _ = app.session.execute("sketch.construction", &json!({ "curves": rest, "value": true }));
    }
    app.session.undo.truncate(depth);
}

/// Selected curves of the active sketch.
fn selected_curves(app: &SolveApp) -> Vec<String> {
    app.session.selection.iter().filter_map(|s| if let Sel::SketchCurve { id } = s { Some(id.clone()) } else { None }).collect()
}

/// Set the line type of the selected curves: 0 normal, 1 construction, 2 centerline (lines).
pub fn set_linetype(app: &mut SolveApp, kind: u8) {
    let curves = selected_curves(app);
    if curves.is_empty() {
        app.set_status("Select sketch curves first", false);
        return;
    }
    let lines: Vec<String> = {
        let st = app.session.world_state();
        let Some(ss) = app.session.active_sketch.and_then(|s| st.sketch(s)) else { return };
        curves
            .iter()
            .filter(|id| {
                ss.sketch
                    .curve_index(id)
                    .and_then(|i| ss.sketch.curves.get(i))
                    .is_some_and(|c| matches!(c.kind, solvecraft_engine::sketch::CurveKind::Line { .. }))
            })
            .cloned()
            .collect()
    };
    let r = match kind {
        0 => {
            if !lines.is_empty() {
                let _ = app.run("sketch.centerline", json!({ "curves": lines, "value": false }));
            }
            app.run("sketch.construction", json!({ "curves": curves, "value": false }))
        }
        1 => app.run("sketch.construction", json!({ "curves": curves, "value": true })),
        _ if lines.is_empty() => Err("a centerline must be a line".to_string()),
        _ => app.run("sketch.centerline", json!({ "curves": lines, "value": true })),
    };
    if let Err(e) = r {
        app.set_status(e, true);
    }
}

/// Draw the palette (while sketching and no command dialog is open).
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    app.viewport.palette_rect = None;
    let _language = crate::i18n::scope(&app.preferences.language);
    if app.dialog.is_some() {
        return;
    }
    let (opts, dof_line, ok) = {
        let st = app.session.world_state();
        let Some(ss) = app.session.active_sketch.and_then(|s| st.sketch(s)) else { return };
        (ss.sketch.view.clone(), dof_text(ss.report.dof, ss.report.ok()), ss.report.ok() && ss.report.dof == 0)
    };
    let t = Tokens::get();
    let vp = app.viewport.rect.unwrap_or_else(|| ctx.content_rect());
    let bottom = vp.bottom().min(ctx.content_rect().bottom()) - 8.0;
    // Prefer the space below Axis View, but reclaim it on short/HiDPI views.
    let top = (vp.top() + crate::viewport::AXIS_VIEW_CLEARANCE).min((bottom - 360.0).max(vp.top()));
    let anchor = egui::pos2(vp.right(), top);
    let frame = egui::Frame::window(&ctx.global_style())
        .fill(t.dialog_bg)
        .stroke(Stroke::new(1.0, t.dialog_border))
        .corner_radius(egui::CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 })
        .shadow(egui::Shadow { offset: [-2, 2], blur: 8, spread: 0, color: Color32::from_black_alpha(40) });
    let shown = |k: &str| match k {
        "grid" => !opts.hide_grid,
        "snap" => !opts.no_snap,
        "slice" => opts.slice,
        "show_profile" => !opts.hide_profile,
        "show_points" => !opts.hide_points,
        "show_dimensions" => !opts.hide_dimensions,
        "show_constraints" => !opts.hide_constraints,
        "show_projected" => !opts.hide_projected,
        "sketch_3d" => opts.three_d,
        _ => false,
    };
    let mut set: Option<(String, bool)> = None;
    let mut linetype: Option<u8> = None;
    let mut look = false;
    let mut finish = false;
    let has_curves = !selected_curves(app).is_empty();
    let response = crate::frame::docked(
        app.tr("ui.heading_sketch_palette"),
        egui::Id::new("sc_sketch_palette"),
        anchor,
        bottom - anchor.y,
        crate::frame::Width::Normal,
    )
    .frame(frame)
    .show(ctx, |ui| {
        let width = WIDTH.min((vp.width() * 0.46 - 16.0).max(120.0));
        ui.set_width(width);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.horizontal(|ui| {
            let folded = FOLDED.with(Cell::get);
            if ui.add(egui::Button::new(RichText::new(if folded { "+" } else { "−" }).size(14.0)).frame(false)).clicked() {
                FOLDED.with(|f| f.set(!folded));
            }
            ui.label(RichText::new(app.tr("ui.heading_sketch_palette")).strong().size(12.5));
        });
        if FOLDED.with(Cell::get) {
            return;
        }
        let ofold = OPTIONS_FOLDED.with(Cell::get);
        if ui
            .add(egui::Button::new(RichText::new(format!("{} {}", if ofold { "+" } else { "−" }, app.tr("ui.options"))).size(12.0)).frame(false))
            .clicked()
        {
            OPTIONS_FOLDED.with(|f| f.set(!ofold));
        }
        if !ofold {
            let options_height = (bottom - ui.cursor().top() - 68.0).max(24.0);
            let options =
                egui::ScrollArea::vertical().id_salt("sc_palette_options").max_height(options_height).auto_shrink([false, true]).show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(app.tr("ui.linetype"));
                        ui.horizontal(|ui| {
                            // With a selection: convert it; without: the mode new curves are drawn in.
                            let current = mode(app);
                            for (i, (label, tip)) in [("—", "Normal"), ("- -", "Construction"), ("-·-", "Centerline")].iter().enumerate() {
                                let on = !has_curves && current == i as u8;
                                let tr_tip = app.tr_label(tip);
                                let tip = if has_curves {
                                    crate::i18n::format("sketch.make_selection", &[("linetype", tr_tip)])
                                } else {
                                    crate::i18n::format("sketch.draw_curves", &[("linetype", tr_tip)])
                                };
                                if ui.add(egui::Button::new(*label).selected(on).min_size(vec2(30.0, 20.0))).on_hover_text(tip).clicked() {
                                    linetype = Some(i as u8);
                                }
                            }
                        });
                        ui.label(app.tr("ui.look_at"));
                        if ui.button(app.tr("ui.look_at")).clicked() {
                            look = true;
                        }
                    });
                    for (k, label) in TOGGLES {
                        // The label is part of the checkbox: clicking it toggles too.
                        let mut on = shown(k);
                        if ui.checkbox(&mut on, app.tr_label(label)).changed() {
                            set = Some((k.to_string(), on));
                        }
                    }
                });
            crate::scenario::publish_rect("sketch_palette:options", options.inner_rect);
        }
        ui.add_space(4.0);
        ui.label(RichText::new(crate::i18n::tr_dyn(&dof_line)).color(if ok { t.sketch_accent } else { t.text_dim }));
        ui.add_space(4.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let button = ui.add(egui::Button::new(RichText::new(app.tr("ui.finish_sketch")).color(Color32::WHITE)).fill(t.accent));
            crate::scenario::publish_rect("sketch_palette:finish", button.rect);
            crate::scenario::publish_handle("sketch_palette:finish", button.rect.center());
            if button.clicked() {
                finish = true;
            }
        });
    });
    app.viewport.palette_rect = response.map(|r| r.response.rect);
    if let Some(r) = app.viewport.palette_rect {
        crate::scenario::publish_rect("sketch_palette:outer", r);
    }
    if let Some((k, on)) = set {
        let mut p = serde_json::Map::new();
        p.insert(k, Value::Bool(on));
        if let Err(e) = app.run("sketch.options", Value::Object(p)) {
            app.set_status(e, true);
        }
    }
    if let Some(k) = linetype {
        if has_curves {
            set_linetype(app, k);
        } else {
            set_mode(app, k);
        }
    }
    if look {
        crate::dialogs::look_at_sketch(app);
    }
    if finish {
        app.finish_sketch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_footer_and_scrolled_lower_option_remain_reachable_at_scale_and_resize() {
        for (scale, height) in [(1.0, 900.0), (2.0, 900.0), (2.0, 720.0)] {
            FOLDED.with(|v| v.set(false));
            OPTIONS_FOLDED.with(|v| v.set(false));
            let mut h = crate::scenario::Harness::new();
            h.app.run("sketch.create", json!({"plane":"XY"})).unwrap();
            h.app.run("sketch.circle.center", json!({"center":[0,0],"radius":1})).unwrap();
            h.call("ui.resize", json!({"width":1400,"height":height}));
            h.call("ui.prefs", json!({"ui_scale":scale}));
            h.settle();
            let baseline = h.app.session.doc.to_json();
            let undo = h.app.session.undo.len();
            let vp = h.app.viewport.rect.unwrap();
            let footer = crate::scenario::rect_of("sketch_palette:finish").unwrap();
            assert!(vp.contains_rect(footer), "scale{scale} height{height}: {footer:?} outside {vp:?}");
            let options = crate::scenario::rect_of("sketch_palette:options").unwrap();
            assert!(vp.contains_rect(options));
            h.call("ui.scroll", json!({"x":options.center().x,"y":options.center().y,"delta":-1000}));
            h.settle();
            assert_eq!(h.app.session.doc.to_json(), baseline);
            assert_eq!(h.app.session.undo.len(), undo);
            let [x, y] = h.widget("3D Sketch").unwrap();
            assert!(options.contains(egui::pos2(x as f32, y as f32)), "lower option must be visible after scrolling");
            h.call("ui.click", json!({"x":x,"y":y}));
            h.settle();
            assert!(h.app.session.world_state().sketch(h.app.session.active_sketch.unwrap()).unwrap().sketch.view.three_d);
            assert_eq!(h.app.session.undo.len(), undo + 1);
            h.app.run("edit.undo", json!({})).unwrap();
            assert_eq!(h.app.session.doc.to_json(), baseline);
            h.settle();
            let footer = crate::scenario::rect_of("sketch_palette:finish").unwrap();
            h.call("ui.click", json!({"x":footer.center().x,"y":footer.center().y}));
            h.settle();
            assert!(h.app.session.active_sketch.is_none(), "Finish Sketch must work at scale{scale} height{height}");
            assert_eq!(h.app.session.doc.to_json(), baseline);
        }
    }

    #[test]
    fn dof_readout() {
        assert_eq!(dof_text(0, true), "Fully constrained");
        assert_eq!(dof_text(1, true), "1 degree of freedom");
        assert_eq!(dof_text(4, true), "4 degrees of freedom");
        assert_eq!(dof_text(4, false), "Constraints conflict");
    }
}
