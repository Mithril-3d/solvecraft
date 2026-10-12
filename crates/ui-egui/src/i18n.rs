//! Embedded offline message catalogs. UI messages use stable IDs; engine display labels
//! pass through an adapter without changing engine or scenario identifiers.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::OnceLock;

type Messages = BTreeMap<String, String>;

/// Register locales here after adding their catalogs. Missing translations use English.
pub struct Catalog {
    pub id: &'static str,
    pub name: &'static str,
    source: &'static str,
    messages: OnceLock<Messages>,
}

impl Catalog {
    const fn new(id: &'static str, name: &'static str, source: &'static str) -> Self {
        Self { id, name, source, messages: OnceLock::new() }
    }

    fn messages(&'static self) -> &'static Messages {
        self.messages.get_or_init(|| match serde_json::from_str(self.source) {
            Ok(messages) => messages,
            Err(error) => {
                log::error!("invalid embedded {} message catalog: {error}", self.id);
                Messages::new()
            }
        })
    }
}

static ENGLISH: Catalog = Catalog::new("en", "English", include_str!("locales/en.json"));
static JAPANESE: Catalog = Catalog::new("ja", "日本語", include_str!("locales/ja.json"));
static GERMAN: Catalog = Catalog::new("de", "Deutsch", include_str!("locales/de.json"));
pub const LOCALES: &[&Catalog] = &[&ENGLISH, &JAPANESE, &GERMAN];

pub fn is_supported(id: &str) -> bool {
    LOCALES.iter().any(|catalog| catalog.id == id)
}

/// Immutable locale reference. Catalogs parse once; lookups are logarithmic.
#[derive(Clone, Copy)]
pub struct Locale(&'static Catalog);

impl Locale {
    pub fn for_language(id: &str) -> Self {
        Self(LOCALES.iter().copied().find(|catalog| catalog.id == id).unwrap_or(&ENGLISH))
    }

    pub fn name(self) -> &'static str {
        self.0.name
    }

    /// Stable message ID → translation → English → ID.
    pub fn text(self, id: &str) -> &str {
        self.0.messages().get(id).or_else(|| ENGLISH.messages().get(id)).map_or(id, String::as_str)
    }

    /// Translate source labels from engine/UI tables, preserving unknown labels.
    /// Also accepts stable IDs while table helpers migrate away from source text.
    pub fn label(self, label: &str) -> &str {
        static LABEL_IDS: OnceLock<BTreeMap<&'static str, &'static str>> = OnceLock::new();
        let ids = LABEL_IDS.get_or_init(|| {
            ENGLISH.messages().iter().filter(|(_, text)| !text.contains('{')).map(|(id, text)| (text.as_str(), id.as_str())).collect()
        });
        ids.get(label).map_or_else(|| self.text(label), |id| self.text(id))
    }

    pub fn format(self, id: &str, args: &[(&str, &str)]) -> String {
        interpolate(self.text(id), args)
    }

    /// Translate generated browser names and labels, preserving custom names otherwise.
    pub fn dynamic(self, text: &str) -> String {
        for (prefix, id) in [("Sketch", "browser.sketch_name"), ("Body", "browser.body_name"), ("Component", "browser.component_name")] {
            if let Some(number) = text.strip_prefix(prefix)
                && !number.is_empty()
                && number.chars().all(|c| c.is_ascii_digit())
            {
                return self.format(id, &[("number", number)]);
            }
        }
        for (prefix, id, arg) in [("Units: ", "browser.units", "units"), ("Precision: ", "browser.precision", "precision")] {
            if let Some(value) = text.strip_prefix(prefix) {
                return self.format(id, &[(arg, value)]);
            }
        }
        if let Some(count) = text.strip_prefix("Parameters (").and_then(|s| s.strip_suffix(')')) {
            return self.format("browser.parameters", &[("count", count)]);
        }
        if let Some(count) = text.strip_suffix(" degrees of freedom")
            && !count.is_empty()
            && count.chars().all(|c| c.is_ascii_digit())
        {
            return self.format("sketch.degrees_of_freedom", &[("count", count)]);
        }
        self.label(text).to_string()
    }
}

/// One-pass substitution: argument values are literal, even when they contain braces.
/// Missing arguments remain visible rather than crashing or silently removing text.
fn interpolate(template: &str, args: &[(&str, &str)]) -> String {
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some((before, after)) = remaining.split_once('{') {
        output.push_str(before);
        let Some((name, rest)) = after.split_once('}') else {
            output.push('{');
            output.push_str(after);
            return output;
        };
        if let Some((_, value)) = args.iter().find(|(key, _)| *key == name) {
            output.push_str(value);
        } else {
            output.push('{');
            output.push_str(name);
            output.push('}');
        }
        remaining = rest;
    }
    output.push_str(remaining);
    output
}

thread_local! {
    // Helpers without an app reference share a render scope. Nested renders restore
    // their caller's locale; no owned language strings or persistent global setting.
    static CURRENT: Cell<Locale> = const { Cell::new(Locale(&ENGLISH)) };
}

#[must_use]
pub struct LanguageScope {
    previous: Locale,
    // Drop on the thread where the scope was entered.
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

pub fn scope(id: &str) -> LanguageScope {
    let previous = CURRENT.with(|current| current.replace(Locale::for_language(id)));
    LanguageScope { previous, _thread: std::marker::PhantomData }
}

impl Drop for LanguageScope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.set(self.previous));
    }
}

pub fn tr(id: &str) -> &str {
    CURRENT.with(|current| current.get().text(id))
}
pub fn tr_label(label: &str) -> &str {
    CURRENT.with(|current| current.get().label(label))
}
pub fn format(id: &str, args: &[(&str, &str)]) -> String {
    CURRENT.with(|current| current.get().format(id, args))
}
pub fn tr_dyn(text: &str) -> String {
    CURRENT.with(|current| current.get().dynamic(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn placeholders(text: &str) -> BTreeSet<&str> {
        let mut result = BTreeSet::new();
        let mut rest = text;
        while let Some((_, after)) = rest.split_once('{') {
            let (name, remaining) = after.split_once('}').expect("unclosed placeholder");
            assert!(!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
            result.insert(name);
            rest = remaining;
        }
        result
    }

    #[test]
    fn catalogs_have_valid_ids_and_matching_placeholders() {
        let english: Messages = serde_json::from_str(ENGLISH.source).unwrap();
        let mut ids = BTreeSet::new();
        for catalog in LOCALES {
            assert!(ids.insert(catalog.id), "duplicate locale");
            let messages: Messages = serde_json::from_str(catalog.source).unwrap();
            assert!(!messages.is_empty());
            for (id, text) in messages {
                let original = english.get(&id).expect("unknown message ID");
                assert_eq!(placeholders(&text), placeholders(original), "{}: {id}", catalog.id);
            }
        }
    }

    #[test]
    fn direct_ui_message_ids_exist_in_english_catalog() {
        // Catch a misspelled key before it is shown verbatim in the UI. Engine-table labels
        // deliberately use the separate label adapter and fall back to their source text.
        for source in [
            include_str!("toolbar.rs"),
            include_str!("prefs.rs"),
            include_str!("documents.rs"),
            include_str!("dialogs.rs"),
            include_str!("browser.rs"),
            include_str!("sketch_palette.rs"),
        ] {
            for call in [".tr(\"", "i18n::tr(\"", "i18n::format(\""] {
                for tail in source.split(call).skip(1) {
                    let id = tail.split('"').next().unwrap();
                    assert!(ENGLISH.messages().contains_key(id), "missing English message: {id}");
                }
            }
        }
    }

    #[test]
    fn keys_and_engine_labels_translate_with_english_fallback() {
        let ja = Locale::for_language("ja");
        assert_eq!(ja.text("ui.preferences"), "環境設定");
        assert_eq!(ja.label("Extrude"), "押し出し");
        assert_eq!(ja.label("CREATE SKETCH"), "スケッチを作成");
        assert_eq!(ja.label("NonExistentKey"), "NonExistentKey");
        assert_eq!(Locale::for_language("unknown").text("ui.preferences"), "Preferences");
        static PARTIAL: Catalog = Catalog::new("test", "Test", r#"{"ui.preferences":"Prefs"}"#);
        assert_eq!(Locale(&PARTIAL).text("ui.preferences"), "Prefs");
        assert_eq!(Locale(&PARTIAL).text("ui.extrude"), "Extrude");
    }

    #[test]
    fn templates_allow_reordering_and_literal_braces_in_values() {
        assert_eq!(interpolate("{second} then {first}", &[("first", "{second}"), ("second", "B")]), "B then {second}");
        assert_eq!(interpolate("{missing} / {", &[]), "{missing} / {");
        assert_eq!(Locale::for_language("ja").format("document.save_changes", &[("name", "My {design}")]), "My {design} への変更を保存しますか？");
        assert_eq!(Locale::for_language("en").format("dialog.selected_count", &[("count", "2")]), "↖ 2 selected");
    }

    #[test]
    fn browser_patterns_preserve_numbers_and_custom_names() {
        let ja = Locale::for_language("ja");
        assert_eq!(ja.dynamic("Sketch1"), "スケッチ1");
        assert_eq!(ja.dynamic("Body2"), "ボディ2");
        assert_eq!(ja.dynamic("Component12"), "コンポーネント12");
        assert_eq!(ja.dynamic("Units: mm"), "単位: mm");
        assert_eq!(ja.dynamic("Precision: 3"), "精度: 3");
        assert_eq!(ja.dynamic("Parameters (5)"), "パラメータ (5)");
        assert_eq!(ja.dynamic("4 degrees of freedom"), "自由度 4");
        assert_eq!(ja.dynamic("Sketch of bracket"), "Sketch of bracket");
        assert_eq!(Locale::for_language("en").dynamic("Sketch1"), "Sketch1");
    }

    #[test]
    fn render_scopes_restore_the_previous_language() {
        let _english = scope("en");
        {
            let _japanese = scope("ja");
            assert_eq!(tr("ui.preferences"), "環境設定");
            {
                let _nested = scope("en");
                assert_eq!(tr("ui.preferences"), "Preferences");
            }
            assert_eq!(tr("ui.preferences"), "環境設定");
        }
        assert_eq!(tr("ui.preferences"), "Preferences");
    }
}
