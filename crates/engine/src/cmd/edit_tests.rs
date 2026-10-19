//! Edit Feature must replace the selections as well as the numerical definition.

use serde_json::{Value, json};

use crate::{Sel, Session};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

#[test]
fn g2_fillet_survives_edit_undo_and_reopen() {
    use solidcraft_doc::{Document, FeatureKind, FilletStyle};
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 20, "width": 20, "height": 20, "name": "Base"}));
    run(&mut s, "solid.fillet", json!({"edges": [[20, 0, 10]], "radius": 2, "type": "g2", "name": "Smooth"}));
    assert!(matches!(&s.doc.find_feature("Smooth").unwrap().kind, FeatureKind::Fillet { style: FilletStyle::Curvature, .. }));
    let before = run(&mut s, "inspect.measure", json!({}))["total"]["volume_mm3"].as_f64().unwrap();
    run(&mut s, "timeline.edit", json!({"feature": "Smooth", "set": {"radius": "3"}}));
    let after = run(&mut s, "inspect.measure", json!({}))["total"]["volume_mm3"].as_f64().unwrap();
    assert!(after < before);
    run(&mut s, "edit.undo", json!({}));
    let undo = run(&mut s, "inspect.measure", json!({}))["total"]["volume_mm3"].as_f64().unwrap();
    assert!((undo - before).abs() < 1e-6);
    run(&mut s, "edit.redo", json!({}));
    let mut reopened = Session::new(Document::from_json(&s.doc.to_json()).unwrap());
    assert!(matches!(&reopened.doc.find_feature("Smooth").unwrap().kind, FeatureKind::Fillet { style: FilletStyle::Curvature, .. }));
    let volume = run(&mut reopened, "inspect.measure", json!({}))["total"]["volume_mm3"].as_f64().unwrap();
    assert!((volume - after).abs() < 1e-6);
    assert!(s.execute("solid.fillet", &json!({"edges": [[0, 0, 10]], "radius": 2, "type": "curvature", "fit": "reduce"})).is_err());
}

#[test]
fn redefining_a_blend_replaces_persistent_edge_selection_and_survives_resize() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 20, "name": "Base"}));
    run(&mut s, "solid.fillet", json!({"edges": [[20, 0, 20]], "radius": 2, "name": "Round"}));
    let round_id = s.doc.find_feature("Round").unwrap().id;
    let body_name = s.model.state().bodies[0].name.clone();
    s.doc_mut().feature_mut(round_id).unwrap().participants = vec![body_name];
    s.refresh();
    let old = s.doc.find_feature("Round").unwrap().clone();
    let before = (*s.doc).clone();
    run(&mut s, "timeline.redefine", json!({"feature": "Round", "command": "solid.fillet", "params": {"edges": [[0, 15, 20]], "radius": 2}}));
    let edited = s.doc.find_feature("Round").unwrap().clone();
    assert_eq!(edited.id, old.id);
    assert_eq!(edited.param_names, old.param_names);
    assert_eq!(edited.participants, old.participants);
    assert_ne!(edited.edge_names, old.edge_names, "the new edge must be the reference used on recompute");
    assert_ne!(edited.edge_refs, old.edge_refs);
    assert_eq!(s.doc.next_id, before.next_id, "temporary definitions must not consume feature ids");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(*s.doc, before);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc.find_feature("Round").unwrap().edge_names, edited.edge_names);
    run(&mut s, "timeline.edit", json!({"feature": "Base", "set": {"length": "60", "height": "25"}}));
    let volume = run(&mut s, "inspect.measure", json!({}))["total"]["volume_mm3"].as_f64().unwrap();
    let expected = 60.0 * 30.0 * 25.0 - (4.0 - std::f64::consts::PI) * 30.0;
    assert!((volume - expected).abs() < expected * 1e-4, "{volume} != {expected}");
    assert!(s.model.result(old.id).unwrap().warning.is_none());
}

#[test]
fn redefining_preserves_body_names_participants_and_marker_without_temporary_features() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 20, "width": 10, "height": 5, "name": "Base", "body_name": "Housing"}));
    run(&mut s, "solid.box", json!({"length": 5, "width": 5, "height": 5, "corner": [30, 0, 0], "name": "Other"}));
    let id = s.doc.find_feature("Base").unwrap().id;
    run(&mut s, "timeline.rollback", json!({"position": 1}));
    let before = s.doc.feature(id).unwrap().clone();
    run(&mut s, "timeline.redefine", json!({"feature": id, "command": "solid.box", "params": {"length": 25, "width": 10, "height": 5}}));
    let after = s.doc.feature(id).unwrap();
    assert_eq!(s.doc.features.len(), 2);
    assert_eq!(s.doc.marker, Some(1));
    assert_eq!(after.body_names, before.body_names);
    assert_eq!(after.participants, before.participants);
    assert_eq!(after.param_names, before.param_names);
    assert_eq!(after.name, before.name);
    assert!(s.model.state().body("Housing").is_some());
}

#[test]
fn failed_or_non_feature_redefinitions_leave_session_and_history_intact() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 20, "width": 10, "height": 5, "name": "Base"}));
    run(&mut s, "sketch.create", json!({"plane": "XY"}));
    s.selection = vec![Sel::Feature { id: s.doc.find_feature("Base").unwrap().id }];
    let doc = s.doc.clone();
    let sketch = s.active_sketch;
    let selection = s.selection.clone();
    let undo = s.undo.len();
    for (command, params) in [
        ("solid.cylinder", json!({"radius": 5, "height": 10})),
        ("solid.box", json!({"length": "unknown_parameter", "width": 10, "height": 5})),
        ("file.export", json!({"path": "should-not-be-created.step"})),
        ("edit.undo", json!({})),
    ] {
        assert!(s.execute("timeline.redefine", &json!({"feature": "Base", "command": command, "params": params})).is_err());
        assert_eq!(*s.doc, *doc, "{command}");
        assert_eq!(s.active_sketch, sketch, "{command}");
        assert_eq!(s.selection, selection, "{command}");
        assert_eq!(s.undo.len(), undo, "{command}");
    }
}

#[test]
fn redefining_an_extrude_names_new_optional_inputs() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [10, 10]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"distance": 5, "name": "Pad"}));
    let distance = s.doc.find_feature("Pad").unwrap().param_names[0].clone();
    run(&mut s, "timeline.redefine", json!({"feature": "Pad", "command": "solid.extrude", "params": {"distance": 5, "start_offset": 2}}));
    let f = s.doc.find_feature("Pad").unwrap();
    assert_eq!(f.param_names.len(), f.kind.inputs().len());
    assert_eq!(f.param_names[0], distance);
    assert!(f.param_names.iter().all(|name| !name.is_empty()));
    assert_eq!(s.doc.features.len(), 2);
    let offset = s.doc.find_feature("Pad").unwrap().param_names.last().unwrap().clone();
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature": "Pad", "command": "solid.extrude", "params": {"distance": 5, "distance2": 2, "start_offset": 2}}),
    );
    let f = s.doc.find_feature("Pad").unwrap();
    let inputs = f.kind.inputs();
    let offset_index = inputs.iter().position(|(label, _, _)| *label == "Start offset").unwrap();
    assert_eq!(f.param_names[offset_index], offset, "adding Distance 2 must not steal the offset dimension's parameter name");
    let distance2_index = inputs.iter().position(|(label, _, _)| *label == "Distance 2").unwrap();
    assert_ne!(f.param_names[distance2_index], offset);
}

#[test]
fn redefining_a_shell_replaces_persistent_face_selection() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 20, "name": "Base"}));
    run(&mut s, "solid.shell", json!({"faces": [[20, 15, 20]], "thickness": 2, "name": "Hollow"}));
    let before = s.doc.find_feature("Hollow").unwrap().face_names.clone();
    assert!(!before.is_empty());
    run(&mut s, "timeline.redefine", json!({"feature": "Hollow", "command": "solid.shell", "params": {"faces": [[20, 15, 0]], "thickness": 2}}));
    let after = s.doc.find_feature("Hollow").unwrap().face_names.clone();
    assert!(!after.is_empty());
    assert_ne!(before, after);
    run(&mut s, "timeline.edit", json!({"feature": "Base", "set": {"height": "30"}}));
    let id = s.doc.find_feature("Hollow").unwrap().id;
    assert!(s.model.result(id).unwrap().error.is_none());
    assert!(s.model.result(id).unwrap().warning.is_none());
    assert_eq!(s.doc.find_feature("Hollow").unwrap().face_names, after);
}

#[test]
fn redefine_rejects_sheet_exports_before_they_touch_any_files() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [30, 20]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "sheet.flange", json!({"name": "Plate"}));
    let doc = s.doc.clone();
    let base = std::env::temp_dir().join(format!("solidcraft-redefine-export-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    for command in ["sheet.export_dxf", "sheet.export_bend_table"] {
        for existing in [false, true] {
            let path = base.join(format!("{}-{existing}.out", command.replace('.', "-")));
            if existing {
                std::fs::write(&path, b"original file").unwrap();
            } else if path.exists() {
                std::fs::remove_file(&path).unwrap();
            }
            let error = s.execute("timeline.redefine", &json!({"feature": "Plate", "command": command, "params": {"path": path}})).unwrap_err();
            assert!(error.to_string().contains("does not make a feature"), "{error}");
            assert_eq!(*s.doc, *doc);
            if existing {
                assert_eq!(std::fs::read(&path).unwrap(), b"original file");
                std::fs::remove_file(&path).unwrap();
            } else {
                assert!(!path.exists(), "{command} must not create a file before refusing the edit");
            }
        }
    }
    for command in ["sheet.manage_rules", "sheet.rules", "sheet.flat_pattern", "parameters.list", "plastic.manage_rules"] {
        let error = s.execute("timeline.redefine", &json!({"feature": "Plate", "command": command, "params": {"name": "unexpected"}})).unwrap_err();
        assert!(error.to_string().contains("does not make a feature"), "{error}");
        assert_eq!(*s.doc, *doc);
    }
    std::fs::remove_dir(&base).unwrap();
}

#[test]
fn redefine_keeps_allocator_above_component_and_occurrence_ids() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 20, "width": 20, "height": 10, "body_name": "Base"}));
    run(&mut s, "solid.box", json!({"length": 20, "width": 20, "height": 10, "corner": [10, 0, 0], "body_name": "Tool"}));
    run(&mut s, "solid.combine", json!({"target": "Base", "tools": ["Tool"], "name": "Join"}));
    let original = s.doc.clone();
    let original_feature = s.doc.find_feature("Join").unwrap().id;
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature": "Join", "command": "solid.combine", "params": {"target": "Base", "tools": ["Tool"], "new_component": true, "component_name": "Merged"}}),
    );
    let destination = s.doc.find_component("Merged").unwrap();
    assert_eq!(s.model.state().body("Base").unwrap().component, destination);
    assert_eq!(s.doc.find_feature("Join").unwrap().id, original_feature);
    let combine_index = s.doc.feature_index(original_feature).unwrap();
    assert!(
        matches!(&s.doc.features[combine_index + 1].kind, solidcraft_doc::FeatureKind::BodyTransfer { bodies, to, .. } if bodies == &["Base"] && *to == destination)
    );
    assert!((solidcraft_kernel::measure(&s.world_state().body("Base").unwrap().body).unwrap().volume - 6000.0).abs() < 1e-6);
    assert_eq!(s.world_state().bodies.len(), 1);
    run(&mut s, "timeline.rollback", json!({"position":combine_index + 1}));
    assert_eq!(s.world_state().body("Base").unwrap().component, 0, "the result transfers only AFTER Combine");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(*s.doc, *original);
    run(&mut s, "edit.redo", json!({}));
    let reopened = Session::new(solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap());
    assert_eq!(reopened.world_state().body("Base").unwrap().component, destination);
    assert!((solidcraft_kernel::measure(&reopened.world_state().body("Base").unwrap().body).unwrap().volume - 6000.0).abs() < 1e-6);
    let max_id =
        s.doc.components.iter().map(|c| c.id).chain(s.doc.occurrences.iter().map(|o| o.id)).chain(s.doc.features.iter().map(|f| f.id)).max().unwrap();
    assert!(s.doc.next_id > max_id);
    run(&mut s, "component.create", json!({"name": "Next", "parent": 0}));
    run(&mut s, "solid.box", json!({"length": 2, "width": 2, "height": 2}));
    let ids: Vec<u64> =
        s.doc.components.iter().map(|c| c.id).chain(s.doc.occurrences.iter().map(|o| o.id)).chain(s.doc.features.iter().map(|f| f.id)).collect();
    let unique: std::collections::BTreeSet<u64> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "feature, component and occurrence ids must remain unique: {ids:?}");
}

#[test]
fn redefine_cannot_publish_a_second_transfer_that_breaks_existing_ownership() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length":2,"width":2,"height":2,"body_name":"Base"}));
    run(&mut s, "solid.box", json!({"length":2,"width":2,"height":2,"corner":[1,0,0],"body_name":"Tool"}));
    run(&mut s, "solid.combine", json!({"target":"Base","tools":["Tool"],"name":"Join","new_component":true,"component_name":"Original"}));
    let full = s.doc.to_json();
    let join_index = s.doc.features.iter().position(|feature| feature.name == "Join").unwrap();
    for marker in [None, Some(join_index), Some(join_index + 1)] {
        let mut s = Session::new(solidcraft_doc::Document::from_json(&full).unwrap());
        s.doc_mut().marker = marker;
        s.refresh();
        let before = s.doc.to_json();
        let pending = s.pending_moves.clone();
        let error = s.execute("timeline.redefine", &json!({"feature":"Join","command":"solid.combine","params":{"target":"Base","tools":["Tool"],"new_component":true,"component_name":"Duplicate"}})).unwrap_err();
        assert!(error.to_string().contains("transfer source component"), "marker {marker:?}: {error}");
        assert_eq!(s.doc.to_json(), before);
        assert_eq!(s.pending_moves, pending);
    }
    let reopened = Session::new(solidcraft_doc::Document::from_json(&full).unwrap());
    assert_eq!(reopened.world_state().body("Base").unwrap().component, reopened.doc.find_component("Original").unwrap());
    assert!(reopened.doc.features.iter().all(|feature| reopened.model.result(feature.id).is_some_and(|result| result.error.is_none())));
}

#[test]
fn redefine_maps_placement_inputs_in_the_feature_owner_frame() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 4, "width": 4, "height": 4, "name": "RootBox", "body_name": "RootBody"}));
    let owner = run(&mut s, "component.create", json!({"name": "Owner"}));
    run(&mut s, "solid.box", json!({"length": 4, "width": 4, "height": 4, "corner": [10, 0, 0], "name": "OwnedBox", "body_name": "OwnedBody"}));
    run(&mut s, "solid.fillet", json!({"edges": [[12, 0, 4]], "radius": 0.5, "body": "OwnedBody", "name": "OwnerRound"}));
    run(&mut s, "occurrence.move", json!({"occurrence": owner["occurrence"], "translate": [100, 0, 0]}));
    let other = run(&mut s, "component.create", json!({"name": "Other", "parent": 0}));
    run(&mut s, "occurrence.move", json!({"occurrence": other["occurrence"], "translate": [200, 0, 0]}));
    let active = s.active_component;
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature": "RootBox", "command": "solid.box", "params": {"length": 4, "width": 4, "height": 4, "corner": [2, 0, 0]}}),
    );
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature": "OwnedBox", "command": "solid.box", "params": {"length": 4, "width": 4, "height": 4, "corner": [110, 0, 0]}}),
    );
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature": "OwnerRound", "command": "solid.fillet", "params": {"edges": [[112, 0, 4]], "radius": 0.75, "body": "OwnedBody"}}),
    );
    assert_eq!(s.active_component, active, "editing must not activate the owner in the real session");
    assert_eq!(s.doc.find_feature("OwnedBox").unwrap().component, owner["component"].as_u64().unwrap());
    let root = run(&mut s, "inspect.measure", json!({"bodies": ["RootBody"]}));
    let owned = run(&mut s, "inspect.measure", json!({"bodies": ["OwnedBody"]}));
    assert!((root["bodies"][0]["bbox"]["min"][0].as_f64().unwrap() - 2.0).abs() < 1e-8);
    assert!((owned["bodies"][0]["bbox"]["min"][0].as_f64().unwrap() - 110.0).abs() < 1e-8);
}

#[test]
fn redefining_fillet_style_keeps_primary_parameter_and_downstream_references() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 20, "height": 30}));
    run(&mut s, "solid.fillet", json!({"edges": [[10, 10, 30]], "radius": 2, "name": "Round"}));
    let parameter = s.doc.find_feature("Round").unwrap().param_names[0].clone();
    run(&mut s, "solid.box", json!({"length": parameter, "width": 2, "height": 2, "corner": [50, 0, 0], "name": "Dependent"}));
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature": "Round", "command": "solid.fillet", "params": {"edges": [[10, 10, 30]], "radius": 3, "type": "chord"}}),
    );
    assert_eq!(s.doc.find_feature("Round").unwrap().param_names[0], parameter);
    let dependent = s.doc.find_feature("Dependent").unwrap().id;
    assert!(s.model.result(dependent).unwrap().error.is_none());
    let value = run(&mut s, "expr.evaluate", json!({"expression": parameter}));
    assert_eq!(value["value"], 3.0);
}

#[test]
fn inserting_variable_fillet_control_preserves_parameters_at_existing_positions() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 20, "height": 30}));
    run(
        &mut s,
        "solid.fillet",
        json!({"edges": [[10, 10, 30]], "radius": 2, "radius2": 2, "type": "variable", "start": [10, 0, 30], "mid": [[0.25, 3], [0.75, 4]], "name": "Round"}),
    );
    let parameters = s.doc.find_feature("Round").unwrap().param_names.clone();
    run(&mut s, "solid.box", json!({"length": parameters[2], "width": parameters[3], "height": 2, "corner": [50, 0, 0], "name": "Dependent"}));
    run(
        &mut s,
        "timeline.redefine",
        json!({"feature": "Round", "command": "solid.fillet", "params": {"edges": [[10, 10, 30]], "radius": 2, "radius2": 2, "type": "variable", "start": [10, 0, 30], "mid": [[0.1, 2.5], [0.25, 3], [0.75, 4]]}}),
    );
    let edited = s.doc.find_feature("Round").unwrap();
    assert_eq!(edited.param_names[0..2], parameters[0..2]);
    assert_eq!(edited.param_names[3..5], parameters[2..4]);
    assert!(!parameters.contains(&edited.param_names[2]), "the inserted control needs a new dimension");
    let dependent = s.doc.find_feature("Dependent").unwrap().id;
    assert!(s.model.result(dependent).unwrap().error.is_none());
    let length = run(&mut s, "expr.evaluate", json!({"expression": parameters[2]}));
    let width = run(&mut s, "expr.evaluate", json!({"expression": parameters[3]}));
    assert_eq!(length["value"], 3.0);
    assert_eq!(width["value"], 4.0);
}

#[test]
fn timeline_folders_are_command_backed_saved_undoable_and_follow_feature_ids() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "name": "Base"}));
    run(&mut s, "solid.fillet", json!({"edges": [[10, 5, 10]], "radius": 1, "name": "Round"}));
    let base = s.doc.find_feature("Base").unwrap().id;
    let round = s.doc.find_feature("Round").unwrap().id;

    let made = run(&mut s, "timeline.group", json!({"name": "Housing", "features": ["Base", round]}));
    let group = made["group"].as_u64().unwrap();
    assert_eq!(s.doc.timeline_groups[0].features, vec![base, round]);
    run(&mut s, "timeline.rename", json!({"feature": base, "name": "Foundation"}));
    assert_eq!(s.doc.timeline_groups[0].features, vec![base, round], "folders follow stable ids, not names");
    run(&mut s, "timeline.rename_group", json!({"group": group, "name": "Main housing"}));
    run(&mut s, "timeline.move_to_group", json!({"features": [round], "group": null}));
    assert_eq!(s.doc.timeline_groups[0].features, vec![base]);

    let reopened = solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap();
    assert_eq!(reopened.timeline_groups, s.doc.timeline_groups);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.timeline_groups[0].features, vec![base, round]);
    run(&mut s, "timeline.delete", json!({"features": [base]}));
    assert_eq!(s.doc.timeline_groups[0].features, vec![round], "deleted features leave folders cleanly");
    run(&mut s, "timeline.ungroup", json!({"group": group}));
    assert!(s.doc.timeline_groups.is_empty());
}

#[test]
fn timeline_reorders_selected_features_as_one_block_and_removes_folders_in_one_step() {
    let mut s = Session::default();
    for (name, body, x) in [("First", "FirstBody", 0), ("Second", "SecondBody", 20), ("Third", "ThirdBody", 40)] {
        run(
            &mut s,
            "solid.box",
            json!({"length": 10, "width": 10, "height": 10, "corner": [x, 0, 0], "operation": "new_body", "name": name, "body_name": body}),
        );
    }
    let names = |session: &Session| session.doc.features.iter().map(|feature| feature.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&s), vec!["First", "Second", "Third"]);

    run(&mut s, "timeline.reorder_many", json!({"features": ["First", "Third"], "position": 3}));
    assert_eq!(names(&s), vec!["Second", "First", "Third"], "selected features keep their relative order");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(names(&s), vec!["First", "Second", "Third"]);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(names(&s), vec!["Second", "First", "Third"]);
    run(&mut s, "edit.undo", json!({}));

    let first = run(&mut s, "timeline.group", json!({"name": "One", "features": ["First"]}))["group"].as_u64().unwrap();
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc.timeline_groups.is_empty(), "creating a folder is undoable");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc.timeline_groups.first().map(|group| group.id), Some(first));
    let second = run(&mut s, "timeline.group", json!({"name": "Two", "features": ["Second"]}))["group"].as_u64().unwrap();
    run(&mut s, "timeline.ungroup", json!({"groups": [first, second]}));
    assert!(s.doc.timeline_groups.is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.timeline_groups.len(), 2, "removing several folders is one undoable command");
    run(&mut s, "edit.redo", json!({}));
    assert!(s.doc.timeline_groups.is_empty(), "removing folders is redoable");
}

#[test]
fn timeline_folders_refuse_unknown_non_contiguous_and_split_contents() {
    let mut s = Session::default();
    for (name, body, x) in [("First", "FirstBody", 0), ("Second", "SecondBody", 20), ("Third", "ThirdBody", 40)] {
        run(
            &mut s,
            "solid.box",
            json!({"length": 10, "width": 10, "height": 10, "corner": [x, 0, 0], "operation": "new_body", "name": name, "body_name": body}),
        );
    }
    let before = s.doc.clone();
    let error = s.execute("timeline.group", &json!({"features": ["First", "Third"]})).unwrap_err().to_string();
    assert!(error.contains("contiguous"), "{error}");
    assert_eq!(s.doc, before);

    let group = run(&mut s, "timeline.group", json!({"name": "Pair", "features": ["First", "Second"]}))["group"].as_u64().unwrap();
    let before = s.doc.clone();
    let error = s.execute("timeline.reorder_many", &json!({"features": ["First"], "position": 3})).unwrap_err().to_string();
    assert!(error.contains("contiguous"), "{error}");
    assert_eq!(s.doc, before, "a refused reorder must not split the folder");

    for params in [json!({"features": [999_u64], "group": group}), json!({"groups": [999_u64], "group": group})] {
        let error = s.execute("timeline.move_to_group", &params).unwrap_err().to_string();
        assert!(error.contains("no feature") || error.contains("no timeline folder"), "{error}");
    }

    let mut damaged: Value = serde_json::from_str(&s.doc.to_json()).unwrap();
    let first = s.doc.find_feature("First").unwrap().id;
    let third = s.doc.find_feature("Third").unwrap().id;
    damaged["timeline_groups"] = json!([{"id": 1, "name": "Broken", "features": [first, third]}]);
    let error = solidcraft_doc::Document::from_json(&damaged.to_string()).unwrap_err().to_string();
    assert!(error.contains("not contiguous"), "{error}");
}

#[test]
fn timeline_move_many_is_atomic_and_one_undo_step() {
    let mut s = Session::default();
    for (name, body, x) in [("First", "FirstBody", 0), ("Second", "SecondBody", 20), ("Third", "ThirdBody", 40)] {
        run(
            &mut s,
            "solid.box",
            json!({"length": 10, "width": 10, "height": 10, "corner": [x, 0, 0], "operation": "new_body", "name": name, "body_name": body}),
        );
    }
    let group = run(&mut s, "timeline.group", json!({"name": "Pair", "features": ["First"]}))["group"].as_u64().unwrap();
    let before = (*s.doc).clone();
    run(&mut s, "timeline.move_many", json!({"features": ["Third"], "group": group, "position": 1}));
    assert_eq!(s.doc.features.iter().map(|feature| feature.name.as_str()).collect::<Vec<_>>(), vec!["First", "Third", "Second"]);
    assert_eq!(s.doc.timeline_groups[0].features.len(), 2);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(*s.doc, before, "one undo restores both order and folder membership");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.doc.timeline_groups[0].features.len(), 2);

    let child = run(&mut s, "timeline.group", json!({"name": "Child", "features": ["Second"]}))["group"].as_u64().unwrap();
    let before_parent = (*s.doc).clone();
    let parent = run(&mut s, "timeline.group", json!({"name": "Parent", "groups": [group, child]}))["group"].as_u64().unwrap();
    assert!(s.doc.timeline_groups.iter().filter(|folder| folder.parent == Some(parent)).count() == 2);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(*s.doc, before_parent, "creating a parent for selected folders is one undo step");
}

#[test]
fn failed_timeline_move_many_keeps_folder_membership_and_order() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "name": "Base"}));
    run(&mut s, "solid.fillet", json!({"edges": [[10, 5, 10]], "radius": 1, "name": "Round"}));
    let group = run(&mut s, "timeline.group", json!({"name": "Destination"}))["group"].as_u64().unwrap();
    let before = s.doc.clone();
    let undo = s.undo.len();
    let error = s.execute("timeline.move_many", &json!({"features": ["Base"], "group": group, "position": 2})).unwrap_err().to_string();
    assert!(error.contains("breaks"), "{error}");
    assert_eq!(s.doc, before);
    assert_eq!(s.undo.len(), undo, "a refused atomic move adds no undo step");
}

#[test]
fn timeline_folders_can_nest_and_reject_cycles() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "name": "Base"}));
    let root = run(&mut s, "timeline.group", json!({"name": "Root"}))["group"].as_u64().unwrap();
    let child = run(&mut s, "timeline.group", json!({"name": "Child", "parent": root}))["group"].as_u64().unwrap();
    assert_eq!(s.doc.timeline_groups.iter().find(|group| group.id == child).unwrap().parent, Some(root));
    run(&mut s, "timeline.move_to_group", json!({"groups": [child], "group": null}));
    assert_eq!(s.doc.timeline_groups.iter().find(|group| group.id == child).unwrap().parent, None);
    run(&mut s, "timeline.move_to_group", json!({"groups": [child], "group": root}));
    let error = s.execute("timeline.move_to_group", &json!({"groups": [root], "group": child})).unwrap_err().to_string();
    assert!(error.contains("itself") || error.contains("descendant"), "{error}");
    let before = s.doc.clone();
    let error = s.execute("timeline.move_many", &json!({"groups": [root], "group": child, "position": 0})).unwrap_err().to_string();
    assert!(error.contains("itself") || error.contains("descendant"), "{error}");
    assert_eq!(s.doc, before);
    let reopened = solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap();
    assert_eq!(reopened.timeline_groups, s.doc.timeline_groups);
    run(&mut s, "timeline.ungroup", json!({"group": root}));
    assert_eq!(s.doc.timeline_groups.iter().find(|group| group.id == child).unwrap().parent, None, "deleting a folder promotes its children");
}

/// A field the feature's definition doesn't have is refused with the fields it does have,
/// instead of reporting success and changing nothing (#1491).
#[test]
fn edit_refuses_unknown_fields() {
    let mut s = Session::default();
    run(&mut s, "construct.plane.offset", json!({"base": "XZ", "offset": 50, "name": "P"}));
    let e = s.execute("timeline.edit", &json!({"feature": "P", "set": {"offset": "-50"}})).unwrap_err().to_string();
    assert!(e.contains("unknown field `offset`") && e.contains("`plane.distance`"), "{e}");
    let e = s.execute("timeline.edit", &json!({"feature": "P", "set": {"plane": {"dist": "-50"}}})).unwrap_err().to_string();
    assert!(e.contains("unknown field `plane.dist`") && e.contains("`plane.distance`"), "{e}");
    let def = run(&mut s, "timeline.inspect", json!({"feature": "P"}))["definition"].clone();
    assert_eq!(def["plane"]["distance"], json!("50"), "nothing changed: {def}");
    // The field it does have edits as before.
    run(&mut s, "timeline.edit", json!({"feature": "P", "set": {"plane": {"distance": "-50"}}}));
    let def = run(&mut s, "timeline.inspect", json!({"feature": "P"}))["definition"].clone();
    assert_eq!(def["plane"]["distance"], json!("-50"), "{def}");

    // Fields the definition leaves out at their default are still fields: `combine: false`
    // and `bodies: []` aren't shown on a feature mirror, and set them as they are.
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "name": "Base"}));
    run(&mut s, "solid.mirror", json!({"features": ["Base"], "plane": "YZ", "name": "Twin"}));
    run(&mut s, "timeline.edit", json!({"feature": "Twin", "set": {"combine": false, "bodies": []}}));
    run(&mut s, "timeline.edit", json!({"feature": "Twin", "set": {"combine": true}}));
    // An unknown field next to known ones refuses the whole edit.
    let e = s.execute("timeline.edit", &json!({"feature": "Twin", "set": {"combine": false, "flip": true}})).unwrap_err().to_string();
    assert!(e.contains("unknown field `flip`") && e.contains("`combine`") && e.contains("`plane.name`"), "{e}");
    let def = run(&mut s, "timeline.inspect", json!({"feature": "Twin"}))["definition"].clone();
    assert_eq!(def["combine"], json!(true), "{def}");
}
