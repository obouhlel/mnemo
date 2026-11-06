//! Ruban d'icônes et barres latérales : explorateur de fichiers (arborescence), recherche, tags
//! à gauche ; rétroliens, panneaux des plugins et tickets liés à droite.

use super::*;

fn highlight_job(snippet: &str, ui: &egui::Ui) -> LayoutJob {
    let mut job = LayoutJob::default();
    let normal = TextFormat {
        font_id: FontId::proportional(ui.style().text_styles[&egui::TextStyle::Small].size + 1.0),
        color: ui.visuals().weak_text_color(),
        ..Default::default()
    };
    let hl = TextFormat {
        color: ui.visuals().strong_text_color(),
        background: ui.visuals().selection.bg_fill.gamma_multiply(0.45),
        ..normal.clone()
    };
    let mut on = false;
    for part in snippet.split(['\u{1}', '\u{2}']) {
        job.append(part, 0.0, if on { hl.clone() } else { normal.clone() });
        on = !on;
    }
    // split() alternates correctly only when markers alternate, which ts_headline guarantees
    job.wrap.max_width = ui.available_width();
    job
}

fn tree_ui(
    ui: &mut egui::Ui,
    folder: &TreeFolder,
    open: Option<Uuid>,
    actions: &mut Vec<UiAction>,
    depth: usize,
    expanded: &std::collections::HashSet<String>,
    sort: SortOrder,
) {
    let mut folders: Vec<&TreeFolder> = folder.folders.values().collect();
    if sort == SortOrder::NameDesc {
        folders.reverse();
    }
    for sub in folders {
        let is_open = expanded.contains(&sub.path.to_lowercase());
        let r = theme::tree_row(ui, &sub.name, depth, false, Some(is_open));
        if r.clicked() {
            actions.push(UiAction::ToggleFolder(sub.path.clone()));
        }
        r.context_menu(|ui| {
            if ui.button("Nouvelle note").clicked() {
                actions.push(UiAction::NewNoteIn(sub.path.clone()));
                ui.close();
            }
            if ui.button("Nouveau dossier").clicked() {
                actions.push(UiAction::NewFolderIn(sub.path.clone()));
                ui.close();
            }
            if ui.button("Renommer / déplacer…").clicked() {
                actions.push(UiAction::RenameFolder(sub.path.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked() {
                actions.push(UiAction::DeleteFolder(sub.path.clone()));
                ui.close();
            }
        });
        if is_open {
            tree_ui(ui, sub, open, actions, depth + 1, expanded, sort);
        }
    }
    let mut notes: Vec<&(String, Uuid, chrono::DateTime<chrono::Utc>)> = folder.notes.iter().collect();
    match sort {
        SortOrder::NameAsc => {}
        SortOrder::NameDesc => notes.reverse(),
        SortOrder::Modified => notes.sort_by_key(|n| std::cmp::Reverse(n.2)),
    }
    for (name, id, _) in notes {
        let r = theme::tree_row(ui, name, depth, open == Some(*id), None);
        if r.clicked() {
            if ui.input(|i| i.modifiers.command) {
                actions.push(UiAction::OpenNewTab(*id));
            } else {
                actions.push(UiAction::OpenId(*id));
            }
        }
        if r.middle_clicked() {
            actions.push(UiAction::OpenNewTab(*id));
        }
        let path = if folder.path.is_empty() { name.clone() } else { format!("{}/{}", folder.path, name) };
        r.context_menu(|ui| {
            if ui.button("Ouvrir dans un nouvel onglet").clicked() {
                actions.push(UiAction::OpenNewTab(*id));
                ui.close();
            }
            if ui.button("Renommer / déplacer…").clicked() {
                actions.push(UiAction::RenameNote(*id, path.clone()));
                ui.close();
            }
            if ui.button("Copier le lien [[…]]").clicked() {
                ui.ctx().copy_text(format!("[[{name}]]"));
                ui.close();
            }
            ui.separator();
            if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked() {
                actions.push(UiAction::DeleteNote(*id, path.clone()));
                ui.close();
            }
        });
    }
}

impl MnemoApp {
    pub(super) fn ribbon(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(5.0);
            if theme::icon_button(ui, icon::PANEL_LEFT, "Afficher / masquer le panneau latéral", false).clicked() {
                self.actions.push(UiAction::Core(CoreCmd::ToggleLeft));
            }
            ui.add_space(10.0);
            let items: [(&str, &str, CoreCmd); 6] = [
                (icon::DASHBOARD, "Tableau de bord : projets et tickets (Ctrl+Shift+D)", CoreCmd::Dashboard),
                (icon::SWITCHER, "Ouvrir le sélecteur rapide", CoreCmd::Switcher),
                (icon::GRAPH, "Ouvrir la vue graphique", CoreCmd::Graph),
                (icon::DAILY, "Ouvrir la note du jour", CoreCmd::DailyNote),
                (icon::TEMPLATES, "Insérer un modèle", CoreCmd::Templates),
                (icon::PALETTE, "Ouvrir la palette de commandes", CoreCmd::Palette),
            ];
            for (ic, tip, c) in items {
                if theme::icon_button(ui, ic, tip, false).clicked() {
                    self.actions.push(UiAction::Core(c));
                }
                ui.add_space(2.0);
            }
        });
    }

    /// Name shown in the vault footer: the database name.
    pub(super) fn vault_name(&self) -> String {
        let url = &self.cfg.database_url;
        url.rsplit('/')
            .next()
            .and_then(|s| s.split('?').next())
            .filter(|s| !s.is_empty() && !s.contains('@'))
            .unwrap_or("Mnemo")
            .to_string()
    }

    pub(super) fn left_sidebar(&mut self, ui: &mut egui::Ui, now: f64) {
        let p = self.pal();
        // header: view tabs
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 40.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(6.0);
                for (tab, ic, tip) in [
                    (LeftTab::Files, icon::FILES_TAB, "Fichiers"),
                    (LeftTab::Search, icon::SEARCH, "Rechercher"),
                    (LeftTab::Tags, icon::TAG, "Tags"),
                ] {
                    if theme::icon_button_sized(ui, ic, tip, self.left_tab == tab, true, 16.0).clicked() {
                        self.left_tab = tab;
                        if tab == LeftTab::Search {
                            self.focus_search = true;
                        }
                    }
                }
            },
        );
        let footer_h = 42.0;
        let body_h = (ui.available_height() - footer_h).max(50.0);
        ui.allocate_ui(egui::vec2(ui.available_width(), body_h), |ui| {
            ui.set_min_height(body_h);
            match self.left_tab {
                LeftTab::Files => {
                    ui.horizontal(|ui| {
                        let total = 4.0 * 28.0 + 3.0 * 4.0;
                        ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if theme::icon_button_sized(ui, icon::NEW_NOTE, "Nouvelle note", false, true, 16.0).clicked() {
                            self.actions.push(UiAction::Core(CoreCmd::NewNote));
                        }
                        if theme::icon_button_sized(ui, icon::NEW_FOLDER, "Nouveau dossier", false, true, 16.0)
                            .clicked()
                        {
                            self.actions.push(UiAction::Core(CoreCmd::NewFolder));
                        }
                        let sort =
                            theme::icon_button_sized(ui, icon::SORT, "Changer l'ordre de tri", false, true, 16.0);
                        egui::Popup::menu(&sort).show(|ui| {
                            for (o, label) in [
                                (SortOrder::NameAsc, "Nom du fichier (A à Z)"),
                                (SortOrder::NameDesc, "Nom du fichier (Z à A)"),
                                (SortOrder::Modified, "Date de modification (récent d'abord)"),
                            ] {
                                if ui.selectable_label(self.sort == o, label).clicked() {
                                    self.actions.push(UiAction::SetSort(o));
                                    ui.close();
                                }
                            }
                        });
                        if theme::icon_button_sized(ui, icon::COLLAPSE, "Tout replier", false, true, 16.0).clicked() {
                            self.actions.push(UiAction::Core(CoreCmd::CollapseAll));
                        }
                    });
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let open_id = self.open.as_ref().map(|n| n.id);
                        let tree = std::mem::take(&mut self.tree);
                        let mut actions = vec![];
                        tree_ui(ui, &tree, open_id, &mut actions, 0, &self.expanded, self.sort);
                        self.tree = tree;
                        self.actions.extend(actions);
                        let rest =
                            ui.allocate_response(ui.available_size().max(egui::vec2(10.0, 40.0)), egui::Sense::click());
                        rest.context_menu(|ui| {
                            if ui.button("Nouvelle note").clicked() {
                                self.actions.push(UiAction::Core(CoreCmd::NewNote));
                                ui.close();
                            }
                            if ui.button("Nouveau dossier").clicked() {
                                self.actions.push(UiAction::Core(CoreCmd::NewFolder));
                                ui.close();
                            }
                        });
                    });
                }
                LeftTab::Search => {
                    ui.add_space(2.0);
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 0)).show(ui, |ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.search_q)
                                .id(egui::Id::unique("search-box"))
                                .hint_text("Rechercher…")
                                .margin(egui::Margin::symmetric(8, 5))
                                .background_color(p.bg)
                                .desired_width(f32::INFINITY),
                        );
                        if std::mem::take(&mut self.focus_search) {
                            r.request_focus();
                        }
                        if r.changed() {
                            self.search_due = Some(now + 0.25);
                            if self.search_q.trim().is_empty() {
                                self.search_results.clear();
                            }
                        }
                    });
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 0)).show(ui, |ui| {
                            if self.search_results.is_empty() && !self.search_q.trim().is_empty() {
                                ui.label(RichText::new("Aucun résultat.").color(p.faint));
                            }
                            let open_id = self.open.as_ref().map(|n| n.id);
                            for hit in &self.search_results {
                                if theme::tree_row(ui, basename(&hit.path), 0, open_id == Some(hit.id), Some(true))
                                    .clicked()
                                {
                                    self.actions.push(UiAction::OpenId(hit.id));
                                }
                                egui::Frame::new()
                                    .fill(p.bg)
                                    .corner_radius(4.0)
                                    .inner_margin(egui::Margin::symmetric(8, 5))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(highlight_job(&hit.snippet.replace('\n', " "), ui));
                                    });
                                ui.add_space(6.0);
                            }
                        });
                    });
                }
                LeftTab::Tags => {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            if self.tags.is_empty() {
                                ui.label(RichText::new("Aucun tag. Écrivez #montag dans une note.").color(p.faint));
                            }
                            for (tag, count) in &self.tags {
                                if theme::suggestion_row(ui, &format!("#{tag}"), &count.to_string(), false).clicked() {
                                    self.actions.push(UiAction::OpenTag(tag.clone()));
                                }
                            }
                        });
                    });
                }
            }
        });
        // vault footer
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, p.border));
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), footer_h - 2.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(6.0);
                let name = self.vault_name();
                let r = ui
                    .add(egui::Button::new(RichText::new(format!("{name}  ")).color(p.text).strong()).frame(false))
                    .on_hover_text(format!("Base : {}", redact_url(&self.cfg.database_url)));
                ui.label(theme::icon_text(icon::VAULT_SWITCH, 13.0).color(p.faint));
                if r.clicked() {
                    self.settings_tab = SettingsTab::Connection;
                    self.show_settings = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    if theme::icon_button_sized(ui, icon::SETTINGS, "Paramètres", self.show_settings, true, 16.0)
                        .clicked()
                    {
                        self.actions.push(UiAction::Core(CoreCmd::Settings));
                    }
                    if theme::icon_button_sized(ui, icon::HELP, "Aide et raccourcis", false, true, 16.0).clicked() {
                        self.settings_tab = SettingsTab::About;
                        self.show_settings = true;
                    }
                });
            },
        );
    }

    pub(super) fn right_sidebar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let tabs: Vec<(&str, String)> = std::iter::once((icon::LINK, "Rétroliens".to_string()))
            .chain(std::iter::once((icon::TICKET, "Tickets liés".to_string())))
            .chain(
                self.plugin_panels
                    .iter()
                    .map(|(name, _)| (if name == "Plan" { icon::LIST } else { icon::PUZZLE }, name.clone())),
            )
            .collect();
        self.right_tab = self.right_tab.min(tabs.len() - 1);
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 40.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(6.0);
                for (i, (ic, name)) in tabs.iter().enumerate() {
                    if theme::icon_button_sized(ui, ic, name, self.right_tab == i, true, 16.0).clicked() {
                        self.right_tab = i;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    if theme::icon_button_sized(
                        ui,
                        icon::PANEL_RIGHT,
                        "Masquer le panneau de droite",
                        false,
                        true,
                        16.0,
                    )
                    .clicked()
                    {
                        self.actions.push(UiAction::Core(CoreCmd::ToggleRight));
                    }
                });
            },
        );
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.open.is_none() {
                    ui.label(RichText::new("Aucun fichier actif.").color(p.faint));
                    return;
                }
                if self.right_tab == 0 {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Mentions liées").color(p.text).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(self.backlinks.len().to_string()).color(p.faint).small());
                        });
                    });
                    ui.add_space(4.0);
                    if self.backlinks.is_empty() {
                        ui.label(RichText::new("Aucune mention trouvée.").color(p.faint));
                    }
                    for b in &self.backlinks {
                        if theme::tree_row(ui, basename(&b.path), 0, false, Some(true)).on_hover_text(&b.path).clicked()
                        {
                            self.actions.push(UiAction::OpenId(b.id));
                        }
                        for s in &b.snippets {
                            egui::Frame::new()
                                .fill(p.bg)
                                .corner_radius(4.0)
                                .inner_margin(egui::Margin::symmetric(8, 5))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.add(egui::Label::new(RichText::new(s).size(12.5).color(p.muted)).wrap());
                                });
                            ui.add_space(3.0);
                        }
                        ui.add_space(6.0);
                    }
                } else if self.right_tab == 1 {
                    self.note_tickets_panel(ui);
                } else if let Some((name, md)) = self.plugin_panels.get(self.right_tab - 2) {
                    ui.label(RichText::new(name).color(p.text).strong());
                    ui.add_space(4.0);
                    let src = markdown::preprocess(md);
                    CommonMarkViewer::new().show_with_id(
                        egui::Id::unique(("plugin-panel", self.right_tab)),
                        ui,
                        &mut self.md_cache,
                        &src,
                    );
                }
            });
        });
    }

    /// Side panels auto-hide on narrow windows unless explicitly toggled
    pub(super) fn left_visible(&self, ctx: &egui::Context) -> bool {
        self.cfg.show_left && ctx.content_rect().width() >= 560.0
    }

    pub(super) fn right_visible(&self, ctx: &egui::Context) -> bool {
        self.cfg.show_right && ctx.content_rect().width() >= 1000.0
    }

    pub(super) fn rebuild_tree(&mut self) {
        let mut root = TreeFolder::default();
        fn folder_mut<'a>(root: &'a mut TreeFolder, path: &str) -> &'a mut TreeFolder {
            let mut cur = root;
            let mut acc = String::new();
            for seg in path.split('/') {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(seg);
                let p = acc.clone();
                cur = cur.folders.entry(seg.to_lowercase()).or_insert_with(|| TreeFolder {
                    name: seg.to_string(),
                    path: p,
                    ..Default::default()
                });
            }
            cur
        }
        for f in &self.folders {
            folder_mut(&mut root, f);
        }
        for n in &self.notes {
            let target = match parent(&n.path) {
                Some(p) => folder_mut(&mut root, p),
                None => &mut root,
            };
            target.notes.push((basename(&n.path).to_string(), n.id, n.updated_at));
        }
        fn sort(f: &mut TreeFolder) {
            f.notes.sort_by_key(|n| n.0.to_lowercase());
            f.folders.values_mut().for_each(sort);
        }
        sort(&mut root);
        self.tree = root;
    }
}
