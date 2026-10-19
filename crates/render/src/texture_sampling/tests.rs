use super::*;

const PIXELS: [f32; 16] = [1., 0., 0., 0., 0., 1., 0., 1., 0., 0., 1., 1., 1., 1., 1., 1.];

fn close(actual: [f64; 3], expected: [f64; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 1e-9, "{actual:?} != {expected:?}");
    }
}

#[test]
fn centers_orientation_and_explicit_opaque_alpha() {
    let view = TextureView::new(&PIXELS, [2, 2]).unwrap();
    for (uv, rgb) in [([0.25, 0.25], [1., 0., 0.]), ([0.75, 0.25], [0., 1., 0.]), ([0.25, 0.75], [0., 0., 1.]), ([0.75, 0.75], [1., 1., 1.])] {
        close(view.sample_repeat_opaque(uv).unwrap(), rgb);
    }
    close(view.sample_repeat_opaque([0.5, 0.5]).unwrap(), [0.5; 3]);
    close(view.sample_repeat_opaque([0.5, 0.25]).unwrap(), [0.5, 0.5, 0.]);
    close(view.sample_repeat_opaque([0.375, 0.625]).unwrap(), [0.375, 0.25, 0.75]);
}

#[test]
fn linear_black_white_mixing_is_not_encoded_srgb_averaging() {
    let pixels = [0., 0., 0., 1., 1., 1., 1., 1.];
    let view = TextureView::new(&pixels, [2, 1]).unwrap();
    // Linear 0.5, which would encode to about sRGB 0.735, not encoded 0.5.
    close(view.sample_repeat_opaque([0.5, 0.5]).unwrap(), [0.5; 3]);
}

#[test]
fn signed_repeats_and_both_periodic_seams() {
    let view = TextureView::new(&PIXELS, [2, 2]).unwrap();
    for uv in [[0., 0.], [1., 1.], [-1., -1.], [MAX_REPEAT_COORDINATE, -MAX_REPEAT_COORDINATE]] {
        close(view.sample_repeat_opaque(uv).unwrap(), [0.5; 3]);
    }
    close(view.sample_repeat_opaque([-0.75, -0.25]).unwrap(), [0., 0., 1.]);
    for uv in [[0.13, 0.92], [0., 0.25], [0.75, 0.]] {
        close(view.sample_repeat_opaque(uv).unwrap(), view.sample_repeat_opaque([uv[0] - 7., uv[1] + 9.]).unwrap());
    }
    for epsilon in [-1e-12, 0., 1e-12] {
        close(view.sample_repeat_opaque([epsilon, 0.25]).unwrap(), [0.5, 0.5, 0.]);
        close(view.sample_repeat_opaque([0.25, epsilon]).unwrap(), [0.5, 0., 0.5]);
    }
}

#[test]
fn constant_and_single_texel_invariance() {
    for dimensions in [[1, 1], [1, 3], [3, 1], [3, 5]] {
        let pixels = [0.25, 0.5, 0.75, 0.].repeat((dimensions[0] * dimensions[1]) as usize);
        let view = TextureView::new(&pixels, dimensions).unwrap();
        for uv in [[0., 0.], [-12.125, 3.25], [1., 1.], [MAX_REPEAT_COORDINATE - 0.125, 0.5]] {
            close(view.sample_repeat_opaque(uv).unwrap(), [0.25, 0.5, 0.75]);
        }
    }
}

#[test]
fn rejects_hostile_storage_and_channels() {
    for dimensions in [[0, 1], [1, 0], [MAX_TEXTURE_EDGE + 1, 1], [u32::MAX, u32::MAX]] {
        assert_eq!(TextureView::new(&[], dimensions).unwrap_err(), SamplingError::Dimensions);
    }
    for pixels in [&[][..], &[0.; 3][..], &[0.; 5][..]] {
        assert_eq!(TextureView::new(pixels, [1, 1]).unwrap_err(), SamplingError::Length);
    }
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.01] {
        for channel in 0..4 {
            let mut pixels = [0.; 4];
            pixels[channel] = invalid;
            assert_eq!(TextureView::new(&pixels, [1, 1]).unwrap_err(), SamplingError::Values);
        }
    }
}

#[test]
fn rejects_nonfinite_and_extreme_coordinates() {
    let view = TextureView::new(&PIXELS, [2, 2]).unwrap();
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, MAX_REPEAT_COORDINATE + 0.001, -MAX_REPEAT_COORDINATE - 0.001] {
        for uv in [[invalid, 0.], [0., invalid]] {
            assert_eq!(view.sample_repeat_opaque(uv), Err(SamplingError::Coordinates));
        }
    }
}

#[test]
fn reference_bound_retains_fractional_centers_and_maximum_edge() {
    let view = TextureView::new(&PIXELS, [2, 2]).unwrap();
    close(view.sample_repeat_opaque([MAX_REPEAT_COORDINATE - 0.75, -MAX_REPEAT_COORDINATE + 0.25]).unwrap(), [1., 0., 0.]);
    let mut pixels = [0., 0., 0., 1.].repeat(MAX_TEXTURE_EDGE as usize);
    pixels[0] = 1.;
    let view = TextureView::new(&pixels, [MAX_TEXTURE_EDGE, 1]).unwrap();
    let half_texel = 0.5 / f64::from(MAX_TEXTURE_EDGE);
    close(view.sample_repeat_opaque([half_texel, 0.]).unwrap(), [1., 0., 0.]);
    close(view.sample_repeat_opaque([half_texel - MAX_REPEAT_COORDINATE, 0.]).unwrap(), [1., 0., 0.]);
}
