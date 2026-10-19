# CPU texture reference sampler

`solidcraft_render::texture_sampling` supplies the independent numerical unit in
#1327. It does not install a texture consumer or close the wood rendering work.

Construct `TextureView::new(rgba, [width, height])` from immutable normalized float
RGBA. Dimensions must be positive and no larger than the existing admission edge
limit (2048); storage is exactly `width * height * 4` floats, at most 64 MiB.
Every channel, including alpha, must be finite and within [0, 1]. Construction
scans once and borrows the caller's storage. Sampling does constant work without
allocating. The sampler neither decodes nor converts color, and owns no cache.

The normalization contribution #1267 can provide the inputs directly through
`NormalizedTexture::rgba()` and `receipt().output_dimensions`; no normalization
implementation or dependency is copied into this module. Raw encoded RGBA8 from
the older admission utility is not interchangeable with normalized linear color.

`sample_repeat_opaque([u, v])` returns three f64 channels. Both axes use periodic
bilinear level-zero addressing. UV (0, 0) is the top-left image boundary; v grows
downwards. Texel (x, y) is centered at ((x + 0.5) / width, (y + 0.5) / height).
An integer coordinate lies halfway between the last and first texels. Negative
and unwrapped coordinates repeat; nothing clamps UVs or flips rows a second time.

Interpolation is in the supplied linear color or data space, with f64 arithmetic
on f32 input samples. The helper applies no sRGB transfer, normal-vector decoding,
tangent basis or engineering grain-axis interpretation. Alpha is deliberately
ignored for this opaque API: even zero-alpha RGB participates. It must not be used
as a transparent compositor; there is no claim of premultiplied-alpha filtering.

Coordinates must be finite and have magnitude at most 2^20 repeats. Reduction
precedes multiplication by dimensions. Around this bound, f64 input spacing is
at most 2^-32 repeats, or 2^-21 texels at maximum width. Smaller sub-ULP offsets
cannot be recovered. Tests use a 1e-9 absolute channel tolerance for analytic
fixtures; this is not a universal error bound or f32 GPU parity certification.
Consumers must reject or explicitly reconcile larger mapping ranges rather than
silently clamp them. GPU upload/filtering precision requires separate evidence.

Original generated 1x1/2x2 and rectangular constant fixtures check centers, row
orientation, linear black/white mixing, interpolation, both seams, signed/integer
repeats, alpha policy and hostile storage/coordinate refusal. No third-party
assets or code are added. This invisible helper has no useful app screenshot.

Still required for visible wood: typed bindings and persistence, local millimetre
mapping with one inverse-instance transform, resource lifetime/budgets, product
CPU/GPU integration, minification/mips, PNG and offline reopen evidence. Bilinear
level-zero filtering alone does not prevent aliasing of oblique/minified grain.
HDRI, glass, final integration and mechanical wood properties are separate work.
