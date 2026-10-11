use std::sync::Arc;

/// Install font bytes as fallback for Proportional and Monospace font families.
/// Returns true if the font was parsed and installed successfully.
pub fn install_cjk_font(ctx: &egui::Context, bytes: Vec<u8>) -> bool {
    if ttf_parser::Face::parse(&bytes, 0).is_err() {
        return false;
    }

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("cjk_fallback".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
    fonts.families.entry(egui::FontFamily::Proportional).or_default().push("cjk_fallback".to_owned());
    fonts.families.entry(egui::FontFamily::Monospace).or_default().push("cjk_fallback".to_owned());
    ctx.set_fonts(fonts);
    true
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
fn scan_dir_for_font(dir: &std::path::Path, keywords: &[&str], depth_remaining: usize) -> Option<Vec<u8>> {
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
                && keywords.iter().any(|k| name.contains(k))
                && let Some(bytes) = try_read_font(&p)
            {
                return Some(bytes);
            }
        } else if p.is_dir() {
            subdirs.push(p);
        }
    }

    for sub in subdirs {
        if let Some(bytes) = scan_dir_for_font(&sub, keywords, depth_remaining.saturating_sub(1)) {
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
            if let Some(bytes) = scan_dir_for_font(&user_fonts, &["biz", "gothic", "noto", "yugoth"], 2) {
                return Some(bytes);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let dirs = ["/System/Library/Fonts", "/System/Library/Fonts/Supplemental", "/Library/Fonts"];
        let keywords = ["hiragino", "pingfang", "arial unicode", "songti", "gothic"];
        for d in dirs {
            if let Some(bytes) = scan_dir_for_font(std::path::Path::new(d), &keywords, 2) {
                return Some(bytes);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let search_dirs = [std::path::PathBuf::from("/usr/share/fonts"), std::path::PathBuf::from("/usr/local/share/fonts")];
        let keywords = ["notosanscjk", "noto", "cjk", "takao", "vlgothic", "ipa", "droid"];
        for dir in &search_dirs {
            if let Some(bytes) = scan_dir_for_font(dir, &keywords, 3) {
                return Some(bytes);
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            let user_fonts = std::path::PathBuf::from(home).join(".local/share/fonts");
            if let Some(bytes) = scan_dir_for_font(&user_fonts, &keywords, 2) {
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
