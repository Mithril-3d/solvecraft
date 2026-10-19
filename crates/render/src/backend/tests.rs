use super::*;
use serde_json::json;

fn request() -> RenderRequest {
    RenderRequest {
        backend: None,
        mode: RenderMode::Preview,
        operation: RenderOperation::Image,
        width: 640,
        height: 480,
        format: Some(RenderFormat::Png),
        features: Vec::new(),
        policy: RenderPolicy::Strict,
    }
}

#[test]
fn builtin_resolves_only_the_existing_raster_snapshot_profile() {
    let backends = [builtin_backend()];
    let r = resolve_request(&backends, &request()).unwrap();
    assert_eq!(r.backend.as_str(), "builtin");
    assert_eq!(r.executor.as_str(), "cpu-raster");
    assert_eq!(r.profile.mode, RenderMode::Preview);
    assert_eq!(r.estimated_scratch_bytes, 640 * 480 * 48);
    assert!(r.warnings.is_empty() && r.profile.features.is_empty());
    for mode in [RenderMode::Quick, RenderMode::Final] {
        assert_eq!(
            resolve_request(&backends, &RenderRequest { mode, ..request() }),
            Err(ResolveError::UnsupportedProfile { mode, operation: RenderOperation::Image })
        );
    }
    assert!(matches!(
        resolve_request(&backends, &RenderRequest { operation: RenderOperation::Viewport, format: None, ..request() }),
        Err(ResolveError::UnsupportedProfile { .. })
    ));
    for format in [None, Some(RenderFormat::Exr)] {
        assert_eq!(resolve_request(&backends, &RenderRequest { format, ..request() }), Err(ResolveError::UnsupportedFormat));
    }
    for feature in [RenderFeature::Hdri, RenderFeature::Pbr, RenderFeature::Refraction, RenderFeature::Progressive] {
        assert_eq!(
            resolve_request(&backends, &RenderRequest { features: vec![feature], ..request() }),
            Err(ResolveError::UnsupportedFeature(feature))
        );
    }
}

#[test]
fn registry_order_is_not_backend_or_executor_priority() {
    let mut second = builtin_backend();
    second.id = BackendId::new("test-engine").unwrap();
    second.executors.first_mut().unwrap().id = ExecutorId::new("z-cpu").unwrap();
    let mut other = second.executors.first().unwrap().clone();
    other.id = ExecutorId::new("a-cpu").unwrap();
    second.executors.push(other);
    let mut backends = vec![builtin_backend(), second];
    let req = RenderRequest { backend: Some(BackendId::new("test-engine").unwrap()), ..request() };
    let expected = resolve_request(&backends, &req).unwrap();
    assert_eq!(expected.executor.as_str(), "a-cpu");
    backends.reverse();
    backends.first_mut().unwrap().executors.reverse();
    assert_eq!(resolve_request(&backends, &req).unwrap(), expected);
    assert_eq!(resolve_request(&backends, &request()).unwrap().backend.as_str(), "builtin");
    // A candidate that cannot satisfy the request must not hide a later capable executor.
    backends.first_mut().unwrap().executors.first_mut().unwrap().limits.max_dimension = 1;
    assert_eq!(resolve_request(&backends, &req).unwrap().executor.as_str(), "z-cpu");
}

#[test]
fn explicit_unknown_or_unavailable_backend_never_falls_back() {
    let builtin = builtin_backend();
    let unknown = BackendId::new("missing").unwrap();
    assert_eq!(
        resolve_request(std::slice::from_ref(&builtin), &RenderRequest { backend: Some(unknown.clone()), ..request() }),
        Err(ResolveError::UnknownBackend(unknown))
    );
    let mut second = builtin.clone();
    second.id = BackendId::new("test-engine").unwrap();
    second.availability = Availability::Unavailable { reason: "device initialization failed".into() };
    let req = RenderRequest { backend: Some(second.id.clone()), ..request() };
    assert_eq!(resolve_request(&[builtin, second], &req), Err(ResolveError::BackendUnavailable("device initialization failed".into())));
    let mut backend = builtin_backend();
    backend.executors.first_mut().unwrap().availability = Availability::Unavailable { reason: "executor disabled".into() };
    assert_eq!(resolve_request(&[backend], &request()), Err(ResolveError::NoAvailableExecutor));
}

#[test]
fn ambiguous_registries_are_rejected_before_selection() {
    let mut b = builtin_backend();
    assert!(matches!(resolve_request(&[b.clone(), b.clone()], &request()), Err(ResolveError::DuplicateBackend(_))));
    b.executors.push(b.executors.first().unwrap().clone());
    assert!(matches!(resolve_request(&[b], &request()), Err(ResolveError::DuplicateExecutor { .. })));
    let mut b = builtin_backend();
    let e = b.executors.first_mut().unwrap();
    e.profiles.push(e.profiles.first().unwrap().clone());
    assert!(matches!(resolve_request(&[b], &request()), Err(ResolveError::InvalidRegistry(_))));
}

#[test]
fn capabilities_are_not_combined_across_execution_paths() {
    let mut b = builtin_backend();
    let e = b.executors.first_mut().unwrap();
    e.id = ExecutorId::new("a-png-with-hdri").unwrap();
    e.profiles.first_mut().unwrap().features = vec![RenderFeature::Hdri];
    let mut second = e.clone();
    second.id = ExecutorId::new("z-exr-without-hdri").unwrap();
    let profile = second.profiles.first_mut().unwrap();
    profile.formats = vec![RenderFormat::Exr];
    profile.features.clear();
    b.executors.push(second);
    let req = RenderRequest { format: Some(RenderFormat::Exr), features: vec![RenderFeature::Hdri], ..request() };
    assert_eq!(resolve_request(std::slice::from_ref(&b), &req), Err(ResolveError::UnsupportedFormat));
    b.executors.reverse();
    assert_eq!(resolve_request(&[b], &req), Err(ResolveError::UnsupportedFormat));
}

#[test]
fn resolved_profiles_are_canonical_and_large_feature_lists_are_rejected() {
    let mut b = builtin_backend();
    let profile = b.executors.first_mut().unwrap().profiles.first_mut().unwrap();
    profile.formats.push(RenderFormat::Exr);
    profile.features = vec![RenderFeature::Pbr, RenderFeature::Hdri];
    let expected = resolve_request(std::slice::from_ref(&b), &request()).unwrap();
    let profile = b.executors.first_mut().unwrap().profiles.first_mut().unwrap();
    profile.formats.reverse();
    profile.features.reverse();
    assert_eq!(resolve_request(&[b], &request()).unwrap(), expected);
    assert!(matches!(
        resolve_request(&[builtin_backend()], &RenderRequest { features: vec![RenderFeature::Hdri; 33], ..request() }),
        Err(ResolveError::InvalidRequest(_))
    ));
}

#[test]
fn dimension_pixel_and_scratch_limits_are_checked_without_allocating_images() {
    for (width, height) in [(0, 480), (640, 0)] {
        assert_eq!(resolve_request(&[builtin_backend()], &RenderRequest { width, height, ..request() }), Err(ResolveError::InvalidDimensions));
    }
    for (width, height, resource) in [(4097, 1, "width"), (1, 4097, "height"), (4096, 4096, "pixels")] {
        assert!(matches!(
            resolve_request(&[builtin_backend()], &RenderRequest { width, height, ..request() }),
            Err(ResolveError::LimitExceeded { resource: r, .. }) if r == resource
        ));
    }
    let mut b = builtin_backend();
    b.executors.first_mut().unwrap().limits.max_scratch_bytes = 640 * 480 * 48 - 1;
    assert!(matches!(resolve_request(&[b], &request()), Err(ResolveError::LimitExceeded { resource: "estimated scratch bytes", .. })));
    // Both policy boundaries are inclusive.
    let r = resolve_request(&[builtin_backend()], &RenderRequest { width: 2048, height: 2048, ..request() }).unwrap();
    assert_eq!(r.estimated_scratch_bytes, 201_326_592);
    assert!(resolve_request(&[builtin_backend()], &RenderRequest { width: 4096, height: 1, ..request() }).is_ok());
}

#[test]
fn checked_arithmetic_covers_pixels_supersampling_buffers_and_sums() {
    let mut b = builtin_backend();
    b.executors.first_mut().unwrap().limits = RenderLimits { max_dimension: u64::MAX, max_pixels: u64::MAX, max_scratch_bytes: u64::MAX };
    assert_eq!(resolve_request(&[b], &RenderRequest { width: u64::MAX, height: 2, ..request() }), Err(ResolveError::SizeOverflow));
    let s = ScratchLayout::CPU_RASTER;
    for (width, height) in [(u64::MAX, 1), (u64::MAX / 2, 2), (u64::MAX / 8, 1)] {
        assert_eq!(s.estimate(width, height), Err(ResolveError::SizeOverflow));
    }
    let sum = ScratchLayout { sample_scale: 1, sample_bytes_per_pixel: u64::MAX - 1, output_bytes_per_pixel: 1, encoding_bytes_per_pixel: 1 };
    assert_eq!(sum.estimate(1, 1), Err(ResolveError::SizeOverflow));
}

#[test]
fn identifiers_and_json_reject_unsafe_or_unsupported_input() {
    for id in ["", "Builtin", "../builtin", "with space", "1cpu", "café", &"a".repeat(MAX_RENDER_ID_BYTES + 1)] {
        assert!(BackendId::new(id).is_err(), "{id}");
        assert!(serde_json::from_value::<ExecutorId>(json!(id)).is_err(), "{id}");
    }
    assert!(BackendId::new(&"a".repeat(MAX_RENDER_ID_BYTES)).is_ok());
    let valid = serde_json::to_value(request()).unwrap();
    assert_eq!(serde_json::from_value::<RenderRequest>(valid.clone()).unwrap(), request());
    for (key, value) in [
        ("width", json!(-1)),
        ("width", json!(1.5)),
        ("height", json!("480")),
        ("mode", json!("photo")),
        ("policy", json!("fallback")),
        ("unexpected", json!(true)),
    ] {
        let mut invalid = valid.clone();
        invalid[key] = value;
        assert!(serde_json::from_value::<RenderRequest>(invalid).is_err(), "{key}");
    }
    for text in ["{", "[]", "null", "{\"width\":NaN}", "{\"width\":1e999}"] {
        assert!(serde_json::from_str::<RenderRequest>(text).is_err());
    }
}

#[test]
fn fit_scales_both_axes_into_dimension_pixel_and_scratch_limits() {
    let limits = RenderLimits::default();
    assert_eq!(limits.fit(1920, 1080, None), Some((1920, 1080)));
    assert_eq!(limits.fit(4096, 4096, None), Some((2048, 2048)));
    assert_eq!(limits.fit(0, 1080, None), None);
    // One long side: the dimension limit binds first.
    assert_eq!(limits.fit(8192, 16, None), Some((4096, 8)));
    for (w, h) in [(3840, 2160), (5120, 2880), (7680, 4320), (2160, 3840)] {
        let (fw, fh) = limits.fit(w, h, None).unwrap();
        assert!(fw <= 4096 && fh <= 4096 && fw * fh <= limits.max_pixels, "{w}x{h} -> {fw}x{fh}");
        // Same aspect (within a pixel) and close to the budget.
        assert!((fw as f64 / fh as f64 - w as f64 / h as f64).abs() < 2.0 / fh.min(fw) as f64, "{fw}x{fh}");
        assert!(fw * fh > limits.max_pixels * 99 / 100, "{fw}x{fh}");
    }
    // A scratch layout tighter than the pixel limit binds instead.
    let scratch = ScratchLayout { sample_scale: 1, sample_bytes_per_pixel: 128, output_bytes_per_pixel: 8, encoding_bytes_per_pixel: 0 };
    let (fw, fh) = limits.fit(3840, 2160, Some(scratch)).unwrap();
    assert!(scratch.estimate(fw, fh).unwrap() <= limits.max_scratch_bytes && fw * fh < limits.max_pixels, "{fw}x{fh}");
}
