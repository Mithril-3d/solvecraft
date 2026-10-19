# Bounded texture input (resource admission)

The separate [explicit normalization API](render-texture-normalization.md) handles
admitted high-precision/color/orientation conversions. The strict utility described
below retains its existing refusals; normalization does not enable texture rendering.

`solidcraft_render::texture_input` is an independent input utility for #900, coordinated
with #792 and #813. It accepts in-memory PNG/JPEG bytes and supplies completed RGBA8
pixels. It does not change a document, read files, assign materials, add a resource-pack
format, schedule work, upload GPU data or advertise PBR/triplanar support.

`TextureSource::new(bytes, role)` admits bounded headers; `decode(cancelled)` validates
the whole image before returning `DecodedTexture`. Sources are immutable and shared.
Header admission alone is not proof of valid pixel data. PNG/JPEG codecs run as bounded
whole-image operations, so a host must use its own worker rather than block the UI.
Cancellation is checked before/after decoding and conversion; it cannot interrupt a
codec internally. This is not the cooperative row API of the separate HDR utility.

Roles are `base_color`, `normal_open_gl`, `roughness`, `metallic`, `ambient_occlusion`.
Base color declares sRGB interpretation; other roles retain data-channel values.
The utility does not apply a transfer function, normalize normals, swap DirectX normal
channels, unpack combined maps or infer semantics from file names. Missing maps,
material defaults and sampling/basis choices belong to the eventual renderer adapter.

Initial limits and exclusions:

- Nonempty encoded storage, including Vec capacity, at most 16 MiB.
- PNG/JPEG only, with a required final PNG IEND / JPEG EOI marker. This does not
  constitute a complete container-framing/trailing-data validator.
- Positive dimensions, each at most 2048; completed RGBA8 is at most 16 MiB.
- Codec allocation limit 64 MiB, separately from retained encoded bytes and output
  conversion. This is not a combined CPU/GPU memory guarantee.
- Eight-bit channels only. Sixteen-bit files refuse rather than silently quantize.
- Embedded ICC profiles and nontrivial EXIF orientation refuse. Explicit color-space
  conversion/orientation normalization is required before admission; no ICC transform
  or PNG gamma correction is performed here. Untagged base color is a caller-declared
  sRGB input, not an inferred or certified source color space.
- Third-party decoding/conversion panics are contained and partial buffers never escape.
  GPU allocation, browser scheduling and document/session generations remain host concerns.

The numerical opaque-surface contract is available in `surface`; its product consumer
is being integrated separately in #1046 under #1004. The texture/environment adapter
and its resource ownership are still separate work coordinated in #813.
Wood repeat size/rotation, definition-local grain frames, durable resource identity,
source/license receipts, document packaging and setup-owned HDR rotation/exposure must
use that agreed consumer representation. This input utility does not reserve any of
those fields or a document version. The scalar library from #863 remains distinct.

HDR environments use existing `lighting::HdrSource` / `HdrDecodeTask`, not this PNG/JPEG
path: Radiance RGBE, at most 16 MiB and 2048x1024, standard 2:1 lat-long, finite corrected
radiance. See [HDR input](render-hdr-input.md). Successful decoding alone enables neither
IBL nor GPU preview. The current CPU snapshot executor remains unchanged and must not
claim unsupported HDRI/PBR output.

## Licensed-file probe (2026-10-14)

Files were downloaded outside the checkout for validation only, from the public API
with an identifying SolidCraft User-Agent. No assets are bundled by this change.
[Poly Haven assets are CC0](https://polyhaven.com/license); live API integration has
separate credit/identity conditions. These results are input checks, not shaded images:

| Resource | Author / source | Result |
|---|---|---|
| Black Oak Veneer, 1K base-color PNG | Jenelle van Heerden; [asset](https://polyhaven.com/a/black_oak_veneer) | Refused: 16-bit channel precision is outside this initial utility. |
| Black Oak Veneer, 1K base-color JPEG | Same asset/author | Refused: embedded ICC requires explicit color conversion. |
| Joburg Central Sunset, 1K Radiance HDR | Greg Zaal and Dimitrios Savva; [asset](https://polyhaven.com/a/sunset_jhbcentral) | Existing HDR utility fully validates 1024x512, 1,573,705 encoded bytes, 6,291,456 linear RGB bytes. No lighting result is claimed. |

The refusals are deliberate evidence of remaining normalization work, not successful
wood integration. Provider MD5 values were checked against selected downloads and local
SHA-256 receipts retained with source/license/retrieval metadata. Neither is a claim of
publisher authentication. Shipped assets would require per-file ATTRIBUTION.md entries;
private/user resources need their own portable provenance records. Reuse the maintainer's
starter set when available rather than introduce a competing set.

Tests generate original small PNG/JPEG files in code and check exact channel values,
role interpretation, dimensions/storage limits, precision rejection, truncation,
cancellation and continued source reuse. These tests do not establish GPU performance,
interactive browser behavior or the final resource-pack schema.
