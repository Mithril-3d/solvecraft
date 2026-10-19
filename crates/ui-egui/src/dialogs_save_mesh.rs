//! The Save as mesh dialog (`file.save_mesh`): which bodies, the mesh format, and optionally a
//! slicer to open the saved file in.
//!
//! Every place that saves a mesh opens this dialog: File › Save as mesh, the Mesh tab, and the
//! Save As Mesh item of the body menus. The slicer list holds the programs the user added (kept
//! between runs, newest first), then the slicers found on this machine, then "Custom…", which asks
//! for a program and adds it at the top; "Custom…" itself is never the selection. Pressing Delete
//! over a program the user added removes it.

use std::collections::HashMap;

use egui::{Color32, Sense, vec2};
use serde_json::{Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::slicers::{self, Slicer};

use crate::SolveApp;
use crate::dialogs::{Kind, combo, row_label};
use crate::selection::{BODIES, SelInput};
use crate::theme::Tokens;

/// The formats, as shown and as `file.save_mesh` names them.
/// (label, `format` parameter, file extension).
const FORMATS: [(&str, &str, &str); 3] = [("STL", "stl", "stl"), ("3MF", "3mf", "3mf"), ("3MF with face tags", "3mf-tagged", "3mf")];
/// Edge of a slicer's icon in the list.
const ICON: f32 = 16.0;
/// Most programs the user can keep in the list.
const MAX_CUSTOM: usize = 24;

/// The dialog's values.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveMesh {
    format: usize,
    send: bool,
    /// The chosen slicer: an installed one's id, or the path of one the user added.
    slicer: String,
    /// The slicer list has not been read for this opening yet.
    stale: bool,
}

/// One line of the slicer list.
struct Entry {
    key: String,
    slicer: Slicer,
    /// Added by the user (can be deleted).
    user: bool,
}

/// The slicers known to the dialog, and their icons.
#[derive(Default)]
pub struct SlicerMenu {
    detected: Vec<Slicer>,
    custom: Vec<(String, Slicer)>,
    icons: HashMap<String, Option<egui::TextureHandle>>,
    /// A program chosen in the Custom… file dialog, waiting for the open dialog to take it.
    picked: Option<String>,
}

impl SlicerMenu {
    /// Look at the machine and at the saved programs again.
    pub fn refresh(&mut self, custom_paths: &[String]) {
        self.detected = slicers::detect();
        self.set_custom(custom_paths);
    }

    fn set_custom(&mut self, paths: &[String]) {
        // A program that has been uninstalled drops out of the list (it stays in the settings).
        self.custom = paths.iter().filter_map(|p| slicers::custom(p).map(|s| (p.clone(), s))).collect();
    }

    fn entries(&self) -> Vec<Entry> {
        let user = self.custom.iter().map(|(path, s)| Entry { key: path.clone(), slicer: s.clone(), user: true });
        let found = self.detected.iter().map(|s| Entry { key: s.id.clone(), slicer: s.clone(), user: false });
        user.chain(found).collect()
    }
}

/// The dialog for `file.save_mesh`, with the choices of last time.
pub fn start(app: &SolveApp, id: &str) -> Option<(Kind, Vec<SelInput>)> {
    if id != "file.save_mesh" {
        return None;
    }
    let p = &app.preferences;
    let format = FORMATS.iter().position(|f| f.1 == p.save_mesh_format).unwrap_or(0);
    let kind = Kind::SaveMesh(SaveMesh { format, send: p.save_mesh_send, slicer: p.save_mesh_slicer.clone(), stale: true });
    Some((kind, vec![SelInput::new("Bodies", BODIES, true)]))
}

/// A popup with the slicer list is open (Delete belongs to it, not to the design's selection).
pub fn popup_open(ctx: &egui::Context) -> bool {
    let frame = ctx.cumulative_frame_nr();
    let at = ctx.data(|d| d.get_temp::<u64>(egui::Id::new("sc_slicer_popup")));
    at.is_some_and(|n| n + 1 >= frame)
}

/// The icon of an entry as a texture (`None`: no icon could be read).
fn icon(menu: &mut SlicerMenu, ctx: &egui::Context, e: &Entry) -> Option<egui::TextureId> {
    let tex = menu.icons.entry(e.key.clone()).or_insert_with(|| {
        let bytes = e.slicer.icon.as_ref()?.load()?;
        let img = decode(&bytes)?;
        Some(ctx.load_texture(format!("slicer-icon-{}", e.key), img, egui::TextureOptions::LINEAR))
    });
    tex.as_ref().map(egui::TextureHandle::id)
}

/// An icon file (PNG or ICO) as a small square image.
fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    const SIDE: u32 = 48;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let img = image::load_from_memory(bytes).ok()?;
    let small = image::imageops::resize(&img.to_rgba8(), SIDE, SIDE, image::imageops::FilterType::Lanczos3);
    Some(egui::ColorImage::from_rgba_unmultiplied([SIDE as usize, SIDE as usize], small.as_raw()))
}

/// An icon, or the first letter of the name on a tile when there is none.
fn paint_icon(ui: &egui::Ui, rect: egui::Rect, tex: Option<egui::TextureId>, name: &str) {
    match tex {
        Some(id) => {
            ui.painter().image(id, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            let t = Tokens::get();
            ui.painter().rect_filled(rect, 3.0, t.chip);
            let letter = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(11.0), t.text);
        }
    }
}

/// The rows after the Bodies input; true when Enter was pressed in a field.
pub fn rows(app: &mut SolveApp, ui: &mut egui::Ui, k: &mut SaveMesh) -> bool {
    if k.stale {
        // Look at the machine once per opening, then settle on a slicer that exists.
        k.stale = false;
        app.slicer_menu.picked = None;
        app.slicer_menu.refresh(&app.preferences.custom_slicers);
        let entries = app.slicer_menu.entries();
        if !entries.iter().any(|e| e.key == k.slicer) {
            k.slicer = entries.first().map(|e| e.key.clone()).unwrap_or_default();
        }
        k.send &= !entries.is_empty();
    }
    row_label(ui, "Format");
    combo(ui, "sm_format", &FORMATS.map(|f| f.0), &mut k.format);
    ui.end_row();

    // A browser has no installed programs to send a file to.
    if app.services.pick_program.is_none() {
        k.send = false;
        return false;
    }
    row_label(ui, "Send to slicer");
    ui.checkbox(&mut k.send, "");
    ui.end_row();

    row_label(ui, "Slicer");
    if let Some(path) = app.slicer_menu.picked.take() {
        match add_custom(app, &path) {
            Ok(()) => k.slicer = path,
            Err(e) => app.set_status(e, true),
        }
    }
    let choosing = app.file_picker_busy();
    ui.add_enabled_ui(k.send && !choosing, |ui| slicer_combo(app, ui, k));
    ui.end_row();
    false
}

fn slicer_combo(app: &mut SolveApp, ui: &mut egui::Ui, k: &mut SaveMesh) {
    let entries = app.slicer_menu.entries();
    let ctx = ui.ctx().clone();
    let current = entries.iter().find(|e| e.key == k.slicer);
    let pad = "\u{2003}\u{2003}";
    let text = current.map_or_else(|| crate::i18n::tr_label("No slicer found").to_string(), |e| format!("{pad}{}", e.slicer.name));
    let mut pick_custom = false;
    let mut delete: Option<String> = None;
    let mut chosen: Option<String> = None;
    let r = egui::ComboBox::from_id_salt("sm_slicer").selected_text(text).width(crate::dialogs::FIELD_W).show_ui(ui, |ui| {
        let frame = ctx.cumulative_frame_nr();
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("sc_slicer_popup"), frame));
        for e in &entries {
            let tex = icon(&mut app.slicer_menu, &ctx, e);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
            let selected = e.key == k.slicer;
            if resp.hovered() || selected {
                let v = ui.visuals();
                let fill = if resp.hovered() { v.widgets.hovered.bg_fill } else { v.selection.bg_fill.gamma_multiply(0.5) };
                ui.painter().rect_filled(rect, 2.0, fill);
            }
            paint_icon(ui, egui::Rect::from_min_size(rect.left_center() + vec2(3.0, -ICON / 2.0), vec2(ICON, ICON)), tex, &e.slicer.name);
            ui.painter().text(
                rect.left_center() + vec2(ICON + 9.0, 0.0),
                egui::Align2::LEFT_CENTER,
                &e.slicer.name,
                egui::FontId::proportional(13.0),
                ui.visuals().text_color(),
            );
            crate::scenario::publish_handle(&format!("dialog:Slicer:{}", e.slicer.name), rect.center());
            if e.user {
                let hint = crate::i18n::format("ui.slicer_delete_tip", &[("program", e.slicer.command.first().map_or("", String::as_str))]);
                resp.clone().on_hover_text(hint);
                if resp.hovered() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
                    delete = Some(e.key.clone());
                }
            }
            if resp.clicked() {
                chosen = Some(e.key.clone());
                ui.close();
            }
        }
        if !entries.is_empty() {
            ui.separator();
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
        }
        ui.painter().text(
            rect.left_center() + vec2(ICON + 9.0, 0.0),
            egui::Align2::LEFT_CENTER,
            crate::i18n::tr_label("Custom…"),
            egui::FontId::proportional(13.0),
            Tokens::get().text,
        );
        crate::scenario::publish_handle("dialog:Slicer:Custom", rect.center());
        if resp.on_hover_text(crate::i18n::tr("ui.custom_program_tip")).clicked() {
            pick_custom = true;
            ui.close();
        }
    });
    if let Some(e) = current {
        let tex = icon(&mut app.slicer_menu, &ctx, e);
        let rect = r.response.rect;
        let at = egui::Rect::from_min_size(rect.left_center() + vec2(ui.spacing().button_padding.x + 1.0, -ICON / 2.0), vec2(ICON, ICON));
        paint_icon(ui, at, tex, &e.slicer.name);
    }
    if let Some(key) = chosen {
        k.slicer = key;
    }
    if let Some(key) = delete {
        app.preferences.custom_slicers.retain(|p| *p != key);
        app.slicer_menu.set_custom(&app.preferences.custom_slicers);
        if k.slicer == key {
            k.slicer = app.slicer_menu.entries().first().map(|e| e.key.clone()).unwrap_or_default();
        }
    }
    if pick_custom {
        // Closing the file dialog without a program leaves the choice as it was.
        app.pick_program(|app, path| app.slicer_menu.picked = path);
    }
}

/// Put a program the user picked at the top of the list (and keep it).
fn add_custom(app: &mut SolveApp, path: &str) -> Result<(), String> {
    if slicers::custom(path).is_none() {
        return Err(crate::i18n::format("ui.not_a_program", &[("path", path)]));
    }
    let list = &mut app.preferences.custom_slicers;
    list.retain(|p| p != path);
    list.insert(0, path.to_string());
    list.truncate(MAX_CUSTOM);
    app.slicer_menu.set_custom(&app.preferences.custom_slicers);
    Ok(())
}

fn bodies(inputs: &[SelInput]) -> Vec<String> {
    inputs
        .first()
        .map(|i| i.items.iter().filter_map(|s| if let Sel::Body { name } = s { Some(name.clone()) } else { None }).collect())
        .unwrap_or_default()
}

/// What the Save (or Send) button asks `file.save_mesh` to do, without the file name.
pub fn commands(k: &SaveMesh, inputs: &[SelInput]) -> Result<Vec<(String, Value)>, String> {
    let format = FORMATS.get(k.format).map_or("stl", |f| f.1);
    let mut p = json!({ "format": format });
    let names = bodies(inputs);
    if names.is_empty() {
        return Err(crate::i18n::tr("ui.save_mesh_pick_bodies").into());
    }
    p["bodies"] = json!(names);
    if k.send {
        if k.slicer.is_empty() {
            return Err(crate::i18n::tr("ui.save_mesh_no_slicer").into());
        }
        p["slicer"] = json!(k.slicer);
    }
    Ok(vec![("file.save_mesh".into(), p)])
}

/// Save and Send need at least one body.
pub fn ready(inputs: &[SelInput]) -> bool {
    !bodies(inputs).is_empty()
}

/// The button's label: Save, or Send when the file goes to a slicer.
pub fn button(k: &SaveMesh) -> &'static str {
    if k.send { "Send" } else { "Save" }
}

/// Save: ask for a file name and write the file. Send: write it to a temporary file and start the
/// slicer with it, with no file dialog.
pub fn run(app: &mut SolveApp, k: &SaveMesh, inputs: &[SelInput]) -> Result<(), String> {
    let (_, params) = commands(k, inputs)?.into_iter().next().ok_or("nothing to save")?;
    let (format, ext) = FORMATS.get(k.format).map_or(("stl", "stl"), |f| (f.1, f.2));
    let name = bodies(inputs).first().cloned().unwrap_or_else(|| app.session.doc.name.clone());
    let p = &mut app.preferences;
    p.save_mesh_format = format.into();
    p.save_mesh_send = k.send;
    if !k.slicer.is_empty() {
        p.save_mesh_slicer = k.slicer.clone();
    }
    if k.send {
        let dir = std::env::temp_dir().join("solidcraft-slicer");
        std::fs::create_dir_all(&dir)
            .map_err(|e| crate::i18n::format("ui.save_mesh_temp_failed", &[("folder", &dir.display().to_string()), ("error", &e.to_string())]))?;
        let file: String = name.chars().map(|c| if c.is_alphanumeric() || "-_. ".contains(c) { c } else { '_' }).collect();
        save(app, params, &dir.join(format!("{file}.{ext}")).to_string_lossy());
    } else {
        // The file dialog answers later; the dialog closes meanwhile, as for every export.
        app.pick_save(&format!("{name}.{ext}"), &[ext], move |app, path| {
            if let Some(path) = path {
                save(app, params, &path);
            }
        });
    }
    Ok(())
}

/// Run `file.save_mesh` into `path` and report what happened.
fn save(app: &mut SolveApp, mut params: Value, path: &str) {
    // Runs from the file picker's answer, outside the frame's language scope.
    let _language = crate::i18n::scope(&app.preferences.language);
    params["path"] = json!(path);
    let Ok(result) = app.run("file.save_mesh", params) else { return };
    match result.get("slicer_error").and_then(Value::as_str) {
        Some(e) => app.set_status(e.to_string(), true),
        None => match result.get("slicer").and_then(Value::as_str) {
            Some(s) => app.set_status(crate::i18n::format("ui.sent_to_slicer", &[("slicer", s)]), false),
            None => app.set_status(crate::i18n::format("ui.saved_to", &[("path", path)]), false),
        },
    }
}
