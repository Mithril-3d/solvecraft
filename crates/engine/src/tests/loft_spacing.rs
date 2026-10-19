//! Lofted round tubes through sections that are not evenly spaced.
//!
//! A loft's surface used to pass through its sections at uniform parameters, so a section close to
//! its neighbour (a bore that runs 5 mm past the end of a tube whose other sections are 12 mm
//! apart, or 60 mm in a snail) made the surface overshoot between the others, by several times the
//! tube's radius. The overshoot also made booleans with the tube fail for some section counts and
//! not for their neighbours.

use std::f64::consts::PI;

use super::*;

/// `sections` circles of radius `radius` centred on an arc of radius `path` turning `angle`
/// (radians), in planes normal to the arc, lofted into the body `body`; `over` extends the
/// sections past both ends along the tangent.
fn tube(s: &mut Session, body: &str, radius: f64, path: f64, angle: f64, sections: usize, over: f64) {
    let mut stations: Vec<[f64; 4]> = (0..sections)
        .map(|i| {
            let t = angle * i as f64 / (sections - 1) as f64;
            [path * t.cos(), path * t.sin(), -t.sin(), t.cos()]
        })
        .collect();
    if over > 0.0 {
        let (a, b) = (stations[0], stations[sections - 1]);
        stations.insert(0, [a[0] - over * a[2], a[1] - over * a[3], a[2], a[3]]);
        stations.push([b[0] + over * b[2], b[1] + over * b[3], b[2], b[3]]);
    }
    let mut list = Vec::new();
    for (i, [x, y, tx, ty]) in stations.iter().enumerate() {
        let name = format!("{body}{i}");
        run(s, "sketch.create", json!({"plane": {"origin": [x, y, 0], "x_dir": [0, 0, 1], "y_dir": [ty, -tx, 0]}, "name": name}));
        run(s, "sketch.circle.center", json!({"center": [0, 0], "radius": radius}));
        run(s, "sketch.finish", json!({}));
        list.push(json!({"sketch": name}));
    }
    run(s, "solid.loft", json!({"sections": list, "operation": "new", "name": format!("Loft{body}"), "body_name": body}));
}

fn bounds(s: &mut Session, body: &str) -> ([f64; 3], [f64; 3]) {
    let m = run(s, "inspect.measure", json!({}));
    let b = m["bodies"].as_array().unwrap().iter().find(|b| b["name"] == body).unwrap();
    let get = |k: &str| -> [f64; 3] {
        let v = b["bbox"][k].as_array().unwrap();
        [v[0].as_f64().unwrap(), v[1].as_f64().unwrap(), v[2].as_f64().unwrap()]
    };
    (get("min"), get("max"))
}

#[test]
fn a_bore_with_a_short_overrun_stays_inside_its_tube() {
    // 14 sections 12 mm apart, and one more 5 mm beyond each end: the bore is a tube of radius 18
    // around the arc (z within 18 of the arc's plane), and used to reach z = 88.
    let mut s = Session::default();
    tube(&mut s, "Bore", 18.0, 100.0, PI / 2.0, 14, 5.0);
    let (lo, hi) = bounds(&mut s, "Bore");
    assert!(hi[2] < 18.0 + 0.1 && lo[2] > -18.0 - 0.1, "z from {} to {}", lo[2], hi[2]);
    let want = PI * 18.0 * 18.0 * (100.0 * PI / 2.0 + 10.0);
    let got = volume(&mut s);
    assert!(rel(got, want) < 2e-3, "{got} vs {want}");
}

#[test]
fn a_hollow_elbow_is_cut_for_any_number_of_sections() {
    let (path, outer, inner) = (100.0, 30.0, 18.0);
    // Each of these gave "no result" (the first six) or failed in another way.
    for (degrees, sections) in [(60.0, 5), (60.0, 7), (90.0, 10), (90.0, 14), (120.0, 7), (120.0, 13)] {
        let angle = degrees * PI / 180.0;
        let mut s = Session::default();
        tube(&mut s, "Outer", outer, path, angle, sections, 0.0);
        tube(&mut s, "Inner", inner, path, angle, sections, 5.0);
        run(&mut s, "solid.combine", json!({"target": "Outer", "tools": ["Inner"], "operation": "cut", "name": "Hollow"}));
        assert_eq!(s.model.state().bodies.len(), 1, "{degrees} degrees, {sections} sections");
        let want = PI * (outer * outer - inner * inner) * path * angle;
        let got = volume(&mut s);
        assert!(rel(got, want) < 3e-3, "{degrees} degrees, {sections} sections: {got} vs {want}");
    }
}
