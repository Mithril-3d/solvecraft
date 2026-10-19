use super::{ColorConversion, UnprofiledColor, digest_bytes, icc, zeroed};
use crate::texture_input::{MAX_TEXTURE_EDGE, Result, invalid};
use image::{ImageDecoder, ImageFormat, Limits, metadata::Orientation};
use std::io::Cursor;

mod jpeg_markers;
mod png_markers;

pub(super) const CODEC_BYTES: usize = 64 * 1024 * 1024;
const CMS_WORKSPACE_ESTIMATE: usize = 16 * 1024 * 1024;
const MAX_EXIF_BYTES: usize = 64 * 1024;
const SRGB_CHROMATICITIES: [u32; 8] = [31270, 32900, 64000, 33000, 30000, 60000, 15000, 6000];

#[derive(PartialEq)]
struct Metadata {
    icc: Option<Vec<u8>>,
    orientation: Orientation,
    exif_present: bool,
    orientation_tag_present: bool,
    srgb: bool,
    gamma: Option<u32>,
    chromaticities: Option<[u32; 8]>,
    other_color: bool,
}
pub(super) struct RawImage {
    pub dimensions: [u32; 2],
    pub source_bits: u8,
    pub orientation: Orientation,
    pub exif_present: bool,
    pub orientation_tag_present: bool,
    pub icc: Option<Vec<u8>>,
    pub estimated_working_bytes: usize,
    bits: u8,
    channels: usize,
    pixels: Vec<u8>,
    srgb: bool,
    gamma: Option<u32>,
    chromaticities: Option<[u32; 8]>,
    other_color: bool,
}
impl RawImage {
    pub fn has_png_color_metadata(&self) -> bool {
        self.srgb || self.gamma.is_some() || self.chromaticities.is_some() || self.other_color
    }
    pub fn conversion(&self, role: crate::texture_input::MapRole, policy: UnprofiledColor) -> Result<ColorConversion> {
        if role != crate::texture_input::MapRole::BaseColor {
            if self.icc.is_some() {
                return Err(invalid("ICC profiles on data maps require an explicit separate interpretation policy"));
            }
            return Ok(ColorConversion::RawData);
        }
        if self.other_color {
            return Err(invalid("PNG CICP/HDR metadata is outside initial SDR texture normalization"));
        }
        if let Some(profile) = &self.icc {
            if self.srgb {
                return Err(invalid("PNG has conflicting ICC and sRGB declarations"));
            }
            if self.channels < 3 {
                return Err(invalid("ICC conversion initially requires RGB/RGBA source samples"));
            }
            icc::admit(profile)?;
            return Ok(ColorConversion::IccMatrixTrc { profile_sha256: digest_bytes(profile, &|| false)? });
        }
        if self.srgb {
            return Ok(ColorConversion::PngSrgb);
        }
        if policy == UnprofiledColor::Refuse {
            return Err(invalid("unprofiled color requires an explicit sRGB-primary declaration"));
        }
        if self.chromaticities.is_some_and(|values| values.into_iter().zip(SRGB_CHROMATICITIES).any(|(a, b)| a.abs_diff(b) > 1)) {
            return Err(invalid("PNG chromaticities differ from the declared sRGB primaries"));
        }
        if let Some(gamma) = self.gamma {
            let encoding_gamma = f64::from(gamma) / 100_000.0;
            if !(0.1..=10.0).contains(&encoding_gamma) {
                return Err(invalid("PNG encoding gamma is outside [0.1, 10]"));
            }
            return Ok(ColorConversion::PngGammaSrgbPrimaries { encoding_gamma });
        }
        Ok(ColorConversion::DeclaredSrgb)
    }
    pub fn pixel(&self, x: u32, y: u32) -> Result<[f64; 4]> {
        let [width, height] = self.dimensions;
        if x >= width || y >= height {
            return Err(invalid("pixel lies outside decoded image"));
        }
        let pixel = usize::try_from(u64::from(y) * u64::from(width) + u64::from(x)).map_err(|_| invalid("pixel offset overflow"))?;
        let base = pixel.checked_mul(self.channels).ok_or_else(|| invalid("sample offset overflow"))?;
        let sample = |channel: usize| -> Result<f64> {
            let index = base.checked_add(channel).ok_or_else(|| invalid("sample offset overflow"))?;
            if self.bits == 16 {
                let start = index.checked_mul(2).ok_or_else(|| invalid("16-bit sample offset overflow"))?;
                let end = start.checked_add(2).ok_or_else(|| invalid("16-bit sample offset overflow"))?;
                let bytes: [u8; 2] = self.pixels.get(start..end).and_then(|v| v.try_into().ok()).ok_or_else(|| invalid("truncated 16-bit pixel"))?;
                Ok(f64::from(u16::from_be_bytes(bytes)) / 65535.0)
            } else {
                self.pixels.get(index).map(|v| f64::from(*v) / 255.0).ok_or_else(|| invalid("truncated 8-bit pixel"))
            }
        };
        match self.channels {
            1 => {
                let value = sample(0)?;
                Ok([value, value, value, 1.0])
            }
            2 => {
                let value = sample(0)?;
                Ok([value, value, value, sample(1)?])
            }
            3 => Ok([sample(0)?, sample(1)?, sample(2)?, 1.0]),
            4 => Ok([sample(0)?, sample(1)?, sample(2)?, sample(3)?]),
            _ => Err(invalid("unsupported decoded channel layout")),
        }
    }
}

fn dimensions(width: u32, height: u32, encoded: usize, budget: usize) -> Result<usize> {
    if width == 0 || height == 0 || width > MAX_TEXTURE_EDGE || height > MAX_TEXTURE_EDGE {
        return Err(invalid("normalization dimensions exceed 2048x2048"));
    }
    let pixels = usize::try_from(u64::from(width) * u64::from(height)).map_err(|_| invalid("image size overflow"))?;
    let rows = usize::try_from(u64::from(width) * 3 * 8 * 2).map_err(|_| invalid("row workspace overflow"))?;
    let estimate = pixels
        .checked_mul(8 + 16)
        .and_then(|v| v.checked_add(encoded))
        .and_then(|v| v.checked_add(CODEC_BYTES + CMS_WORKSPACE_ESTIMATE + 2 * icc::MAX_ICC_BYTES + rows))
        .ok_or_else(|| invalid("normalization working estimate overflow"))?;
    if estimate > budget {
        return Err(invalid("normalization exceeds its conservative working budget"));
    }
    Ok(estimate)
}
fn exif_u16(bytes: &[u8], offset: usize, little: bool) -> Result<u16> {
    let end = offset.checked_add(2).ok_or_else(|| invalid("EXIF offset overflow"))?;
    let data: [u8; 2] = bytes.get(offset..end).and_then(|v| v.try_into().ok()).ok_or_else(|| invalid("truncated EXIF SHORT"))?;
    Ok(if little { u16::from_le_bytes(data) } else { u16::from_be_bytes(data) })
}
fn exif_u32(bytes: &[u8], offset: usize, little: bool) -> Result<u32> {
    let end = offset.checked_add(4).ok_or_else(|| invalid("EXIF offset overflow"))?;
    let data: [u8; 4] = bytes.get(offset..end).and_then(|v| v.try_into().ok()).ok_or_else(|| invalid("truncated EXIF LONG"))?;
    Ok(if little { u32::from_le_bytes(data) } else { u32::from_be_bytes(data) })
}
/// Read only classic TIFF IFD0's Orientation. Other EXIF values/thumbnail IFDs are
/// not decoded or certified. TIFF's default for a missing orientation is top-left.
fn orientation(exif: Option<&[u8]>) -> Result<(Orientation, bool)> {
    let Some(bytes) = exif else {
        return Ok((Orientation::NoTransforms, false));
    };
    if bytes.len() > MAX_EXIF_BYTES {
        return Err(invalid("EXIF metadata exceeds 64 KiB"));
    }
    let little = match bytes.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err(invalid("invalid EXIF byte order")),
    };
    if exif_u16(bytes, 2, little)? != 42 {
        return Err(invalid("EXIF needs a classic TIFF header"));
    }
    let start = usize::try_from(exif_u32(bytes, 4, little)?).map_err(|_| invalid("EXIF IFD offset overflow"))?;
    if start < 8 || start % 2 != 0 {
        return Err(invalid("invalid EXIF IFD0 offset"));
    }
    let count = usize::from(exif_u16(bytes, start, little)?);
    if count == 0 || count > 128 {
        return Err(invalid("EXIF IFD0 needs 1..128 entries"));
    }
    let begin = start.checked_add(2).ok_or_else(|| invalid("EXIF IFD offset overflow"))?;
    let end = begin.checked_add(count * 12).ok_or_else(|| invalid("EXIF IFD size overflow"))?;
    let entries = bytes.get(begin..end).ok_or_else(|| invalid("truncated EXIF IFD0"))?;
    let _next_ifd = exif_u32(bytes, end, little)?;
    let mut result = None;
    for entry in entries.as_chunks::<12>().0 {
        if exif_u16(entry, 0, little)? != 0x0112 {
            continue;
        }
        if result.is_some() || exif_u16(entry, 2, little)? != 3 || exif_u32(entry, 4, little)? != 1 {
            return Err(invalid("EXIF Orientation must be one unique SHORT"));
        }
        let value = u8::try_from(exif_u16(entry, 8, little)?).map_err(|_| invalid("invalid EXIF orientation"))?;
        result = Some(Orientation::from_exif(value).ok_or_else(|| invalid("EXIF orientation must be in 1..8"))?);
    }
    Ok((result.unwrap_or(Orientation::NoTransforms), result.is_some()))
}
fn png_metadata(info: &png::Info<'_>) -> Result<Metadata> {
    if info.animation_control.is_some() || info.frame_control.is_some() {
        return Err(invalid("animated PNG textures are unsupported"));
    }
    let icc = match info.icc_profile.as_ref() {
        Some(profile) if profile.len() <= icc::MAX_ICC_BYTES => {
            let mut copy = zeroed::<u8>(profile.len())?;
            copy.copy_from_slice(profile);
            Some(copy)
        }
        Some(_) => return Err(invalid("embedded ICC exceeds 256 KiB")),
        None => None,
    };
    let chromaticities = info.chrm_chunk.map(|chrm| {
        [
            chrm.white.0.into_scaled(),
            chrm.white.1.into_scaled(),
            chrm.red.0.into_scaled(),
            chrm.red.1.into_scaled(),
            chrm.green.0.into_scaled(),
            chrm.green.1.into_scaled(),
            chrm.blue.0.into_scaled(),
            chrm.blue.1.into_scaled(),
        ]
    });
    let (orientation, orientation_tag_present) = orientation(info.exif_metadata.as_deref())?;
    Ok(Metadata {
        icc,
        orientation,
        exif_present: info.exif_metadata.is_some(),
        orientation_tag_present,
        srgb: info.srgb.is_some(),
        gamma: info.gama_chunk.map(|v| v.into_scaled()),
        chromaticities,
        other_color: info.coding_independent_code_points.is_some()
            || info.mastering_display_color_volume.is_some()
            || info.content_light_level.is_some(),
    })
}
pub(super) fn image(bytes: &[u8], format: ImageFormat, budget: usize, encoded_capacity: usize) -> Result<RawImage> {
    match format {
        ImageFormat::Png => png_image(bytes, budget, encoded_capacity),
        ImageFormat::Jpeg => jpeg_image(bytes, budget, encoded_capacity),
        _ => Err(invalid("normalization accepts PNG or JPEG only")),
    }
}
fn png_image(bytes: &[u8], budget: usize, encoded_capacity: usize) -> Result<RawImage> {
    let markers = png_markers::read(bytes)?;
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png::Limits { bytes: CODEC_BYTES });
    decoder.set_ignore_text_chunk(true);
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| invalid(e.to_string()))?;
    let info = reader.info();
    markers.check(info)?;
    let size = [info.width, info.height];
    let estimate = dimensions(info.width, info.height, encoded_capacity, budget)?;
    let source_bits = info.bit_depth as u8;
    let before = png_metadata(info)?;
    let length = reader.output_buffer_size().ok_or_else(|| invalid("PNG decoded storage overflow"))?;
    if length > 32 * 1024 * 1024 {
        return Err(invalid("PNG decoded samples exceed 32 MiB"));
    }
    let mut pixels = zeroed::<u8>(length)?;
    let output = reader.next_frame(&mut pixels).map_err(|e| invalid(e.to_string()))?;
    reader.finish().map_err(|e| invalid(e.to_string()))?;
    let after = png_metadata(reader.info())?;
    if before != after {
        return Err(invalid("late PNG color/orientation metadata requires explicit separate handling"));
    }
    if [output.width, output.height] != size || output.buffer_size() != pixels.len() {
        return Err(invalid("PNG output dimensions changed"));
    }
    let bits = output.bit_depth as u8;
    if !matches!(bits, 8 | 16) || output.color_type == png::ColorType::Indexed {
        return Err(invalid("PNG expansion did not yield 8/16-bit samples"));
    }
    let Metadata { icc, orientation, exif_present, orientation_tag_present, srgb, gamma, chromaticities, other_color } = after;
    Ok(RawImage {
        dimensions: size,
        source_bits,
        orientation,
        exif_present,
        orientation_tag_present,
        icc,
        estimated_working_bytes: estimate,
        bits,
        channels: output.color_type.samples(),
        pixels,
        srgb,
        gamma,
        chromaticities,
        other_color,
    })
}
fn jpeg_image(bytes: &[u8], budget: usize, encoded_capacity: usize) -> Result<RawImage> {
    let markers = jpeg_markers::read(bytes)?;
    let mut decoder = image::codecs::jpeg::JpegDecoder::new(Cursor::new(bytes)).map_err(|e| invalid(e.to_string()))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_TEXTURE_EDGE);
    limits.max_image_height = Some(MAX_TEXTURE_EDGE);
    limits.max_alloc = Some(CODEC_BYTES as u64);
    decoder.set_limits(limits).map_err(|e| invalid(e.to_string()))?;
    let (width, height) = decoder.dimensions();
    let estimate = dimensions(width, height, encoded_capacity, budget)?;
    let channels = match decoder.color_type() {
        image::ColorType::L8 => 1,
        image::ColorType::Rgb8 => 3,
        _ => return Err(invalid("normalization initially accepts only 8-bit JPEG channels")),
    };
    let icc = decoder.icc_profile().map_err(|e| invalid(e.to_string()))?;
    if icc.as_ref().is_some_and(|p| p.len() > icc::MAX_ICC_BYTES || p.capacity() > icc::MAX_ICC_BYTES) {
        return Err(invalid("embedded ICC exceeds 256 KiB"));
    }
    let exif = decoder.exif_metadata().map_err(|e| invalid(e.to_string()))?;
    if markers != (icc.is_some(), exif.is_some()) {
        return Err(invalid("JPEG decoder did not retain declared ICC/EXIF metadata"));
    }
    let (orientation, orientation_tag_present) = orientation(exif.as_deref())?;
    let length = usize::try_from(decoder.total_bytes()).map_err(|_| invalid("JPEG decoded storage overflow"))?;
    if length > 32 * 1024 * 1024 {
        return Err(invalid("JPEG decoded samples exceed 32 MiB"));
    }
    let mut pixels = zeroed::<u8>(length)?;
    decoder.read_image(&mut pixels).map_err(|e| invalid(e.to_string()))?;
    Ok(RawImage {
        dimensions: [width, height],
        source_bits: 8,
        orientation,
        exif_present: exif.is_some(),
        orientation_tag_present,
        icc,
        estimated_working_bytes: estimate,
        bits: 8,
        channels,
        pixels,
        srgb: false,
        gamma: None,
        chromaticities: None,
        other_color: false,
    })
}
