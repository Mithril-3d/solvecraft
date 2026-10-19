use super::history::{History, Lineage};
use super::*;
use truck_geometry::prelude::IntersectionCurve;
use truck_meshalgo::prelude::*;
use truck_topology::*;

/// Only solids consisting of faces whose surface is implemented this trait can be used for set operations.
pub trait ShapeOpsSurface:
    ParametricSurface3D
    + ParameterDivision2D
    + SearchParameter<D2, Point = Point3>
    + SearchNearestParameter<D2, Point = Point3>
    + Invertible
    + Send
    + Sync {
}
impl<S> ShapeOpsSurface for S where S: ParametricSurface3D
        + ParameterDivision2D
        + SearchParameter<D2, Point = Point3>
        + SearchNearestParameter<D2, Point = Point3>
        + Invertible
        + Send
        + Sync
{
}

/// Only solids consisting of edges whose curve is implemented this trait can be used for set operations.
pub trait ShapeOpsCurve<S: ShapeOpsSurface>:
    ParametricCurve3D
    + TrySubs
    + ParameterDivision1D<Point = Point3>
    + Cut
    + Invertible
    + From<IntersectionCurve<PolylineCurve<Point3>, S>>
    + SearchParameter<D1, Point = Point3>
    + SearchNearestParameter<D1, Point = Point3>
    + Send
    + Sync {
}
impl<C, S: ShapeOpsSurface> ShapeOpsCurve<S> for C where C: ParametricCurve3D
        + TrySubs
        + ParameterDivision1D<Point = Point3>
        + Cut
        + Invertible
        + From<IntersectionCurve<PolylineCurve<Point3>, S>>
        + SearchParameter<D1, Point = Point3>
        + SearchNearestParameter<D1, Point = Point3>
        + Send
        + Sync
{
}

fn process_one_pair_of_shells<C: ShapeOpsCurve<S>, S: ShapeOpsSurface>(
    shell0: &Shell<Point3, C, S>,
    shell1: &Shell<Point3, C, S>,
    tol: f64,
    lineage: &mut Lineage<C, S>,
) -> Option<[Shell<Point3, C, S>; 2]> {
    nonpositive_tolerance!(tol);
    // SolidCraft: robust meshing (boundaries from earlier operations lie on surfaces only
    // within tolerance).
    let poly_shell0 = shell0.robust_triangulation(tol);
    let poly_shell1 = shell1.robust_triangulation(tol);
    let loops_store::LoopsStoreQuadruple {
        geom_loops_store0: loops_store0,
        geom_loops_store1: loops_store1,
        ..
    } = crate::tr!(loops_store::create_loops_stores(shell0, &poly_shell0, shell1, &poly_shell1, tol, lineage), "loops stores");
    let mut cls0 = crate::tr!(divide_face::divide_faces(shell0, &loops_store0, tol, lineage), "divide faces 0");
    cls0.integrate_by_component();
    let mut cls1 = crate::tr!(divide_face::divide_faces(shell1, &loops_store1, tol, lineage), "divide faces 1");
    cls1.integrate_by_component();
    let [mut and0, mut or0, unknown0] = cls0.and_or_unknown();
    unknown0.into_iter().try_for_each(|face| {
        // SolidCraft: no panics (wasm can't catch them).
        let pt = face.boundaries().first()?.vertex_iter().next()?.point();
        let dir = hash::take_one_unit(pt);
        let count = poly_shell1.iter().try_fold(0, |count, face| {
            let poly = face.surface()?;
            Some(count + poly.signed_crossing_faces(pt, dir))
        })?;
        if count >= 1 {
            and0.push(face);
        } else {
            or0.push(face);
        }
        Some(())
    })?;
    let [mut and1, mut or1, unknown1] = cls1.and_or_unknown();
    unknown1.into_iter().try_for_each(|face| {
        // SolidCraft: no panics (wasm can't catch them).
        let pt = face.boundaries().first()?.vertex_iter().next()?.point();
        let dir = hash::take_one_unit(pt);
        let count = poly_shell0.iter().try_fold(0, |count, face| {
            let poly = face.surface()?;
            Some(count + poly.signed_crossing_faces(pt, dir))
        })?;
        if count >= 1 {
            and1.push(face);
        } else {
            or1.push(face);
        }
        Some(())
    })?;
    and0.append(&mut and1);
    or0.append(&mut or1);
    Some([and0, or0])
}

/// AND operation between two solids.
pub fn and<C: ShapeOpsCurve<S>, S: ShapeOpsSurface>(
    solid0: &Solid<Point3, C, S>,
    solid1: &Solid<Point3, C, S>,
    tol: f64,
) -> Option<Solid<Point3, C, S>> {
    boolean(solid0, solid1, tol, |[and, _]| and).map(|(solid, _)| solid)
}

/// OR operation between two solids.
pub fn or<C: ShapeOpsCurve<S>, S: ShapeOpsSurface>(
    solid0: &Solid<Point3, C, S>,
    solid1: &Solid<Point3, C, S>,
    tol: f64,
) -> Option<Solid<Point3, C, S>> {
    boolean(solid0, solid1, tol, |[_, or]| or).map(|(solid, _)| solid)
}

/// SolidCraft: AND operation between two solids, with where each face and edge of the result
/// came from (operand 0 is `solid0`, operand 1 is `solid1`): the same result as [`and`], and
/// its history, `None` when a face or edge of it was not recorded. The operands must not share
/// faces or edges.
pub fn and_with_history<C: ShapeOpsCurve<S>, S: ShapeOpsSurface>(
    solid0: &Solid<Point3, C, S>,
    solid1: &Solid<Point3, C, S>,
    tol: f64,
) -> Option<(Solid<Point3, C, S>, Option<History>)> {
    boolean(solid0, solid1, tol, |[and, _]| and)
}

/// SolidCraft: OR operation between two solids, with where each face and edge of the result
/// came from (operand 0 is `solid0`, operand 1 is `solid1`): the same result as [`or`], and
/// its history, `None` when a face or edge of it was not recorded. The operands must not share
/// faces or edges.
pub fn or_with_history<C: ShapeOpsCurve<S>, S: ShapeOpsSurface>(
    solid0: &Solid<Point3, C, S>,
    solid1: &Solid<Point3, C, S>,
    tol: f64,
) -> Option<(Solid<Point3, C, S>, Option<History>)> {
    boolean(solid0, solid1, tol, |[_, or]| or)
}

/// Combines the operands' first shells, then folds in their other shells one at a time
/// (`pick` chooses the AND or the OR shell of each pass). The history is `None` when a face or
/// edge of the result was not recorded; the result stands without it.
fn boolean<C: ShapeOpsCurve<S>, S: ShapeOpsSurface>(
    solid0: &Solid<Point3, C, S>,
    solid1: &Solid<Point3, C, S>,
    tol: f64,
    pick: fn([Shell<Point3, C, S>; 2]) -> Shell<Point3, C, S>,
) -> Option<(Solid<Point3, C, S>, Option<History>)> {
    let mut lineage = Lineage::new([solid0, solid1]);
    let mut iter0 = solid0.boundaries().iter();
    let mut iter1 = solid1.boundaries().iter();
    let shell0 = iter0.next()?;
    let shell1 = iter1.next()?;
    let mut shell = pick(crate::tr!(process_one_pair_of_shells(shell0, shell1, tol, &mut lineage), "first shells"));
    for other in iter0.chain(iter1) {
        shell = pick(crate::tr!(process_one_pair_of_shells(&shell, other, tol, &mut lineage), "another shell"));
    }
    let solid = crate::tr!(Solid::try_new(components_in_order(&shell)).ok(), "result solid");
    let history = lineage.history(&solid);
    Some((solid, history))
}

// SolidCraft: truck's connected components come out in hash order of face addresses, so the
// result's face order changed from run to run. Keep the shell's own order instead.
fn components_in_order<C, S>(shell: &Shell<Point3, C, S>) -> Vec<Shell<Point3, C, S>> {
    let pos: rustc_hash::FxHashMap<_, usize> = shell.face_iter().enumerate().map(|(i, f)| (f.id(), i)).collect();
    let key = |f: &Face<Point3, C, S>| pos.get(&f.id()).copied().unwrap_or(usize::MAX);
    let mut comps: Vec<Vec<Face<Point3, C, S>>> = shell
        .connected_components()
        .into_iter()
        .map(|c| {
            let mut faces: Vec<_> = c.into_iter().collect();
            faces.sort_by_key(|f| key(f));
            faces
        })
        .collect();
    comps.sort_by_key(|c| c.first().map(|f| key(f)).unwrap_or(usize::MAX));
    comps.into_iter().map(Shell::from).collect()
}

#[cfg(test)]
mod tests;
