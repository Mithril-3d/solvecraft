use serde_json::json;
use std::sync::Arc;

use crate::{Session, product_scene};

fn fixture() -> Session {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"body_name":"Specimen","length":10,"width":20,"height":5})).unwrap();
    s
}

#[test]
fn format20_scalar_looks_upgrade_and_format21_setups_keep_their_meaning() {
    let mut s = fixture();
    s.execute("appearance.assign", &json!({"bodies":["Specimen"],"appearance":"Original legacy look","color":"#b47846","opacity":0.4})).unwrap();
    let before = serde_json::to_value(&s.doc.appearances).unwrap();
    let mut legacy = serde_json::to_value(&*s.doc).unwrap();
    legacy["format"] = json!("solidcraft/20");
    legacy.as_object_mut().unwrap().remove("render");
    let upgraded = crate::doc::Document::from_json(&legacy.to_string()).unwrap();
    assert_eq!(upgraded.format, crate::doc::format::format_string(crate::doc::format::FORMAT));
    assert_eq!(serde_json::to_value(&upgraded.appearances).unwrap(), before);
    assert!(upgraded.render.setups.is_empty());
    s.execute("render.settings.set", &json!({"name":"Studio","set":{"width":320,"height":240}})).unwrap();
    let mut saved: serde_json::Value = serde_json::from_str(&s.doc.to_json()).unwrap();
    saved["format"] = json!("solidcraft/21");
    let d = crate::doc::Document::from_json(&saved.to_string()).unwrap();
    assert_eq!(d.format, crate::doc::format::format_string(crate::doc::format::FORMAT));
    assert_eq!(serde_json::to_value(&d.appearances).unwrap(), before);
    assert!(d.appearances.bodies["Specimen"].pbr.is_none());
    assert_eq!(d.render.setups["Studio"].width, 320);
    for version in [crate::doc::format::FORMAT + 1, crate::doc::format::FORMAT + 2] {
        saved["format"] = json!(format!("solidcraft/{version}"));
        assert!(crate::doc::Document::from_json(&saved.to_string()).is_err());
    }
}

#[test]
fn opaque_response_preserves_geometry_mass_undo_and_cold_reopening() {
    let mut s = fixture();
    let before = product_scene::extract(&s).unwrap();
    let mass = s.execute("component.properties", &json!({})).unwrap();
    s.execute(
        "appearance.assign",
        &json!({"bodies":["Specimen"],"color":"#b47846","appearance":"Original opaque test",
        "pbr":{"metallic":1.0,"roughness":0.3,"dielectric_ior":1.5}}),
    )
    .unwrap();
    let after = product_scene::extract(&s).unwrap();
    assert!(Arc::ptr_eq(&before.meshes[0].mesh, &after.meshes[0].mesh));
    assert_eq!(s.execute("component.properties", &json!({})).unwrap(), mass);
    assert!(after.meshes[0].surface.is_some());
    assert_eq!(after.required_features(), vec![crate::render::RenderFeature::Pbr]);
    let saved = s.doc.to_json();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&saved).unwrap()["format"], crate::doc::format::format_string(crate::doc::format::FORMAT));
    let look = &s.doc.appearances.bodies["Specimen"];
    assert_eq!(crate::doc::config::appearance_cell(&serde_json::to_string(look).unwrap()).unwrap().as_ref(), Some(look));
    let list = s.execute("appearance.list", &json!({})).unwrap();
    assert_eq!(list["in_design"][0]["pbr"]["metallic"], 1.0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(product_scene::extract(&s).unwrap().meshes[0].surface.is_none());
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(product_scene::extract(&s).unwrap().meshes[0].surface.is_some());
    let mut reopened = Session::new(crate::doc::Document::from_json(&saved).unwrap());
    assert_eq!(product_scene::extract(&reopened).unwrap().meshes[0].surface, after.meshes[0].surface);
    assert_eq!(reopened.execute("component.properties", &json!({})).unwrap(), mass);
}

#[test]
fn invalid_optical_requests_refuse_atomically_and_scalar_export_cannot_lose_them() {
    let mut s = fixture();
    let doc = s.doc.clone();
    let history = (s.undo.len(), s.redo.len(), s.revision);
    for params in [
        json!({"bodies":["Specimen"],"pbr":{"metallic":0,"roughness":0.5,"dielectric_ior":1.5}}),
        json!({"bodies":["Specimen"],"color":"#ffffff","pbr":{"metallic":0,"roughness":0,"dielectric_ior":1.5}}),
        json!({"bodies":["Specimen"],"color":"#ffffff","opacity":0.5,"pbr":{"metallic":0,"roughness":0.5,"dielectric_ior":1.5}}),
        json!({"bodies":["Specimen"],"appearance":"Named","color":"#ffffff","opacity":"invalid","pbr":{"metallic":0,"roughness":0.5,"dielectric_ior":1.5}}),
        json!({"bodies":["Specimen"],"appearance":"Named","color":"#ffffff","opacity":2,"pbr":{"metallic":0,"roughness":0.5,"dielectric_ior":1.5}}),
        json!({"bodies":["Specimen"],"color":"#ffffff","pbr":{"metallic":0,"roughness":0.5,"dielectric_ior":1.5,"transmission":1}}),
    ] {
        assert!(s.execute("appearance.assign", &params).is_err());
        assert!(Arc::ptr_eq(&doc, &s.doc));
        assert_eq!((s.undo.len(), s.redo.len(), s.revision), history);
    }
    s.execute(
        "appearance.assign",
        &json!({"bodies":["Specimen"],"color":"#ffffff",
        "pbr":{"metallic":0,"roughness":0.5,"dielectric_ior":1.5}}),
    )
    .unwrap();
    assert!(solidcraft_io::catalog::Catalog::from_document(&s.doc, "user.fixture", "Original", Default::default()).is_err());
    let mut invalid: serde_json::Value = serde_json::from_str(&s.doc.to_json()).unwrap();
    invalid["appearances"]["bodies"]["Specimen"]["opacity"] = json!(0.5);
    assert!(crate::doc::Document::from_json(&invalid.to_string()).is_err());
    invalid["appearances"]["bodies"]["Specimen"]["opacity"] = json!(1.0);
    invalid["appearances"]["bodies"]["Specimen"]["pbr"]["roughness"] = json!(0.0);
    assert!(crate::doc::Document::from_json(&invalid.to_string()).is_err());
}

#[test]
fn physical_default_look_and_face_override_keep_independent_optical_meaning() {
    let mut s = fixture();
    s.execute(
        "material.create",
        &json!({"name":"Original density fixture","density_g_cm3":0.7,"color":"#b47846",
        "pbr":{"metallic":0,"roughness":0.4,"dielectric_ior":1.5}}),
    )
    .unwrap();
    s.execute("material.assign", &json!({"bodies":["Specimen"],"material":"Original density fixture"})).unwrap();
    let scene = product_scene::extract(&s).unwrap();
    assert!(scene.meshes[0].surface.is_some());
    s.execute("appearance.assign", &json!({"faces":[[5,10,5]],"body":"Specimen","color":"#0080a0"})).unwrap();
    let scene = product_scene::extract(&s).unwrap();
    assert!(scene.meshes[0].surface.is_some());
    assert!(scene.meshes[0].faces.iter().any(|f| f.color == [0, 128, 160] && f.surface.is_none()));
    assert_eq!(s.doc.user_materials[0].density, 0.7);
    assert!(solidcraft_io::catalog::Catalog::from_document(&s.doc, "user.fixture", "Original", Default::default()).is_err());
}

#[test]
fn independent_component_copy_keeps_its_opaque_response_after_source_edits_and_reopen() {
    let mut s = Session::default();
    let part = s.execute("component.create", &json!({"name":"Part"})).unwrap()["component"].as_u64().unwrap();
    s.execute("solid.box", &json!({"body_name":"Specimen","length":10,"width":20,"height":5})).unwrap();
    s.execute(
        "appearance.assign",
        &json!({"components":["Part"],"color":"#b47846",
        "pbr":{"metallic":1,"roughness":0.3,"dielectric_ior":1.5}}),
    )
    .unwrap();
    s.execute("component.activate", &json!({"component":0})).unwrap();
    let copied = s.execute("component.paste_new", &json!({"component":part,"translate":[30,0,0]})).unwrap()["component"].as_u64().unwrap();
    let original = s.doc.appearances.components[&part].clone();
    assert_eq!(s.doc.appearances.components[&copied], original);
    s.execute(
        "appearance.assign",
        &json!({"components":["Part"],"color":"#b47846",
        "pbr":{"metallic":1,"roughness":0.8,"dielectric_ior":1.5}}),
    )
    .unwrap();
    assert_ne!(s.doc.appearances.components[&part], original);
    assert_eq!(s.doc.appearances.components[&copied], original);
    let reopened = crate::doc::Document::from_json(&s.doc.to_json()).unwrap();
    assert_eq!(reopened.appearances.components[&copied], original);
}
