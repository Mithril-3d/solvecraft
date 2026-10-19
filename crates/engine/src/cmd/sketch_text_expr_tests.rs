//! A sketch text's height, place and angle can be expressions of the design's parameters: the
//! text keeps the expression, follows the parameters, and a number typed instead replaces it.

use serde_json::{Value, json};
use solidcraft_geom::Vec2;
use solidcraft_sketch::LinkSource;

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// Lower-left and upper-right corners of the outlines of the text in sketch `S`.
fn text_box(s: &Session) -> (Vec2, Vec2) {
    let id = s.doc.find_feature("S").unwrap().id;
    let st = s.model.state();
    let sk = &st.sketch(id).unwrap().sketch;
    let (mut lo, mut hi) = (Vec2::new(f64::MAX, f64::MAX), Vec2::new(f64::MIN, f64::MIN));
    for q in sk.points.iter().filter(|q| q.link.is_some()) {
        lo = Vec2::new(lo.x.min(q.pos.x), lo.y.min(q.pos.y));
        hi = Vec2::new(hi.x.max(q.pos.x), hi.y.max(q.pos.y));
    }
    (lo, hi)
}

fn no_trouble(s: &Session) {
    for r in &s.model.results {
        assert!(r.error.is_none() && r.warning.is_none(), "{}: {:?} {:?}", r.name, r.error, r.warning);
    }
}

/// The numbers and expressions stored for the text `link` of sketch `S`.
fn stored(s: &Session, link: &str) -> (Vec2, f64, f64, solidcraft_sketch::TextExprs) {
    let id = s.doc.find_feature("S").unwrap().id;
    let LinkSource::Text { at, height, angle, exprs, .. } = &s.doc.sketch(id).unwrap().link(link).unwrap().source else { panic!("not a text") };
    (*at, *height, *angle, exprs.clone())
}

fn reopened(s: &Session) -> Session {
    Session::new(solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap())
}

#[test]
fn a_texts_height_and_place_typed_as_expressions_follow_their_parameters() {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "tall", "expression": "20 mm"}));
    run(&mut s, "parameters.add", json!({"name": "left", "expression": "30 mm"}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "S"}));
    // Capital letters without descenders: the outlines stand on the baseline, one height tall.
    let r = run(&mut s, "sketch.text", json!({"text": "LIVE", "at": ["left", 25], "height": "tall"}));
    let link = r["link"].as_str().unwrap().to_string();
    assert_eq!(r["exprs"], json!({"x": "left", "height": "tall"}), "{r}");
    run(&mut s, "sketch.finish", json!({}));
    no_trouble(&s);
    let (lo, hi) = text_box(&s);
    assert!((lo.y - 25.0).abs() < 1e-9 && (hi.y - 45.0).abs() < 1e-6, "{lo:?} {hi:?}");
    // The first letter's side bearing is a fixed share of the height.
    let bearing = (lo.x - 30.0) / 20.0;
    assert!(bearing > 0.0 && bearing < 0.5, "{lo:?}");

    // Smaller and further left: the outlines follow both parameters, and so do they after the
    // design is saved and opened again.
    run(&mut s, "parameters.change", json!({"name": "tall", "expression": "8 mm"}));
    run(&mut s, "parameters.change", json!({"name": "left", "expression": "5 mm"}));
    let check = |s: &Session, left: f64, tall: f64| {
        no_trouble(s);
        let (lo, hi) = text_box(s);
        assert!((lo.y - 25.0).abs() < 1e-9 && (hi.y - 25.0 - tall).abs() < 1e-6, "{lo:?} {hi:?}");
        assert!((lo.x - left - bearing * tall).abs() < 1e-6, "{lo:?}");
    };
    check(&s, 5.0, 8.0);
    let json = s.doc.to_json();
    assert!(json.contains("\"exprs\""), "the design file keeps the expressions");
    check(&reopened(&s), 5.0, 8.0);
    let (_, _, _, exprs) = stored(&s, &link);
    assert_eq!((exprs.x.as_deref(), exprs.y, exprs.height.as_deref(), exprs.angle), (Some("left"), None, Some("tall"), None));

    // Editing the sketch afterwards keeps the text where its parameters put it.
    run(&mut s, "sketch.edit", json!({"sketch": "S"}));
    run(&mut s, "sketch.point", json!({"point": [200, 200]}));
    run(&mut s, "sketch.edit_text", json!({"link": link, "text": "LIVE"}));
    run(&mut s, "sketch.finish", json!({}));
    check(&s, 5.0, 8.0);
    let (at, height, _, exprs) = stored(&s, &link);
    assert!((at.x - 5.0).abs() < 1e-9 && (height - 8.0).abs() < 1e-9 && exprs.height.as_deref() == Some("tall"), "{at:?} {height} {exprs:?}");
}

#[test]
fn a_texts_angle_can_be_an_expression_too() {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "turn", "expression": "0 deg"}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "S"}));
    let r = run(&mut s, "sketch.text", json!({"text": "LIVE", "at": [0, 0], "height": 10, "angle": "turn"}));
    let link = r["link"].as_str().unwrap().to_string();
    assert_eq!(r["exprs"], json!({"angle": "turn"}), "{r}");
    run(&mut s, "sketch.finish", json!({}));
    let (lo, hi) = text_box(&s);
    assert!(hi.x - lo.x > hi.y - lo.y, "along the x axis: {lo:?} {hi:?}");
    run(&mut s, "parameters.change", json!({"name": "turn", "expression": "90 deg"}));
    no_trouble(&s);
    let (lo, hi) = text_box(&s);
    assert!(hi.y - lo.y > hi.x - lo.x && hi.x.abs() < 1e-9 && lo.x < -9.0, "upright, the letters run up the y axis: {lo:?} {hi:?}");
    assert_eq!(stored(&s, &link).3.angle.as_deref(), Some("turn"));
    // Reopened, the text stands as the parameter says.
    let (lo, hi) = text_box(&reopened(&s));
    assert!(hi.y - lo.y > hi.x - lo.x, "{lo:?} {hi:?}");
}

#[test]
fn the_parameters_a_text_uses_are_listed_renamed_and_kept() {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "tall", "expression": "12 mm"}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "S"}));
    let r = run(&mut s, "sketch.text", json!({"text": "LIVE", "at": [0, 0], "height": "tall * 2"}));
    let link = r["link"].as_str().unwrap().to_string();
    run(&mut s, "sketch.finish", json!({}));
    let users = run(&mut s, "parameters.users", json!({"name": "tall"}));
    assert!(users.to_string().contains("Text height"), "{users}");
    assert!(users["drives"].to_string().contains("\"S\""), "{users}");
    let e = s.execute("parameters.delete", &json!({"name": "tall"})).unwrap_err().to_string();
    assert!(e.contains("Text height"), "{e}");
    run(&mut s, "parameters.rename", json!({"name": "tall", "new_name": "big"}));
    assert_eq!(stored(&s, &link).3.height.as_deref(), Some("big * 2"));
    no_trouble(&s);
    let (lo, hi) = text_box(&s);
    assert!((hi.y - lo.y - 24.0).abs() < 1e-6, "{lo:?} {hi:?}");
    run(&mut s, "parameters.change", json!({"name": "big", "expression": "5 mm"}));
    let (lo, hi) = text_box(&s);
    assert!((hi.y - lo.y - 10.0).abs() < 1e-6, "{lo:?} {hi:?}");
    // The edges of the dependency graph name it, too.
    let graph = run(&mut s, "parameters.graph", json!({}));
    assert!(graph["edges"].to_string().contains("big"), "{graph}");
}

#[test]
fn a_number_replaces_an_expression_and_plain_numbers_save_as_before() {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "tall", "expression": "20 mm"}));
    run(&mut s, "parameters.add", json!({"name": "left", "expression": "30 mm"}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "S"}));
    // Numbers, and expressions naming no parameter, are numbers: nothing new is saved.
    let r = run(&mut s, "sketch.text", json!({"text": "A", "at": [3, "2 mm + 2 mm"], "height": "10 mm", "angle": "30 deg"}));
    let plain = r["link"].as_str().unwrap().to_string();
    assert!(r.get("exprs").is_none(), "{r}");
    let (at, height, angle, exprs) = stored(&s, &plain);
    assert!(at.dist(Vec2::new(3.0, 4.0)) < 1e-12 && (height - 10.0).abs() < 1e-12 && (angle - 30f64.to_radians()).abs() < 1e-12, "{at:?}");
    assert!(exprs.is_empty());
    assert!(!s.doc.to_json().contains("exprs"), "a design without expressions saves as before");

    let r = run(&mut s, "sketch.text", json!({"text": "LIVE", "at": ["left", "left / 2"], "height": "tall"}));
    let link = r["link"].as_str().unwrap().to_string();
    // What an edit leaves out stays an expression; a number given replaces what it was.
    run(&mut s, "sketch.edit_text", json!({"link": link, "text": "LIVE!"}));
    assert_eq!(stored(&s, &link).3.height.as_deref(), Some("tall"));
    let r = run(&mut s, "sketch.edit_text", json!({"link": link, "height": 12}));
    assert_eq!(r["exprs"], json!({"x": "left", "y": "left / 2"}), "{r}");
    let (at, height, _, exprs) = stored(&s, &link);
    assert!((height - 12.0).abs() < 1e-12 && exprs.height.is_none() && at.dist(Vec2::new(30.0, 15.0)) < 1e-9, "{at:?} {height} {exprs:?}");
    run(&mut s, "sketch.edit_text", json!({"link": link, "at": [7, "left"]}));
    assert_eq!(run(&mut s, "sketch.edit_text", json!({"link": link}))["exprs"], json!({"y": "left"}));
    // Moving the text turns what moves into numbers; the height and the other coordinate stay.
    run(&mut s, "sketch.edit_text", json!({"link": link, "at": ["left", "left"], "height": "tall"}));
    let curve = s.doc.sketch(s.active_sketch.unwrap()).unwrap().link_curves(&link)[0];
    let curve = s.doc.sketch(s.active_sketch.unwrap()).unwrap().curves[curve].id.clone();
    run(&mut s, "sketch.move", json!({"entities": [curve], "translate": [10, 0]}));
    let (at, _, _, exprs) = stored(&s, &link);
    assert!((at.x - 40.0).abs() < 1e-9 && (at.y - 30.0).abs() < 1e-9, "{at:?}");
    assert_eq!((exprs.x, exprs.y.as_deref(), exprs.height.as_deref()), (None, Some("left"), Some("tall")));
}

#[test]
fn a_text_expression_that_cannot_be_used_is_refused_and_changes_nothing() {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "turn", "expression": "30 deg"}));
    run(&mut s, "parameters.add", json!({"name": "tall", "expression": "20 mm"}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "S"}));
    let r = run(&mut s, "sketch.text", json!({"text": "A", "at": [0, 0]}));
    let link = r["link"].as_str().unwrap().to_string();
    let before = s.doc.to_json();
    let undo = s.undo.len();
    for (extra, error) in [
        (json!({"at": ["nope", 0]}), "`at` x"),
        (json!({"at": [0, "turn"]}), "`at` y"),
        (json!({"at": [0, "tall +"]}), "`at` y"),
        (json!({"height": "nope"}), "unknown parameter"),
        (json!({"height": "turn"}), "`height`"),
        (json!({"height": "tall - 30 mm"}), "`height` must be positive"),
        (json!({"height": "-3 mm"}), "`height` must be positive"),
        (json!({"height": "0 * tall"}), "`height` must be positive"),
        (json!({"height": ""}), "`height` must be a length"),
        (json!({"angle": "tall"}), "`angle`"),
        (json!({"angle": "5 mm"}), "`angle`"),
        (json!({"angle": "nope"}), "unknown parameter"),
    ] {
        let mut create = json!({"text": "A", "at": [0, 0]});
        let mut edit = json!({"link": link});
        for (key, value) in extra.as_object().unwrap() {
            create[key] = value.clone();
            edit[key] = value.clone();
        }
        let e = s.execute("sketch.text", &create).unwrap_err().to_string();
        assert!(e.contains(error), "{create}: {e}");
        for live in [false, true] {
            edit["live"] = json!(live);
            let e = s.execute("sketch.edit_text", &edit).unwrap_err().to_string();
            assert!(e.contains(error), "{edit}: {e}");
        }
        assert_eq!(s.doc.to_json(), before, "{extra}");
        assert_eq!(s.undo.len(), undo, "{extra}");
    }
}

#[test]
fn a_parameter_that_leaves_a_text_no_height_is_an_error_of_its_sketch_not_a_crash() {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "tall", "expression": "12 mm"}));
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "S"}));
    run(&mut s, "sketch.text", json!({"text": "LIVE", "at": [0, 0], "height": "tall"}));
    run(&mut s, "sketch.finish", json!({}));
    no_trouble(&s);
    run(&mut s, "parameters.change", json!({"name": "tall", "expression": "0 mm"}));
    let sketch = s.model.results.iter().find(|r| r.name == "S").unwrap();
    assert!(sketch.error.as_deref().is_some_and(|e| e.contains("height")), "{:?}", sketch.error);
    run(&mut s, "parameters.change", json!({"name": "tall", "expression": "6 mm"}));
    no_trouble(&s);
    let (lo, hi) = text_box(&s);
    assert!((hi.y - lo.y - 6.0).abs() < 1e-6, "{lo:?} {hi:?}");
}

/// The height of the outlines of the texts in the sketch feature `id`.
fn outline_height(s: &Session, id: u64) -> f64 {
    let st = s.model.state();
    let sk = &st.sketch(id).unwrap().sketch;
    let ys: Vec<f64> = sk.points.iter().filter(|q| q.link.is_some()).map(|q| q.pos.y).collect();
    ys.iter().cloned().fold(f64::MIN, f64::max) - ys.iter().cloned().fold(f64::MAX, f64::min)
}

/// The first dimension's parameter of the sketch feature `id` and the expression of its text's height.
fn dimension_and_expression(s: &Session, id: u64) -> (String, Option<String>) {
    let sk = s.doc.sketch(id).unwrap();
    let dimension = sk.constraints.iter().find_map(|c| c.param.clone()).expect("a dimension with a parameter");
    let LinkSource::Text { exprs, .. } = &sk.links.iter().find(|l| matches!(l.source, LinkSource::Text { .. })).unwrap().source else {
        panic!("not a text")
    };
    (dimension, exprs.height.clone())
}

#[test]
fn a_cloned_components_text_follows_the_cloned_dimension_it_names_not_the_original() {
    let mut s = Session::default();
    let part = run(&mut s, "component.create", json!({"name": "Part"}))["component"].as_u64().unwrap();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "S"}));
    run(&mut s, "sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [60, 20]}));
    run(&mut s, "sketch.dimension", json!({"entities": ["l1"], "value": 60}));
    let original = s.doc.find_feature("S").unwrap().id;
    let own = s.doc.sketch(original).unwrap().constraints.iter().find_map(|c| c.param.clone()).expect("a dimension with a parameter");
    // The text's height is a quarter of the sketch's own dimension.
    run(&mut s, "sketch.text", json!({"text": "LIVE", "at": [5, 5], "height": format!("{own} / 4")}));
    run(&mut s, "sketch.finish", json!({}));
    no_trouble(&s);
    assert!((outline_height(&s, original) - 15.0).abs() < 1e-6);

    let copy = run(&mut s, "component.paste_new", json!({"component": part, "translate": [0, 50, 0]}))["component"].as_u64().unwrap();
    let clone = s.doc.features.iter().find(|f| f.component == copy && matches!(f.kind, solidcraft_doc::FeatureKind::Sketch { .. })).unwrap().id;
    let (theirs, expression) = dimension_and_expression(&s, clone);
    assert_ne!(theirs, own, "the copy's dimension is its own");
    assert_eq!(expression, Some(format!("{theirs} / 4")), "and so is the expression that uses it");
    no_trouble(&s);
    assert!((outline_height(&s, clone) - 15.0).abs() < 1e-6);
    run(&mut s, "parameters.change", json!({"name": theirs, "expression": "40"}));
    assert!((outline_height(&s, clone) - 10.0).abs() < 1e-6, "the copy follows its own dimension");
    assert!((outline_height(&s, original) - 15.0).abs() < 1e-6, "and the original stays");
    no_trouble(&s);
}

#[test]
fn bare_numbers_in_a_text_expression_are_in_the_designs_units() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane": "XY", "name": "S"}));
    s.doc_mut().units = "in".into();
    let r = run(&mut s, "sketch.text", json!({"text": "A", "at": [0, 0], "height": "1"}));
    let (_, height, _, exprs) = stored(&s, r["link"].as_str().unwrap());
    // Numbers stay millimetres for programmatic callers; an expression is in the design's units.
    assert!((height - 25.4).abs() < 1e-9 && exprs.is_empty(), "{height}");
    let r = run(&mut s, "sketch.text", json!({"text": "A", "at": [0, 0], "height": 2}));
    assert!((stored(&s, r["link"].as_str().unwrap()).1 - 2.0).abs() < 1e-12);
}
