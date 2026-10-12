# SolidCraft roadmap

Status as of **2026-10-11**. SolidCraft supports parametric modelling (sketch →
features → parametric timeline) as an open, pure-Rust, agent-drivable application.

## At a glance

| Question | Answer |
|---|---|
| Where are we? | **M0 and M13 done; M1, M2, M5–M7, M9, M10, M14 in progress.** Sketch with solver and the full SKETCH create/modify set; extrude, revolve (two sides, symmetric, to object), fillet/chamfer (variable radius, chord length, a radius per edge, corner patches), holes, threads (cosmetic and modelled), patterns, mirror, shell (inside/outside/both), draft, loft, sweep, coil, split body and split face, delete face, construction planes, axes and points; persistent naming of faces and edges; parameters and configurations; components, joints, motion studies, exploded views; sheet metal and a plastic enclosure subset; STEP/IGES/3MF/STL/OBJ/DXF; autosave and crash recovery; desktop and browser (wasm) app, CLI, MCP server, oracle harness. |
| Command parity (in scope) | **335 / 541 (62%)** in-scope commands; SOLID + SKETCH **188 / 291 (65%)** — [docs/parity.md](docs/parity.md); registry breadth does not establish workflow or geometric parity |
| Fusion oracle | **76 / 76 parts match** in the checked-in historical report — [docs/oracle.md](docs/oracle.md). The current sprint adds independent analytic and UI regressions; the local Fusion oracle corpus has not been rerun. |
| Tests | Solver, profiles, kernel booleans/blends/measures, expressions, timeline, persistent naming, file formats and migrations, assemblies, sheet metal, engine end-to-end, hostile-input fuzz over every command and headless UI scenarios. `cargo xtask ci` reports the current passing counts. |
| Gates | `cargo xtask ci`: fmt, clippy -D warnings, tests, asset attribution, layering, wasm32 build — green |
| Time to a useful alpha (M0–M6) | ≈ **82 agent hours** remain (sum of the M1–M6 estimates below) |

## Scope (owner decision, 2026-10-07)

The issue batch adds preservation of blind pockets in through cuts, associative projected
offsets and plastic face placements, XZ (+Y normal) with a v6 migration preserving older
designs, interactive angle placement, file-picker sketch imports, Home/recent controls,
toolbar overflow, and the remaining component-frame and assembly WIP work. The enclosure
acceptance scenarios now include board resizes; STEP reopening is automated. Inspection in
Fusion remains an external acceptance step. See [WIP reconciliation](docs/wip-reconciliation.md).

In scope, in priority order:
1. SOLID and SKETCH (now).
2. Parameters / functional constraints (done: see `parameters.*`, `expr.evaluate`; unit-aware,
   cycles reported with their path, rename follows references, deletes list their users):
   user parameters with units and expressions (+-*/, ^,
   parentheses, sin/cos/sqrt/min/max/floor/ceil/round, unit conversion), model parameters
   d1, d2… for every dimension and feature input, references between parameters with a
   dependency graph and cycle detection, re-evaluation on change, comments, favourites,
   CSV/JSON import and export.
3. Components and assemblies with joints (M9).
4. SHEET METAL: flange, contour flange, bend, unfold/refold, flat pattern with DXF export, sheet
   metal rules (thickness, K-factor, bend radius) (M10).
5. A small PLASTIC subset for enclosures, after sheet metal: Boss, Rib, Web, Lip/Groove, Snap Fit.

Deferred until further notice: MESH, FORM (T-splines), SURFACE beyond what solids need, PCB / 3D
PCB / Package, Render, Animation, Simulation, Manufacture/CAM, Drawings. The parity report's
headline counts in-scope tabs only and lists the deferred ones separately.

## Milestones

| # | Milestone | Status | Hours left |
|---|---|---|---|
| M0 | Vertical slice | done | — |
| M1 | Sketch depth: command coverage implemented — projection/intersect/include as linked geometry (auto-project on faces, lost-reference warnings, break link), full SKETCH create/modify set (slots, polygons, tangent arcs/circles, ellipse, fit/control splines, conic, text, trim/extend/break, fillet/chamfer, offset, mirror, patterns, move/scale, blend curve, centerline), constraints incl. curvature and polygon with over-constraint refusal, driven/arc-length/linear-diameter dimensions, AutoConstrain, DXF/SVG insert, 3D sketch curves (Project To Surface, Intersection Curve, Isoparametric Curve, Spun Profile), exact free-form profiles (B-spline/NURBS edges, any spline degree, curves cut by crossings), sweeps and pipes along 3D sketch curves, canvases and decals, curvature combs. Exact free-form Extend and certified associative offsets now preserve geometry and IDs through edits, deletion and reopen. Occurrence-frame 3D wires now store explicit coordinate spaces and retain points and analytic fits through moves, refresh, undo and reopen. Remaining workflow repairs: mixed offset junctions, legacy natural-spline seams, and damaged legacy wire links lacking frame provenance | in progress | 4 |
| M2 | Robust modelling: coplanar booleans in the kernel, holes (placed on faces and on sketch points), threads (cosmetic, then modelled), patterns, mirror, shell, draft, split, construction geometry | in progress | 16 |
| M3 | General fillets and chamfers (curved edges, chains, vertex blends); saddle rolling-ball surfaces now use continuous seam sections and arc-length fitting with shared contact boundaries and bounded accuracy checks | in progress | 30 |
| M4 | Files: STEP import (done: all 29 Fusion STEP files — [docs/step-import.md](docs/step-import.md)), 3MF export and 3MF/STL import as mesh bodies (done), DXF sketches, OBJ import | in progress | 6 |
| M5 | Persistent naming of faces and edges (plan/adr/0002): faces named from their history (sketch curve sides, caps, revolve sections, primitive sides, blends, pattern/mirror copies; split pieces numbered), edges by their faces; fillets/chamfers, sketches on faces, shell/draft/offset/replace faces, hole placements, joint origins and face appearances resolve by name first and warn when the entity split or vanished: done. sheet metal edges by base sketch line and flange side, shell inner faces, sweep and loft roles: done. Left: kernel-reported face history (exact through coincident surfaces) | in progress | 20 |
| M6 | UI depth: selection feedback and origin (done), timeline editing (done: hover and selection highlight modifier faces, including fillets and chamfers; edit feature, history marker drag, reorder with dependency checks, suppress, rename, delete with dependents, re-resolved references; redefining selections refreshes persistent edge/face references while retaining feature identity and dimension names), right-docked dialogs (done), live previews for every feature dialog (done), keyboard-first commands with on-canvas value boxes and sketch inline dimensions (done), drag manipulators with automatic join/cut (done: extrude incl. two sides, fillet, shell, hole depth, move, pattern spacing, revolve rotator; taper handle open), dark theme (done), right-click menu with Repeat (done), measure tool (done), section analysis (done), selection filter (done), drag-solve of sketch points (done); appearance controls now preserve body/face/component targeting, named transparent looks, reuse/search and override removal; preferences | in progress | 6 |
| M7 | Sweep, loft (incl. to a point), rib and web (to next / depth), emboss (planar faces and exact radial emboss/deboss of bounded orthogonal profiles on complete exterior cylinders/sleeves), thread (cosmetic, and modelled M3–M20 outside and inside: the ISO profile screwed in as a faceted cut), coil (smooth B-spline helical walls), pipe, pattern on path, patterns of bodies and components, replace face (parallel), align, remove, feature copy/paste: done. Unique eligible cylindrical targets are inferred for new Emboss features. Left: general curved-face emboss and exact (unfaceted) modelled threads | in progress | 12 |
| M8 | Surface workspace | deferred | — |
| M9 | Components and assemblies: New Component / Create Components from Bodies, active component, per-component origin/bodies/sketches/timeline entries, occurrences with transforms, Copy/Paste vs Paste New, nested Browser tree with visibility and isolate; Ground, Move/Copy of occurrences, joints (rigid, revolute, slider, cylindrical, pin-slot, planar, ball) by joint origins with limits and Drive Joints, As-built Joint, Rigid Group, Contact Sets (driving and motion studies stop where bodies in a set would pass through each other), Motion Study (joint values keyed along a timeline, playback, export of occurrence positions as JSON/CSV), Exploded views (automatic or by hand, shown without changing placements); interference, per-component physical properties; STEP with assembly structure. Physical properties now measure material-aware component/occurrence placements, descendants and repeated instances, including pending moves and body reassignment offsets; contact geometry and automatic exploded centres respect those offsets. Groundwork done: component tree, occurrences with transforms (instances, Paste New), world placement, active component as authoring frame, ground, Capture/Revert Position, moving bodies and sketches. UI: ASSEMBLE panel dialogs for New Component, Joint (snaps on faces, edges, circles and vertices; animated motion preview; offsets, flip, limits), As-built Joint, Joint Origin, Rigid Group, Drive Joints (live slider), Motion Link and Interference (overlaps shown red); joints in the timeline and the Browser | in progress | 32 |
| M10 | Sheet metal: flange (base, edge, contour), hem, unfold/refold, convert, flat pattern + DXF, rules: done (oracle 67-76). PLASTIC enclosure subset: Boss, Rib, Web, Lip/Groove, Snap Fit, Rest, Plastic Rules (library, edit, assign; rule draft and clearance as feature defaults): done, no Fusion oracle (licence), tested against hand-computed geometry. UI: SHEET METAL and PLASTIC tabs in Fusion's layout with dialogs (live preview, Edit Feature, pre-selection) for Flange, Hem, Unfold/Refold, Convert, Sheet Metal Rules, Create Flat Pattern (flat view with bend lines, bend table CSV/JSON and DXF export with exact circle/arc/line cutouts), Boss, Lip/Groove, Snap Fit, Rest, Manage and Assign Plastic Rules; Fold (bend along a sketch line) | in progress | 36 |
| M11 | 2D drawings | deferred | — |
| M12 | Release: cross-platform packages, verification and deliberate publication implemented; original current-UI demo refreshed. Trusted distribution signing and broader performance work remain | in progress | 16 |
| M14 | Configurations: a table of rows (parameter expressions and feature suppressions per row) switched with config.activate (undoable, all or nothing), kept through parameter renames and feature deletes (config.configure, config.table, config.column/row/activate). Per-row body materials and appearances (library looks, custom colour/opacity and clearing overrides), undoable activation and save/reopen: done. Ordered typed configuration rules, atomic validation and structured rule editing: done. Component appearance and certified semantic face style columns, atomic preview/apply and independent-copy remapping: done. Exact constructed sketch-side and immutable imported-face lineage now preserve legacy names and reject retargeting. Left: ambiguous shifted/two-sided caps | in progress | 4 |
| M13 | Never lose work: atomic saves (temporary file, flush, rename; a crash at any moment leaves the old file or the new one) with the replaced version kept as `<file>.bak`; autosave of unsaved changes to the recovery folder every 5 minutes (preference) and after big operations, only when something changed; "Recover unsaved design?" on the next launch (`doc.recovery_list`, `doc.recover`, `doc.recovery_discard`); a kill-mid-save test (process killed while saving a 0.5 MB design 25 times). File format: `"format": "solvecraft/12"`, older formats upgraded step by step (`doc::format`), newer ones refused with a message, out-of-range numbers and broken placements refused; a corpus of designs saved by M0, components-era, joints-era and current builds (apps/solvecraft-cli/tests/corpus) that must open and evaluate as saved; damaged and hostile variants (truncation, bit flips, hostile values, removed keys, repeated features, deep nesting) never panic | done | 6 |

## Dimensioning and error reporting

- Single-line and rectangle-side dimensioning supports selecting a line or its two endpoints.
  Type a value and press Enter to commit one undo step; rejected values remain editable and
  cancellation leaves the sketch unchanged. Fit keeps geometry clear of the Sketch Palette
  and navigation controls, including enlarged displays and nested pending placements.
- Contextual error reporting: a **Create Issue** action beside feature errors opens a
  reviewable GitHub issue draft with the error, operation settings, build diagnostics and
  useful reproduction context. The draft is editable, with optional capture of the actual
  viewport and copy/save controls. Opening GitHub does not submit the issue. Help also exposes
  reporting for unexpected results that did not produce an error.
- The header exposes **Report a bug**, including an accessible compact control on
  narrow windows. It opens the same editable report and screenshot review without
  changing the active model or a pending dimension.

## 0.2.0 release batch

- Includes the new axis navigation, adaptive sketch drawing/picking, origin-corner repairs,
  durable supported profile/face references, endpoint dimensioning and contextual reporting.
- Smooth ruled-wall display joins are hidden only within the supported geometric certificate;
  authored splits and actual creases remain protected. Wireframe retains physical edges.
- Paste New uses a bounded current-geometry comparison for supported convex line/circle
  prisms with separated circular holes; unsupported shapes retain the existing strict check.
- Common-bore Union retries check additional boundary curves before recutting the bore,
  preventing the reproduced thin-web loss. Unknown additional trims refuse; inherited
  common-cylinder trims retain their existing policy, so this is not a general Boolean proof.
- Exact full-path assembly joint editing remains separate work and is not part of this batch.
- Stable 0.2.0 is published: all release payload checksums, native cold reopening, deployed
  web app and both Flatpak architectures were verified. The replacement README demo uses
  the original electronics enclosure story and animated pointer in the actual English
  0.2.0 UI, at 4× playback. Publication follows [the release workflow](docs/releasing.md).

## 0.2.1 critical fixes

- Axis view and navigation controls stay anchored to the viewport when feature dialogs or
  the command palette open, at normal and enlarged display scales. Fit still accounts for
  dialog coverage; covered controls do not receive clicks through dialogs.
- Eligible planar faces survive fillet, chamfer and shell operations with exact source
  certificates. Sketches placed directly on the enclosure wall and inner floor follow
  width and wall changes, including after the USB cut and cold reopening.
- Remove artificial straight-wall and rounded-corner subdivisions left by shell and
  extrusion cuts. Identical generated wall segments span adjacent slabs as one physical
  face, with exact full-height supports and boundary stitching. Preserve authored face
  splits and face appearances. Permanent regressions cover material, STEP, reopening and
  unaffected bodies across the extrusion end plane.
- Feature dialogs use the available screen height for their fields, with OK and Cancel
  outside the scrolling region, including enlarged display scales.
- Design format 13 records shell-face attachments. Older designs remain readable;
  newly saved designs require 0.2.1 or later. Ambiguous or unsupported faces still refuse
  attachment rather than guessing a plane.
- Saved projections from a removed strip retain their geometry and report the lost
  reference. Explicitly reselect the wall to repair that projection.

## View navigation

- Compact clickable signed X/Y/Z axis tips, smooth drag-to-orbit, and Shift-orbit snapping
  to 26 standard and diagonal views. Pointer motion accumulates independently of a lightly
  damped spring, with a boundary margin to prevent flicker. Z-up/Y-up and the nearest square
  top/bottom heading are preserved. Diagonals include 45-degree edge and isometric corner views.

## Known limitations

- Associative shell-face sketches require a unique certified planar source. Nested shell
  attachments and ambiguous coincident sources remain unsupported.
- Fillets and chamfers: constant, variable-radius and chord-length fillets; a radius per edge,
  rounds next to earlier rounds of another radius, and smooth corner patches where three rounds
  meet (a sphere for equal radii); straight edges (convex or concave) between planar faces with perpendicular
  planar end faces (equal-distance chamfers also meet sloped planar end faces, including inner
  basin rims within the rim width); convex fillets also meet sloped planar ends and reflex
  corners around pocket rims, with unequal wall slopes and extensions confined to the face;
  connected equal-distance chamfers also trim each other across unequal sloped rim walls;
  whole smooth loops of a planar face, picked whole or by one edge (tangent
  chain), including circles and walls at any angle (boss bases and tops, bores, pocket floors,
  plate outlines), loops with sharp corners between straight edges (the blends meet in mitres), runs of
  neighbouring edges along a planar face's loop (two top edges of a box: mitred, ending square;
  with the edge between them too, the corner becomes a sphere octant),
  rounded corners rounded again (sphere octants, tori at concave corners); every edge of a
  convex planar body or of an extruded part; fillets inside patterns; closed chains of curved
  edges between any two smooth sides (a branch pipe on a main pipe), by a rolling ball (the
  blend a rational B-spline through exact arcs), closed or ending at planar faces square to the
  edges.
- Booleans: prisms along one direction (plates with holes, slots, patterns of them, equal
  sections stacked end to end) are combined exactly in 2D. Qualified axial cuts preserve
  annular holes with overrun cutters, including inward Press Pull after a joined extrusion;
  curved wall qualification rejects false transverse axes instead of filling holes.
  Bodies touching along one whole face
  (a part and its mirror image) are stitched; coincident planar faces are pushed apart when
  their neighbours are planes or cylinders. Other coincident curved configurations can still
  fail. Bounded containment classification preserves fully internal cavities and nested
  material islands independently of shell order, with STEP round-trip and material-membership
  regressions. Near-contact curved shells use the general boolean path rather than a
  containment shortcut. Spheres are built from six pole-free patches.
- Mass properties come from fine tessellation (curved faces within ~1e-4 relative); triangles
  that cut through curved faces (trimmed faces after booleans) are split onto the surface first.
- STEP import (our own reader, crates/kernel/src/step_in; all 29 Fusion STEP files pass
  `cargo xtask step-corpus`) reads solids (B-rep with analytic and B-spline geometry), units,
  product names, colours and assemblies; analytic offsets on planes, cylinders, cones, spheres
  and regular tori are read exactly (signed/nested distances, units and face sense). Freeform
  offset surfaces and pcurve-only edges are not read yet,
  and assembly components are flattened into bodies. Closed periodic faces (torus, B-spline
  bands) are split in two for meshing; measure() uses a finer tolerance for small radii.
- Persistent naming (plan/adr/0002) names faces from their history and edges by their faces;
  references resolve by name first and warn when the entity split or vanished. Kernel-reported
  face history (exact through coincident surfaces) is not wired in yet.
  New planar sketch attachments use exact box, extrusion cap/side and immutable imported-face
  source certificates with checked target and author occurrence paths. A missing or changed
  source fails explicitly; Redefine Sketch Plane previews reattachment or a fixed plane while
  retaining current geometry and dimension identities. Unsupported modified faces need an
  explicit fixed plane. Legacy attachments retain their original resolver behavior.
- Modelled threads are cut as a faceted body (curved faces within 1/200 of the pitch); Extrude
  and Revolve To Object on a curved face cut the tool back to the target's surface.
- Explicit sketch breaks preserve exact ellipses, fit/control splines (including high degrees)
  and conics; analytic trim/break also sees free-form cutters. Free-form curves cut at a tangent
  touch (no clean crossing) enter that profile as polylines.
  Sweeps along 3D sketch curves follow their polyline (smooth walls when the path is smooth).
- Extend intersects exact ellipses and rational/high-degree splines without moving the line or
  circular support. Associative free-form offsets have a whole-span 0.002 mm normal-distance
  certificate and refine within bounded node limits while retaining existing IDs. Singular,
  self-intersecting and ambiguous curves refuse atomically; mixed junctions and nonsmooth
  closed natural-spline seams remain limitations. New explicitly closed fit splines and closed
  offset outputs use periodic C2 interpolation, supporting repeated certified offsets; older
  natural splines are preserved. Clicking the starting fit point deliberately closes a new loop.
  Newly captured adaptive profile references with complete source coverage retain their outer
  and hole identities through span-count changes, including parameter edits after extrusion.
  Partial, intersected or ambiguous boundaries do not qualify for these whole-source references.
  Legacy curve-span multisets remain exact and unchanged; already-lost selections may need
  reselection. Format 12 stores the new bounded whole-boundary references.
  Projected analytic boundaries follow nested loop identities through resizing, including
  stationary holes and rounded corners. Supported convex, tangent line-and-arc boundaries
  can lose or regain corner arcs during an inset while preserving reserved curve identities
  and linked profile selections. Format 12 saves those associations; compatible older closed
  offsets upgrade structurally without changing their curve IDs. Breaking an association
  freezes its current boundary and materializes downstream profile references. Child offsets
  of a boundary with retired arcs still require breaking its association first; mixed or
  nonconvex junctions and retained references to disappearing individual arcs remain limited.
- Extrude and Revolve editors retain original direction, extent expressions, explicit targets,
  axes, From/To inputs and All-profile intent. No-op edits preserve downstream dimension names
  and selected occurrence frames; cancel and successful edits preserve one-step undo history.
  Placed feature edits preview through the edited timeline entry, and cylindrical Emboss guides
  use the sketch's actual target, author or uniquely placed third-component frame.
  Revolve rotators follow the actual signed axis and support symmetric and two-sided extents;
  dragging a To Object extent converts its derived value to an explicit angle. Legacy reversed-axis
  symmetric geometry is preserved through format 11's explicit legacy convention. New revolves
  use the sketch-normal direction for each selected profile, including To Object and curved
  target trimming, independently of the selected axis sign.
- Cylindrical emboss/deboss wraps bounded orthogonal line profiles, including holes, concave
  outlines and nested islands, onto complete exterior cylinders or coaxial sleeves with exact
  radial walls. New features infer a unique eligible cylinder when the face field is empty;
  ambiguous targets require an explicit face. Saved target
  and author occurrence paths retain the chosen placements; missing/split faces refuse rather
  than retarget. General free-form profiles, seams, stepped cylinders and other curved surfaces
  remain unsupported. STEP round-trip and independent radial/volume checks cover this subset.
- Shell and offset faces work on bodies of planes, cylinders, cones and spheres (convex or not:
  filleted boxes, bosses, domes, countersinks; corners where four or more planes meet, such as a
  floor falling to a drain, as long as the corner's edges are all convex or all concave); draft on convex planar bodies and on walls (planes, cylinders → cones) between caps; lofts through three or more sections are smooth (B-spline sides through every section), two
  sections ruled.

- Sketch corner tools: fillet and all three chamfer modes pick a connected corner even when
  its endpoint overlaps the fixed origin or a loose point. Ordinary drawing still snaps to
  the origin; clicked UI regressions verify geometry, closed profiles and undo.
- Sketch display: drawing, picking, highlighting, tool previews and profile shading share
  adaptive projected curve boundaries. Fit, zoom, display scale and placed sketch frames
  invalidate the display cache without changing CAD geometry or timeline history. Work
  budgets and clipped paths prevent unbounded sampling or invented edges across the eye plane.
  The Sketch Palette scrolls its options on short windows and enlarged displays while
  keeping constraint status and Finish Sketch visible.
- Face projection: closed boundaries have a deterministic outer-first order. Refresh matches
  existing geometry before assigning positions, retaining saved curve IDs and constraints
  across enumeration changes and resize, including concentric circles. Ambiguous identities
  retain the last geometry and report a lost reference; permutation matching is bounded.
- Inline sketch dimensions: Tab and Shift+Tab cycle within the active shape's value boxes,
  selecting visited values for replacement. Expression completion retains forward-Tab
  priority; real keyboard regressions cover rectangles, lines and single-value circles.
  Typed lengths, radii, line angles, signed arc sweeps and slot lengths persist as constraints
  and expressions through save/reopen. Shape creation and its dimensions form one undo step;
  rejected values keep the editable field focused.
- Repeated components: Browser selection, visibility, properties, copy and Move/Copy use
  checked occurrence paths. Active nested occurrences author planar sketches and solids in
  their selected frame. Pending moves compose through parents; dialog cancellation restores
  its selected move. Source materials and body/face appearances follow transformed instances.
  Shared nested definitions remain shared. Deep Paste New copies reachable definition graphs,
  independent feature/input histories, styles and internal motion records, with bounded geometry
  checks and explicit external-link notices. Paste New selects the copy while preserving the active
  authoring context. Imported periodic STEP seams survive placement and reexport with shared
  topology and appearances intact. Recorded body transfers preserve historical Join/Cut membership,
  material, placements and machining data through edits, copying, undo and reopen. New 3D wires
  record their coordinate frame; legacy wires migrate from known provenance without changing
  coordinates. Broken legacy links with missing provenance can require explicit refresh or relink.
  See [occurrence workflows](docs/occurrence-workflows.md).
- Parameters and Configurations have distinct actions in the SOLID toolbar's CONFIGURE panel.
  Parameters opens the user/model variable editor; Configurations opens the configuration table.
  Configure remains available in the panel dropdown.
- Parameter creation supports Enter with validation and continued keyboard entry. Shared
  expression autocomplete supports Up/Down selection, Enter/Tab acceptance and Escape dismissal;
  accepting a suggestion does not submit a parameter or trigger viewport shortcuts.
- Movable window title bars follow the app's dark/light header palette. Radial right-click
  menus are opt-in in Preferences > Navigation; ordinary context menus remain the default.
- Desktop distribution: running Windows windows and executable resources use the app icon;
  packaging validates icon resources. Nix installs launchers, MIME data and generated icons,
  with explicit launch apps and Vulkan-loader/driver discovery. Flatpak feed instructions
  register the required runtime source; release checks exercise desktop modelling on X11/Wayland.
- OpenGL rendering clamps effective anti-aliasing to 1x across UI pipelines, textures and CAD
  callbacks to prevent black frames. Other backends retain the requested sample count, and
  stored preferences remain unchanged.
- Configuration rules: ordered typed conditions can drive parameters and suppressions or
  restrict values. Preview explains effective values without modifying the document; applying
  rules validates rows atomically. Length expressions retain their authoring units across
  document-unit changes. The UI supports structured All/Any/Not conditions and rule ordering.
  Component styles and certified semantic face styles are available; unsupported lineage refuses
  atomically, and stale certified face paint cannot jump to another surface. See [configurations](docs/configurations.md).
- Sheet-metal material: inherited holes, crossing edge notches, overlapping cuts and retained
  islands share the same boundaries in flat preview, DXF and unfolded solids. Detached material
  pieces survive unfold/refold as one logical sheet group, including split flanged sheets.
  Circular cuts crossing bends remain unsupported; free-form bases use bounded sampling.
  Parallel prismatic partial-depth cuts retain pocket side and depth through transfer and
  unfold/refold, with separate pocket contours in preview and DXF. Planar Intersect preserves
  exact holes and retained islands, including the union of multiple selected profiles, and
  records complementary depth layers as pockets while retaining the stock gauge. Unsupported
  tapered or bend-crossing machining refuses atomically instead of restoring material later.
- Rolling-ball fillet accuracy: independent offset-cylinder tee references check surface
  distance, actual trimmed tessellation, supporting-cylinder contacts and tangency at two
  scales. The measured trimmed-part error is below 0.0001 mm; this is an analytic geometry
  check, not a new Fusion oracle result. Under-resolved fits return an error.
- Contact driving checks the trajectory between clear endpoints, including linked joints,
  playback and sparse motion exports. Checks use bounded tessellation samples, not exact
  continuous collision detection; local thin walls and small details remain limitations.
  See [contact-set limits and recovery](docs/contact-sets.md).

## Document-opening performance

- Cold rebuild profiling separates parsing, geometry, placement, display meshing and
  thumbnails. Straight-line closest-point projection and exact curved-surface evaluation
  reuse reduce repeated work without relaxing geometry checks. Three alternating optimized
  benchmark runs reduce median rebuild time by 26% on the public sensor enclosure and 22%
  on a local case assembly; shape measurements and topology match the original build.
  See [performance measurements](docs/perf.md). Further cold Boolean and import costs remain.
- Prepared assembly geometry survives selection and view changes while real geometry and
  placement changes invalidate it. A Pi assembly edge-selection probe drops from 9.24 s to
  0.57 s; initial opening is unchanged. Actual Windows desktop profiling measures 15.87 s
  median opening, with 82% spent rebuilding geometry; halving full opening remains open.

## Kernel

- Boolean intersection-curve construction refuses point-only, collapsed and non-finite
  polylines without panicking. Direct constructor regressions retain valid open endpoints
  and closed spherical intersections; the original interactive trigger remains unidentified.
- Shell of a stepped, joined planar body no longer creates horizontal subdivisions across
  continuous walls. Fresh axis-aligned Cut walls are stitched using their exact construction
  support and shared topology, preserving real steps and intentional boundaries. The local
  reported case returns from 86 to 49 faces with unchanged material and bounds; regressions
  cover stepped stock, STEP round trips and saved projections from removed pieces.

truck (Apache-2.0) behind our own `kernel` crate, with our own blend operation and boolean
retry/validation; see `plan/adr/0001-geometry-kernel.md` (local) — summary in README.
