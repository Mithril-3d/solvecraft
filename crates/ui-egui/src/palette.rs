//! The command line / palette: type a command id with JSON parameters
//! (`Extrude {"distance": 20}`), or part of a command name to search (press S anywhere).

use egui::{RichText, vec2};
use serde_json::Value;
use solidcraft_engine::command_specs;

use crate::SolveApp;
use crate::theme::Tokens;

#[derive(Default)]
pub struct Palette {
    pub text: String,
    pub history: Vec<String>,
    focus: bool,
}

/// Commands whose id or label matches the query: (id, label in the current language, icon).
/// The label matches in the current language and in English, so a name typed in either finds
/// the command.
pub fn matches(q: &str) -> Vec<(&'static str, &'static str, &'static str)> {
    let q = q.trim().to_lowercase();
    let mut v: Vec<(bool, bool, &'static str, &'static str, &'static str)> = command_specs()
        .into_iter()
        .filter_map(|c| {
            let label = crate::i18n::tr_label(c.label);
            let (shown, english, id) = (label.to_lowercase(), c.label.to_lowercase(), c.id.to_lowercase());
            let found = q.is_empty() || shown.contains(&q) || english.contains(&q) || id.contains(&q);
            found.then(|| (!(shown.starts_with(&q) || english.starts_with(&q)), !id.starts_with(&q), label, c.id, c.icon))
        })
        .collect();
    v.sort();
    v.into_iter().map(|(_, _, label, id, icon)| (id, label, icon)).collect()
}

/// Execute a line of palette input.
pub fn submit(app: &mut SolveApp, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    app.palette.history.push(line.to_string());
    let (id, rest) = match line.find(|c: char| c.is_whitespace() || c == '{') {
        Some(i) => (&line[..i], line[i..].trim()),
        None => (line, ""),
    };
    if solidcraft_engine::find_command(id).is_some() {
        if rest.is_empty() {
            app.start(id);
        } else {
            match serde_json::from_str::<Value>(rest) {
                Ok(p) => {
                    let _ = app.run(id, p);
                }
                Err(e) => app.set_status(format!("parameters must be JSON: {e}"), true),
            }
        }
        return;
    }
    if let Some((id, _, _)) = matches(line).first() {
        app.start(id);
    } else {
        app.set_status(format!("no command matches `{line}`"), true);
    }
}

pub fn palette_bar(app: &mut SolveApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    egui::Panel::bottom("sc_palette").exact_size(28.0).frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(8, 3))).show(
        ui,
        |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(RichText::new("›").strong().color(t.accent));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut app.palette.text)
                        // The JSON example keeps its braces here rather than in the catalog, so the
                        // placeholder parser never has to treat JSON punctuation as a name.
                        .hint_text(crate::i18n::format("ui.command_palette_hint", &[("example", "Extrude {\"distance\": 20}")]))
                        .desired_width(520.0),
                );
                if app.palette.focus {
                    r.request_focus();
                    app.palette.focus = false;
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let line = std::mem::take(&mut app.palette.text);
                    submit(app, &line);
                }
                if let Some(last) = app.session.log.last() {
                    ui.label(RichText::new(last).size(11.5).color(t.text_dim));
                }
            });
        },
    );
}

/// The search popup (S).
pub fn popup(app: &mut SolveApp, ctx: &egui::Context) {
    if !app.ui.palette_open {
        return;
    }
    let mut open = true;
    let mut picked: Option<&'static str> = None;
    let t = Tokens::get();
    let center = ctx.content_rect().center();
    crate::frame::window(ctx, crate::i18n::tr_label("Search commands"), crate::frame::Width::Normal)
        .id(egui::Id::new("sc_search"))
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(egui::pos2(center.x - 200.0, 150.0))
        .fixed_size(vec2(400.0, 340.0))
        .collapsible(false)
        .open(&mut open)
        .show(ctx, |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut app.palette.text).hint_text(crate::i18n::tr_label("command name")).desired_width(380.0));
            // Read Enter before taking the focus back (lost_focus reads the focus as it is now).
            let entered = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            r.request_focus();
            let list = matches(&app.palette.text);
            if entered {
                picked = list.first().map(|x| x.0);
            }
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for (id, label, icon) in list.iter().take(60) {
                    let enabled = solidcraft_engine::find_command(id).is_some_and(|c| c.info(&app.session).enabled);
                    ui.horizontal(|ui| {
                        let (ir, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
                        crate::icons::paint(ui.painter(), ir, icon, t.icon, t.icon_fill, t.accent);
                        if ui.add_enabled(enabled, egui::Button::new(*label).frame(false)).clicked() {
                            picked = Some(id);
                        }
                        ui.label(RichText::new(*id).size(10.5).color(t.text_dim));
                    });
                }
            });
        });
    if let Some(id) = picked {
        app.palette.text.clear();
        app.ui.palette_open = false;
        app.start(id);
    } else if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.palette_open = false;
        app.palette.text.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The command line finds a name typed in the current language or in English (#2544).
    #[test]
    fn matches_finds_a_name_in_the_current_language_and_in_english() {
        let _ja = crate::i18n::scope("ja");
        let name = crate::i18n::Locale::for_language("ja").text("cmd.sketch_rectangle_center");
        for query in [name, "Center Rectangle", "sketch.rectangle.center"] {
            let found = matches(query);
            assert_eq!(found.first().map(|m| (m.0, m.1)), Some(("sketch.rectangle.center", name)), "{query:?}");
        }
    }
}
