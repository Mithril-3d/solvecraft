use super::*;
use std::cell::Cell;

fn close(actual: [f64; 3], expected: [f64; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 1e-7, "{actual:?} != {expected:?}");
    }
}

#[test]
fn linear_checkerboard_mean_and_fractional_lod() {
    let pixels = [0., 0., 0., 0., 1., 1., 1., 1., 1., 1., 1., 1., 0., 0., 0., 0.];
    let base = TextureView::new(&pixels, [2, 2]).unwrap();
    let chain = MipChain::prepare(base, 16, || false).unwrap();
    assert_eq!(chain.level_count(), 2);
    assert_eq!(chain.tail_bytes(), 16);
    assert_eq!(chain.retained_bytes(), chain.tail_bytes() + chain.tail.capacity() * size_of::<Level>() + size_of::<MipChain<'_>>());
    assert_eq!(MIP_RECIPE_ID, "solidcraft.opaque-linear-area-mips/1");
    close(chain.sample_repeat_lod_opaque([0.25, 0.25], 0.).unwrap(), [0.; 3]);
    close(chain.sample_repeat_lod_opaque([0.25, 0.25], 1.).unwrap(), [0.5; 3]);
    close(chain.sample_repeat_lod_opaque([0.25, 0.25], 0.25).unwrap(), [0.125; 3]);
    close(chain.sample_repeat_lod_opaque([0.25, 0.25], 0.5).unwrap(), [0.25; 3]);
}

#[test]
fn odd_edges_contribute_by_area_instead_of_being_dropped_or_overweighted() {
    // Five pixels reduce to two equal-width cells: [0,0,0] and [0,0,1].
    // The middle pixel straddles the boundary, the last pixel weighs 1/2.5.
    let mut pixels = [0., 0., 0., 1.].repeat(5);
    pixels[16] = 1.;
    let chain = MipChain::prepare(TextureView::new(&pixels, [5, 1]).unwrap(), 48, || false).unwrap();
    assert_eq!(chain.dimensions(1), Some([2, 1]));
    close(chain.level(1).unwrap().sample_repeat_opaque([0.75, 0.5]).unwrap(), [0.4, 0., 0.]);
    close(chain.sample_repeat_lod_opaque([0., 0.], 2.).unwrap(), [0.2, 0., 0.]);
}

#[test]
fn rectangular_odd_pyramid_preserves_independent_mean() {
    let mut pixels = Vec::new();
    for i in 0..15 {
        pixels.extend([i as f32 / 16., 0.25, (14 - i) as f32 / 16., (i % 2) as f32]);
    }
    let chain = MipChain::prepare(TextureView::new(&pixels, [5, 3]).unwrap(), 48, || false).unwrap();
    assert_eq!(chain.level_count(), 3);
    assert_eq!(chain.dimensions(0), Some([5, 3]));
    assert_eq!(chain.dimensions(1), Some([2, 1]));
    assert_eq!(chain.dimensions(2), Some([1, 1]));
    assert_eq!(chain.dimensions(usize::MAX), None);
    close(chain.sample_repeat_lod_opaque([-9.5, 12.], 2.).unwrap(), [7. / 16., 0.25, 7. / 16.]);
}

#[test]
fn single_texel_borrows_base_and_needs_no_pixel_budget() {
    let pixels = [0.25, 0.5, 0.75, 0.];
    let chain = MipChain::prepare(TextureView::new(&pixels, [1, 1]).unwrap(), 0, || false).unwrap();
    assert_eq!(chain.level_count(), 1);
    assert_eq!(chain.tail_bytes(), 0);
    assert!(std::ptr::eq(chain.level(0).unwrap().rgba.as_ptr(), pixels.as_ptr()));
    close(chain.sample_repeat_lod_opaque([-0.1, 0.7], 0.).unwrap(), [0.25, 0.5, 0.75]);
}

#[test]
fn constant_images_and_single_axes_are_invariant_at_all_lods() {
    for dimensions in [[1, 7], [7, 1], [7, 5]] {
        let pixels = [0.25, 0.5, 0.75, 0.].repeat((dimensions[0] * dimensions[1]) as usize);
        let chain = MipChain::prepare(TextureView::new(&pixels, dimensions).unwrap(), MAX_MIP_TAIL_BYTES, || false).unwrap();
        for quarter in 0..=(chain.level_count() - 1) * 4 {
            close(chain.sample_repeat_lod_opaque([-2.34, 1.92], quarter as f64 / 4.).unwrap(), [0.25, 0.5, 0.75]);
        }
    }
}

#[test]
fn hostile_lod_and_uv_refuse_without_changing_the_chain() {
    let pixels = [0., 0., 0., 1.].repeat(4);
    let chain = MipChain::prepare(TextureView::new(&pixels, [2, 2]).unwrap(), 16, || false).unwrap();
    for lod in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.001, 1.001, f64::MAX] {
        assert_eq!(chain.sample_repeat_lod_opaque([0., 0.], lod), Err(MipError::Lod));
    }
    assert_eq!(chain.sample_repeat_lod_opaque([f64::NAN, 0.], 0.5), Err(MipError::Sampling(SamplingError::Coordinates)));
    close(chain.sample_repeat_lod_opaque([0.25, 0.25], 0.).unwrap(), [0.; 3]);
}

#[test]
fn exact_budget_and_global_cap_are_admitted_before_work() {
    let pixels = [0., 0., 0., 1.].repeat(4);
    let base = TextureView::new(&pixels, [2, 2]).unwrap();
    assert!(matches!(MipChain::prepare(base, 15, || false), Err(MipError::Budget)));
    assert!(MipChain::prepare(base, 16, || false).is_ok());
    assert!(matches!(MipChain::prepare(base, MAX_MIP_TAIL_BYTES + 1, || false), Err(MipError::Budget)));
    assert!(matches!(MipChain::prepare(base, usize::MAX, || false), Err(MipError::Budget)));
    assert_eq!(required_bytes(2048, 2048).unwrap(), 22_369_616);
    assert!(required_bytes(2048, 2048).unwrap() < MAX_MIP_TAIL_BYTES);
}

#[test]
fn cancellation_before_during_and_after_preparation_never_returns_a_partial_chain() {
    let pixels = [0.25, 0.5, 0.75, 0.].repeat(64);
    let base = TextureView::new(&pixels, [8, 8]).unwrap();
    let successful_calls = Cell::new(0);
    MipChain::prepare(base, MAX_MIP_TAIL_BYTES, || {
        successful_calls.set(successful_calls.get() + 1);
        false
    })
    .unwrap();
    for stop in 1..=successful_calls.get() {
        let calls = Cell::new(0);
        let result = MipChain::prepare(base, MAX_MIP_TAIL_BYTES, || {
            calls.set(calls.get() + 1);
            calls.get() == stop
        });
        assert!(matches!(result, Err(MipError::Cancelled)));
    }
    assert!(MipChain::prepare(base, MAX_MIP_TAIL_BYTES, || false).is_ok());
}
