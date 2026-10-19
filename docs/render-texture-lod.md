# Independent CPU LOD reference

`solidcraft_render::texture_lod::select_isotropic_lod` is the small numerical
unit in #1339, supporting the eventual wood consumer under #786. It depends only
on existing texture admission limits and the standard library; #1327/#1332 do
not need to merge first. It installs no texture consumer or renderer capability.

Supply positive base dimensions (each at most 2048) and two derivatives of
unwrapped UV per pixel of the actual shading grid: `duv_dx`, `duv_dy`. Every component must be finite
and have magnitude at most 2^20. Invalid inputs refuse before calculation. The
caller supplies perspective-correct mapping derivatives; the helper does not
infer them from mesh/world bounds, sampled UVs or neighbouring wrapped coordinates.
Differencing after repeat wrapping would turn a seam into a false large footprint.

The shading grid is where samples are evaluated, which may differ from the final
PNG or logical viewport grid. For Sx by Sy supersampling, derivatives measured
per final-image pixel must be divided by Sx for `duv_dx` and Sy for `duv_dy` before
this call. Derivatives already computed on the shading grid must not be divided
again. Choosing a final-pixel footprint for every subsample would over-filter the
image. Downsampling to the final image remains the consumer's separate filter.
The helper infers no SSAA setting, device-pixel ratio or logical display scale.

The texel-space Jacobian is
`[[width*du/dx, width*du/dy], [height*dv/dx, height*dv/dy]]`.
Its largest singular value is the footprint scale. This uses a circular screen
differential convention, not an exact integration over the square pixel area.
It is at least each individual column norm, accounting for shear that a maximum
column estimate misses. An elongated footprint can over-blur its shorter axis:
anisotropic filtering remains separate work.

The implementation scales the Jacobian before forming its quadratic and uses
f64 arithmetic. Admitted gradient/dimension bounds keep all intermediate values
finite. Zero and subpixel footprints select LOD0. Otherwise LOD is log2(scale),
explicitly saturated to the final level of a complete floor-halved chain ending
at 1x1. The result includes the unsaturated scale and maximum level. For odd
dimensions log2 remains a convention approximating non-power-of-two level ratios;
it is not a claim of exact area integration or alias-free minification.

Policy identifier `solidcraft.isotropic-singular-value-lod/1` fixes that
convention. A consumer must explicitly reconcile it with its prepared chain;
incomplete chains and different halving/filter policies cannot silently substitute.
No sampling, alpha/normal interpretation, derivative persistence, allocation,
resource cache, UI, engine command, GPU code or dependency is added. f64 reference
results alone do not certify GPU/f32 precision or visual parity.

Original analytic tests cover zero/magnification, integer and fractional LOD,
rectangular texture scaling, rotated/signed/swapped axes, a sheared Jacobian,
odd/single-axis chains and hostile/extreme inputs. This invisible numerical
helper has no useful app screenshot. The visible consumer still requires typed
bindings, local millimetre mapping, preparation ownership, CPU/GPU integration,
PNG and offline reopen evidence. HDRI and final rendering remain separate work.
