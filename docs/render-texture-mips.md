# Opaque CPU mip reference

The numerical follow-up #1332 reuses the reference sampler in #1327 through
`solidcraft_render::texture_sampling::mips`. It adds no installed renderer,
resource storage, normalization, file decoder, worker, cache or GPU capability.
The visible wood consumer remains separate work.

`MipChain::prepare(base, max_tail_bytes, cancelled)` borrows a validated
`TextureView` as level zero. Each following level has dimensions
`[max(1, width / 2), max(1, height / 2)]` until 1x1, at most 12 levels including
the base. All derived levels are immutable and allocated through fallible reserve.
The base is not copied. `level` borrows a prepared view without another whole-image
admission scan; `dimensions`, `level_count` and `tail_bytes` expose its accounting.

The caller chooses a derived pixel capacity budget no larger than 32 MiB.
The complete required pixel size is checked before work, and retained Vec capacity
is checked before filling each level. A maximum admitted 2048x2048 base requires
22,369,616 derived pixel bytes. The borrowed base (up to 64 MiB), small bounded
level metadata, allocator overhead and other normalization/preparation allocations
are separate. This is not a total process-memory bound. The existing resource host
must count both the retained base lease and derived levels against its cache budget.
`retained_bytes()` includes the actual pixel and level-vector capacities and the
chain value. The host must also budget allocator overhead conservatively; this
helper's cap is not an additional allowance outside #1237's shared preparation pool.
Keep the borrowed base alive through its existing prepared-result/lease.

`MIP_RECIPE_ID` is `solidcraft.opaque-linear-area-mips/1`: include it in preparation
and cache identity alongside source digest, role, normalization policies and host
generation. This recipe fixes floor-halving, linear area filtering and opaque alpha.
It must not alias level-zero-only output, a normal filter or another alpha policy.
Future changes to those semantics need a new recipe identity.

Every destination texel is an area average over its equal-sized cell in the source
image, treating source texels as piecewise constant. Integer overlap numerators
include partial source cells. Thus odd edges are neither discarded nor given the
weight of a complete destination cell. For example, [0,0,0,0,1] reduces to [0,0.4]
then [0.2], preserving its original mean. Intermediates use f64 and each generated
level is stored in f32, so successive levels include f32 rounding. No transfer
function is applied; the input must already be linear normalized color/data.

The API is deliberately opaque: alpha is ignored in RGB filtering and generated
levels have alpha=1. This is not premultiplied filtering or transparent compositing.
Normal maps need a separate role-aware vector filtering policy and must not use
this helper as though scalar averaging produced valid unit normals. There is no
hidden Y flip, tangent basis, grain-axis inference or source orientation transform.

Cancellation is checked before work, at each level, before every destination row,
after a level and before returning the complete chain. Failure/cancellation drops
local partial results; callers receive no partial chain. Preparation is synchronous
and belongs to the existing caller-owned host. Cancellation polls do not yield the
browser event loop, schedule a worker or establish browser responsiveness.

`sample_repeat_lod_opaque(uv, lod)` applies the #1327 periodic bilinear convention
within each level and interpolates adjacent levels in linear space. LOD must be
finite in [0, level_count - 1]; out-of-range values refuse. Signed/unwrapped UVs
keep the sampler's existing finite 2^20 repeat bound. Sampling performs no
allocation or full-image validation scan. GPU/f32 parity is not inferred from
the CPU arithmetic.

LOD is supplied explicitly. Screen-space derivatives, footprint selection,
anisotropic filtering and grazing-angle grain remain consumer work; a mip chain
alone is not a proof of alias-free wood. Typed bindings, local mm mapping and
inverse-instance placement, CPU/GPU consumers, PNG/offline reopen and real native
visual evidence are still required. HDRI and final rendering are not enabled.

Original generated tests cover linear checkerboard and fractional LOD values,
area-weighted odd/rectangular means, 1x1 and single-axis images, budget boundaries,
hostile LOD/UVs and cancellation at every publication checkpoint. No third-party
assets, code or dependencies are added. This numerical helper has no app screenshot.
