//! Éléments fixes de la fenêtre : écran de connexion, barre d'onglets, en-tête de la vue
//! (navigation, fil d'Ariane, modes), onglet vide, barre d'état et notifications.

use super::*;

/// Lays out a column of at most `max_w`, horizontally centered (readable line length).
pub(super) fn centered<R>(ui: &mut egui::Ui, max_w: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let avail = ui.available_width();
    let w = (avail - 48.0).min(max_w).max(120.0);
    let pad = ((avail - w) / 2.0).max(0.0);
    ui.horizontal_top(|ui| {
        ui.add_space(pad);
        ui.vertical(|ui| {
            ui.set_width(w);
            add(ui)
        })
        .inner
    })
    .inner
}

impl MnemoApp {
    pub(super) fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, p.bg2);
        let mut child =
            ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Max)));
        let ui = &mut child;
        ui.spacing_mut().item_spacing.x = 1.0;
        ui.add_space(6.0);
        let right_reserved = 44.0;
        let titles: Vec<String> = self.tabs.iter().map(|t| self.tab_title(t)).collect();
        for (i, title) in titles.iter().enumerate() {
            if ui.available_width() < right_reserved + 60.0 {
                break;
            }
            let active = i == self.active_tab;
            let dirty = active && self.open.as_ref().is_some_and(|n| n.dirty());
            let (r, close) = theme::tab(ui, title, active, dirty);
            if close {
                self.actions.push(UiAction::CloseTab(i));
            } else if r.clicked() {
                self.actions.push(UiAction::SwitchTab(i));
            }
            r.context_menu(|ui| {
                if ui.button("Fermer").clicked() {
                    self.actions.push(UiAction::CloseTab(i));
                    ui.close();
                }
                if ui.button("Fermer les autres onglets").clicked() {
                    self.actions.push(UiAction::CloseOtherTabs(i));
                    ui.close();
                }
            });
        }
        // les icônes sont collées en bas comme les onglets : même ligne médiane que leur titre
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Max), |ui| {
            ui.add_space(4.0);
            if theme::icon_button_sized(ui, icon::PLUS, "Nouvel onglet", false, true, 16.0).clicked() {
                self.actions.push(UiAction::Core(CoreCmd::NewTab));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
                ui.add_space(6.0);
                if !self.right_visible(ui.ctx())
                    && theme::icon_button_sized(
                        ui,
                        icon::PANEL_RIGHT,
                        "Afficher le panneau de droite",
                        false,
                        true,
                        16.0,
                    )
                    .clicked()
                {
                    self.actions.push(UiAction::Core(CoreCmd::ToggleRight));
                }
            });
        });
    }

    pub(super) fn view_header(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
        let Some(n) = &self.open else { return };
        let (id, path) = (n.id, n.path.clone());
        let tab = &self.tabs[self.active_tab];
        let (can_back, can_fwd) = (!tab.back.is_empty(), !tab.fwd.is_empty());
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
        // centered breadcrumb: folders faint, note title normal
        let mut job = LayoutJob::default();
        let fmt = |c: Color32| TextFormat { font_id: FontId::proportional(14.0), color: c, ..Default::default() };
        if let Some(parent) = parent(&path) {
            for seg in parent.split('/') {
                job.append(seg, 0.0, fmt(p.faint));
                job.append("  /  ", 0.0, fmt(p.faint));
            }
        }
        job.append(basename(&path), 0.0, fmt(p.text));
        let galley = ui.painter().layout_job(job);
        let galley_w = galley.size().x;
        let conflict = n.conflict.is_some();
        let flash = ui.input(|i| i.time) - n.remote_flash < 3.0;
        let crumb = egui::Rect::from_center_size(rect.center(), galley.size());
        ui.painter().with_clip_rect(rect.shrink2(egui::vec2(90.0, 0.0))).galley(crumb.min, galley, p.text);
        let crumb_resp = ui.interact(crumb, egui::Id::unique(("crumb", id)), egui::Sense::click());
        if crumb_resp.double_clicked() {
            self.actions.push(UiAction::RenameNote(id, path.clone()));
        }
        crumb_resp.on_hover_text(format!("{path}\nDouble-clic pour déplacer"));

        let mut child = ui
            .new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let ui = &mut child;
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add_space(8.0);
        if theme::icon_button_sized(ui, icon::BACK, "Revenir en arrière", false, can_back, 16.0).clicked() {
            self.actions.push(UiAction::Back);
        }
        if theme::icon_button_sized(ui, icon::FORWARD, "Avancer", false, can_fwd, 16.0).clicked() {
            self.actions.push(UiAction::Forward);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            let more = theme::icon_button_sized(ui, icon::MORE, "Plus d'options", false, true, 16.0);
            egui::Popup::menu(&more).show(|ui| {
                ui.set_min_width(200.0);
                if ui.button("Renommer / déplacer…").clicked() {
                    self.actions.push(UiAction::RenameNote(id, path.clone()));
                    ui.close();
                }
                if ui.button("Copier le lien [[…]]").clicked() {
                    ui.ctx().copy_text(format!("[[{}]]", basename(&path)));
                    ui.close();
                }
                if ui.button("Mode divisé (édition + lecture)").clicked() {
                    self.actions.push(UiAction::Core(CoreCmd::SplitMode));
                    ui.close();
                }
                ui.separator();
                if ui.button(RichText::new("Supprimer le fichier").color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked()
                {
                    self.actions.push(UiAction::DeleteNote(id, path.clone()));
                    ui.close();
                }
            });
            let reading = self.cfg.view_mode == ViewMode::Preview;
            let (ic, tip) = if reading {
                (icon::EDITING, "Basculer en mode édition (Ctrl+E)")
            } else {
                (icon::READING, "Basculer en mode lecture (Ctrl+E)")
            };
            if theme::icon_button_sized(ui, ic, tip, false, true, 16.0).clicked() {
                self.actions.push(UiAction::Core(CoreCmd::ToggleMode));
            }
            if self.cfg.view_mode == ViewMode::Split
                && theme::icon_button_sized(ui, icon::SPLIT, "Quitter le mode divisé", true, true, 16.0).clicked()
            {
                self.cfg.view_mode = ViewMode::Edit;
                self.cfg.save();
            }
            // infos discrètes (plugins, conflit, mise à jour distante), à droite du fil d'Ariane
            let room = ui.available_width() - (rect.width() / 2.0 + galley_w / 2.0) - 16.0;
            if room > 40.0 {
                ui.add_space(6.0);
                ui.scope(|ui| {
                    ui.set_max_width(room);
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let small = |t: &str, c: Color32| egui::Label::new(RichText::new(t).size(12.0).color(c)).truncate();
                    if conflict {
                        ui.add(small("Conflit à résoudre", Color32::from_rgb(0xe9, 0x97, 0x3f)));
                    }
                    if flash {
                        ui.add(small("mis à jour à distance", p.accent));
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                    }
                    for st in self.plugin_status.iter().rev() {
                        ui.add(small(st, p.faint));
                    }
                });
            }
        });
    }

    /// Onglet « Graphe » : en-tête comme une note, puis la vue sur toute la place restante.
    pub(super) fn graph_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Graphe des liens",
            FontId::proportional(14.0),
            p.text,
        );
        // la note mise en évidence : la dernière ouverte
        let current = self
            .cfg
            .last_note
            .as_ref()
            .and_then(|path| self.notes.iter().find(|n| &n.path == path))
            .map(|n| n.id.to_string());
        let mut clicked = None;
        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 16, top: 0, bottom: 16 }).show(ui, |ui| {
            if !self.graph.loaded {
                ui.spinner();
                return;
            }
            clicked = self.graph.ui(ui, current.as_deref());
        });
        if let Some(c) = clicked {
            self.actions.push(UiAction::OpenLink(c));
        }
    }

    pub(super) fn empty_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.32).max(20.0));
            ui.label(RichText::new("Aucun fichier n'est ouvert").size(22.0).color(p.muted));
            ui.add_space(18.0);
            let link = |ui: &mut egui::Ui, t: &str| {
                ui.add(egui::Label::new(RichText::new(t).size(15.0).color(p.link)).sense(egui::Sense::click()))
            };
            if link(ui, "Créer une nouvelle note (Ctrl+N)").clicked() {
                self.actions.push(UiAction::Core(CoreCmd::NewNote));
            }
            ui.add_space(6.0);
            if link(ui, "Aller au fichier (Ctrl+O)").clicked() {
                self.actions.push(UiAction::Core(CoreCmd::Switcher));
            }
            ui.add_space(6.0);
            if self.tabs.len() > 1 && link(ui, "Fermer").clicked() {
                self.actions.push(UiAction::Core(CoreCmd::CloseTab));
            }
        });
    }

    /// Indicateur de synchronisation (bas du ruban) : couleur selon l'état, détails au survol.
    pub(super) fn sync_indicator(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
        let dirty = self.open.as_ref().is_some_and(|n| n.dirty());
        let orange = Color32::from_rgb(0xe9, 0x97, 0x3f);
        let (ic, color, mut tip) = match (&self.conn, self.live) {
            (ConnState::Connected, true) if dirty => (icon::SYNC, p.faint, "Enregistrement…".to_string()),
            (ConnState::Connected, true) => {
                (icon::SYNC, Color32::from_rgb(0x44, 0xcf, 0x6e), "Synchronisé en direct".to_string())
            }
            (ConnState::Connected, false) => (icon::SYNC, orange, "Connecté, flux live en reconnexion…".to_string()),
            (ConnState::Connecting, _) => (icon::SYNC, orange, "Connexion à la base…".to_string()),
            (ConnState::Disconnected(Some(_)), _) => (
                icon::OFFLINE,
                orange,
                "Hors ligne : tout est enregistré en local et sera synchronisé au retour".to_string(),
            ),
            (ConnState::Disconnected(None), _) => (
                icon::LOCAL,
                p.faint,
                "Local uniquement : notes dans le dossier, sans base de synchronisation".to_string(),
            ),
        };
        if let Some(d) = &self.cfg.vault_dir {
            tip.push_str(&format!("\nDossier : {}", d.display()));
        }
        if !self.cfg.database_url.is_empty() {
            tip.push_str(&format!("\nBase : {}", redact_url(&self.cfg.database_url)));
        }
        let (rect, r) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(rect, 5.0, p.hover);
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            ic,
            FontId::new(15.0, theme::icon_family()),
            color,
        );
        if r.on_hover_text(tip).clicked() {
            self.settings_tab = SettingsTab::Connection;
            self.show_settings = true;
        }
    }

    pub(super) fn notices_ui(&mut self, ctx: &egui::Context, now: f64) {
        self.notices.retain(|n| n.1 > now);
        if self.notices.is_empty() {
            return;
        }
        let next = self.notices.iter().map(|n| n.1).fold(f64::MAX, f64::min);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((next - now).max(0.05)));
        egui::Area::new(egui::Id::unique("notices"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 16.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                for (msg, _, err) in &self.notices {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(360.0);
                        let t = RichText::new(msg);
                        ui.label(if *err { t.color(Color32::from_rgb(240, 110, 110)) } else { t });
                    });
                    ui.add_space(6.0);
                }
            });
    }
}
