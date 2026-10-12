//! The design file format version and the migrations that bring older files up to date.
//!
//! A design file starts with `"format": "solvecraft/N"`. A file of an older format is read and
//! upgraded step by step (N → N+1 → … → current): JSON-level steps first, for changes in the
//! file's shape, then document-level steps. A file of a newer format than this build knows is
//! refused with a message saying so, never guessed at.
//!
//! History:
//! - 1: every design up to 2026-10-08 (format string since M0). Fields added over time all
//!   have defaults; designs from before feature input names and before occurrences are filled
//!   in on upgrade.
//! - 2: the same shape; files say 2 once they have been upgraded, so later readers can tell.
//! - 3: appearances per body, face and component with names and opacity
//!   (`"appearances": {"bodies": {name: {name, color, opacity}}, "faces": […], "components": {…}}`);
//!   before, `"appearances"` mapped body names to bare `[r, g, b]` colours.
//! - 5: joins, cuts and intersects change only their own component's bodies unless bodies are
//!   named as `participants`; older features in designs with components get `participants:
//!   ["*"]` (every body), which is what they did, so those designs keep their shape.
//! - 4: joint origins mate by default (B's z against A's, as Fusion does); `flip` now means
//!   "don't mate". Older joints keep their placement: their `flip` is inverted.
//! - 6: XZ origin planes use sketch y = -Z and normal +Y. Older XZ references become
//!   explicit planes retaining their previous axes, including offset-plane expressions.
//! - 7: configuration tables can include body material and appearance columns. Version 6
//!   rows need no transformation; the version stamp lets older readers request an update
//!   instead of encountering an unknown column variant.
//! - 8: ordered configuration rules and persistent signed line-angle, arc-sweep and slot
//!   dimensions. Version 7 geometry and table values retain their meaning without changes.
//! - 9: configured component/semantic-face styles, associative free-form offsets and
//!   explicit cylindrical emboss targets. Existing version 8 geometry and rules are
//!   preserved; new fields use defaults rather than guessing a target or an association.
//! - 10: timeline body transfers, explicit 3D wire coordinate spaces and certified face
//!   bindings. Legacy ownership maps remain unchanged. Retained wire associations determine
//!   their space; saved coordinates are never rewritten during migration.
//! - 11: durable sketch face anchors, explicitly periodic fit splines and versioned revolve
//!   conventions. Absent markers retain legacy attachment and interpolation behavior.
//! - 12: persistent analytic offset boundary roles and association profile references.
//!   Older analytic chains retain their source/output correspondence and saved geometry;
//!   migration never invents retired entities or renumbers existing curves.
//! - 13: exact planar shell face anchors retain the shell producer, original certified
//!   source and distinct inner/outer role. Version 12 anchors retain their identities;
//!   migration never guesses attachments for older designs.

use serde_json::Value;

use crate::{DocError, Document, Result};

/// The format this build writes.
pub const FORMAT: u32 = 13;

pub fn format_string(n: u32) -> String {
    format!("solvecraft/{n}")
}

/// The version in a format string (`solvecraft/N`).
pub fn version_of(s: &str) -> Option<u32> {
    s.strip_prefix("solvecraft/")?.parse().ok()
}

/// JSON-level upgrades: entry `i` turns format `i + 1` into `i + 2`.
type JsonStep = fn(&mut Value) -> Result<()>;
const JSON_STEPS: [JsonStep; 12] =
    [|_| Ok(()), v2_to_v3, v3_to_v4, v4_to_v5, v5_to_v6, |_| Ok(()), |_| Ok(()), |_| Ok(()), |_| Ok(()), |_| Ok(()), |_| Ok(()), |_| Ok(())];

/// Old XZ references retain their exact frame, including nested offset and angled planes.
/// New origin references use +Y; freezing the old frame avoids changing sketch coordinates,
/// dimensions, profiles, extrude directions or parameter expressions in saved designs.
fn v5_to_v6(v: &mut Value) -> Result<()> {
    fn preserve(v: &mut Value) {
        match v {
            Value::Object(o) => {
                let old_xz = o.get("type").and_then(Value::as_str) == Some("origin")
                    && o.get("name").and_then(Value::as_str).is_some_and(|n| n.eq_ignore_ascii_case("XZ") || n.eq_ignore_ascii_case("FRONT"));
                if old_xz {
                    *v = serde_json::json!({"type": "custom", "plane": {"origin": [0, 0, 0], "x_dir": [1, 0, 0], "y_dir": [0, 0, 1]}});
                } else {
                    o.values_mut().for_each(preserve);
                }
            }
            Value::Array(a) => a.iter_mut().for_each(preserve),
            _ => {}
        }
    }
    preserve(v);
    Ok(())
}

/// Features in designs with components keep reaching every body.
fn v4_to_v5(v: &mut Value) -> Result<()> {
    let has_components = v.get("components").and_then(Value::as_array).is_some_and(|c| !c.is_empty());
    if !has_components {
        return Ok(());
    }
    if let Some(Value::Array(fs)) = v.get_mut("features") {
        for f in fs.iter_mut() {
            if let Some(o) = f.as_object_mut()
                && o.get("type").and_then(Value::as_str) != Some("sketch")
                && o.get("participants").is_none()
            {
                o.insert("participants".into(), serde_json::json!(["*"]));
            }
        }
    }
    Ok(())
}

/// Joints placed with the old convention (aligned unless flipped) keep their placement.
fn v3_to_v4(v: &mut Value) -> Result<()> {
    if let Some(Value::Array(js)) = v.get_mut("assembly").and_then(|a| a.get_mut("joints")) {
        for j in js.iter_mut() {
            if let Some(o) = j.as_object_mut() {
                let f = o.get("flip").and_then(Value::as_bool).unwrap_or(false);
                o.insert("flip".into(), Value::Bool(!f));
            }
        }
    }
    Ok(())
}

/// Body colours `{name: [r, g, b]}` become named, opaque body appearances.
fn v2_to_v3(v: &mut Value) -> Result<()> {
    let Some(Value::Object(old)) = v.get("appearances") else { return Ok(()) };
    if !old.values().all(Value::is_array) {
        return Ok(());
    }
    let bodies: serde_json::Map<String, Value> =
        old.iter().map(|(k, c)| (k.clone(), serde_json::json!({"name": "Custom", "color": c, "opacity": 1.0}))).collect();
    if let Some(o) = v.as_object_mut() {
        o.insert("appearances".into(), serde_json::json!({ "bodies": bodies }));
    }
    Ok(())
}

/// Read a design file's JSON, upgrading older formats.
pub fn read(s: &str) -> Result<Document> {
    let mut v: Value = serde_json::from_str(s).map_err(|e| DocError::Invalid(format!("not a SolidCraft design: {e}")))?;
    let Some(obj) = v.as_object() else {
        return Err(DocError::Invalid("not a SolidCraft design (expected a JSON object)".into()));
    };
    let version = match obj.get("format") {
        None => 1,
        Some(Value::String(f)) => version_of(f).ok_or_else(|| DocError::Invalid(format!("not a SolidCraft design (format `{f}`)")))?,
        Some(_) => return Err(DocError::Invalid("not a SolidCraft design (format is not a string)".into())),
    };
    if version == 0 {
        return Err(DocError::Invalid("not a SolidCraft design (format 0)".into()));
    }
    if version > FORMAT {
        return Err(DocError::Invalid(format!(
            "this design was saved by a newer SolidCraft (file format {version}; this version reads formats up to {FORMAT}): update SolidCraft to open it"
        )));
    }
    check_numbers(&v, 0)?;
    crate::config::validate_rules_json(&v)?;
    crate::config::validate_styles_json(&v)?;
    crate::face_anchor::validate_face_anchors_json(&v)?;
    validate_versioned_geometry(&v)?;
    validate_emboss_targets(&v)?;
    validate_freeform_offsets(&v)?;
    validate_sketch_sizes(&v)?;
    crate::format_offsets::validate_raw(&v)?;
    let legacy_wires = validate_wires(&v, version)?;
    validate_body_transfers(&v)?;
    for step in JSON_STEPS.iter().skip(version as usize - 1).take((FORMAT - version) as usize) {
        step(&mut v)?;
    }
    let mut d: Document = serde_json::from_value(v).map_err(|e| DocError::Invalid(format!("design file: {e}")))?;
    let mut legacy_by_feature = std::collections::BTreeMap::<usize, Vec<usize>>::new();
    for (feature, wire) in legacy_wires {
        legacy_by_feature.entry(feature).or_default().push(wire);
    }
    for (feature_index, wire_indices) in legacy_by_feature {
        if let Some(crate::Feature { kind: crate::FeatureKind::Sketch { sketch, .. }, .. }) = d.features.get_mut(feature_index) {
            let mut spaces = std::collections::BTreeMap::new();
            for link in &sketch.links {
                spaces.entry(link.id.as_str()).or_insert_with(|| solvecraft_sketch::WireSpace::from_legacy_link(link));
            }
            for wire_index in wire_indices {
                if let Some(wire) = sketch.wires.get_mut(wire_index) {
                    wire.space = wire.link.as_deref().and_then(|id| spaces.get(id)).copied().unwrap_or_default();
                }
            }
        }
    }
    if d.features.len() > crate::document::MAX_FEATURES
        || d.params.len() > crate::document::MAX_PARAMS
        || d.components.len() > crate::document::MAX_FEATURES
    {
        return Err(DocError::Invalid("document too large".into()));
    }
    for o in &d.occurrences {
        if !rigid(&o.transform) {
            return Err(DocError::Invalid(format!("damaged design: occurrence `{}` has an invalid placement", o.name)));
        }
    }
    let component_ids: std::collections::BTreeSet<_> = d.components.iter().map(|component| component.id).chain(std::iter::once(0)).collect();
    for feature in &d.features {
        if let crate::FeatureKind::Sketch { sketch, .. } = &feature.kind {
            sketch.validate_freeform_offsets().map_err(|e| DocError::Invalid(format!("damaged design: {e}")))?;
            sketch.validate_wires().map_err(|e| DocError::Invalid(format!("damaged design: {e}")))?;
            sketch.validate_periodic_splines().map_err(|e| DocError::Invalid(format!("damaged design: {e}")))?;
        }
        if let crate::FeatureKind::BodyTransfer { from, to, .. } = &feature.kind
            && [from, to].into_iter().any(|id| !component_ids.contains(id))
        {
            return Err(DocError::Invalid("damaged design: body transfer references an unknown component".into()));
        }
        if let crate::FeatureKind::BodyTransfer { to, .. } = &feature.kind
            && feature.component != *to
        {
            return Err(DocError::Invalid("damaged design: body transfer owner must match its destination".into()));
        }
    }
    // Fill in what older designs (and hand-edited files) may lack; a no-op otherwise.
    crate::format_offsets::validate_typed(&d)?;
    normalise(&mut d)?;
    d.format = format_string(FORMAT);
    Ok(d)
}

/// The caps interactive editing enforces, applied to the raw JSON so an oversized sketch is
/// never deserialised. Every sketch in the timeline is solved when the design opens, and the
/// solver sizes its dense matrices by the sketch, so a file with hundreds of thousands of
/// points would ask the allocator for terabytes and abort before anything could report it.
fn validate_sketch_sizes(value: &Value) -> Result<()> {
    let Some(features) = value.get("features").and_then(Value::as_array) else { return Ok(()) };
    let invalid = || DocError::Invalid("damaged design: sketch too large".into());
    for feature in features {
        if feature.get("type").and_then(Value::as_str) != Some("sketch") {
            continue;
        }
        let Some(sketch) = feature.get("sketch") else { continue };
        let len = |key: &str| sketch.get(key).and_then(Value::as_array).map_or(0, Vec::len);
        if len("points") > solvecraft_sketch::MAX_POINTS
            || len("curves") > solvecraft_sketch::MAX_CURVES
            || len("constraints") > solvecraft_sketch::MAX_CONSTRAINTS
        {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Bound arrays before typed allocation; only old files with absent markers need migration.
fn validate_wires(value: &Value, version: u32) -> Result<Vec<(usize, usize)>> {
    let invalid = || DocError::Invalid("damaged design: 3D curve arrays, ids or coordinate spaces are invalid".into());
    let mut legacy = Vec::new();
    let Some(features) = value.get("features").and_then(Value::as_array) else { return Ok(legacy) };
    if features.len() > crate::document::MAX_FEATURES {
        return Err(invalid());
    }
    for (feature_index, feature) in features.iter().enumerate() {
        if feature.get("type").and_then(Value::as_str) != Some("sketch") {
            continue;
        }
        let Some(wires) = feature.get("sketch").and_then(|sketch| sketch.get("wires")) else { continue };
        let wires = wires.as_array().ok_or_else(invalid)?;
        if wires.len() > 20_000 {
            return Err(invalid());
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut work = 0usize;
        for (wire_index, wire) in wires.iter().enumerate() {
            let id = wire.get("id").and_then(Value::as_str).ok_or_else(invalid)?;
            if id.is_empty() || id.len() > 512 || !ids.insert(id) {
                return Err(invalid());
            }
            if let Some(link) = wire.get("link").filter(|link| !link.is_null())
                && link.as_str().is_none_or(|link| link.is_empty() || link.len() > 512)
            {
                return Err(invalid());
            }
            match wire.get("space") {
                None if version < 10 => legacy.push((feature_index, wire_index)),
                None => {}
                Some(Value::String(space)) if matches!(space.as_str(), "world" | "component") => {}
                _ => return Err(invalid()),
            }
            let pts = wire.get("pts").and_then(Value::as_array).ok_or_else(invalid)?;
            let fit = match wire.get("fit") {
                None => &[][..],
                Some(fit) => fit.as_array().ok_or_else(invalid)?.as_slice(),
            };
            work = work.saturating_add(pts.len()).saturating_add(fit.len());
            if work > 1_000_000 || pts.is_empty() || pts.len() > 100_000 || fit.len() > 500 || (pts.len() == 1 && fit.len() != 1) {
                return Err(invalid());
            }
            for point in pts.iter().chain(fit) {
                let point = point.as_array().ok_or_else(invalid)?;
                if point.len() != 3 || point.iter().any(|coordinate| coordinate.as_f64().is_none_or(|n| !n.is_finite() || n.abs() >= MAX_REAL)) {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(legacy)
}

fn validate_body_transfers(value: &Value) -> Result<()> {
    let invalid = || DocError::Invalid("damaged design: body transfer needs bounded unique bodies, components and a rigid placement".into());
    let Some(features) = value.get("features").and_then(Value::as_array) else { return Ok(()) };
    for feature in features {
        if feature.get("type").and_then(Value::as_str) != Some("body_transfer") {
            continue;
        }
        let bodies = feature.get("bodies").and_then(Value::as_array).ok_or_else(invalid)?;
        if bodies.is_empty() || bodies.len() > 1000 {
            return Err(invalid());
        }
        let mut names = std::collections::BTreeSet::new();
        for body in bodies {
            let body = body.as_str().ok_or_else(invalid)?;
            if body.trim().is_empty() || body.len() > 512 || body.chars().any(char::is_control) || !names.insert(body) {
                return Err(invalid());
            }
        }
        for key in ["from", "to"] {
            if feature.get(key).and_then(Value::as_u64).is_none() {
                return Err(invalid());
            }
        }
        let owner = match feature.get("component") {
            None => 0,
            Some(value) => value.as_u64().ok_or_else(invalid)?,
        };
        if feature.get("to").and_then(Value::as_u64) != Some(owner) {
            return Err(DocError::Invalid("damaged design: body transfer owner must match its destination".into()));
        }
        let matrix = feature.get("transform").and_then(Value::as_array).ok_or_else(invalid)?;
        if matrix.len() != 4 {
            return Err(invalid());
        }
        let mut placement = [[0.0; 4]; 4];
        for (out, column) in placement.iter_mut().zip(matrix) {
            let column = column.as_array().ok_or_else(invalid)?;
            if column.len() != 4 {
                return Err(invalid());
            }
            for (out, number) in out.iter_mut().zip(column) {
                *out = number.as_f64().ok_or_else(invalid)?;
            }
        }
        if !crate::body_transfer::valid_transfer_matrix(&placement) {
            return Err(invalid());
        }
    }
    Ok(())
}

/// Validate new explicit targets before typed decoding. Older planar features have none.
fn validate_emboss_targets(value: &Value) -> Result<()> {
    let Some(features) = value.get("features").and_then(Value::as_array) else { return Ok(()) };
    if features.len() > crate::document::MAX_FEATURES {
        return Err(DocError::Invalid("document too large".into()));
    }
    for feature in features {
        if feature.get("type").and_then(Value::as_str) != Some("emboss") {
            continue;
        }
        let Some(target) = feature.get("face").filter(|target| !target.is_null()) else { continue };
        let invalid = || DocError::Invalid("damaged design: an emboss face needs a bounded body name and three finite coordinates".into());
        let object = target.as_object().ok_or_else(invalid)?;
        if object.keys().any(|key| !matches!(key.as_str(), "body" | "at" | "path" | "author_path")) {
            return Err(invalid());
        }
        let body = object.get("body").and_then(Value::as_str).ok_or_else(invalid)?;
        let point = object.get("at").and_then(Value::as_array).ok_or_else(invalid)?;
        if body.trim().is_empty()
            || body.len() > 512
            || body.chars().any(char::is_control)
            || point.len() != 3
            || point.iter().any(|coordinate| coordinate.as_f64().is_none_or(|number| !number.is_finite() || number.abs() > MAX_REAL))
        {
            return Err(invalid());
        }
        for path in ["path", "author_path"].into_iter().filter_map(|key| object.get(key)) {
            let path = path.as_array().ok_or_else(invalid)?;
            if path.len() > 64 {
                return Err(invalid());
            }
            let mut ids = std::collections::BTreeSet::new();
            for id in path {
                let id = id.as_u64().filter(|id| *id != 0).ok_or_else(invalid)?;
                if !ids.insert(id) {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(())
}

/// Bound explicitly versioned geometry before allocating typed spline point arrays.
fn validate_versioned_geometry(value: &Value) -> Result<()> {
    let invalid = || DocError::Invalid("damaged design: invalid periodic spline or revolve convention".into());
    let Some(features) = value.get("features").and_then(Value::as_array) else { return Ok(()) };
    for feature in features {
        if feature.get("type").and_then(Value::as_str) == Some("revolve")
            && let Some(convention) = feature.get("convention")
            && !matches!(convention.as_str(), Some("legacy_axis_start" | "sketch_normal"))
        {
            return Err(invalid());
        }
        if feature.get("type").and_then(Value::as_str) != Some("sketch") {
            continue;
        }
        let Some(sketch) = feature.get("sketch") else { continue };
        if let Some(links) = sketch.get("links").and_then(Value::as_array) {
            for link in links {
                if link.get("auto_face").is_some_and(|marker| !marker.is_boolean()) {
                    return Err(invalid());
                }
            }
        }
        let Some(curves) = sketch.get("curves").and_then(Value::as_array) else { continue };
        let points = sketch.get("points").and_then(Value::as_array);
        let mut work = 0usize;
        for curve in curves {
            if curve.get("type").and_then(Value::as_str) != Some("spline") {
                continue;
            }
            let Some(marker) = curve.get("periodic") else { continue };
            let periodic = marker.as_bool().ok_or_else(invalid)?;
            if !periodic {
                continue;
            }
            let indices = curve.get("pts").and_then(Value::as_array).ok_or_else(invalid)?;
            work = work.saturating_add(indices.len());
            if !(4..=solvecraft_sketch::MAX_SPLINE_POINTS).contains(&indices.len())
                || work > 1_000_000
                || curve.get("control").is_some_and(|control| control.as_bool() != Some(false))
                || curve.get("degree").is_some_and(|degree| degree.as_u64() != Some(3))
            {
                return Err(invalid());
            }
            let count = points.ok_or_else(invalid)?.len();
            if indices.iter().any(|index| index.as_u64().is_none_or(|index| index >= count as u64)) {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

fn validate_freeform_offsets(value: &Value) -> Result<()> {
    let Some(features) = value.get("features").and_then(Value::as_array) else { return Ok(()) };
    for feature in features {
        if feature.get("type").and_then(Value::as_str) != Some("sketch") {
            continue;
        }
        let Some(records) = feature.get("sketch").and_then(|sketch| sketch.get("freeform_offsets")) else { continue };
        let invalid = || DocError::Invalid("damaged design: invalid free-form offset metadata".into());
        let records = records.as_array().ok_or_else(invalid)?;
        if records.len() > 128 {
            return Err(invalid());
        }
        let periodic_outputs: std::collections::BTreeSet<_> = records
            .iter()
            .filter(|record| record.get("certificate").and_then(Value::as_u64) == Some(2))
            .filter_map(|record| record.get("output").and_then(Value::as_str))
            .collect();
        let mut periodic_curves = std::collections::BTreeMap::new();
        if !periodic_outputs.is_empty()
            && let Some(curves) = feature.get("sketch").and_then(|sketch| sketch.get("curves")).and_then(Value::as_array)
        {
            if curves.len() > 1_000_000 {
                return Err(invalid());
            }
            for curve in curves {
                if let Some(id) = curve.get("id").and_then(Value::as_str)
                    && periodic_outputs.contains(id)
                    && periodic_curves.insert(id, curve).is_some()
                {
                    return Err(invalid());
                }
            }
        }
        let mut outputs = std::collections::BTreeSet::new();
        for record in records {
            let object = record.as_object().ok_or_else(invalid)?;
            if object.len() != 9
                || object.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "source" | "output" | "forward" | "distance" | "divisions" | "pieces" | "closed" | "tolerance" | "certificate"
                    )
                })
            {
                return Err(invalid());
            }
            let source = record.get("source").and_then(Value::as_str).ok_or_else(invalid)?;
            let output = record.get("output").and_then(Value::as_str).ok_or_else(invalid)?;
            if [source, output].iter().any(|id| id.is_empty() || id.len() > 512 || id.chars().any(char::is_control))
                || source == output
                || !outputs.insert(output)
            {
                return Err(invalid());
            }
            let divisions = record.get("divisions").and_then(Value::as_u64).ok_or_else(invalid)?;
            let pieces = record.get("pieces").and_then(Value::as_u64).ok_or_else(invalid)?;
            let distance = record.get("distance").and_then(Value::as_f64).ok_or_else(invalid)?;
            let tolerance = record.get("tolerance").and_then(Value::as_f64).ok_or_else(invalid)?;
            if !(1..=256).contains(&divisions)
                || !divisions.is_power_of_two()
                || !(1..=256).contains(&pieces)
                || pieces * divisions >= solvecraft_sketch::MAX_SPLINE_POINTS as u64
                || !distance.is_finite()
                || !(1e-9..=MAX_REAL).contains(&distance.abs())
                || !tolerance.is_finite()
                || !(tolerance > 0.0 && tolerance <= 0.002)
                || !matches!(record.get("certificate").and_then(Value::as_u64), Some(1 | 2))
                || record.get("forward").and_then(Value::as_bool).is_none()
                || record.get("closed").and_then(Value::as_bool).is_none()
            {
                return Err(invalid());
            }
            if record.get("certificate").and_then(Value::as_u64) == Some(2) {
                let output_curve = periodic_curves.get(output);
                if record.get("closed").and_then(Value::as_bool) != Some(true)
                    || output_curve.is_none_or(|curve| {
                        curve.get("type").and_then(Value::as_str) != Some("spline") || curve.get("periodic").and_then(Value::as_bool) != Some(true)
                    })
                {
                    return Err(invalid());
                }
            }
        }
        let mut resolved = std::collections::BTreeSet::new();
        for record in records {
            let source = record.get("source").and_then(Value::as_str).ok_or_else(invalid)?;
            let output = record.get("output").and_then(Value::as_str).ok_or_else(invalid)?;
            if outputs.contains(source) && !resolved.contains(source) {
                return Err(invalid());
            }
            resolved.insert(output);
        }
    }
    Ok(())
}

/// Largest coordinate or size a design may hold (mm): a kilometre-scale part is already far
/// outside what the kernel's tolerances serve.
const MAX_REAL: f64 = 1e9;

/// Every non-integer number in the file is finite and within ±`MAX_REAL` (a damaged or hostile
/// file is refused instead of overflowing the geometry later).
fn check_numbers(v: &Value, depth: usize) -> Result<()> {
    if depth > 64 {
        return Err(DocError::Invalid("damaged design: nested too deeply".into()));
    }
    match v {
        Value::Number(n) if !(n.is_u64() || n.is_i64()) => {
            let x = n.as_f64().unwrap_or(f64::NAN);
            if !(x.is_finite() && x.abs() <= MAX_REAL) {
                return Err(DocError::Invalid(format!("damaged design: the number {n} is out of range")));
            }
        }
        Value::Array(a) => a.iter().try_for_each(|x| check_numbers(x, depth + 1))?,
        Value::Object(m) => m.values().try_for_each(|x| check_numbers(x, depth + 1))?,
        _ => {}
    }
    Ok(())
}

/// A placement: rotation (orthonormal, not mirrored) and a translation in range.
fn rigid(m: &crate::Mat) -> bool {
    let row = |i: usize| m.get(i).copied().unwrap_or_default();
    let (x, y, z, t) = (row(0), row(1), row(2), row(3));
    let dot = |a: [f64; 4], b: [f64; 4]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let unit = |a: [f64; 4]| (dot(a, a) - 1.0).abs() < 1e-6;
    let det = x[0] * (y[1] * z[2] - y[2] * z[1]) - x[1] * (y[0] * z[2] - y[2] * z[0]) + x[2] * (y[0] * z[1] - y[1] * z[0]);
    m.iter().flatten().all(|v| v.is_finite())
        && unit(x)
        && unit(y)
        && unit(z)
        && dot(x, y).abs() < 1e-6
        && dot(y, z).abs() < 1e-6
        && dot(x, z).abs() < 1e-6
        && det > 0.0
        && [x[3], y[3], z[3]].iter().all(|v| v.abs() < 1e-9)
        && (t[3] - 1.0).abs() < 1e-9
        && t[..3].iter().all(|v| v.abs() <= MAX_REAL)
}

/// Feature input names and component occurrences, which format 1 files may predate.
fn normalise(d: &mut Document) -> Result<()> {
    d.name_all_feature_inputs();
    let missing: Vec<(u64, u64)> =
        d.components.iter().filter(|c| !d.occurrences.iter().any(|o| o.component == c.id)).map(|c| (c.id, c.parent)).collect();
    for (c, p) in missing {
        d.add_occurrence(c, p, crate::IDENTITY)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_nine_wire_spaces_preserve_saved_points_and_explicit_markers() {
        use solvecraft_geom::Vec3;
        use solvecraft_sketch::{LinkKind, LinkSource, Sketch, WireSpace};
        let mut sketch = Sketch::new();
        sketch.add_drawn_wire(&[Vec3::new(12.0, 3.0, 4.0)]).unwrap();
        let link = sketch
            .add_link_with_wires(
                LinkKind::Include,
                LinkSource::Edge { body: "Missing".into(), at: Vec3::ZERO },
                &[],
                &[vec![Vec3::new(2.0, 7.0, 9.0), Vec3::new(2.0, 8.0, 9.0)]],
            )
            .unwrap();
        sketch.set_link_lost(&link, true);
        let mut doc = Document::new("legacy wire spaces");
        doc.add_feature(crate::FeatureKind::Sketch { plane: crate::PlaneRef::Origin { name: "XY".into() }, sketch }, None).unwrap();
        let mut json: Value = serde_json::from_str(&doc.to_json()).unwrap();
        json["format"] = Value::from("solvecraft/9");
        for wire in json["features"][0]["sketch"]["wires"].as_array_mut().unwrap() {
            wire.as_object_mut().unwrap().remove("space");
        }
        let old = json["features"][0]["sketch"]["wires"].clone();
        let migrated = read(&json.to_string()).unwrap();
        let crate::FeatureKind::Sketch { sketch, .. } = &migrated.features[0].kind else { panic!("sketch") };
        assert_eq!(sketch.wires[0].space, WireSpace::World);
        assert_eq!(sketch.wires[1].space, WireSpace::Component);
        let saved: Value = serde_json::from_str(&migrated.to_json()).unwrap();
        for (a, b) in old.as_array().unwrap().iter().zip(saved["features"][0]["sketch"]["wires"].as_array().unwrap()) {
            assert_eq!(a["pts"], b["pts"]);
            assert_eq!(a["fit"], b["fit"]);
            assert_eq!(a["link"], b["link"]);
        }
        assert_eq!(read(&migrated.to_json()).unwrap(), migrated);
        json["features"][0]["sketch"]["wires"][1]["space"] = Value::from("world");
        let explicit = read(&json.to_string()).unwrap();
        let crate::FeatureKind::Sketch { sketch, .. } = &explicit.features[0].kind else { panic!("sketch") };
        assert_eq!(sketch.wires[1].space, WireSpace::World);
    }

    #[test]
    fn hostile_wire_and_transfer_shapes_are_refused_before_typed_decode() {
        let base: Value = serde_json::from_str(&Document::new("bounds").to_json()).unwrap();
        let wire = serde_json::json!({"id":"w1", "pts":[[1,2,3],[4,5,6]],"space":"world"});
        for damaged in [
            serde_json::json!({"space":"guess"}),
            serde_json::json!({"pts":[[1,2]]}),
            serde_json::json!({"pts":[[1e9,2,3],[4,5,6]]}),
            serde_json::json!({"fit": vec![[1,2,3];501]}),
        ] {
            let mut changed = wire.clone();
            for (key, value) in damaged.as_object().unwrap() {
                changed[key] = value.clone();
            }
            let mut file = base.clone();
            file["features"] = serde_json::json!([{"type":"sketch","sketch":{"wires":[changed]}}]);
            assert!(validate_wires(&file, FORMAT).is_err());
        }
        let transfer = serde_json::json!({"type":"body_transfer","component":1,"bodies":["Body1"],"from":0,"to":1,"transform":crate::IDENTITY});
        for damaged in [
            serde_json::json!({"bodies":["Body1","Body1"]}),
            serde_json::json!({"from":-1}),
            serde_json::json!({"component":0}),
            serde_json::json!({"transform":[[2,0,0,0],[0,1,0,0],[0,0,1,0],[0,0,0,1]]}),
            serde_json::json!({"bodies":["\n"]}),
        ] {
            let mut changed = transfer.clone();
            for (key, value) in damaged.as_object().unwrap() {
                changed[key] = value.clone();
            }
            let mut file = base.clone();
            file["features"] = serde_json::json!([changed]);
            assert!(validate_body_transfers(&file).is_err());
        }
    }

    #[test]
    fn recorded_transfer_roundtrips_and_unknown_component_is_refused() {
        let mut doc = Document::new("recorded ownership boundary");
        doc.add_feature(crate::FeatureKind::BodyTransfer { bodies: vec!["Body1".into()], from: 0, to: 0, transform: crate::IDENTITY }, None).unwrap();
        assert_eq!(read(&doc.to_json()).unwrap(), doc);
        let mut json: Value = serde_json::from_str(&doc.to_json()).unwrap();
        assert_eq!(json["features"][0]["type"], "body_transfer");
        json["features"][0]["to"] = Value::from(12345);
        json["features"][0]["component"] = Value::from(12345);
        assert!(read(&json.to_string()).unwrap_err().to_string().contains("unknown component"));
        json["features"][0]["to"] = Value::from(0);
        assert!(read(&json.to_string()).unwrap_err().to_string().contains("owner must match"));
        json["features"][0]["component"] = Value::from(0);
        json["features"][0]["transform"][0][0] = Value::from(2);
        assert!(read(&json.to_string()).unwrap_err().to_string().contains("rigid placement"));
    }

    #[test]
    fn version_eight_to_object_offset_keeps_existing_model_parameter_names() {
        use crate::{Direction, Extent, FeatureKind, Operation, PlaneRef, ProfileSel};
        let mut document = Document::new("legacy named extent");
        document.set_param("offset", "2", Some("mm"), None).unwrap();
        let sketch = document
            .add_feature(FeatureKind::Sketch { plane: PlaneRef::Origin { name: "XY".into() }, sketch: solvecraft_sketch::Sketch::new() }, None)
            .unwrap();
        document
            .add_feature(
                FeatureKind::Extrude {
                    sketch,
                    profiles: ProfileSel::All,
                    extent: Extent {
                        distance: "10".into(),
                        direction: Direction::Negative,
                        distance2: None,
                        start_offset: None,
                        through_all: false,
                        taper: Some("0 deg".into()),
                        to: Some(solvecraft_geom::Vec3::new(0.0, 0.0, 10.0)),
                        to_offset: Some("offset + 1 mm".into()),
                        from: Some(solvecraft_geom::Vec3::new(0.0, 0.0, 5.0)),
                    },
                    operation: Operation::NewBody,
                    targets: vec![],
                },
                None,
            )
            .unwrap();
        document
            .add_feature(
                FeatureKind::Box {
                    corner: ["0".into(), "0".into(), "0".into()],
                    length: "d1".into(),
                    width: "4".into(),
                    height: "6".into(),
                    operation: Operation::NewBody,
                },
                None,
            )
            .unwrap();
        let mut legacy: Value = serde_json::from_str(&document.to_json()).unwrap();
        legacy["format"] = Value::from("solvecraft/8");
        // Version 8 exposed distance and taper, but had no model slot for To Offset.
        legacy["features"][1]["param_names"] = serde_json::json!(["d1", "d2"]);
        legacy["features"][2]["param_names"] = serde_json::json!(["d3", "d4", "d5"]);
        let old_kind = legacy["features"][1].clone();
        let upgraded = read(&legacy.to_string()).unwrap();
        assert_eq!(&upgraded.features[1].param_names[..2], ["d1", "d2"]);
        assert_eq!(upgraded.features[2].param_names, ["d3", "d4", "d5"]);
        assert_eq!(upgraded.features[1].param_names.len(), 3);
        assert_eq!(upgraded.features[1].param_names[2], "d6");
        let saved: Value = serde_json::from_str(&upgraded.to_json()).unwrap();
        for field in ["extent", "sketch", "operation", "profiles"] {
            assert_eq!(saved["features"][1][field], old_kind[field]);
        }
        let (values, errors) = upgraded.param_values();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(values.get("d1").unwrap().v, 10.0);
        assert_eq!(values.get("d3").unwrap().v, 10.0);
        assert_eq!(values.get("d6").unwrap().v, 3.0);
        assert_eq!(read(&upgraded.to_json()).unwrap(), upgraded);
    }

    #[test]
    fn saved_offset_cannot_alias_and_overwrite_its_source_geometry() {
        use solvecraft_geom::Vec2;
        use solvecraft_sketch::{CurveKind, Sketch};
        let mut sketch = Sketch::new();
        let center = sketch.add_point(Vec2::ZERO, None).unwrap();
        let major = sketch.add_point(Vec2::new(10.0, 0.0), None).unwrap();
        let source = sketch.add_curve(CurveKind::Ellipse { c: center, m: major, r: 5.0 }, None).unwrap();
        let output = sketch.add_freeform_offset(source, true, -1.0).unwrap();
        assert!(sketch.validate_freeform_offsets().is_ok());
        if let CurveKind::Spline { pts, .. } = &mut sketch.curves[output].kind {
            pts[0] = center;
        } else {
            panic!("offset must produce a fit spline");
        }
        let mut document = Document::new("aliased offset geometry");
        document.add_feature(crate::FeatureKind::Sketch { plane: crate::PlaneRef::Origin { name: "XY".into() }, sketch }, None).unwrap();
        let error = read(&document.to_json()).unwrap_err().to_string();
        assert!(error.contains("damaged design"), "{error}");
    }

    #[test]
    fn structurally_valid_offset_records_must_reference_real_sketch_geometry() {
        let mut document = Document::new("missing offset geometry");
        document
            .add_feature(
                crate::FeatureKind::Sketch { plane: crate::PlaneRef::Origin { name: "XY".into() }, sketch: solvecraft_sketch::Sketch::new() },
                None,
            )
            .unwrap();
        let mut value: Value = serde_json::from_str(&document.to_json()).unwrap();
        value["features"][0]["sketch"]["freeform_offsets"] = serde_json::json!([{
            "source":"ellipse", "output":"offset", "forward":true, "distance":2.0,
            "divisions":8, "pieces":4, "closed":true, "tolerance":0.002, "certificate":1
        }]);
        assert!(validate_freeform_offsets(&value).is_ok());
        let error = read(&value.to_string()).unwrap_err().to_string();
        assert!(error.contains("damaged design"), "{error}");
    }

    #[test]
    fn malformed_face_anchors_are_refused_before_typed_decoding() {
        let anchor = serde_json::json!({
            "target":{"body":"Base", "origin_feature":1, "name":"Base:+z", "point":[0,0,10],
                "certificate":{"kind":"box", "axis":2, "high":true}},
            "component":0, "path":[], "author_path":[], "coordinates":[0,0], "x_axis":[1,0]
        });
        let make = |anchor: Value| {
            serde_json::json!({"features":[{"type":"sketch", "plane":{
                "type":"face", "anchor":anchor
            }}]})
        };
        assert!(crate::face_anchor::validate_face_anchors_json(&make(anchor.clone())).is_ok());
        let mut nested = serde_json::json!({"features":[{"type":"sketch", "plane":{
            "type":"offset", "base":{"type":"at_angle", "base":{"type":"face", "anchor":anchor.clone()}}
        }}]});
        assert!(crate::face_anchor::validate_face_anchors_json(&nested).is_ok());
        nested["features"][0]["plane"]["base"]["base"]["anchor"]["coordinates"] = serde_json::json!([0, 0, 0]);
        assert!(crate::face_anchor::validate_face_anchors_json(&nested).is_err());
        let mut malformed = Vec::new();
        for (key, damaged) in [
            ("component", serde_json::json!(-1)),
            ("path", serde_json::json!([0])),
            ("author_path", serde_json::json!((1..=65).collect::<Vec<u64>>())),
            ("coordinates", serde_json::json!([1_000_000_001_u64, 0])),
            ("coordinates", serde_json::json!([0])),
            ("x_axis", serde_json::json!([0, 0])),
            ("unknown", serde_json::json!(true)),
        ] {
            let mut candidate = anchor.clone();
            candidate[key] = damaged;
            malformed.push(candidate);
        }
        let mut missing_certificate = anchor.clone();
        missing_certificate["target"].as_object_mut().unwrap().remove("certificate");
        malformed.push(missing_certificate);
        let mut invalid_role = anchor;
        invalid_role["target"]["certificate"]["axis"] = Value::from(3);
        malformed.push(invalid_role);
        for candidate in malformed {
            let value = make(candidate);
            assert!(crate::face_anchor::validate_face_anchors_json(&value).is_err());
            let error = read(&value.to_string()).unwrap_err().to_string();
            assert!(!error.contains("design file:"), "must refuse before typed decode: {error}");
        }
    }

    #[test]
    fn periodic_markers_are_bounded_before_typed_decoding() {
        let curve = serde_json::json!({"id":"closed", "type":"spline", "pts":[0,1,2,3], "periodic":true});
        let make = |curve: Value| {
            serde_json::json!({"features":[{"type":"sketch", "sketch":{
                "points":[{}, {}, {}, {}], "curves":[curve]
            }}]})
        };
        assert!(validate_versioned_geometry(&make(curve.clone())).is_ok());
        for (key, malformed) in [
            ("periodic", serde_json::json!("true")),
            ("control", serde_json::json!(true)),
            ("degree", serde_json::json!(2)),
            ("pts", serde_json::json!([0, 1, 2])),
            ("pts", serde_json::json!([0, 1, 2, u64::MAX])),
            ("pts", serde_json::json!(vec![0; solvecraft_sketch::MAX_SPLINE_POINTS + 1])),
        ] {
            let mut damaged = curve.clone();
            damaged[key] = malformed;
            assert!(validate_versioned_geometry(&make(damaged)).is_err(), "{key}");
        }
        let mut legacy = curve;
        legacy.as_object_mut().unwrap().remove("periodic");
        legacy["control"] = Value::Bool(true);
        assert!(validate_versioned_geometry(&make(legacy)).is_ok());
        for convention in [serde_json::json!(null), serde_json::json!(false), serde_json::json!("unknown")] {
            assert!(validate_versioned_geometry(&serde_json::json!({"features":[{"type":"revolve", "convention":convention}]})).is_err());
        }
    }

    #[test]
    fn periodic_offset_certificate_requires_closed_periodic_output() {
        let record = serde_json::json!({"source":"ellipse", "output":"offset", "forward":true,
            "distance":2.0, "divisions":8, "pieces":4, "closed":true,
            "tolerance":0.002, "certificate":2});
        let mut value = serde_json::json!({"features":[{"type":"sketch", "sketch":{
            "freeform_offsets":[record], "curves":[{"id":"offset", "type":"spline", "periodic":true}]
        }}]});
        assert!(validate_freeform_offsets(&value).is_ok());
        let duplicate = value["features"][0]["sketch"]["curves"][0].clone();
        value["features"][0]["sketch"]["curves"].as_array_mut().unwrap().push(duplicate);
        assert!(validate_freeform_offsets(&value).is_err(), "periodic output ID must be unique");
        value["features"][0]["sketch"]["curves"].as_array_mut().unwrap().pop();
        value["features"][0]["sketch"]["curves"][0]["periodic"] = Value::Bool(false);
        assert!(validate_freeform_offsets(&value).is_err());
        value["features"][0]["sketch"]["curves"][0]["periodic"] = Value::Bool(true);
        value["features"][0]["sketch"]["freeform_offsets"][0]["closed"] = Value::Bool(false);
        assert!(validate_freeform_offsets(&value).is_err());
        value["features"][0]["sketch"]["freeform_offsets"][0]["certificate"] = Value::from(1);
        assert!(validate_freeform_offsets(&value).is_ok(), "legacy certificate semantics remain valid");
    }

    #[test]
    fn periodic_file_validation_refuses_damaged_closed_geometry() {
        use solvecraft_geom::Vec2;
        use solvecraft_sketch::{CurveKind, Sketch};
        let mut sketch = Sketch::new();
        let mut pts = Vec::new();
        for point in [Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0), Vec2::ZERO] {
            pts.push(sketch.add_point(point, None).unwrap());
        }
        let first = pts[0];
        let neighbor = pts[1];
        let closing = pts[3];
        sketch.add_curve(CurveKind::Spline { pts, control: false, degree: 3, periodic: true }, None).unwrap();
        let mut document = Document::new("periodic file validation");
        document.add_feature(crate::FeatureKind::Sketch { plane: crate::PlaneRef::Origin { name: "XY".into() }, sketch }, None).unwrap();
        assert_eq!(read(&document.to_json()).unwrap(), document);
        let mut value: Value = serde_json::from_str(&document.to_json()).unwrap();
        value["features"][0]["sketch"]["points"][closing]["pos"] = serde_json::json!([1, 0]);
        assert!(read(&value.to_string()).unwrap_err().to_string().contains("periodic"));
        value["features"][0]["sketch"]["points"][closing]["pos"] = value["features"][0]["sketch"]["points"][first]["pos"].clone();
        value["features"][0]["sketch"]["points"][neighbor]["pos"] = value["features"][0]["sketch"]["points"][first]["pos"].clone();
        assert!(read(&value.to_string()).unwrap_err().to_string().contains("periodic"));
    }

    #[test]
    fn freeform_offset_metadata_is_bounded_before_typed_decoding() {
        let valid = serde_json::json!({"source":"ellipse", "output":"offset", "forward":true,
            "distance":2.0, "divisions":8, "pieces":4, "closed":true,
            "tolerance":0.002, "certificate":1});
        let mut malformed = Vec::new();
        for (key, value) in [
            ("divisions", serde_json::json!(u64::MAX)),
            ("pieces", serde_json::json!(u64::MAX)),
            ("divisions", serde_json::json!(3)),
            ("distance", serde_json::json!(0)),
            ("tolerance", serde_json::json!(0.003)),
            ("certificate", serde_json::json!(2)),
            ("output", serde_json::json!("x".repeat(513))),
            ("source", serde_json::json!("offset")),
            ("unknown", serde_json::json!(true)),
        ] {
            let mut record = valid.clone();
            record[key] = value;
            malformed.push(serde_json::json!([record]));
        }
        malformed.push(serde_json::json!(vec![valid.clone(); 129]));
        malformed.push(serde_json::json!([valid.clone(), valid.clone()]));
        let mut cycle = valid.clone();
        cycle["source"] = serde_json::json!("offset");
        cycle["output"] = serde_json::json!("ellipse");
        malformed.push(serde_json::json!([valid, cycle]));
        for records in malformed {
            let mut value: Value = serde_json::from_str(&Document::new("hostile offset").to_json()).unwrap();
            value["features"] = serde_json::json!([{"type":"sketch", "sketch":{"freeform_offsets":records}}]);
            let error = read(&value.to_string()).unwrap_err().to_string();
            assert!(error.contains("free-form offset metadata"), "{error}");
            value["format"] = Value::from(format_string(FORMAT + 1));
            assert!(read(&value.to_string()).unwrap_err().to_string().contains("newer SolidCraft"));
        }
    }

    #[test]
    fn explicit_emboss_target_bounds_precede_typed_decoding() {
        for target in [
            serde_json::json!({"body":"", "at":[0,0,0]}),
            serde_json::json!({"body":"Body", "at":[0,0]}),
            serde_json::json!({"body":"Body", "at":[0,"invalid",0]}),
            serde_json::json!({"body":"Body", "at":[0,0,0], "unknown":true}),
            serde_json::json!({"body":"x".repeat(513), "at":[0,0,0]}),
            serde_json::json!({"body":"Body", "at":[0,0,0], "path":"bad"}),
            serde_json::json!({"body":"Body", "at":[0,0,0], "path":[0]}),
            serde_json::json!({"body":"Body", "at":[0,0,0], "path":[1,1]}),
            serde_json::json!({"body":"Body", "at":[0,0,0], "path":(1..=65).collect::<Vec<u64>>()}),
            serde_json::json!({"body":"Body", "at":[0,0,0], "author_path":[1,1]}),
            serde_json::json!({"body":"Body", "at":[0,0,0], "author_path":(1..=65).collect::<Vec<u64>>()}),
        ] {
            let mut value: Value = serde_json::from_str(&Document::new("hostile target").to_json()).unwrap();
            value["features"] = serde_json::json!([{"type":"emboss", "face":target}]);
            let error = read(&value.to_string()).unwrap_err().to_string();
            assert!(error.contains("emboss face"), "{error}");
            value["format"] = Value::from(format_string(FORMAT + 1));
            let error = read(&value.to_string()).unwrap_err().to_string();
            assert!(error.contains("newer SolidCraft"), "{error}");
        }
    }

    #[test]
    fn version_six_configuration_rows_upgrade_without_changing_values_or_features() {
        use crate::config::{Column, ConfigRow};
        let mut d = Document::new("legacy configurations");
        d.set_param("width", "10 mm", None, None).unwrap();
        let feature = d
            .add_feature(
                crate::FeatureKind::Box {
                    corner: ["0".into(), "0".into(), "0".into()],
                    length: "width".into(),
                    width: "20".into(),
                    height: "5".into(),
                    operation: crate::Operation::NewBody,
                },
                None,
            )
            .unwrap();
        d.configs.columns = vec![Column::Param("width".into()), Column::Suppress(feature)];
        d.configs.rows = vec![
            ConfigRow { name: "Default".into(), cells: vec!["10 mm".into(), "false".into()] },
            ConfigRow { name: "Alternate".into(), cells: vec!["width / 2 + 5 mm".into(), "yes".into()] },
        ];
        d.configs.active = Some("Default".into());
        let mut legacy: Value = serde_json::from_str(&d.to_json()).unwrap();
        legacy["format"] = Value::from("solvecraft/6");
        let original_table = legacy["configs"].clone();
        let upgraded = read(&legacy.to_string()).unwrap();
        assert_eq!(upgraded, d, "the identity migration preserves the complete design");
        assert_eq!(serde_json::to_value(&upgraded.configs).unwrap(), original_table);
        assert_eq!(upgraded.format, format_string(FORMAT));
        assert!(upgraded.configuration_matches());
        legacy.as_object_mut().unwrap().remove("configs");
        assert!(read(&legacy.to_string()).unwrap().configs.is_empty());
        legacy["configs"] = serde_json::json!({"columns":[],"rows":[]});
        assert!(read(&legacy.to_string()).unwrap().configs.is_empty());
    }

    #[test]
    fn version_seven_styles_upgrade_and_newer_versions_are_refused_before_columns() {
        use crate::config::{Column, ConfigRow};
        let mut d = Document::new("configured styles");
        d.configs.columns = vec![Column::Material("Body".into()), Column::Appearance("Body".into())];
        d.configs.rows = vec![ConfigRow {
            name: "Blue steel".into(),
            cells: vec!["Steel".into(), r#"{"name":"Custom blue","color":[40,90,186],"opacity":0.4}"#.into()],
        }];
        d.apply_configuration("Blue steel").unwrap();
        let mut value: Value = serde_json::from_str(&d.to_json()).unwrap();
        value["format"] = Value::from("solvecraft/7");
        let original_table = value["configs"].clone();
        let roundtrip = read(&value.to_string()).unwrap();
        assert_eq!(roundtrip, d);
        assert_eq!(serde_json::to_value(&roundtrip.configs).unwrap(), original_table);
        assert_eq!(roundtrip.format, format_string(FORMAT));
        assert!(roundtrip.configuration_matches());
        value["format"] = Value::from(format_string(FORMAT + 1));
        value["configs"]["columns"] = serde_json::json!([{"unknown_future_column":"Body"}]);
        value["configs"]["rules"] = serde_json::json!({"unknown_future_rule": true});
        let error = read(&value.to_string()).unwrap_err().to_string();
        assert!(error.contains("newer SolidCraft") && error.contains("update SolidCraft"), "{error}");
        assert!(!error.contains("unknown variant"), "format refusal precedes decoding new variants");
    }

    #[test]
    fn version_seven_origin_sketch_keeps_geometry_dimension_ids_and_expressions() {
        use solvecraft_geom::Vec2;
        use solvecraft_sketch::{ConstraintKind, Sketch};
        let mut d = Document::new("legacy origin dimensions");
        d.set_param("reach", "25 mm", Some("mm"), None).unwrap();
        let dim = d.new_model_param("reach", "mm");
        let mut sketch = Sketch::new();
        let end = sketch.add_point(Vec2::new(25.0, 0.0), Some("tip")).unwrap();
        let line = sketch.add_line_pts(0, end, Some("edge")).unwrap();
        sketch.add_constraint(ConstraintKind::Horizontal { l: line }, None).unwrap();
        sketch.add_constraint(ConstraintKind::Length { l: line, value: 25.0 }, Some(dim)).unwrap();
        d.add_feature(crate::FeatureKind::Sketch { plane: crate::PlaneRef::Origin { name: "XY".into() }, sketch }, Some("Dimensioned")).unwrap();
        let mut legacy: Value = serde_json::from_str(&d.to_json()).unwrap();
        legacy["format"] = Value::from("solvecraft/7");
        let features = legacy["features"].clone();
        let params = legacy["params"].clone();
        let upgraded = read(&legacy.to_string()).unwrap();
        assert_eq!(upgraded, d);
        let upgraded_json: Value = serde_json::from_str(&upgraded.to_json()).unwrap();
        assert_eq!(upgraded_json["features"], features);
        assert_eq!(upgraded_json["params"], params);
        assert_eq!(read(&upgraded.to_json()).unwrap(), upgraded);
    }

    #[test]
    fn version_eight_persistent_angle_sweep_and_slot_dimensions_roundtrip() {
        use solvecraft_geom::Vec2;
        use solvecraft_sketch::{ConstraintKind, Sketch};
        let mut d = Document::new("persistent typed dimensions");
        let heading = d.new_model_param("45 deg", "deg");
        let sweep = d.new_model_param("180 deg", "deg");
        let overall = d.new_model_param("10 mm", "mm");
        let mut sketch = Sketch::new();
        let end = sketch.add_point(Vec2::new(2.0, 2.0), None).unwrap();
        let line = sketch.add_line_pts(0, end, None).unwrap();
        let a = sketch.add_arc(Vec2::ZERO, Vec2::new(0.0, 2.0), Vec2::new(0.0, -2.0), [None; 3], None).unwrap();
        let b = sketch.add_arc(Vec2::new(6.0, 0.0), Vec2::new(6.0, -2.0), Vec2::new(6.0, 2.0), [None; 3], None).unwrap();
        sketch.add_constraint(ConstraintKind::LineAngle { l: line, value: std::f64::consts::FRAC_PI_4 }, Some(heading)).unwrap();
        sketch.add_constraint(ConstraintKind::ArcSweep { c: a, value: std::f64::consts::PI }, Some(sweep)).unwrap();
        sketch.add_constraint(ConstraintKind::SlotOverallLength { a, b, value: 10.0 }, Some(overall)).unwrap();
        d.add_feature(crate::FeatureKind::Sketch { plane: crate::PlaneRef::Origin { name: "XY".into() }, sketch }, None).unwrap();
        let mut saved: Value = serde_json::from_str(&d.to_json()).unwrap();
        assert_eq!(saved["format"], format_string(FORMAT));
        assert_eq!(read(&saved.to_string()).unwrap(), d);
        saved["format"] = Value::from(format_string(FORMAT + 1));
        saved["features"][0]["sketch"]["constraints"][0]["type"] = Value::from("future_dimension");
        let error = read(&saved.to_string()).unwrap_err().to_string();
        assert!(error.contains("newer SolidCraft") && !error.contains("unknown variant"), "{error}");
    }

    #[test]
    fn bulk_input_repair_preserves_existing_names_and_reuses_surplus_names() {
        let mut d = Document::new("input repair");
        d.set_param("d2", "1", None, None).unwrap();
        for i in 0..5 {
            d.add_feature(
                crate::FeatureKind::Box {
                    corner: ["0".into(), "0".into(), "0".into()],
                    length: "10".into(),
                    width: "10".into(),
                    height: "10".into(),
                    operation: crate::Operation::NewBody,
                },
                Some(&format!("Box{i}")),
            )
            .unwrap();
        }
        d.features[0].param_names = vec!["".into(), "custom".into(), "".into(), "d1".into(), "d2".into()];
        d.features[1].param_names.clear();
        d.features[2].param_names = vec!["d7".into(), "".into(), "d9".into()];
        d.features[3].param_names = vec!["d7".into(), "d11".into(), "d12".into(), "d1".into()];
        d.features[4].param_names.clear();
        let mut expected = d.clone();
        for id in expected.features.iter().map(|f| f.id).collect::<Vec<_>>() {
            expected.name_feature_inputs(id);
        }
        d.name_all_feature_inputs();
        assert_eq!(d, expected);
        assert_eq!(read(&d.to_json()).unwrap(), d);
    }

    #[test]
    fn large_unnamed_timeline_loads_with_unique_inputs() {
        let mut d = Document::new("large input repair");
        let id = d
            .add_feature(
                crate::FeatureKind::Box {
                    corner: ["0".into(), "0".into(), "0".into()],
                    length: "10".into(),
                    width: "10".into(),
                    height: "10".into(),
                    operation: crate::Operation::NewBody,
                },
                None,
            )
            .unwrap();
        let mut f = d.feature(id).unwrap().clone();
        f.param_names.clear();
        d.features = (1..=5000)
            .map(|id| {
                let mut f = f.clone();
                f.id = id;
                f
            })
            .collect();
        d.next_id = 5001;
        let back = read(&d.to_json()).unwrap();
        let names: Vec<_> = back.features.iter().flat_map(|f| &f.param_names).collect();
        assert_eq!(names.len(), 15000);
        assert_eq!(names.first().unwrap().as_str(), "d1");
        assert_eq!(names.last().unwrap().as_str(), "d15000");
        assert_eq!(names.iter().collect::<std::collections::BTreeSet<_>>().len(), names.len());
    }

    #[test]
    fn versions() {
        assert_eq!(version_of("solvecraft/1"), Some(1));
        assert_eq!(version_of("solvecraft/x"), None);
        let d = Document::new("A");
        assert_eq!(d.format, format_string(FORMAT));
        let back = read(&d.to_json()).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn old_xz_and_nested_offsets_keep_their_frame() {
        let mut doc = Document::new("old XZ");
        let id = doc
            .add_feature(
                crate::FeatureKind::ConstructionPlane {
                    plane: crate::PlaneRef::Offset { base: Box::new(crate::PlaneRef::Origin { name: "XZ".into() }), distance: "7 mm".into() },
                },
                None,
            )
            .unwrap();
        let mut json: Value = serde_json::from_str(&doc.to_json()).unwrap();
        json["format"] = Value::from("solvecraft/5");
        let migrated = read(&json.to_string()).unwrap();
        let crate::FeatureKind::ConstructionPlane { plane } = &migrated.feature(id).unwrap().kind else { panic!("plane") };
        let (values, errors) = migrated.param_values();
        assert!(errors.is_empty());
        let p = migrated.resolve_plane(&values, plane, 0).unwrap();
        assert_eq!(p.y, solvecraft_geom::Vec3::Z);
        assert_eq!(p.normal(), -solvecraft_geom::Vec3::Y);
        assert_eq!(p.origin, solvecraft_geom::Vec3::new(0.0, -7.0, 0.0));
        assert_eq!(read(&migrated.to_json()).unwrap(), migrated);
        let fresh = doc.resolve_plane(&values, &crate::PlaneRef::Origin { name: "XZ".into() }, 0).unwrap();
        assert_eq!(fresh.normal(), solvecraft_geom::Vec3::Y);
    }

    #[test]
    fn older_upgraded_newer_refused_others_rejected() {
        let mut v: Value = serde_json::from_str(&Document::new("A").to_json()).unwrap();
        v["format"] = Value::from("solvecraft/1");
        assert_eq!(read(&v.to_string()).unwrap().format, format_string(FORMAT));
        v.as_object_mut().unwrap().remove("format");
        assert!(read(&v.to_string()).is_ok(), "a file without a format is format 1");
        v["format"] = Value::from(format_string(FORMAT + 1));
        let e = read(&v.to_string()).unwrap_err().to_string();
        assert!(e.contains("newer SolidCraft"), "{e}");
        for bad in ["solvecraft/0", "acad/3", "solvecraft/-1", "solvecraft/99999999999999999999"] {
            v["format"] = Value::from(bad);
            assert!(read(&v.to_string()).is_err(), "{bad}");
        }
        assert!(read("[]").is_err());
        // Format 2 body colours become body appearances.
        let mut old: Value = serde_json::from_str(&Document::new("A").to_json()).unwrap();
        old["format"] = Value::from("solvecraft/2");
        old["appearances"] = serde_json::json!({"Body1": [200, 10, 20]});
        let d = read(&old.to_string()).unwrap();
        let l = d.appearances.bodies.get("Body1").unwrap();
        // Format 3 joints (aligned unless flipped) keep their placement.
        let mut j: Value = serde_json::from_str(&Document::new("A").to_json()).unwrap();
        j["format"] = Value::from("solvecraft/3");
        j["assembly"] = serde_json::json!({"joints": [{"id": 1, "name": "J", "kind": "rigid", "a": {"occurrence": 0, "snap": {"type": "point", "point": [0, 0, 0]}}, "b": {"occurrence": 0, "snap": {"type": "point", "point": [0, 0, 0]}}}]});
        let dj = read(&j.to_string()).unwrap();
        assert!(dj.assembly.joints[0].flip, "aligned before = not mated now");
        assert_eq!((l.color, l.opacity, l.name.as_str()), ([200, 10, 20], 1.0, "Custom"));
        let mut v: Value = serde_json::from_str(&Document::new("A").to_json()).unwrap();
        v["huge"] = Value::from(1e300);
        assert!(read(&v.to_string()).is_err(), "out-of-range numbers anywhere");
        assert!(read("{\"format\": 2}").is_err());
    }

    /// Every sketch in the timeline is solved when the design opens, and the solver sizes its
    /// dense matrices by the sketch. The caps interactive editing enforces have to hold for a
    /// file too, before the arrays are deserialised.
    #[test]
    fn oversized_sketches_are_refused_before_typed_decoding() {
        use solvecraft_sketch::{MAX_CONSTRAINTS, MAX_CURVES, MAX_POINTS, Sketch};

        // A sketch right at the point cap, chained by coincident constraints.
        let mut sketch = Sketch::new();
        let mut previous = 0;
        while sketch.points.len() < MAX_POINTS {
            let i = sketch.add_point(solvecraft_geom::Vec2::new(0.001 * sketch.points.len() as f64, 0.0), None).unwrap();
            sketch.add_constraint(solvecraft_sketch::ConstraintKind::Coincident { p: previous, q: i }, None).unwrap();
            previous = i;
        }
        let mut doc = Document::new("big sketch");
        doc.add_feature(crate::FeatureKind::Sketch { plane: crate::PlaneRef::Origin { name: "XY".into() }, sketch }, None).unwrap();
        let at_cap: Value = serde_json::from_str(&doc.to_json()).unwrap();
        assert!(read(&at_cap.to_string()).is_ok(), "a sketch at the caps must still load");

        // One past each cap, grown at the JSON level: no typed array is ever built.
        for (key, cap) in [("points", MAX_POINTS), ("constraints", MAX_CONSTRAINTS), ("curves", MAX_CURVES)] {
            let mut value = at_cap.clone();
            let slot = &mut value["features"][0]["sketch"][key];
            let first = slot.as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null);
            *slot = Value::Array(vec![first; cap + 1]);
            let error = read(&value.to_string()).unwrap_err().to_string();
            assert!(error.contains("sketch too large"), "{key}: {error}");
        }
    }
}
