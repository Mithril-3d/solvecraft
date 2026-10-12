//! Certificates are producer identities, never a nearest-surface selection.
use crate::{DocError, Document, FeatureKind, ModelBody, Result, SolvedSketch};
use serde::{Deserialize, Serialize};
use solvecraft_geom::Seg2;

fn valid_signature(outer: &[String], holes: &[Vec<String>]) -> bool {
    let valid = |ids: &[String]| {
        !ids.is_empty() && ids.len() <= 2000 && ids.iter().all(|id| !id.is_empty() && id.len() <= 128) && ids.windows(2).all(|pair| pair[0] < pair[1])
    };
    valid(outer)
        && holes.len() <= 128
        && holes.iter().all(|ids| valid(ids))
        && outer.len() + holes.iter().map(Vec::len).sum::<usize>() <= 2000
        && holes.windows(2).all(|pair| pair[0] < pair[1])
        && holes.iter().flatten().all(|id| outer.binary_search(id).is_err())
        && holes.iter().flatten().collect::<std::collections::BTreeSet<_>>().len() == holes.iter().map(Vec::len).sum::<usize>()
}
fn valid_signature_json(outer: Option<&serde_json::Value>, holes: Option<&serde_json::Value>) -> bool {
    let valid = |value: &serde_json::Value| {
        value.as_array().is_some_and(|ids| {
            !ids.is_empty()
                && ids.len() <= 2000
                && ids.iter().all(|id| id.as_str().is_some_and(|id| !id.is_empty() && id.len() <= 128))
                && ids.windows(2).all(|pair| pair[0].as_str() < pair[1].as_str())
        })
    };
    let Some(outer) = outer.filter(|value| valid(value)).and_then(serde_json::Value::as_array) else {
        return false;
    };
    let Some(holes) = holes.and_then(serde_json::Value::as_array).filter(|holes| holes.len() <= 128) else {
        return false;
    };
    if !holes.iter().all(valid) {
        return false;
    }
    let total = outer.len() + holes.iter().filter_map(serde_json::Value::as_array).map(Vec::len).sum::<usize>();
    let ids: Vec<_> = outer.iter().filter_map(serde_json::Value::as_str).collect();
    total <= 2000
        && holes
            .iter()
            .filter_map(serde_json::Value::as_array)
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == total - outer.len()
        && holes
            .iter()
            .filter_map(serde_json::Value::as_array)
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .all(|id| ids.binary_search(&id).is_err())
        && holes.windows(2).all(|pair| {
            let a: Vec<_> = pair[0].as_array().into_iter().flatten().filter_map(serde_json::Value::as_str).collect();
            let b: Vec<_> = pair[1].as_array().into_iter().flatten().filter_map(serde_json::Value::as_str).collect();
            a < b
        })
}
pub(crate) fn cap_key(outer: &[String], holes: &[Vec<String>]) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for ids in std::iter::once(outer).chain(holes.iter().map(Vec::as_slice)) {
        hash.update((ids.len() as u64).to_le_bytes());
        for id in ids {
            hash.update((id.len() as u64).to_le_bytes());
            hash.update(id.as_bytes());
        }
    }
    format!("{:x}", hash.finalize())
}
pub(crate) fn exact_region_signature(sketch: &SolvedSketch, region: &solvecraft_geom::Region2) -> Option<solvecraft_kernel::CapSignature> {
    if region.outer.segs.len() + region.holes.iter().map(|hole| hole.segs.len()).sum::<usize>() > 4096 || region.holes.len() > 128 {
        return None;
    }
    let signature = |outline: &solvecraft_geom::Loop2| {
        let mut ids = outline.segs.iter().map(|segment| exact_sketch_curve(sketch, segment)).collect::<Option<Vec<_>>>()?;
        ids.sort();
        ids.dedup();
        Some(ids)
    };
    let outer = signature(&region.outer)?;
    let mut holes = region.holes.iter().map(signature).collect::<Option<Vec<_>>>()?;
    holes.sort();
    valid_signature(&outer, &holes).then_some((outer, holes))
}

/// Charge all source-curve comparisons before matching; over-budget tools stay usable but uncertified.
type CapBindings = Vec<Vec<Option<solvecraft_kernel::CapSignature>>>;
pub(crate) fn extrude_cap_bindings(sketch: &SolvedSketch, bodies: &[solvecraft_kernel::Body], budget: usize) -> Option<CapBindings> {
    use solvecraft_kernel::FaceSource;
    if sketch.sketch.curves.len() > 2000 {
        return None;
    }
    let mut faces = 0usize;
    let mut comparisons = 0usize;
    for body in bodies {
        for source in body.face_sources().into_iter().flatten() {
            faces = faces.checked_add(1)?;
            if faces > 50_000 {
                return None;
            }
            let segments = match source {
                Some(FaceSource::ExtrudeSide { .. }) => 1,
                Some(FaceSource::ExtrudeCap { region, .. }) => {
                    region.outer.segs.len().checked_add(region.holes.iter().map(|hole| hole.segs.len()).sum::<usize>())?
                }
                _ => 0,
            };
            comparisons = comparisons.checked_add(segments.checked_mul(sketch.sketch.curves.len())?)?;
            if comparisons > budget {
                return None;
            }
        }
    }
    let mut counts = std::collections::BTreeMap::new();
    let mut bindings = Vec::with_capacity(bodies.len());
    for body in bodies {
        let mut cached: Option<(&solvecraft_geom::Region2, Option<solvecraft_kernel::CapSignature>)> = None;
        let mut values = Vec::new();
        for source in body.face_sources().into_iter().flatten() {
            let signature = if let Some(FaceSource::ExtrudeCap { region, high, .. }) = source {
                let signature = match &cached {
                    Some((previous, signature)) if *previous == region => signature.clone(),
                    _ => exact_region_signature(sketch, region),
                };
                cached = Some((region, signature.clone()));
                if let Some((outer, holes)) = &signature {
                    *counts.entry((*high, outer.clone(), holes.clone())).or_insert(0usize) += 1;
                }
                signature
            } else {
                None
            };
            values.push(signature);
        }
        bindings.push(values);
    }
    for (body, values) in bodies.iter().zip(&mut bindings) {
        for (source, value) in body.face_sources().into_iter().flatten().zip(values) {
            if let (Some(FaceSource::ExtrudeCap { high, .. }), Some((outer, holes))) = (source, value.as_ref())
                && counts.get(&(*high, outer.clone(), holes.clone())) != Some(&1)
            {
                *value = None;
            }
        }
    }
    Some(bindings)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FaceCertificate {
    Box { axis: u8, high: bool },
    ExtrudeCap { sketch: u64, high: bool, outer: Vec<String>, holes: Vec<Vec<String>> },
    ExtrudeSide { sketch: u64, curve: String },
    Imported { payload_sha256: String, item: u64, face: u64 },
    ShellPlanar { source_producer: u64, source: Box<FaceCertificate>, inner: bool },
}

impl FaceCertificate {
    pub fn remap_features(&mut self, features: &std::collections::BTreeMap<u64, u64>) {
        if let Self::ShellPlanar { source_producer, source, .. } = self {
            if let Some(mapped) = features.get(source_producer) {
                *source_producer = *mapped;
            }
            if !matches!(source.as_ref(), Self::ShellPlanar { .. }) {
                source.remap_features(features);
            }
        }
        if let Self::ExtrudeSide { sketch, .. } | Self::ExtrudeCap { sketch, .. } = self
            && let Some(mapped) = features.get(sketch)
        {
            *sketch = *mapped;
        }
    }
    pub fn validate(&self) -> Result<()> {
        let valid = match self {
            Self::Box { axis, .. } => *axis <= 2,
            Self::ExtrudeCap { sketch, outer, holes, .. } => *sketch != 0 && valid_signature(outer, holes),
            Self::ExtrudeSide { sketch, curve } => *sketch != 0 && !curve.is_empty() && curve.len() <= 128,
            Self::Imported { payload_sha256, item, face } => {
                *item != 0
                    && *face != 0
                    && payload_sha256.len() == 64
                    && payload_sha256.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            }
            Self::ShellPlanar { source_producer, source, .. } => {
                *source_producer != 0 && !matches!(source.as_ref(), Self::ShellPlanar { .. }) && source.validate().is_ok()
            }
        };
        if valid { Ok(()) } else { Err(DocError::Invalid("invalid face source certificate".into())) }
    }
}

pub(crate) fn validate_certificate_json(value: &serde_json::Value) -> Result<()> {
    if value.is_null() {
        return Ok(());
    }
    let fields = value.as_object().ok_or_else(|| DocError::Invalid("face certificate must be an object".into()))?;
    let nonzero = |field: &str| fields.get(field).and_then(serde_json::Value::as_u64).is_some_and(|id| id != 0);
    let valid = match fields.get("kind").and_then(serde_json::Value::as_str) {
        Some("box") => {
            fields.len() == 3
                && fields.keys().all(|key| ["kind", "axis", "high"].contains(&key.as_str()))
                && fields.get("axis").and_then(serde_json::Value::as_u64).is_some_and(|axis| axis <= 2)
                && fields.get("high").and_then(serde_json::Value::as_bool).is_some()
        }
        Some("extrude_cap") => {
            fields.len() == 5
                && fields.keys().all(|key| ["kind", "sketch", "high", "outer", "holes"].contains(&key.as_str()))
                && nonzero("sketch")
                && fields.get("high").and_then(serde_json::Value::as_bool).is_some()
                && valid_signature_json(fields.get("outer"), fields.get("holes"))
        }
        Some("extrude_side") => {
            fields.len() == 3
                && fields.keys().all(|key| ["kind", "sketch", "curve"].contains(&key.as_str()))
                && nonzero("sketch")
                && fields.get("curve").and_then(serde_json::Value::as_str).is_some_and(|curve| !curve.is_empty() && curve.len() <= 128)
        }
        Some("imported") => {
            fields.len() == 4
                && fields.keys().all(|key| ["kind", "payload_sha256", "item", "face"].contains(&key.as_str()))
                && nonzero("item")
                && nonzero("face")
                && fields
                    .get("payload_sha256")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        }
        Some("shell_planar") => {
            fields.len() == 4
                && fields.keys().all(|key| ["kind", "source_producer", "source", "inner"].contains(&key.as_str()))
                && nonzero("source_producer")
                && fields.get("inner").and_then(serde_json::Value::as_bool).is_some()
                && fields.get("source").is_some_and(|source| {
                    source.as_object().is_some_and(|source| source.get("kind").and_then(serde_json::Value::as_str) != Some("shell_planar"))
                        && validate_certificate_json(source).is_ok()
                })
        }
        _ => false,
    };
    if valid { Ok(()) } else { Err(DocError::Invalid("malformed or oversized face source certificate".into())) }
}

/// Canonical diagnostic role; producer identity remains in the typed certificate.
pub(crate) fn shell_certificate_role(source: &FaceCertificate, inner: bool) -> Result<String> {
    source.validate()?;
    let role = match source {
        FaceCertificate::Box { axis, high } => {
            let axis = ["x", "y", "z"].get(*axis as usize).ok_or_else(|| DocError::Invalid("invalid shell source axis".into()))?;
            format!("box:{}{axis}", if *high { "+" } else { "-" })
        }
        FaceCertificate::ExtrudeSide { curve, .. } => format!("side:{curve}"),
        FaceCertificate::ExtrudeCap { high, outer, holes, .. } => format!("cap:{}:{}", if *high { "high" } else { "low" }, cap_key(outer, holes)),
        FaceCertificate::Imported { payload_sha256, item, face } => format!("import:{payload_sha256}:{item}:{face}"),
        FaceCertificate::ShellPlanar { .. } => return Err(DocError::Invalid("nested shell source certificates are unsupported".into())),
    };
    Ok(format!("shell:{}:{role}", if inner { "inner" } else { "outer" }))
}

pub(crate) fn flat_source_certificate(source: &solvecraft_kernel::FaceSource) -> Option<(u64, FaceCertificate)> {
    use solvecraft_kernel::FaceSource;
    match source {
        FaceSource::Box { producer: Some(producer), axis, high, .. } => Some((*producer, FaceCertificate::Box { axis: *axis, high: *high })),
        FaceSource::CertifiedExtrudeSide { producer, sketch, curve, .. } => {
            Some((*producer, FaceCertificate::ExtrudeSide { sketch: *sketch, curve: curve.clone() }))
        }
        FaceSource::CertifiedExtrudeCap { producer, sketch, high, outer, holes, .. } => {
            Some((*producer, FaceCertificate::ExtrudeCap { sketch: *sketch, high: *high, outer: outer.clone(), holes: holes.clone() }))
        }
        FaceSource::Imported { producer: Some(producer), payload_sha256: Some(digest), item, face } => {
            Some((*producer, FaceCertificate::Imported { payload_sha256: digest.clone(), item: *item, face: *face }))
        }
        _ => None,
    }
}

pub(crate) fn flat_source_matches(certificate: &FaceCertificate, source: &solvecraft_kernel::FaceSource, producer: u64) -> bool {
    use solvecraft_kernel::FaceSource;
    match (certificate, source) {
        (FaceCertificate::Box { axis, high }, FaceSource::Box { producer: Some(current), axis: current_axis, high: current_high, .. }) => {
            producer == *current && axis == current_axis && high == current_high
        }
        (
            FaceCertificate::ExtrudeSide { sketch, curve },
            FaceSource::CertifiedExtrudeSide { producer: current, sketch: current_sketch, curve: current_curve, .. },
        ) => producer == *current && sketch == current_sketch && curve == current_curve,
        (
            FaceCertificate::ExtrudeCap { sketch, high, outer, holes },
            FaceSource::CertifiedExtrudeCap {
                producer: current,
                sketch: current_sketch,
                high: current_high,
                outer: current_outer,
                holes: current_holes,
                ..
            },
        ) => producer == *current && sketch == current_sketch && high == current_high && outer == current_outer && holes == current_holes,
        (
            FaceCertificate::Imported { payload_sha256, item, face },
            FaceSource::Imported { producer: Some(current), payload_sha256: Some(digest), item: current_item, face: current_face },
        ) => producer == *current && payload_sha256 == digest && item == current_item && face == current_face,
        _ => false,
    }
}

/// Match a generated segment to one exact analytic sketch curve. Coincident
/// candidates are ambiguous and never resolved by distance or candidate order.
pub(crate) fn exact_sketch_curve(sketch: &SolvedSketch, generated: &Seg2) -> Option<String> {
    if sketch.sketch.curves.len() > 2000 {
        return None;
    }
    let mut found = None;
    for (index, curve) in sketch.sketch.curves.iter().enumerate() {
        if curve.construction
            || !matches!(
                curve.kind,
                solvecraft_sketch::CurveKind::Line { .. } | solvecraft_sketch::CurveKind::Arc { .. } | solvecraft_sketch::CurveKind::Circle { .. }
            )
        {
            continue;
        }
        let matches = if let solvecraft_sketch::CurveKind::Circle { c, r } = curve.kind {
            sketch
                .sketch
                .point(c)
                .is_some_and(|center| exact_segment_contains(&Seg2::Arc { center, radius: r, start: 0., sweep: std::f64::consts::TAU }, generated))
        } else {
            sketch.sketch.segs(index).iter().any(|original| exact_segment_contains(original, generated))
        };
        if matches {
            if found.is_some() {
                return None;
            }
            found = Some(curve.id.clone());
        }
    }
    found
}

fn exact_segment_contains(original: &Seg2, generated: &Seg2) -> bool {
    match (original, generated) {
        (Seg2::Line { a, b }, Seg2::Line { a: c, b: d }) => {
            let direction = *b - *a;
            let scale = direction.len().max((*d - *c).len()).max(1.);
            let interval = |point: solvecraft_geom::Vec2| {
                let t = (point - *a).dot(direction) / direction.len2();
                (-1e-10..=1. + 1e-10).contains(&t)
            };
            direction.len() > 1e-12
                && direction.cross(*c - *a).abs() <= 1e-10 * scale * scale
                && direction.cross(*d - *a).abs() <= 1e-10 * scale * scale
                && interval(*c)
                && interval(*d)
        }
        (Seg2::Arc { center: a, radius: ar, start: os, sweep: ow }, Seg2::Arc { center: b, radius: br, start: gs, sweep: gw }) => {
            let tau = std::f64::consts::TAU;
            let tolerance = 1e-10;
            let length = ow.abs();
            let generated_length = gw.abs();
            if a.dist(*b) > 1e-10 * ar.max(*br).max(1.)
                || (ar - br).abs() > 1e-10 * ar.max(*br).max(1.)
                || length > tau + tolerance
                || generated_length > length + tolerance
            {
                return false;
            }
            if length >= tau - tolerance {
                return true;
            }
            let original_low = if *ow >= 0. { *os } else { os + ow };
            let generated_low = if *gw >= 0. { *gs } else { gs + gw };
            let offset = (generated_low - original_low).rem_euclid(tau);
            let offset = if offset > tau - tolerance { 0. } else { offset };
            offset + generated_length <= length + tolerance
        }
        _ => false,
    }
}

impl Document {
    pub fn configuration_face_target(&self, body: &ModelBody, index: usize, point: solvecraft_geom::Vec3) -> Result<crate::config::ConfigFaceTarget> {
        let names = crate::naming::face_names(body);
        let mut name = names
            .get(index)
            .map(|name| crate::naming::strip_piece(name).to_string())
            .ok_or_else(|| DocError::Unknown("face source name missing".into()))?;
        let origin = match body.body.face_sources().and_then(|sources| sources.get(index)).and_then(Option::as_ref) {
            Some(solvecraft_kernel::FaceSource::Imported { producer: Some(producer), .. })
            | Some(solvecraft_kernel::FaceSource::CertifiedExtrudeSide { producer, .. })
            | Some(solvecraft_kernel::FaceSource::CertifiedExtrudeCap { producer, .. })
            | Some(solvecraft_kernel::FaceSource::ShellPlanar { producer: Some(producer), .. })
            | Some(solvecraft_kernel::FaceSource::Box { producer: Some(producer), .. }) => *producer,
            _ => name
                .strip_prefix('F')
                .and_then(|name| name.split_once(':'))
                .and_then(|(id, _)| id.parse().ok())
                .ok_or_else(|| DocError::Invalid("face source producer missing".into()))?,
        };
        let certificate = self.configuration_face_certificate(body, index)?;
        if let Some(FaceCertificate::ExtrudeSide { curve, .. }) = &certificate {
            name = format!("F{origin}:side:{curve}");
        } else if let Some(FaceCertificate::ShellPlanar { source, inner, .. }) = &certificate {
            name = format!("F{origin}:{}", shell_certificate_role(source, *inner)?);
        } else if !name.starts_with(&format!("F{origin}:")) {
            name = format!("F{origin}:face:{index}");
        }
        let inverse = crate::mat_inverse(&body.source_transform).ok_or_else(|| DocError::Invalid("invalid definition placement".into()))?;
        Ok(crate::config::ConfigFaceTarget {
            body: body.definition_name().into(),
            origin_feature: origin,
            name,
            point: crate::apply_point(&inverse, point),
            certificate,
        })
    }
    pub fn configuration_face_certificate(&self, body: &ModelBody, index: usize) -> Result<Option<FaceCertificate>> {
        let Some(source) = body.body.face_sources().and_then(|sources| sources.get(index)).and_then(Option::as_ref) else {
            return Ok(None);
        };
        if let solvecraft_kernel::FaceSource::ShellPlanar { producer: Some(producer), source, inner, .. } = source {
            if !self.feature(*producer).is_some_and(|feature| matches!(feature.kind, FeatureKind::Shell { .. })) {
                return Err(DocError::Invalid("shell source producer missing".into()));
            }
            let (source_producer, source) =
                flat_source_certificate(source).ok_or_else(|| DocError::Invalid("shell has no flat certified source".into()))?;
            if !self.flat_certificate_producer_matches(source_producer, &source) {
                return Err(DocError::Invalid("shell original source producer missing".into()));
            }
            let certificate = FaceCertificate::ShellPlanar { source_producer, source: Box::new(source), inner: *inner };
            certificate.validate()?;
            return Ok(Some(certificate));
        }
        if let solvecraft_kernel::FaceSource::Imported { item, face, payload_sha256: Some(digest), producer: Some(producer) } = source {
            if !self.feature(*producer).is_some_and(|feature| matches!(feature.kind, FeatureKind::Import { .. })) {
                return Err(DocError::Invalid("import source producer missing".into()));
            }
            let certificate = FaceCertificate::Imported { payload_sha256: digest.clone(), item: *item, face: *face };
            certificate.validate()?;
            return Ok(Some(certificate));
        }
        let certificate = match source {
            solvecraft_kernel::FaceSource::CertifiedExtrudeSide { producer, sketch, curve, .. } => {
                if !self.feature(*producer).is_some_and(|feature| matches!(feature.kind, FeatureKind::Extrude { .. })) {
                    return Err(DocError::Invalid("side source producer missing".into()));
                }
                FaceCertificate::ExtrudeSide { sketch: *sketch, curve: curve.clone() }
            }
            _ => return Ok(None),
        };
        certificate.validate()?;
        Ok(Some(certificate))
    }
    pub fn strong_face_target(&self, body: &ModelBody, index: usize, point: solvecraft_geom::Vec3) -> Result<crate::config::ConfigFaceTarget> {
        let mut target = self.configuration_face_target(body, index, point)?;
        match body.body.face_sources().and_then(|sources| sources.get(index)).and_then(Option::as_ref) {
            Some(solvecraft_kernel::FaceSource::Box { producer: Some(producer), axis, high, .. }) => {
                target.origin_feature = *producer;
                let axis_name = ["x", "y", "z"].get(*axis as usize).ok_or_else(|| DocError::Invalid("invalid box face axis".into()))?;
                target.name = format!("F{producer}:{}{axis_name}", if *high { "+" } else { "-" });
                target.certificate = Some(FaceCertificate::Box { axis: *axis, high: *high });
            }
            Some(solvecraft_kernel::FaceSource::CertifiedExtrudeCap { producer, sketch, high, outer, holes, .. }) => {
                target.origin_feature = *producer;
                target.name = format!("F{producer}:cap:{}:{}", if *high { "high" } else { "low" }, cap_key(outer, holes));
                target.certificate = Some(FaceCertificate::ExtrudeCap { sketch: *sketch, high: *high, outer: outer.clone(), holes: holes.clone() });
            }
            _ => {}
        }
        self.certify_config_face(&target)?;
        Ok(target)
    }
}

pub(crate) fn payload_digest(payload: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(payload.as_bytes()))
}

#[cfg(test)]
#[path = "config_shell_tests.rs"]
mod shell_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn certificate_budgets_apply_before_saved_target_and_paint_decode() {
        for certificate in [
            json!({"kind":"extrude_side","sketch":1,"curve":"x".repeat(1_000_000)}),
            json!({"kind":"imported","payload_sha256":"0".repeat(64),"item":0,"face":1}),
            json!({"kind":"imported","payload_sha256":"g".repeat(64),"item":1,"face":1}),
            json!({"kind":"extrude_side","sketch":1,"curve":"c1","extra":[[[[[]]]]]}),
        ] {
            assert!(validate_certificate_json(&certificate).is_err());
            let target = json!({"configs":{"columns":[{"face_appearance":{"body":"B","origin_feature":1,"name":"F1:side:c1","point":[0,0,0],"certificate":certificate}}]}});
            assert!(crate::config::validate_styles_json(&target).is_err());
            let paint = json!({"appearances":{"faces":[{"certificate":certificate}]}});
            assert!(crate::config::validate_styles_json(&paint).is_err());
        }
        assert!(validate_certificate_json(&json!({"kind":"imported","payload_sha256":"a".repeat(64),"item":1,"face":2})).is_ok());
        assert!(validate_certificate_json(&json!({"kind":"extrude_side","sketch":1,"curve":"c1"})).is_ok());
    }
    #[test]
    fn analytic_arc_interval_refuses_unsampled_gaps_and_accepts_wrap_reversal() {
        let radians = |degrees: f64| degrees.to_radians();
        let arc = |start, sweep| Seg2::Arc { center: solvecraft_geom::Vec2::default(), radius: 10., start: radians(start), sweep: radians(sweep) };
        assert!(!exact_segment_contains(&arc(10., 350.), &arc(20., 360.)), "five point probes missed the missing ten-degree gap");
        assert!(!exact_segment_contains(&arc(10., 350.), &arc(20., -360.)));
        assert!(exact_segment_contains(&arc(350., 40.), &arc(355., 20.)));
        assert!(exact_segment_contains(&arc(350., 40.), &arc(15., -20.)));
        assert!(exact_segment_contains(&arc(30., -40.), &arc(15., -20.)));
        assert!(!exact_segment_contains(&arc(350., 40.), &arc(345., 20.)));
        assert!(exact_segment_contains(&arc(0., 360.), &arc(20., -360.)));
    }
}
