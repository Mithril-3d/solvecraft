//! Saved render intent, independent of runtime backend availability and GPU resources.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use solidcraft_geom::Vec3;
use solidcraft_render::{BackendId, Camera, RenderFormat, RenderLimits, RenderMode, UpAxis};

use crate::{DocError, Result};

pub const MAX_RENDER_SETUPS: usize = 32;
pub const MAX_SETUP_NAME_BYTES: usize = 128;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RenderSetup {
    pub backend: BackendId,
    pub mode: RenderMode,
    #[serde(deserialize_with = "SavedCamera::deserialize")]
    pub camera: Camera,
    pub width: u32,
    pub height: u32,
    pub format: RenderFormat,
    pub exposure_ev: f64,
    /// Procedural environment orientation, in radians. No HDRI is loaded in R2.
    pub environment_rotation: f64,
    pub environment_intensity: f64,
    /// sRGB background. Alpha/output colour management belong to rendering phases.
    pub background: [u8; 3],
}

// Reuse Camera's representation and defaults, but reject misspelled saved fields. A remote
// derive constructs Camera itself, so adding a Camera field requires updating this adapter.
// Navigation/named-view deserialization keeps its existing compatibility policy.
#[derive(Deserialize)]
#[serde(remote = "Camera", deny_unknown_fields)]
struct SavedCamera {
    target: Vec3,
    yaw: f64,
    pitch: f64,
    #[serde(default)]
    roll: f64,
    distance: f64,
    fov: f64,
    #[serde(default)]
    up: UpAxis,
}

impl Default for RenderSetup {
    fn default() -> Self {
        // The built-in descriptor owns the canonical default backend identifier.
        Self {
            backend: solidcraft_render::builtin_backend().id,
            mode: RenderMode::Preview,
            camera: Camera::default(),
            width: 1024,
            height: 768,
            format: RenderFormat::Png,
            exposure_ev: 0.0,
            environment_rotation: 0.0,
            environment_intensity: 1.0,
            background: [236, 240, 245],
        }
    }
}

impl RenderSetup {
    pub fn validate(&self) -> Result<()> {
        let limits = RenderLimits::default();
        let (width, height) = (u64::from(self.width), u64::from(self.height));
        if width == 0 || height == 0 {
            return Err(DocError::Invalid("render output must be at least 1 × 1 pixels".into()));
        }
        if let Some((w, h)) = limits.fit(width, height, None)
            && (w, h) != (width, height)
        {
            return Err(DocError::Invalid(format!(
                "render output {width} × {height} exceeds the image limits ({} pixels per side, {} pixels in all); \
                 the largest image at this aspect is {w} × {h}",
                limits.max_dimension, limits.max_pixels
            )));
        }
        let camera = &self.camera;
        if !camera.is_valid()
            || [camera.target.x, camera.target.y, camera.target.z, camera.distance].into_iter().any(|x| x.abs() > 1e9)
            || [camera.yaw, camera.pitch, camera.roll].into_iter().any(|x| x.abs() > 1e6)
            || !camera.fov.is_finite()
            || !(0.0..std::f64::consts::PI).contains(&camera.fov)
        {
            return Err(DocError::Invalid("invalid render camera (finite mm/radians, positive distance, 0 <= fov < pi required)".into()));
        }
        if !self.exposure_ev.is_finite() || !(-32.0..=32.0).contains(&self.exposure_ev) {
            return Err(DocError::Invalid("render exposure must be finite and between -32 and 32 EV".into()));
        }
        if !self.environment_rotation.is_finite() || self.environment_rotation.abs() > 1e6 {
            return Err(DocError::Invalid("render environment rotation must be finite, bounded radians".into()));
        }
        if !self.environment_intensity.is_finite() || !(0.0..=100.0).contains(&self.environment_intensity) {
            return Err(DocError::Invalid("render environment intensity must be finite and between 0 and 100".into()));
        }
        // Desired mode/format/backend may be unavailable on this host. Saving intent is not preflight.
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedSettings")]
pub struct RenderSettings {
    pub setups: BTreeMap<String, RenderSetup>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UncheckedSettings {
    setups: BTreeMap<String, RenderSetup>,
}

impl TryFrom<UncheckedSettings> for RenderSettings {
    type Error = DocError;
    fn try_from(value: UncheckedSettings) -> Result<Self> {
        let settings = Self { setups: value.setups };
        settings.validate()?;
        Ok(settings)
    }
}

impl RenderSettings {
    pub fn is_empty(&self) -> bool {
        self.setups.is_empty()
    }

    pub fn validate_name(name: &str) -> Result<()> {
        if name.trim().is_empty() || name != name.trim() || name.len() > MAX_SETUP_NAME_BYTES || name.chars().any(char::is_control) {
            return Err(DocError::Invalid("render setup name must be trimmed, nonempty, at most 128 bytes and contain no control characters".into()));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.setups.len() > MAX_RENDER_SETUPS {
            return Err(DocError::Invalid("too many render setups (maximum 32)".into()));
        }
        for (name, setup) in &self.setups {
            Self::validate_name(name)?;
            setup.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;
    use std::sync::Arc;

    #[test]
    fn camera_fields_are_strict_without_changing_existing_camera_defaults() {
        let mut camera = serde_json::to_value(Camera::default()).unwrap();
        camera["fov_degrees"] = serde_json::json!(45);
        let saved = serde_json::json!({"setups":{"Studio":{"camera":camera}}});
        let mut doc = serde_json::to_value(Document::default()).unwrap();
        doc["render"] = saved.clone();
        assert!(Document::from_json(&doc.to_string()).is_err());
        assert!(serde_json::from_value::<RenderSettings>(saved).is_err());
        camera.as_object_mut().unwrap().remove("fov_degrees");
        camera.as_object_mut().unwrap().remove("roll");
        camera.as_object_mut().unwrap().remove("up");
        let setup: RenderSetup = serde_json::from_value(serde_json::json!({"camera":camera})).unwrap();
        assert_eq!(setup.camera, Camera::default());
    }

    #[test]
    fn legacy_and_named_setups_round_trip_without_changing_other_data() {
        let old = Document::new("Legacy");
        let mut json = serde_json::to_value(&old).unwrap();
        json["format"] = serde_json::json!("solidcraft/16");
        json.as_object_mut().unwrap().remove("render");
        let mut loaded = Document::from_json(&json.to_string()).unwrap();
        assert!(loaded.render.is_empty());
        assert_eq!(loaded.format, crate::format::format_string(crate::format::FORMAT));
        // Upstream owns 19 for editable offsets; saved render intent needs 20.
        for version in [17, 18, 19] {
            json["format"] = serde_json::json!(crate::format::format_string(version));
            assert_eq!(Document::from_json(&json.to_string()).unwrap(), loaded);
        }
        let before = loaded.clone();
        let setup = RenderSetup {
            backend: serde_json::from_value(serde_json::json!("future-backend")).unwrap(),
            exposure_ev: 2.5,
            mode: RenderMode::Final,
            format: RenderFormat::Exr,
            camera: Camera { up: solidcraft_render::UpAxis::Y, ..Default::default() },
            ..Default::default()
        };
        Arc::make_mut(&mut loaded.render).setups.insert("Studio".into(), setup);
        assert!(loaded.differs_only_in_render(&before));
        assert_eq!(Document::from_json(&loaded.to_json()).unwrap(), loaded);
        let mut previous_draft = serde_json::to_value(&loaded).unwrap();
        previous_draft["format"] = serde_json::json!("solidcraft/17");
        assert_eq!(Document::from_json(&previous_draft.to_string()).unwrap(), loaded);
        loaded.units = "in".into();
        assert!(!loaded.differs_only_in_render(&before));
    }

    #[test]
    fn mixed_document_changes_never_qualify_as_render_only() {
        let before = Document::default();
        let mut render_only = before.clone();
        Arc::make_mut(&mut render_only.render).setups.insert("Studio".into(), RenderSetup::default());
        assert!(render_only.differs_only_in_render(&before));
        for (key, value) in [
            ("marker", serde_json::json!(0)),
            ("materials", serde_json::json!({"Body1":"Steel"})),
            ("visibility", serde_json::json!({"hidden_bodies":["Body1"]})),
            ("body_components", serde_json::json!({"Body1":1})),
            ("next_id", serde_json::json!(99)),
        ] {
            let mut json = serde_json::to_value(&render_only).unwrap();
            json[key] = value;
            let mixed: Document = serde_json::from_value(json).unwrap();
            assert!(!mixed.differs_only_in_render(&before), "{key}");
        }
    }

    #[test]
    fn oversized_output_names_the_limits_and_the_largest_size() {
        let setup = RenderSetup { width: 4096, height: 4096, ..Default::default() };
        let message = setup.validate().unwrap_err().to_string();
        assert!(message.contains("4096 × 4096 exceeds the image limits (4096 pixels per side, 4194304 pixels in all)"), "{message}");
        assert!(message.contains("the largest image at this aspect is 2048 × 2048"), "{message}");
        let setup = RenderSetup { width: 7680, height: 4320, ..Default::default() };
        let message = setup.validate().unwrap_err().to_string();
        assert!(message.contains("the largest image at this aspect is 27"), "{message}");
        assert!(RenderSetup { width: 2048, height: 2048, ..Default::default() }.validate().is_ok());
    }

    #[test]
    fn invalid_saved_intent_is_rejected_at_deserialization() {
        for patch in [
            serde_json::json!({"width":0}),
            serde_json::json!({"width":4097}),
            serde_json::json!({"width":4096,"height":4096}),
            serde_json::json!({"exposure_ev":33}),
            serde_json::json!({"environment_rotation":1000001}),
            serde_json::json!({"environment_intensity":-1}),
            serde_json::json!({"background":[256,0,0]}),
            serde_json::json!({"unknown":1}),
        ] {
            let json = serde_json::json!({"setups":{"Studio":patch}});
            assert!(serde_json::from_value::<RenderSettings>(json).is_err());
        }
        for name in ["", " padded", "line\nfeed"] {
            assert!(RenderSettings::validate_name(name).is_err());
        }
        let mut settings = RenderSettings::default();
        for i in 0..=MAX_RENDER_SETUPS {
            settings.setups.insert(i.to_string(), RenderSetup::default());
        }
        assert!(serde_json::from_value::<RenderSettings>(serde_json::to_value(settings).unwrap()).is_err());
        let mut setup = RenderSetup::default();
        setup.camera.fov = f64::NAN;
        assert!(setup.validate().is_err());
        for field in ["yaw", "pitch", "roll", "distance", "fov"] {
            let mut value = serde_json::to_value(RenderSetup::default()).unwrap();
            value["camera"][field] = serde_json::json!(1e12);
            let setup: RenderSetup = serde_json::from_value(value).unwrap();
            assert!(setup.validate().is_err(), "{field}");
        }
        setup.camera = Camera::default();
        setup.environment_intensity = f64::INFINITY;
        assert!(setup.validate().is_err());
    }
}
