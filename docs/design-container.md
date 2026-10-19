# Design containers, version 1

New designs use `.solidcraft`; `.solvecraft` remains a supported legacy extension. Opening detects the ZIP
local-header signature (`50 4b 03 04`) independently of the name; otherwise it uses
the existing JSON design reader and migrations. Plain JSON remains a fully supported,
self-contained input/output format. `io::write_design` keeps its JSON contract;
`io::write_design_container` is the fallible container writer.

Packages have `container.json` with `{"format":"solidcraft-container/1"}` and
`document.json` with the versioned parametric document (`solidcraft/N`, the format in `crates/doc/src/format.rs`).
The independent versions let document migrations evolve without changing blob encodings.
Existing serialized document fields are retained automatically, without a second copy
of the document schema or a deep clone of embedded payloads.

Within `document.json`:

- Import features have empty `step` and a `step_blob` reference.
- Mesh imports retain each mesh's name and optional colour, with empty `positions`
  and `triangles` plus a `mesh_blob` reference.
- Canvases retain placement, format and pixel metadata, with empty `data` plus an
  `image_blob` reference. Raw image bytes replace base64 in the package.

References are `blobs/<lowercase SHA-256>.<step|mesh|png|jpeg>`, hashed over the exact
uncompressed payload. Identical content with the same encoding is written once.
Required parts, malformed references, ambiguous embedded payloads, incorrect hashes,
invalid mesh data and unsupported versions produce errors before session adoption.

## Mesh encoding

Mesh blobs begin with the eight ASCII bytes `SCMESH01`, followed by little-endian
`u64` coordinate count and `u64` index count. These count scalar values, not vertices
or triangles. Next come that many IEEE-754 `f64` coordinates, then `u32` indices.
Both counts must be divisible by three; the byte length must match exactly. Coordinates
must be finite and at most 1e9 mm in magnitude; indices must address an existing vertex.
Names and colours remain in JSON. Signed zero and full coordinate precision survive.

The default `fast` policy stores STEP and binary meshes without Deflate for faster
saving and reading. Small document metadata uses Deflate level one; PNG/JPEG are
stored directly. An explicit `compact` policy compresses STEP and mesh payloads at
level one. Readers support both policies. This deliberately favors interactive file
operations over maximum compression; compressed STEP was slower in the storage benchmark.
Use `"compression":"compact"` with `file.save` or `file.save_as` to favor file size,
or `io::write_design_container_with_compression` from Rust.

## Bounds and recovery

Input and individual expanded parts are capped at 256 MiB. Total declared expanded
parts and resolved payloads are each capped at 512 MiB, including repeated references
in the latter budget. There are at most 10,000 entries. Duplicate/ambiguous entry names,
path traversal, ZIP64 and multi-disk packages are refused. Entries are read in memory;
no archive paths are extracted to the filesystem. Exact part lengths and ZIP CRCs are
checked, along with blob hashes. Hashes detect correspondence and corruption, not trust.

Saving retains the existing temporary-file/flush/rename protocol and `.bak` backup.
Recovery reads both older JSON and new containers. Browser OPFS, downloads and recovery
use the same container reader/writer; JSON export and Save As remain available.

## Provisional saved display

Experimental engine saves with `preview: true` can include `cache/preview.json` and
`cache/preview.bin` when the evaluated
world display meshes are already warm. Ordinary saves omit this redundant display data
to avoid extra save work; it is separate from the native B-rep cache below. Saving never triangulates a cold body for this
cache. Pending occurrence moves, active sketch edits, feature errors and oversized
previews omit it. The cache is bounded to 8 MiB, 1,024 bodies and 100,000 triangles;
its manifest is at most 64 KiB. Body names/colours and contiguous binary mesh slices
use the same lossless encoding as imported meshes. Extra cache size is optional;
it is omitted if it would put the container near the file-size cap.

The manifest includes SHA-256 of the exact document metadata (whose blob references
contain content hashes), the preview payload, `solidcraft-preview/1` and the application
package version. A version mismatch invalidates the display cache. Development builds
with the same package version may briefly show an older picture before the authoritative
rebuild replaces it; previews never supply model state. Earlier draft containers using
`solvecraft-container/1` remain readable, with their obsolete previews treated as misses.
Missing, obsolete, oversized or malformed preview data is a cache miss, not a failed
design open. All archive-wide safety budgets still apply.

Desktop Open can render these meshes on its existing file worker and show a clearly
labelled saved picture in the opening dialog before timeline reconstruction finishes.
It uses only the normal command's file snapshot, without a second file read. The picture
is approximate shading without face paint or picking. A conservative 16-million-pixel
raster budget rejects excessive overdraw before rendering, including repeated large
triangles in a small hostile cache. The picture is never adopted as editable
geometry, used for measurements, or used to seed evaluated model state. The original
design remains intact until the complete rebuilt session is ready. Cancellation or
any change of active document/revision discards both preview and completion.

Normal design reading, CLI and browser opening still rebuild authoritatively.
Native B-rep caches can supply provisional display as described below; they never
supply evaluated session state. The saved picture is not an interactive viewport.

## Seekable reader (phase 2, first increment)

`io::DesignContainerReader<R: Read + Seek>` accepts a container source such as a
file or a memory cursor. It inspects a bounded ZIP directory suffix and entry
headers, then `preview()` reads the optional display entries and document metadata.
`read_document()` consumes the reader and resolves and validates the required
source blobs through the same archive. No complete archive buffer is required.
The existing byte-based readers use this implementation too; JSON keeps its existing
reader. `read_design_file` integrates it with desktop Open using one file handle,
without buffering the complete container. Browser VFS uses a memory cursor.

Metadata used to validate a provisional cache is retained in an immutable buffer
and reused for authoritative reconstruction. Concurrent in-place writes therefore
either retain that document or fail required-part CRC/hash validation. Optional
preview success never validates source blobs or makes a document editable. An invalid required blob still
fails the later document read. ZIP directory scans can touch a bounded suffix of a
stored blob; deferral means not loading or decoding the complete payload, not zero
physical reads of its byte range. Optional cache metadata is capped at 16 MiB;
larger metadata misses the cache and uses the normal authoritative part-size budget.

## Native evaluated cache

Engine Save/Save As can include `cache/key.json` and `cache/brep.bin` when
explicitly requested with `cache: true` and geometry is warm. The default is
`cache: false`, including browser downloads, to avoid extra save time and file size.
JSON and recovery/autosave
remain independent of the native cache. Saving never forces cold placement,
evaluation or tessellation. Active sketches, uncaptured moves, errors, mesh bodies,
unsupported adapter representations and exceeded budgets omit the cache.

The manifest records `solidcraft-brep-cache/1`, the package version, the explicit
producer revision `solidcraft-truck-adapter/2`, exact document/payload SHA-256 hashes
and contiguous named body slices. The producer revision is deliberately maintained
when adapter semantics change; it is not a source-tree fingerprint or build script.
The binary data uses [SolidCraft's portable B-rep schema and codec](native-brep.md),
including topology, curves/surfaces, face-source certificates, appearances and
operation history. Truck's serialized structures are never stored. A different
kernel can implement the same schema; until its adapter is tested it conservatively
misses this producer's cache and rebuilds the authoritative document.

Native caches cap 4,096 bodies, 100,000 total faces, a 1 MiB manifest and 64 MiB
binary payload. Individual bodies have the schema/codec's additional limits.
The reader validates metadata, manifest, slices and bodies before requesting a
bounded provisional mesh. Original STEP/image/mesh blobs are resolved afterwards.
No redundant display mesh is stored by default: `preview: true` remains a separate
opt-in for already warm display meshes, which take precedence when present.

Every failure in the optional cache is a miss. Even a checksum-valid forged body
never becomes editable state: Open builds a fresh session from the document and
original blobs, then replaces the provisional picture. Picking, measurements,
persistent names and subsequent modelling use that rebuilt session. Existing
cancellation/document-revision guards also apply. The web build retains synchronous
authoritative rebuilding; portability does not imply background web evaluation.

A counting-reader regression uses a 2.6 MB stored source blob and obtains the saved
preview with fewer than 256 KiB read, then verifies the authoritative document is
identical after resolving its source. Corrupt-source and preview-miss regressions,
and truncation/seeded mutations through the combined reader, protect the boundary.
This is an IO-volume test, not a geometry or first-frame benchmark.

## Review increments and kernel independence

The implementation is split into the following review increments after the
phase-one container PR (#327):

1. Seekable IO and its read-volume/hostile-input tests (the API above).
2. A versioned SolidCraft-owned B-rep schema and bounded binary codec, tested without
   constructing a kernel body. Topology references, oriented edge uses and face
   loops, geometric domains, analytic/spline curve and surface definitions, units,
   tolerances and orientation must have explicit semantics. Preserve face-source
   certificates, persistent names and history using SolidCraft-owned identities.
   No truck serialized structs, memory layouts, pointers or opaque kernel blobs
   form part of the persisted contract. No evaluated STEP round trip is a substitute.
3. Separate adapters between the neutral schema and supported kernels. Unsupported
   geometry or provenance is a cache miss. Explicitly fitted intersection splines
   carry their fitting target and remain provisional display geometry. A neutral
   file schema alone does not prove interoperability: each adapter needs round-trip,
   modelling and naming tests; compatibility with a second kernel is unverified
   until that adapter exists. Kernel/version information belongs in producer and
   capability metadata, not in the topology encoding. Conservative cache invalidation
   is allowed across kernels while authoritative document reopening still works.
4. Integrate deferred source reading and provisional B-rep display into the open
   worker with immutable-input and document/revision guards. Rebuild authoritatively
   before editing, picking or measuring. B-rep alone is not a complete evaluated
   session: sketches, construction, placements and other non-solid state still matter.
5. Measure cold first display, time until editing, save latency, size and peak memory
   on a public STEP-based design. Include cache hits/misses, plain JSON and browser
   fallback, and report baselines and limitations in `docs/perf.md`.

Default saving must remain fast: do not force cold evaluation or meshing merely to
produce a disposable cache. Do not add redundant display meshes by default; the
existing preview remains opt-in. GPU tessellation is independent work. Continue
using format/application constants rather than source-tree hashing build scripts,
retain JSON Save As/MCP/script compatibility and the documented legacy shims, and
run standalone wasm validation on each published code revision.

## Portable thumbnail

A warm save with `cache: true` may add `cache/thumbnail.png` and its small
`cache/thumbnail.json` key. The picture is a 240 x 150 PNG, bounded to 256 KiB,
keyed by `solidcraft-thumbnail/1`, application version, exact document SHA-256 and
PNG SHA-256. It contains an isometric view of visible solid bodies with their
appearances. It is display-only and independent of the producing geometry kernel.

Saving uses only already placed bodies and cached meshes, at most 1,024 bodies,
100,000 triangles and 100,000 vertices. A preflight also caps projected pixel-box
work at four million supersampled pixels and face-color lookups at one million,
so overlapping triangles cannot make a small picture arbitrarily expensive.
It never evaluates or tessellates a cold
model for a picture. Active edits, pending moves, model errors, absent warm
geometry or exceeded limits omit the picture. Ordinary saves and browser downloads
default to `cache: false` and omit the PNG; `cache: true` explicitly opts in. Plain JSON
never contains the PNG. The same writer is used by desktop and browser saves.

The seekable thumbnail reader checks the small key, retained document metadata,
PNG size/header and hashes without resolving source blobs. The Home/Recent Files
reader tries its local picture cache, then this portable picture, then its existing
background reconstruction. PNG decoding also has dimension and allocation limits.
Missing, mismatched, truncated or malformed pictures are misses and never prevent
authoritative document opening. Embedding a PNG does not install an operating-system
file-browser thumbnail provider.

Desktop thumbnail reads seek within the file. The browser VFS supplies a bounded
in-memory buffer instead; it avoids geometry reconstruction but does not provide
range-based OPFS reads. IO checks the PNG header and hashes, while the consuming
UI performs the full bounded PNG decode.
