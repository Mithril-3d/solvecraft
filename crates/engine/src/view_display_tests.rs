use super::*;
use serde_json::json;
use solvecraft_geom::Vec2;
use solvecraft_sketch::{LinkGeom, LinkKind, LinkSource};

fn line(sk: &mut Sketch, y: f64) -> usize {
    sk.add_line(Vec2::new(0., y), Vec2::new(2., y), None, None, None).unwrap()
}

#[test]
fn sampler_keeps_style_precedence_hidden_references_text_and_disjoint_parts() {
    let mut sk = Sketch::new();
    let ordinary = line(&mut sk, 0.);
    let fixed = line(&mut sk, 1.);
    sk.curves[fixed].fixed = true;
    sk.curves[fixed].construction = true;
    let centerline = line(&mut sk, 2.);
    sk.curves[centerline].centerline = true;
    let construction = line(&mut sk, 3.);
    sk.curves[construction].construction = true;
    let determined = line(&mut sk, 4.);
    let projected = sk.add_link(LinkKind::Project, LinkSource::Origin, &[LinkGeom::Line(Vec2::new(0., 5.), Vec2::new(2., 5.))]).unwrap();
    let lost = sk.add_link(LinkKind::Project, LinkSource::Origin, &[LinkGeom::Line(Vec2::new(0., 6.), Vec2::new(2., 6.))]).unwrap();
    sk.set_link_lost(&lost, true);
    let text = sk
        .add_link(
            LinkKind::Text,
            LinkSource::Text { text: "A".into(), at: Vec2::ZERO, height: 2., angle: 0. },
            &[LinkGeom::Line(Vec2::new(0., 7.), Vec2::new(2., 7.))],
        )
        .unwrap();
    let mut determined_flags = vec![false; sk.curves.len()];
    determined_flags[determined] = true;
    let saved = serde_json::to_value(&sk).unwrap();
    let mut called = Vec::new();
    let lines = sketch_lines_with_sampler(&sk, &Plane::XY, true, &determined_flags, |_, i, _| {
        called.push(i);
        vec![vec![Vec3::new(i as f64, 0., 0.), Vec3::new(i as f64, 1., 0.)], vec![Vec3::new(i as f64, 3., 0.), Vec3::new(i as f64, 4., 0.)]]
    });
    assert_eq!(called, (0..sk.curves.len()).collect::<Vec<_>>());
    for (i, expected) in [
        colors::SKETCH,
        colors::SKETCH_LOCKED,
        colors::SKETCH_CENTERLINE,
        colors::SKETCH_CONSTRUCTION,
        colors::SKETCH_FIXED,
        colors::SKETCH_PROJECTED,
        colors::SKETCH_LOST,
        colors::SKETCH,
    ]
    .into_iter()
    .enumerate()
    {
        for part in &lines[2 * i..2 * i + 2] {
            assert_eq!(part.1, expected);
            assert_eq!(part.0.len(), 2);
        }
        assert_eq!(lines[2 * i].2, [fixed, centerline, construction].contains(&i));
    }
    assert_eq!(serde_json::to_value(&sk).unwrap(), saved);
    sk.view.hide_projected = true;
    called.clear();
    let finished = sketch_lines_with_sampler(&sk, &Plane::XY, false, &determined_flags, |_, i, _| {
        called.push(i);
        vec![vec![Vec3::ZERO, Vec3::X]]
    });
    assert_eq!(called, vec![ordinary, fixed, centerline, construction, determined, 7]);
    assert_eq!(finished[0].1, colors::SKETCH_DONE);
    assert_eq!(finished[4].1, colors::SKETCH_DONE);
    assert_eq!(finished[5].1, colors::SKETCH_DONE);
    assert!(sk.link(&projected).is_some() && sk.link(&text).is_some());
}

#[test]
fn legacy_paths_match_sampler_wrapper_and_placed_wires_are_unchanged() {
    let mut sk = Sketch::new();
    sk.add_circle(Vec2::new(2., 3.), 1., None, None).unwrap();
    sk.add_drawn_wire(&[Vec3::new(8., 9., 10.), Vec3::new(11., 12., 13.)]).unwrap();
    let plane = Plane::new(Vec3::new(20., 30., 40.), Vec3::Y, Vec3::Z).unwrap();
    let old = sketch_lines(&sk, &plane, true, &[false]);
    let adapted = sketch_lines_with_sampler(&sk, &plane, true, &[false], world_curve_parts);
    assert_eq!(old, adapted);
    let clipped = sketch_lines_with_sampler(&sk, &plane, true, &[], |_, _, _| Vec::new());
    assert_eq!(clipped, vec![(sk.wires[0].pts.clone(), colors::SKETCH, false)]);
    let mut included = sk.wires[0].clone();
    included.id = "included".into();
    included.fit.clear();
    sk.wires.push(included);
    let wires = sketch_lines_with_sampler(&sk, &plane, false, &[], |_, _, _| Vec::new());
    assert_eq!(wires[0].1, colors::SKETCH_DONE);
    assert_eq!(wires[1], (sk.wires[1].pts.clone(), colors::SKETCH_PROJECTED, false));
    sk.view.hide_projected = true;
    assert_eq!(sketch_lines_with_sampler(&sk, &plane, false, &[], |_, _, _| Vec::new()), vec![wires[0].clone()]);
}

#[test]
fn scene_sampler_uses_exact_active_occurrence_plane_and_preserves_other_scene_content() {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length":4,"width":5,"height":6})).unwrap();
    let component = s.execute("component.create", &json!({"name":"Part"})).unwrap()["component"].as_u64().unwrap();
    let occurrence = s.execute("occurrence.copy", &json!({"component":component,"translate":[20,30,40]})).unwrap()["occurrence"].as_u64().unwrap();
    s.execute("occurrence.move", &json!({"occurrence":occurrence,"angle":"90 deg","axis":[1,0,0],"origin":[20,30,40],"capture":false})).unwrap();
    s.execute("component.activate", &json!({"component":component,"occurrence":occurrence,"path":[occurrence]})).unwrap();
    s.execute("sketch.create", &json!({"plane":"XY"})).unwrap();
    s.execute("sketch.circle.center", &json!({"center":[2,3],"radius":1})).unwrap();
    s.section = Some((Vec3::new(0., 0., 3.), Vec3::Z));
    let cam = Camera::default();
    let old = scene(&s, &cam);
    let default = scene_with_sketch_sampler(&s, &cam, world_curve_parts);
    assert_eq!(old.lines.len(), default.lines.len());
    for (a, b) in old.lines.iter().zip(&default.lines) {
        assert_eq!((&a.points, a.color, a.width, a.on_top), (&b.points, b.color, b.width, b.on_top));
    }
    let mut planes = Vec::new();
    let parts = vec![vec![Vec3::new(22., 33., 40.), Vec3::new(23., 33., 40.)], vec![Vec3::new(25., 33., 40.), Vec3::new(26., 33., 40.)]];
    let sampled = scene_with_sketch_sampler(&s, &cam, |_, _, plane| {
        planes.push(*plane);
        parts.clone()
    });
    assert_eq!(planes.len(), 1);
    assert!(planes[0].origin.dist(Vec3::new(20., 30., 40.)) < 1e-9);
    assert!(planes[0].x.dist(Vec3::X) < 1e-9);
    assert!(planes[0].y.dist(Vec3::Z) < 1e-9);
    assert_eq!(sampled.clip, old.clip);
    assert_eq!(sampled.background, old.background);
    assert_eq!(sampled.radius, old.radius);
    assert_eq!(sampled.meshes.len(), old.meshes.len());
    assert!(Arc::ptr_eq(&sampled.meshes[0].mesh, &old.meshes[0].mesh));
    assert_eq!(sampled.meshes[0].color, old.meshes[0].color);
    assert_eq!(sampled.meshes[0].face_colors, old.meshes[0].face_colors);
    let lines = &sampled.lines[sampled.lines.len() - 2..];
    for (line, part) in lines.iter().zip(parts) {
        assert_eq!(line.points, part);
        assert!(line.on_top);
        assert_eq!(line.width, 1.6);
    }
    let active = s.active_sketch.unwrap();
    s.execute("sketch.finish", &json!({})).unwrap();
    s.visibility.hidden_sketches.push(active);
    let hidden = scene_with_sketch_sampler(&s, &cam, |_, _, _| panic!("hidden sketch sampled"));
    assert!(hidden.lines.iter().all(|line| !line.on_top));
}
