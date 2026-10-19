//! Pre-allocation bounds for versioned analytic offset roles and boundary references.
use std::collections::BTreeSet;

use serde_json::Value;

use crate::{DocError, Document, FeatureKind, ProfileSel, Result};

fn invalid() -> DocError {
    DocError::Invalid("damaged design: invalid analytic offset topology or boundary reference".into())
}

fn id(value: &Value) -> Result<&str> {
    value.as_str().filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)).ok_or_else(invalid)
}

fn exact_keys(value: &Value, keys: &[&str]) -> Result<()> {
    let object = value.as_object().ok_or_else(invalid)?;
    if object.len() != keys.len() || object.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err(invalid());
    }
    Ok(())
}

/// Validate a complete-source profile reference before copying or decoding it.
pub fn whole_profile_selection_from_value(value: &Value) -> Result<ProfileSel> {
    validate_whole_reference(value)?;
    serde_json::from_value(value.clone()).map_err(|_| invalid())
}

fn validate_whole_reference(value: &Value) -> Result<()> {
    exact_keys(value, &["type", "profiles"])?;
    if value.get("type").and_then(Value::as_str) != Some("whole_boundaries") {
        return Err(invalid());
    }
    let profiles = value.get("profiles").and_then(Value::as_array).ok_or_else(invalid)?;
    if profiles.is_empty() || profiles.len() > 128 {
        return Err(invalid());
    }
    let mut total = 0usize;
    let mut identities = BTreeSet::new();
    for profile in profiles {
        exact_keys(profile, &["outer", "holes"])?;
        let mut boundary = |value: &Value| -> Result<Vec<String>> {
            let values = value.as_array().ok_or_else(invalid)?;
            if values.is_empty() || values.len() > 512 {
                return Err(invalid());
            }
            total += values.len();
            if total > 20_000 {
                return Err(invalid());
            }
            let mut previous = None;
            // Bound each raw ID before any owned allocation. Require canonical order.
            for value in values {
                let current = id(value)?;
                if previous.is_some_and(|previous| previous >= current) {
                    return Err(invalid());
                }
                previous = Some(current);
            }
            values.iter().map(|value| Ok(id(value)?.to_owned())).collect()
        };
        let outer = boundary(profile.get("outer").ok_or_else(invalid)?)?;
        let holes = profile.get("holes").and_then(Value::as_array).ok_or_else(invalid)?;
        if holes.len() > 128 {
            return Err(invalid());
        }
        let mut sorted_holes = Vec::with_capacity(holes.len());
        let mut all: BTreeSet<_> = outer.iter().cloned().collect();
        for hole in holes {
            let hole = boundary(hole)?;
            if sorted_holes.last().is_some_and(|previous| previous >= &hole) || hole.iter().any(|curve| !all.insert(curve.clone())) {
                return Err(invalid());
            }
            sorted_holes.push(hole);
        }
        if !identities.insert((outer, sorted_holes)) {
            return Err(invalid());
        }
    }
    Ok(())
}

pub(crate) fn validate_raw(value: &Value) -> Result<()> {
    let Some(features) = value.get("features").and_then(Value::as_array) else { return Ok(()) };
    let mut total_roles = 0usize;
    for feature in features {
        if feature.get("type").and_then(Value::as_str) != Some("sketch") {
            continue;
        }
        let Some(offsets) = feature.get("sketch").and_then(|sketch| sketch.get("offsets")) else { continue };
        let offsets = offsets.as_array().ok_or_else(invalid)?;
        let mut groups = BTreeSet::new();
        let mut outputs = BTreeSet::new();
        let mut point_owners = std::collections::BTreeMap::new();
        let mut sketch_roles = 0usize;
        for offset in offsets {
            let Some(topology) = offset.get("topology").filter(|topology| !topology.is_null()) else { continue };
            if groups.len() >= 128 {
                return Err(invalid());
            }
            exact_keys(topology, &["version", "id", "roles"])?;
            let group = id(topology.get("id").ok_or_else(invalid)?)?;
            if !groups.insert(group)
                || topology.get("version").and_then(Value::as_u64) != Some(1)
                || offset.get("closed").and_then(Value::as_bool) != Some(true)
            {
                return Err(invalid());
            }
            let distance = offset.get("distance").and_then(Value::as_f64).ok_or_else(invalid)?;
            if !distance.is_finite() || distance.abs() <= 1e-9 || distance.abs() >= 1e9 {
                return Err(invalid());
            }
            let sources = offset.get("sources").and_then(Value::as_array).ok_or_else(invalid)?;
            let roles = topology.get("roles").and_then(Value::as_array).ok_or_else(invalid)?;
            let active = offset.get("outputs").and_then(Value::as_array).ok_or_else(invalid)?;
            if roles.is_empty() || roles.len() > 512 || roles.len() != sources.len() || active.len() > roles.len() {
                return Err(invalid());
            }
            total_roles = total_roles.saturating_add(roles.len());
            sketch_roles = sketch_roles.saturating_add(roles.len());
            if total_roles > 1_000_000 || sketch_roles > 20_000 {
                return Err(invalid());
            }
            let mut active_index = 0;
            let mut source_ids = BTreeSet::new();
            for (source, role) in sources.iter().zip(roles) {
                let source = source.as_array().ok_or_else(invalid)?;
                if source.len() != 2 || source.get(1).and_then(Value::as_bool).is_none() {
                    return Err(invalid());
                }
                let source = id(source.first().ok_or_else(invalid)?)?;
                if !source_ids.insert(source) {
                    return Err(invalid());
                }
                let role_object = role.as_object().ok_or_else(invalid)?;
                if role_object.keys().any(|key| !matches!(key.as_str(), "output" | "kind" | "start" | "end" | "retired" | "flags")) {
                    return Err(invalid());
                }
                if let Some(flags) = role.get("flags") {
                    exact_keys(flags, &["construction", "centerline", "reversed", "fixed"])?;
                    if flags.as_object().is_none_or(|flags| flags.values().any(|flag| flag.as_bool().is_none())) {
                        return Err(invalid());
                    }
                }
                let output = id(role.get("output").ok_or_else(invalid)?)?;
                if source == output || !outputs.insert(output) {
                    return Err(invalid());
                }
                if role.get("start") == role.get("end") {
                    return Err(invalid());
                }
                for key in ["start", "end"] {
                    let point = id(role.get(key).ok_or_else(invalid)?)?;
                    if point == output || point == source || point_owners.insert(point, group).is_some_and(|owner| owner != group) {
                        return Err(invalid());
                    }
                }
                let kind = role.get("kind").and_then(Value::as_str).ok_or_else(invalid)?;
                let retired = role.get("retired").and_then(Value::as_bool).ok_or_else(invalid)?;
                if !matches!(kind, "line" | "arc") || (retired && kind != "arc") {
                    return Err(invalid());
                }
                if !retired {
                    if active.get(active_index).map(id).transpose()?.is_none_or(|id| id != output) {
                        return Err(invalid());
                    }
                    active_index += 1;
                }
            }
            if active_index != active.len() || active_index < 3 {
                return Err(invalid());
            }
        }
        if groups.iter().any(|group| outputs.contains(group) || point_owners.contains_key(group))
            || outputs.iter().any(|output| point_owners.contains_key(output))
        {
            return Err(invalid());
        }
    }
    fn references(value: &Value) -> Result<()> {
        if value.get("type").and_then(Value::as_str) == Some("whole_boundaries") {
            return validate_whole_reference(value);
        }
        if value.get("type").and_then(Value::as_str) == Some("offset_boundaries") {
            exact_keys(value, &["type", "associations"])?;
            let ids = value.get("associations").and_then(Value::as_array).ok_or_else(invalid)?;
            if ids.is_empty() || ids.len() > 128 {
                return Err(invalid());
            }
            let mut seen = BTreeSet::new();
            for value in ids {
                if !seen.insert(id(value)?) {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
    // Only feature profile fields can contain this variant. Do not impose a new
    // document-wide node limit on unrelated legacy sketches or imported payloads.
    for feature in features {
        if let Some(selection) = feature.get("profiles") {
            references(selection)?;
        }
        if let Some(sections) = feature.get("sections").and_then(Value::as_array) {
            if sections.len() > 10_000 {
                return Err(invalid());
            }
            for section in sections {
                if let Some(selection) = section.get("profiles") {
                    references(selection)?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_typed(document: &Document) -> Result<()> {
    let mut sketches = std::collections::BTreeMap::new();
    for feature in &document.features {
        if let FeatureKind::Sketch { sketch, .. } = &feature.kind {
            sketch.validate_offset_topologies()?;
            sketches.insert(feature.id, sketch);
        }
        for (sketch, selection) in feature.kind.profile_sels() {
            if let ProfileSel::OffsetBoundaries { associations } = selection {
                // A deleted association or source sketch is a repairable model error,
                // just like an ordinary missing curve reference. Keep that design loadable.
                let Some(sketch) = sketches.get(&sketch) else { continue };
                for association in associations {
                    let count =
                        sketch.offsets.iter().filter_map(|offset| offset.topology.as_ref()).filter(|topology| &topology.id == association).count();
                    if count > 1 {
                        return Err(invalid());
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use serde_json::json;

    fn fixture() -> Value {
        json!({"features":[{"type":"sketch","sketch":{"offsets":[{
            "sources":[["s0",true],["s1",true],["s2",true],["s3",true]],
            "outputs":["l0","l1","l2"],"closed":true,"distance":1.4,
            "topology":{"version":1,"id":"o1","roles":[
                {"output":"l0","kind":"line","start":"p0","end":"p1","retired":false},
                {"output":"a0","kind":"arc","start":"p1","end":"p2","retired":true},
                {"output":"l1","kind":"line","start":"p2","end":"p3","retired":false},
                {"output":"l2","kind":"line","start":"p3","end":"p0","retired":false}
            ]}
        }]}}]})
    }

    #[test]
    fn whole_boundaries_raw_identity_is_bounded_canonical_and_complete() {
        let good = json!({"type":"whole_boundaries","profiles":[{"outer":["s2"],"holes":[["s1"]]}]});
        assert!(matches!(whole_profile_selection_from_value(&good).unwrap(), ProfileSel::WholeBoundaries { profiles } if profiles.len() == 1));
        let malformed = [
            json!({"type":"whole_boundaries","profiles":[]}),
            json!({"type":"whole_boundaries","profiles":[{"outer":[],"holes":[]}]}),
            json!({"type":"whole_boundaries","profiles":[{"outer":["s1","s1"],"holes":[]}]}),
            json!({"type":"whole_boundaries","profiles":[{"outer":["s2","s1"],"holes":[]}]}),
            json!({"type":"whole_boundaries","profiles":[{"outer":["s1"],"holes":[["s1"]]}]}),
            json!({"type":"whole_boundaries","profiles":[{"outer":["s2"],"holes":[["s1"],["s1"]]}]}),
            json!({"type":"whole_boundaries","profiles":[{"outer":["s2"],"holes":[["s3"],["s1"]]}]}),
            json!({"type":"whole_boundaries","profiles":[{"outer":["s2"],"holes":[],"extra":true}]}),
            json!({"type":"whole_boundaries","profiles":[{"outer":["s2"],"holes":[]},{"outer":["s2"],"holes":[]}]}),
            json!({"type":"whole_boundaries","profiles":[{"outer":["x".repeat(513)],"holes":[]}]}),
            json!({"type":"whole_boundaries","profiles":[{"outer":["a\nb"],"holes":[]}]}),
        ];
        for value in malformed {
            assert!(whole_profile_selection_from_value(&value).is_err(), "accepted malformed reference {value}");
            assert!(validate_raw(&json!({"features":[{"type":"extrude","profiles":value}]})).is_err());
        }
        let many = (0..129).map(|index| json!({"outer":[format!("c{index:03}")],"holes":[]})).collect::<Vec<_>>();
        assert!(whole_profile_selection_from_value(&json!({"type":"whole_boundaries","profiles":&many[..128]})).is_ok());
        assert!(whole_profile_selection_from_value(&json!({"type":"whole_boundaries","profiles":many})).is_err());
        let oversized = (0..513).map(|index| format!("c{index:03}")).collect::<Vec<_>>();
        assert!(whole_profile_selection_from_value(&json!({"type":"whole_boundaries","profiles":[{"outer":oversized,"holes":[]}]})).is_err());
        let too_many_holes = (0..129).map(|index| vec![format!("hole{index:03}")]).collect::<Vec<_>>();
        assert!(
            whole_profile_selection_from_value(&json!({"type":"whole_boundaries","profiles":[{"outer":["outer"],"holes":too_many_holes}]})).is_err()
        );
        let aggregate = (0..40)
            .map(|profile| {
                let outer = (0..512).map(|curve| format!("profile{profile:03}curve{curve:03}")).collect::<Vec<_>>();
                json!({"outer":outer,"holes":[]})
            })
            .collect::<Vec<_>>();
        assert!(whole_profile_selection_from_value(&json!({"type":"whole_boundaries","profiles":aggregate})).is_err());
    }

    #[test]
    fn raw_offset_roles_reject_aliases_order_damage_and_hostile_shapes() {
        let good = fixture();
        assert!(validate_raw(&good).is_ok());
        let path = "/features/0/sketch/offsets/0";
        let mut flags = good.clone();
        flags.pointer_mut(&format!("{path}/topology/roles/1")).unwrap()["flags"] =
            json!({"construction":true,"centerline":false,"reversed":true,"fixed":false});
        assert!(validate_raw(&flags).is_ok());
        *flags.pointer_mut(&format!("{path}/topology/roles/1/flags/construction")).unwrap() = json!(1);
        assert!(validate_raw(&flags).is_err());
        for (suffix, replacement) in [
            ("/topology/version", json!(2)),
            ("/topology/id", json!("x".repeat(513))),
            ("/closed", json!(false)),
            ("/distance", json!(1e9)),
            ("/outputs", json!(["l1", "l0", "l2"])),
            ("/outputs", json!(["l0", "a0", "l1", "l2"])),
            ("/topology/roles/1/kind", json!("line")),
            ("/topology/roles/1/output", json!("l0")),
            ("/topology/roles/1/start", json!("a0")),
            ("/topology/roles/0/kind", json!("guess")),
            ("/sources/0/1", json!(0)),
            ("/sources/0/0", json!("l0")),
            ("/topology/roles", json!(vec![json!({}); 513])),
        ] {
            let mut damaged = good.clone();
            *damaged.pointer_mut(&format!("{path}{suffix}")).unwrap() = replacement;
            assert!(validate_raw(&damaged).is_err(), "accepted {suffix}");
        }
        let mut extra = good.clone();
        extra.pointer_mut(&format!("{path}/topology")).unwrap()["fallback"] = json!([]);
        assert!(validate_raw(&extra).is_err());
        let mut excessive = good.clone();
        let chain = excessive.pointer(path).unwrap().clone();
        let groups = (0..129)
            .map(|index| {
                let mut chain = chain.clone();
                chain["topology"]["id"] = json!(format!("o{index}"));
                for source in chain["sources"].as_array_mut().unwrap() {
                    source[0] = json!(format!("g{index}-{}", source[0].as_str().unwrap()));
                }
                for output in chain["outputs"].as_array_mut().unwrap() {
                    *output = json!(format!("g{index}-{}", output.as_str().unwrap()));
                }
                for role in chain["topology"]["roles"].as_array_mut().unwrap() {
                    for key in ["output", "start", "end"] {
                        role[key] = json!(format!("g{index}-{}", role[key].as_str().unwrap()));
                    }
                }
                chain
            })
            .collect::<Vec<_>>();
        excessive["features"][0]["sketch"]["offsets"] = json!(&groups[..128]);
        assert!(validate_raw(&excessive).is_ok(), "the permitted group boundary must be accepted");
        excessive["features"][0]["sketch"]["offsets"] = json!(groups);
        assert!(validate_raw(&excessive).is_err());
    }

    #[test]
    fn raw_boundary_references_are_bounded_and_future_versions_refuse_first() {
        let mut good = fixture();
        good["features"].as_array_mut().unwrap().push(json!({"type":"extrude","profiles":{"type":"offset_boundaries","associations":["o1"]}}));
        assert!(validate_raw(&good).is_ok());
        for associations in [json!([]), json!(["o1", "o1"]), json!(["x".repeat(513)]), json!(vec!["o"; 129]), json!([0])] {
            let mut damaged = good.clone();
            damaged["features"][1]["profiles"]["associations"] = associations;
            assert!(validate_raw(&damaged).is_err());
            damaged["format"] = json!(crate::format::format_string(crate::format::FORMAT + 1));
            assert!(crate::format::read(&damaged.to_string()).unwrap_err().to_string().contains("newer SolidCraft"));
        }
        // Absent metadata preserves the historical one-to-one chain representation.
        let mut legacy = fixture();
        legacy["features"][0]["sketch"]["offsets"][0].as_object_mut().unwrap().remove("topology");
        legacy["features"][0]["sketch"]["offsets"][0]["closed"] = json!(false);
        assert!(validate_raw(&legacy).is_ok());
    }

    #[test]
    fn formats_one_through_eleven_keep_legacy_offset_geometry_and_ids() {
        let mut sketch = solidcraft_sketch::Sketch::new();
        let center = sketch.add_point(solidcraft_geom::Vec2::new(4., 7.), Some("center")).unwrap();
        sketch.add_circle(solidcraft_geom::Vec2::new(4., 7.), 10., Some(center), Some("source")).unwrap();
        sketch.add_circle(solidcraft_geom::Vec2::new(4., 7.), 9., Some(center), Some("offset")).unwrap();
        sketch.offsets.push(solidcraft_sketch::OffsetChain {
            sources: vec![("source".into(), true)],
            outputs: vec!["offset".into()],
            distance: 1.,
            closed: true,
            topology: None,
        });
        let mut document = Document::new("legacy offset circles");
        let sketch_id =
            document.add_feature(FeatureKind::Sketch { plane: crate::PlaneRef::Origin { name: "XY".into() }, sketch }, Some("Circles")).unwrap();
        let original: Value = serde_json::from_str(&document.to_json()).unwrap();
        for version in 1..=11 {
            let mut old = original.clone();
            old["format"] = json!(crate::format::format_string(version));
            let upgraded = crate::format::read(&old.to_string()).unwrap();
            let saved: Value = serde_json::from_str(&upgraded.to_json()).unwrap();
            assert_eq!(saved["features"][0]["sketch"]["offsets"], original["features"][0]["sketch"]["offsets"]);
            assert_eq!(saved["features"][0]["sketch"]["curves"], original["features"][0]["sketch"]["curves"]);
            assert_eq!(saved["features"][0]["sketch"]["points"], original["features"][0]["sketch"]["points"]);
            let mut model = crate::Model::new();
            model.evaluate(&upgraded);
            assert!(model.results.iter().all(|result| result.error.is_none()));
            let state = model.state();
            let solved = state.sketch(sketch_id).unwrap();
            let radius = solved.sketch.curves.iter().find(|curve| curve.id == "offset").map(|curve| &curve.kind);
            assert!(matches!(radius,Some(solidcraft_sketch::CurveKind::Circle {r,..}) if *r ==9.));
        }
    }

    #[test]
    fn missing_boundary_or_defining_sketch_reopens_as_a_model_error_without_retarget() {
        for missing_sketch in [false, true] {
            let mut sketch = solidcraft_sketch::Sketch::new();
            sketch.add_circle(solidcraft_geom::Vec2::new(0., 0.), 10., None, Some("unrelated-circle")).unwrap();
            let mut document = Document::new("repairable lost boundary");
            let id = document.add_feature(FeatureKind::Sketch { plane: crate::PlaneRef::Origin { name: "XY".into() }, sketch }, None).unwrap();
            document
                .add_feature(
                    FeatureKind::Extrude {
                        thin: None,
                        sketch: crate::SketchId(if missing_sketch { 999_999 } else { id }),
                        profiles: ProfileSel::OffsetBoundaries { associations: vec!["deleted-o1".into()] },
                        extent: crate::Extent {
                            convention: crate::ExtrudeConvention::LegacyPerSide,
                            distance: "2".into(),
                            direction: crate::Direction::Positive,
                            distance2: None,
                            start_offset: None,
                            through_all: false,
                            taper: None,
                            to: None,
                            to_offset: None,
                            from: None,
                        },
                        operation: crate::Operation::NewBody,
                        targets: vec![],
                    },
                    None,
                )
                .unwrap();
            let reopened = crate::format::read(&document.to_json()).unwrap();
            assert_eq!(reopened, document);
            let mut model = crate::Model::new();
            model.evaluate(&reopened);
            assert!(model.results.last().unwrap().error.is_some());
            assert!(model.state().bodies.is_empty(), "lost boundary must not select the remaining ordinary circle");
        }
    }
}
