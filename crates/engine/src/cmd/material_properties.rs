//! Read-only validation of candidate engineering data; no assignment or simulation.
use super::CommandSpec;
use crate::params::bad;
use crate::{Result, Session};
use serde_json::{Value, json};

pub static COMMANDS: &[CommandSpec] = &[CommandSpec::new("material.properties.validate", "Validate Physical Material Data", validate)
    .icon("material")
    .noundo()
    .doc_only()
    .params("card: schema_version 1 engineering material card in SI units. Returns numerical consistency, derived isotropic moduli, missing conditions and sources needing review. Read-only; does not assign/save properties or provide an analysis solver. See docs/physical-material-cards.md"),
    CommandSpec::new("material.properties.directional", "Directional Elastic Modulus", directional).icon("material").noundo().doc_only()
        .params("card: schema_version 1 engineering material card; direction: three finite nonzero-vector components in the card's material-local axes. Returns normalized direction, Young modulus in Pa and source/condition review. Read-only constitutive query; no structural solver, persistence or body-axis assignment. See docs/physical-material-cards.md")];

fn directional(_: &mut Session, params: &Value) -> Result<Value> {
    let cmd = "material.properties.directional";
    let object = params
        .as_object()
        .filter(|o| o.len() == 2 && o.contains_key("card") && o.contains_key("direction"))
        .ok_or_else(|| bad(cmd, "expected {card: ..., direction: [x,y,z]}"))?;
    let value = object.get("card").ok_or_else(|| bad(cmd, "missing card"))?;
    let direction = object
        .get("direction")
        .and_then(Value::as_array)
        .filter(|v| v.len() == 3)
        .ok_or_else(|| bad(cmd, "direction: expected exactly three numeric components"))?;
    let component = |i| direction.get(i).and_then(Value::as_f64).ok_or_else(|| bad(cmd, "direction: expected numeric components"));
    let direction = [component(0)?, component(1)?, component(2)?];
    let card = solidcraft_doc::physical_material::admit_card(value).map_err(|e| bad(cmd, e.to_string()))?;
    let response = card.directional_modulus(direction).map_err(|e| bad(cmd, e.to_string()))?;
    serde_json::to_value(response).map_err(|e| bad(cmd, e.to_string()))
}

fn validate(_: &mut Session, params: &Value) -> Result<Value> {
    let object = params
        .as_object()
        .filter(|o| o.len() == 1 && o.contains_key("card"))
        .ok_or_else(|| bad("material.properties.validate", "expected {card: ...}"))?;
    let card = object.get("card").ok_or_else(|| bad("material.properties.validate", "missing card"))?;
    let result = solidcraft_doc::physical_material::validate_card(card).map_err(|e| bad("material.properties.validate", e.to_string()))?;
    Ok(json!({"valid":true,"validation":result}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn directional_command_reports_modulus_and_preserves_state_on_refusal() {
        let mut s = Session::default();
        let card = json!({"schema_version":1,"designation":"Synthetic","family":"other",
            "sources":[{"id":"test","title":"Original synthetic fixture","kind":"illustrative"}],
            "properties":[{"source_id":"test","data":{"kind":"elasticity","model":{"symmetry":"isotropic","young_modulus_pa":12e9,"poisson_ratio":0.25}}}]});
        let doc = s.doc.clone();
        let before = (s.revision, s.evaluations, s.undo.len(), s.redo.len(), s.camera, s.selection.clone());
        let response = s.execute("material.properties.directional", &json!({"card":card,"direction":[1,2,3]})).unwrap();
        assert_eq!(response["young_modulus_pa"], 12e9);
        assert_eq!(response["validation"]["source_review_required"], json!(["test"]));
        assert_eq!(response["validation"]["missing_condition_fields"], json!({"elasticity":["temperature_k","state"]}));
        for direction in [json!([0, 0, 0]), json!([1, 2]), json!([1, 2, 3, 4]), json!([true, 0, 0]), json!(["1", 0, 0]), Value::Null] {
            assert!(s.execute("material.properties.directional", &json!({"card":card,"direction":direction})).is_err());
        }
        assert!(s.execute("material.properties.directional", &json!({"card":card,"direction":[1,0,0],"assign":true})).is_err());
        assert!(Arc::ptr_eq(&doc, &s.doc));
        assert_eq!(before, (s.revision, s.evaluations, s.undo.len(), s.redo.len(), s.camera, s.selection.clone()));
    }

    #[test]
    fn command_reports_analytic_moduli_and_refuses_unstable_data() {
        let mut s = Session::default();
        let mut card = json!({"schema_version":1,"designation":"Synthetic elastic fixture","family":"other",
            "sources":[{"id":"fixture","title":"Original analytic fixture","kind":"illustrative"}],
            "properties":[{"source_id":"fixture","data":{"kind":"elasticity","model":{
                "symmetry":"isotropic","young_modulus_pa":12e9,"poisson_ratio":0.25}}}]});
        let result = s.execute("material.properties.validate", &json!({"card":card})).unwrap();
        assert_eq!(result["validation"]["isotropic_moduli"], json!({"shear_pa":4.8e9,"bulk_pa":8e9}));
        assert_eq!(result["validation"]["source_review_required"], json!(["fixture"]));
        assert_eq!(result["validation"]["rendering_properties_available"], false);
        assert_eq!(result["validation"]["missing_condition_fields"], json!({"elasticity":["temperature_k","state"]}));
        card["properties"][0]["data"]["model"]["poisson_ratio"] = json!(0.6);
        assert!(s.execute("material.properties.validate", &json!({"card":card})).is_err());
    }
    #[test]
    fn validation_preserves_document_history_selection_and_evaluation() {
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length":10,"width":20,"height":5})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        let doc = s.doc.clone();
        let undo = s.undo.clone();
        let redo = s.redo.clone();
        let before = (s.is_dirty(), s.revision, s.evaluations, s.model.is_stale(), s.camera, s.selection.clone());
        let card = json!({"schema_version":1,"designation":"Synthetic empty card","family":"other","sources":[],"properties":[]});
        let result = s.execute("material.properties.validate", &json!({"card":card})).unwrap();
        assert_eq!(result["validation"]["analysis_solver_available"], false);
        assert_eq!(result["validation"]["missing_condition_fields"], json!({}));
        for p in [json!({}), json!({"card":card,"assign":true}), json!({"card":{"schema_version":9}})] {
            assert!(s.execute("material.properties.validate", &p).is_err());
        }
        assert!(Arc::ptr_eq(&doc, &s.doc));
        assert_eq!(before, (s.is_dirty(), s.revision, s.evaluations, s.model.is_stale(), s.camera, s.selection.clone()));
        for (a, b) in [(&undo, &s.undo), (&redo, &s.redo)] {
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(b) {
                assert!(Arc::ptr_eq(&a.doc, &b.doc));
            }
        }
    }

    #[test]
    fn validation_does_not_settle_live_geometry() {
        let mut s = Session::default();
        for (id, p) in [
            ("sketch.create", json!({"plane":"XY"})),
            ("sketch.rectangle.two_point", json!({"p0":[5,5],"p1":[25,20]})),
            ("sketch.dimension", json!({"entities":["l1"],"value":20})),
            ("sketch.dimension", json!({"entities":["l2"],"value":15})),
            ("sketch.finish", json!({})),
            ("solid.extrude", json!({"distance":5})),
            ("sketch.edit", json!({"sketch":"Sketch1"})),
            ("sketch.drag", json!({"entity":"l1.start","from":[5,5],"to":[4,5],"live":true})),
        ] {
            s.execute(id, &p).unwrap();
        }
        assert!(s.model.is_stale());
        let doc = s.doc.clone();
        let before = (s.revision, s.evaluations, s.active_sketch, s.undo.len(), s.redo.len());
        let card = json!({"schema_version":1,"designation":"Synthetic","family":"other","sources":[],"properties":[]});
        s.execute("material.properties.validate", &json!({"card":card})).unwrap();
        assert!(s.execute("material.properties.validate", &json!({"card":false})).is_err());
        let elastic = json!({"schema_version":1,"designation":"Synthetic","family":"other",
            "sources":[{"id":"test","title":"Original fixture","kind":"illustrative"}],
            "properties":[{"source_id":"test","data":{"kind":"elasticity","model":{"symmetry":"isotropic","young_modulus_pa":12e9,"poisson_ratio":0.25}}}]});
        s.execute("material.properties.directional", &json!({"card":elastic,"direction":[1,2,3]})).unwrap();
        assert!(s.execute("material.properties.directional", &json!({"card":elastic,"direction":[0,0,0]})).is_err());
        assert!(Arc::ptr_eq(&doc, &s.doc));
        assert!(s.model.is_stale());
        assert_eq!(before, (s.revision, s.evaluations, s.active_sketch, s.undo.len(), s.redo.len()));
    }
}
