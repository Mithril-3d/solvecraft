use super::*;

/// An axis-aligned box as a closed mesh (each side two triangles, counter-clockwise from outside).
fn cuboid(lo: Vec3, hi: Vec3) -> Mesh {
    let v = |x: bool, y: bool, z: bool| Vec3::new(if x { hi.x } else { lo.x }, if y { hi.y } else { lo.y }, if z { hi.z } else { lo.z });
    let quads = [
        [v(false, false, false), v(false, true, false), v(true, true, false), v(true, false, false)],
        [v(false, false, true), v(true, false, true), v(true, true, true), v(false, true, true)],
        [v(false, false, false), v(true, false, false), v(true, false, true), v(false, false, true)],
        [v(false, true, false), v(false, true, true), v(true, true, true), v(true, true, false)],
        [v(false, false, false), v(false, false, true), v(false, true, true), v(false, true, false)],
        [v(true, false, false), v(true, true, false), v(true, true, true), v(true, false, true)],
    ];
    let mut m = Mesh::default();
    for (f, q) in quads.iter().enumerate() {
        let base = m.positions.len() as u32;
        m.positions.extend_from_slice(q);
        m.normals.extend_from_slice(&[Vec3::Z; 4]);
        m.triangles.push([base, base + 1, base + 2]);
        m.triangles.push([base, base + 2, base + 3]);
        m.tri_face.extend([f as u32, f as u32]);
    }
    m
}

#[test]
fn the_tree_finds_the_nearest_point_as_a_search_of_every_triangle_does() {
    // A crumpled sheet of triangles from a fixed pseudo-random sequence.
    let mut seed = 12345u64;
    let mut rnd = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut m = Mesh::default();
    for _ in 0..500 {
        let c = Vec3::new(rnd() * 100.0, rnd() * 50.0, rnd() * 20.0);
        let base = m.positions.len() as u32;
        for _ in 0..3 {
            m.positions.push(c + Vec3::new(rnd() * 6.0 - 3.0, rnd() * 6.0 - 3.0, rnd() * 6.0 - 3.0));
        }
        m.triangles.push([base, base + 1, base + 2]);
    }
    let tree = TriangleTree::new(&m);
    assert_eq!(tree.len(), 500);
    for _ in 0..300 {
        let p = Vec3::new(rnd() * 140.0 - 20.0, rnd() * 90.0 - 20.0, rnd() * 60.0 - 20.0);
        let brute =
            m.triangles.iter().filter_map(|t| m.tri(t)).map(|[a, b, c]| closest_on_triangle(p, a, b, c).dist(p)).fold(f64::INFINITY, f64::min);
        let (d, i) = tree.nearest(p).unwrap();
        assert!((d - brute).abs() < 1e-12, "{d} vs {brute}");
        let [a, b, c] = m.tri(&m.triangles[i as usize]).unwrap();
        assert!((closest_on_triangle(p, a, b, c).dist(p) - d).abs() < 1e-12);
    }
    assert!(TriangleTree::new(&Mesh::default()).nearest(Vec3::ZERO).is_none());
}

#[test]
fn a_mesh_and_its_copy_are_identical() {
    let a = cuboid(Vec3::ZERO, Vec3::new(40.0, 30.0, 10.0));
    let c = compare(&a, &a.clone(), &CompareOptions::default(), &serial);
    assert!(c.regions.is_empty(), "{:?}", c.regions);
    // The same triangles: nothing but rounding apart.
    assert!(c.a_to_b.max < 1e-9 && c.b_to_a.max < 1e-9, "{} {}", c.a_to_b.max, c.b_to_a.max);
    // Sampled at the requested spacing: the whole surface, and nothing beyond the tolerance.
    assert!((c.a_to_b.area - 2.0 * (1200.0 + 400.0 + 300.0)).abs() < 1e-6, "{}", c.a_to_b.area);
    assert_eq!(c.a_to_b.beyond_area, 0.0);
    assert_eq!(c.spacing, 0.5);
}

#[test]
fn a_box_moved_up_has_material_only_it_has_on_top_and_none_below() {
    // b is a moved 0.3 up: a slab 40 x 30 x 0.3 of each side.
    let a = cuboid(Vec3::ZERO, Vec3::new(40.0, 30.0, 10.0));
    let b = cuboid(Vec3::new(0.0, 0.0, 0.3), Vec3::new(40.0, 30.0, 10.3));
    let c = compare(&a, &b, &CompareOptions::default(), &serial);
    assert_eq!(c.regions.len(), 2, "{:?}", c.regions);
    assert!((c.a_to_b.max - 0.3).abs() < 1e-9 && (c.b_to_a.max - 0.3).abs() < 1e-9);
    for r in &c.regions {
        // Integrated along lines 0.18 mm apart: the slab's rim falls between them.
        assert!((r.volume - 360.0).abs() < 360.0 * 0.01, "{r:?}");
        assert_eq!(r.lines.axis, 2);
        assert_eq!(r.lines.skipped, 0);
        let z = if r.side == Side::OnlyInA { 0.15 } else { 10.15 };
        assert!(r.center.dist(Vec3::new(20.0, 15.0, z)) < 0.1, "{r:?}");
        assert!((r.bbox.min.z - (z - 0.15)).abs() < 1e-9 && (r.bbox.max.z - (z + 0.15)).abs() < 1e-9, "{r:?}");
        // Each slab is bounded by the bottom (top) of both meshes, and by the 0.3 mm of the sides
        // of the one that reaches further, less what lies within the tolerance of the other.
        let (outer, inner) = if r.side == Side::OnlyInA { (r.area_a, r.area_b) } else { (r.area_b, r.area_a) };
        assert!((outer - (1200.0 + 140.0 * 0.29)).abs() < 2.0 && (inner - 39.98 * 29.98).abs() < 2.0, "{r:?}");
    }
    let sides: Vec<Side> = c.regions.iter().map(|r| r.side).collect();
    assert!(sides.contains(&Side::OnlyInA) && sides.contains(&Side::OnlyInB));
}

#[test]
fn a_box_inside_another_is_material_only_the_outer_one_has() {
    // No surface of one is near the other's: one region, the shell between them.
    let a = cuboid(Vec3::ZERO, Vec3::new(20.0, 20.0, 20.0));
    let b = cuboid(Vec3::new(5.0, 5.0, 5.0), Vec3::new(15.0, 15.0, 15.0));
    let c = compare(&a, &b, &CompareOptions::default(), &serial);
    assert_eq!(c.regions.len(), 1, "{:?}", c.regions);
    let r = &c.regions[0];
    assert_eq!(r.side, Side::OnlyInA);
    assert!((r.volume - 7000.0).abs() < 7000.0 * 0.01, "{}", r.volume);
    assert!(r.center.dist(Vec3::new(10.0, 10.0, 10.0)) < 1e-2, "{:?}", r.center);
    assert_eq!(r.faces_a.len(), 6);
    assert_eq!(r.faces_b.len(), 6);
}

#[test]
fn a_tight_budget_coarsens_the_spacing() {
    let a = cuboid(Vec3::ZERO, Vec3::new(400.0, 300.0, 100.0));
    let o = CompareOptions { max_points: 20_000, ..CompareOptions::default() };
    let c = compare(&a, &a, &o, &serial);
    assert!(c.spacing > 0.5, "{}", c.spacing);
    assert!((5_000..=24_000).contains(&c.a_to_b.points), "{}", c.a_to_b.points);
    assert!(c.regions.is_empty());
}
