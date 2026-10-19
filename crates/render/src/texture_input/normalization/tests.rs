use super::*;
use crate::texture_input::TextureSource;
use std::{borrow::Cow, cell::Cell};

fn png(info: png::Info<'static>, pixels: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let encoder = png::Encoder::with_info(&mut bytes, info).unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
    }
    bytes
}
fn info(width: u32, height: u32, color: png::ColorType, depth: png::BitDepth) -> png::Info<'static> {
    let mut info = png::Info::with_size(width, height);
    info.color_type = color;
    info.bit_depth = depth;
    info
}
fn options() -> NormalizationOptions {
    NormalizationOptions { unprofiled_color: UnprofiledColor::DeclareSrgbPrimaries, orientation: OrientationPolicy::Apply, ..Default::default() }
}
fn exif(orientation: u8) -> Vec<u8> {
    // Original minimal little-endian TIFF IFD: one SHORT Orientation entry.
    let mut bytes = b"II\x2a\x00\x08\x00\x00\x00\x01\x00\x12\x01\x03\x00\x01\x00\x00\x00".to_vec();
    bytes.extend_from_slice(&[orientation, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}
fn linear_profile() -> Vec<u8> {
    let mut profile = moxcms::ColorProfile::new_srgb();
    profile.cicp = None;
    let text = moxcms::ProfileText::Localizable(vec![moxcms::LocalizableString::new("en".into(), "US".into(), "Original synthetic fixture".into())]);
    profile.description = Some(text.clone());
    profile.copyright = Some(text);
    let curve = moxcms::ToneReprCurve::Parametric(vec![1.0]);
    profile.red_trc = Some(curve.clone());
    profile.green_trc = Some(curve.clone());
    profile.blue_trc = Some(curve);
    profile.encode().unwrap()
}
fn close(actual: f32, expected: f64) {
    assert!((f64::from(actual) - expected).abs() < 1e-7, "{actual} != {expected}");
}

#[test]
fn sixteen_bit_data_and_alpha_are_retained_without_eight_bit_quantization() {
    let values = [32768_u16, 32769, 65535, 12345];
    let bytes = png(info(1, 1, png::ColorType::Rgba, png::BitDepth::Sixteen), &values.into_iter().flat_map(u16::to_be_bytes).collect::<Vec<_>>());
    assert!(TextureSource::new(bytes.clone(), MapRole::Roughness).is_err(), "legacy utility must retain its refusal");
    let source = NormalizationSource::new(bytes.clone(), MapRole::Roughness, NormalizationOptions::default()).unwrap();
    let result = source.decode(|| false).unwrap();
    for (value, channel) in values.into_iter().zip(result.rgba()) {
        close(*channel, f64::from(value) / 65535.0);
    }
    assert!(result.rgba()[1] > result.rgba()[0]);
    assert_eq!(result.receipt().source_channel_bits, 16);
    assert_eq!(result.receipt().conversion, ColorConversion::RawData);
    assert_eq!(result.receipt().output_encoding, "data");
    assert_eq!(result.receipt().source_sha256, format!("{:x}", Sha256::digest(&bytes)));
    assert_eq!(source.decode(|| false).unwrap().receipt(), result.receipt());
}

#[test]
fn declared_color_is_linearized_once_and_alpha_stays_unassociated() {
    let bytes = png(info(1, 1, png::ColorType::Rgba, png::BitDepth::Eight), &[128, 64, 255, 0]);
    assert!(NormalizationSource::new(bytes.clone(), MapRole::BaseColor, Default::default()).unwrap().decode(|| false).is_err());
    let source = NormalizationSource::new(bytes, MapRole::BaseColor, options()).unwrap();
    let result = source.decode(|| false).unwrap();
    close(result.rgba()[0], 0.21586050011389926);
    close(result.rgba()[1], 0.05126945837404324);
    close(result.rgba()[2], 1.0);
    close(result.rgba()[3], 0.0);
    assert_eq!(result.receipt().conversion, ColorConversion::DeclaredSrgb);
}

#[test]
fn png_gamma_and_srgb_metadata_have_explicit_meaning_and_data_is_not_color_corrected() {
    let mut metadata = info(1, 1, png::ColorType::Grayscale, png::BitDepth::Eight);
    metadata.gama_chunk = Some(png::ScaledFloat::from_scaled(100_000));
    metadata.source_gamma = metadata.gama_chunk;
    let bytes = png(metadata.clone(), &[128]);
    let color = NormalizationSource::new(bytes.clone(), MapRole::BaseColor, options()).unwrap().decode(|| false).unwrap();
    close(color.rgba()[0], 128.0 / 255.0);
    assert_eq!(color.receipt().conversion, ColorConversion::PngGammaSrgbPrimaries { encoding_gamma: 1.0 });
    let data = NormalizationSource::new(bytes, MapRole::Metallic, Default::default()).unwrap().decode(|| false).unwrap();
    close(data.rgba()[0], 128.0 / 255.0);
    assert!(data.receipt().ignored_png_color_metadata);
    metadata.srgb = Some(png::SrgbRenderingIntent::RelativeColorimetric);
    let tagged = NormalizationSource::new(png(metadata, &[128]), MapRole::BaseColor, Default::default()).unwrap().decode(|| false).unwrap();
    close(tagged.rgba()[0], 0.21586050011389926);
    assert_eq!(tagged.receipt().conversion, ColorConversion::PngSrgb);
}

#[test]
fn full_image_icc_conversion_is_nonidentity_and_never_changes_alpha_or_data_semantics() {
    let profile = linear_profile();
    let mut metadata = info(1, 1, png::ColorType::Rgba, png::BitDepth::Eight);
    metadata.icc_profile = Some(Cow::Owned(profile.clone()));
    let bytes = png(metadata, &[64, 128, 192, 80]);
    let source = NormalizationSource::new(bytes.clone(), MapRole::BaseColor, Default::default()).unwrap();
    let result = source.decode(|| false).unwrap();
    for (channel, value) in result.rgba().iter().take(3).zip([64.0, 128.0, 192.0]) {
        assert!((f64::from(*channel) - value / 255.0).abs() < 8.0 / 65535.0);
    }
    close(result.rgba()[3], 80.0 / 255.0);
    assert_eq!(result.receipt().conversion, ColorConversion::IccMatrixTrc { profile_sha256: format!("{:x}", Sha256::digest(&profile)) });
    assert!(
        NormalizationSource::new(bytes, MapRole::Roughness, Default::default())
            .unwrap()
            .decode(|| false)
            .unwrap_err()
            .to_string()
            .contains("data maps")
    );
}

#[test]
fn every_orientation_is_an_exact_pixel_permutation_and_normal_vectors_refuse_rotation() {
    let cases = [
        (1, [3, 2], vec![1, 2, 3, 4, 5, 6]),
        (2, [3, 2], vec![3, 2, 1, 6, 5, 4]),
        (3, [3, 2], vec![6, 5, 4, 3, 2, 1]),
        (4, [3, 2], vec![4, 5, 6, 1, 2, 3]),
        (5, [2, 3], vec![1, 4, 2, 5, 3, 6]),
        (6, [2, 3], vec![4, 1, 5, 2, 6, 3]),
        (7, [2, 3], vec![6, 3, 5, 2, 4, 1]),
        (8, [2, 3], vec![3, 6, 2, 5, 1, 4]),
    ];
    for (orientation, dimensions, expected) in cases {
        let mut metadata = info(3, 2, png::ColorType::Grayscale, png::BitDepth::Eight);
        metadata.exif_metadata = Some(Cow::Owned(exif(orientation)));
        let bytes = png(metadata, &[1, 2, 3, 4, 5, 6]);
        let result = NormalizationSource::new(bytes.clone(), MapRole::Roughness, options()).unwrap().decode(|| false).unwrap();
        assert_eq!(result.receipt().output_dimensions, dimensions);
        assert_eq!(result.receipt().source_exif_orientation, orientation);
        for (pixel, value) in result.rgba().as_chunks::<4>().0.iter().zip(expected) {
            close(pixel[0], f64::from(value) / 255.0);
        }
        if orientation != 1 {
            let normal = NormalizationSource::new(bytes.clone(), MapRole::NormalOpenGl, options()).unwrap().decode(|| false).unwrap_err();
            assert!(normal.to_string().contains("vector/basis"));
            assert!(NormalizationSource::new(bytes, MapRole::Roughness, Default::default()).unwrap().decode(|| false).is_err());
        }
    }
}

#[test]
fn cancellation_after_work_does_not_publish_partial_pixels_and_source_can_be_reused() {
    let bytes = png(info(2, 3, png::ColorType::Grayscale, png::BitDepth::Eight), &[1, 2, 3, 4, 5, 6]);
    let source = NormalizationSource::new(bytes, MapRole::Roughness, Default::default()).unwrap();
    assert!(source.decode(|| true).unwrap_err().to_string().contains("cancelled"));
    for threshold in [4, 6] {
        let calls = Cell::new(0);
        let error = source
            .decode(|| {
                let count = calls.get();
                calls.set(count + 1);
                count >= threshold
            })
            .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
    }
    assert_eq!(source.decode(|| false).unwrap().rgba().len(), 24);
}

#[test]
fn retained_encoded_capacity_is_charged_to_the_working_budget() {
    let mut bytes = png(info(1, 1, png::ColorType::Grayscale, png::BitDepth::Eight), &[42]);
    let baseline = NormalizationSource::new(bytes.clone(), MapRole::Roughness, Default::default()).unwrap().decode(|| false).unwrap();
    let budget = baseline.receipt().estimated_working_bytes;
    bytes.reserve_exact(65536);
    let source =
        NormalizationSource::new(bytes, MapRole::Roughness, NormalizationOptions { max_working_bytes: budget, ..Default::default() }).unwrap();
    assert!(source.decode(|| false).unwrap_err().to_string().contains("working budget"));
    assert_eq!(baseline.receipt().normalized_bytes, 16);
}

#[test]
fn exif_without_an_orientation_uses_the_tiff_default_and_big_endian_is_supported() {
    let mut metadata = info(3, 2, png::ColorType::Grayscale, png::BitDepth::Eight);
    let mut missing = exif(3);
    missing[10..12].copy_from_slice(&0x0100_u16.to_le_bytes()); // ImageWidth, no Orientation.
    metadata.exif_metadata = Some(Cow::Owned(missing));
    let result = NormalizationSource::new(png(metadata.clone(), &[1, 2, 3, 4, 5, 6]), MapRole::Roughness, Default::default())
        .unwrap()
        .decode(|| false)
        .unwrap();
    assert!(result.receipt().exif_metadata_present);
    assert!(!result.receipt().exif_orientation_tag_present);
    assert_eq!(result.receipt().source_exif_orientation, 1);
    let big = b"MM\x00\x2a\x00\x00\x00\x08\x00\x01\x01\x12\x00\x03\x00\x00\x00\x01\x00\x06\x00\x00\x00\x00\x00\x00".to_vec();
    metadata.exif_metadata = Some(Cow::Owned(big));
    let result = NormalizationSource::new(png(metadata, &[1, 2, 3, 4, 5, 6]), MapRole::Roughness, options()).unwrap().decode(|| false).unwrap();
    assert_eq!(result.receipt().output_dimensions, [2, 3]);
    for (pixel, value) in result.rgba().as_chunks::<4>().0.iter().zip([4.0, 1.0, 5.0, 2.0, 6.0, 3.0]) {
        close(pixel[0], value / 255.0);
    }
}

#[test]
fn budgets_dimensions_metadata_and_truncation_refuse_without_silent_repair() {
    let bytes = png(info(1, 1, png::ColorType::Grayscale, png::BitDepth::Eight), &[1]);
    for budget in [0, MAX_WORKING_BYTES + 1] {
        assert!(
            NormalizationSource::new(bytes.clone(), MapRole::Roughness, NormalizationOptions { max_working_bytes: budget, ..Default::default() })
                .is_err()
        );
    }
    assert!(
        NormalizationSource::new(bytes.clone(), MapRole::Roughness, NormalizationOptions { max_working_bytes: 1, ..Default::default() }).is_err()
    );
    assert!(NormalizationSource::new(bytes[..bytes.len() - 1].to_vec(), MapRole::Roughness, Default::default()).is_err());
    let oversized = png(info(2049, 1, png::ColorType::Grayscale, png::BitDepth::Eight), &vec![0; 2049]);
    assert!(NormalizationSource::new(oversized, MapRole::Roughness, Default::default()).unwrap().decode(|| false).is_err());
    let mut metadata = info(1, 1, png::ColorType::Rgb, png::BitDepth::Eight);
    metadata.exif_metadata = Some(Cow::Owned(vec![0, 1, 2]));
    assert!(NormalizationSource::new(png(metadata, &[1, 2, 3]), MapRole::Roughness, options()).unwrap().decode(|| false).is_err());
    let mut metadata = info(1, 1, png::ColorType::Rgb, png::BitDepth::Eight);
    metadata.chrm_chunk = Some(png::SourceChromaticities::new((0.3127, 0.3290), (0.64, 0.33), (0.21, 0.71), (0.15, 0.06)));
    metadata.source_chromaticities = metadata.chrm_chunk;
    assert!(NormalizationSource::new(png(metadata, &[1, 2, 3]), MapRole::BaseColor, options()).unwrap().decode(|| false).is_err());
}

#[test]
fn jpeg_decode_records_explicit_eight_bit_precision_and_linearization() {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 100).write_image(&[128, 128, 128], 1, 1, image::ExtendedColorType::Rgb8).unwrap();
    let result = NormalizationSource::new(bytes, MapRole::BaseColor, options()).unwrap().decode(|| false).unwrap();
    assert_eq!(result.receipt().format, "jpeg");
    assert_eq!(result.receipt().source_channel_bits, 8);
    assert_eq!(result.receipt().output_dimensions, [1, 1]);
    assert!((f64::from(result.rgba()[0]) - 0.21586050011389926).abs() < 0.01, "JPEG is a lossy input codec");
}

#[test]
fn indexed_transparency_expands_without_premultiplying_color() {
    let mut metadata = info(2, 1, png::ColorType::Indexed, png::BitDepth::Eight);
    metadata.palette = Some(Cow::Owned(vec![255, 128, 64, 10, 20, 30]));
    metadata.trns = Some(Cow::Owned(vec![0, 128]));
    let result = NormalizationSource::new(png(metadata, &[0, 1]), MapRole::Roughness, Default::default()).unwrap().decode(|| false).unwrap();
    for (actual, expected) in result.rgba().iter().zip([255.0, 128.0, 64.0, 0.0, 10.0, 20.0, 30.0, 128.0]) {
        close(*actual, expected / 255.0);
    }
}

#[test]
fn out_of_gamut_icc_requires_explicit_preview_clipping_and_records_loss() {
    let mut profile = moxcms::ColorProfile::new_from_slice(&linear_profile()).unwrap();
    profile.red_colorant.x *= 2.0; // Original synthetic stress matrix, not a measured material.
    let mut metadata = info(1, 1, png::ColorType::Rgb, png::BitDepth::Eight);
    metadata.icc_profile = Some(Cow::Owned(profile.encode().unwrap()));
    let bytes = png(metadata, &[255, 0, 0]);
    let error = NormalizationSource::new(bytes.clone(), MapRole::BaseColor, Default::default()).unwrap().decode(|| false).unwrap_err();
    assert!(error.to_string().contains("gamut"), "{error}");
    let result = NormalizationSource::new(bytes, MapRole::BaseColor, NormalizationOptions { gamut: GamutPolicy::Clip, ..Default::default() })
        .unwrap()
        .decode(|| false)
        .unwrap();
    assert!(result.receipt().clamped_rgb_channels > 0);
    assert_eq!(result.receipt().gamut_policy, GamutPolicy::Clip);
    assert!(result.rgba().iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
}

fn with_chunk(kind: [u8; 4], data: &[u8], late: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        let mut writer = encoder.write_header().unwrap();
        if !late {
            writer.write_chunk(png::chunk::ChunkType(kind), data).unwrap();
        }
        writer.write_image_data(&[20, 40, 60]).unwrap();
        if late {
            writer.write_chunk(png::chunk::ChunkType(kind), data).unwrap();
        }
    }
    bytes
}

#[test]
fn malformed_or_late_semantic_chunks_never_fall_back_to_untagged_color() {
    for (kind, data, late) in [
        (*b"iCCP", vec![0, 0, 1, 2, 3], false),
        (*b"sRGB", vec![255], false),
        (*b"gAMA", vec![0], false),
        (*b"eXIf", exif(6), true),
        (*b"gAMA", 100_000_u32.to_be_bytes().to_vec(), true),
    ] {
        let source = NormalizationSource::new(with_chunk(kind, &data, late), MapRole::BaseColor, options()).unwrap();
        assert!(source.decode(|| false).is_err(), "silently accepted {kind:?}");
    }
    // Unknown ancillary metadata is left to the established codec, not blanket-refused.
    assert!(NormalizationSource::new(with_chunk(*b"teSt", &[42], false), MapRole::BaseColor, options()).unwrap().decode(|| false).is_ok());
    let mut bad_crc = with_chunk(*b"gAMA", &100_000_u32.to_be_bytes(), false);
    let index = bad_crc.windows(4).position(|v| v == b"gAMA").unwrap() + 8;
    bad_crc[index] ^= 1;
    assert!(NormalizationSource::new(bad_crc, MapRole::BaseColor, options()).unwrap().decode(|| false).is_err());
}

#[test]
fn duplicate_conflicting_and_excessive_metadata_refuses() {
    let mut bytes = with_chunk(*b"sRGB", &[0], false);
    let srgb = bytes[33..46].to_vec(); // Original fixture's 13-byte sRGB chunk.
    bytes.splice(33..33, srgb);
    assert!(NormalizationSource::new(bytes, MapRole::BaseColor, options()).unwrap().decode(|| false).is_err());
    let mut metadata = info(1, 1, png::ColorType::Rgb, png::BitDepth::Eight);
    metadata.icc_profile = Some(Cow::Owned(linear_profile()));
    let mut bytes = png(metadata, &[20, 40, 60]);
    let srgb = with_chunk(*b"sRGB", &[0], false)[33..46].to_vec();
    bytes.splice(33..33, srgb);
    assert!(NormalizationSource::new(bytes, MapRole::BaseColor, options()).unwrap().decode(|| false).is_err());
    let mut bytes = with_chunk(*b"teSt", &[42], false);
    let chunk = bytes[33..46].to_vec();
    bytes.splice(33..33, chunk.repeat(4096));
    assert!(NormalizationSource::new(bytes, MapRole::BaseColor, options()).unwrap().decode(|| false).unwrap_err().to_string().contains("4096"));
}

#[test]
fn invalid_exif_values_types_and_offsets_refuse() {
    for (offset, value) in [(18, 0), (18, 9), (12, 4), (14, 2), (4, 7)] {
        let mut data = exif(1);
        data[offset] = value;
        let bytes = with_chunk(*b"eXIf", &data, false);
        assert!(NormalizationSource::new(bytes, MapRole::Roughness, options()).unwrap().decode(|| false).is_err());
    }
}

#[test]
fn differing_icc_channel_curves_refuse_instead_of_using_a_same_curve_optimization() {
    let mut profile = moxcms::ColorProfile::new_from_slice(&linear_profile()).unwrap();
    profile.blue_trc = Some(moxcms::ToneReprCurve::Parametric(vec![2.0]));
    let mut metadata = info(1, 1, png::ColorType::Rgb, png::BitDepth::Eight);
    metadata.icc_profile = Some(Cow::Owned(profile.encode().unwrap()));
    let error =
        NormalizationSource::new(png(metadata, &[128, 128, 128]), MapRole::BaseColor, Default::default()).unwrap().decode(|| false).unwrap_err();
    assert!(error.to_string().contains("identical RGB tone curves"));
}

#[test]
fn jpeg_icc_conversion_and_metadata_presence_are_checked_without_fallback() {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 100);
        encoder.set_icc_profile(linear_profile()).unwrap();
        encoder.write_image(&[128, 128, 128], 1, 1, image::ExtendedColorType::Rgb8).unwrap();
    }
    let result = NormalizationSource::new(bytes.clone(), MapRole::BaseColor, Default::default()).unwrap().decode(|| false).unwrap();
    assert!((result.rgba()[0] - 128.0 / 255.0).abs() < 0.01, "linear ICC must differ from sRGB interpretation");
    let position = bytes.windows(12).position(|v| v == b"ICC_PROFILE\0").unwrap();
    let mut malformed = bytes.clone();
    malformed[position + 12] = 0; // Illegal sequence number makes the codec return None.
    assert!(NormalizationSource::new(malformed, MapRole::BaseColor, options()).unwrap().decode(|| false).unwrap_err().to_string().contains("retain"));
    let mut duplicate_exif = bytes.clone();
    let exif = [b"Exif\0\0".as_slice(), &exif(1)].concat();
    let segment = [vec![255, 225], u16::try_from(exif.len() + 2).unwrap().to_be_bytes().to_vec(), exif].concat();
    duplicate_exif.splice(2..2, segment.repeat(2));
    assert!(NormalizationSource::new(duplicate_exif, MapRole::BaseColor, options()).unwrap().decode(|| false).is_err());
    let mut late = bytes;
    let end = late.len() - 2;
    late.splice(end..end, segment);
    assert!(NormalizationSource::new(late, MapRole::BaseColor, options()).unwrap().decode(|| false).unwrap_err().to_string().contains("late"));
}
