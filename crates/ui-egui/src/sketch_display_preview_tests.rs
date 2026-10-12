use super::{Tool, preview_curve};
use crate::scenario::Harness;
use crate::viewport::Proj;
use serde_json::{Value, json};
use solvecraft_engine::geom::{Plane, Vec2, Vec3};
use solvecraft_engine::render::{Camera, Mat4};
use solvecraft_engine::sketch::Sketch;

fn tool(command: &str, points: &[[f64; 2]]) -> Tool {
    let mut tool = Tool::for_command(command).unwrap();
    tool.pts = points.iter().map(|p| (Vec2::new(p[0], p[1]), None)).collect();
    tool
}
fn spans(sk: &Sketch) -> Vec<solvecraft_engine::geom::Seg2> {
    sk.exact_segs(0).unwrap_or_else(|| sk.segs(0))
}
fn projection(scale: f64) -> Proj {
    Proj {
        cam: Camera::default(),
        vp: Mat4([[scale / 700.0, 0.0, 0.0, 0.0], [0.0, scale / 450.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.5, 1.0]]),
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0)),
    }
}
fn screen_error(paths: &[Vec<Vec3>], point: Vec3, proj: &Proj, ppp: f32) -> f32 {
    let point = proj.to_screen(point).unwrap();
    paths
        .iter()
        .flat_map(|part| part.windows(2))
        .map(|pair| {
            let a = proj.to_screen(pair[0]).unwrap();
            let b = proj.to_screen(pair[1]).unwrap();
            let ab = b - a;
            let t = if ab.length_sq() > 0.0 { ((point - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
            point.distance(a + ab * t) * ppp
        })
        .fold(f32::INFINITY, f32::min)
}

#[test]
fn pixel_preview_temporary_curves_match_committed_geometry_and_sampling() {
    let cases: Vec<(&str, Vec<[f64; 2]>, [f64; 2], Option<f64>, Value)> = vec![
        ("sketch.circle.center", vec![[0., 0.]], [1., 0.], None, json!({"center":[0,0],"radius":1})),
        ("sketch.circle.two_point", vec![[-1., 0.]], [1., 0.], None, json!({"p0":[-1,0],"p1":[1,0]})),
        ("sketch.circle.three_point", vec![[1., 0.], [-1., 0.]], [0., 1.], None, json!({"p0":[1,0],"p1":[-1,0],"p2":[0,1]})),
        ("sketch.arc.three_point", vec![[1., 0.], [-1., 0.]], [0., -1.], None, json!({"start":[1,0],"end":[-1,0],"through":[0,-1]})),
        (
            "sketch.arc.center_point",
            vec![[0., 0.], [1., 0.]],
            [0., -1.],
            Some(-std::f64::consts::FRAC_PI_2),
            json!({"center":[0,0],"start":[1,0],"sweep":-90}),
        ),
        ("sketch.ellipse", vec![[0., 0.], [1., 0.]], [0., 0.05], None, json!({"center":[0,0],"major":[1,0],"minor":[0,0.05]})),
        ("sketch.spline.fit_point", vec![[0., 0.], [0.3, 0.8], [0.6, -0.8]], [1., 0.], None, json!({"points":[[0,0],[0.3,0.8],[0.6,-0.8],[1,0]]})),
        (
            "sketch.spline.control_point_5",
            vec![[0., 0.], [0.2, 0.9], [0.4, -0.9], [0.6, 0.9], [0.8, -0.9]],
            [1., 0.],
            None,
            json!({"points":[[0,0],[0.2,0.9],[0.4,-0.9],[0.6,0.9],[0.8,-0.9],[1,0]]}),
        ),
        ("sketch.spline.fit_point", vec![[0., 0.], [1., 0.], [0.5, 1.]], [0., 0.], None, json!({"points":[[0,0],[1,0],[0.5,1]],"periodic":true})),
    ];
    for (command, points, hover, sweep, params) in cases {
        let tool = tool(command, &points);
        let preview = preview_curve(&tool, Vec2::new(hover[0], hover[1]), sweep).unwrap();
        let mut h = Harness::new();
        h.app.run("sketch.create", json!({"plane":"XY"})).unwrap();
        h.app.run(command, params).unwrap();
        let state = h.app.session.world_state();
        let committed = &state.sketch(h.app.session.active_sketch.unwrap()).unwrap().sketch;
        let a = spans(&preview);
        let b = spans(committed);
        assert_eq!(a.len(), b.len(), "{command}");
        for (a, b) in a.iter().zip(&b) {
            for i in 0..=50 {
                assert!(a.point_at(i as f64 / 50.0).dist(b.point_at(i as f64 / 50.0)) < 1e-8, "{command}");
            }
        }
        for ppp in [1.0, 1.5, 2.0] {
            for scale in [2.0, 20.0, 220.0, 1000.0] {
                let proj = projection(scale / f64::from(ppp));
                let temporary = crate::sketch_display::curve_parts(&preview, 0, &Plane::XY, &proj, ppp);
                let permanent = crate::sketch_display::curve_parts(committed, 0, &Plane::XY, &proj, ppp);
                assert_eq!(temporary, permanent, "{command}");
                assert!(temporary.iter().map(Vec::len).sum::<usize>() <= 65_536);
                let mut visible = 0;
                for segment in &a {
                    for i in 0..=300 {
                        let point = Plane::XY.to_world(segment.point_at(i as f64 / 300.0));
                        if !proj.rect.contains(proj.to_screen(point).unwrap()) {
                            continue;
                        }
                        visible += 1;
                        let error = screen_error(&temporary, point, &proj, ppp);
                        assert!(error <= 1.0, "{command}, scale{scale}, ppp{ppp}: {error}");
                    }
                }
                if scale <= 220.0 {
                    assert_eq!(visible, a.len() * 301, "the low/normal zoom fixture must test the entire curve: {command}");
                }
            }
        }
    }
}

#[test]
fn pixel_preview_fully_offscreen_circle_is_clipped_without_false_connections() {
    let curve = preview_curve(&tool("sketch.circle.center", &[[0., 0.]]), Vec2::X, None).unwrap();
    let proj = projection(1000.0);
    // The viewport half diagonal is about 832px: a 1000px circle never enters it.
    assert!((proj.rect.width() * 0.5).hypot(proj.rect.height() * 0.5) < 1000.0);
    assert!(crate::sketch_display::curve_parts(&curve, 0, &Plane::XY, &proj, 1.0).is_empty());
}

#[test]
fn pixel_preview_rejects_degenerate_and_unbounded_input_without_changing_document() {
    assert!(preview_curve(&tool("sketch.circle.center", &[[0., 0.]]), Vec2::new(f64::NAN, 0.), None).is_none());
    assert!(preview_curve(&tool("sketch.arc.three_point", &[[0., 0.], [1., 0.]]), Vec2::new(2., 0.), None).is_none());
    let points = vec![[0., 0.]; 500];
    assert!(preview_curve(&tool("sketch.spline.fit_point", &points), Vec2::X, None).is_none());
}
