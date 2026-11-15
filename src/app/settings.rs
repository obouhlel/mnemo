//! Fenêtre des paramètres façon Obsidian : barre latérale avec recherche, ligne de réglage commune
//! (`setting_row`), sélecteur de dossier natif, et onglets Synchronisation, Plugins, GitHub et À propos
//! (Apparence est dans appearance.rs, Dossier des notes dans vault.rs).

use super::*;
use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;

/// Largeur de la colonne des contrôles, la même sur toutes les pages.
pub(super) const CONTROL_WIDTH: f32 = 260.0;
/// Largeur des menus déroulants, la même partout.
pub(super) const COMBO_WIDTH: f32 = 200.0;

/// Onglets : (onglet, titre, icône Lucide). Les cinq premiers forment le groupe « Options ».
const TABS: [(SettingsTab, &str, &str); 6] = [
    (SettingsTab::Connection, "Synchronisation", "refresh-cw"),
    (SettingsTab::Vault, "Dossier des notes", "folder"),
    (SettingsTab::Appearance, "Apparence", "palette"),
    (SettingsTab::Github, "GitHub", "github"),
    (SettingsTab::About, "À propos", "info"),
    (SettingsTab::Plugins, "Plugins communautaires", "puzzle"),
];

fn search_id() -> egui::Id {
    egui::Id::unique("settings-search")
}
fn filter_id() -> egui::Id {
    egui::Id::unique("settings-filter")
}
fn hits_id() -> egui::Id {
    egui::Id::unique("settings-hits")
}

/// Recherche en cours (minuscules, sans accents) ; vide si aucune.
fn filter(ui: &egui::Ui) -> String {
    ui.data(|d| d.get_temp::<String>(filter_id())).unwrap_or_default()
}

/// Une recherche filtre les réglages : les textes d'explication sont masqués.
pub(super) fn filtering(ui: &egui::Ui) -> bool {
    !filter(ui).is_empty()
}

/// Caractère d'une icône Lucide, en texte.
pub(super) fn lucide_icon(name: &str) -> String {
    theme::lucide(name).map(String::from).unwrap_or_default()
}

/// La ligne de réglage commune à toutes les pages : nom et description à gauche (largeur fixe),
/// contrôle aligné à droite dans une colonne de largeur homogène, séparateur fin.
/// Renvoie `false` si la ligne est masquée par la recherche.
pub(super) fn setting_row(ui: &mut egui::Ui, name: &str, desc: &str, add: impl FnOnce(&mut egui::Ui)) -> bool {
    setting_row_tip(ui, name, desc, None, add)
}

/// Comme `setting_row`, avec une infobulle sur le nom (le chemin d'un snippet…).
pub(super) fn setting_row_tip(
    ui: &mut egui::Ui,
    name: &str,
    desc: &str,
    tip: Option<&str>,
    add: impl FnOnce(&mut egui::Ui),
) -> bool {
    let q = filter(ui);
    if !q.is_empty() {
        if !fold_accents(name).contains(&q) && !fold_accents(desc).contains(&q) {
            return false;
        }
        ui.data_mut(|d| *d.get_temp_mut_or_default::<usize>(hits_id()) += 1);
    }
    let p = theme::palette(ui.visuals().dark_mode);
    let width = ui.available_width();
    let control_w = CONTROL_WIDTH.min(width * 0.45);
    let gap = 24.0;
    let text_w = (width - control_w - gap).max(120.0);
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let text =
            ui.allocate_ui_with_layout(egui::vec2(text_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = 3.0;
                let r = ui.add(egui::Label::new(RichText::new(name).color(p.text)).wrap());
                if let Some(tip) = tip {
                    r.on_hover_text(tip);
                }
                if !desc.is_empty() {
                    ui.add(egui::Label::new(RichText::new(desc).size(12.5).color(p.muted)).wrap());
                }
            });
        ui.add_space(gap);
        let h = text.response.rect.height().max(ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(egui::vec2(control_w, h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.set_min_size(egui::vec2(control_w, h));
            ui.spacing_mut().item_spacing.x = 6.0;
            add(ui);
        });
    });
    ui.add_space(10.0);
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, p.border));
    ui.add_space(6.0);
    true
}

/// Titre d'une section dans une page (masqué pendant une recherche).
pub(super) fn setting_heading(ui: &mut egui::Ui, title: &str) {
    setting_heading_with(ui, title, |_| {});
}

/// Titre de section suivi de petits boutons (icônes) à sa droite.
pub(super) fn setting_heading_with(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    if filtering(ui) {
        return;
    }
    let p = theme::palette(ui.visuals().dark_mode);
    ui.add_space(18.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(15.5).strong().color(p.text));
        ui.add_space(4.0);
        add(ui);
    });
    ui.add_space(4.0);
}

/// Texte d'explication (masqué pendant une recherche).
pub(super) fn settings_note(ui: &mut egui::Ui, text: &str) {
    if filtering(ui) {
        return;
    }
    let p = theme::palette(ui.visuals().dark_mode);
    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.muted)).wrap());
    ui.add_space(10.0);
}

/// Chemin raccourci (`…/Obsidian/.obsidian`, `~/Notes`), le chemin complet en infobulle.
pub(super) fn path_label(ui: &mut egui::Ui, path: &Path) {
    let p = theme::palette(ui.visuals().dark_mode);
    // ≈ 7 px par caractère à cette taille : le chemin est raccourci par le début plutôt que tronqué à la fin
    let max = ((ui.available_width() / 7.0) as usize).clamp(12, 60);
    ui.add(
        egui::Label::new(RichText::new(crate::config::display_path(path, max)).size(13.0).color(p.muted)).truncate(),
    )
    .on_hover_text(path.display().to_string());
}

/// Petit bouton icône des paramètres (recharger, ouvrir le dossier…).
pub(super) fn small_icon_button(ui: &mut egui::Ui, icon: &str, tooltip: &str) -> egui::Response {
    theme::icon_button_sized(ui, &lucide_icon(icon), tooltip, false, true, 15.0)
}

/// Ouvre un dossier dans le gestionnaire de fichiers du système.
pub(super) fn open_folder(ctx: &egui::Context, dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    ctx.open_url(egui::OpenUrl::new_tab(super::attachments::file_uri(dir)));
}

/// À quoi sert le dossier demandé au sélecteur natif.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum PickTarget {
    Notes,
    Obsidian,
}

/// Sélecteur de dossier natif ouvert sur un autre thread (il ne bloque pas l'interface).
pub(super) struct FolderPick {
    target: PickTarget,
    rx: std::sync::mpsc::Receiver<Option<PathBuf>>,
}

impl MnemoApp {
    /// Ouvre le sélecteur de dossier du système ; le résultat est appliqué dès qu'il arrive.
    pub(super) fn pick_folder(&mut self, ctx: &egui::Context, target: PickTarget, start: Option<PathBuf>) {
        if self.folder_pick.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let title = match target {
            PickTarget::Notes => "Dossier des notes",
            PickTarget::Obsidian => "Dossier d'un coffre Obsidian",
        };
        std::thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new().set_title(title);
            if let Some(dir) = start.filter(|d| d.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            let _ = tx.send(dialog.pick_folder());
            ctx.request_repaint();
        });
        self.folder_pick = Some(FolderPick { target, rx });
    }

    fn poll_folder_pick(&mut self, now: f64) {
        let Some(pick) = &self.folder_pick else { return };
        let picked = match pick.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => None,
        };
        let target = pick.target;
        self.folder_pick = None;
        let Some(dir) = picked else { return };
        match target {
            PickTarget::Notes => {
                let dir = super::vault::notes_dir_of(dir);
                if Some(&dir) != self.cfg.vault_dir.as_ref() {
                    self.set_notes_dir(dir, now);
                }
            }
            PickTarget::Obsidian => {
                // le dossier du coffre convient aussi
                let dir = if dir.join(".obsidian").is_dir() { dir.join(".obsidian") } else { dir };
                self.cfg.obsidian_dir = Some(dir);
                self.cfg.save();
                self.reload_css();
            }
        }
    }

    pub(super) fn settings_window(&mut self, ctx: &egui::Context, now: f64) {
        self.poll_folder_pick(now);
        let p = self.palette();
        let mut open = self.show_settings;
        let screen = ctx.content_rect();
        // ≈ 80 % de la fenêtre, au plus 1000 × 700
        let size = egui::vec2(
            (screen.width() * 0.8).min(1000.0).max((screen.width() - 16.0).min(560.0)),
            (screen.height() * 0.8).min(700.0).max((screen.height() - 16.0).min(380.0)),
        );
        let sidebar_w = 220.0;
        let modal = egui::Modal::new(egui::Id::unique("settings-modal"))
            .backdrop_color(Color32::from_black_alpha(if self.cfg.dark_mode { 130 } else { 70 }))
            .frame(
                egui::Frame::new()
                    .fill(p.bg)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(12.0)
                    .shadow(ctx.global_style().visuals.window_shadow),
            );
        let resp = modal.show(ctx, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                self.settings_sidebar(ui, &p, sidebar_w, size.y);
                // contenu
                ui.vertical(|ui| {
                    let content_w = size.x - sidebar_w;
                    ui.set_width(content_w);
                    ui.set_max_width(content_w);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.allocate_ui_with_layout(
                        egui::vec2(content_w, 40.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.add_space(10.0);
                            if theme::icon_button_sized(ui, "\u{e1b2}", "Fermer", false, true, 16.0).clicked() {
                                open = false;
                            }
                        },
                    );
                    // marge à droite : la barre de défilement ne colle ni au bord ni au bouton de fermeture
                    egui::Frame::new().inner_margin(egui::Margin { right: 6, bottom: 8, ..Default::default() }).show(
                        ui,
                        |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("settings-scroll")
                                .auto_shrink([false, false])
                                .max_height(size.y - 48.0)
                                .show(ui, |ui| {
                                    egui::Frame::new()
                                        .inner_margin(egui::Margin { left: 36, right: 30, top: 0, bottom: 24 })
                                        .show(ui, |ui| {
                                            ui.set_max_width(content_w - 6.0 - 66.0);
                                            // la fenêtre a le fond de l'éditeur : champs sur le fond des panneaux
                                            ui.visuals_mut().text_edit_bg_color = Some(p.bg2);
                                            ui.visuals_mut().extreme_bg_color = p.bg2;
                                            self.settings_pages(ui, &p, now);
                                        });
                                });
                        },
                    );
                });
            });
        });
        if resp.should_close() {
            open = false;
        }
        if !open {
            // la recherche ne survit pas à la fermeture
            ctx.data_mut(|d| {
                d.remove::<String>(search_id());
                d.remove::<String>(filter_id());
            });
        }
        self.show_settings = open;
    }

    /// Barre latérale pleine hauteur : recherche, puis les onglets avec leur icône.
    fn settings_sidebar(&mut self, ui: &mut egui::Ui, p: &theme::Palette, width: f32, height: f32) {
        egui::Frame::new()
            .fill(p.bg2)
            .corner_radius(egui::CornerRadius { nw: 12, sw: 12, ne: 0, se: 0 })
            .inner_margin(egui::Margin::symmetric(10, 12))
            .show(ui, |ui| {
                ui.set_width(width - 20.0);
                ui.set_min_height(height - 24.0);
                // la ligne parente est horizontale : les entrées s'empilent dans une colonne
                ui.vertical(|ui| self.settings_nav(ui, p));
            });
    }

    /// Contenu de la barre latérale : recherche, groupes et onglets.
    fn settings_nav(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut q = ui.data_mut(|d| d.get_temp_mut_or_default::<String>(search_id()).clone());
        // champ sur le fond de l'éditeur : il se détache de la barre latérale
        ui.visuals_mut().text_edit_bg_color = Some(p.bg);
        let r = ui.add(
            egui::TextEdit::singleline(&mut q)
                .hint_text("Rechercher…")
                .desired_width(f32::INFINITY)
                .margin(egui::Margin::symmetric(8, 5)),
        );
        if r.changed() {
            let folded = fold_accents(q.trim());
            ui.data_mut(|d| {
                d.insert_temp(search_id(), q.clone());
                d.insert_temp(filter_id(), folded);
            });
        }
        ui.add_space(14.0);
        let searching = !q.trim().is_empty();
        let group = |ui: &mut egui::Ui, title: &str| {
            egui::Frame::new()
                .inner_margin(egui::Margin { left: 8, ..Default::default() })
                .show(ui, |ui| ui.label(RichText::new(title).size(12.5).color(p.muted).strong()));
            ui.add_space(4.0);
        };
        group(ui, "Options");
        for (i, (tab, label, icon)) in TABS.iter().enumerate() {
            if i == 5 {
                ui.add_space(14.0);
                group(ui, "Plugins");
            }
            let active = !searching && self.settings_tab == *tab;
            if theme::tree_row_icon(ui, label, 0, active, None, Some(*icon)).clicked() {
                self.settings_tab = *tab;
                ui.data_mut(|d| {
                    d.remove::<String>(search_id());
                    d.remove::<String>(filter_id());
                });
            }
        }
    }

    /// La page de l'onglet choisi, ou, pendant une recherche, les réglages correspondants de toutes les pages.
    fn settings_pages(&mut self, ui: &mut egui::Ui, p: &theme::Palette, now: f64) {
        let q = filter(ui);
        if q.is_empty() {
            let title = TABS.iter().find(|t| t.0 == self.settings_tab).map(|t| t.1).unwrap_or_default();
            ui.label(RichText::new(title).size(21.0).strong().color(p.text));
            ui.add_space(14.0);
            self.settings_content(ui, now);
            return;
        }
        let mut total = 0;
        for (i, (tab, title, _)) in TABS.iter().enumerate() {
            // le titre d'une page n'apparaît que si elle avait des résultats (frame précédente)
            let key = egui::Id::unique(("settings-page-hits", i));
            let before = ui.data(|d| d.get_temp::<usize>(key)).unwrap_or(1);
            if before > 0 {
                ui.add_space(if total == 0 { 0.0 } else { 18.0 });
                ui.label(RichText::new(*title).size(13.0).strong().color(p.faint));
                ui.add_space(4.0);
            }
            ui.data_mut(|d| d.insert_temp(hits_id(), 0usize));
            let saved = self.settings_tab;
            self.settings_tab = *tab;
            self.settings_content(ui, now);
            self.settings_tab = saved;
            let hits = ui.data(|d| d.get_temp::<usize>(hits_id())).unwrap_or(0);
            if hits != before {
                ui.data_mut(|d| d.insert_temp(key, hits));
                ui.ctx().request_repaint();
            }
            total += hits;
        }
        if total == 0 {
            let raw = ui.data(|d| d.get_temp::<String>(search_id())).unwrap_or_default();
            ui.label(RichText::new(format!("Aucun réglage ne correspond à « {} ».", raw.trim())).color(p.muted));
        }
    }

    pub(super) fn settings_content(&mut self, ui: &mut egui::Ui, now: f64) {
        match self.settings_tab {
            SettingsTab::Connection => self.connection_settings(ui, now),
            SettingsTab::Vault => self.vault_settings(ui, now),
            SettingsTab::Appearance => self.appearance_settings(ui, now),
            SettingsTab::Plugins => self.plugins_settings(ui, now),
            SettingsTab::Github => self.github_settings(ui),
            SettingsTab::About => self.about_settings(ui),
        }
    }

    fn connection_settings(&mut self, ui: &mut egui::Ui, now: f64) {
        settings_note(
            ui,
            "Facultatif. Vos notes restent dans votre dossier local ; une base PostgreSQL permet de les \
             retrouver sur plusieurs appareils, en direct, et d'utiliser le tableau de bord. Hors connexion, \
             tout continue en local et se synchronise au retour.",
        );
        let (mut connect, mut stop) = (false, false);
        let mut url = std::mem::take(&mut self.url_input);
        let mut show = self.show_password;
        setting_row(
            ui,
            "Base PostgreSQL",
            "Adresse de connexion. Ajoutez ?sslmode=require pour un serveur distant (Neon, Supabase…), sinon la \
             connexion et le mot de passe ne sont pas chiffrés. MNEMO_DATABASE_URL a priorité sur ce réglage.",
            |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut url)
                        .password(!show)
                        .hint_text("postgres://utilisateur:motdepasse@hôte:5432/base")
                        .desired_width(ui.available_width()),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    connect = true;
                }
            },
        );
        setting_row(ui, "Afficher l'adresse", "Montre le mot de passe contenu dans l'adresse.", |ui| {
            theme::toggle(ui, &mut show);
        });
        self.url_input = url;
        self.show_password = show;
        let state = match &self.conn {
            ConnState::Connected if self.live => "Synchronisé en direct".to_string(),
            ConnState::Connected => "Connecté, flux temps réel en reconnexion".to_string(),
            ConnState::Connecting => "Connexion…".to_string(),
            ConnState::Disconnected(Some(e)) => format!("Hors ligne, nouvel essai toutes les 30 s ({e})"),
            ConnState::Disconnected(None) => "Local uniquement".to_string(),
        };
        let configured = !self.cfg.database_url.is_empty();
        let detail =
            if configured { redact_url(&self.cfg.database_url) } else { "Aucune base configurée.".to_string() };
        setting_row(ui, &state, &detail, |ui| {
            if configured && ui.button("Arrêter").on_hover_text("Arrêter la synchronisation").clicked() {
                stop = true;
            }
            if ui.button(if configured { "Reconnecter" } else { "Connecter" }).clicked() {
                connect = true;
            }
        });
        if connect {
            self.connect();
        }
        if stop {
            self.disconnect();
            self.notice("Synchronisation arrêtée : Mnemo continue avec le dossier local", false, now);
        }
        settings_note(
            ui,
            "Pas encore de base ? docker run -d --name mnemo-db -e POSTGRES_PASSWORD=postgres \
             -e POSTGRES_DB=vault -p 5432:5432 postgres:16",
        );
        let config = crate::config::config_dir();
        let mut open_config = false;
        setting_row(ui, "Cette instance", &format!("Identifiant : {}", self.client_id), |ui| {
            if small_icon_button(ui, "folder-open", "Ouvrir le dossier de configuration").clicked() {
                open_config = true;
            }
            path_label(ui, &config.join("config.toml"));
        });
        if open_config {
            open_folder(ui.ctx(), &config);
        }
    }

    fn about_settings(&mut self, ui: &mut egui::Ui) {
        setting_row(
            ui,
            &format!("Mnemo {}", env!("CARGO_PKG_VERSION")),
            "Base de connaissances Markdown native (Rust + egui). Synchronisation temps réel par PostgreSQL \
             (LISTEN/NOTIFY), plugins en Rhai.",
            |_| {},
        );
        setting_heading(ui, "Raccourcis");
        let p = self.palette();
        let shortcuts = CORE_COMMANDS
            .iter()
            .filter(|c| !c.2.is_empty())
            .map(|(_, name, key)| (*name, *key))
            .chain([("Palette de commandes", "Ctrl+P")]);
        for (name, key) in shortcuts {
            setting_row(ui, name, "", |ui| {
                ui.label(RichText::new(key).monospace().color(p.muted));
            });
        }
    }

    pub(super) fn plugins_settings(&mut self, ui: &mut egui::Ui, now: f64) {
        let dir = self.plugins.dir.clone();
        let (mut reload, mut open, mut reinstall) = (false, false, false);
        setting_row(
            ui,
            "Plugins installés",
            "Plugins Rhai lus dans le dossier des plugins. L'activation et les réglages sont stockés dans la base : \
             ils sont synchronisés entre vos appareils.",
            |ui| {
                if small_icon_button(ui, "folder-open", &format!("Ouvrir le dossier des plugins\n{}", dir.display()))
                    .clicked()
                {
                    open = true;
                }
                if small_icon_button(ui, "rotate-cw", "Recharger les plugins").clicked() {
                    reload = true;
                }
            },
        );
        setting_row(
            ui,
            "Plugins d'exemple",
            "Réinstalle les plugins fournis avec Mnemo (leurs fichiers sont remplacés).",
            |ui| {
                if ui.button("Réinstaller").clicked() {
                    reinstall = true;
                }
            },
        );
        if open {
            open_folder(ui.ctx(), &dir);
        }
        if reload {
            self.actions.push(UiAction::Core(CoreCmd::ReloadPlugins));
        }
        if reinstall {
            match self.plugins.install_bundled(true) {
                Ok(()) => self.actions.push(UiAction::Core(CoreCmd::ReloadPlugins)),
                Err(e) => self.notice(format!("Erreur : {e}"), true, now),
            }
        }
        if !self.plugins.plugins.is_empty() {
            setting_heading(ui, "Plugins");
        }
        let mut to_save: Vec<(String, PluginState)> = vec![];
        let mut retry = None;
        for (i, p) in self.plugins.plugins.iter().enumerate() {
            let m = &p.manifest;
            let mut desc = m.description.clone();
            if !desc.is_empty() {
                desc.push('\n');
            }
            desc.push_str(&format!("v{}  ·  {}", m.version, m.id));
            if !m.author.is_empty() {
                desc.push_str(&format!("  ·  par {}", m.author));
            }
            let mut enabled = p.enabled;
            let shown = setting_row(ui, &m.name, &desc, |ui| {
                if theme::toggle(ui, &mut enabled).changed() {
                    let mut st = self.plugins.state_of(&m.id);
                    st.enabled = enabled;
                    to_save.push((m.id.clone(), st));
                }
            });
            if !shown {
                continue;
            }
            if let Some(e) = &p.error {
                ui.horizontal(|ui| {
                    ui.colored_label(Color32::from_rgb(230, 90, 90), e);
                    if ui.small_button("Réessayer").clicked() {
                        retry = Some(i);
                    }
                });
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
                                theme::toggle(ui, b);
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
                ui.add_space(6.0);
            }
        }
        if self.plugins.plugins.is_empty() {
            settings_note(ui, "Aucun plugin trouvé dans le dossier.");
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
        settings_note(
            ui,
            "Reliez vos projets du tableau de bord à des dépôts GitHub : import des issues, publication des tickets, \
             synchronisation du titre, de la description, des labels (tags) et de l'état ouvert / fermé.",
        );
        let (mut save, mut remove) = (false, false);
        let stored = self.cfg.github_token.is_some();
        let mut input = std::mem::take(&mut self.github_input);
        setting_row(
            ui,
            "Token d'accès personnel",
            "GitHub › Settings › Developer settings › Personal access tokens. Fine-grained : accès « Issues : \
             lecture et écriture » sur les dépôts voulus ; classique : portée « repo ». Stocké uniquement dans la \
             configuration locale (ou la variable MNEMO_GITHUB_TOKEN), jamais dans la base.",
            |ui| {
                if stored && small_icon_button(ui, "trash-2", "Supprimer le token").clicked() {
                    remove = true;
                }
                if ui.button("Enregistrer").clicked() {
                    save = true;
                }
                let hint = if stored { "•••••••• (enregistré)" } else { "ghp_… ou github_pat_…" };
                let r = ui.add(
                    egui::TextEdit::singleline(&mut input)
                        .password(true)
                        .hint_text(hint)
                        .desired_width(ui.available_width()),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    save = true;
                }
            },
        );
        self.github_input = input;
        if save && !self.github_input.trim().is_empty() {
            self.cfg.github_token = Some(self.github_input.trim().to_string());
            self.github_input.clear();
            self.cfg.save();
            self.send_github_config();
            self.dash.github_user = None;
        }
        if remove {
            self.cfg.github_token = None;
            self.cfg.save();
            self.send_github_config();
            self.dash.github_user = None;
        }
        let mut api_done = false;
        setting_row(
            ui,
            "URL de l'API",
            "https://api.github.com, ou https://<serveur>/api/v3 pour GitHub Enterprise.",
            |ui| {
                let r =
                    ui.add(egui::TextEdit::singleline(&mut self.cfg.github_api).desired_width(ui.available_width()));
                api_done = r.lost_focus();
            },
        );
        if api_done {
            self.cfg.save();
            self.send_github_config();
        }
        let mut auto = self.cfg.github_auto_sync;
        setting_row(
            ui,
            "Synchronisation automatique",
            "Synchronise les projets reliés toutes les 5 minutes, tableau de bord ouvert.",
            |ui| {
                theme::toggle(ui, &mut auto);
            },
        );
        if auto != self.cfg.github_auto_sync {
            self.cfg.github_auto_sync = auto;
            self.cfg.save();
        }
        let status = match &self.dash.github_user {
            Some(Ok(login)) => format!("Connecté en tant que {login}."),
            Some(Err(e)) => e.clone(),
            None if stored => "Vérifie le token auprès de GitHub.".into(),
            None => "Enregistrez d'abord un token.".into(),
        };
        let mut test = false;
        setting_row(ui, "Tester la connexion", &status, |ui| {
            if ui.add_enabled(stored, egui::Button::new("Tester")).clicked() {
                test = true;
            }
        });
        if test {
            self.dash.github_user = None;
            self.db.send(Cmd::GithubTest);
        }
        let linked: Vec<String> = self
            .board
            .projects
            .iter()
            .filter_map(|pr| pr.github_repo.as_ref().map(|r| format!("{} → {r}", pr.name)))
            .collect();
        let linked = if linked.is_empty() {
            "Aucun : ouvrez le tableau de bord › Modifier le projet › Dépôt GitHub.".to_string()
        } else {
            linked.join("\n")
        };
        setting_row(ui, "Projets reliés", &linked, |_| {});
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
