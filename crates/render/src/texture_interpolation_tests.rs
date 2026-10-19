use super::*;
const UV: [[f64; 2]; 3] = [[0., 0.], [1., 0.], [0., 1.]];
const WEIGHTS: [f64; 3] = [0.5, 0.25, 0.25];
const DX: [f64; 3] = [-1., 1., 0.];
const DY: [f64; 3] = [-1., 0., 1.];

fn close(actual: [f64; 2], expected: [f64; 2]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 1e-10, "{actual:?} != {expected:?}");
    }
}

#[test]
fn orthographic_uv_and_derivatives_match_the_affine_plane() {
    let result = PerspectiveUv::new(UV, [1.; 3]).unwrap().evaluate(WEIGHTS, DX, DY).unwrap();
    close(result.uv, [0.25, 0.25]);
    close(result.duv_dx, [1., 0.]);
    close(result.duv_dy, [0., 1.]);
}

#[test]
fn perspective_quotient_and_derivatives_match_closed_form() {
    let result = PerspectiveUv::new(UV, [1., 2., 1.]).unwrap().evaluate(WEIGHTS, DX, DY).unwrap();
    close(result.uv, [0.4, 0.2]);
    close(result.duv_dx, [1.28, -0.16]);
    close(result.duv_dy, [0., 0.8]);
}

#[test]
fn common_clip_scale_and_vertex_order_preserve_the_result() {
    for scale in [1e-6, 1., 1e6] {
        for order in [[0, 1, 2], [2, 1, 0], [1, 2, 0]] {
            let q = [scale, 2. * scale, scale];
            let interpolation = PerspectiveUv::new(order.map(|i| UV[i]), order.map(|i| q[i])).unwrap();
            let result = interpolation.evaluate(order.map(|i| WEIGHTS[i]), order.map(|i| DX[i]), order.map(|i| DY[i])).unwrap();
            close(result.uv, [0.4, 0.2]);
            close(result.duv_dx, [1.28, -0.16]);
            close(result.duv_dy, [0., 0.8]);
        }
    }
}

#[test]
fn constant_large_offset_has_exactly_zero_gradients() {
    let interpolation = PerspectiveUv::new([[999_999., -999_999.]; 3], [1e-12, 1., 1e12]).unwrap();
    let result = interpolation.evaluate(WEIGHTS, DX, DY).unwrap();
    assert_eq!(result.uv, [999_999., -999_999.]);
    assert_eq!(result.duv_dx, [0.; 2]);
    assert_eq!(result.duv_dy, [0.; 2]);
}

#[test]
fn vertices_edges_and_shading_grid_units_remain_explicit() {
    let interpolation = PerspectiveUv::new(UV, [1., 2., 1.]).unwrap();
    close(interpolation.evaluate([0., 1., 0.], DX, DY).unwrap().uv, [1., 0.]);
    close(interpolation.evaluate([0.5, 0.5, 0.], DX, DY).unwrap().uv, [2. / 3., 0.]);
    let fine = interpolation.evaluate(WEIGHTS, DX.map(|v| v / 2.), DY.map(|v| v / 4.)).unwrap();
    close(fine.uv, [0.4, 0.2]);
    close(fine.duv_dx, [0.64, -0.08]);
    close(fine.duv_dy, [0., 0.2]);
}

#[test]
fn invalid_vertices_and_reciprocal_w_are_refused() {
    for value in [f64::NAN, f64::INFINITY, MAX_UV + 1., -MAX_UV - 1.] {
        assert!(matches!(PerspectiveUv::new([[value, 0.]; 3], [1.; 3]), Err(InterpolationError::VertexUv)));
    }
    for value in [f64::NAN, f64::INFINITY, 0., -1., 1e-13, 1e13] {
        assert!(matches!(PerspectiveUv::new(UV, [1., value, 1.]), Err(InterpolationError::ReciprocalW)));
    }
}

#[test]
fn malformed_partitions_and_gradients_are_refused() {
    let interpolation = PerspectiveUv::new(UV, [1.; 3]).unwrap();
    for weights in [[0.; 3], [1.; 3], [-0.1, 0.6, 0.5], [f64::NAN, 0., 1.], [f64::INFINITY, 0., 0.]] {
        assert_eq!(interpolation.evaluate(weights, DX, DY), Err(InterpolationError::Barycentrics));
    }
    for derivatives in [[1.; 3], [f64::NAN, 0., 0.], [f64::INFINITY, 0., 0.], [-MAX_DERIVATIVE - 1., MAX_DERIVATIVE + 1., 0.]] {
        assert_eq!(interpolation.evaluate(WEIGHTS, derivatives, DY), Err(InterpolationError::Derivatives));
        assert_eq!(interpolation.evaluate(WEIGHTS, DX, derivatives), Err(InterpolationError::Derivatives));
    }
}

#[test]
fn unsupported_output_gradient_refuses_without_affine_fallback() {
    let interpolation = PerspectiveUv::new(UV, [1e-12, 1e12, 1.]).unwrap();
    assert_eq!(interpolation.evaluate([1., 0., 0.], DX, DY), Err(InterpolationError::Output));
    assert!(PerspectiveUv::new(UV, [1.; 3]).unwrap().evaluate(WEIGHTS, DX, DY).is_ok());
}

#[test]
fn near_edge_admission_keeps_exact_weights_and_refuses_beyond_bound_or_nonpositive_denominator() {
    let e = BARYCENTRIC_EDGE_TOLERANCE;
    let interpolation = PerspectiveUv::new(UV, [1.; 3]).unwrap();
    let result = interpolation.evaluate([-e, 1. + 2. * e, -e], DX, DY).unwrap();
    close(result.uv, [1. + 2. * e, -e]);
    close(result.duv_dx, [1., 0.]);
    close(result.duv_dy, [0., 1.]);
    let outside = -e * 1.00001;
    assert_eq!(interpolation.evaluate([outside, 0.5, 0.5 - outside], DX, DY), Err(InterpolationError::Barycentrics));
    let perspective = PerspectiveUv::new(UV, [1e12, 1e-12, 1.]).unwrap();
    assert_eq!(perspective.evaluate([-e, 1. + e, 0.], DX, DY), Err(InterpolationError::Output));
}
