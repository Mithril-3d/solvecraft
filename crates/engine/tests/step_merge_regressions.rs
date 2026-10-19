//! STEP export regressions found by round-tripping the robustness designs: faces the writer
//! merges must keep their whole surface, so the body reads back as the same solid.

use serde_json::{Value, json};
use solidcraft_engine::Session;
use solidcraft_kernel::{measure, step_export, step_import};

fn run(s: &mut Session, script: Value) {
    for c in script.as_array().into_iter().flatten() {
        let (id, p) = (c["command"].as_str().unwrap_or_default(), &c["params"]);
        if let Err(e) = s.execute(id, p) {
            panic!("{id} {p}: {e}");
        }
    }
}

/// The design's one body exports, and reads back closed with the same volume and area.
fn round_trips(s: &Session) {
    let st = s.model.state();
    assert_eq!(st.bodies.len(), 1);
    let body = &st.bodies[0].body;
    let text = step_export(&[body], "test").unwrap_or_else(|e| panic!("export: {e}"));
    let imp = step_import(&text).unwrap_or_else(|e| panic!("import: {e}"));
    assert_eq!(imp.bodies.len(), 1, "{:?}", imp.warnings);
    assert!(imp.bodies[0].closed && imp.warnings.is_empty(), "{:?}", imp.warnings);
    let (a, b) = (measure(body).unwrap(), measure(&imp.bodies[0].body).unwrap());
    assert!((a.volume - b.volume).abs() < 1e-3 * a.volume, "volume {} -> {}", a.volume, b.volume);
    assert!((a.area - b.area).abs() < 1e-3 * a.area, "area {} -> {}", a.area, b.area);
}

/// Robustness seed 17, shrunk: a thin three-quarter tube shelled 1 mm with its bottom open. The
/// export merged each wall's three quarter pieces into one face but kept the first piece's
/// quarter sheet as its surface; the body read back closed, with no warning, 7% too small.
#[test]
fn shelled_thin_three_quarter_tube_keeps_its_volume() {
    let mut s = Session::default();
    run(
        &mut s,
        json!([
            {"command":"sketch.create","params":{"plane":"XZ"}},
            {"command":"sketch.rectangle.two_point","params":{"p0":[10.0,0],"p1":[12.5,27.5]}},
            {"command":"sketch.finish","params":{}},
            {"command":"solid.revolve","params":{"angle":"270 deg","axis":"Z"}},
            {"command":"solid.shell","params":{"faces":[[-7.182422893157272,7.182422893245109,0.0]],"thickness":1.0}}
        ]),
    );
    round_trips(&s);
}

/// The same tube turned further: the merged wall leaves a gap of 60°, 30° or 10°. Its two straight
/// end edges lie either side of the gap, so the mean direction of the face's points no longer
/// points into the face, and a seam put opposite it falls inside the face. The seam belongs in the
/// middle of the gap.
#[test]
fn shelled_thin_wide_tube_keeps_its_volume() {
    for turn in [300.0_f64, 330.0, 350.0] {
        let mid = (turn / 2.0).to_radians();
        let mut s = Session::default();
        run(
            &mut s,
            json!([
                {"command":"sketch.create","params":{"plane":"XZ"}},
                {"command":"sketch.rectangle.two_point","params":{"p0":[10.0,0],"p1":[12.5,27.5]}},
                {"command":"sketch.finish","params":{}},
                {"command":"solid.revolve","params":{"angle":format!("{turn} deg"),"axis":"Z"}},
                {"command":"solid.shell","params":{"faces":[[11.0 * mid.cos(), 11.0 * mid.sin(), 0.0]],"thickness":1.0}}
            ]),
        );
        let st = s.model.state();
        assert_eq!(st.bodies.len(), 1, "{turn} deg");
        let body = &st.bodies[0].body;
        let text = step_export(&[body], "test").unwrap_or_else(|e| panic!("{turn} deg export: {e}"));
        let imp = step_import(&text).unwrap_or_else(|e| panic!("{turn} deg import: {e}"));
        assert_eq!(imp.bodies.len(), 1, "{turn} deg {:?}", imp.warnings);
        assert!(imp.bodies[0].closed && imp.warnings.is_empty(), "{turn} deg {:?}", imp.warnings);
        let (a, b) = (measure(body).unwrap(), measure(&imp.bodies[0].body).unwrap());
        assert!((a.volume - b.volume).abs() < 1e-3 * a.volume, "{turn} deg volume {} -> {}", a.volume, b.volume);
        assert!((a.area - b.area).abs() < 1e-3 * a.area, "{turn} deg area {} -> {}", a.area, b.area);
    }
}
