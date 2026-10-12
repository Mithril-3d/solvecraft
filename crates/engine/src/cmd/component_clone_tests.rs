#![allow(clippy::unwrap_used)]
use crate::Session;
use serde_json::{Value, json};
fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap()
}
fn component(s: &mut Session, name: &str) -> u64 {
    run(s, "component.create", json!({"name":name}))["component"].as_u64().unwrap()
}
fn volume(s: &mut Session, id: u64) -> f64 {
    run(s, "component.properties", json!({"component":id}))["total"]["volume_mm3"].as_f64().unwrap()
}
fn box2(s: &mut Session) -> u64 {
    run(s, "solid.box", json!({"length":2,"width":2,"height":2,"body_name":"Cube"}))["feature"].as_u64().unwrap()
}
fn copy(s: &mut Session, id: u64) -> Value {
    run(s, "component.paste_new", json!({"component":id,"translate":[20,0,0]}))
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn coincident_independent_primitives_keep_their_own_semantic_faces() {
    let mut s = Session::default();
    let first = box2(&mut s);
    let second = run(&mut s, "solid.box", json!({"length":2,"width":2,"height":2,"body_name":"Other"}))["feature"].as_u64().unwrap();
    let state = s.model.state();
    let a = state.body("Cube").unwrap();
    let b = state.body("Other").unwrap();
    let a_names = solvecraft_doc::naming::face_names(a);
    let b_names = solvecraft_doc::naming::face_names(b);
    assert!(a_names.contains(&format!("F{first}:+z")));
    assert!(b_names.contains(&format!("F{second}:+z")));
    assert!(!b_names.contains(&format!("F{first}:+z")));
}

#[test]
fn nested_definition_dag_and_histories_are_independent() {
    let mut s = Session::default();
    let parent = component(&mut s, "Assembly");
    let child = component(&mut s, "Child");
    let feature = box2(&mut s);
    run(&mut s, "occurrence.copy", json!({"component":child}));
    let copied = copy(&mut s, parent);
    let new = copied["component"].as_u64().unwrap();
    assert_eq!(copied["components"], 2);
    near(volume(&mut s, new), 16.0);
    let kids: Vec<_> = s.doc.occurrences.iter().filter(|o| o.parent == new).collect();
    assert_eq!(kids.len(), 2);
    assert_eq!(kids[0].component, kids[1].component);
    assert_ne!(kids[0].component, child);
    let parameter = s.doc.feature(feature).unwrap().param_names[0].clone();
    run(&mut s, "parameters.change", json!({"name":parameter,"expression":"3 mm"}));
    near(volume(&mut s, parent), 24.0);
    near(volume(&mut s, new), 16.0);
    let saved = s.doc.to_json();
    let reopened = solvecraft_doc::Document::from_json(&saved).unwrap();
    assert_eq!(reopened.occurrences.iter().filter(|o| o.parent == new).count(), 2);
}

#[test]
fn reassigned_body_is_an_independent_base_feature() {
    let mut s = Session::default();
    let feature = box2(&mut s);
    let part = component(&mut s, "Part");
    run(&mut s, "component.move_bodies", json!({"component":part,"bodies":["Cube"]}));
    let copied = copy(&mut s, part);
    let new = copied["component"].as_u64().unwrap();
    assert_eq!(copied["frozen_bodies"], json!(["Cube"]));
    near(volume(&mut s, new), 8.0);
    let parameter = s.doc.feature(feature).unwrap().param_names[0].clone();
    run(&mut s, "parameters.change", json!({"name":parameter,"expression":"3 mm"}));
    near(volume(&mut s, part), 12.0);
    near(volume(&mut s, new), 8.0);
}

#[test]
fn clone_preserves_physical_material_and_independent_styles() {
    let mut s = Session::default();
    let part = component(&mut s, "Part");
    box2(&mut s);
    run(&mut s, "material.assign", json!({"bodies":["Cube"],"material":"Brass"}));
    run(&mut s, "appearance.assign", json!({"components":[part.to_string()],"color":"#123456","opacity":0.4}));
    run(&mut s, "appearance.assign", json!({"bodies":["Cube"],"color":"#abcdef"}));
    run(&mut s, "appearance.assign", json!({"body":"Cube","faces":[[1,1,2]],"color":"#00ff00"}));
    let copied = copy(&mut s, part);
    let new = copied["component"].as_u64().unwrap();
    let props = run(&mut s, "component.properties", json!({"component":new}));
    near(props["total"]["mass_g"].as_f64().unwrap(), 0.068);
    let copied_body = s.model.state().bodies.iter().find(|b| b.component == new).unwrap().name.clone();
    assert_eq!(s.doc.appearances.bodies[&copied_body], s.doc.appearances.bodies["Cube"]);
    let state = s.model.state();
    let body = state.body(&copied_body).unwrap();
    assert!(s.doc.face_colors(body).iter().any(|(_, look)| look.color == [0, 255, 0]));
    run(&mut s, "appearance.assign", json!({"bodies":[copied_body],"color":"#ff0000"}));
    assert_eq!(s.doc.appearances.bodies["Cube"].color, [0xab, 0xcd, 0xef]);
    assert_eq!(s.doc.appearances.components[&new].opacity, 0.4);
}

#[test]
fn reassigned_body_owned_move_replays_once_and_original_stays_unchanged() {
    let mut s = Session::default();
    box2(&mut s);
    let part = component(&mut s, "Part");
    run(&mut s, "component.move_bodies", json!({"component":part,"bodies":["Cube"]}));
    run(&mut s, "solid.move", json!({"bodies":["Cube"],"translate":[10,0,0]}));
    let before = run(&mut s, "component.properties", json!({"component":part}));
    let copied = copy(&mut s, part);
    let new = copied["component"].as_u64().unwrap();
    let after = run(&mut s, "component.properties", json!({"component":new}));
    near(after["total"]["center_of_mass"][0].as_f64().unwrap(), 31.0);
    assert_eq!(run(&mut s, "component.properties", json!({"component":part})), before);
}

#[test]
fn copied_external_destructive_target_is_atomic() {
    let mut s = Session::default();
    box2(&mut s);
    let part = component(&mut s, "Part");
    run(&mut s, "solid.move", json!({"bodies":["Cube"],"translate":[10,0,0]}));
    let saved = s.doc.to_json();
    let selection = s.selection.clone();
    let active = s.active_component;
    let pending = s.pending_moves.clone();
    assert!(s.execute("component.paste_new", &json!({"component":part})).is_err());
    assert_eq!(s.doc.to_json(), saved);
    assert_eq!(s.selection, selection);
    assert_eq!(s.active_component, active);
    assert_eq!(s.pending_moves, pending);
}

#[test]
fn generated_feature_parameter_dependencies_are_independent() {
    let mut s = Session::default();
    let part = component(&mut s, "Part");
    let source = box2(&mut s);
    let source_names = s.doc.feature(source).unwrap().param_names.clone();
    run(&mut s, "parameters.change", json!({"name":source_names[1],"expression":format!("{} * 2",source_names[0])}));
    let result = copy(&mut s, part);
    let clone = result["component"].as_u64().unwrap();
    near(volume(&mut s, clone), 16.0);
    let clone_names = s
        .doc
        .features
        .iter()
        .find(|f| f.component == clone && matches!(f.kind, solvecraft_doc::FeatureKind::Box { .. }))
        .unwrap()
        .param_names
        .clone();
    assert_ne!(clone_names, source_names);
    run(&mut s, "parameters.change", json!({"name":source_names[0],"expression":"3 mm"}));
    near(volume(&mut s, part), 36.0);
    near(volume(&mut s, clone), 16.0);
    run(&mut s, "parameters.change", json!({"name":clone_names[0],"expression":"4 mm"}));
    near(volume(&mut s, clone), 64.0);
    near(volume(&mut s, part), 36.0);
}

#[test]
fn selected_nested_pending_placement_survives_copy_and_undo() {
    let mut s = Session::default();
    let parent = component(&mut s, "Parent");
    let parent_first = s.doc.occurrence_of(parent).unwrap().id;
    let child = component(&mut s, "Child");
    let leaf = s.doc.occurrence_of(child).unwrap().id;
    box2(&mut s);
    let second = run(&mut s, "occurrence.copy", json!({"component":parent,"translate":[50,0,0]}))["occurrence"].as_u64().unwrap();
    run(&mut s, "occurrence.move", json!({"occurrence":leaf,"capture":false,"translate":[5,0,0]}));
    run(&mut s, "occurrence.move", json!({"occurrence":second,"capture":false,"translate":[2,0,0]}));
    let pending = s.pending_moves.clone();
    let prior_context = (s.active_component, s.active_occurrence_path.clone(), s.active_sketch);
    let copied = run(&mut s, "component.paste_new", json!({"component":child,"occurrence":leaf,"path":[second,leaf],"translate":[10,0,0]}));
    let new_leaf = copied["occurrence"].as_u64().unwrap();
    assert_eq!((s.active_component, s.active_occurrence_path.clone(), s.active_sketch), prior_context);
    assert_eq!(s.selection, vec![crate::Sel::Occurrence { path: vec![second, new_leaf] }]);
    assert_eq!(s.pending_moves, pending);
    assert_eq!(s.occurrence_placement(&[parent_first, leaf]).unwrap().world[3][0], 5.0);
    let props = run(&mut s, "occurrence.properties", json!({"occurrence":new_leaf,"path":[second,new_leaf]}));
    near(props["total"]["center_of_mass"][0].as_f64().unwrap(), 68.0);
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.doc.occurrences.iter().any(|o| o.id == new_leaf));
    assert_eq!(s.pending_moves, pending);
}

#[test]
fn copied_sheet_base_targets_its_own_sketch_history() {
    let mut s = Session::default();
    let part = component(&mut s, "Sheet");
    let sketch = run(&mut s, "sketch.create", json!({"plane":"XY","name":"Base"}))["sketch"].as_u64().unwrap();
    run(&mut s, "sketch.rectangle.two_point", json!({"p0":[0,0],"p1":[10,10]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "sheet.flange", json!({"sketch":"Base"}));
    let source_volume = volume(&mut s, part);
    let copied = copy(&mut s, part);
    let new = copied["component"].as_u64().unwrap();
    near(volume(&mut s, new), source_volume);
    let clone_sketch = s.doc.features.iter().find(|f| f.component == new && matches!(f.kind, solvecraft_doc::FeatureKind::Sketch { .. })).unwrap().id;
    assert_ne!(clone_sketch, sketch);
    let base = s.doc.features.iter().find(|f| f.component == new && matches!(f.kind, solvecraft_doc::FeatureKind::SheetBase { .. })).unwrap();
    assert_eq!(base.kind.input_sketches(), vec![clone_sketch]);
    // Mutate only source geometry, then verify the copied base remains unchanged.
    if let solvecraft_doc::FeatureKind::Sketch { sketch, .. } = &mut s.doc_mut().feature_mut(sketch).unwrap().kind {
        for p in &mut sketch.points {
            p.pos.x *= 2.0;
        }
    }
    s.refresh();
    near(volume(&mut s, new), source_volume);
}

#[test]
fn renamed_pattern_outputs_and_later_body_operands_follow_clone() {
    let mut s = Session::default();
    let part = component(&mut s, "PatternPart");
    box2(&mut s);
    run(&mut s, "solid.pattern.rectangular", json!({"bodies":["Cube"],"dir1":[1,0,0],"count1":3,"spacing1":5}));
    let last = s.model.state().bodies.last().unwrap().name.clone();
    run(&mut s, "body.rename", json!({"body":last,"name":"Custom Output"}));
    run(&mut s, "solid.move", json!({"bodies":["Custom Output"],"translate":[0,5,0]}));
    let copied = copy(&mut s, part);
    let new = copied["component"].as_u64().unwrap();
    near(volume(&mut s, new), 24.0);
    let state = s.model.state();
    let target = state.bodies.iter().find(|b| b.name == "Custom Output (copy)").unwrap();
    near(solvecraft_kernel::measure(&target.body).unwrap().centroid.y, 6.0);
    assert_eq!(s.doc.features.iter().filter(|f| f.component == new && matches!(f.kind, solvecraft_doc::FeatureKind::Move { .. })).count(), 1);
}

#[test]
fn configuration_rows_follow_copied_parameters_material_and_strict_face_lineage() {
    let mut s = Session::default();
    let part = component(&mut s, "Configured");
    let source = box2(&mut s);
    let length = s.doc.feature(source).unwrap().param_names[0].clone();
    let target = solvecraft_doc::config::ConfigFaceTarget {
        body: "Cube".into(),
        origin_feature: source,
        name: format!("F{source}:+z"),
        point: solvecraft_geom::Vec3::new(1.0, 1.0, 2.0),
        certificate: None,
    };
    run(&mut s, "config.configure", json!({"params":[length],"materials":["Cube"],"component_appearances":[part],"face_appearances":[target]}));
    let face_key = s.doc.config_key(&solvecraft_doc::config::Column::FaceAppearance(target.clone()));
    run(
        &mut s,
        "config.row",
        json!({"name":"Large","values":{length.clone():"4 mm","material:Cube":"Brass",face_key:{"name":"Custom","color":[0,255,0],"opacity":1.0}}}),
    );
    let result = copy(&mut s, part);
    let new = result["component"].as_u64().unwrap();
    assert_eq!(s.doc.configs.columns.len(), 8);
    run(&mut s, "config.activate", json!({"row":"Large"}));
    near(volume(&mut s, part), 16.0);
    near(volume(&mut s, new), 16.0);
    let props = run(&mut s, "component.properties", json!({"component":new}));
    near(props["total"]["mass_g"].as_f64().unwrap(), 0.136);
    let state = s.model.state();
    let body = state.bodies.iter().find(|b| b.component == new).unwrap();
    assert!(s.doc.face_colors(body).iter().any(|(_, look)| look.color == [0, 255, 0]));
    assert!(s.doc.configuration_style_status(&state).is_ok());
    assert!(s.doc.configuration_matches());
}

#[test]
fn configured_extrude_cap_copy_keeps_independent_semantic_lineage() {
    let mut s = Session::default();
    let part = component(&mut s, "Extruded");
    run(&mut s, "sketch.create", json!({"plane":"XY"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0":[0,0],"p1":[2,2]}));
    run(&mut s, "sketch.finish", json!({}));
    let feature = run(&mut s, "solid.extrude", json!({"distance":2}))["feature"].as_u64().unwrap();
    let body_name = s.model.state().bodies.last().unwrap().name.clone();
    let target = solvecraft_doc::config::ConfigFaceTarget {
        body: body_name,
        origin_feature: feature,
        name: format!("F{feature}:top"),
        point: solvecraft_geom::Vec3::new(1.0, 1.0, 2.0),
        certificate: None,
    };
    run(&mut s, "config.configure", json!({"face_appearances":[target]}));
    let key = s.doc.config_key(&solvecraft_doc::config::Column::FaceAppearance(target));
    run(&mut s, "config.row", json!({"name":"Painted","values":{key:{"name":"Custom","color":[0,255,0],"opacity":1.0}}}));
    let copied = copy(&mut s, part)["component"].as_u64().unwrap();
    run(&mut s, "config.activate", json!({"row":"Painted"}));
    let state = s.model.state();
    let body = state.bodies.iter().find(|b| b.component == copied).unwrap();
    assert!(solvecraft_doc::naming::face_names(body).contains(&format!("F{}:top", body.feature)));
    assert!(s.doc.face_colors(body).iter().any(|(_, look)| look.color == [0, 255, 0]));
    assert!(s.doc.configuration_style_status(&state).is_ok());
    let saved = s.doc.to_json();
    let reopened = solvecraft_doc::Document::from_json(&saved).unwrap();
    let mut model = solvecraft_doc::Model::default();
    model.evaluate(&reopened);
    assert!(reopened.configuration_style_status(&model.state()).is_ok());
}

#[test]
fn selected_second_occurrence_emboss_copy_remaps_target_and_author_paths_and_reopens() {
    let mut s = Session::default();
    let part = component(&mut s, "SleevePart");
    let cylinder = run(&mut s, "solid.cylinder", json!({"radius":10,"height":20,"body_name":"Sleeve"}))["feature"].as_u64().unwrap();
    let second = run(&mut s, "occurrence.copy", json!({"component":part,"translate":[80,0,0]}))["occurrence"].as_u64().unwrap();
    run(&mut s, "component.activate", json!({"component":part,"occurrence":second,"path":[second]}));
    run(&mut s, "sketch.create", json!({"name":"Badge","plane":{"origin":[80,10,0],"x_dir":[-1,0,0],"y_dir":[0,0,1]}}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0":[-2,5],"p1":[2,10]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.emboss", json!({"sketch":"Badge","face":[80,10,7.5],"depth":1}));
    let original_volume = volume(&mut s, part);
    let copied = run(&mut s, "component.paste_new", json!({"component":part,"occurrence":second,"path":[second],"translate":[20,0,0]}));
    let new = copied["component"].as_u64().unwrap();
    let placement = copied["occurrence"].as_u64().unwrap();
    let emboss = s.doc.features.iter().find(|f| f.component == new && matches!(f.kind, solvecraft_doc::FeatureKind::Emboss { .. })).unwrap();
    let solvecraft_doc::FeatureKind::Emboss { face: Some(face), .. } = &emboss.kind else { panic!("missing selected face") };
    assert_eq!(face.body, "Sleeve (copy)");
    assert_eq!(face.path, vec![placement]);
    assert_eq!(face.author_path, vec![placement]);
    near(volume(&mut s, new), original_volume);
    let clone_volume = volume(&mut s, new);
    let height = s.doc.feature(cylinder).unwrap().param_names[1].clone();
    run(&mut s, "parameters.change", json!({"name":height,"expression":"30 mm"}));
    near(volume(&mut s, new), clone_volume);
    assert!(volume(&mut s, part) > original_volume);
    let mut reopened = Session { doc: solvecraft_doc::Document::from_json(&s.doc.to_json()).unwrap().into(), ..Session::default() };
    reopened.refresh();
    assert!(reopened.doc.features.iter().all(|f| reopened.model.result(f.id).is_some_and(|r| r.error.is_none())));
    near(volume(&mut reopened, new), clone_volume);
}

#[test]
fn reassigned_sample_plate_paste_new_preserves_complex_frozen_geometry() {
    let mut s = Session::default();
    s.run_script(&crate::sample::script()).unwrap();
    let body = s.model.state().bodies.first().unwrap().name.clone();
    let created = run(&mut s, "component.from_bodies", json!({"bodies":[body]}));
    let part = created["components"][0]["component"].as_u64().unwrap();
    run(&mut s, "component.activate", json!({"component":part}));
    let occurrence = s.doc.occurrence_of(part).unwrap().id;
    run(&mut s, "occurrence.ground", json!({"occurrence":occurrence,"grounded":true}));
    let original_volume = volume(&mut s, part);
    let source_mesh = s.model.state().body(&body).unwrap().mesh();
    let boss_probe = solvecraft_geom::Vec3::new(40.0, 35.0, 12.0);
    let source_boss = source_mesh.contains(boss_probe);
    let copied = run(&mut s, "component.paste_new", json!({"component":part,"translate":[10,10,0]}));
    assert_eq!(s.doc.components.len(), 2);
    let new = copied["component"].as_u64().unwrap();
    // Imported curved faces may tessellate differently; keep the same geometric
    // accuracy required by copy, and independently probe plate and hole material.
    assert!((volume(&mut s, new) - original_volume).abs() < 1e-5 * original_volume);
    near(volume(&mut s, part), original_volume);
    let state = s.model.state();
    let copied_body = state.bodies.iter().find(|body| body.component == new).unwrap();
    let mesh = copied_body.mesh();
    for (point, expected) in
        [([5.0, 25.0, 4.0], true), ([12.0, 25.0, 4.0], false), ([68.0, 25.0, 4.0], false), ([40.0, 25.0, 12.0], false), ([0.0, 0.0, 4.0], false)]
    {
        assert_eq!(mesh.contains(solvecraft_geom::Vec3::new(point[0], point[1], point[2])), expected, "{point:?}");
    }
    assert_eq!(mesh.contains(boss_probe), source_boss);
    assert_eq!(state.body(&body).unwrap().mesh().contains(boss_probe), source_boss);
    let mut reopened = Session { doc: solvecraft_doc::Document::from_json(&s.doc.to_json()).unwrap().into(), ..Session::default() };
    reopened.refresh();
    assert!((volume(&mut reopened, new) - original_volume).abs() < 1e-5 * original_volume);
}

#[test]
fn generated_copy_parameters_cannot_heal_an_unrelated_missing_reference() {
    let mut s = Session::default();
    let part = component(&mut s, "Valid");
    let valid = box2(&mut s);
    run(&mut s, "component.activate", json!({"component":0}));
    let kind = s.doc.feature(valid).unwrap().kind.clone();
    let broken = s.doc_mut().add_feature(kind, Some("Broken reference")).unwrap();
    let mut probe = (*s.doc).clone();
    let missing = probe.new_model_param("2 mm", "mm");
    let feature = s.doc_mut().feature_mut(broken).unwrap();
    feature.component = 0;
    if let solvecraft_doc::FeatureKind::Box { width, .. } = &mut feature.kind {
        *width = missing;
    }
    s.refresh();
    let original_error = s.model.result(broken).unwrap().error.clone();
    assert!(original_error.is_some());
    let saved = s.doc.to_json();
    let context = (s.active_component, s.active_occurrence_path.clone(), s.active_sketch);
    let selection = s.selection.clone();
    let pending = s.pending_moves.clone();
    let error = s.execute("component.paste_new", &json!({"component":part})).unwrap_err();
    assert!(error.to_string().contains("original feature's error state"), "{error}");
    assert_eq!(s.doc.to_json(), saved);
    assert_eq!(s.model.result(broken).unwrap().error, original_error);
    assert_eq!((s.active_component, s.active_occurrence_path.clone(), s.active_sketch), context);
    assert_eq!(s.selection, selection);
    assert_eq!(s.pending_moves, pending);
}

#[test]
fn external_transfer_entry_and_internal_rotated_transfer_clone_once() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length":4,"width":3,"height":2,"body_name":"Foreign"}));
    let parent = component(&mut s, "Assembly");
    let a = component(&mut s, "A");
    let ao = s.doc.occurrence_of(a).unwrap().id;
    run(&mut s, "occurrence.move", json!({"occurrence":ao,"translate":[10,0,0],"angle":90,"axis":[0,0,1],"capture":true}));
    run(&mut s, "component.move_bodies", json!({"component":a,"bodies":["Foreign"]}));
    run(&mut s, "component.activate", json!({"component":parent}));
    let b = component(&mut s, "B");
    let bo = s.doc.occurrence_of(b).unwrap().id;
    run(&mut s, "occurrence.move", json!({"occurrence":bo,"translate":[0,20,0],"angle":90,"axis":[1,0,0],"capture":true}));
    run(&mut s, "component.move_bodies", json!({"component":b,"bodies":["Foreign"]}));
    run(&mut s, "solid.move", json!({"bodies":["Foreign"],"translate":[0,0,5]}));
    let before = s.model.state().body("Foreign").unwrap().mesh().bounds();
    let copied = copy(&mut s, parent);
    assert_eq!(copied["frozen_bodies"], json!(["Foreign"]));
    let state = s.model.state();
    let body = state.body("Foreign (copy)").unwrap();
    let after = body.mesh().bounds();
    assert!(before.min.dist(after.min) < 1e-6);
    assert!(before.max.dist(after.max) < 1e-6);
    assert!(body.transferred);
    let mut reopened = Session { doc: solvecraft_doc::Document::from_json(&s.doc.to_json()).unwrap().into(), ..Session::default() };
    reopened.refresh();
    assert!(reopened.model.state().body("Foreign (copy)").unwrap().mesh().bounds().min.dist(before.min) < 1e-6);
}

#[test]
fn suppressed_external_entry_does_not_supply_clone_baseline() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length":4,"width":3,"height":2,"body_name":"Foreign"}));
    let part = component(&mut s, "Part");
    let occurrence = s.doc.occurrence_of(part).unwrap().id;
    run(&mut s, "occurrence.move", json!({"occurrence":occurrence,"translate":[25,0,0],"angle":90,"axis":[0,0,1],"capture":true}));
    let skipped = s
        .doc_mut()
        .add_feature(
            solvecraft_doc::FeatureKind::BodyTransfer { bodies: vec!["Foreign".into()], from: 0, to: part, transform: solvecraft_doc::IDENTITY },
            None,
        )
        .unwrap();
    s.doc_mut().feature_mut(skipped).unwrap().suppressed = true;
    s.refresh();
    run(&mut s, "component.move_bodies", json!({"component":part,"bodies":["Foreign"]}));
    let before = s.model.state().body("Foreign").unwrap().mesh().bounds();
    copy(&mut s, part);
    let after = s.model.state().body("Foreign (copy)").unwrap().mesh().bounds();
    assert!(before.min.dist(after.min) < 1e-6);
    assert!(before.max.dist(after.max) < 1e-6);
}

#[test]
fn transferred_frozen_face_paint_ignores_consumed_legacy_offset() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length":2,"width":2,"height":2,"body_name":"Foreign"}));
    run(&mut s, "appearance.assign", json!({"body":"Foreign","faces":[[1,1,2]],"color":"#00ff00"}));
    s.doc_mut().body_offsets.insert(
        "Foreign".into(),
        solvecraft_doc::rigid(solvecraft_geom::Vec3::new(7.0, 0.0, 0.0), solvecraft_geom::Vec3::ZERO, solvecraft_geom::Vec3::Z, 0.0),
    );
    s.refresh();
    let part = component(&mut s, "Part");
    run(&mut s, "component.move_bodies", json!({"component":part,"bodies":["Foreign"]}));
    let original = s.doc.appearances.faces.iter().find(|face| face.body == "Foreign").unwrap();
    let expected = solvecraft_doc::naming::point_on_face(&s.model.state(), original.name.as_ref().unwrap(), original.point).unwrap().0;
    copy(&mut s, part);
    let paint = s.doc.appearances.faces.iter().find(|face| face.body == "Foreign (copy)").unwrap();
    assert!(paint.point.dist(expected) < 1e-6, "{:?} versus {expected:?}", paint.point);
}

#[test]
fn boundary_material_proof_requires_resolvable_stock_and_void() {
    use solvecraft_geom::Vec3;
    let a = solvecraft_kernel::box_solid(Vec3::ZERO, Vec3::new(10., 10., 10.)).unwrap().tessellate(1e-3).unwrap();
    let b = solvecraft_kernel::box_solid(Vec3::new(1e-5, 0., 0.), Vec3::new(10.00001, 10., 10.)).unwrap().tessellate(1e-3).unwrap();
    let p = Vec3::new(5e-6, 5., 5.);
    assert_ne!(a.contains(p), b.contains(p));
    assert!((super::mesh_distance(&a, p) - 5e-6).abs() < 1e-12);
    assert!((super::mesh_distance(&b, p) - 5e-6).abs() < 1e-12);
    assert!(super::material_probe_matches(&a, &b, p, 1e-3, 1e-3));
    let shifted = solvecraft_kernel::box_solid(Vec3::new(1., 0., 0.), Vec3::new(11., 10., 10.)).unwrap().tessellate(1e-3).unwrap();
    assert!(!super::material_probe_matches(&a, &shifted, p, 1e-3, 1e-3));
    let thin = solvecraft_kernel::box_solid(Vec3::ZERO, Vec3::new(10., 10., 1e-4)).unwrap().tessellate(1e-6).unwrap();
    let thin_shifted = solvecraft_kernel::box_solid(Vec3::new(0., 0., 1e-4), Vec3::new(10., 10., 2e-4)).unwrap().tessellate(1e-6).unwrap();
    // Every resolvable replacement is void: thin ambiguous material is refused.
    assert!(!super::material_probe_matches(&thin, &thin_shifted, Vec3::new(5., 5., 5e-5), 1e-3, 1e-3));
    assert!(!super::material_probe_matches(&a, &b, p, f64::INFINITY, 1e-3));
}

#[test]
fn frozen_bore_boundary_copy_preserves_stock_void_and_reopen() {
    use solvecraft_geom::Vec3;
    let bore = |center: f64| {
        let mut s = Session::default();
        run(&mut s, "solid.box", json!({"length":30,"width":20,"height":10,"body_name":"Drilled"}));
        run(&mut s, "solid.cylinder", json!({"base":[center,10,0],"radius":4,"height":10,"operation":"cut"}));
        s
    };
    let mut s = bore(10.);
    let source = s.model.state().body("Drilled").unwrap().body.tessellate(1e-3).unwrap();
    // The exact R4 wall passes through x6; a chord may end either side of that
    // cardinal angle, so its distance is bounded by the requested sag tolerance.
    assert!(super::mesh_distance(&source, Vec3::new(6., 10., 5.)) <= 1e-3);
    let shifted = bore(11.).model.state().body("Drilled").unwrap().body.tessellate(1e-3).unwrap();
    let lost = solvecraft_kernel::box_solid(Vec3::ZERO, Vec3::new(30., 20., 10.)).unwrap().tessellate(1e-3).unwrap();
    assert!(!super::material_probe_matches(&source, &shifted, Vec3::new(6.5, 10., 5.), 1e-3, 1e-3));
    assert!(!super::material_probe_matches(&source, &lost, Vec3::new(10., 10., 5.), 1e-3, 1e-3));
    let created = run(&mut s, "component.from_bodies", json!({"bodies":["Drilled"]}));
    let part = created["components"][0]["component"].as_u64().unwrap();
    let copied = copy(&mut s, part)["component"].as_u64().unwrap();
    let check = |session: &Session| {
        let state = session.model.state();
        let body = state.bodies.iter().find(|body| session.doc.model_body_component(body) == copied).unwrap();
        let mesh = body.body.tessellate(1e-3).unwrap();
        for (point, stock) in
            [(Vec3::new(5.5, 10., 5.), true), (Vec3::new(6.5, 10., 5.), false), (Vec3::new(10., 10., 5.), false), (Vec3::new(18., 10., 5.), true)]
        {
            assert_eq!(mesh.contains(point), stock, "{point:?}");
        }
    };
    check(&s);
    check(&Session::new(solvecraft_doc::Document::from_json(&s.doc.to_json()).unwrap()));
}

#[test]
fn frozen_r6_bore_copy_uses_current_geometry_despite_mesh_measure_bias() {
    use solvecraft_geom::Vec3;
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length":20,"width":20,"height":10,"body_name":"Drilled"}));
    run(&mut s, "solid.cylinder", json!({"base":[10,10,0],"radius":6,"height":10,"operation":"cut"}));
    let source = s.model.state().body("Drilled").unwrap().body.clone();
    let original = solvecraft_kernel::step_export(&[&source], "source").unwrap();
    let created = run(&mut s, "component.from_bodies", json!({"bodies":["Drilled"]}));
    let part = created["components"][0]["component"].as_u64().unwrap();
    let before = s.doc.to_json();
    let copied = copy(&mut s, part)["component"].as_u64().unwrap();
    let check = |session: &Session| {
        let state = session.model.state();
        let body = state.bodies.iter().find(|b| session.doc.model_body_component(b) == copied).unwrap();
        assert_eq!(solvecraft_kernel::certified_prism_equivalent(&source, &body.body).unwrap(), Some(true));
        let mesh = body.body.tessellate(1e-3).unwrap();
        for (point, material) in
            [(Vec3::new(3., 10., 5.), true), (Vec3::new(5., 10., 5.), false), (Vec3::new(10., 10., 5.), false), (Vec3::new(18., 10., 5.), true)]
        {
            assert_eq!(mesh.contains(point), material, "{point:?}");
        }
        let original_body = state.body("Drilled").unwrap();
        assert_eq!(solvecraft_kernel::step_export(&[&original_body.body], "source").unwrap(), original);
    };
    check(&s);
    check(&Session::new(solvecraft_doc::Document::from_json(&s.doc.to_json()).unwrap()));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.to_json(), before);
    run(&mut s, "edit.redo", json!({}));
    check(&s);
}
