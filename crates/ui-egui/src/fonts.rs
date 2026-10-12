//! System font loading and fallback setup for multi-language (CJK/Japanese) support.
//!
//! The UI's Latin font always comes first; a CJK font is only ever appended as a fallback, so
//! English text renders exactly as before and only glyphs the Latin font lacks use the CJK face.

use std::sync::Arc;

/// Name of the CJK fallback entry in [`egui::FontDefinitions`].
pub const CJK_FALLBACK: &str = "cjk_fallback";

/// UI languages whose text needs the CJK font. The web app's bundled font covers Japanese only;
/// add Chinese or Korean here together with a font that covers them.
pub const CJK_LANGUAGES: &[&str] = &["ja"];

/// Whether the UI language needs a CJK font (the web app fetches one only then).
pub fn needs_cjk_font(language: &str) -> bool {
    CJK_LANGUAGES.contains(&language)
}

/// `fonts` with `bytes` appended as the last fallback of the Proportional and Monospace
/// families, after the Latin UI font. `None` when the bytes are not a font.
pub fn with_cjk_fallback(mut fonts: egui::FontDefinitions, bytes: Vec<u8>) -> Option<egui::FontDefinitions> {
    ttf_parser::Face::parse(&bytes, 0).ok()?;
    fonts.font_data.insert(CJK_FALLBACK.to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        list.retain(|name| name != CJK_FALLBACK);
        list.push(CJK_FALLBACK.to_owned());
    }
    Some(fonts)
}

/// Install font bytes as fallback for Proportional and Monospace font families.
/// Returns true if the font was parsed and installed successfully.
pub fn install_cjk_font(ctx: &egui::Context, bytes: Vec<u8>) -> bool {
    match with_cjk_fallback(egui::FontDefinitions::default(), bytes) {
        Some(fonts) => {
            ctx.set_fonts(fonts);
            true
        }
        None => false,
    }
}

/// Lower-case file-name fragments that identify CJK fonts. Deliberately specific: generic
/// words such as "noto" or "gothic" also match Latin-only fonts (NotoSans-Regular, Century Gothic).
pub const CJK_FONT_KEYWORDS: &[&str] = &[
    "cjk",
    "notosansjp",
    "notoserifjp",
    "bizud",
    "biz-ud",
    "yugoth",
    "meiryo",
    "msgothic",
    "hiragino",
    "pingfang",
    "songti",
    "arial unicode",
    "ipag",
    "ipaexg",
    "takao",
    "vlgothic",
    "droidsansfallback",
];

/// Whether a font file name looks like a CJK font.
pub fn is_cjk_font_name(file_name: &str) -> bool {
    let name = file_name.to_ascii_lowercase();
    CJK_FONT_KEYWORDS.iter().any(|k| name.contains(k))
}

/// Session state of the web app's on-demand CJK font. Pure state so it can be tested off the web:
/// the shell does the fetch when [`CjkFontLoader::tick`] asks for it.
#[derive(Debug, Default)]
pub struct CjkFontLoader {
    status: CjkFontStatus,
    /// The saved language while this session falls back to English after a failed load.
    saved_language: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CjkFontStatus {
    #[default]
    Unloaded,
    Loading,
    Installed,
    Failed,
}

impl CjkFontLoader {
    pub fn status(&self) -> CjkFontStatus {
        self.status
    }

    /// Called every frame with the live language. Returns true when the font fetch should start
    /// now (only for a CJK language, only once). After a failed load a CJK language falls back to
    /// English for this session; the saved preference is kept (see [`Self::persisted_language`]).
    pub fn tick(&mut self, language: &mut String) -> bool {
        if self.saved_language.is_some() && language != "en" {
            // The user picked a language themselves since the fallback: that is the preference now.
            self.saved_language = None;
        }
        if !needs_cjk_font(language) {
            return false;
        }
        match self.status {
            CjkFontStatus::Unloaded => {
                self.status = CjkFontStatus::Loading;
                true
            }
            CjkFontStatus::Failed => {
                self.fall_back(language);
                false
            }
            CjkFontStatus::Loading | CjkFontStatus::Installed => false,
        }
    }

    /// Record the outcome of the fetch and install.
    pub fn finished(&mut self, installed: bool, language: &mut String) {
        if installed {
            self.status = CjkFontStatus::Installed;
        } else {
            self.status = CjkFontStatus::Failed;
            if needs_cjk_font(language) {
                self.fall_back(language);
            }
        }
    }

    fn fall_back(&mut self, language: &mut String) {
        self.saved_language = Some(std::mem::replace(language, "en".to_owned()));
    }

    /// The language to write to storage: the user's own choice, not this session's fallback.
    pub fn persisted_language<'a>(&'a self, live: &'a str) -> &'a str {
        self.saved_language.as_deref().unwrap_or(live)
    }
}

/// Candidate font paths to look for on different operating systems.
#[cfg(not(target_arch = "wasm32"))]
const FONT_CANDIDATES: &[&str] = &[
    // Windows
    "C:\\Windows\\Fonts\\meiryo.ttc",
    "C:\\Windows\\Fonts\\YuGothM.ttc",
    "C:\\Windows\\Fonts\\YuGothR.ttc",
    "C:\\Windows\\Fonts\\msgothic.ttc",
    "C:\\Windows\\Fonts\\BIZ-UDGothicR.ttc",
    "C:\\Windows\\Fonts\\BIZ-UDMinchoM.ttc",
    "C:\\Windows\\Fonts\\yumin.ttf",
    // macOS
    "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
    "/System/Library/Fonts/Hiragino Sans W3.ttc",
    "/System/Library/Fonts/Hiragino Sans W4.ttc",
    "/System/Library/Fonts/Hiragino Sans W6.ttc",
    "/System/Library/Fonts/Supplemental/Hiragino Sans W3.ttc",
    "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    "/System/Library/Fonts/Supplemental/Songti.ttc",
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    "/Library/Fonts/Arial Unicode.ttf",
    "/Library/Fonts/Hiragino Sans W3.ttc",
    // Linux
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJKjp-Regular.otf",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf",
    "/usr/share/fonts/truetype/fonts-japanese-gothic.ttf",
    "/usr/share/fonts/truetype/takao-gothic/TakaoPGothic.ttf",
    "/usr/share/fonts/truetype/takao-gothic/TakaoGothic.ttf",
    "/usr/share/fonts/truetype/vlgothic/VL-PGothic-Regular.ttf",
    "/usr/share/fonts/truetype/ipafont/ipag.ttf",
    "/usr/share/fonts/truetype/ipaexfont/ipaexg.ttf",
    "/usr/share/fonts/opentype/ipafont/ipag.ttf",
    "/usr/share/fonts/opentype/ipaexfont/ipaexg.ttf",
    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
    "/usr/share/fonts/cjkuni-uming/uming.ttc",
];

#[cfg(not(target_arch = "wasm32"))]
fn try_read_font(path: &std::path::Path) -> Option<Vec<u8>> {
    if let Ok(bytes) = std::fs::read(path)
        && ttf_parser::Face::parse(&bytes, 0).is_ok()
    {
        Some(bytes)
    } else {
        None
    }
}

/// Recursively search for matching CJK font files up to a bounded depth.
#[cfg(not(target_arch = "wasm32"))]
fn scan_dir_for_font(dir: &std::path::Path, depth_remaining: usize) -> Option<Vec<u8>> {
    if depth_remaining == 0 {
        return None;
    }
    let rd = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();

    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_file() {
            let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            let is_font = name.ends_with(".ttc") || name.ends_with(".ttf") || name.ends_with(".otf");
            if is_font
                && is_cjk_font_name(&name)
                && let Some(bytes) = try_read_font(&p)
            {
                return Some(bytes);
            }
        } else if p.is_dir() {
            subdirs.push(p);
        }
    }

    for sub in subdirs {
        if let Some(bytes) = scan_dir_for_font(&sub, depth_remaining.saturating_sub(1)) {
            return Some(bytes);
        }
    }

    None
}

/// Find a CJK fallback font on the local system or in CRAFT_FONTS_DIR.
#[cfg(not(target_arch = "wasm32"))]
fn find_system_cjk_font() -> Option<Vec<u8>> {
    // 1. Check optional craft-fonts pack directory if specified.
    if let Ok(craft_dir) = std::env::var("CRAFT_FONTS_DIR") {
        let dir = std::path::Path::new(&craft_dir);
        let names = ["BIZUDPGothic-Regular.ttf", "BIZ-UDGothicR.ttc", "NotoSansCJK-Regular.ttc", "NotoSansJP-Regular.ttf"];
        for name in names {
            if let Some(bytes) = try_read_font(&dir.join(name)) {
                return Some(bytes);
            }
        }
    }

    // 2. Check well-known candidate paths.
    for path in FONT_CANDIDATES {
        if let Some(bytes) = try_read_font(std::path::Path::new(path)) {
            return Some(bytes);
        }
    }

    // 3. Platform-specific dynamic directory search.
    #[cfg(target_os = "windows")]
    {
        if let Ok(windir) = std::env::var("WINDIR").or_else(|_| std::env::var("SystemRoot")) {
            let fonts_dir = std::path::PathBuf::from(windir).join("Fonts");
            let names = ["meiryo.ttc", "YuGothM.ttc", "YuGothR.ttc", "msgothic.ttc", "BIZ-UDGothicR.ttc"];
            for name in names {
                if let Some(bytes) = try_read_font(&fonts_dir.join(name)) {
                    return Some(bytes);
                }
            }
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let user_fonts = std::path::PathBuf::from(local).join("Microsoft").join("Windows").join("Fonts");
            if let Some(bytes) = scan_dir_for_font(&user_fonts, 2) {
                return Some(bytes);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let dirs = ["/System/Library/Fonts", "/System/Library/Fonts/Supplemental", "/Library/Fonts"];
        for d in dirs {
            if let Some(bytes) = scan_dir_for_font(std::path::Path::new(d), 2) {
                return Some(bytes);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let search_dirs = [std::path::PathBuf::from("/usr/share/fonts"), std::path::PathBuf::from("/usr/local/share/fonts")];
        for dir in &search_dirs {
            if let Some(bytes) = scan_dir_for_font(dir, 3) {
                return Some(bytes);
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            let user_fonts = std::path::PathBuf::from(home).join(".local/share/fonts");
            if let Some(bytes) = scan_dir_for_font(&user_fonts, 2) {
                return Some(bytes);
            }
        }
    }

    None
}

/// Configure font definitions with system CJK fallback if available.
#[cfg(not(target_arch = "wasm32"))]
pub fn setup_fonts(ctx: &egui::Context) {
    if let Some(bytes) = find_system_cjk_font() {
        install_cjk_font(ctx, bytes);
    }
}

/// On web, setup_fonts does not perform synchronous disk reads.
/// Use asynchronous loading through [`solvecraft_web`] or host scripts.
#[cfg(target_arch = "wasm32")]
pub fn setup_fonts(_ctx: &egui::Context) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn latin_font_bytes() -> Vec<u8> {
        let defaults = egui::FontDefinitions::default();
        let name = &defaults.families[&egui::FontFamily::Proportional][0];
        defaults.font_data[name].font.to_vec()
    }

    #[test]
    fn default_ui_language_is_english() {
        assert_eq!(crate::prefs::Prefs::default().language, "en");
        assert!(!needs_cjk_font("en"));
    }

    #[test]
    fn cjk_font_is_a_fallback_after_the_latin_ui_font() {
        let defaults = egui::FontDefinitions::default();
        let fonts = with_cjk_fallback(defaults.clone(), latin_font_bytes()).unwrap();
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let list = &fonts.families[&family];
            assert_eq!(list.first(), defaults.families[&family].first(), "the Latin UI font stays first");
            assert_eq!(list.last().map(String::as_str), Some(CJK_FALLBACK), "CJK is the last fallback");
        }
        assert!(with_cjk_fallback(defaults, b"not a font".to_vec()).is_none());
    }

    #[test]
    fn directory_scan_matches_cjk_fonts_only() {
        for cjk in [
            "NotoSansCJK-Regular.ttc",
            "NotoSansJP-Regular.ttf",
            "BIZUDPGothic-Regular.ttf",
            "YuGothM.ttc",
            "meiryo.ttc",
            "msgothic.ttc",
            "Hiragino Sans W3.ttc",
            "ipag.ttf",
            "TakaoPGothic.ttf",
            "DroidSansFallbackFull.ttf",
        ] {
            assert!(is_cjk_font_name(cjk), "{cjk} is a CJK font");
        }
        for latin in ["NotoSans-Regular.ttf", "NotoSerif-Bold.ttf", "NotoMono-Regular.ttf", "GOTHIC.TTF", "DejaVuSans.ttf", "DroidSans.ttf"] {
            assert!(!is_cjk_font_name(latin), "{latin} is not a CJK font");
        }
    }

    #[test]
    fn web_font_is_fetched_only_for_a_cjk_language() {
        let mut loader = CjkFontLoader::default();
        for lang in ["en", "de"] {
            let mut language = lang.to_owned();
            assert!(!loader.tick(&mut language), "no CJK font fetch for {lang}");
        }
        let mut language = "ja".to_owned();
        assert!(loader.tick(&mut language), "Japanese starts the fetch");
        assert!(!loader.tick(&mut language), "the fetch starts once");
        loader.finished(true, &mut language);
        assert_eq!(loader.status(), CjkFontStatus::Installed);
        assert_eq!(language, "ja");
    }

    #[test]
    fn failed_font_fetch_falls_back_for_the_session_without_changing_the_saved_language() {
        let mut loader = CjkFontLoader::default();
        let mut language = "ja".to_owned();
        assert!(loader.tick(&mut language));
        loader.finished(false, &mut language);
        assert_eq!(language, "en", "this session shows English instead of tofu");
        assert_eq!(loader.persisted_language(&language), "ja", "the saved preference stays Japanese");
        assert!(!loader.tick(&mut language));
        assert_eq!(loader.persisted_language(&language), "ja");

        // Choosing a language afterwards makes that the saved preference.
        language = "de".to_owned();
        assert!(!loader.tick(&mut language));
        assert_eq!(loader.persisted_language(&language), "de");
    }
}
