use super::*;
use solidcraft_io::catalog::{Definition, Entry, FORMAT, ScalarLook, ScalarMaterial};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

struct File(String);
impl File {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(
            solidcraft_io::vfs::temp_dir()
                .join(format!("solidcraft-catalog-{}-{}.json", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
                .to_string_lossy()
                .into_owned(),
        )
    }
    fn write(&self, catalog: &Catalog) {
        std::fs::write(&self.0, catalog.encode().unwrap()).unwrap();
    }
}
impl Drop for File {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn catalog() -> Catalog {
    Catalog {
        format: FORMAT.into(),
        id: "user.test".into(),
        name: "Original scalar test library".into(),
        entries: vec![
            Entry::new(
                "user.test",
                Definition::Appearance(ScalarLook { name: "Warm oak colour".into(), color: [180, 120, 70], opacity: 0.8 }),
                Provenance::default(),
            )
            .unwrap(),
            Entry::new(
                "user.test",
                Definition::Material(ScalarMaterial {
                    name: "Timber density".into(),
                    density_g_cm3: 0.7,
                    appearance: Some(ScalarLook { name: "Timber tint".into(), color: [180, 120, 70], opacity: 1.0 }),
                }),
                Provenance::default(),
            )
            .unwrap(),
        ],
    }
}
fn session() -> Session {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length":10,"width":10,"height":10})).unwrap();
    s.mark_saved();
    s
}
fn apply_entry(s: &mut Session, file: &File, entry: &Entry, targets: Value) -> Result<Value> {
    s.execute("library.apply", &json!({"path":file.0,"entry":entry.id,"revision":entry.revision,"targets":targets}))
}

#[test]
fn catalog_export_obeys_file_scope_in_new_file_and_overwrite_modes() {
    let file = File::new();
    let base = std::path::Path::new(&file.0).with_extension("scope");
    std::fs::create_dir(&base).unwrap();
    let root = base.join("allowed");
    let outside = base.join("outside");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let sentinel = outside.join("existing.json");
    std::fs::write(&sentinel, b"previous contents").unwrap();
    let mut s = session();
    let before = s.doc.clone();
    let undo = s.undo.len();
    solidcraft_io::vfs::scoped(Some(solidcraft_io::vfs::PathScope::new([root.clone()])), || {
        for path in [outside.join("new.json"), root.join("../outside/traversal.json"), sentinel.clone()] {
            for overwrite in [false, true] {
                let error = s.execute("library.export", &json!({"path":path,"overwrite":overwrite})).unwrap_err();
                assert!(error.to_string().contains("outside the folders this agent may use"), "{error}");
            }
        }
        for overwrite in [false, true] {
            s.execute("library.export", &json!({"path":root.join("allowed.json"),"overwrite":overwrite})).unwrap();
        }
    });
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"previous contents");
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
    assert!(root.join("allowed.json").is_file());
    assert!(Arc::ptr_eq(&before, &s.doc));
    assert_eq!(s.undo.len(), undo);
    assert!(!s.is_dirty());
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn catalog_appearance_copy_keeps_mass_and_survives_undo_and_cold_reopen_without_file() {
    let file = File::new();
    let catalog = catalog();
    file.write(&catalog);
    let mut s = session();
    let original_format = s.doc.format.clone();
    let before = s.execute("component.properties", &json!({})).unwrap()["total"]["mass_g"].as_f64().unwrap();
    let undo = s.undo.len();
    apply_entry(&mut s, &file, &catalog.entries[0], json!({"bodies":["Body1"]})).unwrap();
    assert_eq!(s.undo.len(), undo + 1);
    assert_eq!(s.doc.appearances.bodies["Body1"].name, "Warm oak colour");
    assert_eq!(s.execute("component.properties", &json!({})).unwrap()["total"]["mass_g"].as_f64().unwrap(), before);
    let saved = solidcraft_io::write_design(&s.doc);
    std::fs::remove_file(&file.0).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc.appearances.bodies.is_empty());
    s.execute("edit.redo", &json!({})).unwrap();
    let reopened = Session::new(solidcraft_io::read_design(&saved).unwrap());
    assert_eq!(reopened.doc.appearances, s.doc.appearances);
    assert_eq!(reopened.doc.format, original_format, "scalar library adds no document schema");
}

#[test]
fn catalog_physical_copy_preserves_exact_look_label_and_is_one_transaction() {
    let file = File::new();
    let catalog = catalog();
    file.write(&catalog);
    let mut s = session();
    let undo = s.undo.len();
    apply_entry(&mut s, &file, &catalog.entries[1], json!({"bodies":["Body1"]})).unwrap();
    assert_eq!(s.undo.len(), undo + 1);
    assert_eq!(s.doc.user_materials[0].appearance.as_ref().unwrap().name, "Timber tint");
    let mass = s.execute("component.properties", &json!({})).unwrap()["total"]["mass_g"].as_f64().unwrap();
    assert!((mass - 0.7).abs() < 1e-8);
    let saved = solidcraft_io::write_design(&s.doc);
    std::fs::remove_file(&file.0).unwrap();
    let mut reopened = Session::new(solidcraft_io::read_design(&saved).unwrap());
    let actual = reopened.execute("component.properties", &json!({})).unwrap()["total"]["mass_g"].as_f64().unwrap();
    assert!((actual - 0.7).abs() < 1e-8);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc.user_materials.is_empty());
    assert!(s.doc.materials.is_empty());
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc.user_materials, reopened.doc.user_materials);
}

#[test]
fn catalog_conflicts_stale_revisions_and_invalid_targets_are_atomic() {
    let file = File::new();
    let catalog = catalog();
    file.write(&catalog);
    let mut s = session();
    s.execute("material.create", &json!({"name":"Timber density","density_g_cm3":1.2})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let redo = s.redo.len();
    let doc = s.doc.clone();
    let undo = s.undo.len();
    let revision = s.revision;
    assert!(apply_entry(&mut s, &file, &catalog.entries[1], json!({"bodies":["missing"]})).is_err());
    assert_eq!(*s.doc, *doc);
    assert_eq!((s.undo.len(), s.redo.len(), s.revision), (undo, redo, revision));
    s.execute("edit.redo", &json!({})).unwrap();
    let doc = s.doc.clone();
    let history = (s.undo.len(), s.redo.len(), s.revision);
    assert!(apply_entry(&mut s, &file, &catalog.entries[1], json!({"bodies":["Body1"]})).is_err());
    assert_eq!(*s.doc, *doc);
    assert_eq!((s.undo.len(), s.redo.len(), s.revision), history);
    let mut changed = catalog.clone();
    if let Definition::Appearance(look) = &mut changed.entries[0].definition {
        look.color = [1, 2, 3];
    }
    changed.entries[0].revision = changed.entries[0].content_revision().unwrap();
    file.write(&changed);
    assert!(apply_entry(&mut s, &file, &catalog.entries[0], json!({"bodies":["Body1"]})).is_err());
    assert_eq!(*s.doc, *doc);
    assert_eq!((s.undo.len(), s.redo.len(), s.revision), history);
}

#[test]
fn catalog_explicit_material_rename_and_identical_definition_reuse() {
    let file = File::new();
    let catalog = catalog();
    file.write(&catalog);
    let entry = &catalog.entries[1];
    let mut s = session();
    s.execute("material.create", &json!({"name":"Timber density","density_g_cm3":1.2})).unwrap();
    let params = json!({"path":file.0,"entry":entry.id,"revision":entry.revision,"name":"Imported timber"});
    s.execute("library.apply", &params).unwrap();
    assert_eq!(s.doc.user_materials.len(), 2);
    let undo = s.undo.len();
    let revision = s.revision;
    let again = s.execute("library.apply", &params).unwrap();
    assert_eq!(again["result"]["reused"], true);
    assert_eq!((s.undo.len(), s.revision), (undo, revision));
    assert_eq!(s.doc.user_materials[0].density, 1.2);
}

#[test]
fn catalog_inspection_export_and_no_clobber_do_not_touch_design_or_history() {
    let file = File::new();
    let mut s = session();
    s.execute("appearance.assign", &json!({"bodies":["Body1"],"appearance":"Warm oak colour","color":[180,120,70],"opacity":0.8})).unwrap();
    s.mark_saved();
    let doc = s.doc.clone();
    let undo = s.undo.len();
    let revision = s.revision;
    let result = s.execute("library.export", &json!({"path":file.0})).unwrap();
    assert_eq!(result["entries"], 1);
    assert!(Arc::ptr_eq(&doc, &s.doc));
    assert_eq!((s.undo.len(), s.revision), (undo, revision));
    assert!(!s.is_dirty());
    let before = std::fs::read(&file.0).unwrap();
    s.execute("library.inspect", &json!({"path":file.0})).unwrap();
    assert!(s.execute("library.export", &json!({"path":file.0})).is_err());
    assert_eq!(before, std::fs::read(&file.0).unwrap());
    assert!(Arc::ptr_eq(&doc, &s.doc));
    assert_eq!((s.undo.len(), s.revision), (undo, revision));
    s.execute("library.export", &json!({"path":file.0,"overwrite":true})).unwrap();
    assert_eq!(before, std::fs::read(&file.0).unwrap());
    for command in ["library.inspect", "library.export", "library.apply"] {
        assert!(s.execute(command, &json!({"path":file.0,"unknown":true})).is_err());
    }
}

#[test]
fn lossy_scalar_export_never_creates_or_replaces_a_file_or_changes_history() {
    let mut s = session();
    s.execute(
        "appearance.assign",
        &json!({"bodies":["Body1"],"color":[180,120,70],
        "pbr":{"metallic":1.0,"roughness":0.3,"dielectric_ior":1.5}}),
    )
    .unwrap();
    s.mark_saved();
    let doc = s.doc.clone();
    let history = (s.undo.len(), s.redo.len(), s.revision);
    for existing in [false, true] {
        let file = File::new();
        if existing {
            std::fs::write(&file.0, b"preserve existing library").unwrap();
        }
        for overwrite in [false, true] {
            let error = s.execute("library.export", &json!({"path":file.0,"overwrite":overwrite})).unwrap_err();
            assert!(error.to_string().contains("cannot preserve PBR"));
            if existing {
                assert_eq!(std::fs::read(&file.0).unwrap(), b"preserve existing library");
            } else {
                assert!(!std::path::Path::new(&file.0).exists());
            }
            assert!(Arc::ptr_eq(&doc, &s.doc));
            assert_eq!((s.undo.len(), s.redo.len(), s.revision), history);
            assert!(!s.is_dirty());
        }
    }
}
