use super::*;

fn bits(v: Vec3) -> [u64; 3] {
    [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
}

fn check_body(body: &Body) -> usize {
    let meshes = mesh_shells(&body.solid, 0.5);
    let mut changed = 0;
    for (face, meshed) in body.solid.face_iter().zip(meshes.iter().flat_map(|shell| shell.face_iter())) {
        let Some(mesh) = meshed.surface() else { continue };
        let surface = face.surface();
        let cached = refined_with_cache(&mesh, &surface, 0.002, true);
        let original = refined_with_cache(&mesh, &surface, 0.002, false);
        match (cached, original) {
            (None, None) => {}
            (Some(cached), Some(original)) => {
                changed += 1;
                assert_eq!(cached.len(), original.len(), "triangle count/winding must not change");
                for (cached, original) in cached.into_iter().zip(original) {
                    for ((point, normal), (old_point, old_normal)) in cached.into_iter().zip(original) {
                        assert_eq!(bits(point), bits(old_point), "every refined coordinate must remain bit-identical");
                        assert_eq!(bits(normal), bits(old_normal), "every refined normal must remain bit-identical");
                    }
                }
            }
            _ => panic!("memoization changed the refine/no-refine decision"),
        }
    }
    changed
}

#[test]
fn exact_refinement_cache_preserves_periodic_seams_and_normals() {
    let cylinder = crate::cylinder(Vec3::ZERO, Vec3::Z, 15., 8.).unwrap();
    let torus = crate::torus(Vec3::ZERO, 18., 4.).unwrap();
    assert!(check_body(&cylinder) > 0, "the fixture must actually refine");
    assert!(check_body(&torus) > 0, "the periodic fixture must actually refine");
}

#[test]
fn exact_refinement_cache_preserves_singular_poles_and_placed_surfaces() {
    let sphere = crate::sphere(Vec3::ZERO, 12.).unwrap();
    assert!(check_body(&sphere) > 0, "the polar fixture must actually refine");
    let placed = crate::transform(&sphere, Vec3::new(120., -85., 40.), Vec3::ZERO, Vec3::new(1., 2., 3.), 0.371).unwrap();
    assert!(check_body(&placed) > 0);
}

#[test]
fn exact_refinement_cache_keeps_unnecessary_planar_refinement_absent() {
    let body = crate::box_solid(Vec3::ZERO, Vec3::new(10., 20., 30.)).unwrap();
    assert_eq!(check_body(&body), 0);
}
