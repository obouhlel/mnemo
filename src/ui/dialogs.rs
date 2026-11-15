//! Dialogues modaux : nouvelle note / dossier, renommer, supprimer, sélecteur de notes (Ctrl+O)
//! et palette de commandes (Ctrl+P).

use super::*;

fn prompt_footer(ui: &mut egui::Ui, p: &theme::Palette, items: &[(&str, &str)]) {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, p.border));
    egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            for (k, label) in items {
                ui.label(RichText::new(*k).size(12.0).color(p.muted).strong());
                ui.label(RichText::new(*label).size(12.0).color(p.faint));
                ui.add_space(10.0);
            }
        });
    });
}

fn nav_keys(ui: &mut egui::Ui, sel: &mut usize, len: usize) {
    if len == 0 {
        return;
    }
    ui.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            *sel = (*sel + 1) % len;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            *sel = (*sel + len - 1) % len;
        }
    });
    *sel = (*sel).min(len - 1);
}

impl MnemoApp {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context, now: f64) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut close = false;
        let focus = std::mem::take(&mut self.focus_dialog);
        let prompt = matches!(dialog, Dialog::Switcher { .. } | Dialog::Palette { .. });
        let p = self.palette();
        let mid = egui::Id::unique("mnemo-dialog");
        let mut modal = egui::Modal::new(mid)
            .backdrop_color(Color32::from_black_alpha(if self.cfg.dark_mode { 110 } else { 50 }))
            .frame(
                egui::Frame::new()
                    .fill(p.bg2)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(10.0)
                    .inner_margin(if prompt { egui::Margin::ZERO } else { egui::Margin::same(20) })
                    .shadow(ctx.global_style().visuals.window_shadow),
            );
        if prompt {
            modal = modal.area(egui::Modal::default_area(mid).anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0)));
        }
        let modal = modal.show(ctx, |ui| {
            ui.set_width(if prompt { 640.0 } else { 460.0 });
            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            let prompt_field = |ui: &mut egui::Ui, s: &mut String, hint: &str| {
                let r = egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(16, 13))
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(s)
                                .hint_text(RichText::new(hint).color(p.faint))
                                .font(FontId::proportional(16.0))
                                .text_color(p.text)
                                .frame(egui::Frame::NONE)
                                .desired_width(f32::INFINITY),
                        )
                    })
                    .inner;
                if focus {
                    r.request_focus();
                }
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
                ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, p.border));
                ui.add_space(6.0);
                r
            };
            let text_field = |ui: &mut egui::Ui, s: &mut String, hint: &str| {
                let r = ui.add(egui::TextEdit::singleline(s).hint_text(hint).desired_width(f32::INFINITY));
                if focus {
                    r.request_focus();
                }
                r
            };
            match &mut dialog {
                Dialog::NewNote { input, error } => {
                    ui.heading("Nouvelle note");
                    ui.weak("Utilisez « / » pour ranger dans un dossier, ex : Projets/Idée");
                    text_field(ui, input, "Dossier/Nom de la note");
                    if let Some(e) = error {
                        ui.colored_label(Color32::from_rgb(230, 90, 90), e.as_str());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Créer").clicked() || enter {
                            match normalize_path(input) {
                                Ok(path) => {
                                    if self.notes.iter().any(|n| n.path.eq_ignore_ascii_case(&path)) {
                                        *error = Some("Une note porte déjà ce nom".into());
                                    } else {
                                        let content = String::new();
                                        self.flush_save();
                                        self.db.send(Cmd::Create { path, content, open: true, if_missing: false });
                                        close = true;
                                    }
                                }
                                Err(e) => *error = Some(e),
                            }
                        }
                        if ui.button("Annuler").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::NewFolder { input, error } => {
                    ui.heading("Nouveau dossier");
                    text_field(ui, input, "Dossier/Sous-dossier");
                    if let Some(e) = error {
                        ui.colored_label(Color32::from_rgb(230, 90, 90), e.as_str());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Créer").clicked() || enter {
                            match normalize_path(input) {
                                Ok(p) => {
                                    self.db.send(Cmd::CreateFolder(p));
                                    close = true;
                                }
                                Err(e) => *error = Some(e),
                            }
                        }
                        if ui.button("Annuler").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::RenameNote { id, input, error } => {
                    ui.heading("Renommer / déplacer");
                    ui.weak("Les liens [[…]] des autres notes seront mis à jour.");
                    text_field(ui, input, "Dossier/Nouveau nom");
                    if let Some(e) = error {
                        ui.colored_label(Color32::from_rgb(230, 90, 90), e.as_str());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Renommer").clicked() || enter {
                            match normalize_path(input) {
                                Ok(p) => {
                                    self.flush_save();
                                    self.db.send(Cmd::Rename { id: *id, new_path: p });
                                    close = true;
                                }
                                Err(e) => *error = Some(e),
                            }
                        }
                        if ui.button("Annuler").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::RenameFolder { from, input, error } => {
                    ui.heading(format!("Renommer le dossier « {from} »"));
                    text_field(ui, input, "Nouveau chemin");
                    if let Some(e) = error {
                        ui.colored_label(Color32::from_rgb(230, 90, 90), e.as_str());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Renommer").clicked() || enter {
                            match normalize_path(input) {
                                Ok(p) => {
                                    self.flush_save();
                                    self.boards_moved(from, &p);
                                    self.db.send(Cmd::RenameFolder { from: from.clone(), to: p });
                                    close = true;
                                }
                                Err(e) => *error = Some(e),
                            }
                        }
                        if ui.button("Annuler").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::DeleteNote { id, path } => {
                    ui.heading("Supprimer la note ?");
                    ui.label(format!("« {path} » sera supprimée définitivement de la base, pour tous les appareils."));
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(230, 90, 90))).clicked() {
                            if self.open.as_ref().is_some_and(|n| n.id == *id) {
                                self.open = None;
                            }
                            self.db.send(Cmd::Delete(*id));
                            close = true;
                        }
                        if ui.button("Annuler").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::DeleteFolder { path } => {
                    let count = self
                        .notes
                        .iter()
                        .filter(|n| n.path.to_lowercase().starts_with(&(path.to_lowercase() + "/")))
                        .count();
                    ui.heading("Supprimer le dossier ?");
                    ui.label(format!("« {path} » et ses {count} note(s) seront supprimés définitivement."));
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(230, 90, 90))).clicked() {
                            if self
                                .open
                                .as_ref()
                                .is_some_and(|n| n.path.to_lowercase().starts_with(&(path.to_lowercase() + "/")))
                            {
                                self.open = None;
                            }
                            self.boards_deleted(path, true);
                            self.db.send(Cmd::DeleteFolder(path.clone()));
                            close = true;
                        }
                        if ui.button("Annuler").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Switcher { query, sel } => {
                    let q = fold_accents(query);
                    let words: Vec<&str> = q.split_whitespace().collect();
                    let mut items: Vec<&NoteMeta> =
                        self.notes.iter().filter(|n| words.iter().all(|w| fold_accents(&n.path).contains(w))).collect();
                    items.sort_by_key(|n| {
                        (!fold_accents(basename(&n.path)).starts_with(&q), std::cmp::Reverse(n.updated_at))
                    });
                    items.truncate(12);
                    let create =
                        !query.trim().is_empty() && !items.iter().any(|n| n.path.eq_ignore_ascii_case(query.trim()));
                    nav_keys(ui, sel, items.len() + usize::from(create));
                    let r = prompt_field(ui, query, "Trouver ou créer une note…");
                    if r.changed() {
                        *sel = 0;
                    }
                    let mut chosen: Option<Option<Uuid>> = None;
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, n) in items.iter().enumerate() {
                            let r =
                                theme::suggestion_row(ui, basename(&n.path), parent(&n.path).unwrap_or(""), *sel == i);
                            if r.clicked() || (enter && *sel == i) {
                                chosen = Some(Some(n.id));
                            }
                        }
                        if create {
                            let i = items.len();
                            let r = theme::suggestion_row(ui, query.trim(), "Entrée pour créer", *sel == i);
                            if r.clicked() || (enter && *sel == i) {
                                chosen = Some(None);
                            }
                        }
                        if items.is_empty() && !create {
                            ui.add_space(6.0);
                            ui.label(RichText::new("  Aucun résultat.").color(p.faint));
                        }
                    });
                    prompt_footer(
                        ui,
                        &p,
                        &[
                            ("↑↓", "pour naviguer"),
                            ("↵", "pour ouvrir"),
                            ("ctrl ↵", "dans un nouvel onglet"),
                            ("échap", "pour annuler"),
                        ],
                    );
                    let new_tab = ui.input(|i| i.modifiers.command);
                    match chosen {
                        Some(Some(id)) => {
                            self.actions.push(if new_tab { UiAction::OpenNewTab(id) } else { UiAction::OpenId(id) });
                            close = true;
                        }
                        Some(None) => {
                            self.actions.push(UiAction::OpenLink(query.trim().to_string()));
                            close = true;
                        }
                        None => {}
                    }
                }
                Dialog::Palette { query, sel } => {
                    let q = fold_accents(query);
                    let words: Vec<&str> = q.split_whitespace().collect();
                    let mut items: Vec<(String, String, UiAction)> = CORE_COMMANDS
                        .iter()
                        .map(|(c, name, key)| (name.to_string(), key.to_string(), UiAction::Core(*c)))
                        .chain(
                            self.plugin_cmds
                                .iter()
                                .map(|(i, id, name)| (name.clone(), String::new(), UiAction::Plugin(*i, id.clone()))),
                        )
                        .filter(|(name, _, _)| words.iter().all(|w| fold_accents(name).contains(w)))
                        .collect();
                    items.truncate(14);
                    nav_keys(ui, sel, items.len());
                    let r = prompt_field(ui, query, "Tapez une commande…");
                    if r.changed() {
                        *sel = 0;
                    }
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, (name, key, action)) in items.iter().enumerate() {
                            if theme::suggestion_row(ui, name, key, *sel == i).clicked() || (enter && *sel == i) {
                                self.actions.push(action.clone());
                                close = true;
                            }
                        }
                        if items.is_empty() {
                            ui.add_space(6.0);
                            ui.label(RichText::new("  Aucune commande trouvée.").color(p.faint));
                        }
                    });
                    prompt_footer(
                        ui,
                        &p,
                        &[("↑↓", "pour naviguer"), ("↵", "pour utiliser"), ("échap", "pour annuler")],
                    );
                }
            }
        });
        if modal.should_close() {
            close = true;
        }
        if !close {
            self.dialog = Some(dialog);
        }
        let _ = now;
    }
}
