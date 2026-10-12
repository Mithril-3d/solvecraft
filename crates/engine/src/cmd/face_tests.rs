#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use crate::Session;
use serde_json::{Value, json};
use solvecraft_doc::{Document, FeatureKind, PlaneRef};
use solvecraft_geom::Vec3;
use std::f64::consts::PI;

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap_or_else(|e| panic!("{command}: {e}"))
}
fn check(s: &mut Session, want: f64, hole: bool, height: f64) {
    s.refresh();
    assert!(s.model.results.iter().all(|result| result.error.is_none() && result.warning.is_none()), "{:?}", s.model.results);
    let state = s.model.state();
    let got: f64 = state.bodies.iter().map(|b| solvecraft_kernel::measure(&b.body).unwrap().volume).sum();
    // Legacy linked outlines fit tessellated curves; enforce the existing 1e-4 material envelope.
    assert!((got - want).abs() < want * 1e-4, "{got} vs {want}");
    assert!(state.bodies.iter().any(|b| b.body.contains(Vec3::new(5.0, 5.0, height - 1.0))));
    assert!(!state.bodies.iter().any(|b| b.body.contains(Vec3::new(5.0, 5.0, height + 1.0))));
    if hole {
        assert!(!state.bodies.iter().any(|b| b.body.contains(Vec3::new(20.0, 15.0, height - 1.0))));
    }
}
#[test]
fn generated_face_profiles_keep_exact_links_live_after_edit_and_reopen() {
    for hole in [false, true] {
        let mut s = Session::default();
        let base = run(&mut s, "solid.box", json!({"length":40,"width":30,"height":20,"name":"Base"}))["feature"].as_u64().unwrap();
        if hole {
            run(&mut s, "sketch.create", json!({"plane":"XY"}));
            run(&mut s, "sketch.circle.center", json!({"center":[20,15],"radius":5}));
            run(&mut s, "sketch.finish", json!({}));
            run(&mut s, "solid.extrude", json!({"through_all":true,"operation":"cut"}));
        }
        let result = run(&mut s, "solid.extrude", json!({"face":[5,5,20],"distance":10,"operation":"new","name":"Pad"}));
        let pad = s.doc.feature(result["feature"].as_u64().unwrap()).unwrap();
        let FeatureKind::Extrude { sketch, .. } = pad.kind else { panic!("extrude") };
        let generated = s.doc.feature(sketch).unwrap();
        let FeatureKind::Sketch { plane, sketch: profile } = &generated.kind else { panic!("sketch") };
        let PlaneRef::Face { anchor: Some(anchor), .. } = plane else { panic!("the original Box cap remains exact after a hole cut") };
        assert_eq!(anchor.target.origin_feature, base);
        assert!(matches!(anchor.target.certificate, Some(solvecraft_doc::config::FaceCertificate::Box { axis: 2, high: true })));
        assert!(!profile.links.is_empty());
        assert!(profile.links.iter().all(|link| link.auto_face && !link.lost));
        let area = 1200.0 - if hole { 25.0 * PI } else { 0.0 };
        check(&mut s, area * 30.0, hole, 30.0);
        run(&mut s, "timeline.edit", json!({"feature":"Base","set":{"height":"25","length":"50"}}));
        let area = 1500.0 - if hole { 25.0 * PI } else { 0.0 };
        check(&mut s, area * 35.0, hole, 35.0);
        let edited = s.model.state();
        assert!(edited.bodies.iter().any(|body| body.body.contains(Vec3::new(49.0, 5.0, 34.0))));
        assert!(!edited.bodies.iter().any(|body| body.body.contains(Vec3::new(51.0, 5.0, 34.0))));
        assert!(edited.sketch(sketch).unwrap().sketch.links.iter().all(|link| link.auto_face && !link.lost));
        let solved = edited.sketch(sketch).unwrap().plane;
        assert!((solved.origin.z - 25.0).abs() < 1e-6);
        let before = s.model.state();
        let before_volume: f64 = before.bodies.iter().map(|body| solvecraft_kernel::measure(&body.body).unwrap().volume).sum();
        let before_bounds: Vec<_> = before.bodies.iter().map(|body| (body.name.clone(), body.mesh().bounds())).collect();
        let mut reopened = Session::new(Document::from_json(&s.doc.to_json()).unwrap());
        check(&mut reopened, area * 35.0, hole, 35.0);
        let after = reopened.model.state();
        let after_volume: f64 = after.bodies.iter().map(|body| solvecraft_kernel::measure(&body.body).unwrap().volume).sum();
        assert!((before_volume - after_volume).abs() < 1e-6);
        for (name, bounds) in before_bounds {
            let cold = after.body(&name).unwrap().mesh().bounds();
            assert!(bounds.min.dist(cold.min) < 1e-9 && bounds.max.dist(cold.max) < 1e-9);
        }
    }
}
#[test]
fn internal_capture_does_not_bypass_missing_ambiguous_or_invalid_path_picks() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length":40,"width":30,"height":20,"body_name":"A"}));
    run(&mut s, "solid.box", json!({"length":40,"width":30,"height":20,"body_name":"B"}));
    let before = s.doc.to_json();
    assert!(s.execute("solid.extrude", &json!({"face":[5,5,20],"distance":10,"operation":"new"})).is_err());
    assert_eq!(s.doc.to_json(), before);
    for pick in [json!({"face":[5,5,20]}), json!({"face":[500,5,20]}), json!({"face":[5,5,20],"body":"A","index":0,"path":[999999]})] {
        assert!(super::super::sketch::face::capture_internal(&s, &pick, "internal").is_err(), "{pick}");
    }
}

#[test]
fn generated_profiles_and_press_pull_convert_rotated_occurrence_picks_once() {
    for hole in [false, true] {
        let mut s = Session::default();
        let component = run(&mut s, "component.create", json!({"name":"Arm"}));
        let cid = component["component"].as_u64().unwrap();
        let occ = component["occurrence"].as_u64().unwrap();
        run(&mut s, "solid.box", json!({"length":40,"width":30,"height":20,"name":"Base","body_name":"Stock"}));
        if hole {
            run(&mut s, "sketch.create", json!({"plane":"XY"}));
            run(&mut s, "sketch.circle.center", json!({"center":[20,15],"radius":5}));
            run(&mut s, "sketch.finish", json!({}));
            run(&mut s, "solid.extrude", json!({"through_all":true,"operation":"cut"}));
        }
        run(&mut s, "component.activate", json!({"component":0}));
        run(&mut s, "occurrence.move", json!({"occurrence":occ,"translate":[30,20,10],"axis":[1,0,0],"angle":"90 deg","capture":true}));
        run(&mut s, "component.activate", json!({"component":cid,"occurrence":occ}));
        run(&mut s, "solid.extrude", json!({"face":[35,0,15],"distance":10,"operation":"join"}));
        let area = 1200.0 - if hole { 25.0 * PI } else { 0.0 };
        check(&mut s, area * 30.0, hole, 30.0);
        run(&mut s, "solid.press_pull", json!({"face":[35,-10,15],"distance":5}));
        check(&mut s, area * 35.0, hole, 35.0);
        let before_cut = s.doc.to_json();
        run(&mut s, "solid.press_pull", json!({"face":[35,-15,15],"distance":-5}));
        let height = 30.0;
        check(&mut s, area * height, hole, height);
        let after_cut = s.doc.to_json();
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc.to_json(), before_cut);
        check(&mut s, area * 35.0, hole, 35.0);
        run(&mut s, "edit.redo", json!({}));
        assert_eq!(s.doc.to_json(), after_cut);
        check(&mut s, area * height, hole, height);
        let world = s.world_state();
        let occupied = Vec3::new(35.0, 21.0 - height, 15.0);
        let absent = Vec3::new(35.0, 19.0 - height, 15.0);
        assert!(world.bodies.iter().any(|body| body.body.contains(occupied)));
        assert!(!world.bodies.iter().any(|body| body.body.contains(absent)));
        if hole {
            assert!(!world.bodies.iter().any(|body| body.body.contains(Vec3::new(50.0, 21.0 - height, 25.0))));
        }
        let before_volume: f64 = world.bodies.iter().map(|body| solvecraft_kernel::measure(&body.body).unwrap().volume).sum();
        let before_bounds: Vec<_> = world.bodies.iter().map(|body| (body.name.clone(), body.mesh().bounds())).collect();
        let reopened = Session::new(Document::from_json(&s.doc.to_json()).unwrap());
        let after = reopened.world_state();
        assert!(after.bodies.iter().any(|body| body.body.contains(occupied)));
        let v: f64 = after.bodies.iter().map(|body| solvecraft_kernel::measure(&body.body).unwrap().volume).sum();
        assert!((v - area * height).abs() < area * height * 1e-4);
        assert!((v - before_volume).abs() < 1e-6);
        for (name, bounds) in before_bounds {
            let cold = after.body(&name).unwrap().mesh().bounds();
            assert!(bounds.min.dist(cold.min) < 1e-9 && bounds.max.dist(cold.max) < 1e-9);
        }
    }
}

#[test]
fn legacy_internal_capture_refuses_an_uncaptured_source_in_another_component() {
    let mut s = Session::default();
    let component = run(&mut s, "component.create", json!({"name":"Source"}));
    let occurrence = component["occurrence"].as_u64().unwrap();
    run(&mut s, "solid.box", json!({"length":40,"width":30,"height":20,"body_name":"Stock"}));
    run(&mut s, "solid.press_pull", json!({"face":[5,5,20],"distance":5}));
    run(&mut s, "component.activate", json!({"component":0}));
    run(&mut s, "occurrence.move", json!({"occurrence":occurrence,"translate":[30,0,0],"capture":false}));
    let state = s.model.state();
    let index = state
        .body("Stock")
        .unwrap()
        .body
        .faces(1e-4)
        .unwrap()
        .iter()
        .find(|face| face.plane_normal.is_some_and(|normal| normal.z > 0.999) && (face.centroid.z - 25.0).abs() < 1e-6)
        .unwrap()
        .index;
    let before = s.doc.to_json();
    let error =
        super::super::sketch::face::capture_internal(&s, &json!({"face":[35,5,25],"body":"Stock","index":index,"path":[occurrence]}), "internal")
            .unwrap_err();
    assert!(error.to_string().contains("capture the source occurrence position"), "{error}");
    assert_eq!(s.doc.to_json(), before);
}

#[test]
fn repeated_source_face_profile_requires_and_uses_the_exact_active_occurrence() {
    let mut s = Session::default();
    let component = run(&mut s, "component.create", json!({"name":"Repeated"}));
    let cid = component["component"].as_u64().unwrap();
    run(&mut s, "solid.box", json!({"length":10,"width":10,"height":10,"body_name":"Stock"}));
    run(&mut s, "component.activate", json!({"component":0}));
    let copied = run(&mut s, "occurrence.copy", json!({"component":cid,"translate":[50,0,0]}));
    let occurrence = copied["occurrence"].as_u64().unwrap();
    let before = s.doc.to_json();
    let error = s.execute("solid.extrude", &json!({"face":[55,5,10],"distance":5,"operation":"new"})).unwrap_err();
    assert!(error.to_string().contains("activate the picked occurrence"), "{error}");
    assert_eq!(s.doc.to_json(), before);
    run(&mut s, "component.activate", json!({"component":cid,"occurrence":occurrence}));
    run(&mut s, "solid.extrude", json!({"face":[55,5,10],"distance":5,"operation":"new"}));
    let state = s.world_state();
    for x in [5.0, 55.0] {
        assert!(state.bodies.iter().any(|body| body.body.contains(Vec3::new(x, 5.0, 14.0))));
        assert!(!state.bodies.iter().any(|body| body.body.contains(Vec3::new(x, 5.0, 16.0))));
    }
}
