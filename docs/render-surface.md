# Opaque surface numerical contract

First numerical slice of [#1004](https://github.com/bherbruck/solidcraft/issues/1004),
under #813/#620. `solidcraft_render::surface` is an original, device-independent
reference for the upcoming integrated consumer. It does not change existing looks,
commands, document formats, snapshots or viewport rendering. No backend advertises
PBR because this module exists. No external code, dataset or file asset is added.

## Admission and color

`OpaqueSurface::new` accepts linear-sRGB reflectance channels in [0, 1], metallic
fraction in [0, 1], perceptual roughness in [0.045, 1] and dielectric relative IOR
in [1, 3]. All values must be finite. Private fields prevent bypassing admission;
future command/deserialization adapters must call this constructor. The lower
roughness bound is an explicit preview limit, not a silently clamped perfect mirror.
These are rendering bounds, not a claim that all real materials fit this model.

`srgb8_to_linear` decodes existing color channels with the piecewise sRGB transfer,
including its linear toe. Texture roles, other source primaries, HDR radiance,
exposure, tone mapping and output encoding are separate responsibilities. Mechanical
density, elastic constants, condition-dependent measurements and material axes are
not inputs to this visual contract.

## Response and limitations

`brdf(normal, view, light)` returns RGB response in inverse steradians. Directions
point away from the surface and are normalized using max-component scaling, allowing
finite extreme input magnitudes without overflow. Zero/nonfinite directions fail;
directions tangent to or below the surface return zero. Consumers apply incident
radiance and the light cosine exactly once, then exposure/display transfer separately.

The specular term uses isotropic GGX, separable Smith masking/shadowing and Schlick
Fresnel. Alpha is perceptual roughness squared. Dielectric normal reflectance is
`((ior - 1)/(ior + 1))²`; metallic fraction interpolates it towards base color and
removes the diffuse substrate. The algebra evaluates `G1/cos(theta)` directly to
avoid unstable division at grazing angles without clamping the cosine to an epsilon.

Diffuse is Lambert weighted by `(1-metallic) * (1-F(n·view)) * (1-F(n·light))` per
channel. This symmetric entry/exit attenuation preserves reciprocity and avoids
adding unattenuated diffuse to grazing reflection. It is a conservative preview
approximation; there is no internal reflection or microfacet multiple-scattering
compensation. The model can lose energy, particularly rough metals. It is not an
exact layered BSDF, spectral conductor model or measurement-derived material fit.

No transmission, refraction, absorption, thin film, anisotropy, subsurface scattering,
texture mapping, IBL, caustics or path integration is provided. In particular, this
opaque model must not be used as an opacity-based substitute for glass.

## Evidence and remaining gates

Tests use independent normal-incidence analytic values, reciprocal direction pairs,
grazing/extreme-scale inputs, invalid admission and sRGB reference values. Deterministic
256×256 hemisphere quadrature checks white reflectance at metallic fractions
0/0.5/1, roughness 0.25/0.5/1, IOR 1/1.5/3 and view cosines 0.2/1, with a 0.025
numerical margin. This finite grid is not a universal energy-conservation proof and
does not resolve the narrowest admitted lobes. Narrow-lobe importance quadrature
and device parity remain consumer acceptance gates.

An initial half-vector-only diffuse attenuation failed the grazing white dielectric
test (integrated response 1.2324 at roughness 0.25, view cosine 0.2). The symmetric
entry/exit formulation replaces it; the test tolerance was not relaxed to accept
that result.

Next: connect this meaning through design-owned appearance definitions, existing
assignment commands and product scenes; test persistence, undo and unchanged mass;
install the actual integrated GPU/CPU consumers and record original native examples.
Only then advertise executor capabilities. Unsupported executors must refuse the
new response atomically rather than silently render the legacy scalar approximation.
No screenshots are claimed for this numerical-only slice.
