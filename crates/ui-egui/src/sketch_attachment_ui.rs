//! Reviewable sketch-plane rebinding. Saved lost planes are repair guides, never model geometry.

use egui::{Pos2, RichText};
use serde_json::{Value, json};
use solvecraft_engine::Sel;
use solvecraft_engine::doc::face_anchor::plane_face_anchor;
use solvecraft_engine::doc::{EmbossFace, FaceAttachmentStatus, FeatureKind, mat_inverse};
use solvecraft_engine::geom::{Plane, Vec3};

use crate::SolveApp;
use crate::theme::Tokens;

#[derive(Clone, Debug)]
pub struct Draft {
    pub sketch: u64,
    pub candidate: Option<Value>,
    pub label: String,
    pub error: Option<String>,
    pub author_path: Option<Vec<u64>>,
}

impl Draft {
    pub fn new(app: &SolveApp, sketch: u64) -> Self {
        let author_path = app.session.doc.feature(sketch).and_then(|feature| match &feature.kind {
            FeatureKind::Sketch { plane, .. } if plane_face_anchor(plane).is_some() => {
                plane_face_anchor(plane).map(|anchor| anchor.author_path.clone())
            }
            _ if feature.component == app.session.active_component => app.session.active_occurrence_path.clone(),
            _ => None,
        });
        Self { sketch, candidate: None, label: String::new(), error: None, author_path }
    }

    fn choose(&mut self, plane: Value, label: String) {
        self.candidate = Some(plane);
        self.label = label;
        self.error = None;
    }

    pub fn commands(&self) -> Option<Vec<(String, Value)>> {
        let mut params = json!({"sketch":self.sketch,"plane":self.candidate.as_ref()?});
        if let Some(path) = &self.author_path {
            params["author_path"] = json!(path);
        }
        Some(vec![("sketch.redefine".into(), params)])
    }

    pub fn choose_saved_fixed(&mut self, app: &SolveApp, world: Plane) -> Result<(), String> {
        let doc = app.session.placed_document();
        let owner = doc.feature(self.sketch).ok_or("Sketch no longer exists")?.component;
        let path = self.author_path.clone().unwrap_or_else(|| doc.first_path(owner));
        if self.author_path.is_none()
            && path.iter().any(|id| {
                doc.occurrences
                    .iter()
                    .find(|o| o.id == *id)
                    .is_none_or(|selected| doc.occurrences.iter().filter(|o| o.component == selected.component).count() != 1)
            })
        {
            return Err("Select an exact author placement before converting this plane".into());
        }
        let frame = EmbossFace::component_frame(&doc, owner, &path).map_err(|error| error.to_string())?;
        mat_inverse(&frame).ok_or("Sketch author frame is invalid")?;
        self.author_path = Some(path);
        // Public commands take world geometry. Session maps Custom planes into
        // this exact author placement once, including redefine's saved path.
        self.choose(fixed_plane(world), "Fixed plane: independent of the source body".into());
        Ok(())
    }
}

pub fn clear(app: &mut SolveApp) {
    app.tree.redefine = None;
    app.tree.redefine_draft = None;
}

pub fn commands(app: &SolveApp) -> Option<Vec<(String, Value)>> {
    let draft = app.tree.redefine_draft.as_ref()?;
    (app.tree.redefine == Some(draft.sketch)).then(|| draft.commands()).flatten()
}

fn attachment(app: &SolveApp, sketch: u64) -> FaceAttachmentStatus {
    app.session.placed_document().sketch_face_attachment(sketch, &app.session.model)
}

fn fixed_plane(plane: Plane) -> Value {
    json!({"origin":plane.origin,"x_dir":plane.x,"y_dir":plane.y})
}

pub fn face_plane(app: &SolveApp, selection: &Sel) -> Option<Value> {
    match selection {
        Sel::Face { body, index, point } => {
            let doc = app.session.placed_document();
            let (_, _, _, path, _) = doc.body_frames_with_source(&app.session.model.state()).into_iter().find(|(world, _, _, _, _)| world == body)?;
            Some(json!({"face":point,"body":body,"index":index,"path":path}))
        }
        _ => None,
    }
}

fn face_label(app: &SolveApp, selection: &Sel) -> String {
    let Sel::Face { body, .. } = selection else { return String::new() };
    let placed = app.session.placed_document();
    if let Some((_, source, _, path, _)) =
        placed.body_frames_with_source(&app.session.model.state()).into_iter().find(|(world, _, _, _, _)| world == body)
    {
        format!("Face on {source} / {}", path_label(app, &path))
    } else {
        format!("Face on {body}")
    }
}

fn path_label(app: &SolveApp, path: &[u64]) -> String {
    if path.is_empty() {
        return "Root".into();
    }
    (1..=path.len())
        .map(|end| app.session.occurrence_placement(&path[..end]).map(|placement| placement.name).unwrap_or_else(|_| "Missing occurrence".into()))
        .collect::<Vec<_>>()
        .join(" / ")
}

pub fn panel(app: &mut SolveApp, ctx: &egui::Context, at: Pos2) {
    let Some(sketch) = app.tree.redefine else { return };
    if app.session.doc.feature(sketch).is_none() {
        clear(app);
        return;
    }
    let mut draft = app.tree.redefine_draft.take().filter(|draft| draft.sketch == sketch).unwrap_or_else(|| Draft::new(app, sketch));
    let status = attachment(app, sketch);
    let name = app.session.doc.feature(sketch).map(|feature| feature.name.as_str()).unwrap_or("Sketch");
    let selected = app.session.selection.iter().find(|selection| matches!(selection, Sel::Face { .. })).cloned();
    let mut open = true;
    let mut apply = false;
    let mut cancel = false;
    let t = Tokens::get();
    crate::frame::docked(
        format!("Redefine Sketch Plane: {name}"),
        egui::Id::new("sc_redefine"),
        at,
        app.viewport.rect.map(|rect| rect.bottom()).unwrap_or_else(|| ctx.content_rect().bottom()) - at.y - 8.0,
        crate::frame::Width::Normal,
    )
    .open(&mut open)
    .scroll([false, true])
    .show(ctx, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.strong(format!("Redefine Sketch Plane: {name}"));
        ui.separator();
        let saved_plane = match &status {
            FaceAttachmentStatus::Healthy { plane, target, path } => {
                ui.label(format!("Attached to {} / {}", target.body, path_label(app, path)));
                Some(*plane)
            }
            FaceAttachmentStatus::Lost { message, repair_plane } => {
                ui.label(RichText::new("Face attachment lost").color(t.error));
                ui.label(message);
                if repair_plane.is_some() {
                    ui.label("The dashed sketch is a saved plane guide. Select a new face to repair the attachment.");
                } else {
                    ui.label("The saved author occurrence is missing. Select a valid author placement before rebinding.");
                }
                *repair_plane
            }
            FaceAttachmentStatus::Legacy { plane } => {
                ui.label("Legacy face attachment. Rebinding to a certified face creates an exact attachment.");
                *plane
            }
            FaceAttachmentStatus::Fixed { plane } => {
                ui.label("Sketch plane has no body-face attachment.");
                *plane
            }
        };
        ui.separator();
        if matches!(&status, FaceAttachmentStatus::Lost { repair_plane: None, .. }) {
            let author = app.session.doc.feature(sketch).map(|feature| feature.component);
            let available = author == Some(app.session.active_component);
            if ui.add_enabled(available, egui::Button::new("Use active author placement")).clicked() {
                draft.author_path = Some(app.session.active_occurrence_path.clone().unwrap_or_default());
                draft.error = None;
            }
            if available && let Some(path) = &draft.author_path {
                ui.label(format!("Author placement: {}", path_label(app, path)));
            }
        }
        ui.label("Choose a new plane. Sketch geometry and dimensions stay unchanged in its two-dimensional coordinates.");
        ui.horizontal_wrapped(|ui| {
            for plane in ["XY", "XZ", "YZ"] {
                if ui.button(format!("Use {plane} plane")).clicked() {
                    draft.choose(json!(plane), plane.into());
                }
            }
        });
        for feature in &app.session.doc.features {
            if matches!(feature.kind, FeatureKind::ConstructionPlane { .. }) && ui.button(&feature.name).clicked() {
                draft.choose(json!(feature.name), feature.name.clone());
            }
        }
        if ui.add_enabled(selected.is_some(), egui::Button::new("Reattach selected face")).clicked()
            && let Some(selection @ Sel::Face { .. }) = &selected
            && let Some(plane) = face_plane(app, selection)
        {
            let label = face_label(app, selection);
            draft.choose(plane, label);
        }
        if ui.add_enabled(saved_plane.is_some(), egui::Button::new("Use saved fixed plane")).clicked()
            && let Some(plane) = saved_plane
            && let Err(error) = draft.choose_saved_fixed(app, plane)
        {
            draft.error = Some(error);
        }
        if ui.add_enabled(selected.is_some(), egui::Button::new("Use selected face as fixed plane")).clicked()
            && let Some(selection) = &selected
            && let Some(mut plane) = face_plane(app, selection)
        {
            plane["fixed"] = json!(true);
            draft.choose(plane, "Fixed plane: independent of the selected body".into());
        }
        if draft.candidate.is_some() {
            ui.separator();
            ui.label(format!("Preview: {}", draft.label));
        }
        if let Some(error) = draft.error.as_ref().or(app.preview.error.as_ref()) {
            ui.label(RichText::new(error).color(t.error));
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(draft.candidate.is_some() && app.preview.error.is_none(), egui::Button::new("Apply")).clicked() {
                apply = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        cancel = true;
        app.esc_handled = true;
    }
    if apply && let Some(commands) = draft.commands() {
        for (command, params) in commands {
            match app.run(&command, params) {
                Ok(_) => cancel = true,
                Err(error) => draft.error = Some(error),
            }
        }
    }
    if cancel || !open {
        clear(app);
    } else {
        app.tree.redefine_draft = Some(draft);
    }
}

/// Only a lost reference's explicit saved author frame supplies a guide. It is
/// absent from ModelState, selection, snapping and all downstream evaluation.
pub fn lost_guide(app: &SolveApp) -> Option<(Plane, &solvecraft_engine::sketch::Sketch)> {
    let sketch = app.tree.redefine?;
    let FaceAttachmentStatus::Lost { repair_plane: Some(plane), .. } = attachment(app, sketch) else { return None };
    let FeatureKind::Sketch { sketch, .. } = &app.session.doc.feature(sketch)?.kind else { return None };
    Some((plane, sketch))
}

pub fn guide_lines(app: &SolveApp) -> Vec<Vec<Vec3>> {
    let Some((plane, sketch)) = lost_guide(app) else { return Vec::new() };
    (0..sketch.curves.len())
        .flat_map(|index| sketch.segs(index))
        .map(|segment| segment.polyline(0.05).into_iter().map(|point| plane.to_world(point)).collect())
        .collect()
}

pub fn display_guide_lines(app: &SolveApp, proj: &crate::viewport::Proj) -> Vec<Vec<Vec3>> {
    let Some((plane, sketch)) = lost_guide(app) else { return Vec::new() };
    let mut budget = solvecraft_engine::render::SamplingBudget::new(65_536, 262_144);
    (0..sketch.curves.len())
        .flat_map(|index| crate::sketch_display::curve_with_budget(sketch, index, &plane, proj, app.viewport.pixels_per_point(), &mut budget).parts)
        .collect()
}
