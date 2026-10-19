//! Metadata presence/framing admission from T.81 and ICC's JPEG embedding note.
//! This does not decode entropy data or replace the established JPEG codec.
use crate::texture_input::{Result, invalid};

pub(super) fn read(bytes: &[u8]) -> Result<(bool, bool)> {
    if bytes.get(..2) != Some(b"\xff\xd8") {
        return Err(invalid("invalid JPEG SOI"));
    }
    let mut offset = 2;
    let mut entropy = false;
    let mut scan_seen = false;
    let mut exif = false;
    let mut icc = false;
    let mut segments = 0;
    while offset < bytes.len() {
        if bytes.get(offset) != Some(&255) {
            if !entropy {
                return Err(invalid("JPEG marker expected"));
            }
            offset += 1;
            continue;
        }
        while bytes.get(offset) == Some(&255) {
            offset += 1;
        }
        let marker = *bytes.get(offset).ok_or_else(|| invalid("truncated JPEG marker"))?;
        offset += 1;
        if entropy && (marker == 0 || (0xd0..=0xd7).contains(&marker)) {
            continue;
        }
        entropy = false;
        segments += 1;
        if segments > 4096 {
            return Err(invalid("JPEG normalization exceeds 4096 marker segments"));
        }
        if marker == 0xd9 {
            return if offset == bytes.len() { Ok((icc, exif)) } else { Err(invalid("JPEG EOI must be final")) };
        }
        if marker == 0 || marker == 0xd8 || marker == 1 || (0xd0..=0xd7).contains(&marker) {
            return Err(invalid("unsupported JPEG standalone marker"));
        }
        let header_end = offset.checked_add(2).ok_or_else(|| invalid("JPEG offset overflow"))?;
        let length: [u8; 2] = bytes.get(offset..header_end).and_then(|v| v.try_into().ok()).ok_or_else(|| invalid("truncated JPEG length"))?;
        let length = usize::from(u16::from_be_bytes(length));
        let end = offset.checked_add(length).ok_or_else(|| invalid("JPEG segment overflow"))?;
        if length < 2 {
            return Err(invalid("invalid JPEG segment length"));
        }
        let data = bytes.get(header_end..end).ok_or_else(|| invalid("truncated JPEG segment"))?;
        if marker == 0xe2 && data.starts_with(b"ICC_PROFILE\0") {
            if scan_seen || data.len() <= 14 {
                return Err(invalid("late or truncated JPEG ICC metadata"));
            }
            icc = true; // The codec assembles sequences; presence must survive that validation.
        }
        if marker == 0xe1 && data.starts_with(b"Exif\0\0") {
            if scan_seen || exif || data.len() <= 6 {
                return Err(invalid("duplicate, late or truncated JPEG EXIF metadata"));
            }
            exif = true;
        }
        if (0xc0..=0xcf).contains(&marker)
            && !matches!(marker, 0xc4 | 0xc8 | 0xcc)
            && (!matches!(marker, 0xc0..=0xc2) || data.first() != Some(&8) || !matches!(data.get(5), Some(1 | 3)))
        {
            return Err(invalid("initial JPEG normalization requires 8-bit grayscale or three-component DCT input"));
        }
        if marker == 0xda {
            scan_seen = true;
            entropy = true;
        }
        offset = end;
    }
    Err(invalid("JPEG EOI missing"))
}
