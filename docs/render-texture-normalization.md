# Explicit texture normalization (#1218)

`texture_input::normalization` is a pure-Rust, in-memory input utility, separate
from the strict RGBA8 [texture admission API](render-texture-input.md). It does not
install a renderer, command, resource format, document property or GPU texture.
The old API still refuses 16-bit samples, ICC and nonidentity EXIF orientation.

## Input, policy and completed output

`NormalizationSource::new(bytes, role, options)` retains immutable encoded storage
and admits format/storage/budget only. `decode(cancelled)` admits metadata and
completes normalization before returning an immutable `NormalizedTexture`.
Header admission is not successful decoding. No partial result is returned.

- PNG channels expand to RGB/gray with alpha at their original 8/16-bit sample
  depth; palette/sub-byte expansion uses the existing PNG codec. Output is RGBA
  f32, without an implicit RGBA8 preview. This preserves distinguishable adjacent
  16-bit raw samples; ICC conversion and floating-point arithmetic are not lossless.
- JPEG is initially 8-bit grayscale or three-component sequential/progressive DCT;
  four-component/CMYK, arithmetic and lossless JPEG are outside this policy.
  JPEG decoding is lossy irrespective of the output precision.
- Base color returns **linear sRGB** RGB and unassociated normalized alpha. A
  renderer must not apply sRGB decoding again. Alpha never receives a color transform.
- Roughness, metallic, AO and OpenGL normal channels remain normalized **data**.
  They receive no gamma correction, ICC conversion, normal renormalization, channel
  convention conversion or filename inference. ICC on a data map refuses.
- Untagged color refuses by default. `DeclareSrgbPrimaries` is an explicit caller
  declaration, not source certification: it uses PNG gAMA when present (encoding
  gamma in [0.1, 10]), otherwise the sRGB transfer function. Non-sRGB cHRM refuses.
  An explicit sRGB chunk selects sRGB transfer; ICC plus sRGB refuses.
- PNG HDR/CICP metadata and animation refuse for base color; HDR environments use
  the separate [Radiance input utility](render-hdr-input.md). Data roles record
  ignored PNG color metadata, rather than applying it to data channels.
- EXIF uses classic TIFF IFD0 only: either byte order, at most 128 entries,
  one SHORT Orientation in 1..8. Missing Orientation uses TIFF's top-left default,
  with distinct receipt flags for absent EXIF and absent Orientation. Other EXIF
  fields/thumbnail IFDs are not interpreted or certified. Malformed admission refuses.
  Nonidentity orientation requires `Apply`; all eight operations are exact pixel
  permutations, without interpolation. Oriented normal maps refuse until a vector
  and tangent-basis conversion contract exists.

## ICC scope and measured limitations

The already locked moxcms 0.8.1 is now a direct dependency (BSD-3-Clause OR
Apache-2.0). PNG 0.18.1 and SHA-256 were also already in the lockfile. No dependency
version, copied source, external image/profile, commercial asset or material dataset
is added. Fixtures are generated from original synthetic values in test code.

Initial ICC admission supports version 2/4 RGB input/display matrix/TRC profiles
with XYZ PCS and **identical RGB tone curves**. Required colorant/white-point/curve
tags are checked; CLUT and unknown conversion tags refuse. Limits: 256 KiB profile,
64 tags, 64 KiB per tag and 4096 sampled curve values. Offsets are four-byte aligned;
exact shared ranges are allowed, partial overlaps and duplicate signatures refuse.
The optional informational `chrm` tag initially admits three explicit device xy
channels (type 0, bounded xy values); PCS colorants still define the conversion.
This is bounded supported-profile admission, not a complete ICC conformance checker.

Conversion uses relative colorimetric intent, no source CICP override or fixed-point
preference, and **extended range**. Testing found the default destination LUT clips
before loss can be counted. Extended range preserves excursions until our policy.
f64 rows retain the CMS's 16-bit source LUT where analytic evaluation is unavailable;
the library still uses f32 matrix/curve arithmetic internally. No universal accuracy
or full-f64 colorimetry is claimed. Unequal RGB curves refuse because the pinned
same-curve optimization can treat two matching curves as three matching curves.

`GamutPolicy::Refuse` rejects encoded destination RGB outside [0, 1], allowing an
explicit edge tolerance of 3/65535 for finite-precision roundoff. `Clip` is an
explicit lossy preview policy. Every clamped RGB channel, including edge-tolerance
clamps, is counted. Nonfinite values always refuse. Output RGB is then linearized.

## Resource and provenance boundaries

- Encoded length **and capacity**: at most 16 MiB. Dimensions: at most 2048 per edge.
  Raw decoded storage: at most 32 MiB. Completed float RGBA: at most 64 MiB.
- Maximum working admission: 256 MiB, charging retained encoded capacity, 24 bytes
  per pixel for raw/output allowances, a 64 MiB codec reservation, 16 MiB conservative
  CMS workspace, two 256 KiB profile allowances and two f64 RGB rows. The constructor
  requires encoded capacity plus codec reservation before header work. Codec defaults
  outside our control, allocator overhead and other jobs mean this is **not an RSS,
  CPU-time or combined GPU/CPU ceiling**. Owned large buffers use fallible allocation.
- Framing/presence checks cap PNG chunks and JPEG marker segments at 4096, refuse
  duplicate/late semantic metadata and require the codec to retain declared ICC/EXIF
  information. JPEG scan stuffing/restarts are skipped without entropy decoding.
  Existing codecs remain responsible for pixels and format validation; no second
  compression decoder is introduced. PNG is fully finished before publication.
- Cancellation is checked before/after whole-image decode/profile setup, per
  normalization row and during hashing. It cannot interrupt codec/profile construction.
  The host must schedule this work away from the interactive UI thread. Worker ownership,
  document generations, stale-result rejection, pooling and browser scheduling remain
  mandatory consumer work; an immutable completed object does not implement those jobs.
- A read-only serializable receipt records recipe version 1, source and normalized
  SHA-256 (little-endian f32 sample bytes), role, precision, dimensions, color conversion,
  ICC identity, orientation operations/presence, chosen policies, clamp count, storage
  and working estimate. It includes no paths or license approval. Hashes identify bytes,
  not publishers. The receipt is diagnostic, not a persistent resource-pack schema or
  permission to redistribute images/profiles. Per-file provenance and licensing remain
  the host/library's responsibility.

Tests use independent analytic sRGB/linear expectations, adjacent 16-bit samples,
alpha/palette data, original ICC stress profiles, all eight orientations, malformed
and ignored metadata, budgets and cancellation. This utility alone cannot demonstrate
wood shading, material assignment, PBR/HDRI, GPU behavior or solver properties.

## Individually licensed file probe (2026-10-15)

The same [Black Oak Veneer](https://polyhaven.com/a/black_oak_veneer) 1K diffuse
files that the legacy utility refused now complete normalization with their actual
embedded ICC profile. Author: Jenelle van Heerden; asset license:
[Poly Haven CC0](https://polyhaven.com/license), verified at retrieval. Provider API
calls identified SolidCraft; this is an offline validation probe, not live API integration.
Files, generated output and probe harness are not bundled or committed.

| Input | Source SHA-256 | Completed result |
|---|---|---|
| PNG, 5,262,864 bytes | `3303ebadba1277488e43b0936b27743525711985db0da62ac9f989f695f6065c` | 16-bit source, 1024x1024 linear-sRGB RGBA f32, 16,777,216 output bytes, zero clamped channels |
| JPEG, 580,236 bytes | `7893f9bfeda626364f0a803dea6327c9f3a7b2f2fe330f6b4f326f6b06c0c265` | 8-bit source, same dimensions/encoding/storage, zero clamped channels |

Both receipts identify ICC SHA-256
`500fa2f01de5a0e051ce3b4111b0c94aa223cda0f4ccfbc1c631adc30a050616`.
The first probe refused its optional `chrm` tag. Support was added from ICC.1
sections 9.2.16/10.2, with an original valid/malformed fixture; no profile was stripped
or source primaries guessed. EXIF is present without Orientation, so TIFF's identity
default applies. PNG/JPEG output hashes differ as expected for different input bytes
and JPEG loss; these are completed input results, not wood rendering evidence.
Local receipts retain download URL/license/retrieval metadata and provider MD5 checks.
Any future shipped file needs its own ATTRIBUTION.md entry and reviewed provenance.

Public format references: [PNG third edition](https://www.w3.org/TR/png-3/),
[TIFF 6](https://www.itu.int/itudoc/itu-t/com16/tiff-fx/docs/tiff6.pdf),
[ICC.1](https://www.color.org/specification/ICC1v43_2010-12.pdf),
[ICC embedding note](https://www.color.org/technotes/ICC-Technote-ProfileEmbedding.pdf)
and [JPEG T.81](https://www.w3.org/Graphics/JPEG/itu-t81.pdf).
