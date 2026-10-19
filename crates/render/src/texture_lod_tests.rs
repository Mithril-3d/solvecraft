use super::*;

fn close(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-12 * b.abs().max(1.0), "{a} != {b}");
}

#[test]
fn magnification_zero_integer_and_fractional_minification() {
    for (footprint, expected) in [(0., 0.), (0.25, 0.), (1., 0.), (2., 1.), (2_f64.sqrt(), 0.5), (8., 3.)] {
        let result = select_isotropic_lod([256, 256], [footprint / 256., 0.], [0., footprint / 256.]).unwrap();
        close(result.footprint_texels, footprint);
        close(result.lod, expected);
        assert_eq!(result.max_level, 8);
    }
}

#[test]
fn rectangular_textures_scale_each_uv_axis_separately() {
    let result = select_isotropic_lod([512, 128], [1. / 512., 0.], [0., 1. / 128.]).unwrap();
    close(result.footprint_texels, 1.);
    close(result.lod, 0.);
    let result = select_isotropic_lod([512, 128], [0., 4. / 128.], [2. / 512., 0.]).unwrap();
    close(result.footprint_texels, 4.);
    close(result.lod, 2.);
}

#[test]
fn supplied_derivatives_follow_the_actual_supersampling_grid() {
    // A final-image pixel covers4 texels. On a2x4 shading grid its per-step
    // footprints are2 and1; final downsampling remains a separate operation.
    let dx = [4. / 256., 0.];
    let dy = [0., 4. / 256.];
    let coarse = select_isotropic_lod([256, 256], dx, dy).unwrap();
    let fine = select_isotropic_lod([256, 256], dx.map(|v| v / 2.), dy.map(|v| v / 4.)).unwrap();
    close(coarse.lod, 2.);
    close(fine.footprint_texels, 2.);
    close(fine.lod, 1.);
}

#[test]
fn shear_uses_singular_value_rather_than_smaller_max_column_norm() {
    // J columns both [2,0]: max column length2, true largest singular value sqrt8.
    let result = select_isotropic_lod([256, 256], [2. / 256., 0.], [2. / 256., 0.]).unwrap();
    close(result.footprint_texels, 8_f64.sqrt());
    close(result.lod, 1.5);
}

#[test]
fn rotation_sign_and_screen_axis_swap_preserve_the_footprint() {
    let angle = 0.37_f64;
    let (s, c) = angle.sin_cos();
    // Rotated texel-space axes with singular values4 and2, rectangular base.
    let x = [4. * c / 512., 4. * s / 128.];
    let y = [-2. * s / 512., 2. * c / 128.];
    for (dx, dy) in [(x, y), (y, x), (x.map(|v| -v), y), (x, y.map(|v| -v))] {
        let result = select_isotropic_lod([512, 128], dx, dy).unwrap();
        close(result.footprint_texels, 4.);
        close(result.lod, 2.);
    }
}

#[test]
fn full_chain_saturation_is_explicit_for_odd_single_axis_and_single_texel_images() {
    for (dimensions, last) in [([5, 3], 2), ([7, 1], 2), ([1, 7], 2), ([1, 1], 0), ([2048, 2048], 11)] {
        let result = select_isotropic_lod(dimensions, [MAX_UV_GRADIENT; 2], [MAX_UV_GRADIENT; 2]).unwrap();
        assert_eq!(result.max_level, last);
        close(result.lod, f64::from(last));
        assert!(result.footprint_texels.is_finite());
    }
}

#[test]
fn zero_and_tiny_gradients_are_finite() {
    for value in [0., -0., f64::MIN_POSITIVE, f64::from_bits(1)] {
        let result = select_isotropic_lod([2048, 2048], [value; 2], [value; 2]).unwrap();
        assert!(result.footprint_texels.is_finite());
        close(result.lod, 0.);
    }
}

#[test]
fn hostile_dimensions_and_each_gradient_component_refuse() {
    for dimensions in [[0, 1], [1, 0], [MAX_TEXTURE_EDGE + 1, 1], [u32::MAX; 2]] {
        assert_eq!(select_isotropic_lod(dimensions, [0.; 2], [0.; 2]), Err(LodError::Dimensions));
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, MAX_UV_GRADIENT + 1., -MAX_UV_GRADIENT - 1.] {
        for (x, y) in [([value, 0.], [0., 0.]), ([0., value], [0., 0.]), ([0., 0.], [value, 0.]), ([0., 0.], [0., value])] {
            assert_eq!(select_isotropic_lod([2, 2], x, y), Err(LodError::Gradients));
        }
    }
}
