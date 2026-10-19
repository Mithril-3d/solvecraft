#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;

fn plane() -> Plane {
    Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y).unwrap()
}

fn view(scale: f64, pixel_scale: f64) -> PixelCurveView {
    PixelCurveView {
        vp: Mat4([[scale, 0.0, 0.0, 0.0], [0.0, scale, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]),
        width_px: 1400.0 * pixel_scale,
        height_px: 900.0 * pixel_scale,
    }
}

fn dense_error(s: &Seg2, plane: &Plane, v: &PixelCurveView, path: &SampledPath) -> f64 {
    let segments: Vec<_> =
        path.parts.iter().flat_map(|part| part.windows(2)).map(|w| (screen(v.vp.apply(w[0]), v), screen(v.vp.apply(w[1]), v))).collect();
    (0..=12_000)
        .filter_map(|i| {
            let q = v.vp.apply(plane.to_world(s.point_at(i as f64 / 12_000.0)));
            (distances(q, v, true).iter().all(|x| *x >= 0.0))
                .then(|| segments.iter().map(|(a, b)| distance_segment(screen(q, v), *a, *b)).fold(f64::INFINITY, f64::min))
        })
        .fold(0.0, f64::max)
}

#[test]
fn exact_arcs_ellipses_rational_and_looping_control_hulls_bound_pixels() {
    let mut degree7 = [Vec2::ZERO; solidcraft_geom::MAX_BEZIER_CTRL];
    for (i, p) in degree7.iter_mut().enumerate() {
        *p = Vec2::new(i as f64 / 4.0 - 0.875, if i % 2 == 0 { -0.8 } else { 0.8 });
    }
    let curves = [
        Seg2::Arc { center: Vec2::ZERO, radius: 1.0, start: 0.27, sweep: -5.8 },
        Seg2::Conic { a: Vec2::new(0.8, 0.0), apex: Vec2::new(0.8, 0.025), b: Vec2::new(0.0, 0.025), w: 0.5_f64.sqrt() },
        Seg2::Conic { a: Vec2::new(-0.8, -0.4), apex: Vec2::new(0.0, 0.8), b: Vec2::new(0.8, -0.4), w: 0.06 },
        Seg2::Cubic { p0: Vec2::new(-0.8, 0.0), p1: Vec2::new(0.9, 0.9), p2: Vec2::new(-0.9, -0.9), p3: Vec2::new(0.8, 0.0) },
        Seg2::Cubic { p0: Vec2::new(0.0, -0.4), p1: Vec2::new(1.0, 0.9), p2: Vec2::new(-1.0, 0.9), p3: Vec2::new(0.0, -0.4) },
        Seg2::Bezier { n: 7, p: degree7 },
    ];
    for scale in [0.003, 0.03, 0.3, 1.3] {
        for pixels in [1.0, 1.5, 2.0] {
            for s in curves {
                let v = view(scale, pixels);
                let path = sample_segment(&s, &plane(), &v, &mut SamplingBudget::default());
                assert_eq!(path.quality, SamplingQuality::Complete, "{s:?}");
                assert!(dense_error(&s, &plane(), &v, &path) <= 0.501, "{s:?}");
                assert!(path.vertex_count < 1024);
            }
        }
    }
}

#[test]
fn fit_circle_zoom_budget_and_closed_unclipped_fill() {
    let s = Seg2::Arc { center: Vec2::ZERO, radius: 1.0, start: 0.0, sweep: std::f64::consts::TAU };
    let small = sample_segment(&s, &plane(), &view(0.003, 1.0), &mut SamplingBudget::default());
    let fitted = sample_segment(&s, &plane(), &view(219.0 / 700.0, 1.0), &mut SamplingBudget::default());
    assert!(fitted.vertex_count > small.vertex_count);
    assert!(dense_error(&s, &plane(), &view(219.0 / 700.0, 1.0), &fitted) < 0.5);
    let fill = sample_segment_unclipped(&s, &plane(), &view(3.0, 2.0), &mut SamplingBudget::default());
    assert_eq!(fill.quality, SamplingQuality::Complete);
    assert_eq!(fill.parts.len(), 1);
    let part = &fill.parts[0];
    assert!(part.first().unwrap().dist(*part.last().unwrap()) < 1e-12);
    let limited = sample_segment_unclipped(&s, &plane(), &view(1e20, 1.0), &mut SamplingBudget::new(64, 1000));
    assert_eq!(limited.quality, SamplingQuality::BudgetLimited);
    assert!(limited.vertex_count <= 64);
    assert!(limited.parts.iter().flatten().all(|p| [p.x, p.y, p.z].iter().all(|v| v.is_finite())));
}

#[test]
fn perspective_tilt_exact_s_curve_and_clipping_do_not_bridge_gaps() {
    let p = Plane::new(Vec3::new(0.0, 0.0, -2.0), Vec3::new(0.8, 0.0, 0.6), Vec3::Y).unwrap();
    let v = PixelCurveView {
        vp: Mat4([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, -1.01, -1.0], [0.0, 0.0, -0.101, 0.0]]),
        width_px: 900.0,
        height_px: 720.0,
    };
    let s = Seg2::Cubic { p0: Vec2::new(-0.9, 0.0), p1: Vec2::new(0.9, 1.0), p2: Vec2::new(-0.9, -1.0), p3: Vec2::new(0.9, 0.0) };
    let path = sample_segment(&s, &p, &v, &mut SamplingBudget::default());
    assert_eq!(path.quality, SamplingQuality::Complete);
    assert!(dense_error(&s, &p, &v, &path) < 0.501);
    let crossing = Plane::new(Vec3::new(0.0, 0.0, -0.15), Vec3::new(0.6, 0.0, 0.8), Vec3::Y).unwrap();
    let s = Seg2::Arc { center: Vec2::ZERO, radius: 0.2, start: std::f64::consts::PI, sweep: std::f64::consts::TAU };
    let path = sample_segment_unclipped(&s, &crossing, &v, &mut SamplingBudget::default());
    assert!(!path.parts.is_empty());
    assert_eq!(path.quality, SamplingQuality::BudgetLimited);
    for point in path.parts.iter().flatten() {
        let q = v.vp.apply(*point);
        assert!(q[3] > 0.0 && q[2] >= -1e-10);
    }
    assert!(path.parts.len() >= 2);
}

#[test]
fn invalid_weights_matrices_and_shared_frame_exhaustion_are_explicit() {
    let bad = Seg2::Conic { a: Vec2::ZERO, apex: Vec2::X, b: Vec2::Y, w: -1.0 };
    assert_eq!(sample_segment(&bad, &plane(), &view(1.0, 1.0), &mut SamplingBudget::default()).quality, SamplingQuality::Invalid);
    let line = Seg2::Line { a: Vec2::ZERO, b: Vec2::X };
    let mut budget = SamplingBudget::new(2, 2);
    assert_eq!(sample_segment(&line, &plane(), &view(0.2, 1.0), &mut budget).quality, SamplingQuality::Complete);
    assert_eq!(sample_segment(&line, &plane(), &view(0.2, 1.0), &mut budget).quality, SamplingQuality::BudgetLimited);
    let mut bad = view(1.0, 1.0);
    bad.vp.0[0][0] = f64::NAN;
    assert_eq!(sample_segment(&line, &plane(), &bad, &mut SamplingBudget::default()).quality, SamplingQuality::Invalid);
    // The projected coordinates themselves are small, but huge VP summands cancel.
    // Do not certify accuracy from the small result alone.
    let far = Plane::new(Vec3::new(1e15, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
    let mut cancellation = view(0.2, 1.0);
    cancellation.vp.0[3][0] = -2e14;
    let path = sample_segment_unclipped(&line, &far, &cancellation, &mut SamplingBudget::new(16, 100));
    assert_eq!(path.quality, SamplingQuality::BudgetLimited);
    assert!(path.vertex_count <= 16);
    let plane_cancellation = Plane::new(Vec3::new(1e9, 0.0, 0.0), Vec3::X, Vec3::Y).unwrap();
    let local_circle = Seg2::Arc { center: Vec2::new(-1e9 + 1.0, 0.0), radius: 1.0, start: 0.0, sweep: std::f64::consts::TAU };
    let path = sample_segment_unclipped(&local_circle, &plane_cancellation, &view(1e9, 1.0), &mut SamplingBudget::new(16, 100));
    assert_eq!(path.quality, SamplingQuality::BudgetLimited);
    assert!(path.vertex_count <= 16);
}
