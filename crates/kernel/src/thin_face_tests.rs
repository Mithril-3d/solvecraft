//! A face thinner than the tolerance still meshes: the lens of a sphere's patch that pokes out
//! of a box's side has two edges that, sampled coarsely, run along one line in its parameters.
//! The shell's mesh had a hole there.

use std::collections::HashMap;

use solidcraft_geom::Vec3;

use crate::{BoolOp, build::box_solid, build::sphere, ops::boolean};

/// Triangle sides in the mesh with no triangle on the other side of them, with the points
/// within a thousandth of a millimetre taken as one.
fn open_sides(mesh: &solidcraft_geom::Mesh) -> usize {
    let key = |i: u32| {
        let p = mesh.positions[i as usize];
        [p.x, p.y, p.z].map(|c| (c * 1e3).round() as i64)
    };
    let mut sides: HashMap<([i64; 3], [i64; 3]), u32> = HashMap::new();
    for t in &mesh.triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *sides.entry((key(a), key(b))).or_default() += 1;
        }
    }
    sides.keys().filter(|(a, b)| !sides.contains_key(&(*b, *a))).count()
}

/// The sphere's equator lies in the box's top and its circle on the box's side x = 22.5 dips to
/// z = 24.7, while the edge of the sphere's bottom patch bulges 0.15 mm past that side: a
/// face of about 0.35 mm² between the two. At 1 % of the body it had no triangle.
#[test]
fn a_lens_thinner_than_the_tolerance_is_meshed() {
    let a = sphere(Vec3::new(20.0, 10.0, 27.5), 3.75).unwrap();
    let b = box_solid(Vec3::new(15.0, 0.0, 22.5), Vec3::new(22.5, 20.0, 27.5)).unwrap();
    let union = boolean(&a, &b, BoolOp::Union).unwrap().unwrap();
    assert!(union.validity().is_empty(), "{:?}", union.validity());
    let size = union.size();
    for tol in [size * 1e-2, size * 2.5e-3, size * 1e-3] {
        let mesh = union.tessellate(tol).unwrap();
        let faces: std::collections::HashSet<u32> = mesh.tri_face.iter().copied().collect();
        assert_eq!(faces.len(), union.face_count(), "faces with triangles at {tol}");
        assert_eq!(open_sides(&mesh), 0, "open sides at {tol}");
    }
}
