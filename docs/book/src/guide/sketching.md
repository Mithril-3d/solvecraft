# Sketching and constraints

A sketch lies on a plane: an origin plane, a construction plane or a planar face. Start one with
**Create Sketch** (`sketch.create`) and click a plane, or pick a face first. **Finish Sketch**
returns to the model.

Curves adapt to the current zoom and display scale. Picking and highlighting use the same
visible boundary, including small circles at Fit and sketches in rotated components.
Profile shading follows those curves while preserving holes. Zooming changes the display
without changing sketch geometry, dimensions, or undo history.
On short windows or enlarged displays, the Sketch Palette's options scroll while its
constraint status and **Finish Sketch** button remain visible.

## Geometry

Lines and polylines (L), rectangles (two-point R, three-point, centre), circles (centre C,
two-point, three-point, tangent), arcs (three-point, centre, tangent), polygons, slots, ellipses,
fit-point and control-point splines, conics, points and text. **Construction** (X) turns curves
into reference geometry that doesn't form profiles. Modify tools include trim (T), extend, break,
fillet, chamfer, offset (O), mirror, patterns, move/copy and scale.

**Project** (P) and **Intersect** bring model edges into the sketch as linked geometry that
follows later changes. Sketching on a face projects its edges automatically.

Run from a script or the control channel, every command that adds sketch geometry returns the
ids of what it added (`created: {points, curves, constraints, dimensions}`). Ids are stable and
never reused within a sketch, even after a delete, so a script can dimension or constrain the new
geometry right away.

Extend finds intersections with exact ellipses, rational conics and spline spans. It keeps the
original line or circular support and extends only beyond the selected endpoint; conflicting
constraints or an ambiguous overlap leave the sketch unchanged.

Closed convex chains of tangent lines and circular arcs support inward offsets that consume
a rounded corner. When the offset passes the corner radius, the neighbouring straight
supports meet at a sharp corner. The surviving lines and endpoint IDs stay stable; the
retired arcs return under their original IDs if a later edit restores their radius.
Whole-boundary profile selections follow this transition through save and reopen, so an
extruded lip can remain associative. Existing eligible closed analytic offsets upgrade
without changing their saved geometry or IDs.

This transition requires a closed convex boundary with tangent junctions and a certified
nonempty region. An extinguished arc referenced individually by a constraint or another
offset cannot retire. Creating another offset from an already sharp retired boundary also
requires breaking its association first. Open, nonconvex and mixed free-form chains keep
their existing behavior. Break Offset Association freezes the current boundary, including
private arc centres and construction flags, for independent editing.

Offsets of smooth ellipses, conics and splines follow their source after dimension changes,
dragging and reopening a design. Their stored cubic spline has a whole-span normal-distance
error bound of 0.002 mm. Singular tangents, curvature cusps, possible self-intersections and
excessive subdivisions are refused. Offset these curves individually: mixed chain junctions
currently require separate geometry. `sketch.offset.break` releases an offset for independent
editing, and undo restores its association. Older independently sampled offsets remain independent.
Smooth open and newly created smooth closed offsets can be offset again, including bounded
refinement after a source edit. New closed outputs use a periodic cubic fit with continuous
tangent and curvature at the seam. Break Offset Association preserves that interpolation,
and its closing point remains joined when dragged. Deleting an output also removes its
unused private fit points; deleting a source with a retained dependent is refused.

For explicit closed fit splines, `sketch.spline.fit_point` accepts `periodic: true` and at
least three distinct cyclic samples. It closes the loop automatically. Existing calls and
older saved closed splines retain natural interpolation; older associative closed offsets
also retain their saved natural fit. A legacy seam without a unique normal still refuses
another offset. Periodic intent is stored on each curve and survives save, reopen and Break.
While drawing a fit spline, click its starting point after three distinct samples to close
it smoothly. Right-click finishes an ordinary natural fit. Offset accepts both ellipse
handles and dense fitted curves; the second click sets the side and distance from the curve.

Enable **3D Sketch** in the Sketch Palette to draw lines, fit splines and points off the plane.
New 3D geometry follows its sketch's component placement, including the selected occurrence
and pending position changes. Click a drawn fit point to show its move triad; its arrows move
along world X, Y and Z. Scripts also supply world coordinates to `sketch.line3d`,
`sketch.spline3d`, `sketch.point3d` and `sketch.move3d`. Activate the desired occurrence before
editing an explicit sketch belonging to a component with multiple placements.

Older drawn or detached 3D wires keep their saved world positions. Included and surface-linked
wires with an explicit component marker keep that meaning after the link is broken. Old files
whose links became lost after an incorrectly scoped cross-component refresh may require a
fresh link once the source is available; their stored point values are preserved on opening.

## Constraints and dimensions

**Horizontal/Vertical** accepts a click on a line or clicks on two distinct sketch points,
including the origin, endpoints and circle centres. For two points, it chooses horizontal
or vertical alignment from their current positions. Esc cancels a pending first point.

The solver is SolidCraft's own (damped Gauss–Newton with degree-of-freedom analysis). Constraints:
coincident, horizontal/vertical, parallel, perpendicular, tangent, equal, concentric, collinear,
midpoint, symmetry, fix, curvature (G2) and polygon. **Sketch Dimension** (D) adds linear,
aligned, horizontal, vertical, radius, diameter and angle dimensions. Each is a parameter
(`d1`, `d2`, …), so it can hold an expression.

For **Coincident**, pick two points, a point and a curve (in either order), or two
distinct straight lines. Two lines join their closest pair of endpoints; the first
picked line stays in place when the existing constraints allow it. Pick endpoints
explicitly when you want to connect a different pair.

Horizontal/Vertical also applies immediately to one selected line or two distinct selected
points. Shift-click to add the second point before choosing the constraint.

For Parallel, Perpendicular, Equal, Tangent, Concentric and Collinear, select exactly two
sketch curves (Shift-click to add the second) and choose the constraint to apply it immediately.
You can also start the constraint first and then pick its curves. An incomplete or mixed
selection leaves the tool ready for ordinary picking.

Coincident accepts an exact selection of two points or a point and a curve. Midpoint
accepts one point and one line, in either selection order. Symmetry accepts two distinct
points and one symmetry line, regardless of where the line occurs in the selection.
Successful constraints clear the selection; rejected constraints preserve it.

Under-constrained geometry is blue and fully constrained geometry turns dark, and the status
shows the remaining degrees of freedom. A constraint or dimension that would over-constrain the
sketch is refused rather than applied. Drag a point to move it; the solver keeps every
constraint while you drag.

## Profiles

Closed loops become profiles automatically, including regions split by crossing lines and
islands inside outlines. Extrude, revolve, sweep and loft pick profiles.

A feature remembers a picked profile by the curves around it, not by where it was clicked:
`"profiles": [{"curves": ["l1", "l2", "a1", "l3"]}]` (or `[{"curves": ["c1"]}]` for a circle)
picks the region whose outer loop is made of exactly those curves, or the pieces of them that
other curves cut off. It is found again on every recompute, so the feature keeps the same region
when a parameter moves the geometry. Islands inside the region are its holes. When no region, or
more than one (both halves of a circle cut by a line), is bounded by exactly those curves, the
feature reports an error rather than taking another region. Profiles picked by index or by point
(`[{"point": [x, y]}]`) are stored the same way when the feature is made.
