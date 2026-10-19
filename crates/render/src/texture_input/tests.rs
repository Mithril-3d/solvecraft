use super::*;

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([64, 128, 192, 255]));
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}
#[test]
fn texture_input_preserves_channels_and_explicit_map_interpretation() {
    for role in [MapRole::BaseColor, MapRole::NormalOpenGl, MapRole::Roughness, MapRole::Metallic, MapRole::AmbientOcclusion] {
        let source = TextureSource::new(png(3, 2), role).unwrap();
        let decoded = source.decode(|| false).unwrap();
        assert_eq!((decoded.info().width, decoded.info().height, decoded.info().rgba_bytes), (3, 2, 24));
        assert_eq!(decoded.info().interpretation, if role == MapRole::BaseColor { Interpretation::Srgb } else { Interpretation::Data });
        assert!(decoded.rgba().as_chunks::<4>().0.iter().all(|p| *p == [64, 128, 192, 255]));
    }
}
#[test]
fn texture_input_rejects_dimensions_formats_and_corrupt_pixels() {
    assert!(TextureSource::new(png(MAX_TEXTURE_EDGE + 1, 1), MapRole::BaseColor).is_err());
    assert!(TextureSource::new(Vec::new(), MapRole::BaseColor).is_err());
    assert!(TextureSource::new(b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 2\nabcdefgh".to_vec(), MapRole::BaseColor).is_err());
    let bytes = png(3, 2);
    for n in 0..bytes.len() {
        if let Ok(source) = TextureSource::new(bytes.get(..n).unwrap().to_vec(), MapRole::BaseColor) {
            assert!(source.decode(|| false).is_err(), "truncated PNG accepted at {n}");
        }
    }
}
#[test]
fn texture_input_cancellation_never_publishes_partial_pixels() {
    let source = TextureSource::new(png(3, 2), MapRole::NormalOpenGl).unwrap();
    assert!(source.decode(|| true).is_err());
    let calls = std::cell::Cell::new(0);
    assert!(
        source
            .decode(|| {
                calls.set(calls.get() + 1);
                calls.get() == 2
            })
            .is_err()
    );
    assert!(source.clone().decode(|| false).is_ok());
}
#[test]
fn texture_input_jpeg_is_supported_without_hdr_feature_confusion() {
    let image = image::RgbImage::from_pixel(2, 2, image::Rgb([64, 128, 192]));
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, ImageFormat::Jpeg).unwrap();
    let source = TextureSource::new(out.into_inner(), MapRole::BaseColor).unwrap();
    assert_eq!(source.decode(|| false).unwrap().rgba().len(), 16);
}

#[test]
fn texture_input_caps_encoded_storage_and_refuses_precision_loss() {
    let mut bytes = Vec::with_capacity(MAX_TEXTURE_BYTES + 1);
    bytes.extend(png(1, 1));
    assert!(TextureSource::new(bytes, MapRole::Roughness).is_err());
    assert!(TextureSource::new(vec![0; MAX_TEXTURE_BYTES + 1], MapRole::BaseColor).is_err());
    let image = image::DynamicImage::ImageLuma16(image::ImageBuffer::from_pixel(1, 1, image::Luma([32768u16])));
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, ImageFormat::Png).unwrap();
    assert!(TextureSource::new(out.into_inner(), MapRole::NormalOpenGl).is_err());
}
