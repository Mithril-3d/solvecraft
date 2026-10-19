//! Bounded RGB matrix/TRC profile admission before calling the existing permissive CMS.
//! Header/tag layout follows the public ICC.1 profile specification, not another CAD tool.

use crate::texture_input::{Result, guarded, invalid};
use moxcms::{ColorProfile, DataColorSpace, Layout, ParsingOptions, ProfileClass, RenderingIntent, TransformF64Executor, TransformOptions};
use std::sync::Arc;

pub(super) const MAX_ICC_BYTES: usize = 256 * 1024;
const MAX_TAGS: usize = 64;
const MAX_TAG_BYTES: usize = 64 * 1024;
const MAX_TRC_VALUES: usize = 4096;

fn word(bytes: &[u8], offset: usize) -> Result<usize> {
    let end = offset.checked_add(4).ok_or_else(|| invalid("ICC offset overflow"))?;
    let value: [u8; 4] = bytes.get(offset..end).and_then(|v| v.try_into().ok()).ok_or_else(|| invalid("truncated ICC word"))?;
    usize::try_from(u32::from_be_bytes(value)).map_err(|_| invalid("ICC word is too large"))
}

#[derive(Clone, Copy)]
struct TagRange {
    signature: [u8; 4],
    start: usize,
    end: usize,
}

/// Reject unsupported conversion semantics instead of silently falling back to a matrix.
pub(super) fn admit(bytes: &[u8]) -> Result<()> {
    if !(132..=MAX_ICC_BYTES).contains(&bytes.len()) || word(bytes, 0)? != bytes.len() {
        return Err(invalid("ICC profile must have its declared size, at most 256 KiB"));
    }
    if !matches!(bytes.get(8), Some(2 | 4))
        || !matches!(bytes.get(12..16), Some(b"scnr" | b"mntr"))
        || bytes.get(16..20) != Some(b"RGB ")
        || bytes.get(20..24) != Some(b"XYZ ")
        || bytes.get(36..40) != Some(b"acsp")
        || word(bytes, 64)? > 3
    {
        return Err(invalid("only version-2/4 RGB input/display ICC profiles with XYZ PCS are supported"));
    }
    let count = word(bytes, 128)?;
    if count == 0 || count > MAX_TAGS {
        return Err(invalid("ICC profile must have 1..64 tags"));
    }
    let table_end = 132 + count * 12; // count has already been capped.
    let table = bytes.get(132..table_end).ok_or_else(|| invalid("truncated ICC tag table"))?;
    let mut ranges: Vec<TagRange> = Vec::new();
    ranges.try_reserve_exact(count).map_err(|_| invalid("cannot allocate ICC tag table"))?;
    for entry in table.as_chunks::<12>().0 {
        let signature: [u8; 4] = entry.get(..4).and_then(|v| v.try_into().ok()).ok_or_else(|| invalid("invalid ICC tag"))?;
        if !matches!(
            &signature,
            b"rXYZ"
                | b"gXYZ"
                | b"bXYZ"
                | b"rTRC"
                | b"gTRC"
                | b"bTRC"
                | b"wtpt"
                | b"bkpt"
                | b"chad"
                | b"desc"
                | b"cprt"
                | b"dmnd"
                | b"dmdd"
                | b"tech"
                | b"view"
                | b"meas"
                | b"lumi"
                | b"chrm"
        ) {
            return Err(invalid("ICC tag is outside the initial RGB matrix/TRC profile policy"));
        }
        let start = word(entry, 4)?;
        let size = word(entry, 8)?;
        let end = start.checked_add(size).ok_or_else(|| invalid("ICC tag range overflow"))?;
        if start < table_end || start % 4 != 0 || !(8..=MAX_TAG_BYTES).contains(&size) {
            return Err(invalid("ICC tag range is invalid or exceeds 64 KiB"));
        }
        let data = bytes.get(start..end).ok_or_else(|| invalid("ICC tag extends past its profile"))?;
        for earlier in &ranges {
            // ICC permits identical shared data ranges, notably shared tone curves.
            if signature == earlier.signature || (start < earlier.end && earlier.start < end && (start != earlier.start || end != earlier.end)) {
                return Err(invalid("ICC has duplicate tags or partially overlapping data"));
            }
        }
        if data.get(4..8) != Some(&[0; 4]) {
            return Err(invalid("ICC tag reserved bytes must be zero"));
        }
        if matches!(&signature, b"rXYZ" | b"gXYZ" | b"bXYZ" | b"wtpt" | b"bkpt" | b"lumi") && (size != 20 || data.get(..4) != Some(b"XYZ ")) {
            return Err(invalid("ICC colorant/white-point tags must contain one XYZ value"));
        }
        if matches!(&signature, b"rTRC" | b"gTRC" | b"bTRC") {
            admit_curve(data)?;
        }
        if &signature == b"chad" && (size != 44 || data.get(..4) != Some(b"sf32")) {
            return Err(invalid("ICC chromatic adaptation must be one 3x3 matrix"));
        }
        if &signature == b"chrm" {
            // Informational device chromaticities, not a replacement for PCS colorants.
            // Initially admit three explicit channels (type 0), not enumerated phosphors.
            if size != 36 || data.get(..4) != Some(b"chrm") || data.get(8..12) != Some(&[0, 3, 0, 0]) {
                return Err(invalid("ICC chromaticities require three explicit device channels"));
            }
            for pair in data.get(12..).ok_or_else(|| invalid("truncated ICC chromaticities"))?.as_chunks::<8>().0 {
                let x = word(pair, 0)?;
                let y = word(pair, 4)?;
                if y == 0 || x > 65536 || y > 65536 || x + y > 65536 {
                    return Err(invalid("ICC device chromaticities are outside the initial xy policy"));
                }
            }
        }
        ranges.push(TagRange { signature, start, end });
    }
    for required in [*b"rXYZ", *b"gXYZ", *b"bXYZ", *b"rTRC", *b"gTRC", *b"bTRC", *b"wtpt"] {
        if !ranges.iter().any(|tag| tag.signature == required) {
            return Err(invalid("ICC matrix/TRC profile is missing a required tag"));
        }
    }
    Ok(())
}

fn admit_curve(data: &[u8]) -> Result<()> {
    match data.get(..4) {
        Some(b"curv") => {
            let count = word(data, 8)?;
            if count > MAX_TRC_VALUES || data.len() < 12 + count * 2 {
                return Err(invalid("ICC tone curve exceeds 4096 values or is truncated"));
            }
        }
        Some(b"para") => {
            let kind: [u8; 2] = data.get(8..10).and_then(|v| v.try_into().ok()).ok_or_else(|| invalid("truncated ICC parametric curve"))?;
            let parameters = match u16::from_be_bytes(kind) {
                0 => 1,
                1 => 3,
                2 => 4,
                3 => 5,
                4 => 7,
                _ => return Err(invalid("unsupported ICC parametric curve type")),
            };
            if data.len() < 12 + parameters * 4 {
                return Err(invalid("truncated ICC parametric curve parameters"));
            }
        }
        _ => return Err(invalid("ICC tone curve must be a bounded curve or parametric curve")),
    }
    Ok(())
}

/// ICC transform returns encoded sRGB; the caller separately linearizes RGB and preserves
/// alpha. Extended range is essential: the default destination LUT clips before callers
/// can detect gamut loss. f64 retains a 16-bit source LUT when an analytic curve is unavailable;
/// the CMS still uses f32 matrix/curve arithmetic internally, not full f64 colorimetry.
pub(super) fn prepare(bytes: &[u8]) -> Result<Arc<TransformF64Executor>> {
    admit(bytes)?;
    guarded(|| {
        let options = ParsingOptions { max_profile_size: MAX_ICC_BYTES + 1, max_allowed_clut_size: 0, max_allowed_trc_size: MAX_TRC_VALUES };
        let source = ColorProfile::new_from_slice_with_options(bytes, options).map_err(|e| invalid(format!("ICC profile: {e}")))?;
        if source.color_space != DataColorSpace::Rgb
            || source.pcs != DataColorSpace::Xyz
            || !matches!(source.profile_class, ProfileClass::InputDevice | ProfileClass::DisplayDevice)
            || !source.is_matrix_shaper()
        {
            return Err(invalid("ICC profile does not provide the admitted RGB matrix/TRC conversion"));
        }
        // The pinned CMS's same-curve optimization can treat two matching curves as
        // three matching curves. Keep initial admission narrower than that ambiguity.
        if source.red_trc != source.green_trc || source.red_trc != source.blue_trc {
            return Err(invalid("initial ICC conversion requires identical RGB tone curves"));
        }
        let options = TransformOptions {
            rendering_intent: RenderingIntent::RelativeColorimetric,
            allow_use_cicp_transfer: false,
            prefer_fixed_point: false,
            allow_extended_range_rgb_xyz: true,
            ..TransformOptions::default()
        };
        source.create_transform_f64(Layout::Rgb, &ColorProfile::new_srgb(), Layout::Rgb, options).map_err(|e| invalid(format!("ICC transform: {e}")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(linear: bool) -> Vec<u8> {
        let mut profile = ColorProfile::new_srgb();
        profile.cicp = None;
        // Original even-length text keeps the pinned writer's tags aligned. Its default
        // odd-length text can produce an unaligned following tag; admission stays strict.
        let text =
            moxcms::ProfileText::Localizable(vec![moxcms::LocalizableString::new("en".into(), "US".into(), "Original synthetic fixture".into())]);
        profile.description = Some(text.clone());
        profile.copyright = Some(text);
        if linear {
            let curve = moxcms::ToneReprCurve::Parametric(vec![1.0]);
            profile.red_trc = Some(curve.clone());
            profile.green_trc = Some(curve.clone());
            profile.blue_trc = Some(curve);
        }
        profile.encode().unwrap()
    }

    #[test]
    fn linear_rgb_profile_is_converted_instead_of_discarded() {
        let transform = prepare(&profile(true)).unwrap();
        let mut out = [0.0; 3];
        transform.transform(&[0.25, 0.5, 0.75], &mut out).unwrap();
        for (input, actual) in [0.25_f64, 0.5, 0.75].into_iter().zip(out) {
            let expected = 1.055 * input.powf(1.0 / 2.4) - 0.055;
            assert!((actual - expected).abs() < 3.0 / 65535.0, "{actual} != {expected}");
        }
    }

    #[test]
    fn adjacent_16_bit_source_values_remain_distinguishable() {
        let transform = prepare(&profile(false)).unwrap();
        let input = [32768.0 / 65535.0, 32769.0 / 65535.0, 0.0];
        let mut out = [0.0; 3];
        transform.transform(&input, &mut out).unwrap();
        assert!(out[1] - out[0] > 0.5 / 65535.0);
        assert!((out[0] - input[0]).abs() < 3.0 / 65535.0);
        assert!((out[1] - input[1]).abs() < 3.0 / 65535.0);
    }

    #[test]
    fn shared_curve_ranges_are_allowed_but_partial_overlap_is_not() {
        let mut encoded = profile(false);
        let count = word(&encoded, 128).unwrap();
        let entries: Vec<_> = encoded[132..132 + count * 12]
            .as_chunks::<12>()
            .0
            .iter()
            .enumerate()
            .map(|(i, tag)| (132 + i * 12, <[u8; 4]>::try_from(&tag[..4]).unwrap()))
            .collect();
        let red = entries.iter().find(|(_, signature)| signature == b"rTRC").unwrap().0;
        let shared = encoded[red + 4..red + 12].to_vec();
        for &(offset, signature) in &entries {
            if signature == *b"gTRC" || signature == *b"bTRC" {
                encoded[offset + 4..offset + 12].copy_from_slice(&shared);
            }
        }
        assert!(prepare(&encoded).is_ok());
        let mut overlap = profile(false);
        let first_start = word(&overlap, 136).unwrap();
        overlap[148..152].copy_from_slice(&(u32::try_from(first_start).unwrap() + 4).to_be_bytes());
        assert!(admit(&overlap).unwrap_err().to_string().contains("overlapping"));
    }

    #[test]
    fn malformed_profiles_and_unsupported_semantics_refuse_before_transform() {
        let original = profile(false);
        for length in [0, 1, 127, 131, original.len() - 1] {
            assert!(admit(&original[..length]).is_err());
        }
        let mut large = vec![0; MAX_ICC_BYTES + 1];
        assert!(admit(&large).is_err());
        large.clear();
        for (offset, replacement) in [(8, vec![3]), (16, b"CMYK".to_vec()), (128, 65u32.to_be_bytes().to_vec()), (132, b"A2B0".to_vec())] {
            let mut bad = original.clone();
            bad[offset..offset + replacement.len()].copy_from_slice(&replacement);
            assert!(admit(&bad).is_err());
        }
        for (offset, replacement) in [(136, u32::MAX), (140, u32::MAX), (136, 253)] {
            let mut bad = original.clone();
            bad[offset..offset + 4].copy_from_slice(&replacement.to_be_bytes());
            assert!(admit(&bad).is_err());
        }
        let mut duplicate = original;
        let first: [u8; 4] = duplicate[132..136].try_into().unwrap();
        duplicate[144..148].copy_from_slice(&first);
        assert!(admit(&duplicate).is_err());
    }

    #[test]
    fn explicit_device_chromaticities_are_validated_without_replacing_pcs_colorants() {
        let mut encoded = profile(true);
        let count = word(&encoded, 128).unwrap();
        let table_end = 132 + count * 12;
        encoded.splice(table_end..table_end, [0; 12]);
        for entry in encoded[132..table_end].as_chunks_mut::<12>().0 {
            let shifted = u32::try_from(word(entry, 4).unwrap() + 12).unwrap();
            entry[4..8].copy_from_slice(&shifted.to_be_bytes());
        }
        let start = (encoded.len() + 3) & !3;
        encoded.resize(start, 0);
        let mut data = b"chrm\0\0\0\0\0\x03\0\0".to_vec();
        for value in [0.64_f64, 0.33, 0.3, 0.6, 0.15, 0.06] {
            data.extend_from_slice(&((value * 65536.0).round() as u32).to_be_bytes());
        }
        encoded.extend_from_slice(&data);
        let size = u32::try_from(encoded.len()).unwrap();
        encoded[..4].copy_from_slice(&size.to_be_bytes());
        encoded[128..132].copy_from_slice(&u32::try_from(count + 1).unwrap().to_be_bytes());
        encoded[table_end..table_end + 4].copy_from_slice(b"chrm");
        encoded[table_end + 4..table_end + 8].copy_from_slice(&u32::try_from(start).unwrap().to_be_bytes());
        encoded[table_end + 8..table_end + 12].copy_from_slice(&36_u32.to_be_bytes());
        let mut result = [0.0; 3];
        prepare(&encoded).unwrap().transform(&[0.25, 0.5, 0.75], &mut result).unwrap();
        assert!((result[0] - (1.055 * 0.25_f64.powf(1.0 / 2.4) - 0.055)).abs() < 3.0 / 65535.0);
        for (offset, replacement) in [(start + 9, 4), (start + 11, 1), (start + 4, 1), (start + 12, 255)] {
            let mut bad = encoded.clone();
            bad[offset] = replacement;
            assert!(admit(&bad).is_err());
        }
    }
}
