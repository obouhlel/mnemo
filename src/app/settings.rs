//! Fenêtre des paramètres et ses onglets Connexion, Plugins, GitHub et À propos
//! (l'onglet Apparence est dans appearance.rs).

use super::*;

impl MnemoApp {
    pub(super) fn settings_window(&mut self, ctx: &egui::Context, now: f64) {
        let p = self.pal();
        let mut open = self.show_settings;
        let screen = ctx.content_rect();
        let size =
            egui::vec2((screen.width() - 80.0).clamp(500.0, 1000.0), (screen.height() - 90.0).clamp(360.0, 720.0));
        let modal = egui::Modal::new(egui::Id::unique("settings-modal"))
            .backdrop_color(Color32::from_black_alpha(if self.cfg.dark_mode { 110 } else { 50 }))
            .frame(
                egui::Frame::new()
                    .fill(p.bg)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(10.0)
                    .shadow(ctx.global_style().visuals.window_shadow),
            );
        let resp = modal.show(ctx, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                // left navigation
                egui::Frame::new()
                    .fill(p.bg2)
                    .corner_radius(egui::CornerRadius { nw: 10, sw: 10, ne: 0, se: 0 })
                    .inner_margin(egui::Margin::symmetric(10, 14))
                    .show(ui, |ui| {
                        ui.set_width(200.0);
                        ui.set_min_height(size.y - 28.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(RichText::new("Options").size(12.0).color(p.faint).strong());
                            ui.add_space(4.0);
                            for (t, label) in [
                                (SettingsTab::Connection, "Base de données"),
                                (SettingsTab::Appearance, "Apparence"),
                                (SettingsTab::Github, "GitHub"),
                                (SettingsTab::About, "À propos"),
                            ] {
                                if theme::tree_row(ui, label, 0, self.settings_tab == t, None).clicked() {
                                    self.settings_tab = t;
                                }
                            }
                            ui.add_space(14.0);
                            ui.label(RichText::new("Plugins").size(12.0).color(p.faint).strong());
                            ui.add_space(4.0);
                            if theme::tree_row(
                                ui,
                                "Plugins communautaires",
                                0,
                                self.settings_tab == SettingsTab::Plugins,
                                None,
                            )
                            .clicked()
                            {
                                self.settings_tab = SettingsTab::Plugins;
                            }
                        });
                    });
                // content
                ui.vertical(|ui| {
                    ui.set_width(size.x - 220.0);
                    ui.set_max_width(size.x - 220.0);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 34.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.add_space(8.0);
                            if theme::icon_button_sized(ui, "\u{e1b2}", "Fermer", false, true, 16.0).clicked() {
                                open = false;
                            }
                        },
                    );
                    egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(size.y - 44.0).show(ui, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin { left: 28, right: 28, top: 0, bottom: 20 }).show(
                            ui,
                            |ui| {
                                ui.set_max_width(size.x - 220.0 - 56.0);
                                let title = match self.settings_tab {
                                    SettingsTab::Connection => "Base de données",
                                    SettingsTab::Appearance => "Apparence",
                                    SettingsTab::Plugins => "Plugins",
                                    SettingsTab::Github => "GitHub",
                                    SettingsTab::About => "À propos",
                                };
                                ui.label(RichText::new(title).size(20.0).strong().color(p.text));
                                ui.add_space(12.0);
                                self.settings_content(ui, now);
                            },
                        );
                    });
                });
            });
        });
        if resp.should_close() {
            open = false;
        }
        self.show_settings = open;
    }

    pub(super) fn settings_content(&mut self, ui: &mut egui::Ui, now: f64) {
        {
            match self.settings_tab {
                SettingsTab::Connection => {
                    ui.label("URL PostgreSQL");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.url_input)
                                .password(!self.show_password)
                                .desired_width(420.0),
                        );
                        ui.checkbox(&mut self.show_password, "Afficher");
                    });
                    if ui.button("Se (re)connecter").clicked() {
                        self.connect();
                    }
                    ui.add_space(8.0);
                    ui.label(format!("Connecté à : {}", redact_url(&self.cfg.database_url)));
                    ui.label(format!("Live (LISTEN/NOTIFY) : {}", if self.live { "actif" } else { "inactif" }));
                    ui.weak(format!("Identifiant de cette instance : {}", self.client_id));
                    ui.add_space(8.0);
                    ui.weak(format!(
                        "Fichier de configuration : {}",
                        crate::config::config_dir().join("config.toml").display()
                    ));
                    ui.weak("La variable d'environnement MNEMO_DATABASE_URL a priorité sur ce réglage.");
                }
                SettingsTab::Appearance => self.appearance_settings(ui, now),
                SettingsTab::Plugins => self.plugins_settings(ui, now),
                SettingsTab::Github => self.github_settings(ui),
                SettingsTab::About => {
                    ui.heading("Mnemo");
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.label("Base de connaissances Markdown native (Rust + egui), stockée dans PostgreSQL.");
                    ui.label("Synchronisation temps réel via LISTEN/NOTIFY, plugins en Rhai.");
                    ui.add_space(8.0);
                    ui.label(RichText::new("Raccourcis").strong());
                    for (_, name, key) in CORE_COMMANDS.iter().filter(|c| !c.2.is_empty()) {
                        ui.label(format!("{key:>14}   {name}"));
                    }
                    ui.label(format!("{:>14}   Palette de commandes", "Ctrl+P"));
                }
            }
        }
    }

    pub(super) fn plugins_settings(&mut self, ui: &mut egui::Ui, now: f64) {
        ui.horizontal(|ui| {
            if ui.button("⟳ Recharger").clicked() {
                self.actions.push(UiAction::Core(CoreCmd::ReloadPlugins));
            }
            if ui.button("Réinstaller les plugins d'exemple").clicked() {
                match self.plugins.install_bundled(true) {
                    Ok(()) => self.actions.push(UiAction::Core(CoreCmd::ReloadPlugins)),
                    Err(e) => self.notice(format!("Erreur : {e}"), true, now),
                }
            }
        });
        ui.weak(format!("Dossier des plugins : {}", self.plugins.dir.display()));
        ui.weak("L'activation et les réglages sont stockés dans la base : ils sont synchronisés entre vos appareils.");
        ui.add_space(6.0);
        let mut to_save: Vec<(String, PluginState)> = vec![];
        let mut retry = None;
        for (i, p) in self.plugins.plugins.iter().enumerate() {
            let m = &p.manifest;
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let mut enabled = p.enabled;
                    if ui.checkbox(&mut enabled, RichText::new(&m.name).strong()).changed() {
                        let mut st = self.plugins.state_of(&m.id);
                        st.enabled = enabled;
                        to_save.push((m.id.clone(), st));
                    }
                    ui.weak(format!(
                        "v{}  ·  {}{}",
                        m.version,
                        m.id,
                        if m.author.is_empty() { String::new() } else { format!("  ·  par {}", m.author) }
                    ));
                });
                if !m.description.is_empty() {
                    ui.label(&m.description);
                }
                if let Some(e) = &p.error {
                    ui.colored_label(Color32::from_rgb(230, 90, 90), e);
                    if ui.small_button("Réessayer").clicked() {
                        retry = Some(i);
                    }
                }
                if !m.settings.is_empty() {
                    egui::CollapsingHeader::new("Réglages").id_salt(("plugin-settings", &m.id)).show(ui, |ui| {
                        let stored = self.plugins.state_of(&m.id);
                        let edit = self.plugin_edit.entry(m.id.clone()).or_insert_with(|| {
                            let mut map = serde_json::Map::new();
                            for d in &m.settings {
                                let v = stored
                                    .settings
                                    .get(&d.key)
                                    .cloned()
                                    .or_else(|| d.default.as_ref().map(toml_to_json))
                                    .unwrap_or(serde_json::Value::String(String::new()));
                                map.insert(d.key.clone(), v);
                            }
                            map
                        });
                        for d in &m.settings {
                            let v = edit.entry(d.key.clone()).or_insert(serde_json::Value::Null);
                            ui.label(&d.label);
                            match v {
                                serde_json::Value::Bool(b) => {
                                    ui.checkbox(b, "");
                                }
                                serde_json::Value::Number(num) => {
                                    let mut x = num.as_f64().unwrap_or(0.0);
                                    if ui.add(egui::DragValue::new(&mut x)).changed() {
                                        *v = if x.fract() == 0.0 {
                                            serde_json::json!(x as i64)
                                        } else {
                                            serde_json::json!(x)
                                        };
                                    }
                                }
                                other => {
                                    let mut s = other.as_str().map(str::to_string).unwrap_or_default();
                                    let multiline = d.kind.as_deref() == Some("multiline");
                                    let r = if multiline {
                                        ui.add(
                                            egui::TextEdit::multiline(&mut s)
                                                .desired_rows(4)
                                                .desired_width(f32::INFINITY)
                                                .code_editor(),
                                        )
                                    } else {
                                        ui.add(egui::TextEdit::singleline(&mut s).desired_width(f32::INFINITY))
                                    };
                                    if r.changed() {
                                        *other = serde_json::Value::String(s);
                                    }
                                }
                            }
                        }
                        if ui.button("Enregistrer les réglages").clicked() {
                            let mut st = stored.clone();
                            st.enabled = p.enabled;
                            st.settings = edit.clone();
                            to_save.push((m.id.clone(), st));
                        }
                    });
                }
            });
            ui.add_space(4.0);
        }
        if self.plugins.plugins.is_empty() {
            ui.weak("Aucun plugin trouvé dans le dossier.");
        }
        for (id, st) in to_save {
            // optimistic local apply, then persist (the DB broadcasts to other devices)
            let mut states: HashMap<String, PluginState> = self
                .plugins
                .plugins
                .iter()
                .map(|p| (p.manifest.id.clone(), self.plugins.state_of(&p.manifest.id)))
                .collect();
            states.insert(id.clone(), st.clone());
            self.plugins.apply_states(states);
            self.plugin_changed();
            self.db.send(Cmd::SetPlugin { id, state: st });
        }
        if let Some(i) = retry {
            self.plugins.clear_error(i);
            self.plugin_changed();
        }
    }

    pub(super) fn github_settings(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        ui.add(egui::Label::new(RichText::new(
            "Reliez vos projets du tableau de bord à des dépôts GitHub : import des issues, publication des tickets, \
             synchronisation du titre, de la description, des labels (tags) et de l'état ouvert / fermé.",
        ).color(p.muted)).wrap());
        ui.add_space(10.0);
        ui.label(RichText::new("Token d'accès personnel").color(p.text));
        ui.add(egui::Label::new(RichText::new(
            "GitHub › Settings › Developer settings › Personal access tokens. Fine-grained : accès « Issues : lecture et écriture » \
             sur les dépôts voulus ; classique : portée « repo ». Le token est stocké uniquement dans le fichier de configuration local \
             (ou la variable MNEMO_GITHUB_TOKEN), jamais dans la base.",
        ).size(12.5).color(p.faint)).wrap());
        ui.horizontal(|ui| {
            let hint = if self.cfg.github_token.is_some() {
                "•••••••• (enregistré)"
            } else {
                "ghp_… ou github_pat_…"
            };
            ui.add(
                egui::TextEdit::singleline(&mut self.github_input).password(true).hint_text(hint).desired_width(320.0),
            );
            if ui.button("Enregistrer").clicked() && !self.github_input.trim().is_empty() {
                self.cfg.github_token = Some(self.github_input.trim().to_string());
                self.github_input.clear();
                self.cfg.save();
                self.send_github_config();
                self.dash.github_user = None;
            }
            if self.cfg.github_token.is_some() && ui.button("Supprimer").clicked() {
                self.cfg.github_token = None;
                self.cfg.save();
                self.send_github_config();
                self.dash.github_user = None;
            }
        });
        ui.add_space(8.0);
        ui.label(RichText::new("URL de l'API").color(p.text));
        let r = ui.add(egui::TextEdit::singleline(&mut self.cfg.github_api).desired_width(320.0));
        ui.label(
            RichText::new("https://api.github.com, ou https://<serveur>/api/v3 pour GitHub Enterprise")
                .size(12.0)
                .color(p.faint),
        );
        if r.lost_focus() {
            self.cfg.save();
            self.send_github_config();
        }
        ui.add_space(8.0);
        if ui
            .checkbox(
                &mut self.cfg.github_auto_sync,
                "Synchroniser automatiquement toutes les 5 minutes (tableau de bord ouvert)",
            )
            .changed()
        {
            self.cfg.save();
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(self.cfg.github_token.is_some(), egui::Button::new("Tester la connexion")).clicked() {
                self.dash.github_user = None;
                self.db.send(Cmd::GithubTest);
            }
            match &self.dash.github_user {
                Some(Ok(login)) => {
                    ui.label(
                        RichText::new(format!("Connecté en tant que {login}"))
                            .color(crate::board::Status::Done.color()),
                    );
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(Color32::from_rgb(0xfb, 0x46, 0x4c)));
                }
                None => {}
            }
        });
        ui.add_space(10.0);
        let linked: Vec<String> = self
            .board
            .projects
            .iter()
            .filter_map(|pr| pr.github_repo.as_ref().map(|r| format!("{} → {r}", pr.name)))
            .collect();
        ui.label(RichText::new("Projets reliés").color(p.text));
        if linked.is_empty() {
            ui.label(
                RichText::new("Aucun : ouvrez le tableau de bord › Modifier le projet › Dépôt GitHub.").color(p.faint),
            );
        }
        for l in linked {
            ui.label(RichText::new(format!("• {l}")).color(p.muted));
        }
    }

    pub(super) fn send_github_config(&self) {
        let cfg = self
            .cfg
            .github_token
            .clone()
            .filter(|t| !t.trim().is_empty())
            .map(|token| crate::github::GhConfig { token, api: self.cfg.github_api.clone() });
        self.db.send(Cmd::SetGithub(cfg));
    }
}
