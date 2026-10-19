//! Bounded framing/presence checks; the PNG crate still decodes and validates chunks.
use crate::texture_input::{Result, invalid};

const SEMANTIC: [[u8; 4]; 8] = [*b"iCCP", *b"sRGB", *b"gAMA", *b"cHRM", *b"eXIf", *b"cICP", *b"mDCV", *b"cLLI"];

pub(super) struct Markers([bool; 8]);

impl Markers {
    pub fn check(&self, info: &png::Info<'_>) -> Result<()> {
        let parsed = [
            info.icc_profile.is_some(),
            info.srgb.is_some(),
            info.gama_chunk.is_some(),
            info.chrm_chunk.is_some(),
            info.exif_metadata.is_some(),
            info.coding_independent_code_points.is_some(),
            info.mastering_display_color_volume.is_some(),
            info.content_light_level.is_some(),
        ];
        if self.0 != parsed {
            return Err(invalid("PNG decoder did not retain declared color/orientation metadata"));
        }
        Ok(())
    }
}

pub(super) fn read(bytes: &[u8]) -> Result<Markers> {
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return Err(invalid("invalid PNG signature"));
    }
    let mut offset = 8_usize;
    let mut markers = [false; 8];
    let mut pixels_seen = false;
    for _ in 0..4096 {
        let header_end = offset.checked_add(8).ok_or_else(|| invalid("PNG chunk offset overflow"))?;
        let header = bytes.get(offset..header_end).ok_or_else(|| invalid("truncated PNG chunk header"))?;
        let length: [u8; 4] = header.get(..4).and_then(|v| v.try_into().ok()).ok_or_else(|| invalid("invalid PNG length"))?;
        let length = usize::try_from(u32::from_be_bytes(length)).map_err(|_| invalid("PNG length overflow"))?;
        let kind = header.get(4..8).ok_or_else(|| invalid("invalid PNG chunk type"))?;
        let end = header_end.checked_add(length).and_then(|v| v.checked_add(4)).ok_or_else(|| invalid("PNG chunk size overflow"))?;
        if end > bytes.len() {
            return Err(invalid("truncated PNG chunk"));
        }
        if let Some(index) = SEMANTIC.iter().position(|tag| tag == kind) {
            let seen = markers.get_mut(index).ok_or_else(|| invalid("PNG metadata index overflow"))?;
            if *seen || pixels_seen {
                return Err(invalid("duplicate or late PNG color/orientation metadata is unsupported"));
            }
            *seen = true;
        }
        if kind == b"IDAT" {
            pixels_seen = true;
        }
        if kind == b"IEND" {
            if length != 0 || end != bytes.len() {
                return Err(invalid("PNG IEND must be empty and final"));
            }
            if matches!(markers, [true, true, ..]) {
                return Err(invalid("PNG has conflicting ICC and sRGB declarations"));
            }
            return Ok(Markers(markers));
        }
        offset = end;
    }
    Err(invalid("PNG normalization exceeds 4096 chunks"))
}
