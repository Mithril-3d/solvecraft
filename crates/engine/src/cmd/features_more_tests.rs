use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn auto_emboss_params(mut p: Value, auto: bool) -> Value {
    if auto {
        if let Some(params) = p.get_mut("params").and_then(Value::as_object_mut) {
            params.remove("face");
        } else if let Some(object) = p.as_object_mut() {
            object.remove("face");
        }
    }
    p
}

fn measure(s: &mut Session) -> Value {
    run(s, "inspect.measure", json!({}))
}

fn volume(s: &mut Session) -> f64 {
    measure(s)["total"]["volume_mm3"].as_f64().unwrap_or(f64::NAN)
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1e-12)
}

/// An L bracket: a 40 x 20 x 5 base and a 5 x 20 x 40 wall at x = 0..5.
fn bracket() -> Session {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 20, "height": 5, "corner": [0, 0, 0], "body_name": "L"}));
    run(&mut s, "solid.box", json!({"length": 5, "width": 20, "height": 40, "corner": [0, 0, 0], "operation": "join"}));
    assert!(rel(volume(&mut s), 4000.0 + 3500.0) < 1e-9);
    // Mid-plane of the bracket (y = 10): sketch x = X, sketch y = Z.
    run(&mut s, "sketch.create", json!({"plane": {"origin": [0, 10, 0], "x_dir": [1, 0, 0], "y_dir": [0, 0, 1]}, "name": "RibSketch"}));
    run(&mut s, "sketch.line", json!({"points": [[5, 25], [25, 5]], "ids": ["l1"]}));
    run(&mut s, "sketch.finish", json!({}));
    s
}

#[test]
fn rib_to_the_next_face_fills_the_corner() {
    let mut s = bracket();
    run(&mut s, "solid.rib", json!({"sketch": "RibSketch", "curves": ["l1"], "thickness": 4}));
    // The triangle (5,5)-(5,25)-(25,5) of area 200, 4 thick.
    assert!(rel(volume(&mut s), 7500.0 + 800.0) < 1e-6, "{}", volume(&mut s));
    assert_eq!(measure(&mut s)["body_count"], 1);
    // The thickness is a parameter.
    let t = s.doc.features.last().and_then(|f| f.param_names.first().cloned()).unwrap_or_default();
    run(&mut s, "parameters.change", json!({"name": t, "expression": "2"}));
    assert!(rel(volume(&mut s), 7500.0 + 400.0) < 1e-6);
}

#[test]
fn rib_with_a_depth_and_web() {
    let mut s = bracket();
    run(&mut s, "solid.rib", json!({"sketch": "RibSketch", "curves": ["l1"], "thickness": 4, "depth": 2}));
    let v = volume(&mut s);
    assert!(v > 7500.0 + 100.0 && v < 7500.0 + 800.0, "{v}");
    let mut s = bracket();
    run(&mut s, "solid.web", json!({"sketch": "RibSketch", "curves": ["l1"], "thickness": 4}));
    assert!(rel(volume(&mut s), 8300.0) < 1e-6);
    // Curves that face no body.
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Far"}));
    run(&mut s, "sketch.line", json!({"points": [[50, 50], [60, 60]], "ids": ["l1"]}));
    run(&mut s, "sketch.finish", json!({}));
    let e = s.execute("solid.rib", &json!({"sketch": "Far", "curves": ["l1"], "thickness": 1}));
    assert!(e.is_err_and(|e| e.to_string().contains("do not face a body")));
}

#[test]
fn emboss_and_deboss_on_a_face() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 30, "height": 10}));
    run(&mut s, "sketch.create", json!({"plane": {"face": [20, 15, 10]}, "name": "Logo"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [5, 5], "p1": [15, 15]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.emboss", json!({"sketch": "Logo", "depth": 2}));
    assert!(rel(volume(&mut s), 12000.0 + 200.0) < 1e-6, "{}", volume(&mut s));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "solid.emboss", json!({"sketch": "Logo", "depth": 3, "mode": "deboss"}));
    assert!(rel(volume(&mut s), 12000.0 - 300.0) < 1e-6);
    assert!(s.execute("solid.emboss", &json!({"sketch": "Logo", "depth": 1, "mode": "sideways"})).is_err());
}

fn cylinder_badge() -> Session {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name":"badge_depth","expression":"1 mm"}));
    run(&mut s, "solid.cylinder", json!({"radius":"10 mm","height":"20 mm","body_name":"Sleeve"}));
    run(&mut s, "sketch.create", json!({"plane":{"origin":[0,10,0],"x_dir":[-1,0,0],"y_dir":[0,0,1]},"name":"Badge"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0":[-2,5],"p1":[2,10]}));
    run(&mut s, "sketch.finish", json!({}));
    s
}

fn exact_cylinder_volume(s: &Session) -> f64 {
    s.model.state().body("Sleeve").unwrap().body.tessellate_with(1e-4, true).unwrap().measure().volume
}

#[test]
fn cylinder_emboss_persists_selection_and_replays_radial_depth() {
    use solvecraft_geom::Vec3;
    let mut s = cylinder_badge();
    let before = exact_cylinder_volume(&s);
    run(
        &mut s,
        "solid.emboss",
        json!({"sketch":"Badge","profiles":"all","face":[0,10,7.5],"depth":"badge_depth","targets":["Sleeve"],"name":"Raised Badge"}),
    );
    assert!((exact_cylinder_volume(&s) - before - 21.0).abs() < 0.02);
    let state = s.model.state();
    let body = &state.body("Sleeve").unwrap().body;
    assert_eq!(body.shell_count(), 1);
    let mesh = body.tessellate_with(0.001, true).unwrap();
    assert!(mesh.contains(Vec3::new(-10.5 * 0.15_f64.sin(), 10.5 * 0.15_f64.cos(), 7.5)));
    let feature = s.doc.find_feature("Raised Badge").unwrap();
    assert!(
        matches!(&feature.kind,solvecraft_doc::FeatureKind::Emboss{face:Some(face),..} if face.body=="Sleeve"&&face.at.dist(Vec3::new(0.,10.,7.5))<1e-10)
    );
    let saved = s.doc.to_json();
    let mut loaded = Session { doc: solvecraft_doc::Document::from_json(&saved).unwrap().into(), ..Default::default() };
    loaded.refresh();
    assert!(loaded.model.results.iter().all(|f| f.error.is_none()), "{:?}", loaded.model.results);
    assert!((exact_cylinder_volume(&loaded) - before - 21.0).abs() < 0.02);
    run(&mut loaded, "parameters.change", json!({"name":"badge_depth","expression":"2 mm"}));
    assert!(loaded.model.results.iter().all(|f| f.error.is_none()), "{:?}", loaded.model.results);
    assert!((exact_cylinder_volume(&loaded) - before - 44.0).abs() < 0.02);
}

#[test]
fn cylinder_deboss_is_connected_and_unsupported_profile_failure_is_atomic() {
    let mut s = cylinder_badge();
    let before = exact_cylinder_volume(&s);
    run(&mut s, "solid.emboss", json!({"sketch":"Badge","face":[0,10,7.5],"depth":1,"mode":"deboss","targets":["Sleeve"]}));
    assert!((exact_cylinder_volume(&s) - before + 19.0).abs() < 0.02);
    assert_eq!(s.model.state().body("Sleeve").unwrap().body.shell_count(), 1);
    let mut bad = cylinder_badge();
    run(&mut bad, "sketch.create", json!({"plane":{"origin":[0,10,0],"x_dir":[-1,0,0],"y_dir":[0,0,1]},"name":"Circle Badge"}));
    run(&mut bad, "sketch.circle.center", json!({"center":[0,7.5],"radius":1}));
    run(&mut bad, "sketch.finish", json!({}));
    let saved = bad.doc.to_json();
    let v = exact_cylinder_volume(&bad);
    assert!(bad.execute("solid.emboss", &json!({"sketch":"Circle Badge","face":[0,10,7.5],"depth":1,"targets":["Sleeve"]})).is_err());
    assert_eq!(bad.doc.to_json(), saved);
    assert_eq!(exact_cylinder_volume(&bad), v);
}

#[test]
fn cylinder_small_chord_pick_is_saved_on_the_exact_surface() {
    let mut s = Session::default();
    run(&mut s, "solid.cylinder", json!({"radius":"0.1mm","height":"0.2mm","body_name":"Tiny"}));
    run(&mut s, "sketch.create", json!({"plane":{"origin":[0,0.1,0],"x_dir":[-1,0,0],"y_dir":[0,0,1]},"name":"Badge"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0":[-0.02,0.05],"p1":[0.02,0.1]}));
    run(&mut s, "sketch.finish", json!({}));
    let mesh = s.model.state().bodies.first().unwrap().mesh();
    let pick = mesh
        .triangles
        .iter()
        .filter_map(|tri| mesh.tri(tri))
        .map(|[a, b, c]| (a + b + c) / 3.)
        .find(|p| p.y > 0.08 && p.z > 0.04 && p.z < 0.16)
        .unwrap();
    assert!(((pick.x * pick.x + pick.y * pick.y).sqrt() - 0.1).abs() > 1e-7);
    run(&mut s, "solid.emboss", json!({"sketch":"Badge","profiles":"all","face":[pick.x,pick.y,pick.z],"depth":"0.01mm","targets":["Tiny"]}));
    let solvecraft_doc::FeatureKind::Emboss { face: Some(face), .. } = &s.doc.features.last().unwrap().kind else {
        panic!("missing face");
    };
    assert!(((face.at.x * face.at.x + face.at.y * face.at.y).sqrt() - 0.1).abs() < 1e-12);
    assert_eq!(s.model.state().bodies.first().unwrap().body.shell_count(), 1);
}

#[test]
fn cylinder_cross_component_emboss_keeps_source_owner_and_placements() {
    for (pick_offset, auto) in [(0., false), (80., false), (0., true), (80., true)] {
        let run = |s: &mut Session, id: &str, p: Value| run(s, id, auto_emboss_params(p, auto && matches!(id, "solid.emboss" | "timeline.redefine")));
        let mut s = Session::default();
        let a = run(&mut s, "component.create", json!({"name":"Source"}))["component"].as_u64().unwrap();
        run(&mut s, "solid.cylinder", json!({"radius":10,"height":20,"body_name":"Sleeve"}));
        let source_feature = s.model.state().bodies.first().unwrap().feature;
        let source_names = solvecraft_doc::naming::face_names(s.model.state().body("Sleeve").unwrap());
        let before = exact_cylinder_volume(&s);
        run(&mut s, "occurrence.move", json!({"occurrence":"Source:1","axis":[0,0,1],"angle":"90 deg","translate":[50,0,0]}));
        let second = run(&mut s, "occurrence.copy", json!({"component":a,"translate":[80,0,0]}))["occurrence"].as_u64().unwrap();
        run(&mut s, "component.activate", json!({"component":"root"}));
        let author = run(&mut s, "component.create", json!({"name":"Author"}))["component"].as_u64().unwrap();
        run(&mut s, "occurrence.move", json!({"occurrence":"Author:1","axis":[1,0,0],"angle":"90 deg","translate":[-100,40,30]}));
        let author_second = run(&mut s, "occurrence.copy", json!({"component":author,"translate":[0,100,0]}))["occurrence"].as_u64().unwrap();
        run(&mut s, "component.activate", json!({"component":author,"occurrence":author_second}));
        run(&mut s, "parameters.add", json!({"name":"badge_depth","expression":"1mm"}));
        run(&mut s, "sketch.create", json!({"plane":{"origin":[40.+pick_offset,0,0],"x_dir":[0,-1,0],"y_dir":[0,0,1]},"name":"Badge"}));
        run(&mut s, "sketch.rectangle.two_point", json!({"p0":[-2,5],"p1":[2,10]}));
        run(&mut s, "sketch.finish", json!({}));
        let placements = serde_json::to_value(&s.doc.occurrences).unwrap();
        let target = s
            .world_state()
            .bodies
            .iter()
            .find(|b| b.body.tessellate_with(0.001, true).unwrap().contains(solvecraft_geom::Vec3::new(40.5 + pick_offset, 0., 7.5)))
            .unwrap()
            .name
            .clone();
        run(
            &mut s,
            "solid.emboss",
            json!({"sketch":"Badge","profiles":"all","face":[40.+pick_offset,0,7.5],"depth":"badge_depth","targets":[target.clone()]}),
        );
        let solvecraft_doc::FeatureKind::Emboss { face: Some(face), .. } = &s.doc.features.last().unwrap().kind else {
            panic!("missing selection");
        };
        assert_eq!(face.path, if pick_offset == 0. { s.doc.first_path(a) } else { vec![second] });
        assert_eq!(face.author_path, vec![author_second]);
        let state = s.model.state();
        let body = state.body("Sleeve").unwrap();
        assert_eq!(body.feature, source_feature);
        assert_eq!(s.doc.body_component("Sleeve", body.feature), a);
        assert_eq!(serde_json::to_value(&s.doc.occurrences).unwrap(), placements);
        let selected = s.doc.features.last().unwrap().face_names.first().unwrap();
        assert!(!selected.is_empty() && source_names.contains(selected), "{selected:?} not in {source_names:?}");
        assert!((exact_cylinder_volume(&s) - before - 21.).abs() < 0.02);
        let world = s.world_state();
        for x in [39.5, 119.5] {
            assert!(world.bodies.iter().any(|b| b.body.tessellate_with(0.001, true).unwrap().contains(solvecraft_geom::Vec3::new(x, 0., 7.5))));
        }
        let mut loaded = Session { doc: solvecraft_doc::Document::from_json(&s.doc.to_json()).unwrap().into(), ..Default::default() };
        loaded.refresh();
        assert!(loaded.model.results.iter().all(|r| r.error.is_none()), "{:?}", loaded.model.results);
        run(&mut loaded, "parameters.change", json!({"name":"badge_depth","expression":"2mm"}));
        assert!((exact_cylinder_volume(&loaded) - before - 44.).abs() < 0.02);
        assert_eq!(loaded.doc.body_component("Sleeve", loaded.model.state().body("Sleeve").unwrap().feature), a);
        assert_eq!(serde_json::to_value(&loaded.doc.occurrences).unwrap(), placements);
        let feature = loaded.doc.features.last().unwrap().clone();
        run(&mut loaded, "component.activate", json!({"component":"root"}));
        run(
            &mut loaded,
            "timeline.redefine",
            json!({"feature":feature.id,"command":"solid.emboss","params":{"sketch":"Badge","profiles":"all","face":[40.+pick_offset,0,7.5],"depth":"badge_depth","targets":[target.clone()]}}),
        );
        assert_eq!(loaded.doc.feature(feature.id).unwrap().kind, feature.kind);
        assert_eq!(loaded.doc.feature(feature.id).unwrap().face_names, feature.face_names);
        run(
            &mut loaded,
            "timeline.redefine",
            json!({"feature":feature.id,"command":"solid.emboss","params":{"sketch":"Badge","profiles":"all","face":[40.+pick_offset,0,7.5],"depth":"3mm","targets":[target]}}),
        );
        assert!((exact_cylinder_volume(&loaded) - before - 69.).abs() < 0.02);
        let mut broken = loaded.doc.as_ref().clone();
        let id = broken.features.last().unwrap().id;
        let solvecraft_doc::FeatureKind::Emboss { face: Some(face), .. } = &mut broken.feature_mut(id).unwrap().kind else {
            panic!("missing selection");
        };
        face.path = vec![u64::MAX];
        let mut invalid = Session { doc: broken.into(), ..Default::default() };
        invalid.refresh();
        assert!(invalid.model.results.iter().find(|r| r.id == id).unwrap().error.as_ref().unwrap().contains("placement"));
        assert!((exact_cylinder_volume(&invalid) - before).abs() < 0.02);
    }
}

#[test]
fn cylinder_active_second_source_occurrence_authors_shared_geometry() {
    check_active_second_source(false);
    check_active_second_source(true);
}
fn check_active_second_source(auto: bool) {
    let run = |s: &mut Session, id: &str, p: Value| run(s, id, auto_emboss_params(p, auto && id == "solid.emboss"));
    let mut s = Session::default();
    let component = run(&mut s, "component.create", json!({"name":"Source"}))["component"].as_u64().unwrap();
    run(&mut s, "solid.cylinder", json!({"radius":10,"height":20,"body_name":"Sleeve"}));
    let before = exact_cylinder_volume(&s);
    let second = run(&mut s, "occurrence.copy", json!({"component":component,"translate":[80,0,0]}))["occurrence"].as_u64().unwrap();
    run(&mut s, "component.activate", json!({"component":component,"occurrence":second}));
    run(&mut s, "sketch.create", json!({"plane":{"origin":[80,10,0],"x_dir":[-1,0,0],"y_dir":[0,0,1]},"name":"Badge"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0":[-2,5],"p1":[2,10]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.emboss", json!({"sketch":"Badge","profiles":"all","face":[80,10,7.5],"depth":1}));
    assert!((exact_cylinder_volume(&s) - before - 21.).abs() < 0.02);
    let solvecraft_doc::FeatureKind::Emboss { face: Some(face), .. } = &s.doc.features.last().unwrap().kind else {
        panic!("missing selection");
    };
    assert_eq!(face.path, vec![second]);
    assert_eq!(face.author_path, vec![second]);
    let mut loaded = Session { doc: solvecraft_doc::Document::from_json(&s.doc.to_json()).unwrap().into(), ..Default::default() };
    loaded.refresh();
    assert!(loaded.model.results.iter().all(|r| r.error.is_none()), "{:?}", loaded.model.results);
    assert!((exact_cylinder_volume(&loaded) - before - 21.).abs() < 0.02);
}

#[test]
fn cylinder_pending_leaf_or_ancestor_positions_require_capture_atomically() {
    for (ancestor, auto) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut s = Session::default();
        let parent = run(&mut s, "component.create", json!({"name":"Parent"}))["occurrence"].as_u64().unwrap();
        let leaf = run(&mut s, "component.create", json!({"name":"Source"}))["occurrence"].as_u64().unwrap();
        run(&mut s, "solid.cylinder", json!({"radius":10,"height":20,"body_name":"Sleeve"}));
        run(&mut s, "sketch.create", json!({"plane":{"origin":[0,10,0],"x_dir":[-1,0,0],"y_dir":[0,0,1]},"name":"Badge"}));
        run(&mut s, "sketch.rectangle.two_point", json!({"p0":[-2,5],"p1":[2,10]}));
        run(&mut s, "sketch.finish", json!({}));
        run(&mut s, "occurrence.move", json!({"occurrence":if ancestor {parent}else{leaf},"translate":[5,0,0],"capture":false}));
        let before = s.doc.to_json();
        let pending = s.pending_moves.clone();
        let volume = exact_cylinder_volume(&s);
        let error = s
            .execute("solid.emboss", &auto_emboss_params(json!({"sketch":"Badge","profiles":"all","face":[5,10,7.5],"depth":1}), auto))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Capture Position"), "{error}");
        assert_eq!(s.doc.to_json(), before);
        assert_eq!(s.pending_moves, pending);
        assert_eq!(exact_cylinder_volume(&s), volume);
    }
}

#[test]
fn cylinder_pending_third_component_profile_requires_capture_atomically() {
    check_pending_third_profile(false);
    check_pending_third_profile(true);
}
fn check_pending_third_profile(auto: bool) {
    let mut s = Session::default();
    run(&mut s, "solid.cylinder", json!({"radius":10,"height":20,"body_name":"Sleeve"}));
    let profile_occurrence = run(&mut s, "component.create", json!({"name":"ProfileOwner"}))["occurrence"].as_u64().unwrap();
    run(&mut s, "sketch.create", json!({"plane":{"origin":[0,10,0],"x_dir":[-1,0,0],"y_dir":[0,0,1]},"name":"Badge"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0":[-2,5],"p1":[2,10]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "component.activate", json!({"component":"root"}));
    run(&mut s, "component.create", json!({"name":"Author"}));
    run(&mut s, "occurrence.move", json!({"occurrence":profile_occurrence,"translate":[5,0,0],"capture":false}));
    let before = s.doc.to_json();
    let pending = s.pending_moves.clone();
    let volume = exact_cylinder_volume(&s);
    let error = s
        .execute(
            "solid.emboss",
            &auto_emboss_params(json!({"sketch":"Badge","profiles":"all","face":[0,10,7.5],"depth":1,"targets":["Sleeve"]}), auto),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("Capture Position"), "{error}");
    assert_eq!(s.doc.to_json(), before);
    assert_eq!(s.pending_moves, pending);
    assert_eq!(exact_cylinder_volume(&s), volume);
}

#[test]
fn cylinder_body_offset_keeps_exact_profile_and_anchor_frames() {
    check_body_offset(false);
    check_body_offset(true);
}
fn check_body_offset(auto: bool) {
    let run = |s: &mut Session, id: &str, p: Value| run(s, id, auto_emboss_params(p, auto && id == "solid.emboss"));
    let mut s = Session::default();
    run(&mut s, "solid.cylinder", json!({"radius":10,"height":20,"body_name":"Sleeve"}));
    let before = exact_cylinder_volume(&s);
    let holder = run(&mut s, "component.create", json!({"name":"Holder"}))["component"].as_u64().unwrap();
    run(&mut s, "occurrence.move", json!({"occurrence":"Holder:1","translate":[0,50,0]}));
    std::sync::Arc::make_mut(&mut s.doc).body_components.insert("Sleeve".into(), holder);
    std::sync::Arc::make_mut(&mut s.doc)
        .body_offsets
        .insert("Sleeve".into(), [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [0., -50., 0., 1.]]);
    s.refresh();
    assert!(s.doc.body_offsets.contains_key("Sleeve"));
    let offset = s.doc.body_offsets["Sleeve"];
    run(&mut s, "sketch.create", json!({"plane":{"origin":[0,10,0],"x_dir":[-1,0,0],"y_dir":[0,0,1]},"name":"Badge"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0":[-2,5],"p1":[2,10]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.emboss", json!({"sketch":"Badge","profiles":"all","face":[0,10,7.5],"depth":1,"targets":["Sleeve"]}));
    assert!((exact_cylinder_volume(&s) - before - 21.).abs() < 0.02);
    assert_eq!(s.doc.body_offsets["Sleeve"], offset);
    assert_eq!(s.doc.body_component("Sleeve", s.model.state().body("Sleeve").unwrap().feature), holder);
    assert!(s.world_state().body("Sleeve").unwrap().body.tessellate_with(0.001, true).unwrap().contains(solvecraft_geom::Vec3::new(0., 10.5, 7.5)));
}

#[test]
fn replace_face_align_remove() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "body_name": "A"}));
    run(&mut s, "solid.replace_face", json!({"faces": [[5, 5, 10]], "target": {"origin": [0, 0, 15], "normal": [0, 0, 1]}}));
    assert!(rel(volume(&mut s), 1500.0) < 1e-6);
    let e = s.execute("solid.replace_face", &json!({"faces": [[5, 5, 15]], "target": {"origin": [0, 0, 15], "normal": [1, 0, 1]}}));
    assert!(e.is_err() || s.model.results.last().is_some_and(|r| r.error.as_deref().unwrap_or("").contains("parallel")));
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "body_name": "A"}));
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "corner": [50, 0, 0], "body_name": "B"}));
    // B's top face onto A's top face: B turns over and sits on A.
    run(&mut s, "solid.align", json!({"bodies": ["B"], "from_face": [55, 5, 10], "to_face": [5, 5, 10]}));
    let m = measure(&mut s);
    let b = m["bodies"].as_array().unwrap().iter().find(|b| b["name"] == "B").unwrap().clone();
    let (lo, hi) = (b["bbox"]["min"].clone(), b["bbox"]["max"].clone());
    for (got, want) in [(&lo[0], 0.0), (&lo[2], 10.0), (&hi[0], 10.0), (&hi[2], 20.0)] {
        assert!((got.as_f64().unwrap_or(f64::NAN) - want).abs() < 1e-6, "{b}");
    }
    // Points only: a plain move.
    run(&mut s, "solid.align", json!({"bodies": ["B"], "from": [0, 0, 10], "to": [0, 0, 30]}));
    let m = measure(&mut s);
    let b = m["bodies"].as_array().unwrap().iter().find(|b| b["name"] == "B").unwrap().clone();
    assert!((b["bbox"]["min"][2].as_f64().unwrap_or(0.0) - 30.0).abs() < 1e-6, "{b}");
    run(&mut s, "solid.remove", json!({"bodies": ["B"]}));
    assert_eq!(measure(&mut s)["body_count"], 1);
}

#[test]
fn coil_and_loft_to_a_point() {
    use std::f64::consts::{PI, TAU};
    let mut s = Session::default();
    run(&mut s, "solid.coil", json!({"diameter": 40, "revolutions": 2, "pitch": 8, "section_size": 4}));
    assert!(rel(volume(&mut s), PI * 4.0 * TAU * 20.0 * 2.0) < 1e-3, "{}", volume(&mut s));
    // Height and revolutions give the pitch; a square section outside the diameter.
    let mut s = Session::default();
    run(
        &mut s,
        "solid.coil",
        json!({"diameter": 40, "revolutions": 1.5, "height": 15, "section": "square", "section_size": 2, "section_position": "outside", "axis": "X"}),
    );
    assert!(rel(volume(&mut s), 4.0 * TAU * 21.0 * 1.5) < 1e-3, "{}", volume(&mut s));
    assert!(s.execute("solid.coil", &json!({"diameter": 40, "pitch": 1, "revolutions": 3, "section_size": 4})).is_err(), "runs into itself");
    assert!(s.execute("solid.coil", &json!({"diameter": 40, "section_size": 4, "pitch": 8})).is_err(), "two of three");
    // Pyramid: a square lofted to a point.
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "Base"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [-10, -10], "p1": [10, 10]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "sketch.create", json!({"plane": {"origin": [0, 0, 30], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]}, "name": "Top"}));
    run(&mut s, "sketch.point", json!({"point": [0, 0], "id": "apex"}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.loft", json!({"sections": [{"sketch": "Base"}, {"sketch": "Top", "point": "apex"}]}));
    assert!(rel(volume(&mut s), 400.0 * 30.0 / 3.0) < 1e-6);
    assert!(s.execute("solid.loft", &json!({"sections": [{"sketch": "Base"}, {"sketch": "Top", "point": "nope"}]})).is_err());
}

/// A 3 mm shelled basin (100 x 60 x 30, open top) with a fall lofted from its whole outline
/// 10 up down to a drain pad (`pad`: corners, 1 up), in either section order. The loft's top
/// edges lie on the outer walls; the cut used to fail for one order of the sections (an
/// `Option::unwrap()` panic inside the kernel, or "shell not oriented and closed").
fn basin_fall(pad: [f64; 4], reverse: bool, op: &str) -> (Session, std::result::Result<Value, String>) {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 100, "width": 60, "height": 30, "corner": [0, 0, 0]}));
    run(&mut s, "solid.shell", json!({"faces": [[50, 30, 30]], "thickness": 3}));
    run(&mut s, "sketch.create", json!({"plane": {"origin": [0, 0, 10], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]}, "name": "Rim"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [100, 60]}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "sketch.create", json!({"plane": {"origin": [0, 0, 1], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]}, "name": "Pad"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [pad[0], pad[1]], "p1": [pad[2], pad[3]]}));
    run(&mut s, "sketch.finish", json!({}));
    let mut secs = vec![json!({"sketch": "Rim"}), json!({"sketch": "Pad"})];
    if reverse {
        secs.reverse();
    }
    let r = s.execute("solid.loft", &json!({"sections": secs, "operation": op})).map_err(|e| e.to_string());
    (s, r)
}

#[test]
fn loft_cut_through_a_shelled_basin_in_either_order() {
    // What is left after the cut: the basin less the loft's part of the floor and walls
    // (integrated over the height), the same for a pad in any corner.
    let want = 41582.028;
    for pad in [[80.0, 40.0, 90.0, 50.0], [10.0, 40.0, 20.0, 50.0], [10.0, 10.0, 20.0, 20.0], [80.0, 10.0, 90.0, 20.0]] {
        for reverse in [false, true] {
            let (mut s, r) = basin_fall(pad, reverse, "cut");
            assert!(r.is_ok(), "{pad:?} reverse {reverse}: {r:?}");
            let v = volume(&mut s);
            assert!((v - want).abs() < 1.0, "{pad:?} reverse {reverse}: {v} vs {want}");
            assert_eq!(measure(&mut s)["body_count"], 1);
        }
    }
    // As a new body and joined: a valid solid of positive volume either way.
    for op in ["new", "join"] {
        let mut vols = Vec::new();
        for reverse in [false, true] {
            let (mut s, r) = basin_fall([80.0, 40.0, 90.0, 50.0], reverse, op);
            assert!(r.is_ok(), "{op} reverse {reverse}: {r:?}");
            vols.push(volume(&mut s));
        }
        assert!(vols[0] > 42948.0 && rel(vols[0], vols[1]) < 1e-6, "{op}: {vols:?}");
    }
}

#[test]
fn cylinder_auto_inference_saves_a_profile_anchor_and_replays_both_depth_signs() {
    use solvecraft_geom::Vec3;
    for mode in ["emboss", "deboss"] {
        let mut s = cylinder_badge();
        let before = exact_cylinder_volume(&s);
        let initial = s.doc.to_json();
        run(&mut s, "solid.emboss", json!({"sketch":"Badge","profiles":"all","depth":"badge_depth","mode":mode,"name":"Auto Badge"}));
        let feature = s.doc.find_feature("Auto Badge").unwrap().clone();
        let solvecraft_doc::FeatureKind::Emboss { face: Some(face), targets, .. } = &feature.kind else {
            panic!("missing inferred association");
        };
        assert_eq!(targets, &["Sleeve"]);
        assert_eq!(face.body, "Sleeve");
        assert!(face.at.z > 0. && face.at.z < 20.);
        assert!((face.at.x.hypot(face.at.y) - 10.).abs() < 1e-10);
        assert!(!feature.face_names.first().unwrap().is_empty());
        let expected = if mode == "emboss" { 21. } else { -19. };
        assert!((exact_cylinder_volume(&s) - before - expected).abs() < 0.02);
        let state = s.model.state();
        let body = &state.body("Sleeve").unwrap().body;
        assert_eq!(body.shell_count(), 1);
        assert_eq!(
            body.tessellate_with(0.001, true).unwrap().contains(Vec3::new(0., if mode == "emboss" { 10.5 } else { 9.5 }, 7.5)),
            mode == "emboss"
        );
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc.to_json(), initial);
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(s.doc.find_feature("Auto Badge").unwrap().kind, feature.kind);
        let mut loaded = Session { doc: solvecraft_doc::Document::from_json(&s.doc.to_json()).unwrap().into(), ..Default::default() };
        loaded.refresh();
        assert!(loaded.model.results.iter().all(|r| r.error.is_none()), "{:?}", loaded.model.results);
        run(
            &mut loaded,
            "timeline.redefine",
            json!({"feature":feature.id,"command":"solid.emboss","params":{"sketch":"Badge","profiles":"all","depth":"badge_depth","mode":mode}}),
        );
        assert_eq!(loaded.doc.feature(feature.id).unwrap().kind, feature.kind);
        assert_eq!(loaded.doc.feature(feature.id).unwrap().face_names, feature.face_names);
        run(&mut loaded, "parameters.change", json!({"name":"badge_depth","expression":"2mm"}));
        assert!((exact_cylinder_volume(&loaded) - before - if mode == "emboss" { 44. } else { -36. }).abs() < 0.02);
    }
}

#[test]
fn cylinder_auto_inference_ignores_remote_finite_tangent_walls_in_each_body_order() {
    for remote_first in [false, true] {
        let mut s = Session::default();
        for (name, z) in if remote_first { [("Remote", 100.), ("Sleeve", 0.)] } else { [("Sleeve", 0.), ("Remote", 100.)] } {
            run(&mut s, "solid.cylinder", json!({"radius":10,"height":20,"base":[0,0,z],"body_name":name}));
        }
        run(&mut s, "solid.cylinder", json!({"radius":8,"height":10,"base":[0,0,120],"body_name":"Remote Step"}));
        run(&mut s, "solid.combine", json!({"target":"Remote","tools":["Remote Step"],"operation":"join"}));
        run(&mut s, "sketch.create", json!({"plane":{"origin":[0,10,0],"x_dir":[-1,0,0],"y_dir":[0,0,1]},"name":"Badge"}));
        run(&mut s, "sketch.rectangle.two_point", json!({"p0":[-2,5],"p1":[2,10]}));
        run(&mut s, "sketch.finish", json!({}));
        let before = exact_cylinder_volume(&s);
        let remote = s.model.state().body("Remote").unwrap().body.tessellate_with(1e-4, true).unwrap().measure().volume;
        run(&mut s, "solid.emboss", json!({"sketch":"Badge","depth":1}));
        assert!((exact_cylinder_volume(&s) - before - 21.).abs() < 0.02);
        assert_eq!(s.model.state().body("Remote").unwrap().body.tessellate_with(1e-4, true).unwrap().measure().volume, remote);
    }
}

#[test]
fn cylinder_auto_inference_retains_selected_holes_and_nested_islands() {
    use solvecraft_geom::Vec3;
    for mode in ["emboss", "deboss"] {
        let mut s = cylinder_badge();
        run(&mut s, "sketch.edit", json!({"sketch":"Badge"}));
        run(&mut s, "sketch.rectangle.two_point", json!({"p0":[-1,6],"p1":[1,9]}));
        run(&mut s, "sketch.rectangle.two_point", json!({"p0":[-0.5,6.5],"p1":[0.5,8.5]}));
        run(&mut s, "sketch.finish", json!({}));
        let before = exact_cylinder_volume(&s);
        run(&mut s, "solid.emboss", json!({"sketch":"Badge","profiles":[{"point":[1.5,7.5]},{"point":[0,7.5]}],"depth":1,"mode":mode}));
        let depth = if mode == "emboss" { 1. } else { -1. };
        let expected = 16. * (depth + depth * depth / 20.);
        assert!((exact_cylinder_volume(&s) - before - expected).abs() < 0.02);
        let st = s.model.state();
        let body = &st.body("Sleeve").unwrap().body;
        assert_eq!(body.shell_count(), 1);
        let mesh = body.tessellate_with(0.001, true).unwrap();
        for (s, raised) in [(0., true), (0.75, false), (1.5, true)] {
            let theta = s / 10.0_f64;
            let radius = if mode == "emboss" { 10.5 } else { 9.5 };
            assert_eq!(mesh.contains(Vec3::new(-radius * theta.sin(), radius * theta.cos(), 7.5)), if mode == "emboss" { raised } else { !raised });
        }
    }
}

#[test]
fn cylinder_auto_inference_refuses_ambiguity_curves_and_seam_crossing_atomically() {
    let mut s = cylinder_badge();
    run(&mut s, "solid.cylinder", json!({"radius":10,"height":20,"body_name":"Coincident"}));
    let before = s.doc.to_json();
    let error = s.execute("solid.emboss", &json!({"sketch":"Badge","depth":1})).unwrap_err().to_string();
    assert!(error.contains("multiple tangent cylinders"), "{error}");
    assert_eq!(s.doc.to_json(), before);
    run(&mut s, "solid.emboss", json!({"sketch":"Badge","depth":1,"targets":["Sleeve"]}));
    assert!(matches!(&s.doc.features.last().unwrap().kind,solvecraft_doc::FeatureKind::Emboss{face:Some(face),..} if face.body=="Sleeve"));
    for circle in [false, true] {
        let mut s = cylinder_badge();
        run(&mut s, "sketch.create", json!({"plane":{"origin":[0,10,0],"x_dir":[-1,0,0],"y_dir":[0,0,1]},"name":"Unsupported"}));
        if circle {
            run(&mut s, "sketch.circle.center", json!({"center":[0,7.5],"radius":1}));
        } else {
            run(&mut s, "sketch.rectangle.two_point", json!({"p0":[-20,5],"p1":[20,10]}));
        }
        run(&mut s, "sketch.finish", json!({}));
        let before = s.doc.to_json();
        let volume = exact_cylinder_volume(&s);
        let error = s.execute("solid.emboss", &json!({"sketch":"Unsupported","depth":1})).unwrap_err().to_string();
        assert!(error.contains(if circle { "orthogonal line" } else { "seam" }), "{error}");
        assert_eq!(s.doc.to_json(), before);
        assert_eq!(exact_cylinder_volume(&s), volume);
    }
}
