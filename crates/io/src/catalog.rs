//! Portable scalar libraries; no renderer, network, external resources or design links.
use crate::{IoError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solidcraft_doc::{Document, UserMaterial, appearance::Look, valid_density};
use std::collections::BTreeMap;

pub const FORMAT: &str = "solidcraft-library/1";
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 1024;
/// Bound source inspection even when many assignments reuse the same definition.
pub const MAX_SOURCE_REFERENCES: usize = 100_000;
fn invalid(message: impl Into<String>) -> IoError {
    IoError::Invalid(message.into())
}
fn text(value: &str, max: usize) -> bool {
    !value.is_empty() && value == value.trim() && value.len() <= max && !value.chars().any(char::is_control)
}
fn identifier(value: &str) -> bool {
    value.len() <= 64
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Information only: never fetched or treated as a local resource path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}
impl Default for Provenance {
    fn default() -> Self {
        Self { license: "LicenseRef-User-Provided".into(), author: None, source_url: None }
    }
}
impl Provenance {
    fn validate(&self) -> Result<()> {
        if !text(&self.license, 128)
            || self.author.as_ref().is_some_and(|s| !text(s, 256))
            || self.source_url.as_ref().is_some_and(|s| !text(s, 2048))
        {
            return Err(invalid("invalid library provenance (license 128, author 256, source URL 2048 bytes)"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarLook {
    pub name: String,
    pub color: [u8; 3],
    pub opacity: f64,
}
impl ScalarLook {
    pub fn from_look(look: &Look) -> Result<Self> {
        if look.pbr.is_some() {
            return Err(invalid("scalar catalog v1 cannot preserve PBR parameters; export refused"));
        }
        Ok(Self { name: look.name.clone(), color: look.color, opacity: look.opacity })
    }
    pub fn to_look(&self) -> Look {
        Look { name: self.name.clone(), color: self.color, opacity: self.opacity, pbr: None }
    }
    fn validate(&self) -> Result<()> {
        if !text(&self.name, 128) || !self.opacity.is_finite() || !(0.0..=1.0).contains(&self.opacity) {
            return Err(invalid("library appearance needs a 1–128 byte name and finite opacity 0–1"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarMaterial {
    pub name: String,
    pub density_g_cm3: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<ScalarLook>,
}
impl ScalarMaterial {
    pub fn from_material(material: &UserMaterial) -> Result<Self> {
        Ok(Self {
            name: material.name.clone(),
            density_g_cm3: material.density,
            appearance: material.appearance.as_ref().map(ScalarLook::from_look).transpose()?,
        })
    }
    pub fn to_material(&self, name: String) -> UserMaterial {
        UserMaterial { name, density: self.density_g_cm3, appearance: self.appearance.as_ref().map(ScalarLook::to_look) }
    }
    fn validate(&self) -> Result<()> {
        if !text(&self.name, 128) || !valid_density(self.density_g_cm3) {
            return Err(invalid("invalid library material name or density"));
        }
        if let Some(look) = &self.appearance {
            look.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case", deny_unknown_fields)]
pub enum Definition {
    Appearance(ScalarLook),
    Material(ScalarMaterial),
}
impl Definition {
    pub fn name(&self) -> &str {
        match self {
            Self::Appearance(l) => &l.name,
            Self::Material(m) => &m.name,
        }
    }
    fn validate(&self) -> Result<()> {
        match self {
            Self::Appearance(l) => l.validate(),
            Self::Material(m) => m.validate(),
        }
    }
    fn kind(&self) -> &str {
        match self {
            Self::Appearance(_) => "appearance",
            Self::Material(_) => "material",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub revision: String,
    pub provenance: Provenance,
    pub definition: Definition,
}
impl Entry {
    /// Revision is SHA-256 of compact serde JSON `[provenance, definition]` in this v1 contract.
    pub fn content_revision(&self) -> Result<String> {
        self.provenance.validate()?;
        self.definition.validate()?;
        let bytes = serde_json::to_vec(&(&self.provenance, &self.definition)).map_err(|e| invalid(e.to_string()))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
    pub fn new(pack_id: &str, definition: Definition, provenance: Provenance) -> Result<Self> {
        if !identifier(pack_id) {
            return Err(invalid("invalid library identifier"));
        }
        let mut out = Self { id: String::new(), revision: String::new(), provenance, definition };
        out.revision = out.content_revision()?;
        let prefix = out.revision.get(..16).ok_or_else(|| invalid("invalid library revision"))?;
        out.id = format!("{pack_id}/{}-{prefix}", out.definition.kind());
        Ok(out)
    }
    fn validate(&self, pack_id: &str) -> Result<()> {
        let (pack, local) = self.id.split_once('/').ok_or_else(|| invalid("entry ID must be qualified by its library ID"))?;
        if pack != pack_id || !identifier(local) || self.revision != self.content_revision()? {
            return Err(invalid("invalid entry identity or content revision"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub format: String,
    pub id: String,
    pub name: String,
    #[serde(deserialize_with = "bounded_entries")]
    pub entries: Vec<Entry>,
}

fn bounded_entries<'de, D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Vec<Entry>, D::Error> {
    struct Entries;
    impl<'de> serde::de::Visitor<'de> for Entries {
        type Value = Vec<Entry>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("at most 1024 scalar library entries")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error> {
            let mut entries = Vec::new();
            loop {
                if entries.len() == MAX_ENTRIES {
                    if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                        return Err(serde::de::Error::custom("scalar library exceeds 1024 entries"));
                    }
                    break;
                }
                let Some(entry) = sequence.next_element::<Entry>()? else { break };
                let revision = entry.content_revision().map_err(serde::de::Error::custom)?;
                if entry.revision != revision || entry.id.len() > 129 {
                    return Err(serde::de::Error::custom("invalid scalar library revision or identity"));
                }
                entries.try_reserve(1).map_err(|_| serde::de::Error::custom("could not reserve scalar library entries"))?;
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_seq(Entries)
}
impl Catalog {
    pub fn validate(&self) -> Result<()> {
        if self.format != FORMAT || !identifier(&self.id) || !text(&self.name, 128) || self.entries.len() > MAX_ENTRIES {
            return Err(invalid("unsupported or invalid scalar library (v1, 1024 entries, 128 byte name)"));
        }
        let mut ids = std::collections::BTreeSet::new();
        for entry in &self.entries {
            entry.validate(&self.id)?;
            if !ids.insert(&entry.id) {
                return Err(invalid("duplicate library entry identity"));
            }
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err(invalid("scalar library exceeds 4 MiB"));
        }
        let catalog: Self = serde_json::from_slice(bytes).map_err(|e| invalid(format!("invalid scalar library: {e}")))?;
        catalog.validate()?;
        Ok(catalog)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| invalid(e.to_string()))?;
        if bytes.len() > MAX_BYTES {
            return Err(invalid("scalar library exceeds 4 MiB"));
        }
        Ok(bytes)
    }
    pub fn read(path: &str) -> Result<Self> {
        // Check the VFS size first as well: its browser read clones the stored bytes.
        if crate::vfs::len(path).map_err(|e| invalid(e.to_string()))? > MAX_BYTES as u64 {
            return Err(invalid("scalar library exceeds 4 MiB"));
        }
        Self::decode(&crate::vfs::read(path, MAX_BYTES).map_err(|e| invalid(e.to_string()))?)
    }
    pub fn from_document(doc: &Document, id: &str, name: &str, provenance: Provenance) -> Result<Self> {
        if !identifier(id) || !text(name, 128) {
            return Err(invalid("invalid scalar library identifier or name"));
        }
        provenance.validate()?;
        let references = doc
            .appearances
            .bodies
            .len()
            .saturating_add(doc.appearances.components.len())
            .saturating_add(doc.appearances.faces.len())
            .saturating_add(doc.user_materials.len().saturating_mul(2));
        if references > MAX_SOURCE_REFERENCES {
            return Err(invalid("library export exceeds 100000 source references"));
        }
        let mut entries = BTreeMap::new();
        let mut insert = |definition| -> Result<()> {
            let entry = Entry::new(id, definition, provenance.clone())?;
            if let Some(previous) = entries.get(&entry.id) {
                if previous != &entry {
                    return Err(invalid("library content identifier collision"));
                }
            } else {
                if entries.len() >= MAX_ENTRIES {
                    return Err(invalid("scalar library exceeds 1024 unique entries"));
                }
                entries.insert(entry.id.clone(), entry);
            }
            Ok(())
        };
        for look in doc.appearances.bodies.values().chain(doc.appearances.components.values()).chain(doc.appearances.faces.iter().map(|f| &f.look)) {
            insert(Definition::Appearance(ScalarLook::from_look(look)?))?;
        }
        for material in &doc.user_materials {
            insert(Definition::Material(ScalarMaterial::from_material(material)?))?;
            if let Some(look) = &material.appearance {
                insert(Definition::Appearance(ScalarLook::from_look(look)?))?;
            }
        }
        let out = Self { format: FORMAT.into(), id: id.into(), name: name.into(), entries: entries.into_values().collect() };
        out.validate()?;
        Ok(out)
    }
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
