use super::*;

/// A rounded square of four quadratic curves (a font's kind) about (20, 15).
fn rounded() -> Region2 {
    let p = Vec2::new;
    let q = |a: Vec2, apex: Vec2, b: Vec2| Seg2::Conic { a, apex, b, w: 1.0 };
    let segs = vec![
        q(p(10.0, 15.0), p(10.0, 5.0), p(20.0, 5.0)),
        q(p(20.0, 5.0), p(30.0, 5.0), p(30.0, 15.0)),
        q(p(30.0, 15.0), p(30.0, 25.0), p(20.0, 25.0)),
        q(p(20.0, 25.0), p(10.0, 25.0), p(10.0, 15.0)),
    ];
    Region2 { outer: Loop2 { segs }, holes: vec![] }
}

fn block() -> Body {
    crate::box_solid(Vec3::new(0.0, 0.0, 0.0), Vec3::new(40.0, 30.0, 10.0)).unwrap()
}

/// Its free-form faces (the tool's walls kept as the curves they were).
fn spline_faces(b: &Body) -> usize {
    b.solid.face_iter().filter(|f| matches!(f.surface(), mt::Surface::BSplineSurface(_) | mt::Surface::NurbsSurface(_))).count()
}

/// A shape of a font's curves engraved into a block is decided in 2D, its walls the exact curves.
#[test]
fn an_engraving_of_quadratic_curves_is_exact_in_2d() {
    let region = rounded();
    let area = region.outer.signed_area().abs();
    let tool = crate::extrude(&Plane::XY, std::slice::from_ref(&region), 9.0, 11.0).unwrap().remove(0);
    let out = prism_boolean(&block(), &tool, BoolOp::Cut).expect("the 2D route takes it").unwrap().unwrap();
    assert!(out.validity().is_empty(), "{:?}", out.validity());
    // Half the tool, measured as the result is (meshes of curved faces are off by about 1e-4).
    let taken = 12000.0 - crate::measure(&out).unwrap().volume;
    let half = crate::measure(&tool).unwrap().volume / 2.0;
    assert!((taken - half).abs() < 1e-6 * half && (taken - area).abs() < 2e-4 * area, "{taken} vs {half} ({area})");
    assert_eq!(spline_faces(&out), 4);
}

/// A shape running off the block's side is cut where its curves exactly cross the side.
#[test]
fn an_engraving_running_off_the_side_ends_on_its_exact_crossings() {
    let region = rounded();
    // Moved 14 to the left: the block's side x = 0 cuts its two left curves.
    let shift = |s: &Seg2| match *s {
        Seg2::Conic { a, apex, b, w } => {
            Seg2::Conic { a: a - Vec2::new(14.0, 0.0), apex: apex - Vec2::new(14.0, 0.0), b: b - Vec2::new(14.0, 0.0), w }
        }
        other => other,
    };
    let moved = Region2 { outer: Loop2 { segs: region.outer.segs.iter().map(shift).collect() }, holes: vec![] };
    let tool = crate::extrude(&Plane::XY, std::slice::from_ref(&moved), 9.0, 11.0).unwrap().remove(0);
    let out = prism_boolean(&block(), &tool, BoolOp::Cut).expect("the 2D route takes it").unwrap().unwrap();
    assert!(out.validity().is_empty(), "{:?}", out.validity());
    // What the block keeps of the shape: the shape's part at x > 0.
    let inside = planar_regions_boolean(
        std::slice::from_ref(&moved),
        &[Region2 {
            outer: Loop2::polygon(&[Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0), Vec2::new(40.0, 30.0), Vec2::new(0.0, 30.0)]),
            holes: vec![],
        }],
        BoolOp::Intersect,
        1e-6,
    )
    .unwrap();
    let area: f64 = inside.iter().map(|r| r.outer.signed_area().abs()).sum();
    let taken = 12000.0 - crate::measure(&out).unwrap().volume;
    assert!((taken - area).abs() < 2e-4 * area, "{taken} vs {area}");
    assert!(spline_faces(&out) >= 2, "{}", spline_faces(&out));
}

/// A tool that misses its target leaves it as it is: the 2D route declines it (the B-rep
/// boolean then reuses the target itself). An engraving taking far less than a ten-thousandth
/// of a large part is a change all the same, and the route keeps it: told from the exact
/// sections, not from volumes measured off meshes, which could not tell it from no change.
#[test]
fn a_tool_that_misses_is_declined_and_a_fine_engraving_is_not() {
    let big = crate::box_solid(Vec3::new(0.0, 0.0, 0.0), Vec3::new(200.0, 200.0, 10.0)).unwrap();
    let region = rounded();
    let tool = |lo: f64, hi: f64| crate::extrude(&Plane::XY, std::slice::from_ref(&region), lo, hi).unwrap().remove(0);
    assert!(prism_boolean(&big, &tool(10.0, 12.0), BoolOp::Cut).is_none(), "a tool above the part misses it");
    // 0.05 mm deep: 17 mm³ of a 400000 mm³ part.
    let out = prism_boolean(&big, &tool(9.95, 11.0), BoolOp::Cut).expect("an engraving is a change").unwrap().unwrap();
    let taken = 400_000.0 - crate::measure(&out).unwrap().volume;
    let want = region.outer.signed_area().abs() * 0.05;
    assert!((taken - want).abs() < 2e-3 * want, "{taken} vs {want}");
}
