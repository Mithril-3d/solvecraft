//! Original SolidCraft mark, shared by desktop, packaging and CLI.
/// This block is the icon image data
pub fn rgba(size: usize) -> Result<Vec<u8>, String> {
    if !(1..=2048).contains(&size) {
        return Err("icon size must be 1..=2048".into());
    }
    type P = (f32, f32);
    // Coverage buffer at 4x4 samples per pixel (2x2 for big sizes), averaged down at the end.
    let ss = if size > 512 { 2 } else { 4 };
    let n = size * ss;
    let mut buf = vec![[0u8; 4]; n * n];
    let s = n as f32 / 412.217; // SVG units -> samples: the logo fills the height
    let ox = (n as f32 - 357.0 * s) / 2.0; // and is centred sideways
    // Fill a convex polygon given in SVG units.
    let mut fill = |pts: &[P], col: [u8; 3]| {
        let v: Vec<P> = pts.iter().map(|p| (ox + p.0 * s, p.1 * s)).collect();
        let (top, bottom) = v.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
        for y in ((top - 0.5).ceil() as i32).max(0)..=((bottom - 0.5).floor() as i32).min(n as i32 - 1) {
            let cy = y as f32 + 0.5;
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for i in 0..v.len() {
                let (a, b) = (v[i], v[(i + 1) % v.len()]);
                if (a.1 <= cy) != (b.1 <= cy) {
                    let x = a.0 + (cy - a.1) / (b.1 - a.1) * (b.0 - a.0);
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
            if lo > hi {
                continue;
            }
            for x in ((lo - 0.5).ceil() as i32).max(0)..=((hi - 0.5).floor() as i32).min(n as i32 - 1) {
                buf[y as usize * n + x as usize] = [col[0], col[1], col[2], 255];
            }
        }
    };
    // Faces: each is a few overlapping convex pieces (the filler only does convex shapes).
    let (light, mid, dark) = ([96, 158, 236], [58, 136, 238], [55, 108, 197]);
    fill(&[(3.5, 105.9), (177.0, 5.8), (263.4, 55.6), (263.8, 156.0), (177.0, 406.4)], light);
    fill(&[(177.0, 206.1), (263.8, 156.0), (350.5, 206.2), (350.5, 306.3), (177.0, 406.4)], light);
    fill(&[(3.5, 105.9), (177.0, 206.1), (177.0, 406.4)], mid);
    fill(&[(3.5, 105.9), (3.5, 205.7), (90.3, 256.2), (177.0, 206.1)], mid);
    fill(&[(177.0, 206.1), (90.3, 256.2), (90.0, 356.2), (177.0, 406.4)], mid);
    fill(&[(264.1, 155.8), (177.0, 206.1), (177.0, 406.4), (263.8, 256.2)], dark);
    fill(&[(263.8, 256.2), (350.5, 206.2), (350.5, 306.3), (177.0, 406.4)], dark);
    fill(&[(177.0, 206.1), (177.0, 406.4), (350.5, 306.3)], dark);
    fill(&[(263.8, 55.9), (263.8, 156.0), (177.1, 105.9)], dark);
    // White edge lines, 10 units wide: a rectangle per segment and a disc at each inner corner.
    let white = [255, 255, 255];
    let mut stroke = |pts: &[P], closed: bool| {
        let m = pts.len();
        for i in 0..(if closed { m } else { m - 1 }) {
            let (a, b) = (pts[i], pts[(i + 1) % m]);
            let l = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            let (nx, ny) = (-(b.1 - a.1) / l * 5.0, (b.0 - a.0) / l * 5.0);
            fill(&[(a.0 + nx, a.1 + ny), (b.0 + nx, b.1 + ny), (b.0 - nx, b.1 - ny), (a.0 - nx, a.1 - ny)], white);
        }
        for c in &pts[if closed { 0 } else { 1 }..if closed { m } else { m - 1 }] {
            let disc: Vec<P> = (0..16)
                .map(|k| {
                    let t = k as f32 * std::f32::consts::TAU / 16.0;
                    (c.0 + 5.0 * t.cos(), c.1 + 5.0 * t.sin())
                })
                .collect();
            fill(&disc, white);
        }
    };
    stroke(&[(263.8, 55.9), (177.1, 105.9), (263.8, 156.0), (263.8, 256.2), (350.6, 206.1)], false);
    stroke(&[(5.0, 105.9), (178.5, 206.1), (178.5, 406.4)], false);
    stroke(&[(177.0, 206.1), (263.8, 156.0)], false);
    stroke(
        &[
            (178.5, 5.8),
            (5.0, 105.9),
            (5.0, 206.1),
            (91.7, 256.2),
            (91.7, 356.4),
            (178.5, 406.4),
            (352.0, 306.3),
            (352.0, 206.1),
            (265.2, 156.0),
            (265.2, 55.9),
        ],
        true,
    );
    // Average each ss x ss block down to one pixel (straight alpha = covered share).
    let mut rgba = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let (mut sum, mut cnt) = ([0u32; 3], 0u32);
            for j in 0..ss {
                for i in 0..ss {
                    let p = buf[(y * ss + j) * n + x * ss + i];
                    if p[3] != 0 {
                        cnt += 1;
                        for k in 0..3 {
                            sum[k] += u32::from(p[k]);
                        }
                    }
                }
            }
            if cnt == 0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let a = 255 * cnt / (ss * ss) as u32;
                rgba.extend_from_slice(&[(sum[0] / cnt) as u8, (sum[1] / cnt) as u8, (sum[2] / cnt) as u8, a as u8]);
            }
        }
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    #[test]
    fn shared_mark_has_transparency_and_original_colors_at_window_and_package_sizes() {
        for size in [16, 24, 32, 48, 64, 128, 256] {
            let pixels = super::rgba(size).expect("valid size");
            assert_eq!(pixels.len(), size * size * 4);
            assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] == 0));
            assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] == 255 && p[2] > p[0]));
        }
        assert!(super::rgba(0).is_err());
        assert!(super::rgba(2049).is_err());
    }
}
