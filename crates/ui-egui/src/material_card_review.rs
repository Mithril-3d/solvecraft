//! Candidate review through the existing validator, independent of assigned-card persistence.
use egui::RichText;
use serde_json::{Value, json};
use solidcraft_engine::doc::physical_material::{Family, MaterialCard, PropertyData, admit_card};

use crate::{SolveApp, i18n::tr, theme::Tokens};

const MAX_BYTES: usize = 256 * 1024;
const KINDS: [&str; 9] = [
    "density",
    "elasticity",
    "tensile_strength",
    "compressive_strength",
    "yield_strength",
    "shear_strength",
    "thermal_conductivity",
    "thermal_expansion",
    "specific_heat",
];

#[derive(Default)]
pub struct ReviewWindow {
    input: String,
    reviewed: Option<(MaterialCard, Value)>,
    error: Option<String>,
}

impl ReviewWindow {
    fn review(&mut self, app: &mut SolveApp) {
        self.reviewed = None;
        let result = (|| {
            if self.input.len() > MAX_BYTES {
                return Err(tr("ui.material_review.limit").to_string());
            }
            let value: Value = serde_json::from_str(&self.input).map_err(|e| e.to_string())?;
            let report = app.run("material.properties.validate", json!({"card":value}))?;
            let card = admit_card(&value).map_err(|e| e.to_string())?;
            Ok((card, report))
        })();
        match result {
            Ok(reviewed) => {
                self.reviewed = Some(reviewed);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

/// Open only through ui.start; executing the validator never opens a window.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut window) = app.material_card_review.take() else { return };
    let mut open = true;
    let mut close = false;
    crate::frame::window(ctx, tr("ui.material_review.title"), crate::frame::Width::Wide)
        .id(egui::Id::new("sc_material_card_review"))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(tr("ui.material_review.scope"));
            egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 180.0).max(100.0)).show(ui, |ui| {
                ui.label(tr("ui.material_review.input"));
                let response = ui.add(
                    egui::TextEdit::multiline(&mut window.input)
                        .id(egui::Id::new("sc_material_card_input"))
                        .desired_width(f32::INFINITY)
                        .desired_rows(5)
                        .char_limit(MAX_BYTES),
                );
                if response.changed() {
                    window.reviewed = None;
                    window.error = None;
                }
                if ui.button(tr("ui.material_review.review")).clicked() {
                    window.review(app);
                }
                if let Some(error) = &window.error {
                    ui.colored_label(Tokens::get().error, error);
                }
                if let Some((card, report)) = &window.reviewed {
                    show_card(ui, card, report);
                }
            });
            ui.separator();
            close = ui.button(tr("ui.close")).clicked();
        });
    if open && !close {
        app.material_card_review = Some(window);
    }
}

fn value(ui: &mut egui::Ui, field: &str, value: Option<String>, unit: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.monospace(field);
        ui.label(value.unwrap_or_else(|| tr("ui.material_review.unknown").to_string()));
        ui.label(unit);
    });
}

fn numbers(ui: &mut egui::Ui, field: &str, values: &[f64], unit: &str) {
    value(ui, field, Some(values.iter().map(|v| format!("{v:e}")).collect::<Vec<_>>().join(", ")), unit);
}

fn show_card(ui: &mut egui::Ui, card: &MaterialCard, report: &Value) {
    ui.separator();
    ui.heading(&card.designation);
    // Schema identifiers are inert metadata, not translated source claims or assigned body axes.
    let family = serde_json::to_value(card.family).unwrap_or(Value::Null);
    value(ui, "family", family.as_str().map(str::to_string), "");
    ui.strong(tr("ui.material_review.properties"));
    for kind in KINDS {
        let label = format!("ui.material_review.{kind}");
        let Some(property) = card.properties.iter().find(|p| p.data.kind() == kind) else {
            ui.label(format!("{}: {}", tr(&label), tr("ui.material_review.unknown")));
            continue;
        };
        egui::CollapsingHeader::new(tr(&label)).id_salt(kind).default_open(true).show(ui, |ui| {
            show_data(ui, &property.data);
            if let Some(axes) = serde_json::to_value(&property.data)
                .ok()
                .and_then(|d| d.get("axes").or_else(|| d.get("model").and_then(|m| m.get("axes"))).and_then(Value::as_str).map(str::to_string))
            {
                let axes = if axes == "wood_lrt" { "wood_lrt (1=L, 2=R, 3=T)".to_string() } else { axes };
                value(ui, "axes", Some(axes), "");
                ui.label(tr("ui.material_review.axes"));
            }
            ui.strong(tr("ui.material_review.conditions"));
            let c = &property.conditions;
            value(ui, "temperature_k", c.temperature_k.map(|v| v.to_string()), "K");
            value(ui, "state", c.state.clone(), "");
            if card.family == Family::Wood || c.moisture_dry_basis_kg_kg.is_some() {
                value(ui, "moisture_dry_basis_kg_kg", c.moisture_dry_basis_kg_kg.map(|v| v.to_string()), tr("ui.material_review.moisture_unit"));
            }
            if card.family == Family::Polymer || c.strain_rate_per_s.is_some() {
                value(ui, "strain_rate_per_s", c.strain_rate_per_s.map(|v| v.to_string()), "s⁻¹");
            }
            ui.strong(tr("ui.material_review.source"));
            if let Some(source) = card.sources.iter().find(|s| s.id == property.source_id) {
                ui.label(&source.title);
                value(ui, "source_id", Some(source.id.clone()), "");
                value(ui, "kind", serde_json::to_value(source.kind).ok().and_then(|v| v.as_str().map(str::to_string)), "");
                value(ui, "license", source.license.clone(), "");
                value(ui, "reference", source.reference.clone(), "");
            }
        });
    }
    if let Some(moduli) = report.get("validation").and_then(|v| v.get("isotropic_moduli")).filter(|v| v.is_object()) {
        ui.strong(tr("ui.material_review.derived"));
        for field in ["shear_pa", "bulk_pa"] {
            value(ui, field, moduli.get(field).and_then(Value::as_f64).map(|v| format!("{v:e}")), "Pa");
        }
    }
    ui.label(RichText::new(tr("ui.material_review.claims")).small().color(Tokens::get().text_dim));
}

fn show_data(ui: &mut egui::Ui, data: &PropertyData) {
    match data {
        PropertyData::Density { kg_m3 } => numbers(ui, "density", &[*kg_m3], "kg/m³"),
        PropertyData::Elasticity { model } => match model {
            solidcraft_engine::doc::physical_material::Elasticity::Isotropic { young_modulus_pa, poisson_ratio } => {
                numbers(ui, "E", &[*young_modulus_pa], "Pa");
                numbers(ui, "ν", &[*poisson_ratio], "");
            }
            solidcraft_engine::doc::physical_material::Elasticity::Orthotropic { young_moduli_pa, shear_moduli_pa, poisson_ratios, .. } => {
                numbers(ui, "E1, E2, E3", young_moduli_pa, "Pa");
                numbers(ui, "G12, G23, G31", shear_moduli_pa, "Pa");
                numbers(ui, "ν12, ν23, ν31", poisson_ratios, "");
            }
        },
        PropertyData::TensileStrength { pa } | PropertyData::CompressiveStrength { pa } | PropertyData::YieldStrength { pa } => {
            numbers(ui, "σ", &[*pa], "Pa");
        }
        PropertyData::ShearStrength { pa } => numbers(ui, "τ", &[*pa], "Pa"),
        PropertyData::ThermalConductivity { w_m_k, .. } => numbers(ui, "k1, k2, k3", w_m_k, "W/(m·K)"),
        PropertyData::ThermalExpansion { per_k, .. } => numbers(ui, "α1, α2, α3", per_k, "K⁻¹"),
        PropertyData::SpecificHeat { j_kg_k } => numbers(ui, "c", &[*j_kg_k], "J/(kg·K)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::Harness;
    use std::sync::Arc;

    fn candidate() -> Value {
        json!({"schema_version":1,"designation":"Original synthetic candidate","family":"wood",
            "sources":[{"id":"example","title":"Original illustrative <b>source</b>","kind":"illustrative",
                "license":"Unverified licence claim","reference":"https://invalid.example/not-fetched"}],
            "properties":[{"source_id":"example","data":{"kind":"elasticity","model":{
                "symmetry":"isotropic","young_modulus_pa":12e9,"poisson_ratio":0.25}},
                "conditions":{"temperature_k":293.15,"moisture_dry_basis_kg_kg":0.,"state":"synthetic specimen"}}]})
    }

    #[test]
    fn clicked_candidate_review_and_close_preserve_design_history_and_unknowns() {
        let mut h = Harness::new();
        h.app.run("solid.box", json!({"length":10,"width":10,"height":10})).unwrap();
        h.app.run("edit.undo", json!({})).unwrap();
        let doc = h.app.session.doc.clone();
        let before = (
            h.app.session.revision,
            h.app.session.model.is_stale(),
            h.app.session.undo.len(),
            h.app.session.redo.len(),
            h.app.session.selection.clone(),
        );
        h.step(&json!({"start":"material.properties.validate"})).unwrap();
        h.app.material_card_review.as_mut().unwrap().input = candidate().to_string();
        h.frames(2);
        h.step(&json!({"widget":"Review card"})).unwrap();
        let (card, report) = h.app.material_card_review.as_ref().unwrap().reviewed.as_ref().unwrap();
        assert_eq!(card.designation, "Original synthetic candidate");
        assert_eq!(card.properties.len(), 1, "missing values were not filled with zeros");
        assert_eq!(card.sources[0].title, "Original illustrative <b>source</b>");
        assert_eq!(card.properties[0].conditions.moisture_dry_basis_kg_kg, Some(0.));
        assert_eq!(report["validation"]["isotropic_moduli"], json!({"shear_pa":4.8e9,"bulk_pa":8e9}));
        assert_eq!(report["validation"]["source_review_required"], json!(["example"]));
        assert!(h.widget("Original synthetic candidate").is_some());
        assert!(h.widget("Density: Unknown").is_some());
        assert!(h.widget("Pa").is_some());
        h.ctx.memory_mut(|m| m.request_focus(egui::Id::new("sc_material_card_input")));
        h.step(&json!({"text":" "})).unwrap();
        assert!(h.app.material_card_review.as_ref().unwrap().reviewed.is_none(), "editing invalidates the old report");
        h.step(&json!({"widget":"Close"})).unwrap();
        assert!(h.app.material_card_review.is_none());
        assert!(Arc::ptr_eq(&doc, &h.app.session.doc));
        assert_eq!(
            before,
            (
                h.app.session.revision,
                h.app.session.model.is_stale(),
                h.app.session.undo.len(),
                h.app.session.redo.len(),
                h.app.session.selection.clone()
            )
        );
    }

    #[test]
    fn invalid_and_oversized_candidates_never_keep_a_previous_review() {
        let mut app = SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
        let doc = app.session.doc.clone();
        let mut window = ReviewWindow { input: candidate().to_string(), ..Default::default() };
        window.review(&mut app);
        assert!(window.reviewed.is_some());
        for input in ["[".into(), "{\"schema_version\":2}".into(), "é".repeat(MAX_BYTES / 2 + 1)] {
            window.input = input;
            window.review(&mut app);
            assert!(window.reviewed.is_none());
            assert!(window.error.is_some());
        }
        assert!(window.error.as_deref().unwrap().contains("256 KiB"));
        assert!(Arc::ptr_eq(&doc, &app.session.doc));
        assert!(app.session.undo.is_empty() && app.session.redo.is_empty());
        assert!(!app.session.is_dirty());
    }

    #[test]
    fn every_property_variant_renders_its_declared_si_unit_and_inert_source() {
        let fixtures = [
            (json!({"kind":"density","kg_m3":600.}), "kg/m³"),
            (
                json!({"kind":"elasticity","model":{"symmetry":"orthotropic","axes":"wood_lrt",
                "young_moduli_pa":[12e9,3e9,1e9],"shear_moduli_pa":[2e9,1e9,0.5e9],"poisson_ratios":[0.,0.,0.]}}),
                "Pa",
            ),
            (json!({"kind":"tensile_strength","pa":1e8}), "Pa"),
            (json!({"kind":"compressive_strength","pa":1e8}), "Pa"),
            (json!({"kind":"yield_strength","pa":1e8}), "Pa"),
            (json!({"kind":"shear_strength","pa":1e8}), "Pa"),
            (json!({"kind":"thermal_conductivity","w_m_k":[1.,2.,3.],"axes":"material123"}), "W/(m·K)"),
            (json!({"kind":"thermal_expansion","per_k":[-1e-6,0.,2e-6],"axes":"material123"}), "K⁻¹"),
            (json!({"kind":"specific_heat","j_kg_k":500.}), "J/(kg·K)"),
        ];
        for (data, unit) in fixtures {
            let mut h = Harness::new();
            h.app.start("material.properties.validate");
            let mut card = candidate();
            card["properties"][0]["data"] = data;
            h.app.material_card_review.as_mut().unwrap().input = card.to_string();
            h.frames(2);
            h.step(&json!({"widget":"Review card"})).unwrap();
            assert!(h.app.material_card_review.as_ref().unwrap().error.is_none(), "{card}");
            assert!(h.widget(unit).is_some(), "expected {unit} for {card}");
            assert!(h.widget("Original illustrative <b>source</b>").is_some(), "source is displayed as literal text");
        }
    }

    #[test]
    fn executing_json_validation_does_not_open_the_inspector() {
        let mut app = SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
        app.run("material.properties.validate", json!({"card":candidate()})).unwrap();
        assert!(app.material_card_review.is_none());
    }

    #[test]
    fn reviewer_messages_exist_in_every_locale() {
        let keys = [
            "title",
            "scope",
            "input",
            "review",
            "limit",
            "unknown",
            "properties",
            "conditions",
            "source",
            "axes",
            "derived",
            "claims",
            "moisture_unit",
        ];
        for catalog in crate::i18n::LOCALES {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/locales").join(format!("{}.json", catalog.id));
            let messages: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
            for key in keys.into_iter().chain(KINDS) {
                let key = format!("ui.material_review.{key}");
                assert!(messages.get(&key).and_then(Value::as_str).is_some_and(|s| !s.is_empty()), "{}: {key}", catalog.id);
            }
        }
    }
}
