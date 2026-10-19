//! Application icons of installed slicers, read from the application itself: PNG files, macOS
//! `.icns` files and the icon resources of Windows programs. Everything read is untrusted:
//! offsets and sizes are checked and capped, and nothing panics.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// Largest icon image taken from a file.
const MAX_ICON_BYTES: usize = 1 << 20;
/// Most resource directory entries looked at in one directory.
const MAX_RES_ENTRIES: usize = 512;

pub fn read_capped(path: &Path, max: u64) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(max.checked_add(1)?).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= max).then_some(bytes)
}

/// The largest PNG image inside an `.icns` file (up to a size cap).
pub fn icns_png(data: &[u8]) -> Option<Vec<u8>> {
    if data.get(..4)? != b"icns" {
        return None;
    }
    let mut best: Option<&[u8]> = None;
    let mut at = 8usize;
    while let Some(head) = data.get(at..at.checked_add(8)?) {
        let len = u32::from_be_bytes(head.get(4..8)?.try_into().ok()?) as usize;
        if len < 8 {
            break;
        }
        let body = data.get(at + 8..at.checked_add(len)?)?;
        if body.starts_with(&PNG_MAGIC) && body.len() <= MAX_ICON_BYTES && best.is_none_or(|b| body.len() > b.len()) {
            best = Some(body);
        }
        at = at.checked_add(len)?;
    }
    best.map(<[u8]>::to_vec)
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// A Windows program, read in pieces (the files are large).
struct Pe<R> {
    file: R,
    sections: Vec<(u32, u32, u32)>,
    /// File offset of the resource section's start, and its RVA.
    res: (u64, u32),
}

impl<R: Read + Seek> Pe<R> {
    fn bytes(&mut self, at: u64, len: usize) -> Option<Vec<u8>> {
        if len > MAX_ICON_BYTES {
            return None;
        }
        self.file.seek(SeekFrom::Start(at)).ok()?;
        let mut out = vec![0; len];
        self.file.read_exact(&mut out).ok()?;
        Some(out)
    }

    fn open(mut file: R) -> Option<Pe<R>> {
        file.seek(SeekFrom::Start(0)).ok()?;
        let mut head = vec![0; 64];
        file.read_exact(&mut head).ok()?;
        if head.get(..2)? != b"MZ" {
            return None;
        }
        let pe_at = u64::from(u32_at(&head, 0x3C)?);
        let mut pe = Pe { file, sections: Vec::new(), res: (0, 0) };
        let coff = pe.bytes(pe_at, 24)?;
        if coff.get(..4)? != b"PE\0\0" {
            return None;
        }
        let count = usize::from(u16_at(&coff, 6)?).min(96);
        let optional_len = usize::from(u16_at(&coff, 20)?);
        let optional = pe.bytes(pe_at + 24, optional_len.min(512))?;
        let dirs_at = match u16_at(&optional, 0)? {
            0x10B => 96,
            0x20B => 112,
            _ => return None,
        };
        // Data directory 2: the resource table.
        let (rva, size) = (u32_at(&optional, dirs_at + 16)?, u32_at(&optional, dirs_at + 20)?);
        if rva == 0 || size == 0 {
            return None;
        }
        let table = pe.bytes(pe_at + 24 + optional_len as u64, count * 40)?;
        for i in 0..count {
            let s = table.get(i * 40..i * 40 + 40)?;
            // (virtual address, virtual size, file offset)
            pe.sections.push((u32_at(s, 12)?, u32_at(s, 8)?.max(u32_at(s, 16)?), u32_at(s, 20)?));
        }
        pe.res = (u64::from(pe.rva_to_offset(rva)?), rva);
        Some(pe)
    }

    fn rva_to_offset(&self, rva: u32) -> Option<u32> {
        self.sections.iter().find(|(va, size, _)| rva >= *va && rva - *va < *size).and_then(|(va, _, raw)| raw.checked_add(rva - va))
    }

    /// The entries of a resource directory at `offset` into the resource section: (id, target),
    /// where the target's high bit marks a subdirectory.
    fn directory(&mut self, offset: u32) -> Option<Vec<(u32, u32)>> {
        let at = self.res.0.checked_add(u64::from(offset))?;
        let head = self.bytes(at, 16)?;
        let n = (usize::from(u16_at(&head, 12)?) + usize::from(u16_at(&head, 14)?)).min(MAX_RES_ENTRIES);
        let list = self.bytes(at + 16, n * 8)?;
        (0..n).map(|i| Some((u32_at(&list, i * 8)?, u32_at(&list, i * 8 + 4)?))).collect()
    }

    /// The first resource of `kind` (3 icon, 14 icon group): `id` picks one, `None` the first.
    fn resource(&mut self, kind: u32, id: Option<u32>) -> Option<Vec<u8>> {
        let types = self.directory(0)?;
        let (_, kind_dir) = types.into_iter().find(|(t, target)| *t == kind && target & 0x8000_0000 != 0)?;
        let named = self.directory(kind_dir & 0x7FFF_FFFF)?;
        let (_, name_dir) = named.into_iter().find(|(n, _)| id.is_none_or(|id| *n == id))?;
        let target = if name_dir & 0x8000_0000 != 0 {
            // One level of languages; the first will do.
            self.directory(name_dir & 0x7FFF_FFFF)?.first()?.1
        } else {
            name_dir
        };
        if target & 0x8000_0000 != 0 {
            return None;
        }
        let entry = self.bytes(self.res.0.checked_add(u64::from(target))?, 16)?;
        let (rva, size) = (u32_at(&entry, 0)?, u32_at(&entry, 4)? as usize);
        self.bytes(u64::from(self.rva_to_offset(rva)?), size)
    }

    /// The program's first icon group as a one-image `.ico` file.
    fn icon(&mut self) -> Option<Vec<u8>> {
        let group = self.resource(14, None)?;
        let count = usize::from(u16_at(&group, 4)?).min(64);
        // The largest image (a width of 0 means 256).
        let mut best: Option<(u32, usize)> = None;
        for i in 0..count {
            let e = group.get(6 + i * 14..6 + i * 14 + 14)?;
            let width = if *e.first()? == 0 { 256 } else { u32::from(*e.first()?) };
            if best.is_none_or(|(w, _)| width > w) {
                best = Some((width, i));
            }
        }
        let e = group.get(6 + best?.1 * 14..)?.get(..14)?;
        let data = self.resource(3, Some(u32::from(u16_at(e, 12)?)))?;
        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend_from_slice(e.get(..8)?);
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(&data);
        Some(ico)
    }
}

/// The main icon of a Windows program as `.ico` bytes.
pub fn exe_icon(path: &Path) -> Option<Vec<u8>> {
    Pe::open(std::fs::File::open(path).ok()?)?.icon()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn png_like(extra: usize) -> Vec<u8> {
        let mut v = PNG_MAGIC.to_vec();
        v.extend(std::iter::repeat_n(7u8, extra));
        v
    }

    #[test]
    fn icns_gives_the_largest_png() {
        let (small, big) = (png_like(4), png_like(40));
        let mut file = b"icns".to_vec();
        file.extend_from_slice(&0u32.to_be_bytes());
        for (kind, body) in [(b"is32", vec![1u8; 10]), (b"ic07", small), (b"ic08", big.clone())] {
            file.extend_from_slice(kind);
            file.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
            file.extend_from_slice(&body);
        }
        assert_eq!(icns_png(&file), Some(big));
        assert_eq!(icns_png(b"nope"), None);
        // A length that runs past the file ends the walk instead of panicking.
        let mut bad = b"icns\0\0\0\0ic07".to_vec();
        bad.extend_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(icns_png(&bad), None);
    }

    /// A tiny PE file: one `.rsrc` section holding an icon group of one 16×16 image.
    fn synthetic_pe(image: &[u8]) -> Vec<u8> {
        let le32 = |v: u32| v.to_le_bytes().to_vec();
        let le16 = |v: u16| v.to_le_bytes().to_vec();
        let (pe_at, opt_len, raw) = (0x40usize, 0xF0usize, 0x200u32);
        let rva = 0x1000u32;
        // Resource section content, built back to front with absolute section offsets.
        let group_data: Vec<u8> = [le16(0), le16(1), le16(1), vec![16, 16, 0, 0], le16(1), le16(32), le32(image.len() as u32), le16(5)].concat();
        let dir = |entries: &[(u32, u32)]| -> Vec<u8> {
            let mut d = [le32(0), le32(0), le16(0), le16(0), le16(0), le16(entries.len() as u16)].concat();
            for (id, target) in entries {
                d.extend(le32(*id));
                d.extend(le32(*target));
            }
            d
        };
        // Layout: 0 root | 32 RT_ICON dir | 56 RT_GROUP_ICON dir | 80 icon 5 langs | 104 group 1 langs
        // | 128 data entries (icon, group) | 160 icon bytes | then group bytes.
        let icon_at = 160u32;
        let group_at = icon_at + image.len() as u32;
        let mut rsrc = dir(&[(3, 0x8000_0000 | 32), (14, 0x8000_0000 | 56)]);
        rsrc.extend(dir(&[(5, 0x8000_0000 | 80)]));
        rsrc.extend(dir(&[(1, 0x8000_0000 | 104)]));
        rsrc.extend(dir(&[(0x409, 128)]));
        rsrc.extend(dir(&[(0x409, 144)]));
        assert_eq!(rsrc.len(), 128);
        for (at, len) in [(icon_at, image.len()), (group_at, group_data.len())] {
            rsrc.extend([le32(rva + at), le32(len as u32), le32(0), le32(0)].concat());
        }
        rsrc.extend_from_slice(image);
        rsrc.extend_from_slice(&group_data);
        let mut f = vec![0u8; 0x40];
        f[..2].copy_from_slice(b"MZ");
        f[0x3C..0x40].copy_from_slice(&(pe_at as u32).to_le_bytes());
        f.extend(b"PE\0\0");
        f.extend([le16(0x8664), le16(1), le32(0), le32(0), le32(0), le16(opt_len as u16), le16(0)].concat());
        let mut opt = vec![0u8; opt_len];
        opt[..2].copy_from_slice(&0x20Bu16.to_le_bytes());
        opt[112 + 16..112 + 20].copy_from_slice(&rva.to_le_bytes());
        opt[112 + 20..112 + 24].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
        f.extend(opt);
        let mut section = b".rsrc\0\0\0".to_vec();
        section.extend([le32(rsrc.len() as u32), le32(rva), le32(rsrc.len() as u32), le32(raw), vec![0; 16]].concat());
        f.extend(section);
        f.resize(raw as usize, 0);
        f.extend(rsrc);
        f
    }

    #[test]
    fn reads_the_icon_group_of_a_program() {
        let image = vec![9u8; 40];
        let ico = Pe::open(Cursor::new(synthetic_pe(&image))).and_then(|mut p| p.icon()).expect("icon");
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(&ico[6..8], &[16, 16]);
        assert_eq!(u32::from_le_bytes([ico[14], ico[15], ico[16], ico[17]]), 40);
        assert_eq!(&ico[22..], &image[..]);
    }

    #[test]
    fn hostile_programs_give_nothing() {
        let good = synthetic_pe(&[1; 8]);
        assert!(Pe::open(Cursor::new(Vec::new())).is_none());
        assert!(Pe::open(Cursor::new(b"MZ".to_vec())).is_none());
        // Every truncation, and every single corrupted byte, must end without a panic.
        for cut in 0..good.len() {
            let _ = Pe::open(Cursor::new(good[..cut].to_vec())).and_then(|mut p| p.icon());
        }
        for at in 0..good.len() {
            let mut bad = good.clone();
            bad[at] = bad[at].wrapping_add(0x80);
            let _ = Pe::open(Cursor::new(bad)).and_then(|mut p| p.icon());
        }
    }
}
