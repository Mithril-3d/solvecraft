# UI localization

SolveCraft embeds its translations and works offline. English and Japanese are currently
registered in `crates/ui-egui/src/i18n.rs`. Adding a language requires an embedded catalog and
one registration there; the preferences selector and validation use the same registry.

## Messages

`crates/ui-egui/src/locales/en.json` defines stable message IDs and English fallback text.
Keep IDs when changing wording. Other catalogs use the same IDs; missing messages fall back
to English. Use `app.tr("ui.preferences")` for static messages and
`Locale::format` for full sentences with named placeholders. Translate the entire sentence,
including punctuation, so a translator can reorder its arguments. Placeholder values are
inserted literally, even if a document name contains braces.

The English-label adapter (`app.tr_label`) supports existing engine and UI tables. It maps
display text to a message ID, then uses the same catalog lookup. New UI messages should use
stable IDs directly. The adapter does not change command IDs, model names, or scenario handles.
When a table's English wording changes, update the corresponding English catalog entry too.
Generated browser labels use named templates while their underlying model names stay intact.

Catalogs are parsed once and looked up through ordered maps. This is offline localization,
not a claim of zero runtime overhead. The catalog tests check registered locale IDs,
translation keys, and placeholder consistency. Run `cargo xtask ci` before submitting changes.

## Rendering and coverage

One immutable `Locale` type handles all translations. Table widgets without an app reference
use a render scope that restores the previous locale when it exits, including nested renders.
Standalone translated panels enter the same scope. Measure rendered text to size custom
widgets; UTF-8 byte lengths do not describe their displayed width.

The desktop host tries system CJK fonts across Windows, macOS and Linux, as well as an optional
`CRAFT_FONTS_DIR` font pack, and skips files that cannot be parsed. No fonts are redistributed
in native desktop distribution packages; native builds rely entirely on existing operating system fonts
or craft-fonts.

In the browser, `solvecraft-web` bundles a self-hosted Japanese font (`BIZUDPGothic-Regular.ttf`, licensed
under SIL OFL 1.1) within the web distribution artifacts only. The font is loaded asynchronously from the same
origin (`./fonts/BIZUDPGothic-Regular.ttf`) on-demand when the interface language is set to Japanese, without
contacting any external CDNs or third-party servers. For users with an English interface, no font download
occurs. If font loading fails, the web interface gracefully falls back to English to prevent missing glyphs
(tofu). This bundled font specifically targets Japanese; Korean (Hangul) and Chinese scripts are not covered.

The current messages support named substitution, not locale-specific plural selection,
number/date formatting, or right-to-left layout and shaping. Add those capabilities when
introducing messages or languages that need them; use an established message formatter for
plural rules instead of adding language branches to panels. The catalogs cover the UI surfaces
introduced by this PR, not every engine error or every newer command dialog.
