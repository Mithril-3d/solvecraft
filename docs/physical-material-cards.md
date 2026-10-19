# Physical material data cards

`material.properties.validate {card}` checks a caller-supplied candidate record,
without assigning it, saving it in the design, reading files, fetching sources or
changing history, selection, camera or live geometry. This is phase 1 of #923/#922,
building towards #785. Existing `material.create/edit` and their density/appearance
behavior are unchanged. The design format is unchanged.

Validation means numerical and structural consistency. It does not authenticate
sources, approve redistribution, certify design values, provide a solver or enable
advanced rendering. The result reports `analysis_solver_available: false` and
`rendering_properties_available: false`. Missing properties remain unknown.

## Contract (standalone card schema 1)

### Candidate review window

Search for **Validate Physical Material Data** in the command palette (or use
`ui.start` with `material.properties.validate`) to open the candidate reviewer.
Paste a schema-1 card object and choose **Review card**. The existing engine
validator checks it; the window shows known properties in SI units, unknown
properties, per-property conditions and source claims as inert text. Material-local
axes do not assign a frame to a body. Isotropic G/K come from the engine report.

Editing the input clears the previous review. Invalid input shows its refusal
without retaining a stale valid result. Input is capped at 262,144 characters and
256 KiB before JSON parsing; the validator's tighter structural and text budgets
still apply. The window does not read files, fetch references, assign properties,
replace density, modify history or save the candidate. Close discards it. Calling
the engine's JSON command still returns its report without opening a dialog.

This main-based review unit is independent of assigned-card persistence, local
file import and the design-owned inspector follow-up in #1094. No dataset or
rendering capability is supplied by reviewing a candidate.

A card has `schema_version: 1`, `designation`, `family` (`wood`, `polymer`, `glass`,
`metal`, `other`), `sources` and `properties`. Source IDs are exact, case-sensitive
local references. Each source has `id`, `title`, `kind` (`user_supplied`,
`measurement`, `public_reference`, `illustrative`), optional `license` and
`reference` strings. These strings are metadata supplied by the caller; even a
declared open license remains in `source_review_required`. Nothing is downloaded.

Every property has `data`, `source_id` and optional `conditions`. Conditions are
`temperature_k`, `moisture_dry_basis_kg_kg`, `strain_rate_per_s` and a free-text
`state` (grade, temper, processing or specimen condition). Conditions belong to
the property, not an assumed global room temperature. An elastic block represents
constants measured under the same conditions and source; mixed-condition curves
and independently sourced constants need a later representation.

| `data.kind` | Explicit SI fields |
|---|---|
| `density` | `kg_m3` |
| `elasticity` | `model`, described below |
| `tensile_strength`, `compressive_strength`, `yield_strength`, `shear_strength` | `pa` |
| `thermal_conductivity` | `w_m_k: [k1,k2,k3]`, `axes` |
| `thermal_expansion` | `per_k: [a1,a2,a3]`, `axes` |
| `specific_heat` | `j_kg_k` |

Directional values use `axes: material123` (material-local 1/2/3) or `wood_lrt`
(longitudinal/radial/tangential). These labels do not assign a frame to a body or
infer mechanical orientation from texture rotation. Equal directional values can
describe an isotropic thermal property. Thermal expansion may be zero or negative.

An isotropic elastic model has `symmetry: isotropic`, `young_modulus_pa` and
`poisson_ratio`. Stability requires E > 0 and -1 < nu < 0.5. Valid auxetic values
are accepted. The result derives G = E/[2(1+nu)] and K = E/[3(1-2nu)], in Pa.

An orthotropic model has `symmetry: orthotropic`, `axes`,
`young_moduli_pa: [E1,E2,E3]`, `shear_moduli_pa: [G12,G23,G31]` and
`poisson_ratios: [nu12,nu23,nu31]`. Here nu_ij = -eps_j/eps_i under stress along i;
reciprocity supplies nu_ij/Ei = nu_ji/Ej. The normal compliance is symmetric with
diagonal 1/Ei and off-diagonal -nu_ij/Ei. Diagonal scaling produces unit diagonal;
Sylvester's criterion checks its positive definiteness. All three shear moduli
must be positive. General orthotropic Poisson ratios are not capped at 0.5.
Moduli whose reciprocals cannot be represented finitely are rejected; the
isotropic derived response and its reciprocals must also remain finite/positive.

Moisture is kg water / kg dry material and can exceed one. It is not a wet-mass
fraction. Unspecified temperature/state, wood moisture or polymer strain rate are
reported in `unspecified_conditions`; they are not guessed or replaced by zero.
The validation report also returns `missing_condition_fields`, a map from each
incomplete property kind to its exact absent field names. Temperature and state
apply to every family; wood additionally checks dry-basis moisture and polymers
strain rate, following the same reporting policy as `unspecified_conditions`.
For example, a wood density with temperature and state but no moisture reports
`{"density":["moisture_dry_basis_kg_kg"]}`. Complete properties are omitted;
an empty card returns an empty map, without inventing properties or conditions.
This detail is read-only and does not change admission or the saved card schema.
Older serialized validation reports without the map deserialize with an empty
map; regenerate validation from the card to obtain the missing-field detail.
The family label does not certify that a chosen constitutive model fits a material.
Scalar strengths are individual measurements, not a three-dimensional failure
envelope. Typed strength directions and test methods remain a separate extension.

## Admission limits

Before cloning/deserializing JSON: at most 2,048 nodes, depth 12, 32 array entries
or object fields and a 256 KiB conservative accounting budget (32 bytes per node
plus string/key bytes). The typed record allows 32 sources and 16 properties;
duplicate source IDs or property kinds and unknown fields are rejected.

Text limits in UTF-8 bytes: designation/title 256, source ID 128, license 512,
reference 2,048 and state 1,024; controls and blank text are rejected. All numeric
values must be finite. Positive-value upper admission limits are density 1e6
kg/m³, moduli/strengths 1e15 Pa, conductivity 1e9 W/(m K), heat capacity 1e9
J/(kg K), temperature 20,000 K and strain rate 1e12 /s. Expansion is within ±1 /K,
dry-basis moisture within [0,1e6] (also accommodating highly hydrated polymers),
orthotropic Poisson ratios within ±1e6 in addition
to the compliance stability test. These are generous technical admission limits,
not measured applicability ranges or a conditioning guarantee for a future solver.

## Original synthetic example

These deliberately invented constants demonstrate validation, not a real grade:

```json
{
  "card": {
    "schema_version": 1,
    "designation": "Synthetic elastic example — not design data",
    "family": "other",
    "sources": [{
      "id": "example",
      "title": "Original synthetic example",
      "kind": "illustrative",
      "license": "MIT OR Apache-2.0"
    }],
    "properties": [{
      "data": {
        "kind": "elasticity",
        "model": {"symmetry": "isotropic", "young_modulus_pa": 12000000000, "poisson_ratio": 0.25}
      },
      "source_id": "example"
    }]
  }
}
```

The result has G = 4.8e9 Pa, K = 8e9 Pa, unspecified conditions and source review
still required. No assigned mass, saved material or appearance changes.

## Next phases and references

Persistence/library integration needs a separately reviewed document migration,
atomic edits, undo/redo and offline reopening, coordinated with #863. Curves,
uncertainty, hardness scales, viscoelasticity and failure criteria remain later work.
Visual PBR/texture/environment consumption uses the renderer contract requested
in #813, with #909/#837 input utilities; accepting physical data advertises none
of those capabilities.

The [USDA Wood Handbook](https://research.fs.usda.gov/fpl/wood-handbook), including
[chapter 4's dry-mass moisture definition](https://research.fs.usda.gov/download/treesearch/62243.pdf), is a public
scientific reference for moisture, mechanical behavior and the distinction between
specimen properties and graded design values. No handbook data/table is bundled.
The [Khronos PBR material model](https://www.khronos.org/gltf/pbr) is a reference for
the separate optical track. No third-party code, engineering dataset or asset is
added by this implementation.

## Directional elasticity query

`material.properties.directional {card, direction: [x,y,z]}` reports Young's modulus
in Pa for a uniaxial, small-strain constitutive test along a material-local direction.
It returns the safely normalized direction, declared axes (`null` for isotropy), and
the full validation/source/condition report. It neither assigns axes to a body nor
predicts a part's displacement, failure, strength or safety factor. No solver or
render capability is enabled. Missing elasticity refuses instead of inventing data.

For orthotropy, let n=(x,y,z) be the unit direction. With engineering shear
compliance 1/G and the reciprocal Poisson convention above:

```
1/E(n) = x^4/E1 + y^4/E2 + z^4/E3
       + x²y²(1/G12 - 2 nu12/E1)
       + y²z²(1/G23 - 2 nu23/E2)
       + z²x²(1/G31 - 2 nu31/E3).
```

This follows by applying stress sigma = n tensor n, computing strain with the
symmetric compliance, then contracting n with strain and n. The implementation
uses a Cholesky sum of squares for the normal contribution, avoiding cancellation
between the positive diagonal and negative coupling terms. Shear contributions
are also nonnegative. Isotropy returns E directly, independent of orientation.

Input directions must contain exactly three finite numbers and be nonzero; arbitrary
magnitude is allowed. Max-component scaling avoids overflow or underflow while
normalizing very large or tiny vectors. Unrepresentable compliance/modulus refuses;
this finite-precision check does not establish a condition-number certificate.
The card's test conditions still apply; changing direction does not extrapolate
temperature, moisture, process, rate, damage or nonlinear behavior. Synthetic analytic
tests cover principal axes, the isotropic limit and off-axis shear/coupling terms.
No physical dataset or external implementation is included.
