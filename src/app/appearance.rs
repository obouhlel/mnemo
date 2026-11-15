//! Thèmes et snippets CSS au format Obsidian : dossiers, chargement, rechargement à chaud
//! et onglet « Apparence » des paramètres.

use super::settings::{
    COMBO_WIDTH, PickTarget, filtering, open_folder, path_label, setting_heading, setting_heading_with, setting_row,
    setting_row_tip, settings_note, small_icon_button,
};
use super::*;
use crate::config::ThemeMode;

/// Documentation des thèmes et des variables CSS prises en charge.
const THEMES_DOC: &str = "https://github.com/obouhlel/mnemo/blob/main/docs/themes.md";

const EXAMPLE_SNIPPET: &str = include_str!("../../assets/texts/example-snippet.css");

/// Variables CSS qui reçoivent la couleur d'accent (appearance.json ou paramètres).
const ACCENT_VARS: [&str; 5] =
    ["--color-accent", "--interactive-accent", "--text-accent", "--checkbox-color", "--blockquote-border-color"];

impl MnemoApp {
    pub(super) fn css_dirs(&self) -> Vec<std::path::PathBuf> {
        let mut dirs = vec![];
        if let Some(d) = self.obsidian_dir() {
            dirs.push(d);
        }
        dirs.push(crate::config::config_dir());
        dirs
    }

    pub(super) fn install_example_snippet(&self) {
        let dir = crate::config::config_dir().join("snippets");
        let file = dir.join("exemple-accent-vert.css");
        if !dir.exists() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::create_dir_all(crate::config::config_dir().join("themes"));
            let _ = std::fs::write(&file, EXAMPLE_SNIPPET);
        }
    }

    /// (Re)lit thèmes, snippets, appearance.json et app.json, puis applique leurs variables CSS,
    /// la couleur d'accent et les polices.
    pub(super) fn reload_css(&mut self) {
        let dirs = self.css_dirs();
        let (themes, snippets) = crate::css::discover(&dirs);
        self.css.themes = themes;
        self.css.snippets = snippets;
        let mut watched: Vec<std::path::PathBuf> = vec![];
        let mut extra: Vec<(String, String)> = vec![];
        let mut report = vec![];
        let mut layout = crate::css::EditorLayout::default();
        // polices choisies dans les paramètres (remplacées par celles du coffre si son apparence est utilisée)
        let mut fonts = theme::FontChoice {
            text: self.cfg.text_font.clone(),
            interface: self.cfg.interface_font.clone(),
            mono: self.cfg.mono_font.clone(),
        };
        self.css.vault_appearance = None;
        let (theme_name, snippet_names) = match (self.obsidian_dir(), self.cfg.follow_appearance) {
            (Some(dir), true) => {
                watched.push(dir.join("appearance.json"));
                watched.push(dir.join("app.json"));
                layout = crate::css::read_app_json(&dir);
                match crate::css::read_appearance(&dir) {
                    Some(a) => {
                        layout.ribbon = a.show_ribbon.unwrap_or(true);
                        fonts = theme::FontChoice {
                            text: a.text_font.clone(),
                            interface: a.interface_font.clone(),
                            mono: a.mono_font.clone(),
                        };
                        if let Some(size) = a.base_font_size {
                            extra.push(("--font-text-size".into(), format!("{size}px")));
                        }
                        if let Some(acc) = &a.accent {
                            extra.extend(ACCENT_VARS.iter().map(|k| (k.to_string(), acc.clone())));
                        }
                        // le mode clair / sombre du coffre devient le mode choisi (sync_theme_mode l'applique)
                        if let Some(dark) = a.dark {
                            let mode = if dark { ThemeMode::Dark } else { ThemeMode::Light };
                            if self.cfg.mode() != mode {
                                self.cfg.theme_mode = Some(mode);
                                self.cfg.dark_mode = dark;
                                self.cfg.save();
                            }
                        }
                        report.push("Apparence du coffre Obsidian lue (appearance.json)".to_string());
                        let picked = (a.css_theme.clone(), a.snippets.clone());
                        self.css.vault_appearance = Some(a);
                        picked
                    }
                    None => {
                        report.push("Réglages d'apparence (appearance.json) introuvables dans ce coffre".to_string());
                        (self.cfg.css_theme.clone(), self.cfg.css_snippets.clone())
                    }
                }
            }
            _ => (self.cfg.css_theme.clone(), self.cfg.css_snippets.clone()),
        };
        // couleur d'accent choisie dans les paramètres (quand l'apparence du coffre n'est pas utilisée)
        if self.css.vault_appearance.is_none()
            && let Some(acc) = &self.cfg.accent_color
        {
            extra.extend(ACCENT_VARS.iter().map(|k| (k.to_string(), acc.clone())));
        }
        let mut parsed = crate::css::Parsed::default();
        let mut files = vec![];
        if let Some(name) = &theme_name {
            match self.css.themes.iter().find(|(n, _)| n == name) {
                Some((_, path)) => files.push((format!("thème « {name} »"), path.clone())),
                None => report.push(format!("Thème « {name} » introuvable (themes/{name}/theme.css)")),
            }
        }
        for name in &snippet_names {
            match self.css.snippets.iter().find(|(n, _)| n == name) {
                Some((_, path)) => files.push((format!("snippet « {name} »"), path.clone())),
                None => report.push(format!("Snippet « {name} » introuvable")),
            }
        }
        for (label, path) in &files {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    crate::css::parse(&text, &mut parsed);
                    report.push(format!("{label} chargé"));
                }
                Err(e) => report.push(format!("{label} : lecture impossible ({e})")),
            }
            watched.push(path.clone());
        }
        self.css.active_theme = theme_name;
        self.css.active_snippets = snippet_names;
        if parsed.vars.is_empty() && extra.is_empty() {
            theme::set_override(None);
        } else {
            let d = crate::css::apply(&parsed, &extra, true, theme::builtin_palette(true), theme::Metrics::default());
            let l = crate::css::apply(&parsed, &extra, false, theme::builtin_palette(false), theme::Metrics::default());
            let mut names: Vec<&str> = parsed.vars.iter().map(|v| v.1.as_str()).collect();
            names.sort();
            names.dedup();
            let known = names.iter().filter(|n| crate::css::is_known(n)).count();
            report.push(format!(
                "{} variable(s) lue(s) : {known} appliquée(s) directement, {} autre(s) utilisée(s) seulement via var()",
                names.len(),
                names.len() - known
            ));
            if parsed.ignored_rules > 0 {
                report.push(format!(
                    "{} règle(s) ciblant des éléments précis ignorée(s) (non applicables hors navigateur)",
                    parsed.ignored_rules
                ));
            }
            // polices du thème, si ni appearance.json ni les paramètres n'en imposent
            let theme_font = |name: &str| {
                crate::css::var_value(&parsed, name, self.cfg.dark_mode).and_then(|v| crate::fonts::first_family(&v))
            };
            fonts.text = fonts.text.or_else(|| theme_font("--font-text-theme"));
            fonts.interface = fonts.interface.or_else(|| theme_font("--font-interface-theme"));
            fonts.mono = fonts.mono.or_else(|| theme_font("--font-monospace-theme"));
            let _ = (d.used.len(), l.used.len());
            theme::set_override(Some(theme::Override { dark: (d.palette, d.metrics), light: (l.palette, l.metrics) }));
        }
        let mtime = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        self.css.watched = watched
            .into_iter()
            .map(|p| {
                let t = mtime(&p);
                (p, t)
            })
            .collect();
        for (what, f) in [("texte", &fonts.text), ("interface", &fonts.interface), ("code", &fonts.mono)] {
            if let Some(f) = f {
                report.push(format!("Police du {what} : {f}"));
            }
        }
        self.css.report = report;
        self.css.layout = layout;
        self.css.fonts = fonts;
        self.theme_applied = None;
        self.preview_gen = u64::MAX;
    }

    /// Hot reload: re-apply CSS when a watched file changes (checked once per second).
    pub(super) fn watch_css(&mut self, ctx: &egui::Context, now: f64) {
        if now - self.css.last_check < 1.0 {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
            return;
        }
        self.css.last_check = now;
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        let changed = self.css.watched.iter().any(|(p, t)| std::fs::metadata(p).and_then(|m| m.modified()).ok() != *t);
        if changed {
            self.reload_css();
            self.notice("Thème CSS rechargé", false, now);
        }
    }

    /// Applique le mode clair / sombre choisi ; « Système » suit le réglage du système d'exploitation.
    pub(super) fn sync_theme_mode(&mut self, ctx: &egui::Context) {
        let dark = match self.cfg.mode() {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => ctx.system_theme().map_or(self.cfg.dark_mode, |t| t == egui::Theme::Dark),
        };
        if dark != self.cfg.dark_mode {
            self.cfg.dark_mode = dark;
            self.cfg.save();
        }
    }

    pub(super) fn appearance_settings(&mut self, ui: &mut egui::Ui, now: f64) {
        let p = self.palette();
        let obsidian = self.obsidian_dir();
        let vault_obsidian = self.vault_obsidian_dir();
        let following = self.cfg.follow_appearance && obsidian.is_some();
        let vault = if following { self.css.vault_appearance.clone() } else { None };
        let mut changed = false;
        let mut customize = false;

        // ------------------------------------------------------------------ thème
        setting_heading(ui, "Thème");
        if obsidian.is_some() {
            let mut follow = self.cfg.follow_appearance;
            setting_row(
                ui,
                "Utiliser l'apparence du coffre Obsidian",
                "Reprend le thème, la couleur d'accent, les snippets, les polices, la taille du texte et le mode \
                 clair / sombre réglés dans Obsidian.",
                |ui| {
                    theme::toggle(ui, &mut follow);
                },
            );
            if follow != self.cfg.follow_appearance {
                self.cfg.follow_appearance = follow;
                changed = true;
            }
        }
        let vault_dark = vault.as_ref().and_then(|a| a.dark);
        let mut mode = self.cfg.mode();
        let mode_desc = if vault_dark.is_some() {
            "Défini par le coffre Obsidian."
        } else {
            "Clair, sombre, ou comme le système."
        };
        setting_row(ui, "Mode", mode_desc, |ui| match vault_dark {
            Some(d) => readonly(ui, &p, if d { "Sombre" } else { "Clair" }, &mut customize),
            None => {
                let label = |m: ThemeMode| match m {
                    ThemeMode::Light => "Clair",
                    ThemeMode::Dark => "Sombre",
                    ThemeMode::System => "Système",
                };
                egui::ComboBox::from_id_salt("theme-mode").width(COMBO_WIDTH).selected_text(label(mode)).show_ui(
                    ui,
                    |ui| {
                        for m in [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System] {
                            ui.selectable_value(&mut mode, m, label(m));
                        }
                    },
                );
            }
        });
        if vault_dark.is_none() && mode != self.cfg.mode() {
            self.cfg.theme_mode = Some(mode);
            self.cfg.save();
            ui.ctx().request_repaint();
        }

        let mut theme_sel = self.css.active_theme.clone();
        let names: Vec<String> = self.css.themes.iter().map(|t| t.0.clone()).collect();
        let theme_desc = if following { "Thème choisi dans le coffre Obsidian." } else { "Thèmes installés." };
        setting_row(ui, "Thème", theme_desc, |ui| {
            if following {
                readonly(ui, &p, theme_sel.as_deref().unwrap_or("Par défaut"), &mut customize);
            } else {
                egui::ComboBox::from_id_salt("css-theme")
                    .width(COMBO_WIDTH)
                    .selected_text(theme_sel.clone().unwrap_or_else(|| "Par défaut".into()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut theme_sel, None, "Par défaut");
                        for n in &names {
                            ui.selectable_value(&mut theme_sel, Some(n.clone()), n);
                        }
                    });
            }
        });
        if !following && theme_sel != self.css.active_theme {
            self.cfg.css_theme = theme_sel;
            changed = true;
        }

        let accent_desc = if following {
            "Couleur d'accent du coffre Obsidian."
        } else {
            "Liens, cases cochées, sélection et éléments actifs."
        };
        let mut accent = self.cfg.accent_color.clone();
        setting_row(ui, "Couleur d'accent", accent_desc, |ui| {
            if following {
                let shown = vault.as_ref().and_then(|a| a.accent.clone());
                readonly(ui, &p, shown.as_deref().unwrap_or("Celle du thème"), &mut customize);
                if let Some(c) = shown.as_deref().and_then(crate::css::parse_color) {
                    swatch(ui, c);
                }
            } else {
                if accent.is_some() && small_icon_button(ui, "rotate-ccw", "Revenir à la couleur du thème").clicked()
                {
                    accent = None;
                }
                let mut c = accent.as_deref().and_then(crate::css::parse_color).unwrap_or(p.accent);
                if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).changed()
                {
                    accent = Some(format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b()));
                }
            }
        });
        if !following && accent != self.cfg.accent_color {
            self.cfg.accent_color = accent;
            changed = true;
        }

        // ------------------------------------------------------------------ texte
        setting_heading(ui, "Texte");
        let mut picks = [self.cfg.interface_font.clone(), self.cfg.text_font.clone(), self.cfg.mono_font.clone()];
        let from_vault = vault.as_ref().map(|a| [a.interface_font.clone(), a.text_font.clone(), a.mono_font.clone()]);
        for (i, (title, desc, desc_vault)) in [
            (
                "Police de l'interface",
                "Menus, panneaux et boutons. Polices installées sur le système.",
                "Police de l'interface du coffre Obsidian.",
            ),
            (
                "Police du texte",
                "Texte des notes : éditeur, mode lecture et titre.",
                "Police du texte du coffre Obsidian.",
            ),
            ("Police du code", "Blocs de code et code en ligne.", "Police du code du coffre Obsidian."),
        ]
        .into_iter()
        .enumerate()
        {
            let value = &mut picks[i];
            setting_row(ui, title, if following { desc_vault } else { desc }, |ui| match &from_vault {
                Some(v) => readonly(ui, &p, v[i].as_deref().unwrap_or("Par défaut"), &mut customize),
                None => font_combo(ui, ("font-choice", i), value),
            });
        }
        if !following {
            let [interface, text, mono] = picks;
            if interface != self.cfg.interface_font || text != self.cfg.text_font || mono != self.cfg.mono_font {
                self.cfg.interface_font = interface;
                self.cfg.text_font = text;
                self.cfg.mono_font = mono;
                changed = true;
            }
        }
        let vault_size = vault.as_ref().and_then(|a| a.base_font_size);
        let css_size = self.metrics().font_size;
        let mut size = self.cfg.font_size.round() as i32;
        let size_desc = if vault_size.is_some() {
            "Taille réglée dans le coffre Obsidian."
        } else if css_size.is_some() {
            "Taille du texte des notes (le thème actif impose sa propre taille)."
        } else {
            "Taille du texte des notes, en édition et en lecture."
        };
        setting_row(ui, "Taille du texte", size_desc, |ui| match vault_size {
            Some(s) => readonly(ui, &p, &format!("{} px", s.round()), &mut customize),
            None => {
                ui.add(egui::Slider::new(&mut size, 11..=26).suffix(" px"));
            }
        });
        if vault_size.is_none() && size as f32 != self.cfg.font_size.round() {
            self.cfg.font_size = size as f32;
            self.cfg.save();
        }

        // ------------------------------------------------------------------ snippets
        let snippet_dir = obsidian.clone().unwrap_or_else(crate::config::config_dir).join("snippets");
        let dirs: Vec<String> = self.css_dirs().iter().map(|d| d.display().to_string()).collect();
        let (mut reload, mut open) = (false, false);
        setting_heading_with(ui, "Snippets CSS", |ui| {
            if small_icon_button(
                ui,
                "rotate-cw",
                &format!("Recharger les thèmes et snippets\nDossiers lus :\n{}", dirs.join("\n")),
            )
            .clicked()
            {
                reload = true;
            }
            if small_icon_button(
                ui,
                "folder-open",
                &format!("Ouvrir le dossier des snippets\n{}", snippet_dir.display()),
            )
            .clicked()
            {
                open = true;
            }
        });
        if !filtering(ui) {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(
                    RichText::new("Fichiers CSS au format Obsidian, rechargés dès qu'ils changent.")
                        .size(12.5)
                        .color(p.muted),
                );
                ui.hyperlink_to(RichText::new("En savoir plus").size(12.5), THEMES_DOC);
            });
            ui.add_space(6.0);
        }
        if following {
            let snippets = vault.as_ref().map(|a| a.snippets.join(", ")).filter(|s| !s.is_empty());
            setting_row(ui, "Snippets activés", "Snippets activés dans le coffre Obsidian.", |ui| {
                readonly(ui, &p, snippets.as_deref().unwrap_or("Aucun"), &mut customize);
            });
        } else {
            if self.css.snippets.is_empty() {
                settings_note(ui, "Aucun snippet : ajoutez des fichiers .css dans le dossier des snippets.");
            }
            let snippets = self.css.snippets.clone();
            for (name, path) in &snippets {
                let mut on = self.css.active_snippets.contains(name);
                let before = on;
                setting_row_tip(ui, name, "", Some(&path.display().to_string()), |ui| {
                    theme::toggle(ui, &mut on);
                });
                if on != before {
                    let mut list = self.css.active_snippets.clone();
                    if on {
                        list.push(name.clone());
                    } else {
                        list.retain(|n| n != name);
                    }
                    self.cfg.css_snippets = list;
                    changed = true;
                }
            }
        }
        if open {
            open_folder(ui.ctx(), &snippet_dir);
        }
        if reload {
            changed = true;
        }

        // ------------------------------------------------------------------ avancé
        setting_heading(ui, "Avancé");
        let mut browse = false;
        let mut clear = false;
        match &vault_obsidian {
            // le dossier des notes est un coffre : son .obsidian est utilisé, pas de réglage à faire
            Some(dir) => {
                setting_row(
                    ui,
                    "Coffre Obsidian",
                    "Utilise l'apparence du coffre : le dossier des notes est un coffre Obsidian.",
                    |ui| path_label(ui, dir),
                );
            }
            None => {
                let current = self.cfg.obsidian_dir.clone();
                let busy = self.folder_pick.is_some();
                setting_row(
                    ui,
                    "Apparence d'un autre coffre Obsidian",
                    "Facultatif : reprend les thèmes, snippets et réglages d'apparence d'un coffre existant.",
                    |ui| {
                        if current.is_some() && small_icon_button(ui, "x", "Ne plus utiliser ce coffre").clicked() {
                            clear = true;
                        }
                        if ui.add_enabled(!busy, egui::Button::new("Parcourir…")).clicked() {
                            browse = true;
                        }
                        if let Some(dir) = &current {
                            path_label(ui, dir);
                        }
                    },
                );
            }
        }
        if browse {
            let start = self.cfg.obsidian_dir.clone().or_else(|| self.cfg.vault_dir.clone());
            self.pick_folder(ui.ctx(), PickTarget::Obsidian, start);
        }
        if clear {
            self.cfg.obsidian_dir = None;
            changed = true;
        }
        if !filtering(ui) {
            ui.add_space(8.0);
            egui::CollapsingHeader::new(RichText::new("Rapport de chargement").size(13.0).color(p.muted))
                .id_salt("css-report")
                .show(ui, |ui| {
                    if self.css.report.is_empty() {
                        ui.label(RichText::new("Thème par défaut, aucun CSS chargé.").size(12.5).color(p.faint));
                    }
                    for line in &self.css.report {
                        ui.add(egui::Label::new(RichText::new(format!("• {line}")).size(12.5).color(p.muted)).wrap());
                    }
                });
        }

        if customize {
            // reprend les valeurs du coffre comme point de départ
            if let Some(a) = &vault {
                self.cfg.css_theme = a.css_theme.clone();
                self.cfg.css_snippets = a.snippets.clone();
                if a.accent.is_some() {
                    self.cfg.accent_color = a.accent.clone();
                }
                if let Some(d) = a.dark {
                    self.cfg.theme_mode = Some(if d { ThemeMode::Dark } else { ThemeMode::Light });
                }
                if let Some(s) = a.base_font_size {
                    self.cfg.font_size = s.round();
                }
                for (dst, src) in [
                    (&mut self.cfg.interface_font, &a.interface_font),
                    (&mut self.cfg.text_font, &a.text_font),
                    (&mut self.cfg.mono_font, &a.mono_font),
                ] {
                    if src.is_some() {
                        *dst = src.clone();
                    }
                }
            }
            self.cfg.follow_appearance = false;
            changed = true;
        }
        if changed {
            self.cfg.save();
            self.reload_css();
            if reload {
                self.notice("Thèmes et snippets rechargés", false, now);
            }
        }
    }
}

/// Valeur reprise du coffre Obsidian (lecture seule), avec un lien « Personnaliser » qui reprend la main.
fn readonly(ui: &mut egui::Ui, p: &theme::Palette, value: &str, customize: &mut bool) {
    if ui
        .link(RichText::new("Personnaliser").size(12.5))
        .on_hover_text("Ne plus utiliser l'apparence du coffre")
        .clicked()
    {
        *customize = true;
    }
    ui.add_space(6.0);
    ui.add(egui::Label::new(RichText::new(value).color(p.text)).truncate());
}

/// Pastille de couleur.
fn swatch(ui: &mut egui::Ui, c: Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 4.0, c);
}

/// Menu des polices installées sur le système ; `None` : la police par défaut (celle du thème, sinon Inter).
fn font_combo(ui: &mut egui::Ui, id: (&str, usize), value: &mut Option<String>) {
    egui::ComboBox::from_id_salt(id)
        .width(COMBO_WIDTH)
        .height(320.0)
        .selected_text(value.clone().unwrap_or_else(|| "Par défaut".into()))
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "Par défaut");
            for f in crate::fonts::families() {
                ui.selectable_value(value, Some(f.clone()), f);
            }
        });
}
