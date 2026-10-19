# Perspective UV differential reference

The independent L1 helper in #1346 supports the first wood consumer #1345.
`solidcraft_render::texture_interpolation::PerspectiveUv` accepts three unwrapped
vertex UVs and positive reciprocal clip-w values. `evaluate` takes the existing
rasterizer's screen barycentric weights and their x/y derivatives. It returns UV
and analytic perspective-correct derivatives without changing raster coverage,
projection, clipping, local mm mapping, instance transforms or resource ownership.
No pending helper branch, new dependency, file asset or GPU code is required.

Vertex UV magnitudes are at most one million repeats. Reciprocal-w is finite in
[1e-12, 1e12]; only a common scale is removed, preserving relative perspective.
Weights must be finite in [-1e-9, 1+2e-9] and sum to one within 1e-12. The lower
bound matches the existing raster's near-edge tolerance; the upper bound follows
from two weights at that lower bound and partition of unity. This admits bounded
near-edge extrapolation and preserves the exact supplied weights, including small
negative UV results. It neither clamps weights nor replaces the caller's shared-edge
top-left decision. Negative near-edge weights can make a perspective denominator
nonpositive at extreme clip-w ratios; that explicitly refuses. The caller retains
its existing raw reciprocal-w sum/depth/coverage guards before invoking the helper.
Each derivative must
be finite with magnitude at most one million and sum to zero within
`1e-12 * max(1, largest_component_magnitude)`. No weights or derivatives are
silently repaired, normalized or clamped. The caller must supply derivatives
consistent with its projected triangle; admission alone cannot prove that.

For q=1/w, the interpolation is `UV = sum(b*q*UV_vertex) / sum(b*q)`.
The helper subtracts the first vertex's UV before forming the numerator, then
adds it back to the result. This makes a constant large UV offset have exactly
zero derivatives instead of cancellation artifacts. Derivatives use the quotient
rule as `(dN - (N/D)*dD)/D`, avoiding a squared tiny denominator. Intermediates
are f64. Returned UV and derivative magnitudes must remain at most one million,
finite, or the evaluation refuses. No affine fallback hides unsupported results.

UVs remain unwrapped: do not repeat-wrap vertices before interpolation. No Y flip
or normal/tangent interpretation is added. The barycentric derivatives are per
pixel of the actual shading grid. When starting from final-image-pixel derivatives
and supersampling Sx by Sy, the caller divides x/y derivatives by Sx/Sy exactly
once. Already shading-grid derivatives must not be rescaled. This matches #1339's
LOD boundary without introducing a dependency on that unpublished implementation.

Caller responsibilities include positive clip-w after near-plane clipping,
triangle degeneracy/coverage/top-left rules, sample placement and valid derivatives.
This is a numerical reference, not a new rasterizer, clipping algorithm or proof
of that caller policy. GPU interpolation/gradient precision requires separate parity
evidence. Anisotropy, normal filtering, grain-frame inference and a visible texture
consumer remain separate work. No HDRI or final renderer capability is advertised.

Version identifier `solidcraft.perspective-uv-differential/1` documents the input,
partition, precision and refusal policy. Original analytic tests cover affine and
perspective values/derivatives, reciprocal-w scaling, vertex order, large constant
offsets, edges, supersampling units and hostile/refused outputs. This invisible
helper has no useful app screenshot; actual wood still requires PNG, CPU/GPU and
offline-reopen evidence from the installed consumer.
