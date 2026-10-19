//! A shell whose open face meets a concave edge must not cut into the wall past that edge.
//!
//! Lower box `[0, 25] × [0, 17.5] × [0, 35]`, top box `[7.5, 22.5] × [0, w] × [35, 42.5]` with
//! `w ≤ 17.5`, and the shell opens the top box's left face (x = 7.5). That face's bottom edge is
//! concave: the lower box's top runs on from it. With walls `t` thick, the cavity is
//!
//! - in the lower box: `[t, 25 − t] × [t, 17.5 − t] × [t, 35 − t]`
//! - in the top box, out to the open face: `[7.5, 22.5 − t] × [t, w − t] × [35 − t, 42.5 − t]`
//!
//! The shell's volume is the solid's volume less those two boxes. The open face used to be
//! pushed out past the lower box's top, cutting a strip through that wall.
use serde_json::json;
use solidcraft_engine::Session;

fn run(s: &mut Session, id: &str, p: serde_json::Value) {
    if let Err(e) = s.execute(id, &p) {
        panic!("{id} {p}: {e}");
    }
}

fn shell_volume(w: f64, t: f64) -> f64 {
    let solid = 25.0 * 17.5 * 35.0 + 15.0 * w * 7.5;
    let lower = (25.0 - 2.0 * t) * (17.5 - 2.0 * t) * (35.0 - 2.0 * t);
    let upper = (15.0 - t) * (w - 2.0 * t) * 7.5;
    solid - lower - upper
}

fn shelled(w: f64, t: f64) -> Session {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"height": 35.0, "length": 25.0, "width": 17.5}));
    run(&mut s, "solid.box", json!({"corner": [7.5, 0.0, 35.0], "height": 7.5, "length": 15.0, "operation": "join", "width": w}));
    run(&mut s, "solid.shell", json!({"faces": [[7.5, w / 2.0, 38.75]], "thickness": t}));
    s
}

fn check(s: &Session, want: f64) {
    let errors: Vec<_> = s.model.results.iter().filter_map(|r| r.error.clone()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let st = s.model.state();
    assert_eq!(st.bodies.len(), 1, "one body");
    let b = &st.bodies[0].body;
    assert!(b.validity().is_empty(), "{:?}", b.validity());
    assert_eq!(b.shell_count(), 1, "one shell");
    let got = solidcraft_kernel::measure(b).map(|m| m.volume).unwrap_or(f64::NAN);
    assert!((got - want).abs() < 1e-6 * want, "shell volume {got}, want {want}");
}

#[test]
fn open_face_flush_with_the_back_keeps_the_wall_past_its_concave_edge() {
    for t in [1.0, 1.5] {
        check(&shelled(17.5, t), shell_volume(17.5, t));
    }
}

#[test]
fn open_face_inside_the_footprint_keeps_the_wall_past_its_concave_edge() {
    check(&shelled(15.0, 1.5), shell_volume(15.0, 1.5));
}
