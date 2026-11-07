//! Thèmes et snippets CSS au format Obsidian : dossiers, chargement, rechargement à chaud
//! et onglet « Apparence » des paramètres.

use super::*;

const EXAMPLE_SNIPPET: &str = include_str!("../../assets/texts/example-snippet.css");

impl MnemoApp {
    pub(super) fn css_dirs(&self) -> Vec<std::path::PathBuf> {
        let mut dirs = vec![];
        if let Some(d) = &self.cfg.obsidian_dir {
            dirs.push(d.clone());
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

    /// (Re)reads themes, snippets and appearance.json and applies their CSS variables.
    pub(super) fn reload_css(&mut self) {
        let dirs = self.css_dirs();
        let (themes, snippets) = crate::css::discover(&dirs);
        self.css.themes = themes;
        self.css.snippets = snippets;
        let mut watched: Vec<std::path::PathBuf> = vec![];
        let mut extra: Vec<(String, String)> = vec![];
        let mut report = vec![];
        let (theme_name, snippet_names) = match (&self.cfg.obsidian_dir, self.cfg.follow_appearance) {
            (Some(dir), true) => {
                watched.push(dir.join("appearance.json"));
                match crate::css::read_appearance(dir) {
                    Some(a) => {
                        if let Some(size) = a.base_font_size {
                            extra.push(("--font-text-size".into(), format!("{size}px")));
                        }
                        if let Some(acc) = &a.accent {
                            for k in [
                                "--color-accent",
                                "--interactive-accent",
                                "--text-accent",
                                "--checkbox-color",
                                "--blockquote-border-color",
                            ] {
                                extra.push((k.into(), acc.clone()));
                            }
                        }
                        if let Some(dark) = a.dark
                            && dark != self.cfg.dark_mode
                        {
                            self.cfg.dark_mode = dark;
                            self.cfg.save();
                        }
                        report.push("appearance.json lu".to_string());
                        (a.css_theme, a.snippets)
                    }
                    None => {
                        report.push("appearance.json introuvable dans ce dossier".to_string());
                        (self.cfg.css_theme.clone(), self.cfg.css_snippets.clone())
                    }
                }
            }
            _ => (self.cfg.css_theme.clone(), self.cfg.css_snippets.clone()),
        };
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
            if names.iter().any(|n| n.starts_with("--font-") && !n.ends_with("-size")) {
                report.push("Polices personnalisées (--font-text / --font-interface) non prises en charge : Inter reste utilisée".into());
            }
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
        self.css.report = report;
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

    pub(super) fn appearance_settings(&mut self, ui: &mut egui::Ui, now: f64) {
        let p = self.palette();
        let following = self.cfg.follow_appearance && self.cfg.obsidian_dir.is_some();
        let row = |ui: &mut egui::Ui, title: &str, desc: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
            super::vault::settings_row(ui, &p, title, desc, |ui| add(ui));
        };
        let mut changed = false;
        let mut dark = self.cfg.dark_mode;
        row(ui, "Thème de base", "Sombre ou clair.", &mut |ui| {
            egui::ComboBox::from_id_salt("base-theme").selected_text(if dark { "Sombre" } else { "Clair" }).show_ui(
                ui,
                |ui| {
                    ui.selectable_value(&mut dark, true, "Sombre");
                    ui.selectable_value(&mut dark, false, "Clair");
                },
            );
        });
        if dark != self.cfg.dark_mode {
            self.cfg.dark_mode = dark;
            changed = true;
        }
        let mut size = self.cfg.font_size;
        row(
            ui,
            "Taille de police",
            "Taille du texte de l'éditeur et du mode lecture (remplacée par --font-text-size si un thème la définit).",
            &mut |ui| {
                ui.add(egui::Slider::new(&mut size, 11.0..=26.0).step_by(1.0).suffix(" px"));
            },
        );
        if size != self.cfg.font_size {
            self.cfg.font_size = size;
            changed = true;
        }

        ui.add_space(10.0);
        ui.label(RichText::new("Thèmes et snippets CSS").size(16.0).strong().color(p.text));
        ui.add(egui::Label::new(RichText::new(
            "Mnemo lit les fichiers CSS au format Obsidian (themes/<Nom>/theme.css, snippets/*.css) et applique leurs variables CSS \
             (--background-primary, --text-normal, --interactive-accent, --font-text-size, --file-line-width…). Les fichiers sont rechargés à chaud.",
        ).size(12.5).color(p.muted)).wrap());
        ui.add_space(10.0);

        let mut input = self.css.obsidian_input.clone();
        let mut apply_dir = false;
        row(
            ui,
            "Dossier .obsidian d'un coffre",
            "Facultatif : réutilise les thèmes, snippets et appearance.json d'un coffre Obsidian existant.",
            &mut |ui| {
                if ui.button("Appliquer").clicked() {
                    apply_dir = true;
                }
                ui.add(egui::TextEdit::singleline(&mut input).hint_text("~/MonCoffre/.obsidian").desired_width(220.0));
            },
        );
        self.css.obsidian_input = input;
        if apply_dir {
            let raw = self.css.obsidian_input.trim();
            let expanded = if let Some(rest) = raw.strip_prefix("~/") {
                directories::BaseDirs::new().map(|b| b.home_dir().join(rest)).unwrap_or_else(|| raw.into())
            } else {
                std::path::PathBuf::from(raw)
            };
            if raw.is_empty() {
                self.cfg.obsidian_dir = None;
            } else if expanded.is_dir() {
                // accept the vault folder itself too
                let dir = if expanded.join(".obsidian").is_dir() { expanded.join(".obsidian") } else { expanded };
                self.css.obsidian_input = dir.display().to_string();
                self.cfg.obsidian_dir = Some(dir);
            } else {
                self.notice(format!("Dossier introuvable : {}", expanded.display()), true, now);
            }
            changed = true;
        }
        if self.cfg.obsidian_dir.is_some() {
            let mut follow = self.cfg.follow_appearance;
            row(
                ui,
                "Suivre appearance.json",
                "Utilise le thème, les snippets activés, la taille de police, la couleur d'accent et le mode clair/sombre de ce coffre.",
                &mut |ui| {
                    ui.checkbox(&mut follow, "");
                },
            );
            if follow != self.cfg.follow_appearance {
                self.cfg.follow_appearance = follow;
                changed = true;
            }
        }

        let mut theme_sel = self.css.active_theme.clone();
        let names: Vec<String> = self.css.themes.iter().map(|t| t.0.clone()).collect();
        row(
            ui,
            "Thème",
            if following { "Défini par appearance.json." } else { "Thèmes trouvés dans les dossiers themes/." },
            &mut |ui| {
                ui.add_enabled_ui(!following, |ui| {
                    egui::ComboBox::from_id_salt("css-theme")
                        .selected_text(theme_sel.clone().unwrap_or_else(|| "Par défaut".into()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut theme_sel, None, "Par défaut");
                            for n in &names {
                                ui.selectable_value(&mut theme_sel, Some(n.clone()), n);
                            }
                        });
                });
            },
        );
        if !following && theme_sel != self.css.active_theme {
            self.cfg.css_theme = theme_sel;
            changed = true;
        }

        ui.add_space(6.0);
        ui.label(RichText::new("Snippets CSS").color(p.text));
        ui.add_space(4.0);
        if self.css.snippets.is_empty() {
            ui.label(RichText::new("Aucun snippet trouvé.").color(p.faint));
        }
        let snippets = self.css.snippets.clone();
        for (name, path) in &snippets {
            let mut on = self.css.active_snippets.contains(name);
            let before = on;
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!following, |ui| ui.checkbox(&mut on, ""));
                ui.label(RichText::new(name).color(p.text));
                ui.add(
                    egui::Label::new(RichText::new(path.display().to_string()).size(11.5).color(p.faint)).truncate(),
                )
                .on_hover_text(path.display().to_string());
            });
            if on != before && !following {
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
        ui.add_space(8.0);
        if ui.button("Recharger").clicked() {
            changed = true;
        }
        ui.add_space(4.0);
        for d in self.css_dirs() {
            ui.add(
                egui::Label::new(RichText::new(format!("Dossier lu : {}", d.display())).size(11.5).color(p.faint))
                    .truncate(),
            )
            .on_hover_text(d.display().to_string());
        }
        ui.add_space(8.0);
        egui::Frame::new().fill(p.bg2).corner_radius(6.0).inner_margin(10.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Rapport").size(12.5).strong().color(p.muted));
            if self.css.report.is_empty() {
                ui.label(RichText::new("Thème par défaut, aucun CSS chargé.").size(12.5).color(p.faint));
            }
            for line in &self.css.report {
                ui.add(egui::Label::new(RichText::new(format!("• {line}")).size(12.5).color(p.muted)).wrap());
            }
        });
        if changed {
            self.cfg.save();
            self.reload_css();
        }
    }
}
